mod ai;
mod attachments;
mod bot;
mod cli;
mod document;
mod parser;
mod timeline;
mod util;

use std::env;
use std::io;
use std::path::Path;
use std::sync::Arc;

use ai::AIChatService;
use cli::*;

pub(crate) fn get_configured_owner_id() -> Option<i64> {
    load_environment();
    env::var("OWNER_USER_ID")
        .ok()
        .or_else(|| ai::service::load_app_setting("OWNER_USER_ID"))
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
}

fn get_config_path() -> std::path::PathBuf {
    // 1. Current working directory .env
    if Path::new(".env").exists() {
        return Path::new(".env").to_path_buf();
    }
    // 2. ~/.xiao.env or ~/.xiao/.env
    if let Ok(home) = env::var("HOME") {
        let home_env = Path::new(&home).join(".xiao.env");
        if home_env.exists() {
            return home_env;
        }
        let app_dir_env = Path::new(&home).join("xiao").join(".env");
        if app_dir_env.exists() {
            return app_dir_env;
        }
        let legacy_app_dir_env = Path::new(&home).join("XiaoAI").join(".env");
        if legacy_app_dir_env.exists() {
            return legacy_app_dir_env;
        }
    }
    Path::new(".env").to_path_buf()
}

pub(crate) fn load_environment() {
    let cfg_path = get_config_path();
    if cfg_path.exists() {
        let _ = dotenvy::from_path(&cfg_path);
    } else {
        let _ = dotenvy::dotenv();
    }
}

pub(crate) fn save_env_kv(key: &str, value: &str) -> io::Result<()> {
    ai::service::save_app_setting(key, value)
}

pub(crate) fn save_token_to_env(token: &str) -> io::Result<()> {
    ai::service::save_app_setting("BOT_TOKEN", token)
}

pub(crate) fn get_configured_token() -> Option<String> {
    load_environment();
    if let Ok(token) = env::var("BOT_TOKEN") {
        let trimmed = token.trim().to_string();
        if !trimmed.is_empty() && trimmed != "YOUR_TELEGRAM_BOT_TOKEN_HERE" {
            return Some(trimmed);
        }
    }
    if let Some(token) = ai::service::load_app_setting("BOT_TOKEN") {
        let trimmed = token.trim().to_string();
        if !trimmed.is_empty() && trimmed != "YOUR_TELEGRAM_BOT_TOKEN_HERE" {
            return Some(trimmed);
        }
    }
    None
}

// ==========================================
// Main CLI Entrypoint
// ==========================================

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let args: Vec<String> = env::args().collect();
    let subcommand = args.get(1).map(|s| s.as_str()).unwrap_or("start");

    let ai_service = Arc::new(AIChatService::new());

    match subcommand {
        "-v" | "--version" | "version" => {
            println!("xiao v{}", env!("CARGO_PKG_VERSION"));
            return;
        }
        "ai" => {
            let action_arg = args.get(2).map(|s| s.as_str());
            let target_arg = args.get(3).map(|s| s.as_str());
            run_cli_ai_hub(&ai_service, action_arg, target_arg).await;
            return;
        }
        "setup" => {
            let _ = run_cli_quickstart_wizard(&ai_service).await;
            return;
        }
        "status" => {
            run_cli_status(&ai_service).await;
            return;
        }
        "context" => {
            let chat_arg = args.get(2).map(|s| s.as_str());
            let thread_arg = args.get(3).map(|s| s.as_str());
            run_cli_context(&ai_service, chat_arg, thread_arg).await;
            return;
        }
        "memory" => {
            let action_arg = args.get(2).map(|s| s.as_str());
            let target_arg = args.get(3).map(|s| s.as_str());
            run_cli_memory(&ai_service, action_arg, target_arg).await;
            return;
        }
        "gateway" => {
            let action_arg = args.get(2).map(|s| s.as_str());
            let target_arg = args.get(3).map(|s| s.as_str());
            run_cli_gateway_hub(action_arg, target_arg).await;
            return;
        }
        "mcp" => {
            let action_arg = args.get(2).map(|s| s.as_str());
            let target_arg = args.get(3).map(|s| s.as_str());
            run_cli_mcp_hub(&ai_service, action_arg, target_arg).await;
            return;
        }
        "doctor" | "probe" => {
            run_cli_probe_menu(&ai_service).await;
            return;
        }
        "provider" => {
            let action_arg = args.get(2).map(|s| s.as_str());
            run_cli_provider_menu(&ai_service, action_arg).await;
            return;
        }
        "model" => {
            let filter_arg = args.get(2).map(|s| s.as_str());
            if filter_arg == Some("addon") || filter_arg == Some("addons") {
                run_cli_addon_menu(&ai_service).await;
            } else {
                run_cli_model_picker(&ai_service, filter_arg).await;
            }
            return;
        }
        "addon" => {
            run_cli_addon_menu(&ai_service).await;
            return;
        }
        "chat" => {
            let prompt_arg = if args.len() > 2 {
                Some(args[2..].join(" "))
            } else {
                None
            };
            run_cli_chat(&ai_service, prompt_arg).await;
            return;
        }
        "help" | "--help" | "-h" => {
            print_cli_help();
            return;
        }
        "start" => {
            crate::bot::daemon::run_daemon(ai_service).await;
        }
        unknown => {
            println!("\x1b[31m✖ Error: Perintah '{unknown}' tidak dikenal. Jalankan 'xiao help' untuk bantuan.\x1b[0m");
            std::process::exit(1);
        }
    }
}
