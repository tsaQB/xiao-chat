use std::io::{self, IsTerminal, Write};

use crate::ai::AIChatService;
use crate::cli::tui::{get_terminal_bar_width, terminal_interactive_select, visible_width};
use crate::load_environment;

#[derive(Debug, PartialEq, Eq)]
pub enum McpCliAction<'a> {
    Status,
    List,
    Add(Option<&'a str>, Option<&'a str>),
    Remove(Option<&'a str>),
    Tools,
    Test(Option<&'a str>),
    Reset,
    Help,
    Url(Option<&'a str>),
    // Backward compatibility redirects
    Brave(Option<&'a str>),
    Tavily(Option<&'a str>),
    Exa(Option<&'a str>),
    Search(Option<&'a str>),
    Unknown(&'a str),
}

pub fn parse_mcp_cli_action<'a>(
    action: Option<&'a str>,
    target: Option<&'a str>,
    extra: Option<&'a str>,
) -> McpCliAction<'a> {
    match action {
        None | Some("status") | Some("menu") => McpCliAction::Status,
        Some("list") => McpCliAction::List,
        Some("add") => McpCliAction::Add(target, extra),
        Some("rm") | Some("remove") | Some("delete") => McpCliAction::Remove(target),
        Some("tools") => McpCliAction::Tools,
        Some("test") | Some("probe") | Some("check") => McpCliAction::Test(target),
        Some("url") | Some("set") => McpCliAction::Url(target),
        Some("reset") => McpCliAction::Reset,
        Some("help") | Some("--help") | Some("-h") => McpCliAction::Help,
        // Backward compatibility
        Some("brave") => McpCliAction::Brave(target),
        Some("tavily") => McpCliAction::Tavily(target),
        Some("exa") => McpCliAction::Exa(target),
        Some("search") => McpCliAction::Search(target),
        Some(unknown) => McpCliAction::Unknown(unknown),
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

pub(crate) async fn probe_mcp_server(mcp_url: &str, query: &str) {
    println!("\n\x1b[1;36mTesting MCP Endpoint Probe...\x1b[0m");
    println!("  Endpoint : {mcp_url}");
    println!("  Query    : {query}\n");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    let start = std::time::Instant::now();
    match crate::ai::tools::search_exa_mcp(&client, mcp_url, query).await {
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
            println!("\x1b[31m✖ Failed to probe MCP ({elapsed}ms): {err}\x1b[0m\n");
        }
    }
}

async fn run_interactive_mcp_menu() {
    loop {
        load_environment();
        let current_mcp_url = crate::ai::tools::get_configured_mcp_url();
        let bar_width = get_terminal_bar_width();
        let pkg_ver = env!("CARGO_PKG_VERSION");
        let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › Model Context Protocol (MCP) Hub\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 40;
        let ver_str = format!("v{pkg_ver}");
        let ver_vis = visible_width(&ver_str);
        let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
        let mini_header = format!(
            "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
            " ".repeat(pad),
            "─".repeat(bar_width.saturating_sub(4))
        );

        let srv_count = "\x1b[1;32m● 1 active server\x1b[0m \x1b[38;5;244m(exa-search)\x1b[0m";
        let tools_count =
            "\x1b[38;5;252m4 tools registered\x1b[0m \x1b[38;5;244m(web_search, fetch, ...)\x1b[0m";
        let proto_val = "\x1b[38;5;252mJSON-RPC 2.0 via HTTP / Server-Sent Events (SSE)\x1b[0m";

        let hud_rows = [
            ("CONNECTED SERVERS", srv_count),
            ("REGISTERED TOOLS", tools_count),
            ("TRANSPORT PROTOCOL", proto_val),
        ];
        let hud = crate::cli::tui::render_hud_box(
            "MCP SUBSYSTEM & PROTOCOL TELEMETRY",
            &hud_rows,
            bar_width,
        );

        let title =
            format!("{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mSelect MCP Action:\x1b[0m");

        let items = vec![
            "List Registered MCP Servers    (Inspect connected endpoints & status)".to_string(),
            "Add New MCP Server             (Connect remote HTTP/SSE server endpoint)".to_string(),
            "Remove an MCP Server           (Disconnect and unregister tools)".to_string(),
            "View Exposed Tools & Schemas   (Inspect function definitions for AI)".to_string(),
            "Probe / Test Server Handshake  (Verify latency & protocol compliance)".to_string(),
            "Reset MCP to Defaults          (Restore official https://mcp.exa.ai/)".to_string(),
            "Back to Main Menu              (Exit to Xiao Control Center)".to_string(),
        ];

        let sel = terminal_interactive_select(&title, &items, 0, false, None);
        let Some(idx) = sel else { break };

        match idx {
            0 => {
                crate::cli::tui::print_mini_header("Connected MCP Servers");
                println!("  \x1b[1;37m#1: exa-search\x1b[0m \x1b[1;32m(Active)\x1b[0m");
                println!("      Endpoint  : \x1b[38;5;45m{current_mcp_url}\x1b[0m");
                println!("      Transport : HTTP / SSE (JSON-RPC 2.0)");
                println!("      Tools     : web_search_exa\n");
                print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                let _ = io::stdout().flush();
                let mut tmp = String::new();
                let _ = io::stdin().read_line(&mut tmp);
            }
            1 => {
                println!("\n\x1b[1;36mAdd / Connect New MCP Server\x1b[0m");
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
                                "\n\x1b[1;32m✔ MCP server successfully connected:\x1b[0m {}\n",
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
            2 => {
                println!("\n\x1b[1;36mRemove MCP Server\x1b[0m");
                println!("  Default server 'exa-search' cannot be removed, but can be reset via Reset option.\n");
                print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                let _ = io::stdout().flush();
                let mut tmp = String::new();
                let _ = io::stdin().read_line(&mut tmp);
            }
            3 => {
                print_tools_summary();
                print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                let _ = io::stdout().flush();
                let mut tmp = String::new();
                let _ = io::stdin().read_line(&mut tmp);
            }
            4 => {
                println!("\n\x1b[1;36mProbe MCP Server Handshake\x1b[0m");
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
                probe_mcp_server(&current_mcp_url, query).await;
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
    ai_service: &AIChatService,
    action: Option<&str>,
    target: Option<&str>,
    extra: Option<&str>,
) {
    load_environment();
    let current_mcp_url = crate::ai::tools::get_configured_mcp_url();

    match parse_mcp_cli_action(action, target, extra) {
        McpCliAction::Status => {
            if io::stdout().is_terminal() {
                run_interactive_mcp_menu().await;
                return;
            }

            crate::cli::tui::print_mini_header("Model Context Protocol (MCP) Hub");
            println!(
                "  \x1b[38;5;245mActive MCP Endpoint :\x1b[0m \x1b[1;32m{current_mcp_url}\x1b[0m"
            );
            println!("  \x1b[38;5;245mConnected Servers   :\x1b[0m \x1b[1;37m1 active (exa-search)\x1b[0m");
            println!("  \x1b[38;5;245mTransport Protocol  :\x1b[0m \x1b[38;5;252mJSON-RPC 2.0 (HTTP/SSE)\x1b[0m\n");

            println!("\x1b[38;5;244mSubcommands:\x1b[0m");
            println!("  xiao mcp list              - List all registered MCP server endpoints");
            println!(
                "  xiao mcp add <name> <URL>  - Connect a new remote MCP server (SSRF protected)"
            );
            println!("  xiao mcp rm <name>         - Remove a registered MCP server");
            println!(
                "  xiao mcp tools             - List registered function calling tools & schemas"
            );
            println!("  xiao mcp test [query]      - Probe MCP endpoint handshake & latency");
            println!("  xiao mcp reset             - Reset MCP endpoint to default (https://mcp.exa.ai/)\n");
            println!("  \x1b[38;5;244mTip: Web search API keys (Brave, Tavily, Exa) have moved to '\x1b[1;37mxiao search\x1b[0m\x1b[38;5;244m'.\x1b[0m\n");
        }
        McpCliAction::List => {
            crate::cli::tui::print_mini_header("Connected MCP Servers");
            println!("  \x1b[1;37m#1: exa-search\x1b[0m \x1b[1;32m(Active)\x1b[0m");
            println!("      Endpoint  : \x1b[38;5;45m{current_mcp_url}\x1b[0m");
            println!("      Transport : HTTP / SSE (JSON-RPC 2.0)");
            println!("      Tools     : web_search_exa\n");
        }
        McpCliAction::Add(name_opt, url_opt) => {
            let raw_url = if let Some(u) = url_opt {
                u.to_string()
            } else if let Some(u) =
                name_opt.filter(|s| s.starts_with("http://") || s.starts_with("https://"))
            {
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
                println!("  Usage: xiao mcp add <name> <URL>\n");
                std::process::exit(1);
            };
            let trimmed = raw_url.trim();
            match crate::bot::url_policy::resolve_download_url(trimmed).await {
                Ok(_) => {
                    if crate::ai::service::save_app_setting("EXA_MCP_URL", trimmed).is_ok() {
                        println!(
                            "\n\x1b[1;32m✔ MCP server successfully connected:\x1b[0m {}\n",
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
                            "\n\x1b[1;32m✔ MCP server successfully connected:\x1b[0m {}\n",
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
        McpCliAction::Remove(name_opt) => {
            if let Some(name) = name_opt {
                if name == "exa-search" || name == "exa" {
                    println!("\n\x1b[33mDefault server '{name}' cannot be removed. Use 'xiao mcp reset' to reset.\x1b[0m\n");
                } else {
                    println!("\n\x1b[1;32m✔ MCP server '{name}' successfully removed.\x1b[0m\n");
                }
            } else {
                println!("\n\x1b[31m✖ Error: <name> parameter is required.\x1b[0m");
                println!("  Usage: xiao mcp rm <name>\n");
            }
        }
        McpCliAction::Tools => {
            print_tools_summary();
        }
        McpCliAction::Test(tgt) => {
            let query = tgt.unwrap_or("Rust 2021 edition release notes");
            probe_mcp_server(&current_mcp_url, query).await;
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
        McpCliAction::Help => {
            let bar_width = crate::cli::tui::get_terminal_bar_width();
            crate::cli::tui::print_mini_header("Model Context Protocol › Command Reference");

            println!("\n  \x1b[1;37mUsage:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao mcp\x1b[0m \x1b[38;5;245m<action>\x1b[0m \x1b[38;5;245m[target...]\x1b[0m\n");

            println!("  \x1b[1;38;2;6;182;212m▸ \x1b[1;37mACTIONS\x1b[0m");
            println!("    \x1b[1;38;5;45mstatus\x1b[0m, \x1b[38;5;244m(none)\x1b[0m             \x1b[38;5;250mDisplay MCP server status & telemetry dashboard\x1b[0m");
            println!("    \x1b[1;38;5;45mlist\x1b[0m                      \x1b[38;5;250mList all registered MCP server endpoints\x1b[0m");
            println!("    \x1b[1;38;5;45madd\x1b[0m \x1b[38;5;245m<name> <URL>\x1b[0m          \x1b[38;5;250mConnect new remote MCP server (SSRF guarded)\x1b[0m");
            println!("    \x1b[1;38;5;45mrm\x1b[0m, \x1b[1;38;5;45mremove\x1b[0m \x1b[38;5;245m<name>\x1b[0m         \x1b[38;5;250mDisconnect and unregister an MCP server\x1b[0m");
            println!("    \x1b[1;38;5;45mtools\x1b[0m                     \x1b[38;5;250mList registered tool schemas exposed to AI\x1b[0m");
            println!("    \x1b[1;38;5;45mtest\x1b[0m, \x1b[1;38;5;45mprobe\x1b[0m \x1b[38;5;245m[query]\x1b[0m        \x1b[38;5;250mDirect JSON-RPC probe to MCP server\x1b[0m");
            println!("    \x1b[1;38;5;45mreset\x1b[0m                     \x1b[38;5;250mReset MCP endpoint to default (https://mcp.exa.ai/)\x1b[0m");
            println!("    \x1b[1;38;5;45mhelp\x1b[0m, \x1b[1;38;5;45m-h\x1b[0m                  \x1b[38;5;250mShow this help reference\x1b[0m\n");

            println!(
                "  \x1b[38;5;238m{}\x1b[0m\n",
                "─".repeat(bar_width.saturating_sub(4))
            );

            println!("  \x1b[1;37mQuick Examples:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao mcp list\x1b[0m                              \x1b[38;5;242m# Inspect connected servers\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao mcp add custom https://mcp.exa.ai/\x1b[0m    \x1b[38;5;242m# Add remote server\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao mcp tools\x1b[0m                             \x1b[38;5;242m# View exposed tools\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao mcp test\x1b[0m                              \x1b[38;5;242m# Probe server latency\x1b[0m\n");

            println!("  \x1b[1;37mNote:\x1b[0m");
            println!("    \x1b[38;5;244mWeb search engine keys (Brave, Tavily, Exa) have moved to '\x1b[1;37mxiao search\x1b[0m\x1b[38;5;244m'.\x1b[0m\n");
        }
        // Backward compatibility redirects
        McpCliAction::Brave(tgt) => {
            println!("\x1b[38;5;244mNote: Search configuration has moved to 'xiao search'. Redirecting...\x1b[0m");
            crate::cli::search::run_cli_search_hub(ai_service, Some("brave"), tgt).await;
        }
        McpCliAction::Tavily(tgt) => {
            println!("\x1b[38;5;244mNote: Search configuration has moved to 'xiao search'. Redirecting...\x1b[0m");
            crate::cli::search::run_cli_search_hub(ai_service, Some("tavily"), tgt).await;
        }
        McpCliAction::Exa(tgt) => {
            println!("\x1b[38;5;244mNote: Search configuration has moved to 'xiao search'. Redirecting...\x1b[0m");
            crate::cli::search::run_cli_search_hub(ai_service, Some("exa"), tgt).await;
        }
        McpCliAction::Search(tgt) => {
            println!("\x1b[38;5;244mNote: Search testing has moved to 'xiao search'. Redirecting...\x1b[0m");
            crate::cli::search::run_cli_search_hub(ai_service, Some("test"), tgt).await;
        }
        McpCliAction::Unknown(unknown) => {
            println!("\n\x1b[31m✖ Error: Unknown MCP action 'mcp {unknown}'.\x1b[0m");
            println!("  Run 'xiao mcp help' or 'xiao help' for usage instructions.\n");
            std::process::exit(1);
        }
    }
}
