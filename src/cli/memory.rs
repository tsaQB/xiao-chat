use std::io::{self, IsTerminal, Write};

use crate::ai::AIChatService;
use crate::cli::tui::{get_terminal_bar_width, terminal_interactive_select, visible_width};
use crate::{get_configured_owner_id, load_environment};

#[derive(Debug, PartialEq, Eq)]
pub enum MemoryCliAction<'a> {
    Menu,
    List,
    Clear,
    Remove(Option<&'a str>),
    Help,
    Unknown(&'a str),
}

pub fn parse_memory_cli_action<'a>(
    action: Option<&'a str>,
    target: Option<&'a str>,
) -> MemoryCliAction<'a> {
    match action {
        Some("help") | Some("--help") | Some("-h") => MemoryCliAction::Help,
        Some("clear") => MemoryCliAction::Clear,
        Some("rm") | Some("remove") | Some("delete") => MemoryCliAction::Remove(target),
        Some("menu") | None => MemoryCliAction::Menu,
        Some("list") => MemoryCliAction::List,
        Some(unknown) => MemoryCliAction::Unknown(unknown),
    }
}

async fn run_interactive_memory_menu(owner_id: i64) {
    loop {
        load_environment();
        let memories = crate::ai::storage::get_user_memories_async(owner_id).await;
        let bar_width = get_terminal_bar_width();
        let pkg_ver = env!("CARGO_PKG_VERSION");
        let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › Long-Term Memory Hub\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 27;
        let ver_str = format!("v{pkg_ver}");
        let ver_vis = visible_width(&ver_str);
        let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
        let mini_header = format!(
            "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
            " ".repeat(pad),
            "─".repeat(bar_width.saturating_sub(4))
        );

        let owner_str =
            format!("\x1b[1;36m{owner_id}\x1b[0m \x1b[38;5;244m(Strict Whitelist)\x1b[0m");
        let count_str = format!("\x1b[1;32m{} facts recorded\x1b[0m", memories.len());
        let engine_str = "\x1b[38;5;252mSQLite (WAL mode) · Tier-1 Profile\x1b[0m";

        let hud_rows = [
            ("TARGET OWNER ID", owner_str.as_str()),
            ("STORED FACTS", count_str.as_str()),
            ("STORAGE ENGINE", engine_str),
        ];
        let hud =
            crate::cli::tui::render_hud_box("MEMORY SUBSYSTEM TELEMETRY", &hud_rows, bar_width);

        let title =
            format!("{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mSelect Memory Action:\x1b[0m");

        let items = vec![
            "Browse All Facts              (Inspect all key-value memory entries in detail)"
                .to_string(),
            "Delete a Fact Interactive      (Select and remove a single remembered key)"
                .to_string(),
            "Clear All Facts for Owner      (Wipe entire Tier-1 memory database with confirmation)"
                .to_string(),
            "Back to Main Menu              (Exit to Xiao Control Center)".to_string(),
        ];

        let sel = terminal_interactive_select(&title, &items, 0, false, None);
        let Some(idx) = sel else { break };

        match idx {
            0 => {
                crate::cli::tui::print_mini_header("Stored Long-Term Facts (Tier 1)");
                if memories.is_empty() {
                    println!("  \x1b[38;5;244m(No facts stored yet. Xiao will automatically remember important facts during conversations.)\x1b[0m\n");
                } else {
                    println!("  \x1b[1;37m{:<25} Remembered Fact\x1b[0m", "Key / Topic");
                    println!(
                        "  \x1b[38;5;238m{}\x1b[0m",
                        "─".repeat(bar_width.saturating_sub(4))
                    );
                    for (k, f) in &memories {
                        println!(
                            "  \x1b[1;38;5;45m{:<25}\x1b[0m \x1b[38;5;252m{}\x1b[0m",
                            k, f
                        );
                    }
                    println!(
                        "  \x1b[38;5;238m{}\x1b[0m\n",
                        "─".repeat(bar_width.saturating_sub(4))
                    );
                }
                crate::cli::tui::print_press_enter();
            }
            1 => {
                if memories.is_empty() {
                    println!("\n\x1b[33mNo facts currently recorded to delete.\x1b[0m\n");
                    crate::cli::tui::print_press_enter();
                    continue;
                }

                let mut fact_items: Vec<String> = memories
                    .iter()
                    .map(|(k, f)| {
                        let snippet = crate::util::truncate_chars_with_ellipsis(f, 45);
                        format!("{:<20} ({snippet})", k)
                    })
                    .collect();
                fact_items.push("Cancel / Back".to_string());

                let sub_sel = terminal_interactive_select(
                    "Select a fact to delete from memory:",
                    &fact_items,
                    0,
                    false,
                    None,
                );

                if let Some(fact_idx) = sub_sel {
                    if fact_idx < memories.len() {
                        let (target_key, _) = &memories[fact_idx];
                        if crate::ai::storage::delete_user_memory_async(
                            owner_id,
                            target_key.clone(),
                        )
                        .await
                        {
                            println!(
                                "\n\x1b[1;32m✔ Fact '{target_key}' successfully removed.\x1b[0m\n"
                            );
                        } else {
                            println!("\n\x1b[31m✖ Failed to remove fact '{target_key}'.\x1b[0m\n");
                        }
                        crate::cli::tui::print_press_enter();
                    }
                }
            }
            2 => {
                print!("\n\x1b[33mAre you sure you want to clear ALL facts for Owner ({owner_id})? [y/N]: \x1b[0m");
                let _ = io::stdout().flush();
                let mut confirm = String::new();
                let _ = io::stdin().read_line(&mut confirm);
                if confirm.trim().eq_ignore_ascii_case("y") {
                    if crate::ai::storage::clear_user_memories_async(owner_id).await {
                        println!("\n\x1b[1;32m✔ All long-term memories for Owner ({owner_id}) cleared.\x1b[0m\n");
                    } else {
                        println!("\n\x1b[31m✖ Failed to clear memories.\x1b[0m\n");
                    }
                } else {
                    println!("\n\x1b[38;5;244mOperation cancelled.\x1b[0m\n");
                }
                crate::cli::tui::print_press_enter();
            }
            _ => break,
        }
    }
}

pub(crate) async fn run_cli_memory(
    _ai_service: &AIChatService,
    action: Option<&str>,
    target: Option<&str>,
) {
    load_environment();

    let parsed = parse_memory_cli_action(action, target);
    match parsed {
        MemoryCliAction::Help => {
            let bar_width = crate::cli::tui::get_terminal_bar_width();
            crate::cli::tui::print_mini_header("Long-Term Memory › Command Reference");

            println!("\n  \x1b[1;37mUsage:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao memory\x1b[0m \x1b[38;5;245m<action>\x1b[0m \x1b[38;5;245m[target...]\x1b[0m\n");

            println!("  \x1b[1;38;2;6;182;212m▸ \x1b[1;37mACTIONS\x1b[0m");
            println!("    \x1b[1;38;5;45mlist\x1b[0m                      \x1b[38;5;250mDirectly output remembered long-term facts\x1b[0m");
            println!("    \x1b[1;38;5;45mmenu\x1b[0m, \x1b[38;5;244m(none)\x1b[0m              \x1b[38;5;250mOpen interactive long-term memory hub (TUI)\x1b[0m");
            println!("    \x1b[1;38;5;45mrm\x1b[0m, \x1b[1;38;5;45mremove\x1b[0m \x1b[38;5;245m<key>\x1b[0m           \x1b[38;5;250mRemove a specific remembered fact\x1b[0m");
            println!("    \x1b[1;38;5;45mclear\x1b[0m                     \x1b[38;5;250mWipe all remembered facts for the owner\x1b[0m");
            println!("    \x1b[1;38;5;45mhelp\x1b[0m, \x1b[1;38;5;45m-h\x1b[0m                  \x1b[38;5;250mShow this help reference\x1b[0m\n");

            println!(
                "  \x1b[38;5;238m{}\x1b[0m\n",
                "─".repeat(bar_width.saturating_sub(4))
            );

            println!("  \x1b[1;37mQuick Examples:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao memory\x1b[0m                       \x1b[38;5;242m# Interactive memory hub\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao memory list\x1b[0m                  \x1b[38;5;242m# Direct fact table output\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao memory rm user_language\x1b[0m      \x1b[38;5;242m# Remove specific fact\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao memory clear\x1b[0m                 \x1b[38;5;242m# Wipe all facts\x1b[0m\n");
        }
        MemoryCliAction::Unknown(unknown) => {
            println!("\n\x1b[31m✖ Error: Unknown action '{unknown}'.\x1b[0m");
            println!("  Usage: xiao memory [list|rm <key>|clear]\n");
            std::process::exit(1);
        }
        action => {
            let owner_id = get_configured_owner_id().unwrap_or(0);
            if owner_id == 0 {
                println!(
                    "\n\x1b[31m✖ OWNER_USER_ID is not configured. Run 'xiao gateway owner <ID>'.\x1b[0m\n"
                );
                crate::cli::tui::print_press_enter();
                return;
            }

            match action {
                MemoryCliAction::Clear => {
                    if crate::ai::storage::clear_user_memories_async(owner_id).await {
                        println!("\n\x1b[1;32m✔ All long-term memories (Tier 1) for Owner ({owner_id}) successfully cleared.\x1b[0m\n");
                    } else {
                        println!("\n\x1b[31m✖ Failed to clear user memories.\x1b[0m\n");
                        std::process::exit(1);
                    }
                }
                MemoryCliAction::Remove(Some(key)) => {
                    if crate::ai::storage::delete_user_memory_async(owner_id, key.to_string()).await
                    {
                        println!("\n\x1b[1;32m✔ Memory '{key}' successfully removed for Owner ({owner_id}).\x1b[0m\n");
                    } else {
                        println!("\n\x1b[31m✖ Failed to remove memory '{key}'.\x1b[0m\n");
                        std::process::exit(1);
                    }
                }
                MemoryCliAction::Remove(None) => {
                    if io::stdin().is_terminal() && io::stdout().is_terminal() {
                        let memories = crate::ai::storage::get_user_memories_async(owner_id).await;
                        if memories.is_empty() {
                            println!("\n\x1b[33mNo facts currently recorded to delete.\x1b[0m\n");
                            return;
                        }

                        let mut fact_items: Vec<String> = memories
                            .iter()
                            .map(|(k, f)| {
                                let snippet = crate::util::truncate_chars_with_ellipsis(f, 45);
                                format!("{:<20} ({snippet})", k)
                            })
                            .collect();
                        fact_items.push("Cancel / Back".to_string());

                        let sub_sel = terminal_interactive_select(
                            "Select a fact to delete from memory:",
                            &fact_items,
                            0,
                            false,
                            None,
                        );

                        if let Some(fact_idx) = sub_sel {
                            if fact_idx < memories.len() {
                                let (target_key, _) = &memories[fact_idx];
                                if crate::ai::storage::delete_user_memory_async(
                                    owner_id,
                                    target_key.clone(),
                                )
                                .await
                                {
                                    println!(
                                        "\n\x1b[1;32m✔ Memory '{target_key}' successfully removed for Owner ({owner_id}).\x1b[0m\n"
                                    );
                                } else {
                                    println!(
                                        "\n\x1b[31m✖ Failed to delete memory '{target_key}'.\x1b[0m\n"
                                    );
                                }
                            }
                        }
                    } else {
                        println!("\n\x1b[31m✖ Error: <key> parameter is required.\x1b[0m");
                        println!("  Usage: xiao memory rm <key>\n");
                        std::process::exit(1);
                    }
                }
                MemoryCliAction::Menu => {
                    if io::stdout().is_terminal() {
                        run_interactive_memory_menu(owner_id).await;
                    } else {
                        let memories = crate::ai::storage::get_user_memories_async(owner_id).await;
                        print_memory_fact_table(&memories, owner_id);
                    }
                }
                MemoryCliAction::List => {
                    let memories = crate::ai::storage::get_user_memories_async(owner_id).await;
                    print_memory_fact_table(&memories, owner_id);
                }
                MemoryCliAction::Help | MemoryCliAction::Unknown(_) => {}
            }
        }
    }
}

pub fn format_memory_fact_table(
    memories: &[(String, String)],
    owner_id: i64,
    bar_width: usize,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "  \x1b[38;5;245mOwner ID :\x1b[0m \x1b[1;37m{owner_id}\x1b[0m\n"
    ));
    out.push_str(&format!(
        "  \x1b[38;5;245mTotal    :\x1b[0m \x1b[1;37m{} facts remembered\x1b[0m\n\n",
        memories.len()
    ));

    if memories.is_empty() {
        out.push_str("  \x1b[38;5;244m(No facts stored yet. Xiao will automatically remember important facts during conversations.)\x1b[0m\n");
    } else {
        out.push_str(&format!(
            "  \x1b[1;37m{:<25} Remembered Fact\x1b[0m\n",
            "Key / Topic"
        ));
        out.push_str(&format!(
            "  \x1b[38;5;238m{}\x1b[0m\n",
            "─".repeat(bar_width.saturating_sub(4))
        ));
        for (key, fact) in memories {
            out.push_str(&format!(
                "  \x1b[1;38;5;45m{:<25}\x1b[0m \x1b[38;5;252m{}\x1b[0m\n",
                key, fact
            ));
        }
        out.push_str(&format!(
            "  \x1b[38;5;238m{}\x1b[0m\n",
            "─".repeat(bar_width.saturating_sub(4))
        ));
        out.push_str(
            "  \x1b[38;5;244mManage: 'xiao memory rm <key>' or 'xiao memory clear'\x1b[0m\n",
        );
    }
    out
}

pub fn print_memory_fact_table(memories: &[(String, String)], owner_id: i64) {
    let bar_width = get_terminal_bar_width();
    crate::cli::tui::print_mini_header("Xiao Long-Term Memory (Tier 1 Facts)");
    print!(
        "{}",
        format_memory_fact_table(memories, owner_id, bar_width)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_cli_action_parsing() {
        assert_eq!(parse_memory_cli_action(None, None), MemoryCliAction::Menu);
        assert_eq!(
            parse_memory_cli_action(Some("menu"), None),
            MemoryCliAction::Menu
        );
        assert_eq!(
            parse_memory_cli_action(Some("list"), None),
            MemoryCliAction::List
        );
        assert_eq!(
            parse_memory_cli_action(Some("clear"), None),
            MemoryCliAction::Clear
        );
        assert_eq!(
            parse_memory_cli_action(Some("rm"), Some("pref")),
            MemoryCliAction::Remove(Some("pref"))
        );
        assert_eq!(
            parse_memory_cli_action(Some("help"), None),
            MemoryCliAction::Help
        );
    }

    #[test]
    fn test_format_memory_fact_table() {
        let empty_table = format_memory_fact_table(&[], 12345, 80);
        assert!(empty_table.contains("Owner ID"));
        assert!(empty_table.contains("12345"));
        assert!(empty_table.contains("0 facts remembered"));
        assert!(empty_table.contains("No facts stored yet"));

        let facts = vec![
            ("preferred_language".to_string(), "Rust".to_string()),
            ("project_name".to_string(), "XiaoBot".to_string()),
        ];
        let populated_table = format_memory_fact_table(&facts, 12345, 80);
        assert!(populated_table.contains("preferred_language"));
        assert!(populated_table.contains("Rust"));
        assert!(populated_table.contains("project_name"));
        assert!(populated_table.contains("XiaoBot"));
        assert!(populated_table.contains("2 facts remembered"));
    }

    #[test]
    fn test_unicode_safe_fact_truncation() {
        let long_fact =
            "这是一段非常长的中文事实记录，用来测试截断是否会出现字节边界错误！".repeat(2);
        let truncated_fact = crate::util::truncate_chars_with_ellipsis(&long_fact, 45);
        assert!(truncated_fact.ends_with("..."));
        assert_eq!(truncated_fact.chars().count(), 48);

        let misaligned_fact = format!("a{}", "这是一段非常长的事实记录".repeat(5));
        let safe_snippet = crate::util::truncate_chars_with_ellipsis(&misaligned_fact, 45);
        assert!(safe_snippet.ends_with("..."));
    }
}
