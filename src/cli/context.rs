use crate::ai::AIChatService;
use crate::cli::tui::{get_terminal_bar_width, print_mini_header, render_hud_box};
use crate::{get_configured_owner_id, load_environment};

#[derive(Debug, PartialEq, Eq)]
pub enum ContextCliArgs<'a> {
    Inspect {
        chat_id: Option<i64>,
        thread_id: Option<i64>,
    },
    Help,
    InvalidChatId(&'a str),
    InvalidThreadId(&'a str),
}

pub fn parse_context_cli_args<'a>(
    arg1: Option<&'a str>,
    arg2: Option<&'a str>,
) -> ContextCliArgs<'a> {
    if matches!(arg1, Some("help") | Some("--help") | Some("-h")) {
        return ContextCliArgs::Help;
    }
    let chat_id = match arg1 {
        Some(s) => match s.parse::<i64>() {
            Ok(id) => Some(id),
            Err(_) => return ContextCliArgs::InvalidChatId(s),
        },
        None => None,
    };
    let thread_id = match arg2 {
        Some(s) => match s.parse::<i64>() {
            Ok(id) => Some(id),
            Err(_) => return ContextCliArgs::InvalidThreadId(s),
        },
        None => None,
    };
    ContextCliArgs::Inspect { chat_id, thread_id }
}

pub fn format_context_gauge(usage_pct: f64, gauge_len: usize) -> (String, &'static str) {
    let filled = ((usage_pct / 100.0) * gauge_len as f64).round() as usize;
    let filled = filled.min(gauge_len);
    let empty = gauge_len.saturating_sub(filled);
    let gauge_color = if usage_pct < 60.0 {
        "\x1b[1;32m"
    } else if usage_pct < 85.0 {
        "\x1b[1;33m"
    } else {
        "\x1b[1;31m"
    };
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));
    (bar, gauge_color)
}

pub(crate) async fn run_cli_context(
    ai_service: &AIChatService,
    raw_chat: Option<&str>,
    raw_thread: Option<&str>,
) {
    match parse_context_cli_args(raw_chat, raw_thread) {
        ContextCliArgs::Help => {
            let bar_width = get_terminal_bar_width();
            print_mini_header("Context Window › Command Reference");

            println!("\n  \x1b[1;37mUsage:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao context\x1b[0m \x1b[38;5;245m[chat_id] [thread_id]\x1b[0m\n");

            println!("  \x1b[1;38;2;6;182;212m▸ \x1b[1;37mDESCRIPTION\x1b[0m");
            println!("    \x1b[38;5;250mDisplays token consumption, sliding window turns, memory facts count, and visual gauge.\x1b[0m");
            println!("    \x1b[38;5;244mIf chat_id and thread_id are omitted, defaults to the owner's private chat session.\x1b[0m\n");

            println!(
                "  \x1b[38;5;238m{}\x1b[0m\n",
                "─".repeat(bar_width.saturating_sub(4))
            );

            println!("  \x1b[1;37mQuick Examples:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao context\x1b[0m                      \x1b[38;5;242m# Inspect owner's context budget\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao context -1001234567890 42\x1b[0m    \x1b[38;5;242m# Inspect specific group/forum topic\x1b[0m\n");
        }
        ContextCliArgs::InvalidChatId(s) => {
            println!("\x1b[31m✖ Error: Chat ID '{s}' must be an integer.\x1b[0m");
            println!("  Run 'xiao context help' for usage instructions.\n");
            std::process::exit(1);
        }
        ContextCliArgs::InvalidThreadId(s) => {
            println!("\x1b[31m✖ Error: Thread ID '{s}' must be an integer.\x1b[0m");
            println!("  Run 'xiao context help' for usage instructions.\n");
            std::process::exit(1);
        }
        ContextCliArgs::Inspect {
            chat_id: target_chat_id,
            thread_id: target_thread_id,
        } => {
            load_environment();
            let owner_id = get_configured_owner_id().unwrap_or(0);
            let chat_id = target_chat_id.unwrap_or(owner_id);
            let thread_id = target_thread_id.unwrap_or(0);
            let stats = ai_service
                .get_scoped_context_stats(chat_id, thread_id, owner_id)
                .await;
            let bar_width = get_terminal_bar_width();
            print_mini_header("Context Window & Token Breakdown");

            let scope_val = format!(
                "{} (chat: {chat_id}, thread: {thread_id})",
                stats.session_name
            );
            let model_val = stats.model_name.clone();
            let limit_val = format!("{} tokens ({})", stats.limit_tokens, stats.limit_str);
            let usage_val = format!(
                "{} tokens (~{} chars) · {:.1}%",
                stats.total_tokens, stats.total_chars, stats.usage_pct
            );
            let reserve_val = format!("{} tokens", stats.output_reserve_tokens);

            let hud_rows = [
                ("ACTIVE SESSION", scope_val.as_str()),
                ("MAIN MODEL", model_val.as_str()),
                ("CONTEXT LIMIT", limit_val.as_str()),
                ("TOKEN USAGE", usage_val.as_str()),
                ("OUTPUT RESERVE", reserve_val.as_str()),
            ];
            let hud = render_hud_box("TOKEN & CONTEXT BUDGET", &hud_rows, bar_width);
            println!("\n{hud}");

            let gauge_len = 30;
            let (bar, gauge_color) = format_context_gauge(stats.usage_pct, gauge_len);
            println!("\n  \x1b[1;37m▸ CONTEXT UTILIZATION GAUGE\x1b[0m");
            println!(
                "    {}[{}]\x1b[0m \x1b[1;37m{:.1}%\x1b[0m",
                gauge_color, bar, stats.usage_pct
            );

            let memories = crate::ai::storage::get_user_memories_async(owner_id).await;
            println!("\n  \x1b[1;37m▸ TIER-1 LONG-TERM FACTS\x1b[0m");
            println!(
                "    • Total Stored   : \x1b[1;36m{} facts recorded\x1b[0m \x1b[38;5;244m(manage with 'xiao memory')\x1b[0m",
                memories.len()
            );

            if !stats.messages_breakdown.is_empty() {
                println!("\n  \x1b[1;37m▸ SLIDING WINDOW BREAKDOWN (RECENT)\x1b[0m");
                println!(
                    "  \x1b[38;5;238m{}\x1b[0m",
                    "─".repeat(bar_width.saturating_sub(4))
                );
                for item in stats.messages_breakdown.iter().rev().take(10) {
                    let role_badge = if item.role == "user" {
                        "\x1b[1;34m[User]     \x1b[0m"
                    } else if item.role == "assistant" {
                        "\x1b[1;32m[Assistant]\x1b[0m"
                    } else {
                        "\x1b[1;33m[System]   \x1b[0m"
                    };
                    println!(
                        "  {} \x1b[38;5;245m{:>5} tok\x1b[0m │ \x1b[38;5;252m{}\x1b[0m",
                        role_badge, item.tokens, item.preview
                    );
                }
                println!(
                    "  \x1b[38;5;238m{}\x1b[0m\n",
                    "─".repeat(bar_width.saturating_sub(4))
                );
            } else {
                println!();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_context_cli_args() {
        assert_eq!(
            parse_context_cli_args(Some("help"), None),
            ContextCliArgs::Help
        );
        assert_eq!(
            parse_context_cli_args(Some("--help"), None),
            ContextCliArgs::Help
        );
        assert_eq!(
            parse_context_cli_args(Some("12345"), Some("678")),
            ContextCliArgs::Inspect {
                chat_id: Some(12345),
                thread_id: Some(678)
            }
        );
        assert_eq!(
            parse_context_cli_args(None, None),
            ContextCliArgs::Inspect {
                chat_id: None,
                thread_id: None
            }
        );
        assert_eq!(
            parse_context_cli_args(Some("abc"), None),
            ContextCliArgs::InvalidChatId("abc")
        );
        assert_eq!(
            parse_context_cli_args(Some("123"), Some("def")),
            ContextCliArgs::InvalidThreadId("def")
        );
    }

    #[test]
    fn test_format_context_gauge() {
        let (bar20, col20) = format_context_gauge(20.0, 10);
        assert_eq!(bar20, "██░░░░░░░░");
        assert_eq!(col20, "\x1b[1;32m");

        let (bar70, col70) = format_context_gauge(70.0, 10);
        assert_eq!(bar70, "███████░░░");
        assert_eq!(col70, "\x1b[1;33m");

        let (bar95, col95) = format_context_gauge(95.0, 10);
        assert_eq!(bar95, "██████████");
        assert_eq!(col95, "\x1b[1;31m");
    }
}
