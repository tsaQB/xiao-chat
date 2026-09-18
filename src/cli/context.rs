use crate::ai::AIChatService;
use crate::cli::tui::get_terminal_bar_width;
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
            println!("\n\x1b[1;36mxiao context — Context Window & Token Breakdown\x1b[0m\n");
            println!("\x1b[1;37mUsage:\x1b[0m");
            println!("  xiao context [chat_id] [thread_id]\n");
            println!("Displays token consumption, sliding window turns, memory facts count, and visual gauge.");
            println!("If omitted, defaults to the owner's private chat session.\n");
        }
        ContextCliArgs::InvalidChatId(s) => {
            println!("\x1b[31m✖ Error: Chat ID '{s}' harus berupa angka (integer).\x1b[0m");
            println!("  Jalankan 'xiao context help' untuk panduan penggunaan.\n");
            std::process::exit(1);
        }
        ContextCliArgs::InvalidThreadId(s) => {
            println!("\x1b[31m✖ Error: Thread ID '{s}' harus berupa angka (integer).\x1b[0m");
            println!("  Jalankan 'xiao context help' untuk panduan penggunaan.\n");
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

            println!("\n\x1b[1;36m== Xiao Context Window & Memory Gauge ==\x1b[0m\n");
            println!(
                "  \x1b[38;5;245mScope           :\x1b[0m \x1b[1;37m{}\x1b[0m \x1b[38;5;244m(chat: {chat_id}, thread: {thread_id})\x1b[0m",
                stats.session_name
            );
            println!(
                "  \x1b[38;5;245mMain Model      :\x1b[0m \x1b[1;37m{}\x1b[0m",
                stats.model_name
            );
            println!("  \x1b[38;5;245mContext Limit   :\x1b[0m \x1b[1;37m{} tokens\x1b[0m \x1b[38;5;244m({})\x1b[0m", stats.limit_tokens, stats.limit_str);
            println!("  \x1b[38;5;245mActive Tokens   :\x1b[0m \x1b[1;37m{} tokens\x1b[0m \x1b[38;5;244m(~{} chars)\x1b[0m", stats.total_tokens, stats.total_chars);
            println!(
                "  \x1b[38;5;245mOutput Reserve  :\x1b[0m \x1b[1;37m{} tokens\x1b[0m",
                stats.output_reserve_tokens
            );
            println!("  \x1b[38;5;245mSliding Window  :\x1b[0m \x1b[1;37m{} messages\x1b[0m \x1b[38;5;244m({} turns)\x1b[0m", stats.total_messages, stats.total_turns);
            println!(
                "  \x1b[38;5;245mAttachments     :\x1b[0m \x1b[1;37m{}\x1b[0m",
                stats.attachment_count
            );

            let gauge_len = 30;
            let (bar, gauge_color) = format_context_gauge(stats.usage_pct, gauge_len);
            println!("\n  \x1b[1;37mContext Utilization:\x1b[0m");
            println!(
                "  {}[{}]\x1b[0m \x1b[1;37m{:.1}%\x1b[0m",
                gauge_color, bar, stats.usage_pct
            );

            let memories = crate::ai::storage::get_user_memories_async(owner_id).await;
            println!("\n  \x1b[1;37mLong-Term Facts (Tier 1):\x1b[0m \x1b[1;36m{} facts stored\x1b[0m \x1b[38;5;244m(manage with 'xiao memory')\x1b[0m", memories.len());

            if !stats.messages_breakdown.is_empty() {
                println!("\n  \x1b[1;37mActive Sliding Window Breakdown (Recent):\x1b[0m");
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
