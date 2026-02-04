mod api;
mod app;
mod config;
mod sandbox;
mod session;
mod tools;
mod ui;

use app::{App, AppState};
use config::{Config, Mode};
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io::{self, stdout, BufRead, Write};
use std::time::Duration;

pub fn self_update() -> Result<String, String> {
    let current_version = env!("CARGO_PKG_VERSION");

    // Check latest version from GitHub API
    let api_url = "https://api.github.com/repos/fairhill1/hal/releases/latest";
    let api_response: serde_json::Value = ureq::get(api_url)
        .call()
        .map_err(|e| format!("Failed to check latest version: {}", e))?
        .body_mut()
        .read_json()
        .map_err(|e| format!("Failed to parse release info: {}", e))?;

    let latest_tag = api_response["tag_name"]
        .as_str()
        .ok_or("No tag_name in release")?
        .trim_start_matches('v');

    if latest_tag == current_version {
        return Ok(format!("Already on the latest version (v{}).", current_version));
    }

    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "macos",
        "windows" => "windows",
        other => return Err(format!("Unsupported OS: {}", other)),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(format!("Unsupported architecture: {}", other)),
    };

    let url = format!(
        "https://github.com/fairhill1/hal/releases/latest/download/hal-{}-{}",
        os, arch
    );

    let current_exe = std::env::current_exe().map_err(|e| format!("Failed to get current exe path: {}", e))?;

    let response = ureq::get(&url).call().map_err(|e| format!("Download failed: {}", e))?;

    let body = response
        .into_body()
        .read_to_vec()
        .map_err(|e| format!("Failed to read response: {}", e))?;

    if body.is_empty() {
        return Err("Downloaded file is empty".to_string());
    }

    // Write to a temp file next to the binary, then rename (atomic replace)
    let temp_path = current_exe.with_extension("tmp");
    std::fs::write(&temp_path, &body).map_err(|e| format!("Failed to write temp file: {}", e))?;

    // Set executable permissions on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to set permissions: {}", e))?;
    }

    std::fs::rename(&temp_path, &current_exe).map_err(|e| format!("Failed to replace binary: {}", e))?;

    Ok(format!("Updated v{} → v{}. Restart hal to use the new version.", current_version, latest_tag))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut config = Config::load();
    let mut session_to_load: Option<session::Session> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-c" | "--coach" => {
                config.mode = Mode::Coach;
            }
            "--model" | "-m" => {
                if i + 1 < args.len() {
                    config.default_provider = args[i + 1].clone();
                    i += 1;
                }
            }
            "--resume" | "-r" => {
                session_to_load = session::get_latest_session();
            }
            "--session" | "-s" => {
                if i + 1 < args.len() {
                    match session::Session::load(&args[i + 1]) {
                        Ok(s) => session_to_load = Some(s),
                        Err(e) => {
                            eprintln!("Failed to load session: {}", e);
                            std::process::exit(1);
                        }
                    }
                    i += 1;
                }
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            "update" => {
                match self_update() {
                    Ok(msg) => { println!("{}", msg); return; }
                    Err(e) => { eprintln!("Update failed: {}", e); std::process::exit(1); }
                }
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // Check if the current provider has an API key configured
    let needs_setup = {
        let provider = config.get_provider();
        match provider {
            Some(p) => p.api_key.is_none() && std::env::var(&p.api_key_env).is_err(),
            None => true,
        }
    };

    if needs_setup {
        config = match setup(config) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Setup failed: {}", e);
                std::process::exit(1);
            }
        };
    }

    if let Err(e) = run(config, session_to_load) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn setup(mut config: Config) -> Result<Config, String> {
    println!();
    println!("  Welcome to hal!");
    println!();

    // Collect provider names sorted for stable ordering
    let mut provider_names: Vec<String> = config.providers.keys().cloned().collect();
    provider_names.sort();

    // Put the default provider first
    if let Some(pos) = provider_names.iter().position(|n| n == &config.default_provider) {
        let name = provider_names.remove(pos);
        provider_names.insert(0, name);
    }

    println!("  Select a provider:");
    for (i, name) in provider_names.iter().enumerate() {
        println!("    {}. {}", i + 1, name);
    }
    print!("  > ");
    io::stdout().flush().map_err(|e| e.to_string())?;

    let stdin = io::stdin();
    let choice_line = stdin.lock().lines().next()
        .ok_or_else(|| "No input".to_string())?
        .map_err(|e| e.to_string())?;

    let choice: usize = choice_line.trim().parse()
        .map_err(|_| "Invalid choice".to_string())?;

    if choice == 0 || choice > provider_names.len() {
        return Err("Invalid choice".to_string());
    }

    let selected_name = &provider_names[choice - 1];
    config.default_provider = selected_name.clone();

    let env_var = config.providers.get(selected_name)
        .map(|p| p.api_key_env.clone())
        .unwrap_or_default();

    println!();
    print!("  Enter your {} API key (or set ${}): ", selected_name, env_var);
    io::stdout().flush().map_err(|e| e.to_string())?;

    let key_line = stdin.lock().lines().next()
        .ok_or_else(|| "No input".to_string())?
        .map_err(|e| e.to_string())?;

    let key = key_line.trim().to_string();
    if key.is_empty() {
        return Err("API key cannot be empty".to_string());
    }

    // Store the key in the provider config
    if let Some(provider) = config.providers.get_mut(selected_name) {
        provider.api_key = Some(key);
    }

    config.save().map_err(|e| format!("Failed to save config: {}", e))?;

    println!();
    println!("  Saved! Starting hal...");
    println!();

    Ok(config)
}

fn print_help() {
    println!("hal - Chat with LLMs from your terminal");
    println!("\nUSAGE:");
    println!("    hal [OPTIONS]");
    println!("    hal update");
    println!("\nOPTIONS:");
    println!("    -c, --coach              Run in coach mode");
    println!("    -m, --model <NAME>       Model name from config (default: gemini)");
    println!("    -r, --resume             Resume the last session");
    println!("    -s, --session <ID>       Load a specific session by ID");
    println!("    -h, --help               Print help");
    println!("\nCOMMANDS:");
    println!("    update                   Update hal to the latest version");
}

fn run(config: Config, session: Option<session::Session>) -> Result<(), String> {
    let mut app = App::new(config, session)?;

    // Setup terminal
    enable_raw_mode().map_err(|e| e.to_string())?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste, EnableMouseCapture).map_err(|e| e.to_string())?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

    let result = run_app(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste,
        DisableMouseCapture
    )
    .ok();
    terminal.show_cursor().ok();

    result
}

fn run_app<B: Backend + Write>(terminal: &mut Terminal<B>, app: &mut App) -> Result<(), String> {
    loop {
        terminal.draw(|f| ui::draw(f, app)).map_err(|e| e.to_string())?;

        // If we're processing, poll for API response
        if app.state != AppState::Idle {
            // Poll for events with short timeout to keep spinner animated
            if event::poll(Duration::from_millis(50)).unwrap_or(false) {
                if let Ok(ev) = event::read() {
                    handle_event(app, ev);
                }
            }
            // Check if API response or tool result is ready (non-blocking)
            app.poll_api_response();
            app.poll_tool_result();
        } else {
            // Wait for events when idle
            if let Ok(ev) = event::read() {
                handle_event(app, ev);
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn handle_event(app: &mut App, event: Event) {
    // Clear error on any input
    if matches!(event, Event::Key(_)) {
        app.error = None;
    }

    match event {
        Event::Paste(text) => {
            app.paste(&text);
        }
        Event::Key(key) => handle_key(app, key),
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => app.scroll_up(),
            MouseEventKind::ScrollDown => app.scroll_down(),
            _ => {}
        },
        _ => {}
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    // Always allow quit
    if matches!(
        key,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
    ) | matches!(
        key,
        KeyEvent {
            code: KeyCode::Char('d'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
    ) {
        app.should_quit = true;
        return;
    }

    // Handle permission modal
    if app.has_modal() {
        match key.code {
            KeyCode::Up => app.modal_up(),
            KeyCode::Down => app.modal_down(),
            KeyCode::Enter => app.modal_select(),
            KeyCode::Esc => app.modal_cancel(),
            _ => {}
        }
        return;
    }

    let is_processing = app.state != AppState::Idle;

    match key {
        // Submit - blocked while processing
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } => {
            if is_processing {
                return;
            }
            if app.picker_active() {
                app.select_picker_item();
            } else {
                app.submit_input();
            }
        }

        // Tab (select picker item)
        KeyEvent {
            code: KeyCode::Tab, ..
        } => {
            if app.picker_active() {
                app.select_picker_item();
            }
        }

        // Escape - abort if processing, otherwise cancel picker
        KeyEvent {
            code: KeyCode::Esc, ..
        } => {
            if is_processing {
                app.abort_request();
            } else {
                app.cancel_picker();
            }
        }

        // Backspace
        KeyEvent {
            code: KeyCode::Backspace,
            ..
        } => {
            app.delete_char();
        }

        // Arrow keys
        KeyEvent {
            code: KeyCode::Up, ..
        } => {
            app.history_up();
        }
        KeyEvent {
            code: KeyCode::Down,
            ..
        } => {
            app.history_down();
        }
        KeyEvent {
            code: KeyCode::Left,
            ..
        } => {
            app.move_cursor_left();
        }
        KeyEvent {
            code: KeyCode::Right,
            ..
        } => {
            app.move_cursor_right();
        }

        // Scroll
        KeyEvent {
            code: KeyCode::PageUp, ..
        }
        | KeyEvent {
            code: KeyCode::Char('u'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            app.scroll_up();
        }
        KeyEvent {
            code: KeyCode::PageDown,
            ..
        } => {
            app.scroll_down();
        }

        // Regular character
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        } if !modifiers.contains(KeyModifiers::CONTROL) => {
            app.insert_char(c);
        }

        _ => {}
    }
}
