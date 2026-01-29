use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxConfig {
    #[serde(default)]
    pub allowed_paths: Vec<String>,
}

impl SandboxConfig {
    pub fn load_merged() -> Self {
        let global = Self::load_global();
        let project = Self::load_project();

        let mut paths: HashSet<String> = HashSet::new();
        paths.extend(global.allowed_paths);
        paths.extend(project.allowed_paths);

        SandboxConfig {
            allowed_paths: paths.into_iter().collect(),
        }
    }

    pub fn load_global() -> Self {
        let path = global_config_path();
        Self::load_from(&path).unwrap_or_default()
    }

    pub fn load_project() -> Self {
        let path = project_config_path();
        Self::load_from(&path).unwrap_or_default()
    }

    fn load_from(path: &Path) -> Option<Self> {
        let content = fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub fn save_global(&self) -> Result<(), String> {
        let path = global_config_path();
        self.save_to(&path)
    }

    pub fn save_project(&self) -> Result<(), String> {
        let path = project_config_path();
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let content = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(path, content).map_err(|e| e.to_string())
    }

    pub fn add_path_global(path: &str) -> Result<(), String> {
        let mut config = Self::load_global();
        let expanded = expand_path(path);
        if !config.allowed_paths.contains(&expanded) {
            config.allowed_paths.push(expanded);
        }
        config.save_global()
    }

    pub fn add_path_project(path: &str) -> Result<(), String> {
        let mut config = Self::load_project();
        let expanded = expand_path(path);
        if !config.allowed_paths.contains(&expanded) {
            config.allowed_paths.push(expanded);
        }
        config.save_project()
    }
}

fn global_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hal")
        .join("sandbox.json")
}

fn project_config_path() -> PathBuf {
    PathBuf::from(".hal").join("sandbox.json")
}

fn expand_path(path: &str) -> String {
    if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(&path[2..]).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

/// Detect paths that a command might need based on common tools
pub fn detect_required_paths(command: &str) -> Vec<PathRequest> {
    let mut requests = Vec::new();
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));

    // Rust/Cargo
    if command.contains("cargo") || command.contains("rustc") || command.contains("rustup") {
        let cargo_home = std::env::var("CARGO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".cargo"));
        let rustup_home = std::env::var("RUSTUP_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".rustup"));

        requests.push(PathRequest {
            path: cargo_home.to_string_lossy().to_string(),
            reason: "Cargo needs access to ~/.cargo for toolchain and crates".to_string(),
        });
        requests.push(PathRequest {
            path: rustup_home.to_string_lossy().to_string(),
            reason: "Rust needs access to ~/.rustup for toolchain".to_string(),
        });
    }

    // Node.js/npm/yarn/pnpm
    if command.contains("npm")
        || command.contains("yarn")
        || command.contains("pnpm")
        || command.contains("node")
        || command.contains("npx")
    {
        requests.push(PathRequest {
            path: home.join(".npm").to_string_lossy().to_string(),
            reason: "npm needs access to ~/.npm for cache".to_string(),
        });
        requests.push(PathRequest {
            path: home.join(".node").to_string_lossy().to_string(),
            reason: "Node needs access to ~/.node".to_string(),
        });
        if let Ok(prefix) = std::env::var("NVM_DIR") {
            requests.push(PathRequest {
                path: prefix,
                reason: "Node version manager directory".to_string(),
            });
        } else {
            requests.push(PathRequest {
                path: home.join(".nvm").to_string_lossy().to_string(),
                reason: "nvm needs access to ~/.nvm".to_string(),
            });
        }
    }

    // Go (careful not to match "cargo")
    if command.starts_with("go ") || command.contains(" go ") {
        let gopath = std::env::var("GOPATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join("go"));
        requests.push(PathRequest {
            path: gopath.to_string_lossy().to_string(),
            reason: "Go needs access to GOPATH".to_string(),
        });
    }

    // Python
    if command.contains("python")
        || command.contains("pip")
        || command.contains("poetry")
        || command.contains("uv")
    {
        requests.push(PathRequest {
            path: home.join(".local").to_string_lossy().to_string(),
            reason: "Python tools often install to ~/.local".to_string(),
        });
        requests.push(PathRequest {
            path: home.join(".cache/pip").to_string_lossy().to_string(),
            reason: "pip cache directory".to_string(),
        });
        requests.push(PathRequest {
            path: home.join(".cache/uv").to_string_lossy().to_string(),
            reason: "uv cache directory".to_string(),
        });
    }

    // Homebrew (macOS)
    if command.contains("brew") {
        requests.push(PathRequest {
            path: "/opt/homebrew".to_string(),
            reason: "Homebrew installation directory".to_string(),
        });
        requests.push(PathRequest {
            path: "/usr/local".to_string(),
            reason: "Homebrew installation directory (Intel Mac)".to_string(),
        });
    }

    // Git (usually fine but might need .gitconfig)
    if command.contains("git") {
        requests.push(PathRequest {
            path: home.join(".gitconfig").to_string_lossy().to_string(),
            reason: "Git configuration file".to_string(),
        });
        requests.push(PathRequest {
            path: home.join(".ssh").to_string_lossy().to_string(),
            reason: "SSH keys for git authentication".to_string(),
        });
    }

    requests
}

#[derive(Debug, Clone)]
pub struct PathRequest {
    pub path: String,
    pub reason: String,
}

/// Get paths that are needed but not yet allowed
#[allow(dead_code)]
pub fn get_missing_paths(command: &str) -> Vec<PathRequest> {
    let config = SandboxConfig::load_merged();
    let required = detect_required_paths(command);

    required
        .into_iter()
        .filter(|req| {
            let req_path = Path::new(&req.path);
            !config.allowed_paths.iter().any(|allowed| {
                let allowed_path = Path::new(allowed);
                req_path.starts_with(allowed_path) || allowed_path.starts_with(req_path)
            })
        })
        .collect()
}

/// Build sandbox profile paths from config
pub fn get_allowed_paths() -> Vec<String> {
    SandboxConfig::load_merged().allowed_paths
}
