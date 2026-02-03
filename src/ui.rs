use crate::app::{App, AppState, MessageRole, PermissionModal, PickerMode, MAX_PICKER_ITEMS};
use ratatui::{
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::sync::OnceLock;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::easy::HighlightLines;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

fn get_syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn get_theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

/// Highlight a diff line with syntax coloring and diff background
fn highlight_diff_line(line: &str, path: Option<&str>) -> Vec<Span<'static>> {
    let (bg_color, code_content) = if line.starts_with('+') {
        (Some(Color::Rgb(30, 50, 30)), &line[1..]) // Dark green bg
    } else if line.starts_with('-') {
        (Some(Color::Rgb(50, 30, 30)), &line[1..]) // Dark red bg
    } else {
        (None, line)
    };

    let prefix = if line.starts_with('+') || line.starts_with('-') {
        &line[..1]
    } else {
        ""
    };

    // Try to get syntax for the file
    let ss = get_syntax_set();
    let ts = get_theme_set();

    let syntax = path
        .and_then(|p| ss.find_syntax_for_file(p).ok().flatten())
        .unwrap_or_else(|| ss.find_syntax_plain_text());

    let theme = &ts.themes["base16-ocean.dark"];
    let mut highlighter = HighlightLines::new(syntax, theme);

    let mut spans = Vec::new();

    // Add the +/- prefix with appropriate color
    if !prefix.is_empty() {
        let prefix_fg = if prefix == "+" {
            Color::Green
        } else {
            Color::Red
        };
        let mut style = Style::default().fg(prefix_fg);
        if let Some(bg) = bg_color {
            style = style.bg(bg);
        }
        spans.push(Span::styled(prefix.to_string(), style));
    }

    // Highlight the code content
    match highlighter.highlight_line(code_content, ss) {
        Ok(highlighted) => {
            for (syntect_style, text) in highlighted {
                let fg = Color::Rgb(
                    syntect_style.foreground.r,
                    syntect_style.foreground.g,
                    syntect_style.foreground.b,
                );
                let mut style = Style::default().fg(fg);
                if let Some(bg) = bg_color {
                    style = style.bg(bg);
                }
                spans.push(Span::styled(text.to_string(), style));
            }
        }
        Err(_) => {
            // Fallback: no syntax highlighting
            let fg = if line.starts_with('+') {
                Color::Green
            } else if line.starts_with('-') {
                Color::Red
            } else {
                Color::Gray
            };
            let mut style = Style::default().fg(fg);
            if let Some(bg) = bg_color {
                style = style.bg(bg);
            }
            spans.push(Span::styled(code_content.to_string(), style));
        }
    }

    spans
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    // Calculate dynamic input height based on content (use char count, not byte length)
    // Account for horizontal padding (2 chars) in width calculation
    let input_char_count = app.input.chars().count();
    let effective_width = frame.area().width.saturating_sub(2) as usize;
    let input_height = calculate_input_height(input_char_count, effective_width);

    let chunks = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(1),    // Chat
        Constraint::Length(input_height), // Input (dynamic)
    ])
    .split(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_chat(frame, app, chunks[1]);
    draw_input(frame, app, chunks[2]);

    // Draw picker popup if active
    if app.picker_active() && !app.picker_results.is_empty() {
        draw_picker(frame, app, chunks[2]);
    }

    // Draw permission modal if active
    if let Some(modal) = &app.permission_modal {
        draw_permission_modal(frame, modal);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let mode = match app.config.mode {
        crate::config::Mode::Coding => "coding",
        crate::config::Mode::Coach => "coach",
    };

    let left = Line::from(vec![
        Span::styled(format!(" hal {}", env!("CARGO_PKG_VERSION")), Style::default().fg(Color::Magenta).bold()),
        Span::styled(" · ", Style::default().fg(Color::Gray)),
        Span::styled(&app.config.default_provider, Style::default().fg(Color::Cyan)),
        Span::styled(format!(" [{}]", mode), Style::default().fg(Color::Gray)),
    ]);

    let right = if let Some((prompt, completion)) = app.token_usage {
        format!("{} in / {} out ", prompt, completion)
    } else {
        String::new()
    };

    // Get working directory for center
    let cwd_full = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let version = env!("CARGO_PKG_VERSION");
    let left_len = 4 + 1 + version.len() + 3 + app.config.default_provider.len() + 2 + mode.len() + 3; // approximate + padding
    let right_len = right.len();
    let available = (area.width as usize).saturating_sub(left_len + right_len + 2);

    // Truncate from left if too long
    let cwd = if cwd_full.len() > available && available > 3 {
        format!("…{}", &cwd_full[cwd_full.len().saturating_sub(available - 1)..])
    } else {
        cwd_full
    };

    let center_x = (area.width as usize).saturating_sub(cwd.len()) / 2;

    // Render left
    frame.render_widget(Paragraph::new(left), area);

    // Render center (working dir)
    if center_x > left_len && !cwd.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(&cwd, Style::default().fg(Color::Gray)))),
            Rect { x: area.x + center_x as u16, width: cwd.chars().count() as u16, ..area },
        );
    }

    // Render right
    if !right.is_empty() {
        let right_width = right.len() as u16;
        let right_x = area.width.saturating_sub(right_width);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(right, Style::default().fg(Color::Gray)))),
            Rect { x: area.x + right_x, width: right_width, ..area },
        );
    }
}

fn draw_chat(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::Gray));

    let block_inner = block.inner(area);
    frame.render_widget(block, area);

    // Add horizontal padding
    let inner_area = Rect {
        x: block_inner.x + 1,
        y: block_inner.y,
        width: block_inner.width.saturating_sub(2),
        height: block_inner.height,
    };

    if app.messages.is_empty() {
        let help = Paragraph::new(Line::from(vec![
            Span::styled("Type ", Style::default().fg(Color::Gray)),
            Span::styled("/help", Style::default().fg(Color::Magenta)),
            Span::styled(" for commands", Style::default().fg(Color::Gray)),
        ]));
        frame.render_widget(help, inner_area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        match &msg.role {
            MessageRole::User => {
                lines.push(Line::from(""));
                // Bright teal for user messages
                let user_color = Color::Rgb(100, 220, 215);
                lines.push(Line::from(vec![
                    Span::styled("› ", Style::default().fg(user_color)),
                    Span::styled(&msg.content, Style::default().fg(user_color)),
                ]));
            }
            MessageRole::Assistant => {
                lines.push(Line::from(""));
                for line in msg.content.lines() {
                    if line.starts_with("### ") {
                        lines.push(Line::from(Span::styled(
                            &line[4..],
                            Style::default().fg(Color::Magenta).italic(),
                        )));
                    } else if line.starts_with("## ") {
                        lines.push(Line::from(Span::styled(
                            &line[3..],
                            Style::default().fg(Color::Magenta),
                        )));
                    } else if line.starts_with("# ") {
                        lines.push(Line::from(Span::styled(
                            &line[2..],
                            Style::default().fg(Color::Magenta).bold(),
                        )));
                    } else if line.starts_with("- ") || line.starts_with("* ") {
                        let mut spans = vec![Span::styled("  • ", Style::default().fg(Color::Magenta))];
                        spans.extend(render_inline_styles(&line[2..], None));
                        lines.push(Line::from(spans));
                    } else if line.starts_with("```") {
                        lines.push(Line::from(Span::styled(
                            line,
                            Style::default().fg(Color::Gray),
                        )));
                    } else if line.starts_with("**") && line.ends_with("**") {
                        lines.push(Line::from(Span::styled(
                            line.trim_matches('*'),
                            Style::default().bold(),
                        )));
                    } else {
                        lines.push(Line::from(render_inline_styles(line, None)));
                    }
                }
            }
            MessageRole::Tool { name, path } => {
                if name == "write_file" || name == "edit_file" {
                    // Render diff inline with syntax highlighting
                    let mut result_lines = msg.content.lines();
                    if let Some(first) = result_lines.next() {
                        lines.push(Line::from(vec![
                            Span::styled("  ◇ ", Style::default().fg(Color::Magenta)),
                            Span::styled(first.to_string(), Style::default().fg(Color::Gray)),
                        ]));
                    }
                    for line in result_lines {
                        if line.is_empty() {
                            continue;
                        }
                        let highlighted = highlight_diff_line(line, path.as_deref());
                        let mut spans = vec![Span::raw("    ")];
                        spans.extend(highlighted);
                        lines.push(Line::from(spans));
                    }
                } else {
                    let display = format_tool_result(name, path.as_deref(), &msg.content);
                    let mut first = true;
                    for line in display.lines() {
                        if first {
                            lines.push(Line::from(vec![
                                Span::styled("  ◇ ", Style::default().fg(Color::Magenta)),
                                Span::styled(line.to_string(), Style::default().fg(Color::Gray)),
                            ]));
                            first = false;
                        } else {
                            lines.push(Line::from(vec![
                                Span::raw("    "),
                                Span::styled(line.to_string(), Style::default().fg(Color::Gray)),
                            ]));
                        }
                    }
                }
            }
        }
    }

    // Add typing indicator if processing
    if app.state != AppState::Idle {
        lines.push(Line::from(""));
        let spinner = get_spinner_frame();
        let status_text = match &app.state {
            AppState::Thinking => "Thinking...".to_string(),
            AppState::ToolCall(name) => name.clone(),
            AppState::Idle => unreachable!(),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", spinner), Style::default().fg(Color::Magenta)),
            Span::styled(status_text, Style::default().fg(Color::Gray)),
        ]));
    }

    // Show error if present
    if let Some(err) = &app.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("✗ {}", err),
            Style::default().fg(Color::Red),
        )));
    }

    // Calculate scroll - we want to show the bottom by default
    // Account for text wrapping when calculating content height
    let width = inner_area.width as usize;
    let content_height: u16 = lines
        .iter()
        .map(|line| {
            let line_width = line.width();
            if width == 0 {
                1
            } else {
                // Every line takes at least 1 row, plus extra rows for wrapping
                // Add 1 as buffer since ratatui's wrapping may differ slightly
                1 + (line_width / width) as u16
            }
        })
        .sum();
    let view_height = inner_area.height;
    let max_scroll = content_height.saturating_sub(view_height);
    app.scroll_offset = app.scroll_offset.min(max_scroll);
    let scroll = max_scroll.saturating_sub(app.scroll_offset);

    let para = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(para, inner_area);
}

fn render_inline_styles(line: &str, base_color: Option<Color>) -> Vec<Span<'_>> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut in_code = false;
    let mut in_bold = false;

    let base_style = match base_color {
        Some(c) => Style::default().fg(c),
        None => Style::default(),
    };

    while let Some(c) = chars.next() {
        if c == '`' && !in_bold {
            if !current.is_empty() {
                spans.push(if in_code {
                    Span::styled(std::mem::take(&mut current), Style::default().fg(Color::Yellow))
                } else {
                    Span::styled(std::mem::take(&mut current), base_style)
                });
            }
            in_code = !in_code;
        } else if c == '*' && chars.peek() == Some(&'*') && !in_code {
            chars.next();
            if !current.is_empty() {
                spans.push(if in_bold {
                    Span::styled(std::mem::take(&mut current), base_style.bold())
                } else {
                    Span::styled(std::mem::take(&mut current), base_style)
                });
            }
            in_bold = !in_bold;
        } else {
            current.push(c);
        }
    }

    if !current.is_empty() {
        spans.push(if in_code {
            Span::styled(current, Style::default().fg(Color::Yellow))
        } else if in_bold {
            Span::styled(current, base_style.bold())
        } else {
            Span::styled(current, base_style)
        });
    }

    if spans.is_empty() {
        spans.push(Span::styled("", base_style));
    }

    spans
}

fn format_tool_result(name: &str, path: Option<&str>, result: &str) -> String {
    if result.starts_with("Error") {
        return result.to_string();
    }

    match name {
        "read_file" => {
            let lines = result.lines().count();
            match path {
                Some(p) => format!("Read {} ({} lines)", p, lines),
                None => format!("Read file ({} lines)", lines),
            }
        }
        "list_dir" => {
            let items = result.lines().count();
            let dir = path.unwrap_or(".");
            if items <= 8 {
                format!("Listed {} ({})", dir, result.lines().collect::<Vec<_>>().join("  "))
            } else {
                format!("Listed {} ({} items)", dir, items)
            }
        }
        "search_files" => {
            let files: Vec<_> = result.lines().collect();
            if files.len() <= 6 {
                format!("Found {}", files.join(", "))
            } else {
                format!("Found {} files", files.len())
            }
        }
        "write_file" => {
            // Extract just the first line (the "Wrote path" part)
            result.lines().next().unwrap_or(result).to_string()
        }
        "bash" => {
            let mut lines = result.lines();
            let cmd = lines.next().unwrap_or("$ ?");
            let output: Vec<_> = lines.collect();
            if output.is_empty() {
                cmd.to_string()
            } else if output.len() <= 10 {
                format!("{}\n{}", cmd, output.join("\n"))
            } else {
                format!("{}\n{}\n... ({} more lines)", cmd, output[..8].join("\n"), output.len() - 8)
            }
        }
        "grep" => {
            // First line is "grep 'pattern':" - extract pattern
            let mut lines = result.lines();
            let header = lines.next().unwrap_or("");
            let pattern = header.strip_prefix("grep '")
                .and_then(|s| s.strip_suffix("':"))
                .unwrap_or("?");
            let matches: Vec<_> = lines.filter(|l| !l.is_empty() && *l != "--").collect();
            if matches.is_empty() || result.contains("no matches") {
                format!("Grep '{}': no matches", pattern)
            } else if matches.len() <= 4 {
                format!("Grep '{}':\n{}", pattern, matches.iter().map(|l| {
                    if l.len() > 60 { format!("{}...", &l[..60]) } else { l.to_string() }
                }).collect::<Vec<_>>().join("\n"))
            } else {
                format!("Grep '{}': {} matches\n{}\n...", pattern, matches.len(), matches[..3].iter().map(|l| {
                    if l.len() > 60 { format!("{}...", &l[..60]) } else { l.to_string() }
                }).collect::<Vec<_>>().join("\n"))
            }
        }
        _ => {
            if result.len() > 60 {
                format!("{}...", &result[..60])
            } else {
                result.to_string()
            }
        }
    }
}

fn draw_input(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::Gray));

    let block_inner = block.inner(area);
    frame.render_widget(block, area);

    // Add horizontal padding to match chat area
    let inner = Rect {
        x: block_inner.x + 1,
        y: block_inner.y,
        width: block_inner.width.saturating_sub(2),
        height: block_inner.height,
    };

    let prefix = "› ";
    let prefix_width = 2; // display width, not byte length
    let width = inner.width as usize;

    // Build wrapped lines manually to preserve prefix styling
    let lines = wrap_input_lines(prefix, &app.input, prefix_width, width);
    let para = Paragraph::new(Text::from(lines));
    frame.render_widget(para, inner);

    // Calculate cursor position in wrapped text (convert byte index to char count)
    let cursor_char_pos = app.input[..app.input_cursor].chars().count();
    let (cursor_x, cursor_y) = calculate_wrapped_cursor(cursor_char_pos, prefix_width, width);
    frame.set_cursor_position(Position::new(
        (inner.x + cursor_x as u16).min(inner.right().saturating_sub(1)),
        (inner.y + cursor_y as u16).min(inner.bottom().saturating_sub(1)),
    ));
}

fn wrap_input_lines(prefix: &str, input: &str, prefix_width: usize, width: usize) -> Vec<Line<'static>> {
    let first_line_cap = width.saturating_sub(prefix_width);

    if first_line_cap == 0 || input.is_empty() {
        return vec![Line::from(vec![
            Span::styled(prefix.to_string(), Style::default().fg(Color::Cyan)),
            Span::raw(input.to_string()),
        ])];
    }

    let mut lines = Vec::new();
    let mut chars = input.chars();

    // First line: prefix + content
    let first_part: String = chars.by_ref().take(first_line_cap).collect();
    lines.push(Line::from(vec![
        Span::styled(prefix.to_string(), Style::default().fg(Color::Cyan)),
        Span::raw(first_part),
    ]));

    // Remaining lines: full width
    loop {
        let part: String = chars.by_ref().take(width).collect();
        if part.is_empty() {
            break;
        }
        lines.push(Line::from(Span::raw(part)));
    }

    lines
}

fn calculate_wrapped_cursor(cursor: usize, prefix_len: usize, width: usize) -> (usize, usize) {
    let first_line_cap = width.saturating_sub(prefix_len);

    if cursor <= first_line_cap {
        (prefix_len + cursor, 0)
    } else {
        let remaining = cursor - first_line_cap;
        let line = 1 + remaining / width;
        let col = remaining % width;
        (col, line)
    }
}

fn calculate_input_height(input_len: usize, width: usize) -> u16 {
    let prefix_len = 2; // "› "
    if width <= prefix_len {
        return 2;
    }

    let first_line_cap = width - prefix_len;
    let content_lines = if input_len <= first_line_cap {
        1
    } else {
        let remaining = input_len - first_line_cap;
        1 + (remaining + width - 1) / width
    };

    (content_lines as u16 + 1).max(2) // +1 for top border
}

fn draw_picker(frame: &mut Frame, app: &App, input_area: Rect) {
    let height = (app.picker_results.len() as u16).min(MAX_PICKER_ITEMS as u16) + 2;
    let width = 40.min(input_area.width.saturating_sub(4));

    let area = Rect {
        x: input_area.x + 3,
        y: input_area.y.saturating_sub(height),
        width,
        height,
    };

    let (title, item_prefix) = match app.picker_mode {
        PickerMode::Files => (" Files ", ""),
        PickerMode::Commands => (" Commands ", "/"),
        PickerMode::None => return,
    };

    let items: Vec<ListItem> = app
        .picker_results
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let style = if i == app.picker_selected {
                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            let prefix = if i == app.picker_selected { "› " } else { "  " };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format!("{}{}", item_prefix, item), style),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray))
            .title(title)
            .title_style(Style::default().fg(Color::Magenta)),
    );

    frame.render_widget(Clear, area);
    frame.render_widget(list, area);
}

fn get_spinner_frame() -> char {
    const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    FRAMES[(ms / 80) as usize % FRAMES.len()]
}

fn draw_permission_modal(frame: &mut Frame, modal: &PermissionModal) {
    let area = frame.area();

    // Modal dimensions
    let width = 60.min(area.width.saturating_sub(4));
    let height = 10.min(area.height.saturating_sub(4));

    // Center the modal
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;

    let modal_area = Rect { x, y, width, height };

    // Clear background
    frame.render_widget(Clear, modal_area);

    // Build content
    let mut lines = vec![
        Line::from(Span::styled(
            "Permission Required",
            Style::default().fg(Color::Magenta).bold(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Path: ", Style::default().fg(Color::Gray)),
            Span::styled(&modal.path, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(Span::styled(&modal.reason, Style::default().fg(Color::Gray))),
        Line::from(""),
    ];

    // Options
    for (i, option) in modal.options.iter().enumerate() {
        let style = if i == modal.selected {
            Style::default().fg(Color::Magenta).bold()
        } else {
            Style::default().fg(Color::White)
        };
        let prefix = if i == modal.selected { "› " } else { "  " };
        lines.push(Line::from(Span::styled(format!("{}{}", prefix, option), style)));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .title(" Sandbox ")
        .title_style(Style::default().fg(Color::Magenta));

    let para = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: true });

    frame.render_widget(para, modal_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inline_bold() {
        let spans = render_inline_styles("hello **world** there", None);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "hello ");
        assert_eq!(spans[1].content, "world");
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[2].content, " there");
    }

    #[test]
    fn test_inline_code() {
        let spans = render_inline_styles("use `foo()` here", None);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "use ");
        assert_eq!(spans[1].content, "foo()");
        assert_eq!(spans[1].style.fg, Some(Color::Yellow));
        assert_eq!(spans[2].content, " here");
    }

    #[test]
    fn test_bold_at_start() {
        let spans = render_inline_styles("**Bold:** rest of line", None);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "Bold:");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content, " rest of line");
    }

    #[test]
    fn test_mixed_bold_and_code() {
        let spans = render_inline_styles("**bold** and `code`", None);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "bold");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content, " and ");
        assert_eq!(spans[2].content, "code");
        assert_eq!(spans[2].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn test_plain_text() {
        let spans = render_inline_styles("just plain text", None);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "just plain text");
    }

    #[test]
    fn test_unclosed_bold() {
        let spans = render_inline_styles("**unclosed bold", None);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "unclosed bold");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    }
}
