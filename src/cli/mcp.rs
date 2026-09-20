use std::io::{self, IsTerminal, Write};

use crate::ai::AIChatService;
use crate::cli::tui::{get_terminal_bar_width, terminal_interactive_select, visible_width};
use crate::load_environment;

#[derive(Debug, PartialEq, Eq)]
pub enum McpCliAction<'a> {
    Status,
    Help,
    Tools,
    Url(Option<&'a str>),
    Test(Option<&'a str>),
    Search(Option<&'a str>),
    Brave(Option<&'a str>),
    Tavily(Option<&'a str>),
    Exa(Option<&'a str>),
    Reset,
    Unknown(&'a str),
}

pub fn parse_mcp_cli_action<'a>(
    action: Option<&'a str>,
    target: Option<&'a str>,
) -> McpCliAction<'a> {
    match action {
        None | Some("status") | Some("list") => McpCliAction::Status,
        Some("help") | Some("--help") | Some("-h") => McpCliAction::Help,
        Some("tools") => McpCliAction::Tools,
        Some("url") | Some("set") => McpCliAction::Url(target),
        Some("test") | Some("check") => McpCliAction::Test(target),
        Some("search") => McpCliAction::Search(target),
        Some("brave") => McpCliAction::Brave(target),
        Some("tavily") => McpCliAction::Tavily(target),
        Some("exa") => McpCliAction::Exa(target),
        Some("reset") => McpCliAction::Reset,
        Some(unknown) => McpCliAction::Unknown(unknown),
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

fn handle_search_key_subcommand(key_name: &str, provider_label: &str, target: Option<&str>) {
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
                println!("  xiao mcp {cmd_prefix} <API_KEY>    - Set API key");
                println!("  xiao mcp {cmd_prefix} rm           - Remove API key\n");
            }
        }
    }
}

pub fn print_tools_summary() {
    crate::cli::tui::print_mini_header("Registered Function Calling Tools");
    let tools_value = crate::ai::tools::get_tools_definition();
    if let Some(tools_arr) = tools_value.as_array() {
        for item in tools_arr {
            let func = item.get("function").unwrap_or(item);
            let name = func
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let desc = func
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            println!("  \x1b[1;32m▸ {}\x1b[0m", name);
            println!("     \x1b[38;5;252m{}\x1b[0m", desc.trim());
            if let Some(params) = func.get("parameters") {
                if let Ok(pretty) = serde_json::to_string_pretty(params) {
                    for line in pretty.lines() {
                        println!("     \x1b[38;5;244m{}\x1b[0m", line);
                    }
                }
            }
            println!();
        }
    }
}

async fn run_cli_configure_keys_submenu() {
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
        let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › MCP › Search API Keys\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 28;
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
            "Back to MCP Menu           (Return to Search & MCP Hub)".to_string(),
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

        print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
        let _ = io::stdout().flush();
        let mut tmp = String::new();
        let _ = io::stdin().read_line(&mut tmp);
    }
}

async fn run_interactive_mcp_menu() {
    loop {
        load_environment();
        let current_mcp_url = crate::ai::tools::get_configured_mcp_url();
        let (search_engine_str, _) = crate::ai::tools::get_search_engine_status();
        let brave_key = crate::ai::tools::get_brave_key();
        let tavily_key = crate::ai::tools::get_tavily_key();
        let exa_key = crate::ai::tools::get_exa_key();

        let brave_mask = brave_key
            .as_deref()
            .map(mask_api_key)
            .unwrap_or_else(|| "not set".to_string());
        let tavily_mask = tavily_key
            .as_deref()
            .map(mask_api_key)
            .unwrap_or_else(|| "not set".to_string());
        let exa_mask = exa_key
            .as_deref()
            .map(mask_api_key)
            .unwrap_or_else(|| "not set".to_string());

        let bar_width = get_terminal_bar_width();
        let pkg_ver = env!("CARGO_PKG_VERSION");
        let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › Search Engine & MCP Tool Hub\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 35;
        let ver_str = format!("v{pkg_ver}");
        let ver_vis = visible_width(&ver_str);
        let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
        let mini_header = format!(
            "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
            " ".repeat(pad),
            "─".repeat(bar_width.saturating_sub(4))
        );

        let keys_val = format!(
            "\x1b[38;5;252mBrave: \x1b[1;36m{}\x1b[0m \x1b[38;5;244m·\x1b[0m \x1b[38;5;252mTavily: \x1b[1;36m{}\x1b[0m \x1b[38;5;244m·\x1b[0m \x1b[38;5;252mExa: \x1b[1;36m{}\x1b[0m",
            brave_mask, tavily_mask, exa_mask
        );

        let hud_rows = [
            ("ACTIVE ENGINE", search_engine_str.as_str()),
            ("MCP ENDPOINT", current_mcp_url.as_str()),
            ("SEARCH KEYS", keys_val.as_str()),
        ];
        let hud =
            crate::cli::tui::render_hud_box("SEARCH ENGINE & MCP TELEMETRY", &hud_rows, bar_width);

        let title = format!("{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mSelect Action:\x1b[0m");

        let items = vec![
            "Test Web Search Pipeline        (Execute live query with active search engine)"
                .to_string(),
            "Probe Exa MCP Protocol          (Direct JSON-RPC health check to MCP endpoint)"
                .to_string(),
            "Configure Search API Keys       (Brave Search, Tavily Search, Exa REST)".to_string(),
            "Set Custom MCP Endpoint         (Configure custom endpoint URL with SSRF guard)"
                .to_string(),
            "View Registered Tools & Schema   (Inspect function schemas exposed to LLM)"
                .to_string(),
            "Reset MCP to Default            (Restore official https://mcp.exa.ai/)".to_string(),
            "Back to Main Menu               (Exit to Xiao Control Center)".to_string(),
        ];

        let sel = terminal_interactive_select(&title, &items, 0, false, None);
        let Some(idx) = sel else { break };

        match idx {
            0 => {
                println!("\n\x1b[1;36mTest Web Search Pipeline\x1b[0m");
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
                    let preview = if result.len() > 500 {
                        &result[..500]
                    } else {
                        &result
                    };
                    println!(
                        "\x1b[38;5;244mResult Snippet:\x1b[0m\n{}\x1b[38;5;244m...\x1b[0m\n",
                        preview.trim()
                    );
                }

                print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                let _ = io::stdout().flush();
                let mut tmp = String::new();
                let _ = io::stdin().read_line(&mut tmp);
            }
            1 => {
                println!("\n\x1b[1;36mProbe Exa MCP Endpoint\x1b[0m");
                print!("Enter probe query [default: 'Rust 2021 edition release notes']: ");
                let _ = io::stdout().flush();
                let mut query_input = String::new();
                let _ = io::stdin().read_line(&mut query_input);
                let trimmed = query_input.trim();
                let query = if trimmed.is_empty() {
                    "Rust 2021 edition release notes"
                } else {
                    trimmed
                };

                println!("\n\x1b[1;36mProbing MCP Endpoint...\x1b[0m");
                println!("  Endpoint : {}", current_mcp_url);
                println!("  Query    : {}\n", query);
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()
                    .unwrap_or_default();
                let start = std::time::Instant::now();
                match crate::ai::tools::search_exa_mcp(&client, &current_mcp_url, query).await {
                    Ok(result) => {
                        let elapsed = start.elapsed().as_millis();
                        println!(
                            "\x1b[1;32m✔ Successfully connected to MCP ({elapsed}ms)\x1b[0m\n"
                        );
                        let preview = if result.len() > 400 {
                            &result[..400]
                        } else {
                            &result
                        };
                        println!(
                            "\x1b[38;5;244mResponse Snippet:\x1b[0m\n{}\x1b[38;5;244m...\x1b[0m\n",
                            preview.trim()
                        );
                    }
                    Err(err) => {
                        let elapsed = start.elapsed().as_millis();
                        println!(
                            "\x1b[31m✖ Failed to probe MCP ({elapsed}ms): {}\x1b[0m\n",
                            err
                        );
                    }
                }

                print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                let _ = io::stdout().flush();
                let mut tmp = String::new();
                let _ = io::stdin().read_line(&mut tmp);
            }
            2 => {
                run_cli_configure_keys_submenu().await;
            }
            3 => {
                println!("\n\x1b[1;36mSet Custom MCP Endpoint URL\x1b[0m");
                print!("Enter endpoint URL (e.g. https://mcp.exa.ai/): ");
                let _ = io::stdout().flush();
                let mut url_input = String::new();
                let _ = io::stdin().read_line(&mut url_input);
                let trimmed = url_input.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match crate::bot::url_policy::resolve_download_url(trimmed).await {
                    Ok(_) => {
                        if crate::ai::service::save_app_setting("EXA_MCP_URL", trimmed).is_ok() {
                            println!(
                                "\n\x1b[1;32m✔ MCP endpoint successfully saved:\x1b[0m {}\n",
                                trimmed
                            );
                        } else {
                            println!(
                                "\n\x1b[31m✖ Failed to save MCP configuration to database.\x1b[0m\n"
                            );
                        }
                    }
                    Err(err) => {
                        println!(
                            "\n\x1b[31m✖ URL rejected by security policy (SSRF/Protocol): {}\x1b[0m\n",
                            err
                        );
                    }
                }
                print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                let _ = io::stdout().flush();
                let mut tmp = String::new();
                let _ = io::stdin().read_line(&mut tmp);
            }
            4 => {
                print_tools_summary();
                print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                let _ = io::stdout().flush();
                let mut tmp = String::new();
                let _ = io::stdin().read_line(&mut tmp);
            }
            5 => {
                let default_url = "https://mcp.exa.ai/";
                if crate::ai::service::save_app_setting("EXA_MCP_URL", default_url).is_ok() {
                    println!(
                        "\n\x1b[1;32m✔ MCP endpoint successfully reset to default:\x1b[0m {}\n",
                        default_url
                    );
                } else {
                    println!("\n\x1b[31m✖ Failed to reset MCP configuration.\x1b[0m\n");
                }
                print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                let _ = io::stdout().flush();
                let mut tmp = String::new();
                let _ = io::stdin().read_line(&mut tmp);
            }
            _ => break,
        }
    }
}

pub(crate) async fn run_cli_mcp_hub(
    _ai_service: &AIChatService,
    action: Option<&str>,
    target: Option<&str>,
) {
    load_environment();
    let current_mcp_url = crate::ai::tools::get_configured_mcp_url();
    let (search_engine_str, _) = crate::ai::tools::get_search_engine_status();
    let brave_key = crate::ai::tools::get_brave_key();
    let tavily_key = crate::ai::tools::get_tavily_key();
    let exa_key = crate::ai::tools::get_exa_key();

    match parse_mcp_cli_action(action, target) {
        McpCliAction::Status => {
            if io::stdout().is_terminal() {
                run_interactive_mcp_menu().await;
                return;
            }

            crate::cli::tui::print_mini_header("Search Engine & MCP Tool Hub");
            println!(
                "  \x1b[38;5;245mActive Engine :\x1b[0m \x1b[1;37m{}\x1b[0m",
                search_engine_str
            );
            println!(
                "  \x1b[38;5;245mMCP Endpoint  :\x1b[0m \x1b[1;32m{}\x1b[0m",
                current_mcp_url
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
                "    \x1b[38;5;245mKeyless Fallbacks:\x1b[0m \x1b[38;5;252mExa MCP \u{2192} DuckDuckGo \u{2192} Wikipedia\x1b[0m"
            );

            println!("\n\x1b[38;5;244mSubcommands:\x1b[0m");
            println!("  xiao mcp url <URL>         - Set custom MCP endpoint URL (SSRF protected)");
            println!("  xiao mcp test [query]      - Test direct Exa MCP protocol probe");
            println!(
                "  xiao mcp search <query>    - Test end-to-end web search with active engine"
            );
            println!(
                "  xiao mcp tools             - List registered function calling tools & schemas"
            );
            println!("  xiao mcp brave [KEY|rm]    - Configure or remove Brave Search API key");
            println!("  xiao mcp tavily [KEY|rm]   - Configure or remove Tavily Search API key");
            println!("  xiao mcp exa [KEY|rm]      - Configure or remove Exa REST API key");
            println!("  xiao mcp reset             - Reset MCP endpoint to default (https://mcp.exa.ai/)\n");
        }
        McpCliAction::Help => {
            let bar_width = crate::cli::tui::get_terminal_bar_width();
            crate::cli::tui::print_mini_header("MCP & Search › Command Reference");

            println!("\n  \x1b[1;37mUsage:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao mcp\x1b[0m \x1b[38;5;245m<action>\x1b[0m \x1b[38;5;245m[target...]\x1b[0m\n");

            println!("  \x1b[1;38;2;6;182;212m▸ \x1b[1;37mACTIONS\x1b[0m");
            println!("    \x1b[1;38;5;45mstatus\x1b[0m, \x1b[38;5;244m(none)\x1b[0m             \x1b[38;5;250mDisplay active search engine status & provider keys\x1b[0m");
            println!("    \x1b[1;38;5;45msearch\x1b[0m \x1b[38;5;245m<query>\x1b[0m             \x1b[38;5;250mExecute live web search query\x1b[0m");
            println!("    \x1b[1;38;5;45mtest\x1b[0m, \x1b[1;38;5;45mprobe\x1b[0m \x1b[38;5;245m[query]\x1b[0m        \x1b[38;5;250mDirect JSON-RPC probe to Exa MCP endpoint\x1b[0m");
            println!("    \x1b[1;38;5;45murl\x1b[0m \x1b[38;5;245m<URL>\x1b[0m                  \x1b[38;5;250mSet custom MCP endpoint URL (SSRF guarded)\x1b[0m");
            println!("    \x1b[1;38;5;45mtools\x1b[0m                     \x1b[38;5;250mList registered tool schemas exposed to LLM\x1b[0m");
            println!("    \x1b[1;38;5;45mbrave\x1b[0m \x1b[38;5;245m[KEY|rm]\x1b[0m            \x1b[38;5;250mConfigure or remove Brave Search API key\x1b[0m");
            println!("    \x1b[1;38;5;45mtavily\x1b[0m \x1b[38;5;245m[KEY|rm]\x1b[0m           \x1b[38;5;250mConfigure or remove Tavily Search API key\x1b[0m");
            println!("    \x1b[1;38;5;45mexa\x1b[0m \x1b[38;5;245m[KEY|rm]\x1b[0m              \x1b[38;5;250mConfigure or remove Exa REST API key\x1b[0m");
            println!("    \x1b[1;38;5;45mreset\x1b[0m                     \x1b[38;5;250mReset MCP endpoint to default (https://mcp.exa.ai/)\x1b[0m");
            println!("    \x1b[1;38;5;45mhelp\x1b[0m, \x1b[1;38;5;45m-h\x1b[0m                  \x1b[38;5;250mShow this help reference\x1b[0m\n");

            println!(
                "  \x1b[38;5;238m{}\x1b[0m\n",
                "─".repeat(bar_width.saturating_sub(4))
            );

            println!("  \x1b[1;37mQuick Examples:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao mcp search \"Latest Rust 1.85 features\"\x1b[0m  \x1b[38;5;242m# Live search test\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao mcp brave BSA...                      \x1b[38;5;242m# Save Brave Search key\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao mcp test                              \x1b[38;5;242m# Probe Exa MCP health\x1b[0m\n");
        }
        McpCliAction::Tools => {
            print_tools_summary();
        }
        McpCliAction::Url(tgt) => {
            let raw_url = if let Some(u) = tgt {
                u.to_string()
            } else if io::stdout().is_terminal() {
                print!("\nEnter new MCP Endpoint URL: ");
                let _ = io::stdout().flush();
                let mut input = String::new();
                if io::stdin().read_line(&mut input).is_err() || input.trim().is_empty() {
                    println!("\x1b[33mOperation cancelled.\x1b[0m\n");
                    return;
                }
                input.trim().to_string()
            } else {
                println!("\n\x1b[31m✖ Error: <URL> parameter is required.\x1b[0m");
                println!("  Usage: xiao mcp url <URL>\n");
                std::process::exit(1);
            };
            let trimmed = raw_url.trim();
            match crate::bot::url_policy::resolve_download_url(trimmed).await {
                Ok(_) => {
                    if crate::ai::service::save_app_setting("EXA_MCP_URL", trimmed).is_ok() {
                        println!(
                            "\n\x1b[1;32m✔ MCP endpoint successfully saved:\x1b[0m {}\n",
                            trimmed
                        );
                    } else {
                        println!(
                            "\n\x1b[31m✖ Failed to save MCP configuration to database.\x1b[0m\n"
                        );
                        std::process::exit(1);
                    }
                }
                Err(err) => {
                    println!(
                        "\n\x1b[31m✖ URL rejected by security policy (SSRF/Protocol): {}\x1b[0m\n",
                        err
                    );
                    std::process::exit(1);
                }
            }
        }
        McpCliAction::Test(tgt) => {
            let query = tgt.unwrap_or("Rust 2021 edition release notes");
            println!("\n\x1b[1;36mTesting Exa MCP Endpoint Probe...\x1b[0m");
            println!("  Endpoint : {}", current_mcp_url);
            println!("  Query    : {}\n", query);
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default();
            let start = std::time::Instant::now();
            match crate::ai::tools::search_exa_mcp(&client, &current_mcp_url, query).await {
                Ok(result) => {
                    let elapsed = start.elapsed().as_millis();
                    println!("\x1b[1;32m✔ Successfully connected to MCP ({elapsed}ms)\x1b[0m\n");
                    let preview = if result.len() > 400 {
                        &result[..400]
                    } else {
                        &result
                    };
                    println!(
                        "\x1b[38;5;244mResponse Snippet:\x1b[0m\n{}\x1b[38;5;244m...\x1b[0m\n",
                        preview.trim()
                    );
                }
                Err(err) => {
                    let elapsed = start.elapsed().as_millis();
                    println!(
                        "\x1b[31m✖ Failed to probe MCP ({elapsed}ms): {}\x1b[0m\n",
                        err
                    );
                    std::process::exit(1);
                }
            }
        }
        McpCliAction::Search(tgt) => {
            let query = tgt.unwrap_or("Rust 2021 edition");
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
                let preview = if result.len() > 500 {
                    &result[..500]
                } else {
                    &result
                };
                println!(
                    "\x1b[38;5;244mResult Snippet:\x1b[0m\n{}\x1b[38;5;244m...\x1b[0m\n",
                    preview.trim()
                );
            }
        }
        McpCliAction::Brave(tgt) => {
            handle_search_key_subcommand("BRAVE_API_KEY", "Brave Search API", tgt);
        }
        McpCliAction::Tavily(tgt) => {
            handle_search_key_subcommand("TAVILY_API_KEY", "Tavily Search API", tgt);
        }
        McpCliAction::Exa(tgt) => {
            handle_search_key_subcommand("EXA_API_KEY", "Exa REST API", tgt);
        }
        McpCliAction::Reset => {
            let default_url = "https://mcp.exa.ai/";
            if crate::ai::service::save_app_setting("EXA_MCP_URL", default_url).is_ok() {
                println!(
                    "\n\x1b[1;32m✔ MCP endpoint successfully reset to default:\x1b[0m {}\n",
                    default_url
                );
            } else {
                println!("\n\x1b[31m✖ Failed to reset MCP configuration.\x1b[0m\n");
                std::process::exit(1);
            }
        }
        McpCliAction::Unknown(unknown) => {
            println!("\n\x1b[31m✖ Error: Unknown sub-command 'mcp {unknown}'.\x1b[0m");
            println!("  Run 'xiao mcp help' or 'xiao help' for usage instructions.\n");
            std::process::exit(1);
        }
    }
}
