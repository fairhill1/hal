use crate::config::Mode;
use serde_json::{json, Value};
use similar::{ChangeTag, TextDiff};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn get_tool_definitions(mode: &Mode) -> Vec<Value> {
    match mode {
        Mode::Coding => vec![
            json!({
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read the contents of a file. Use start_line/end_line to read specific sections (line numbers from grep output).",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "File path to read" },
                            "start_line": { "type": "integer", "description": "Starting line number, 1-indexed (optional)" },
                            "end_line": { "type": "integer", "description": "Ending line number, inclusive (optional)" }
                        },
                        "required": ["path"]
                    }
                }
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "write_file",
                    "description": "Write content to a file (creates or overwrites)",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "File path to write" },
                            "content": { "type": "string", "description": "Content to write" }
                        },
                        "required": ["path", "content"]
                    }
                }
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "edit_file",
                    "description": "Replace a specific string in a file. Use for small edits instead of rewriting the whole file.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "File path to edit" },
                            "old": { "type": "string", "description": "Exact text to find (must match exactly)" },
                            "new": { "type": "string", "description": "Text to replace it with" }
                        },
                        "required": ["path", "old", "new"]
                    }
                }
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "list_dir",
                    "description": "List files and directories in a path",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "Directory path (default: current dir)" }
                        },
                        "required": []
                    }
                }
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "search_files",
                    "description": "Search for files matching a pattern",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "pattern": { "type": "string", "description": "Glob pattern (e.g., '*.rs', '**/*.json')" },
                            "path": { "type": "string", "description": "Starting directory (default: current dir)" }
                        },
                        "required": ["pattern"]
                    }
                }
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "grep",
                    "description": "Search file contents for a pattern. Returns matching lines with path and line number. Prefer this over read_file - refine your pattern if results are too broad rather than reading full files.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "pattern": { "type": "string", "description": "Regex pattern to search for" },
                            "path": { "type": "string", "description": "File or directory to search (default: current dir)" },
                            "context": { "type": "integer", "description": "Lines of context around matches (default: 2)" }
                        },
                        "required": ["pattern"]
                    }
                }
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "bash",
                    "description": "Run a bash command (sandboxed to current directory). Use for git, build tools, package managers, etc.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "command": { "type": "string", "description": "The bash command to execute" }
                        },
                        "required": ["command"]
                    }
                }
            }),
        ],
        Mode::Coach => vec![
            json!({
                "type": "function",
                "function": {
                    "name": "view_projects",
                    "description": "View the current projects.md file",
                    "parameters": {
                        "type": "object",
                        "properties": {},
                        "required": []
                    }
                }
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "update_projects",
                    "description": "Update the projects.md file with new content",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string", "description": "New markdown content for projects.md" }
                        },
                        "required": ["content"]
                    }
                }
            }),
        ],
    }
}

/// Execute a tool by name without needing a function pointer map
/// Used for async tool execution where we can't send function pointers across threads
pub fn execute_tool_by_name(name: &str, args_str: &str) -> String {
    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

    match name {
        "read_file" => tool_read_file(&args),
        "write_file" => tool_write_file(&args),
        "edit_file" => tool_edit_file(&args),
        "list_dir" => tool_list_dir(&args),
        "search_files" => tool_search_files(&args),
        "grep" => tool_grep(&args),
        "bash" => tool_bash(&args),
        "view_projects" => tool_view_projects(&args),
        "update_projects" => tool_update_projects(&args),
        _ => format!("Unknown tool: {}", name),
    }
}

/// Preview a write_file without actually writing. Returns (diff_text, new_content).
pub fn preview_write_file(args_str: &str) -> Result<(String, String), String> {
    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
    let path = args["path"].as_str().unwrap_or("");
    let content = args["content"].as_str().unwrap_or("");

    if path.is_empty() {
        return Err("Error: path is required".to_string());
    }

    let old_content = fs::read_to_string(path).unwrap_or_default();
    let is_new = old_content.is_empty() && !Path::new(path).exists();

    let diff = if is_new {
        let mut output = format!("Created {}\n", path);
        for (i, line) in content.lines().enumerate() {
            output.push_str(&format!("+{:>4}│{}\n", i + 1, line));
        }
        output
    } else if old_content == content {
        format!("No changes to {}", path)
    } else {
        format_diff_with_context(path, "Wrote", &old_content, content)
    };

    Ok((diff, content.to_string()))
}

/// Preview an edit_file without actually writing. Returns (diff_text, new_content).
pub fn preview_edit_file(args_str: &str) -> Result<(String, String), String> {
    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
    let path = args["path"].as_str().unwrap_or("");
    let old = args["old"].as_str().unwrap_or("");
    let new = args["new"].as_str().unwrap_or("");

    if path.is_empty() {
        return Err("Error: path is required".to_string());
    }
    if old.is_empty() {
        return Err("Error: old text is required".to_string());
    }

    let content = fs::read_to_string(path).map_err(|e| format!("Error reading file: {}", e))?;

    let count = content.matches(old).count();
    if count == 0 {
        return Err(format!("Error: text not found in {}", path));
    }
    if count > 1 {
        return Err(format!("Error: text appears {} times in {} - be more specific", count, path));
    }

    let updated = content.replacen(old, new, 1);
    let diff_text = format_diff_with_context(path, "Edited", &content, &updated);

    Ok((diff_text, updated))
}

/// Format a unified diff with 3 lines of context and line numbers.
/// Each line is formatted as: <marker><line_num_4_chars>│<code>
/// Hunks are separated by "···" lines.
fn format_diff_with_context(path: &str, action: &str, old_content: &str, new_content: &str) -> String {
    let diff = similar::TextDiff::from_lines(old_content, new_content);
    let mut output = format!("{} {}\n", action, path);

    for (group_idx, group) in diff.grouped_ops(3).iter().enumerate() {
        if group_idx > 0 {
            output.push_str("···\n");
        }
        for op in group {
            for change in diff.iter_changes(op) {
                let (marker, line_num) = match change.tag() {
                    similar::ChangeTag::Equal => (' ', change.new_index().unwrap_or(0) + 1),
                    similar::ChangeTag::Delete => ('-', change.old_index().unwrap_or(0) + 1),
                    similar::ChangeTag::Insert => ('+', change.new_index().unwrap_or(0) + 1),
                };
                let value = change.value();
                output.push_str(&format!("{}{:>4}│{}", marker, line_num, value));
                if change.missing_newline() {
                    output.push('\n');
                }
            }
        }
    }

    output
}

/// Apply a previewed write (used after user accepts).
pub fn apply_write(path: &str, content: &str) -> String {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(parent) {
                return format!("Error creating directories: {}", e);
            }
        }
    }
    match fs::write(path, content) {
        Ok(_) => format!("Wrote {}", path),
        Err(e) => format!("Error writing file: {}", e),
    }
}

// Coding tools

fn tool_read_file(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or("");
    if path.is_empty() {
        return "Error: path is required".to_string();
    }

    let start_line = args["start_line"].as_u64().map(|n| n as usize);
    let end_line = args["end_line"].as_u64().map(|n| n as usize);

    match fs::read_to_string(path) {
        Ok(content) => {
            match (start_line, end_line) {
                (Some(start), Some(end)) => {
                    let lines: Vec<&str> = content.lines().collect();
                    let start_idx = start.saturating_sub(1); // Convert to 0-indexed
                    let end_idx = end.min(lines.len());
                    if start_idx >= lines.len() {
                        return format!("Error: start_line {} exceeds file length ({})", start, lines.len());
                    }
                    lines[start_idx..end_idx]
                        .iter()
                        .enumerate()
                        .map(|(i, line)| format!("{}: {}", start_idx + i + 1, line))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                (Some(start), None) => {
                    let lines: Vec<&str> = content.lines().collect();
                    let start_idx = start.saturating_sub(1);
                    if start_idx >= lines.len() {
                        return format!("Error: start_line {} exceeds file length ({})", start, lines.len());
                    }
                    lines[start_idx..]
                        .iter()
                        .enumerate()
                        .map(|(i, line)| format!("{}: {}", start_idx + i + 1, line))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                (None, Some(end)) => {
                    let lines: Vec<&str> = content.lines().collect();
                    let end_idx = end.min(lines.len());
                    lines[..end_idx]
                        .iter()
                        .enumerate()
                        .map(|(i, line)| format!("{}: {}", i + 1, line))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                (None, None) => content,
            }
        }
        Err(e) => format!("Error reading file: {}", e),
    }
}

fn tool_write_file(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or("");
    let content = args["content"].as_str().unwrap_or("");

    if path.is_empty() {
        return "Error: path is required".to_string();
    }

    // Read existing content for diff
    let old_content = fs::read_to_string(path).unwrap_or_default();
    let is_new_file = old_content.is_empty() && !Path::new(path).exists();

    // Create parent directories if needed
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(parent) {
                return format!("Error creating directories: {}", e);
            }
        }
    }

    match fs::write(path, content) {
        Ok(_) => {
            if is_new_file {
                let mut output = format!("Created {}\n", path);
                for line in content.lines() {
                    output.push_str(&format!("+{}\n", line));
                }
                output
            } else {
                format_diff(path, &old_content, content)
            }
        }
        Err(e) => format!("Error writing file: {}", e),
    }
}

fn format_diff(path: &str, old: &str, new: &str) -> String {
    if old == new {
        return format!("No changes to {}", path);
    }

    let diff = TextDiff::from_lines(old, new);
    let mut output = format!("Wrote {}\n", path);

    for change in diff.iter_all_changes() {
        let prefix = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => continue,
        };
        output.push_str(&format!("{}{}", prefix, change));
    }

    output
}

fn tool_edit_file(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or("");
    let old = args["old"].as_str().unwrap_or("");
    let new = args["new"].as_str().unwrap_or("");

    if path.is_empty() {
        return "Error: path is required".to_string();
    }
    if old.is_empty() {
        return "Error: old text is required".to_string();
    }

    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return format!("Error reading file: {}", e),
    };

    let count = content.matches(old).count();
    if count == 0 {
        return format!("Error: text not found in {}", path);
    }
    if count > 1 {
        return format!("Error: text appears {} times in {} - be more specific", count, path);
    }

    let updated = content.replacen(old, new, 1);
    match fs::write(path, &updated) {
        Ok(_) => {
            let old_lines: Vec<&str> = old.lines().collect();
            let new_lines: Vec<&str> = new.lines().collect();
            let mut output = format!("Edited {}\n", path);
            for line in old_lines {
                output.push_str(&format!("-{}\n", line));
            }
            for line in new_lines {
                output.push_str(&format!("+{}\n", line));
            }
            output
        }
        Err(e) => format!("Error writing file: {}", e),
    }
}

fn tool_list_dir(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or(".");

    match fs::read_dir(path) {
        Ok(entries) => {
            let mut items: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if e.path().is_dir() {
                        format!("{}/", name)
                    } else {
                        name
                    }
                })
                .collect();
            items.sort();
            items.join("\n")
        }
        Err(e) => format!("Error listing directory: {}", e),
    }
}

fn tool_search_files(args: &Value) -> String {
    let pattern = args["pattern"].as_str().unwrap_or("");
    let base_path = args["path"].as_str().unwrap_or(".");

    if pattern.is_empty() {
        return "Error: pattern is required".to_string();
    }

    let mut results = Vec::new();
    search_recursive(Path::new(base_path), pattern, &mut results);

    if results.is_empty() {
        "No files found".to_string()
    } else {
        results.join("\n")
    }
}

fn search_recursive(dir: &Path, pattern: &str, results: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();

        // Skip hidden directories
        if path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }

        if path.is_dir() {
            search_recursive(&path, pattern, results);
        } else if matches_pattern(&path, pattern) {
            results.push(path.to_string_lossy().to_string());
        }
    }
}

fn matches_pattern(path: &Path, pattern: &str) -> bool {
    let name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let path_str = path.to_string_lossy();

    // Extract filename pattern (after last /)
    let file_pattern = pattern.rsplit('/').next().unwrap_or(pattern);

    // Check if pattern has directory component
    let dir_pattern = if pattern.contains('/') {
        Some(pattern.rsplitn(2, '/').nth(1).unwrap_or(""))
    } else {
        None
    };

    // Match directory part if specified (skip for ** which matches any)
    if let Some(dir) = dir_pattern {
        if !dir.is_empty() && dir != "**" && !dir.ends_with("**") {
            // Check if path contains the directory
            if !path_str.contains(&format!("{}/", dir)) && !path_str.starts_with(&format!("{}/", dir)) {
                return false;
            }
        }
    }

    // Match filename
    match_glob(name, file_pattern)
}

fn match_glob(name: &str, pattern: &str) -> bool {
    glob::Pattern::new(pattern)
        .map(|p| p.matches(name))
        .unwrap_or(false)
}

fn tool_grep(args: &Value) -> String {
    let pattern = args["pattern"].as_str().unwrap_or("");
    let path = args["path"].as_str().unwrap_or(".");
    let context = args["context"].as_i64().unwrap_or(2) as usize;

    if pattern.is_empty() {
        return "Error: pattern is required".to_string();
    }

    let regex = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => return format!("Error: invalid regex: {}", e),
    };

    let mut results = Vec::new();
    grep_recursive(Path::new(path), &regex, context, &mut results);

    if results.is_empty() {
        format!("grep '{}': no matches", pattern)
    } else {
        format!("grep '{}':\n{}", pattern, results.join("\n"))
    }
}

fn grep_recursive(path: &Path, regex: &regex::Regex, context: usize, results: &mut Vec<String>) {
    use ignore::WalkBuilder;

    if path.is_file() {
        grep_file(path, regex, context, results);
        return;
    }

    let mut builder = WalkBuilder::new(path);
    builder
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .add_custom_ignore_filename(".vecoignore");

    for entry in builder.build().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            grep_file(path, regex, context, results);
        }
    }
}

fn grep_file(path: &Path, regex: &regex::Regex, context: usize, results: &mut Vec<String>) {
    let Ok(content) = fs::read_to_string(path) else { return };
    let lines: Vec<&str> = content.lines().collect();
    let path_str = path.to_string_lossy();

    let mut shown_ranges: Vec<(usize, usize)> = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        if regex.is_match(line) {
            let start = i.saturating_sub(context);
            let end = (i + context + 1).min(lines.len());

            // Check if this range overlaps with previous
            if let Some(last) = shown_ranges.last_mut() {
                if start <= last.1 {
                    last.1 = end;
                    continue;
                }
            }
            shown_ranges.push((start, end));
        }
    }

    for (start, end) in shown_ranges {
        for i in start..end {
            let prefix = if regex.is_match(lines[i]) { ":" } else { "-" };
            results.push(format!("{}:{}{}  {}", path_str, i + 1, prefix, lines[i]));
        }
        if context > 0 {
            results.push("--".to_string());
        }
    }
}

fn tool_bash(args: &Value) -> String {
    execute_bash_with_paths(&serde_json::to_string(args).unwrap_or_default(), &[])
}

/// Execute bash command with additional allowed paths
pub fn execute_bash_with_paths(args_str: &str, allowed_paths: &[String]) -> String {
    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
    let command = args["command"].as_str().unwrap_or("");
    if command.is_empty() {
        return "Error: command is required".to_string();
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    match run_sandboxed(command, &cwd, allowed_paths) {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mut result = format!("$ {}\n", command);
            if !stdout.is_empty() {
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if result.len() > command.len() + 3 {
                    result.push('\n');
                }
                result.push_str(&stderr);
            }
            if !output.status.success() {
                result.push_str(&format!("\n[exit code: {}]", output.status.code().unwrap_or(-1)));
            }
            result
        }
        Err(e) => format!("$ {}\nError: {}", command, e),
    }
}

/// Execute bash without sandbox (for debugging)
#[allow(dead_code)]
pub fn execute_bash_unsandboxed(args_str: &str) -> String {
    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
    let command = args["command"].as_str().unwrap_or("");
    if command.is_empty() {
        return "Error: command is required".to_string();
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    match Command::new("bash")
        .args(["-c", command])
        .current_dir(&cwd)
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!("{}{}", stdout, stderr)
        }
        Err(e) => format!("Error: {}", e),
    }
}

fn run_sandboxed(command: &str, cwd: &Path, allowed_paths: &[String]) -> std::io::Result<Output> {
    #[cfg(target_os = "macos")]
    return run_sandbox_macos(command, cwd, allowed_paths);

    #[cfg(target_os = "linux")]
    return run_sandbox_linux(command, cwd, allowed_paths);

    #[cfg(target_os = "windows")]
    return run_sandbox_windows(command, cwd, allowed_paths);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Sandboxed bash not supported on this platform",
    ));
}

#[cfg(target_os = "macos")]
fn run_sandbox_macos(command: &str, cwd: &Path, allowed_paths: &[String]) -> std::io::Result<Output> {
    let cwd_str = cwd.to_string_lossy();

    // Build extra write rules for allowed paths
    let extra_write_rules: String = allowed_paths
        .iter()
        .map(|p| format!("(allow file-write* (subpath \"{}\"))", p))
        .collect::<Vec<_>>()
        .join("\n");

    // Sandbox profile:
    // - Allow all reads (tools need access to many system paths)
    // - Restrict writes to: cwd, /tmp, and explicitly allowed paths
    let profile = format!(
        r#"(version 1)
(deny default)
(allow process*)
(allow file-read*)
(allow file-write* (subpath "{}"))
(allow file-write* (subpath "/tmp"))
(allow file-write* (subpath "/private/tmp"))
(allow file-write* (subpath "/var/folders"))
(allow file-write* (subpath "/private/var/folders"))
(allow file-write* (subpath "/dev"))
{}
(allow sysctl-read)
(allow mach-lookup)
(allow signal)
(allow network*)"#,
        cwd_str, extra_write_rules
    );

    Command::new("sandbox-exec")
        .args(["-p", &profile, "bash", "-c", command])
        .current_dir(cwd)
        .output()
}

#[cfg(target_os = "linux")]
fn run_sandbox_linux(command: &str, cwd: &Path, allowed_paths: &[String]) -> std::io::Result<Output> {
    // Try bwrap (bubblewrap) first, fall back to basic execution with warning
    let cwd_str = cwd.to_string_lossy();

    // Check if bwrap is available
    if Command::new("which").arg("bwrap").output()?.status.success() {
        let args = vec![
            "--ro-bind", "/usr", "/usr",
            "--ro-bind", "/bin", "/bin",
            "--ro-bind", "/lib", "/lib",
            "--ro-bind", "/lib64", "/lib64",
            "--ro-bind", "/etc", "/etc",
        ];

        // Add allowed paths as bind mounts
        let path_args: Vec<String> = allowed_paths
            .iter()
            .flat_map(|p| vec!["--bind".to_string(), p.clone(), p.clone()])
            .collect();

        let mut cmd = Command::new("bwrap");
        for arg in &args {
            cmd.arg(arg);
        }
        for arg in &path_args {
            cmd.arg(arg);
        }
        cmd.args([
            "--bind", &cwd_str, &cwd_str,
            "--chdir", &cwd_str,
            "--unshare-all",
            "--share-net",
            "--die-with-parent",
            "bash", "-c", command,
        ]);
        cmd.output()
    } else {
        // Fallback: run without sandbox but restricted to cwd
        // This is less secure but allows basic functionality
        Command::new("bash")
            .args(["-c", command])
            .current_dir(cwd)
            .output()
    }
}

#[cfg(target_os = "windows")]
fn run_sandbox_windows(command: &str, cwd: &Path, _allowed_paths: &[String]) -> std::io::Result<Output> {
    // Windows sandboxing is complex; for now, just run in cwd
    // Future: could use Windows Sandbox API or AppContainer
    Command::new("cmd")
        .args(["/C", command])
        .current_dir(cwd)
        .output()
}

// Coach tools

fn projects_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("hal")
        .join("projects.md")
}

fn tool_view_projects(_args: &Value) -> String {
    let path = projects_path();

    match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => {
            let default = "# Projects\n\n## Active\n\n## Completed\n\n## Ideas\n";
            let _ = fs::write(&path, default);
            default.to_string()
        }
    }
}

fn tool_update_projects(args: &Value) -> String {
    let content = args["content"].as_str().unwrap_or("");
    let path = projects_path();

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    match fs::write(&path, content) {
        Ok(_) => "Projects updated successfully".to_string(),
        Err(e) => format!("Error updating projects: {}", e),
    }
}
