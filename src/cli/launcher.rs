use std::io::{self, BufRead, IsTerminal};
use std::sync::Arc;

use crate::ai::service::ModelRole;
use crate::ai::AIChatService;
use crate::bot::client::TelegramBotClient;
use crate::cli::ai_hub::run_cli_ai_hub;
use crate::cli::chat::run_cli_chat;
use crate::cli::gateway::run_cli_gateway_hub;
use crate::cli::mcp::run_cli_mcp_hub;
use crate::cli::memory::run_cli_memory;
use crate::cli::status::run_cli_status;
use crate::cli::tui::terminal_interactive_select;
use crate::cli::wizard::run_cli_quickstart_wizard;
use crate::{get_configured_owner_id, get_configured_token, load_environment};

pub fn xiao_banner() -> String {
    format!(
        "\x1b[38;2;16;185;129m  ██╗  ██╗██╗ █████╗  ██████╗ \x1b[0m\r\n\
         \x1b[38;2;13;183;168m  ╚██╗██╔╝██║██╔══██╗██╔═══██╗\x1b[0m\r\n\
         \x1b[38;2;6;182;212m   ╚███╔╝ ██║███████║██║   ██║\x1b[0m\r\n\
         \x1b[38;2;37;148;226m   ██╔██╗ ██║██╔══██║██║   ██║\x1b[0m\r\n\
         \x1b[38;2;91;117;238m  ██╔╝ ██╗██║██║  ██║╚██████╔╝\x1b[0m\r\n\
         \x1b[38;2;139;92;246m  ╚═╝  ╚═╝╚═╝╚═╝  ╚═╝ ╚═════╝ \x1b[0m\r\n\
          \x1b[48;2;15;23;42m\x1b[38;2;6;182;212m RUST 2021 \x1b[0m \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m \x1b[48;2;15;23;42m\x1b[38;2;139;92;246m v{} \x1b[0m",
        env!("CARGO_PKG_VERSION")
    )
}

pub fn build_telemetry_hud(gateway: &str, model: &str, search: &str, bar_width: usize) -> String {
    let rows = [("GATEWAY", gateway), ("MAIN AI", model), ("SEARCH", search)];
    crate::cli::tui::render_hud_box("STATUS TELEMETRY", &rows, bar_width)
}

pub(crate) async fn run_cli_launcher(ai_service: &Arc<AIChatService>) {
    load_environment();

    loop {
        let banner = xiao_banner();

        // 1. Gateway Status
        let token = get_configured_token().unwrap_or_default();
        let owner_id = get_configured_owner_id();
        let gateway_str = if token.is_empty() || token == "YOUR_TELEGRAM_BOT_TOKEN_HERE" {
            "\x1b[38;5;244m○ Not connected\x1b[0m".to_string()
        } else {
            let bot = TelegramBotClient::new(&token);
            match bot.get_me().await {
                Ok(resp) if resp.ok => {
                    let uname = resp
                        .result
                        .and_then(|i| i.username)
                        .unwrap_or_else(|| "Bot".to_string());
                    let owner_str = owner_id
                        .map(|id| format!(" · Owner: {id}"))
                        .unwrap_or_default();
                    format!("\x1b[38;2;16;185;129m●\x1b[0m \x1b[1;37m@{uname}\x1b[0m\x1b[38;5;244m{owner_str}\x1b[0m \x1b[1;38;2;16;185;129m(Online)\x1b[0m")
                }
                _ => "\x1b[38;2;239;68;68m○\x1b[0m \x1b[31mInvalid token / Offline\x1b[0m"
                    .to_string(),
            }
        };

        // 2. Main AI Provider & Model Status
        let main_route = ai_service.resolve_model_route(ModelRole::Main).await;
        let model_str = match main_route {
            Ok(route) => format!(
                "\x1b[38;2;6;182;212m●\x1b[0m \x1b[1;37m{}\x1b[0m \x1b[38;5;244m::\x1b[0m \x1b[1;37m{}\x1b[0m",
                route.provider.name, route.model
            ),
            Err(_) => "\x1b[38;5;244m○ No active Provider\x1b[0m".to_string(),
        };

        // 3. Search Engine Status
        let (search_engine_name, _) = crate::ai::tools::get_search_engine_status();
        let search_str = if search_engine_name.starts_with("Exa MCP") {
            "\x1b[38;2;139;92;246m◈\x1b[0m \x1b[1;37mExa MCP\x1b[0m \x1b[38;5;245m(Keyless)\x1b[0m \x1b[38;5;240m→\x1b[0m \x1b[38;5;248mDuckDuckGo / Wikipedia\x1b[0m".to_string()
        } else {
            format!("\x1b[38;2;16;185;129m●\x1b[0m \x1b[1;37m{search_engine_name}\x1b[0m")
        };

        let bar_width = crate::cli::tui::get_terminal_bar_width();
        let hud = build_telemetry_hud(&gateway_str, &model_str, &search_str, bar_width);
        let title = format!("{banner}\r\n\r\n{hud}");

        let menu_items = vec![
            "Terminal Chat               (Interactive REPL session)".to_string(),
            "Start Telegram Daemon       (Launch polling service)".to_string(),
            "AI Center Hub               (Providers, models, addons, probe)".to_string(),
            "Gateway Manager             (Bot token, owner ID, check)".to_string(),
            "Web Search Engine           (Brave, Tavily, Exa API keys)".to_string(),
            "Model Context Protocol      (MCP servers, dynamic tools)".to_string(),
            "System Status & Context     (Diagnostics & token gauge)".to_string(),
            "Long-Term Memory            (Tier-1 profile facts)".to_string(),
            "Setup Wizard                (Quickstart onboarding)".to_string(),
            "Exit                        (Quit Xiao Control Center)".to_string(),
        ];

        let sel = terminal_interactive_select(&title, &menu_items, 0, false, None);

        match sel {
            Some(0) => {
                run_cli_chat(ai_service, None).await;
            }
            Some(1) => {
                crate::cli::tui::print_mini_header("Telegram Daemon");
                println!(
                    "  \x1b[38;5;244mInitializing Telegram Bot API 10.3 connection...\x1b[0m\n"
                );
                crate::bot::daemon::run_daemon(Arc::clone(ai_service)).await;
                break;
            }
            Some(2) => {
                run_cli_ai_hub(ai_service, None, None).await;
            }
            Some(3) => {
                run_cli_gateway_hub(None, None).await;
            }
            Some(4) => {
                crate::cli::search::run_cli_search_hub(ai_service, None, None).await;
            }
            Some(5) => {
                run_cli_mcp_hub(ai_service, None, None, None).await;
            }
            Some(6) => {
                let diag_items = vec![
                    "System Health & Telemetry Status (Gateways, providers, routing, models)"
                        .to_string(),
                    "Context Window & Token Breakdown (Active tokens, sliding window, gauge)"
                        .to_string(),
                    "Back to Main Menu                (Return to Xiao Control Center)".to_string(),
                ];
                let diag_sel = terminal_interactive_select(
                    "Select Telemetry / Diagnostics View:",
                    &diag_items,
                    0,
                    false,
                    None,
                );
                match diag_sel {
                    Some(0) => {
                        run_cli_status(ai_service).await;
                        if io::stdout().is_terminal() {
                            println!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                            let stdin = io::stdin();
                            let mut reader = stdin.lock();
                            let mut buffer = String::new();
                            let _ = reader.read_line(&mut buffer);
                        }
                    }
                    Some(1) => {
                        crate::cli::context::run_cli_context(ai_service, None, None).await;
                        if io::stdout().is_terminal() {
                            println!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                            let stdin = io::stdin();
                            let mut reader = stdin.lock();
                            let mut buffer = String::new();
                            let _ = reader.read_line(&mut buffer);
                        }
                    }
                    _ => {}
                }
            }
            Some(7) => {
                run_cli_memory(ai_service, None, None).await;
            }
            Some(8) => {
                let _ = run_cli_quickstart_wizard(ai_service).await;
            }
            Some(9) | None => {
                println!("\n\x1b[38;5;244mGoodbye!\x1b[0m\n");
                break;
            }
            _ => break,
        }
    }
}
