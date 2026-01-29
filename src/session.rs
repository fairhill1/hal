use crate::app::ChatMessage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub title: String,
    pub messages: Vec<ChatMessage>,
    pub api_messages: Vec<Value>,
}

impl Session {
    pub fn new() -> Self {
        let now = chrono::Utc::now().timestamp();
        Session {
            id: format!("{}", now),
            created_at: now,
            updated_at: now,
            title: String::new(),
            messages: Vec::new(),
            api_messages: Vec::new(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = sessions_dir().join(format!("{}.json", self.id));

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let content = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, content).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load(id: &str) -> Result<Self, String> {
        let path = sessions_dir().join(format!("{}.json", id));
        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())
    }
}

pub fn sessions_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hal")
        .join("sessions")
}

pub fn list_sessions() -> Vec<Session> {
    let dir = sessions_dir();

    if !dir.exists() {
        return Vec::new();
    }

    let mut sessions: Vec<Session> = fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|e| {
            let content = fs::read_to_string(e.path()).ok()?;
            serde_json::from_str(&content).ok()
        })
        .collect();

    // Sort by updated_at descending (most recent first)
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

pub fn get_latest_session() -> Option<Session> {
    list_sessions().into_iter().next()
}
