use std::io::{self, IsTerminal, Write};

use crate::ai::AIChatService;
use crate::cli::tui::{get_terminal_bar_width, terminal_interactive_select, visible_width};
use crate::load_environment;

#[derive(Debug, PartialEq, Eq)]
pub enum SearchCliAction<'a> {
    Status,
    Test(Option<&'a str>),
    Brave(Option<&'a str>),
    Tavily(Option<&'a str>),
    Exa(Option<&'a str>),
    Engine(Option<&'a str>),
    Help,
    Unknown(&'a str),
}

pub fn parse_search_cli_action<'a>(
    action: Option<&'a str>,
    target: Option<&'a str>,
) -> SearchCliAction<'a> {
    match action {
        None | Some("status") | Some("menu") => SearchCliAction::Status,
        Some("help") | Some("--help") | Some("-h") => SearchCliAction::Help,
        Some("test") | Some("query") | Some("check") => SearchCliAction::Test(target),
        Some("brave") => SearchCliAction::Brave(target),
        Some("tavily") => SearchCliAction::Tavily(target),
        Some("exa") => SearchCliAction::Exa(target),
        Some("engine") | Some("use") => SearchCliAction::Engine(target),
        Some(unknown) => SearchCliAction::Unknown(unknown),
    }
}

pub fn parse_search_args(args: &[String]) -> (Option<&str>, Option<String>) {
    if args.is_empty() {
        return (None, None);
    }
    let first = args[0].as_str();
    match first {
        "status" | "menu" => (Some(first), None),
        "help" | "--help" | "-h" => (Some(first), None),
        "brave" | "tavily" | "exa" | "engine" | "use" => {
            let target = if args.len() > 1 {
                Some(args[1..].join(" "))
            } else {
                None
            };
            (Some(first), target)
        }
        "test" | "query" | "check" => {
            let target = if args.len() > 1 {
                Some(args[1..].join(" "))
            } else {
                None
            };
            (Some(first), target)
        }
        unknown => {
            if unknown.starts_with('-') {
                (Some(first), None)
            } else {
                (Some("test"), Some(args.join(" ")))
            }
        }
    }
}

pub fn mask_api_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        "(not set)".to_string()
    } else if trimmed.len() <= 8 {
        "••••••••".to_string()
    } else {
        let prefix: String = trimmed.chars().take(4).collect();
        let suffix: String = trimmed
            .chars()
            .skip(trimmed.len().saturating_sub(4))
            .collect();
        format!("{prefix}••••{suffix}")
    }
}

fn handle_search_key(key_name: &str, provider_label: &str, target: Option<&str>) {
    match target {
        Some("rm") | Some("remove") | Some("clear") => {
            if crate::ai::service::save_app_setting(key_name, "").is_ok() {
                println!("\n\x1b[1;32m✔ {provider_label} key successfully removed.\x1b[0m\n");
            } else {
                println!("\n\x1b[31m✖ Failed to remove {provider_label} key.\x1b[0m\n");
                std::process::exit(1);
            }
        }
        Some(new_key) if !new_key.trim().is_empty() => {
            let trimmed = new_key.trim();
            if crate::ai::service::save_app_setting(key_name, trimmed).is_ok() {
                println!(
                    "\n\x1b[1;32m✔ {provider_label} key successfully saved:\x1b[0m {}\n",
                    mask_api_key(trimmed)
                );
            } else {
                println!("\n\x1b[31m✖ Failed to save {provider_label} key to database.\x1b[0m\n");
                std::process::exit(1);
            }
        }
        _ => {
            let current = match key_name {
                "BRAVE_API_KEY" => crate::ai::tools::get_brave_key(),
                "TAVILY_API_KEY" => crate::ai::tools::get_tavily_key(),
                "EXA_API_KEY" => crate::ai::tools::get_exa_key(),
                _ => None,
            };
            println!("\n\x1b[1;36m{} Status\x1b[0m", provider_label);
            println!(
                "  Current Key : {}",
                current
                    .as_deref()
                    .map(mask_api_key)
                    .unwrap_or_else(|| "(not set)".to_string())
            );
            if io::stdout().is_terminal() {
                print!("\nEnter new API key (or 'rm' to remove, Enter to cancel): ");
                let _ = io::stdout().flush();
                let mut input = String::new();
                if io::stdin().read_line(&mut input).is_ok() {
                    let trimmed = input.trim();
                    if trimmed == "rm" || trimmed == "remove" || trimmed == "clear" {
                        if crate::ai::service::save_app_setting(key_name, "").is_ok() {
                            println!(
                                "\n\x1b[1;32m✔ {provider_label} key successfully removed.\x1b[0m\n"
                            );
                        } else {
                            println!("\n\x1b[31m✖ Failed to remove {provider_label} key.\x1b[0m\n");
                            std::process::exit(1);
                        }
                    } else if !trimmed.is_empty() {
                        if crate::ai::service::save_app_setting(key_name, trimmed).is_ok() {
                            println!(
                                "\n\x1b[1;32m✔ {provider_label} key successfully saved:\x1b[0m {}\n",
                                mask_api_key(trimmed)
                            );
                        } else {
                            println!("\n\x1b[31m✖ Failed to save {provider_label} key to database.\x1b[0m\n");
                            std::process::exit(1);
                        }
                    }
                }
            } else {
                let cmd_prefix = key_name
                    .split('_')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                println!("\n\x1b[38;5;244mUsage:\x1b[0m");
                println!("  xiao search {cmd_prefix} <API_KEY>    - Set API key");
                println!("  xiao search {cmd_prefix} rm           - Remove API key\n");
            }
        }
    }
}

async fn run_cli_configure_search_keys_submenu() {
    loop {
        let brave_key = crate::ai::tools::get_brave_key();
        let tavily_key = crate::ai::tools::get_tavily_key();
        let exa_key = crate::ai::tools::get_exa_key();

        let brave_mask = brave_key
            .as_deref()
            .map(mask_api_key)
            .unwrap_or_else(|| "(not set)".to_string());
        let tavily_mask = tavily_key
            .as_deref()
            .map(mask_api_key)
            .unwrap_or_else(|| "(not set)".to_string());
        let exa_mask = exa_key
            .as_deref()
            .map(mask_api_key)
            .unwrap_or_else(|| "(not set)".to_string());

        let bar_width = crate::cli::tui::get_terminal_bar_width();
        let pkg_ver = env!("CARGO_PKG_VERSION");
        let title_left =
            "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › Search › API Keys\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 24;
        let ver_str = format!("v{pkg_ver}");
        let ver_vis = crate::cli::tui::visible_width(&ver_str);
        let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
        let mini_header = format!(
            "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
            " ".repeat(pad),
            "─".repeat(bar_width.saturating_sub(4))
        );

        let hud_rows = [
            ("BRAVE SEARCH", brave_mask.as_str()),
            ("TAVILY SEARCH", tavily_mask.as_str()),
            ("EXA REST", exa_mask.as_str()),
        ];
        let hud =
            crate::cli::tui::render_hud_box("SEARCH API KEYS TELEMETRY", &hud_rows, bar_width);

        let title = format!(
            "{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mSelect Provider to Configure:\x1b[0m"
        );

        let items = vec![
            "Brave Search API           (Fast privacy-focused web search)".to_string(),
            "Tavily Search API          (AI-agent optimized search engine)".to_string(),
            "Exa REST API               (Neural semantic search engine)".to_string(),
            "Back to Search Menu        (Return to Web Search Hub)".to_string(),
        ];

        let sel = terminal_interactive_select(&title, &items, 0, false, None);
        let Some(idx) = sel else { break };

        let (key_name, label) = match idx {
            0 => ("BRAVE_API_KEY", "Brave Search API"),
            1 => ("TAVILY_API_KEY", "Tavily Search API"),
            2 => ("EXA_API_KEY", "Exa REST API"),
            _ => break,
        };

        println!("\n\x1b[1;36mConfigure {label}\x1b[0m");
        print!("Enter new API key (or 'rm' to remove, or press Enter to cancel): ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input);
        let trimmed = input.trim();

        if trimmed.is_empty() {
            continue;
        }

        if trimmed.eq_ignore_ascii_case("rm") || trimmed.eq_ignore_ascii_case("remove") {
            if crate::ai::service::save_app_setting(key_name, "").is_ok() {
                println!("\x1b[1;32m✔ {label} key removed.\x1b[0m\n");
            } else {
                println!("\x1b[31m✖ Failed to remove {label} key.\x1b[0m\n");
            }
        } else if crate::ai::service::save_app_setting(key_name, trimmed).is_ok() {
            println!(
                "\x1b[1;32m✔ {label} key saved:\x1b[0m {}\n",
                mask_api_key(trimmed)
            );
        } else {
            println!("\x1b[31m✖ Failed to save {label} key.\x1b[0m\n");
        }

        crate::cli::tui::print_press_enter();
    }
}

async fn run_interactive_search_menu() {
    loop {
        load_environment();
        let (search_engine_str, _) = crate::ai::tools::get_search_engine_status();
        let brave_key = crate::ai::tools::get_brave_key();
        let tavily_key = crate::ai::tools::get_tavily_key();
        let exa_key = crate::ai::tools::get_exa_key();

        let brave_status = if brave_key.is_some() {
            "\x1b[1;32m● Set\x1b[0m"
        } else {
            "\x1b[38;5;244m○ None\x1b[0m"
        };
        let tavily_status = if tavily_key.is_some() {
            "\x1b[1;32m● Set\x1b[0m"
        } else {
            "\x1b[38;5;244m○ None\x1b[0m"
        };
        let exa_status = if exa_key.is_some() {
            "\x1b[1;32m● Set\x1b[0m"
        } else {
            "\x1b[38;5;244m○ None\x1b[0m"
        };

        let bar_width = get_terminal_bar_width();
        let pkg_ver = env!("CARGO_PKG_VERSION");
        let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › Web Search Engine Hub\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 28;
        let ver_str = format!("v{pkg_ver}");
        let ver_vis = visible_width(&ver_str);
        let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
        let mini_header = format!(
            "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
            " ".repeat(pad),
            "─".repeat(bar_width.saturating_sub(4))
        );

        let keys_val = format!(
            "\x1b[38;5;252mBrave: {} \x1b[38;5;244m·\x1b[0m \x1b[38;5;252mTavily: {} \x1b[38;5;244m·\x1b[0m \x1b[38;5;252mExa: {}\x1b[0m",
            brave_status, tavily_status, exa_status
        );
        let fallback_val = "\x1b[38;5;250mDuckDuckGo \u{2192} Wikipedia Knowledge Base\x1b[0m";

        let hud_rows = [
            ("ACTIVE ENGINE", search_engine_str.as_str()),
            ("CONFIGURED KEYS", keys_val.as_str()),
            ("SEARCH FALLBACK", fallback_val),
        ];
        let hud = crate::cli::tui::render_hud_box("SEARCH ENGINE TELEMETRY", &hud_rows, bar_width);

        let title =
            format!("{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mSelect Search Action:\x1b[0m");

        let items = vec![
            "Test Web Search Query         (Execute live query with active search engine)"
                .to_string(),
            "Configure Search API Keys      (Brave Search, Tavily AI, Exa REST)".to_string(),
            "Back to Main Menu              (Exit to Xiao Control Center)".to_string(),
        ];

        let sel = terminal_interactive_select(&title, &items, 0, false, None);
        let Some(idx) = sel else { break };

        match idx {
            0 => {
                println!("\n\x1b[1;36mTest Web Search Query\x1b[0m");
                print!("Enter search query [default: 'Rust 2021 edition release notes']: ");
                let _ = io::stdout().flush();
                let mut query_input = String::new();
                let _ = io::stdin().read_line(&mut query_input);
                let trimmed = query_input.trim();
                let query = if trimmed.is_empty() {
                    "Rust 2021 edition release notes"
                } else {
                    trimmed
                };

                println!("\n\x1b[1;36mExecuting Web Search...\x1b[0m");
                println!("  Active Engine : {}", search_engine_str);
                println!("  Query         : {}\n", query);
                let start = std::time::Instant::now();
                let result = crate::ai::tools::execute_web_search(query).await;
                let elapsed = start.elapsed().as_millis();
                if result.starts_with("Error")
                    || result.contains("tidak dapat menemukan hasil")
                    || result.contains("could not find results")
                    || result.contains("no results found")
                {
                    println!(
                        "\x1b[38;5;214m◈\x1b[0m \x1b[33mSearch finished ({elapsed}ms) with message:\x1b[0m\n{}\n",
                        result.trim()
                    );
                } else {
                    println!(
                        "\x1b[1;32m✔ Successfully retrieved search results ({elapsed}ms)\x1b[0m\n"
                    );
                    let preview = crate::util::truncate_chars(&result, 500);
                    println!(
                        "\x1b[38;5;244mResult Snippet:\x1b[0m\n{}\x1b[38;5;244m...\x1b[0m\n",
                        preview.trim()
                    );
                }

                crate::cli::tui::print_press_enter();
            }
            1 => {
                run_cli_configure_search_keys_submenu().await;
            }
            _ => break,
        }
    }
}

pub(crate) async fn run_cli_search_hub(
    _ai_service: &AIChatService,
    action: Option<&str>,
    target: Option<&str>,
) {
    load_environment();
    let (search_engine_str, _) = crate::ai::tools::get_search_engine_status();
    let brave_key = crate::ai::tools::get_brave_key();
    let tavily_key = crate::ai::tools::get_tavily_key();
    let exa_key = crate::ai::tools::get_exa_key();

    match parse_search_cli_action(action, target) {
        SearchCliAction::Status => {
            if io::stdin().is_terminal() && io::stdout().is_terminal() {
                run_interactive_search_menu().await;
                return;
            }

            crate::cli::tui::print_mini_header("Web Search Engine Hub");
            println!(
                "  \x1b[38;5;245mActive Engine :\x1b[0m \x1b[1;37m{}\x1b[0m",
                search_engine_str
            );
            println!("\n  \x1b[1;37mConfigured Search Providers:\x1b[0m");
            println!(
                "    \x1b[38;5;245mBrave Search API :\x1b[0m {}",
                brave_key
                    .as_deref()
                    .map(mask_api_key)
                    .unwrap_or_else(|| "\x1b[38;5;244m(not set)\x1b[0m".to_string())
            );
            println!(
                "    \x1b[38;5;245mTavily Search API:\x1b[0m {}",
                tavily_key
                    .as_deref()
                    .map(mask_api_key)
                    .unwrap_or_else(|| "\x1b[38;5;244m(not set)\x1b[0m".to_string())
            );
            println!(
                "    \x1b[38;5;245mExa REST API     :\x1b[0m {}",
                exa_key
                    .as_deref()
                    .map(mask_api_key)
                    .unwrap_or_else(|| "\x1b[38;5;244m(not set)\x1b[0m".to_string())
            );
            println!(
                "    \x1b[38;5;245mKeyless Fallbacks:\x1b[0m \x1b[38;5;252mDuckDuckGo \u{2192} Wikipedia\x1b[0m\n"
            );

            println!("\x1b[38;5;244mSubcommands:\x1b[0m");
            println!("  xiao search test <query>    - Test web search query with active engine");
            println!("  xiao search brave [KEY|rm]  - Configure or remove Brave Search API key");
            println!("  xiao search tavily [KEY|rm] - Configure or remove Tavily Search API key");
            println!("  xiao search exa [KEY|rm]    - Configure or remove Exa REST API key\n");
        }
        SearchCliAction::Test(tgt) => {
            let query = tgt.unwrap_or("Rust 2021 edition release notes");
            println!("\n\x1b[1;36mTesting Web Search Pipeline...\x1b[0m");
            println!("  Active Engine : {}", search_engine_str);
            println!("  Query         : {}\n", query);
            let start = std::time::Instant::now();
            let result = crate::ai::tools::execute_web_search(query).await;
            let elapsed = start.elapsed().as_millis();
            if result.starts_with("Error")
                || result.contains("tidak dapat menemukan hasil")
                || result.contains("could not find results")
                || result.contains("no results found")
            {
                println!(
                    "\x1b[38;5;214m◈\x1b[0m \x1b[33mSearch finished ({elapsed}ms) with message:\x1b[0m\n{}\n",
                    result.trim()
                );
            } else {
                println!(
                    "\x1b[1;32m✔ Successfully retrieved search results ({elapsed}ms)\x1b[0m\n"
                );
                let preview = crate::util::truncate_chars(&result, 500);
                println!(
                    "\x1b[38;5;244mResult Snippet:\x1b[0m\n{}\x1b[38;5;244m...\x1b[0m\n",
                    preview.trim()
                );
            }
        }
        SearchCliAction::Brave(tgt) => {
            handle_search_key("BRAVE_API_KEY", "Brave Search API", tgt);
        }
        SearchCliAction::Tavily(tgt) => {
            handle_search_key("TAVILY_API_KEY", "Tavily Search API", tgt);
        }
        SearchCliAction::Exa(tgt) => {
            handle_search_key("EXA_API_KEY", "Exa REST API", tgt);
        }
        SearchCliAction::Engine(tgt) => {
            println!("  Active search engine: \x1b[1;37m{search_engine_str}\x1b[0m");
            if let Some(_eng) = tgt {
                println!("\n  \x1b[38;5;244mNote: Search engine is selected automatically based on configured API keys:\x1b[0m");
                println!("    1. Brave Search  (\x1b[1;37mxiao search brave <KEY>\x1b[0m)");
                println!("    2. Tavily Search (\x1b[1;37mxiao search tavily <KEY>\x1b[0m)");
                println!("    3. Exa REST API  (\x1b[1;37mxiao search exa <KEY>\x1b[0m)");
                println!("    4. Exa MCP / DuckDuckGo fallback (Keyless)\n");
            } else {
                println!();
            }
        }
        SearchCliAction::Help => {
            let bar_width = crate::cli::tui::get_terminal_bar_width();
            crate::cli::tui::print_mini_header("Web Search › Command Reference");

            println!("\n  \x1b[1;37mUsage:\x1b[0m");
            println!(
                "    \x1b[1;38;5;45mxiao search\x1b[0m \x1b[38;5;245m<action>\x1b[0m \x1b[38;5;245m[target...]\x1b[0m\n"
            );

            println!("  \x1b[1;38;2;6;182;212m▸ \x1b[1;37mACTIONS\x1b[0m");
            println!("    \x1b[1;38;5;45mstatus\x1b[0m, \x1b[38;5;244m(none)\x1b[0m             \x1b[38;5;250mDisplay active search engine status & provider keys\x1b[0m");
            println!("    \x1b[1;38;5;45mtest\x1b[0m \x1b[38;5;245m<query>\x1b[0m               \x1b[38;5;250mExecute live web search query\x1b[0m");
            println!("    \x1b[1;38;5;45mbrave\x1b[0m \x1b[38;5;245m[KEY|rm]\x1b[0m            \x1b[38;5;250mConfigure or remove Brave Search API key\x1b[0m");
            println!("    \x1b[1;38;5;45mtavily\x1b[0m \x1b[38;5;245m[KEY|rm]\x1b[0m           \x1b[38;5;250mConfigure or remove Tavily Search API key\x1b[0m");
            println!("    \x1b[1;38;5;45mexa\x1b[0m \x1b[38;5;245m[KEY|rm]\x1b[0m              \x1b[38;5;250mConfigure or remove Exa REST API key\x1b[0m");
            println!("    \x1b[1;38;5;45mengine\x1b[0m                    \x1b[38;5;250mDisplay active engine and priority order\x1b[0m");
            println!("    \x1b[1;38;5;45mhelp\x1b[0m, \x1b[1;38;5;45m-h\x1b[0m                  \x1b[38;5;250mShow this help reference\x1b[0m\n");

            println!(
                "  \x1b[38;5;238m{}\x1b[0m\n",
                "─".repeat(bar_width.saturating_sub(4))
            );

            println!("  \x1b[1;37mQuick Examples:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao search test \"Rust async await\"\x1b[0m  \x1b[38;5;242m# Live search test\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao search brave BSA...                 \x1b[38;5;242m# Save Brave Search key\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao search brave                        \x1b[38;5;242m# Interactively set key\x1b[0m\n");
        }
        SearchCliAction::Unknown(unknown) => {
            // If unknown doesn't start with '-', treat it as a search query!
            if !unknown.starts_with('-') {
                println!("\n\x1b[1;36mExecuting Web Search for: '{unknown}'...\x1b[0m");
                let result = crate::ai::tools::execute_web_search(unknown).await;
                println!("{}\n", result.trim());
                return;
            }
            println!("\n\x1b[31m✖ Error: Unknown search action 'search {unknown}'.\x1b[0m");
            println!("  Run 'xiao search help' or 'xiao help' for usage instructions.\n");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_search_cli_action() {
        assert_eq!(parse_search_cli_action(None, None), SearchCliAction::Status);
        assert_eq!(
            parse_search_cli_action(Some("status"), None),
            SearchCliAction::Status
        );
        assert_eq!(
            parse_search_cli_action(Some("menu"), None),
            SearchCliAction::Status
        );
        assert_eq!(
            parse_search_cli_action(Some("help"), None),
            SearchCliAction::Help
        );
        assert_eq!(
            parse_search_cli_action(Some("--help"), None),
            SearchCliAction::Help
        );
        assert_eq!(
            parse_search_cli_action(Some("-h"), None),
            SearchCliAction::Help
        );
        assert_eq!(
            parse_search_cli_action(Some("test"), Some("rust async")),
            SearchCliAction::Test(Some("rust async"))
        );
        assert_eq!(
            parse_search_cli_action(Some("brave"), Some("key123")),
            SearchCliAction::Brave(Some("key123"))
        );
        assert_eq!(
            parse_search_cli_action(Some("tavily"), Some("tvly123")),
            SearchCliAction::Tavily(Some("tvly123"))
        );
        assert_eq!(
            parse_search_cli_action(Some("exa"), Some("exa123")),
            SearchCliAction::Exa(Some("exa123"))
        );
        assert_eq!(
            parse_search_cli_action(Some("engine"), Some("brave")),
            SearchCliAction::Engine(Some("brave"))
        );
        assert_eq!(
            parse_search_cli_action(Some("foo"), None),
            SearchCliAction::Unknown("foo")
        );
    }

    #[test]
    fn test_multi_word_search_query_parsing() {
        let args1 = vec![
            "test".to_string(),
            "query".to_string(),
            "with".to_string(),
            "spaces".to_string(),
        ];
        assert_eq!(
            parse_search_args(&args1),
            (Some("test"), Some("query with spaces".to_string()))
        );

        let args2 = vec![
            "query".to_string(),
            "another".to_string(),
            "multi".to_string(),
            "word".to_string(),
        ];
        assert_eq!(
            parse_search_args(&args2),
            (Some("query"), Some("another multi word".to_string()))
        );

        let args3 = vec![
            "unquoted".to_string(),
            "direct".to_string(),
            "search".to_string(),
        ];
        assert_eq!(
            parse_search_args(&args3),
            (Some("test"), Some("unquoted direct search".to_string()))
        );

        let args4 = vec!["brave".to_string(), "MY_BRAVE_KEY".to_string()];
        assert_eq!(
            parse_search_args(&args4),
            (Some("brave"), Some("MY_BRAVE_KEY".to_string()))
        );

        let args5 = vec!["status".to_string()];
        assert_eq!(parse_search_args(&args5), (Some("status"), None));

        let args6: Vec<String> = vec![];
        assert_eq!(parse_search_args(&args6), (None, None));
    }

    #[test]
    fn test_mask_api_key() {
        assert_eq!(mask_api_key(""), "(not set)");
        assert_eq!(mask_api_key("   "), "(not set)");
        assert_eq!(mask_api_key("12345678"), "••••••••");
        assert_eq!(mask_api_key("123456789"), "1234••••6789");
        assert_eq!(mask_api_key("sk-ant-api03-abcdefghijklmn"), "sk-a••••klmn");
    }
}
