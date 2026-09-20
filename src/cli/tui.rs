use std::io::{self, Write};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    style::Print,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

pub(crate) struct CleanRawMode;

impl CleanRawMode {
    pub(crate) fn new() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        let _ = execute!(stdout, EnterAlternateScreen, cursor::Hide);
        Ok(Self)
    }
}

impl Drop for CleanRawMode {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen, cursor::Show);
        let _ = terminal::disable_raw_mode();
    }
}

#[inline]
pub(crate) fn cycle_prev(pos: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else if pos == 0 || pos >= len {
        len - 1
    } else {
        pos - 1
    }
}

#[inline]
pub(crate) fn cycle_next(pos: usize, len: usize) -> usize {
    if len == 0 || pos + 1 >= len {
        0
    } else {
        pos + 1
    }
}

pub(crate) const MENU_BAR_WIDTH: usize = 60;

#[inline]
pub(crate) fn get_terminal_bar_width() -> usize {
    crossterm::terminal::size()
        .map(|(w, _)| (w as usize).saturating_sub(2).clamp(40, MENU_BAR_WIDTH))
        .unwrap_or(MENU_BAR_WIDTH)
}

#[inline]
pub(crate) fn visible_width(s: &str) -> usize {
    let mut width = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if (0x40..=0x7E).contains(&(c2 as u32)) {
                        break;
                    }
                }
            }
        } else if !c.is_control() {
            width += 1;
        }
    }
    width
}

pub(crate) fn format_tui_title(title: &str) -> Vec<String> {
    let lines: Vec<&str> = title.lines().collect();
    if lines.len() <= 1 {
        let single = title.trim();
        if single.is_empty() {
            return Vec::new();
        }
        if single.contains("\x1b[") {
            return vec![single.to_string()];
        }
        return vec![format!("\x1b[1;38;5;45m{}\x1b[0m", single)];
    }

    let mut out = Vec::new();
    for raw_line in lines {
        let line = raw_line.trim_end();
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("==") && trimmed.ends_with("==") {
            out.push(format!("\x1b[1;38;5;45m{}\x1b[0m", line));
        } else if let Some((key, val)) = line.split_once(':') {
            if trimmed.starts_with('•') || line.starts_with("  ") || line.starts_with('\t') {
                if val.contains("\x1b[") {
                    out.push(format!("\x1b[38;5;245m{}:\x1b[0m{}", key, val));
                } else if val.is_empty() {
                    out.push(format!("\x1b[38;5;245m{}:\x1b[0m", key));
                } else {
                    out.push(format!(
                        "\x1b[38;5;245m{}:\x1b[0m\x1b[1;37m{}\x1b[0m",
                        key, val
                    ));
                }
            } else if line.contains("\x1b[") {
                out.push(line.to_string());
            } else {
                out.push(format!("\x1b[1;38;5;45m{}\x1b[0m", line));
            }
        } else if line.contains("\x1b[") {
            out.push(line.to_string());
        } else {
            out.push(format!("\x1b[1;38;5;45m{}\x1b[0m", line));
        }
    }
    out
}

pub fn terminal_interactive_select(
    title: &str,
    items: &[String],
    initial_idx: usize,
    allow_search: bool,
    initial_query: Option<&str>,
) -> Option<usize> {
    if items.is_empty() {
        return None;
    }

    let _raw_guard = CleanRawMode::new().ok()?;
    let mut stdout = io::stdout();

    let mut query = initial_query.unwrap_or("").to_string();
    let mut selected_pos = initial_idx.min(items.len() - 1);
    let page_size = 20usize;
    let mut top_idx = 0usize;

    loop {
        let filtered: Vec<(usize, &String)> = if query.is_empty() {
            items.iter().enumerate().collect()
        } else {
            let q_low = query.to_lowercase();
            items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.to_lowercase().contains(&q_low))
                .collect()
        };

        if selected_pos >= filtered.len() {
            selected_pos = filtered.len().saturating_sub(1);
        }

        if selected_pos < top_idx {
            top_idx = selected_pos;
        } else if selected_pos >= top_idx + page_size {
            top_idx = selected_pos + 1 - page_size;
        }

        let bar_width = get_terminal_bar_width();
        let num_width = if filtered.len() >= 100 { 3 } else { 2 };
        let mut buffer = Vec::new();

        let formatted_title = format_tui_title(title);
        let title_len = formatted_title.len();
        for line in formatted_title {
            buffer.push(line);
        }

        if allow_search {
            buffer.push(format!(
                " \x1b[38;5;245mFilter:\x1b[0m \x1b[1;37m{}\x1b[38;5;81m_\x1b[0m",
                query
            ));
        } else if title_len > 1 {
            buffer.push(String::new());
        }

        buffer.push(format!("\x1b[38;5;238m{}\x1b[0m", "─".repeat(bar_width)));

        if filtered.is_empty() {
            buffer.push(
                "  \x1b[38;5;244mTidak ada pilihan yang cocok dengan filter.\x1b[0m".to_string(),
            );
        } else {
            let end_idx = (top_idx + page_size).min(filtered.len());
            if top_idx > 0 {
                buffer.push(format!(
                    "  \x1b[38;5;240m▲ ({} pilihan lagi di atas)\x1b[0m",
                    top_idx
                ));
            }
            for (curr_idx, (orig_idx, item_text)) in filtered[top_idx..end_idx].iter().enumerate() {
                let actual_idx = top_idx + curr_idx;
                let is_sel = actual_idx == selected_pos;
                if is_sel {
                    let item_for_selected =
                        item_text.replace("\x1b[0m", "\x1b[0m\x1b[48;5;237m\x1b[1;37m");
                    let prefix_vis = 3 + num_width + 2;
                    let item_vis = visible_width(item_text);
                    let pad = bar_width.saturating_sub(prefix_vis + item_vis);
                    let padding = " ".repeat(pad);
                    buffer.push(format!(
                        "\x1b[48;5;237m\x1b[1;38;5;81m ▸ \x1b[1;37m{:>num_width$}. {}{}\x1b[0m",
                        orig_idx + 1,
                        item_for_selected,
                        padding,
                        num_width = num_width
                    ));
                } else {
                    let item_for_unselected = item_text.replace("\x1b[0m", "\x1b[0m\x1b[38;5;250m");
                    buffer.push(format!(
                        "   \x1b[38;5;248m{:>num_width$}. \x1b[38;5;250m{}\x1b[0m",
                        orig_idx + 1,
                        item_for_unselected,
                        num_width = num_width
                    ));
                }
            }
            if end_idx < filtered.len() {
                buffer.push(format!(
                    "  \x1b[38;5;240m▼ ({} pilihan lagi di bawah)\x1b[0m",
                    filtered.len() - end_idx
                ));
            }
        }
        buffer.push(format!("\x1b[38;5;238m{}\x1b[0m", "─".repeat(bar_width)));

        let curr = if filtered.is_empty() {
            0
        } else {
            selected_pos + 1
        };
        let total = filtered.len();
        let scroll_hint = if total > page_size {
            let end_idx = (top_idx + page_size).min(total);
            format!(" ({}-{} dari {})", top_idx + 1, end_idx, total)
        } else {
            String::new()
        };

        if allow_search {
            buffer.push(format!(
                "\x1b[38;5;243m[{curr}/{total}]{scroll_hint} • [▲/▼] Geser · [Enter] Pilih · [Ketik] Filter · [Esc] Batal\x1b[0m"
            ));
        } else {
            buffer.push(format!(
                "\x1b[38;5;243m[{curr}/{total}]{scroll_hint} • [▲/▼] Geser · [Enter] Pilih · [Esc] Batal\x1b[0m"
            ));
        }

        let _ = execute!(
            stdout,
            cursor::MoveTo(0, 0),
            Clear(ClearType::All),
            Print(buffer.join("\r\n"))
        );
        let _ = stdout.flush();

        if let Ok(Event::Key(KeyEvent {
            code,
            modifiers,
            kind,
            ..
        })) = event::read()
        {
            if kind == KeyEventKind::Release {
                continue;
            }
            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                return None;
            }
            match code {
                KeyCode::Esc => return None,
                KeyCode::Enter => {
                    if let Some(&(orig_idx, _)) = filtered.get(selected_pos) {
                        return Some(orig_idx);
                    }
                }
                KeyCode::Up => selected_pos = cycle_prev(selected_pos, filtered.len()),
                KeyCode::Down => selected_pos = cycle_next(selected_pos, filtered.len()),
                KeyCode::PageUp => selected_pos = selected_pos.saturating_sub(page_size),
                KeyCode::PageDown => {
                    if !filtered.is_empty() {
                        selected_pos = (selected_pos + page_size).min(filtered.len() - 1);
                    }
                }
                KeyCode::Backspace if allow_search => {
                    query.pop();
                    selected_pos = 0;
                }
                KeyCode::Char(c) if allow_search => {
                    query.push(c);
                    selected_pos = 0;
                }
                _ => {}
            }
        }
    }
}
