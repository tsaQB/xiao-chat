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

pub(crate) const MENU_BAR_WIDTH: usize = 76;

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

#[inline]
pub(crate) fn truncate_visible(s: &str, max_width: usize) -> String {
    if visible_width(s) <= max_width {
        return s.to_string();
    }
    let target = max_width.saturating_sub(1);
    let mut curr_w = 0;
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            if chars.peek() == Some(&'[') {
                if let Some(bracket) = chars.next() {
                    out.push(bracket);
                    for c2 in chars.by_ref() {
                        out.push(c2);
                        if (0x40..=0x7E).contains(&(c2 as u32)) {
                            break;
                        }
                    }
                }
            }
        } else if !c.is_control() {
            if curr_w + 1 > target {
                out.push('…');
                out.push_str("\x1b[0m");
                break;
            }
            curr_w += 1;
            out.push(c);
        }
    }
    out
}

pub fn print_mini_header(subcommand: &str) {
    let bar_width = get_terminal_bar_width();
    let pkg_ver = env!("CARGO_PKG_VERSION");
    let badge_left = "\x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m";
    let title_str = format!("xiao › {subcommand}");
    let ver_str = format!("v{pkg_ver}");
    let left_vis = 7 + 2 + visible_width(&title_str);
    let right_vis = visible_width(&ver_str);
    let pad = bar_width.saturating_sub(left_vis + right_vis + 2);
    let padding = " ".repeat(pad);
    println!(
        "\n  {}  \x1b[1;37m{}\x1b[0m{}\x1b[38;5;244m{}\x1b[0m",
        badge_left, title_str, padding, ver_str
    );
    println!(
        "  \x1b[38;5;238m{}\x1b[0m",
        "─".repeat(bar_width.saturating_sub(4))
    );
}

pub fn render_hud_box(header: &str, rows: &[(&str, &str)], bar_width: usize) -> String {
    let card_inner = bar_width.saturating_sub(6);
    let prefix_len = 3; // "╭─ "
    let header_vis = visible_width(header);
    let rem_top = card_inner.saturating_sub(prefix_len + header_vis);
    let top = format!(
        "  \x1b[38;2;100;116;139m╭─ \x1b[1;38;2;203;213;225m{header}\x1b[0m \x1b[38;2;100;116;139m{}╮\x1b[0m",
        "─".repeat(rem_top)
    );

    let mut out = Vec::with_capacity(rows.len() + 2);
    out.push(top);

    let max_label_len = rows
        .iter()
        .map(|(l, _)| visible_width(l))
        .max()
        .unwrap_or(7)
        .max(7);
    let label_col_w = max_label_len + 7; // 2 spaces + label + 2 spaces + │ + 2 spaces

    for (label, val) in rows {
        let available_space = card_inner.saturating_sub(label_col_w);
        let val_fitted = truncate_visible(val, available_space);
        let val_vis = visible_width(&val_fitted);
        let pad = available_space.saturating_sub(val_vis);
        let padding = " ".repeat(pad);
        let label_pad = " ".repeat(max_label_len.saturating_sub(visible_width(label)));
        out.push(format!(
            "  \x1b[38;2;100;116;139m│\x1b[0m  \x1b[38;2;148;163;184m{label}{label_pad}\x1b[0m  \x1b[38;2;100;116;139m│\x1b[0m  {}{}\x1b[38;2;100;116;139m│\x1b[0m",
            val_fitted, padding
        ));
    }

    let bot = format!(
        "  \x1b[38;2;100;116;139m╰{}╯\x1b[0m",
        "─".repeat(card_inner)
    );
    out.push(bot);

    out.join("\r\n")
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
        if trimmed.starts_with('╭')
            || trimmed.starts_with('│')
            || trimmed.starts_with('╰')
            || trimmed.starts_with('┌')
            || trimmed.starts_with('└')
            || (trimmed.starts_with("\x1b[")
                && (trimmed.contains('╭') || trimmed.contains('│') || trimmed.contains('╰')))
        {
            out.push(line.to_string());
            continue;
        }
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
            buffer.push("  \x1b[38;5;244mNo options matching filter.\x1b[0m".to_string());
        } else {
            let end_idx = (top_idx + page_size).min(filtered.len());
            if top_idx > 0 {
                buffer.push(format!("  \x1b[38;5;240m▲ ({} more above)\x1b[0m", top_idx));
            }
            let card_inner = bar_width.saturating_sub(6);
            for (curr_idx, (orig_idx, item_text)) in filtered[top_idx..end_idx].iter().enumerate() {
                let actual_idx = top_idx + curr_idx;
                let is_sel = actual_idx == selected_pos;
                if is_sel {
                    buffer.push(format!(
                        "  \x1b[38;2;203;213;225m╭{}╮\x1b[0m",
                        "─".repeat(card_inner)
                    ));

                    let prefix_vis = 2 + 1 + 2 + num_width + 2;
                    let max_item_w = card_inner.saturating_sub(prefix_vis);
                    let fitted_text = truncate_visible(item_text, max_item_w);
                    let item_for_selected =
                        fitted_text.replace("\x1b[0m", "\x1b[0m\x1b[48;5;237m\x1b[1;37m");
                    let item_vis = visible_width(&fitted_text);
                    let pad = card_inner.saturating_sub(prefix_vis + item_vis);
                    let padding = " ".repeat(pad);

                    buffer.push(format!(
                        "  \x1b[38;2;203;213;225m│\x1b[0m\x1b[48;5;237m  \x1b[1;37m▸  {:>num_width$}. {}{}\x1b[0m\x1b[38;2;203;213;225m│\x1b[0m",
                        orig_idx + 1,
                        item_for_selected,
                        padding,
                        num_width = num_width
                    ));

                    buffer.push(format!(
                        "  \x1b[38;2;203;213;225m╰{}╯\x1b[0m",
                        "─".repeat(card_inner)
                    ));
                } else {
                    let item_for_unselected = item_text.replace("\x1b[0m", "\x1b[0m\x1b[38;5;250m");
                    buffer.push(format!(
                        "        \x1b[38;5;244m{:>num_width$}. \x1b[38;5;250m{}\x1b[0m",
                        orig_idx + 1,
                        item_for_unselected,
                        num_width = num_width
                    ));
                }
            }
            if end_idx < filtered.len() {
                buffer.push(format!(
                    "  \x1b[38;5;240m▼ ({} more below)\x1b[0m",
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
            format!(" ({}-{} of {})", top_idx + 1, end_idx, total)
        } else {
            String::new()
        };

        if allow_search {
            buffer.push(format!(
                "\x1b[38;5;243m[{curr}/{total}]{scroll_hint} • [▲/▼] Navigate · [Enter] Select · [Type] Filter · [Esc] Cancel\x1b[0m"
            ));
        } else {
            buffer.push(format!(
                "\x1b[38;5;243m[{curr}/{total}]{scroll_hint} • [▲/▼] Navigate · [Enter] Select · [Esc] Cancel\x1b[0m"
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
