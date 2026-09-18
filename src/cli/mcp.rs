use crate::ai::AIChatService;
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
                println!("\n\x1b[1;32m✔ {provider_label} key berhasil dihapus.\x1b[0m\n");
            } else {
                println!("\n\x1b[31m✖ Gagal menghapus {provider_label} key.\x1b[0m\n");
                std::process::exit(1);
            }
        }
        Some(new_key) if !new_key.trim().is_empty() => {
            let trimmed = new_key.trim();
            if crate::ai::service::save_app_setting(key_name, trimmed).is_ok() {
                println!(
                    "\n\x1b[1;32m✔ {provider_label} key berhasil disimpan:\x1b[0m {}\n",
                    mask_api_key(trimmed)
                );
            } else {
                println!("\n\x1b[31m✖ Gagal menyimpan {provider_label} key ke database.\x1b[0m\n");
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
            println!(
                "\n\x1b[1;36mModel Context Protocol (MCP) & Web Search Configuration\x1b[0m\n"
            );
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
            println!("\n\x1b[1;36mxiao mcp — Model Context Protocol & Search Tool Hub\x1b[0m\n");
            println!("\x1b[1;37mUsage:\x1b[0m");
            println!("  xiao mcp [action] [target]\n");
            println!("\x1b[1;37mSubcommands for 'mcp':\x1b[0m");
            println!("     \x1b[36mxiao mcp\x1b[0m                     Display active search status and provider keys");
            println!("     \x1b[36mxiao mcp url <URL>\x1b[0m           Set custom MCP endpoint URL (SSRF protected)");
            println!("     \x1b[36mxiao mcp test [query]\x1b[0m        Probe Exa MCP server with a test query");
            println!("     \x1b[36mxiao mcp search <query>\x1b[0m      Test end-to-end web search tool with active engine");
            println!("     \x1b[36mxiao mcp tools\x1b[0m               List registered tools and schemas");
            println!("     \x1b[36mxiao mcp brave [KEY|rm]\x1b[0m     Set or remove Brave Search API key");
            println!("     \x1b[36mxiao mcp tavily [KEY|rm]\x1b[0m    Set or remove Tavily Search API key");
            println!(
                "     \x1b[36mxiao mcp exa [KEY|rm]\x1b[0m       Set or remove Exa REST API key"
            );
            println!("     \x1b[36mxiao mcp reset\x1b[0m               Reset MCP endpoint to https://mcp.exa.ai/\n");
        }
        McpCliAction::Tools => {
            println!("\n\x1b[1;36m== Registered Function Calling Tools ==\x1b[0m\n");
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
                    println!("  \x1b[1;32m🔧 {}\x1b[0m", name);
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
        McpCliAction::Url(tgt) => {
            let Some(raw_url) = tgt else {
                println!("\n\x1b[31m✖ Error: Parameter <URL> diperlukan.\x1b[0m");
                println!("  Penggunaan: xiao mcp url <URL>\n");
                std::process::exit(1);
            };
            let trimmed = raw_url.trim();
            match crate::bot::url_policy::resolve_download_url(trimmed).await {
                Ok(_) => {
                    if crate::ai::service::save_app_setting("EXA_MCP_URL", trimmed).is_ok() {
                        println!(
                            "\n\x1b[1;32m✔ MCP endpoint berhasil disimpan:\x1b[0m {}\n",
                            trimmed
                        );
                    } else {
                        println!(
                            "\n\x1b[31m✖ Gagal menyimpan konfigurasi MCP ke database.\x1b[0m\n"
                        );
                        std::process::exit(1);
                    }
                }
                Err(err) => {
                    println!("\n\x1b[31m✖ URL ditolak oleh kebijakan keamanan (SSRF/Protokol): {}\x1b[0m\n", err);
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
                    println!("\x1b[1;32m✔ Sukses terhubung ke MCP ({elapsed}ms)\x1b[0m\n");
                    let preview = if result.len() > 400 {
                        &result[..400]
                    } else {
                        &result
                    };
                    println!(
                        "\x1b[38;5;244mCuplikan Respons:\x1b[0m\n{}\x1b[38;5;244m...\x1b[0m\n",
                        preview.trim()
                    );
                }
                Err(err) => {
                    let elapsed = start.elapsed().as_millis();
                    println!("\x1b[31m✖ Gagal probe MCP ({elapsed}ms): {}\x1b[0m\n", err);
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
            if result.starts_with("Error") || result.contains("tidak dapat menemukan hasil") {
                println!(
                    "\x1b[33m⚠ Search selesai ({elapsed}ms) dengan pesan:\x1b[0m\n{}\n",
                    result.trim()
                );
            } else {
                println!("\x1b[1;32m✔ Sukses mendapatkan hasil pencarian ({elapsed}ms)\x1b[0m\n");
                let preview = if result.len() > 500 {
                    &result[..500]
                } else {
                    &result
                };
                println!(
                    "\x1b[38;5;244mCuplikan Hasil:\x1b[0m\n{}\x1b[38;5;244m...\x1b[0m\n",
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
                    "\n\x1b[1;32m✔ MCP endpoint berhasil direset ke default:\x1b[0m {}\n",
                    default_url
                );
            } else {
                println!("\n\x1b[31m✖ Gagal mereset konfigurasi MCP.\x1b[0m\n");
                std::process::exit(1);
            }
        }
        McpCliAction::Unknown(unknown) => {
            println!("\n\x1b[31m✖ Error: Sub-perintah 'mcp {unknown}' tidak dikenal.\x1b[0m");
            println!("  Jalankan 'xiao mcp help' atau 'xiao help' untuk panduan penggunaan.\n");
            std::process::exit(1);
        }
    }
}
