use crate::api;
use crate::config::{Config, Mode, Provider};
use crate::sandbox::{self, SandboxConfig};
use crate::session::{self, Session};
use crate::tools;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;

pub const MAX_PICKER_ITEMS: usize = 10;

fn load_context_file() -> Option<String> {
    let path = Path::new("HAL.md");
    if path.exists() {
        std::fs::read_to_string(path).ok()
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    Idle,
    Thinking,
    ToolCall(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickerMode {
    None,
    Files,
    Commands,
}

#[derive(Debug, Clone)]
pub struct PermissionModal {
    pub path: String,
    pub reason: String,
    pub options: Vec<&'static str>,
    pub selected: usize,
    pub pending_tool_id: String,
}

impl PermissionModal {
    pub fn new(path: String, reason: String, tool_id: String) -> Self {
        Self {
            path,
            reason,
            options: vec!["Allow for project", "Allow globally", "Allow once", "Deny"],
            selected: 0,
            pending_tool_id: tool_id,
        }
    }
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    Tool { name: String, path: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

pub struct App {
    pub config: Config,
    pub input: String,
    pub input_cursor: usize,
    pub messages: Vec<ChatMessage>,
    pub api_messages: Vec<Value>,
    pub state: AppState,
    pub scroll_offset: u16,
    pub history: Vec<String>,
    pub history_pos: usize,
    pub saved_input: String,
    pub picker_mode: PickerMode,
    pub picker_query: String,
    pub picker_results: Vec<String>,
    pub picker_selected: usize,
    pub files_cache: Option<Vec<String>>,
    pub should_quit: bool,
    pub error: Option<String>,
    pub token_usage: Option<(u32, u32)>, // (prompt, completion)
    pub permission_modal: Option<PermissionModal>,
    pub temp_allowed_paths: Vec<String>, // Paths allowed for this session only
    tool_defs: Vec<Value>,
    api_key: String,
    provider: Provider,
    pending_response: Option<Receiver<Result<api::ApiResponse, String>>>,
    pending_tool_calls: Vec<(String, String, String)>, // (id, name, args) waiting for permission
    pending_tool_execution: Option<Receiver<ToolExecutionResult>>,
    session: Session,
    cancel_flag: Arc<AtomicBool>,
}

struct ToolExecutionResult {
    id: String,
    name: String,
    path: Option<String>,
    result: String,
}

impl App {
    pub fn new(config: Config, session: Option<Session>) -> Result<Self, String> {
        let provider = config
            .get_provider()
            .ok_or_else(|| format!("Provider '{}' not found", config.default_provider))?
            .clone();

        let api_key = provider
            .api_key
            .clone()
            .or_else(|| std::env::var(&provider.api_key_env).ok())
            .ok_or_else(|| format!("Set ${} with your API key", provider.api_key_env))?;

        let tool_defs = tools::get_tool_definitions(&config.mode);
        // Build system prompt with optional HAL.md context
        let mut system_prompt = get_system_prompt(&config.mode).to_string();
        if config.mode == Mode::Coding {
            if let Some(context) = load_context_file() {
                system_prompt.push_str("\n\n## Project Context\n\n");
                system_prompt.push_str(&context);
            }
        }

        // Start with system message
        let mut api_messages = vec![json!({
            "role": "system",
            "content": system_prompt
        })];

        // Restore from session if provided
        let (messages, session) = if let Some(mut s) = session {
            // Restore API messages (skip system prompt from saved, use fresh one)
            if s.api_messages.len() > 1 {
                api_messages.extend(s.api_messages[1..].iter().cloned());
            }

            // Update session timestamp
            s.updated_at = chrono::Utc::now().timestamp();
            let msgs = s.messages.clone();
            (msgs, s)
        } else {
            (Vec::new(), Session::new())
        };

        Ok(App {
            config,
            input: String::new(),
            input_cursor: 0,
            messages,
            api_messages,
            state: AppState::Idle,
            scroll_offset: 0,
            history: Vec::new(),
            history_pos: 0,
            saved_input: String::new(),
            picker_mode: PickerMode::None,
            picker_query: String::new(),
            picker_results: Vec::new(),
            picker_selected: 0,
            files_cache: None,
            should_quit: false,
            error: None,
            token_usage: None,
            permission_modal: None,
            temp_allowed_paths: Vec::new(),
            tool_defs,
            api_key,
            provider,
            pending_response: None,
            pending_tool_calls: Vec::new(),
            pending_tool_execution: None,
            session,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn save_session(&mut self) {
        if self.messages.is_empty() {
            return;
        }

        // Update session data
        self.session.updated_at = chrono::Utc::now().timestamp();
        self.session.messages = self.messages.clone();
        self.session.api_messages = self.api_messages.clone();

        // Set title from first user message if not set
        if self.session.title.is_empty() {
            if let Some(first_user) = self.messages.iter().find(|m| matches!(m.role, MessageRole::User)) {
                self.session.title = first_user
                    .content
                    .chars()
                    .take(50)
                    .collect::<String>()
                    .trim()
                    .to_string();
                if first_user.content.len() > 50 {
                    self.session.title.push_str("...");
                }
            }
        }

        let _ = self.session.save();
    }

    pub fn submit_input(&mut self) {
        let input = self.input.trim().to_string();
        if input.is_empty() {
            return;
        }

        // Handle commands
        match input.as_str() {
            "/quit" | "/exit" | "/q" => {
                self.save_session();
                self.should_quit = true;
                return;
            }
            "/clear" => {
                self.save_session();
                self.messages.clear();
                self.api_messages.truncate(1);
                self.input.clear();
                self.input_cursor = 0;
                self.token_usage = None;
                self.session = Session::new();
                return;
            }
            "/sessions" => {
                let sessions = session::list_sessions();
                if sessions.is_empty() {
                    self.messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content: "No saved sessions.".to_string(),
                    });
                } else {
                    let list: Vec<String> = sessions
                        .iter()
                        .take(10)
                        .map(|s| {
                            let date = chrono::DateTime::from_timestamp(s.updated_at, 0)
                                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_default();
                            let title = if s.title.is_empty() { "(untitled)" } else { &s.title };
                            format!("**{}** - {} ({})", s.id, title, date)
                        })
                        .collect();
                    self.messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content: format!("**Saved sessions:**\n{}", list.join("\n")),
                    });
                }
                self.input.clear();
                self.input_cursor = 0;
                return;
            }
            "/help" => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: HELP_TEXT.to_string(),
                });
                self.input.clear();
                self.input_cursor = 0;
                return;
            }
            "/provider" => {
                let mut names: Vec<String> = self.config.providers.keys().cloned().collect();
                names.sort();
                let list: Vec<String> = names
                    .iter()
                    .map(|n| {
                        if n == &self.config.default_provider {
                            format!("- **{}** (active)", n)
                        } else {
                            format!("- {}", n)
                        }
                    })
                    .collect();
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: format!("**Providers:**\n{}\n\nSwitch with `/provider <name>`", list.join("\n")),
                });
                self.input.clear();
                self.input_cursor = 0;
                return;
            }
            _ => {}
        }

        // Handle /provider <name>
        if let Some(name) = input.strip_prefix("/provider ") {
            let name = name.trim().to_string();
            if let Some(new_provider) = self.config.providers.get(&name) {
                let key = new_provider
                    .api_key
                    .clone()
                    .or_else(|| std::env::var(&new_provider.api_key_env).ok());
                if let Some(key) = key {
                    self.config.default_provider = name.clone();
                    self.provider = new_provider.clone();
                    self.api_key = key;
                    let _ = self.config.save();
                    self.messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content: format!("Switched to **{}** ({})", name, self.provider.model),
                    });
                } else {
                    self.messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content: format!("No API key for **{}**. Set it with `/key <key>` after switching, or set ${}", name, new_provider.api_key_env),
                    });
                }
            } else {
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: format!("Unknown provider: {}", name),
                });
            }
            self.input.clear();
            self.input_cursor = 0;
            return;
        }

        // Handle /key <value>
        if let Some(key) = input.strip_prefix("/key ") {
            let key = key.trim().to_string();
            if key.is_empty() {
                self.error = Some("API key cannot be empty".to_string());
            } else {
                self.api_key = key.clone();
                if let Some(provider) = self.config.providers.get_mut(&self.config.default_provider) {
                    provider.api_key = Some(key);
                    self.provider = provider.clone();
                }
                let _ = self.config.save();
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: format!("API key updated for **{}**", self.config.default_provider),
                });
            }
            self.input.clear();
            self.input_cursor = 0;
            return;
        }

        // Handle /load command
        if input.starts_with("/load ") {
            let id = input.strip_prefix("/load ").unwrap().trim();
            match Session::load(id) {
                Ok(s) => {
                    self.save_session(); // Save current before loading

                    // Restore messages
                    self.messages = s.messages.clone();

                    // Restore API messages (keep system prompt, replace rest)
                    self.api_messages.truncate(1);
                    if s.api_messages.len() > 1 {
                        self.api_messages.extend(s.api_messages[1..].iter().cloned());
                    }

                    self.session = s;
                    self.token_usage = None;
                }
                Err(e) => {
                    self.error = Some(format!("Failed to load session: {}", e));
                }
            }
            self.input.clear();
            self.input_cursor = 0;
            return;
        }

        // Add to history
        if self.history.last().map(|s| s.as_str()) != Some(&input) {
            self.history.push(input.clone());
        }
        self.history_pos = self.history.len();

        // Expand file references
        let expanded = expand_file_refs(&input);

        // Add user message
        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: input,
        });

        // Add visual feedback for attached files
        for (path, lines) in &expanded.files_read {
            self.messages.push(ChatMessage {
                role: MessageRole::Tool {
                    name: "read_file".to_string(),
                    path: Some(path.clone()),
                },
                content: "\n".repeat(*lines), // Fake content with right line count
            });
        }

        self.api_messages.push(json!({
            "role": "user",
            "content": expanded.text
        }));

        self.input.clear();
        self.input_cursor = 0;
        self.state = AppState::Thinking;
        self.start_api_call();
    }

    fn start_api_call(&mut self) {
        // Reset cancel flag for new request
        self.cancel_flag.store(false, Ordering::SeqCst);

        let (tx, rx) = mpsc::channel();
        self.pending_response = Some(rx);

        let base_url = self.provider.base_url.clone();
        let api_key = self.api_key.clone();
        let model = self.provider.model.clone();
        let messages = self.api_messages.clone();
        let tool_defs = self.tool_defs.clone();
        let cancel_flag = self.cancel_flag.clone();

        thread::spawn(move || {
            let result = api::chat(&base_url, &api_key, &model, &messages, &tool_defs);
            // Only send if not cancelled
            if !cancel_flag.load(Ordering::SeqCst) {
                let _ = tx.send(result);
            }
        });
    }

    pub fn abort_request(&mut self) {
        if self.state == AppState::Idle {
            return;
        }

        // Signal cancellation
        self.cancel_flag.store(true, Ordering::SeqCst);

        // Clear pending state
        self.pending_response = None;
        self.pending_tool_calls.clear();
        self.pending_tool_execution = None;
        self.state = AppState::Idle;

        // Add aborted message to chat
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: "*Request aborted*".to_string(),
        });
    }

    pub fn poll_api_response(&mut self) {
        if self.state == AppState::Idle {
            return;
        }

        // Check if cancelled
        if self.cancel_flag.load(Ordering::SeqCst) {
            return;
        }

        let response = match &self.pending_response {
            Some(rx) => match rx.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return, // Still waiting
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Could be cancelled or crashed - check flag
                    if self.cancel_flag.load(Ordering::SeqCst) {
                        return;
                    }
                    self.error = Some("API thread crashed".to_string());
                    self.state = AppState::Idle;
                    self.pending_response = None;
                    return;
                }
            },
            None => return,
        };

        self.pending_response = None;

        match response {
            Ok(resp) => {
                // Update token usage
                if let Some(usage) = &resp.usage {
                    self.token_usage = Some((usage.prompt_tokens, usage.completion_tokens));
                }

                if let Some(tool_calls) = resp.tool_calls {
                    self.handle_tool_calls(tool_calls);
                    // process_pending_tools will call start_api_call when done
                } else {
                    let content = resp.content.unwrap_or_default();
                    self.messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content: content.clone(),
                    });
                    self.api_messages.push(json!({
                        "role": "assistant",
                        "content": content
                    }));
                    self.state = AppState::Idle;
                    self.save_session();
                }
            }
            Err(e) => {
                self.error = Some(format!("API error: {}", e));
                self.api_messages.pop();
                self.state = AppState::Idle;
            }
        }
    }

    fn handle_tool_calls(&mut self, tool_calls: Vec<Value>) {
        let calls: Vec<_> = tool_calls
            .iter()
            .map(|call| {
                (
                    call["id"].as_str().unwrap_or("").to_string(),
                    call["function"]["name"].as_str().unwrap_or("").to_string(),
                    call["function"]["arguments"]
                        .as_str()
                        .unwrap_or("{}")
                        .to_string(),
                )
            })
            .collect();

        self.api_messages.push(json!({
            "role": "assistant",
            "content": Value::Null,
            "tool_calls": tool_calls
        }));

        // Store pending calls and process them
        self.pending_tool_calls = calls;
        self.process_pending_tools();
    }

    fn process_pending_tools(&mut self) {
        // If already executing a tool, wait for it
        if self.pending_tool_execution.is_some() {
            return;
        }

        // Get next tool to execute
        let Some((id, name, args)) = self.pending_tool_calls.first().cloned() else {
            // No more tools, continue with API call
            self.state = AppState::Thinking;
            self.start_api_call();
            return;
        };

        // Check if bash tool needs permission
        if name == "bash" {
            if let Some(modal) = self.check_bash_permission(&args, &id) {
                self.permission_modal = Some(modal);
                return; // Wait for user response
            }
        }

        // Remove from pending and start execution
        self.pending_tool_calls.remove(0);
        self.state = AppState::ToolCall(format_tool_call(&name, &args));

        // Extract path from args for tools that have it
        let path = serde_json::from_str::<Value>(&args)
            .ok()
            .and_then(|v| v["path"].as_str().map(|s| s.to_string()));

        // Spawn tool execution in background
        let (tx, rx) = mpsc::channel();
        self.pending_tool_execution = Some(rx);

        let allowed_paths = self.get_all_allowed_paths();
        let name_clone = name.clone();
        let args_clone = args.clone();

        thread::spawn(move || {
            let result = if name_clone == "bash" {
                tools::execute_bash_with_paths(&args_clone, &allowed_paths)
            } else {
                // For non-bash tools, we need to call them directly
                // since we can't send the function pointer across threads
                tools::execute_tool_by_name(&name_clone, &args_clone)
            };

            let _ = tx.send(ToolExecutionResult {
                id,
                name: name_clone,
                path,
                result,
            });
        });
    }

    pub fn poll_tool_result(&mut self) {
        let rx = match &self.pending_tool_execution {
            Some(rx) => rx,
            None => return,
        };

        match rx.try_recv() {
            Ok(tool_result) => {
                self.pending_tool_execution = None;

                self.messages.push(ChatMessage {
                    role: MessageRole::Tool {
                        name: tool_result.name,
                        path: tool_result.path,
                    },
                    content: tool_result.result.clone(),
                });

                self.api_messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_result.id,
                    "content": tool_result.result
                }));

                // Process next tool or start API call
                self.process_pending_tools();
            }
            Err(mpsc::TryRecvError::Empty) => {
                // Still executing, keep waiting
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                // Thread crashed
                self.pending_tool_execution = None;
                self.error = Some("Tool execution thread crashed".to_string());
                self.state = AppState::Idle;
            }
        }
    }

    fn check_bash_permission(&self, args: &str, tool_id: &str) -> Option<PermissionModal> {
        let json: Value = serde_json::from_str(args).unwrap_or_default();
        let command = json["command"].as_str().unwrap_or("");

        if command.is_empty() {
            return None;
        }

        // Get missing paths considering temp allowed paths
        let missing = self.get_missing_paths_for_command(command);

        if let Some(first_missing) = missing.first() {
            Some(PermissionModal::new(
                first_missing.path.clone(),
                first_missing.reason.clone(),
                tool_id.to_string(),
            ))
        } else {
            None
        }
    }

    fn get_missing_paths_for_command(&self, command: &str) -> Vec<sandbox::PathRequest> {
        let config = SandboxConfig::load_merged();
        let required = sandbox::detect_required_paths(command);

        required
            .into_iter()
            .filter(|req| {
                let req_path = std::path::Path::new(&req.path);
                // Check both config and temp allowed paths
                !config.allowed_paths.iter().any(|allowed| {
                    let allowed_path = std::path::Path::new(allowed);
                    req_path.starts_with(allowed_path) || allowed_path.starts_with(req_path)
                }) && !self.temp_allowed_paths.iter().any(|allowed| {
                    let allowed_path = std::path::Path::new(allowed);
                    req_path.starts_with(allowed_path) || allowed_path.starts_with(req_path)
                })
            })
            .collect()
    }

    fn get_all_allowed_paths(&self) -> Vec<String> {
        let mut paths = sandbox::get_allowed_paths();
        paths.extend(self.temp_allowed_paths.clone());
        paths
    }

    pub fn modal_up(&mut self) {
        if let Some(modal) = &mut self.permission_modal {
            if modal.selected > 0 {
                modal.selected -= 1;
            }
        }
    }

    pub fn modal_down(&mut self) {
        if let Some(modal) = &mut self.permission_modal {
            if modal.selected + 1 < modal.options.len() {
                modal.selected += 1;
            }
        }
    }

    pub fn modal_select(&mut self) {
        let modal = match self.permission_modal.take() {
            Some(m) => m,
            None => return,
        };

        match modal.selected {
            0 => {
                // Allow for project
                if let Err(e) = SandboxConfig::add_path_project(&modal.path) {
                    self.error = Some(format!("Failed to save: {}", e));
                }
            }
            1 => {
                // Allow globally
                if let Err(e) = SandboxConfig::add_path_global(&modal.path) {
                    self.error = Some(format!("Failed to save: {}", e));
                }
            }
            2 => {
                // Allow once (temp)
                self.temp_allowed_paths.push(modal.path.clone());
            }
            3 => {
                // Deny - return error to the tool
                let result = format!("Permission denied: access to {} was not granted", modal.path);
                self.messages.push(ChatMessage {
                    role: MessageRole::Tool { name: "bash".to_string(), path: None },
                    content: result.clone(),
                });
                self.api_messages.push(json!({
                    "role": "tool",
                    "tool_call_id": modal.pending_tool_id,
                    "content": result
                }));
                // Remove the denied call from pending
                if !self.pending_tool_calls.is_empty() {
                    self.pending_tool_calls.remove(0);
                }
                // Continue with remaining tools or finish
                if self.pending_tool_calls.is_empty() {
                    self.state = AppState::Thinking;
                    self.start_api_call();
                } else {
                    self.process_pending_tools();
                }
                return;
            }
            _ => return,
        }

        // Permission granted - continue processing (might need more permissions)
        self.process_pending_tools();
    }

    pub fn modal_cancel(&mut self) {
        // Treat cancel as deny
        if self.permission_modal.is_some() {
            // Set selected to Deny and call modal_select
            if let Some(modal) = &mut self.permission_modal {
                modal.selected = 3; // Deny
            }
            self.modal_select();
        }
    }

    pub fn has_modal(&self) -> bool {
        self.permission_modal.is_some()
    }

    pub fn insert_char(&mut self, c: char) {
        // Don't allow duplicate trigger chars while picker is active
        if (c == '@' && self.picker_mode == PickerMode::Files)
            || (c == '/' && self.picker_mode == PickerMode::Commands)
        {
            return;
        }

        self.input.insert(self.input_cursor, c);
        self.input_cursor += c.len_utf8();

        if c == '@' {
            self.activate_picker(PickerMode::Files);
        } else if c == '/' && self.input_cursor == 1 {
            // Only trigger command picker if / is at start of input
            self.activate_picker(PickerMode::Commands);
        } else if self.picker_mode != PickerMode::None {
            self.picker_query.push(c);
            self.update_picker_results();
        }
    }

    pub fn delete_char(&mut self) {
        if self.input_cursor > 0 {
            // Find the start of the previous character
            let prev_start = self.input[..self.input_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            let removed = self.input.remove(prev_start);
            self.input_cursor = prev_start;

            if self.picker_mode != PickerMode::None {
                let trigger = match self.picker_mode {
                    PickerMode::Files => '@',
                    PickerMode::Commands => '/',
                    PickerMode::None => unreachable!(),
                };
                // Close picker if we deleted the trigger character
                if removed == trigger {
                    self.deactivate_picker();
                } else {
                    self.picker_query.pop();
                    self.update_picker_results();
                }
            }
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor = self.input[..self.input_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.input_cursor < self.input.len() {
            self.input_cursor = self.input[self.input_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.input_cursor + i)
                .unwrap_or(self.input.len());
        }
    }

    pub fn history_up(&mut self) {
        if self.picker_mode != PickerMode::None {
            if self.picker_selected > 0 {
                self.picker_selected -= 1;
            }
        } else if !self.history.is_empty() && self.history_pos > 0 {
            if self.history_pos == self.history.len() {
                self.saved_input = self.input.clone();
            }
            self.history_pos -= 1;
            self.input = self.history[self.history_pos].clone();
            self.input_cursor = self.input.len();
        }
    }

    pub fn history_down(&mut self) {
        if self.picker_mode != PickerMode::None {
            if self.picker_selected + 1 < self.picker_results.len() {
                self.picker_selected += 1;
            }
        } else if self.history_pos < self.history.len() {
            self.history_pos += 1;
            if self.history_pos == self.history.len() {
                self.input = self.saved_input.clone();
            } else {
                self.input = self.history[self.history_pos].clone();
            }
            self.input_cursor = self.input.len();
        }
    }

    pub fn select_picker_item(&mut self) {
        if self.picker_mode == PickerMode::None {
            return;
        }

        if let Some(item) = self.picker_results.get(self.picker_selected).cloned() {
            match self.picker_mode {
                PickerMode::Files => {
                    // Find the @ position and replace from there
                    if let Some(at_pos) = self.input[..self.input_cursor].rfind('@') {
                        self.input.replace_range(at_pos.., &format!("@{}", item));
                        self.input_cursor = at_pos + 1 + item.len();
                    }
                }
                PickerMode::Commands => {
                    // Replace entire input with selected command
                    self.input = format!("/{}", item);
                    self.input_cursor = self.input.len();
                }
                PickerMode::None => {}
            }
        }
        self.deactivate_picker();
    }

    pub fn cancel_picker(&mut self) {
        if self.picker_mode != PickerMode::None {
            let trigger = match self.picker_mode {
                PickerMode::Files => '@',
                PickerMode::Commands => '/',
                PickerMode::None => unreachable!(),
            };
            // Remove the trigger char and query
            if let Some(pos) = self.input[..self.input_cursor].rfind(trigger) {
                self.input.truncate(pos);
                self.input_cursor = pos;
            }
            self.deactivate_picker();
        } else {
            self.input.clear();
            self.input_cursor = 0;
        }
    }

    fn activate_picker(&mut self, mode: PickerMode) {
        self.picker_mode = mode;
        self.picker_query.clear();
        self.picker_selected = 0;
        self.update_picker_results();
    }

    fn deactivate_picker(&mut self) {
        self.picker_mode = PickerMode::None;
        self.picker_query.clear();
        self.picker_results.clear();
        self.picker_selected = 0;
    }

    fn update_picker_results(&mut self) {
        let items: Vec<String> = match self.picker_mode {
            PickerMode::Files => self.files_cache.get_or_insert_with(load_files).clone(),
            PickerMode::Commands => get_commands(),
            PickerMode::None => return,
        };
        self.picker_results = filter_items(&items, &self.picker_query, MAX_PICKER_ITEMS);
        self.picker_selected = self.picker_selected.min(self.picker_results.len().saturating_sub(1));
    }

    pub fn picker_active(&self) -> bool {
        self.picker_mode != PickerMode::None
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(3);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(3);
    }

    pub fn paste(&mut self, text: &str) {
        if self.picker_mode != PickerMode::None {
            self.deactivate_picker();
        }
        let cleaned = text.replace('\n', " ").replace('\r', "");
        self.input.insert_str(self.input_cursor, &cleaned);
        self.input_cursor += cleaned.len();
    }
}

fn get_system_prompt(mode: &Mode) -> &'static str {
    match mode {
        Mode::Coding => {
            "You are a coding agent with file access. Be concise. Use grep to locate code, then read specific line ranges when needed. When you complete a task using tools, briefly state what you did and stop. The user can see all tool outputs including file diffs, so never repeat code in markdown blocks after writing files. For build commands (cargo build, npm run, etc.), use `2>&1 | tail -30` by default. If you need to find specific errors in verbose output, use `2>&1 | grep -i error` instead."
        }
        Mode::Coach => {
            "You are a productivity coach. Track projects in projects.md. Give practical advice and encouragement."
        }
    }
}

struct ExpandedInput {
    text: String,
    files_read: Vec<(String, usize)>, // (path, line_count)
}

fn expand_file_refs(input: &str) -> ExpandedInput {
    let mut result = input.to_string();
    let mut files_content = Vec::new();
    let mut files_read = Vec::new();

    for word in input.split_whitespace() {
        if word.starts_with('@') && word.len() > 1 {
            let path_str = &word[1..];
            let path = Path::new(path_str);

            if path.exists() && path.is_file() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    let line_count = content.lines().count();
                    result = result.replace(word, &format!("`{}`", path_str));
                    files_content.push(format!(
                        "\n\n<file path=\"{}\">\n{}\n</file>",
                        path_str,
                        content.trim()
                    ));
                    files_read.push((path_str.to_string(), line_count));
                }
            }
        }
    }

    if !files_content.is_empty() {
        result.push_str(&files_content.join(""));
    }

    ExpandedInput {
        text: result,
        files_read,
    }
}

fn format_tool_call(name: &str, args: &str) -> String {
    let json: Value = serde_json::from_str(args).unwrap_or_default();

    match name {
        "read_file" => {
            let path = json["path"].as_str().unwrap_or("?");
            format!("read {}", path)
        }
        "write_file" => {
            let path = json["path"].as_str().unwrap_or("?");
            format!("write {}", path)
        }
        "list_dir" => {
            let path = json["path"].as_str().unwrap_or(".");
            format!("ls {}", path)
        }
        "search_files" => {
            let pattern = json["pattern"].as_str().unwrap_or("*");
            let path = json["path"].as_str().unwrap_or(".");
            format!("search {} in {}", pattern, path)
        }
        "bash" => {
            let cmd = json["command"].as_str().unwrap_or("?");
            // Truncate long commands
            if cmd.len() > 40 {
                format!("$ {}...", &cmd[..40])
            } else {
                format!("$ {}", cmd)
            }
        }
        "view_projects" => "view projects".to_string(),
        "update_projects" => "update projects".to_string(),
        _ => name.to_string(),
    }
}

fn load_files() -> Vec<String> {
    use ignore::WalkBuilder;

    let mut builder = WalkBuilder::new(".");
    builder
        .hidden(false)
        .ignore(true)
        .git_ignore(true)
        .max_depth(Some(6))
        .add_custom_ignore_filename(".vecoignore");

    builder
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| {
            let p = e.path().to_string_lossy().to_string();
            p.strip_prefix("./").unwrap_or(&p).to_string()
        })
        .collect()
}

fn filter_items(items: &[String], query: &str, max: usize) -> Vec<String> {
    let query_lower = query.to_lowercase();
    items
        .iter()
        .filter(|item| {
            if query_lower.is_empty() {
                true
            } else {
                let item_lower = item.to_lowercase();
                let mut chars = query_lower.chars().peekable();
                for c in item_lower.chars() {
                    if chars.peek() == Some(&c) {
                        chars.next();
                    }
                }
                chars.peek().is_none()
            }
        })
        .take(max)
        .cloned()
        .collect()
}

fn get_commands() -> Vec<String> {
    vec![
        "clear".to_string(),
        "sessions".to_string(),
        "load".to_string(),
        "provider".to_string(),
        "key".to_string(),
        "help".to_string(),
        "quit".to_string(),
    ]
}

const HELP_TEXT: &str = r#"**Commands:**
- `/clear` - Save and start new session
- `/sessions` - List saved sessions
- `/load <id>` - Load a saved session
- `/provider` - List providers
- `/provider <name>` - Switch provider
- `/key <key>` - Set API key for current provider
- `/quit` - Exit (also /exit, /q)
- `/help` - Show this help

**File references:**
- `@` - Type @ to open file picker
- `Tab/Enter` - Select file from picker
- `Esc` - Cancel picker

**Navigation:**
- `↑/↓` - History / picker navigation
- `Ctrl+U/D` - Scroll chat history"#;
