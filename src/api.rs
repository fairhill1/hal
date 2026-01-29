use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Value],
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [Value]>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    #[allow(dead_code)]
    pub total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: Option<String>,
    tool_calls: Option<Vec<Value>>,
}

pub struct ApiResponse {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<Value>>,
    pub usage: Option<Usage>,
}

pub fn chat(
    base_url: &str,
    api_key: &str,
    model: &str,
    messages: &[Value],
    tools: &[Value],
) -> Result<ApiResponse, String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let request = ChatRequest {
        model,
        messages,
        tools: if tools.is_empty() { None } else { Some(tools) },
    };

    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();

    let response = agent.post(&url)
        .header("Authorization", &format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .send_json(&request)
        .map_err(|e| e.to_string())?;

    let status = response.status().as_u16();
    if status >= 400 {
        let body: String = response.into_body().read_to_string()
            .unwrap_or_else(|_| "Unknown error".to_string());
        if let Ok(json) = serde_json::from_str::<Value>(&body) {
            if let Some(msg) = json["error"]["message"].as_str() {
                return Err(format!("{}: {}", status, msg));
            }
        }
        return Err(format!("{}: {}", status, body));
    }

    let body: ChatResponse = response.into_body().read_json()
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let choice = body.choices.into_iter().next()
        .ok_or("No response choices")?;

    Ok(ApiResponse {
        content: choice.message.content,
        tool_calls: choice.message.tool_calls,
        usage: body.usage,
    })
}
