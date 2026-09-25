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
    // 2. XDG_CONFIG_HOME for Linux and Termux
    if let Ok(xdg_config) = env::var("XDG_CONFIG_HOME") {
        let trimmed = xdg_config.trim();
        if !trimmed.is_empty() {
            let config_path = Path::new(trimmed);
            let app_env = config_path.join("xiao").join(".env");
            if app_env.exists() {
                return app_env;
            }
            let dot_app_env = config_path.join(".xiao").join(".env");
            if dot_app_env.exists() {
                return dot_app_env;
            }
            let legacy_app_env = config_path.join("xiaoai").join(".env");
            if legacy_app_env.exists() {
                return legacy_app_env;
            }
        }
    }
    // 3. ~/.xiao.env or ~/.xiao/.env across HOME and USERPROFILE
    let home_candidates = [env::var("HOME").ok(), env::var("USERPROFILE").ok()];
    for home_opt in home_candidates.into_iter().flatten() {
        let trimmed = home_opt.trim();
        if trimmed.is_empty() {
            continue;
        }
        let home_path = Path::new(trimmed);
        let home_env = home_path.join(".xiao.env");
        if home_env.exists() {
            return home_env;
        }
        let dot_app_dir_env = home_path.join(".xiao").join(".env");
        if dot_app_dir_env.exists() {
            return dot_app_dir_env;
        }
        let dot_xiaoai_dir_env = home_path.join(".xiaoai").join(".env");
        if dot_xiaoai_dir_env.exists() {
            return dot_xiaoai_dir_env;
        }
        let xdg_config_fallback = home_path.join(".config").join("xiao").join(".env");
        if xdg_config_fallback.exists() {
            return xdg_config_fallback;
        }
        let app_dir_env = home_path.join("xiao").join(".env");
        if app_dir_env.exists() {
            return app_dir_env;
        }
        let app_dir_xiaoai_env = home_path.join("xiaoai").join(".env");
        if app_dir_xiaoai_env.exists() {
            return app_dir_xiaoai_env;
        }
        let legacy_app_dir_env = home_path.join("XiaoAI").join(".env");
        if legacy_app_dir_env.exists() {
            return legacy_app_dir_env;
        }
    }
    // 4. %APPDATA%\xiao\.env or %APPDATA%\XiaoAI\.env
    if let Ok(appdata) = env::var("APPDATA") {
        let trimmed = appdata.trim();
        if !trimmed.is_empty() {
            let appdata_path = Path::new(trimmed);
            let app_dir_env = appdata_path.join("xiao").join(".env");
            if app_dir_env.exists() {
                return app_dir_env;
            }
            let legacy_app_dir_env = appdata_path.join("XiaoAI").join(".env");
            if legacy_app_dir_env.exists() {
                return legacy_app_dir_env;
            }
            let modern_app_dir_env = appdata_path.join("xiaoai").join(".env");
            if modern_app_dir_env.exists() {
                return modern_app_dir_env;
            }
            let appdata_dot_env = appdata_path.join(".xiao.env");
            if appdata_dot_env.exists() {
                return appdata_dot_env;
            }
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
    load_environment();
    use std::io::IsTerminal;
    let args: Vec<String> = env::args().collect();
    let subcommand = args.get(1).map(|s| s.as_str());

    let ai_service = Arc::new(AIChatService::new());

    match subcommand {
        None => {
            if std::io::stdout().is_terminal() {
                run_cli_chat(&ai_service, None).await;
            } else {
                print_cli_help();
            }
            return;
        }
        Some("menu") => {
            if std::io::stdout().is_terminal() {
                run_cli_launcher(&ai_service).await;
            } else {
                print_cli_help();
            }
            return;
        }
        Some("-v") | Some("--version") | Some("version") => {
            println!("xiao v{}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some("ai") => {
            let action_arg = args.get(2).map(|s| s.as_str());
            let target_arg = args.get(3).map(|s| s.as_str());
            run_cli_ai_hub(&ai_service, action_arg, target_arg).await;
            return;
        }
        Some("setup") => {
            let _ = run_cli_quickstart_wizard(&ai_service).await;
            return;
        }
        Some("status") => {
            run_cli_status(&ai_service).await;
            return;
        }
        Some("context") => {
            let chat_arg = args.get(2).map(|s| s.as_str());
            let thread_arg = args.get(3).map(|s| s.as_str());
            run_cli_context(&ai_service, chat_arg, thread_arg).await;
            return;
        }
        Some("memory") => {
            let action_arg = args.get(2).map(|s| s.as_str());
            let target_arg = args.get(3).map(|s| s.as_str());
            run_cli_memory(&ai_service, action_arg, target_arg).await;
            return;
        }
        Some("gateway") => {
            let action_arg = args.get(2).map(|s| s.as_str());
            let target_arg = args.get(3).map(|s| s.as_str());
            run_cli_gateway_hub(action_arg, target_arg).await;
            return;
        }
        Some("search") => {
            let (action_arg, target_buf) = cli::search::parse_search_args(&args[2..]);
            run_cli_search_hub(&ai_service, action_arg, target_buf.as_deref()).await;
            return;
        }
        Some("mcp") => {
            let action_arg = args.get(2).map(|s| s.as_str());
            let target_arg = args.get(3).map(|s| s.as_str());
            let extra_arg = args.get(4).map(|s| s.as_str());
            run_cli_mcp_hub(&ai_service, action_arg, target_arg, extra_arg).await;
            return;
        }
        Some("chat") => {
            let prompt_arg = if args.len() > 2 {
                Some(args[2..].join(" "))
            } else {
                None
            };
            run_cli_chat(&ai_service, prompt_arg).await;
            return;
        }
        Some("help") | Some("--help") | Some("-h") => {
            print_cli_help();
            return;
        }
        Some("start") => {
            crate::bot::daemon::run_daemon(ai_service).await;
        }
        Some(unknown) => {
            if unknown.starts_with('-') {
                println!("\x1b[31m✖ Error: Unknown option '{unknown}'. Run 'xiao help' for usage instructions.\x1b[0m");
                std::process::exit(1);
            }
            let prompt = args[1..].join(" ");
            run_cli_chat(&ai_service, Some(prompt)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::storage::ENV_TEST_LOCK;

    #[test]
    fn get_config_path_discovers_profile_or_appdata_env_file() {
        let _lock = ENV_TEST_LOCK.lock().expect("ENV_TEST_LOCK poisoned");
        let orig_home = env::var("HOME").ok();
        let orig_profile = env::var("USERPROFILE").ok();
        let orig_appdata = env::var("APPDATA").ok();
        let orig_xdg_config = env::var("XDG_CONFIG_HOME").ok();

        let temp_dir = env::temp_dir().join(format!(
            "xiaoai-cfg-test-{}-{:x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let xiao_dir = temp_dir.join("xiao");
        let _ = std::fs::create_dir_all(&xiao_dir);
        let test_env = xiao_dir.join(".env");
        std::fs::write(&test_env, b"TEST_KEY=123\n").expect("write test env succeeds");

        env::set_var("USERPROFILE", &temp_dir);
        env::remove_var("HOME");
        env::remove_var("APPDATA");
        env::remove_var("XDG_CONFIG_HOME");

        // If local .env doesn't exist, it should find temp_dir/xiao/.env via USERPROFILE
        if !Path::new(".env").exists() {
            let found = get_config_path();
            assert_eq!(found, test_env);
        }

        let _ = std::fs::remove_file(&test_env);
        let _ = std::fs::remove_dir_all(&xiao_dir);

        // Also verify discovery in ~/.xiao/.env (with leading dot)
        let dot_xiao_dir = temp_dir.join(".xiao");
        let _ = std::fs::create_dir_all(&dot_xiao_dir);
        let dot_test_env = dot_xiao_dir.join(".env");
        std::fs::write(&dot_test_env, b"TEST_KEY=456\n").expect("write dot test env succeeds");
        if !Path::new(".env").exists() {
            let found = get_config_path();
            assert_eq!(found, dot_test_env);
        }

        let _ = std::fs::remove_file(dot_test_env);
        let _ = std::fs::remove_dir_all(dot_xiao_dir);
        let _ = std::fs::remove_dir_all(temp_dir);

        if let Some(val) = orig_home {
            env::set_var("HOME", val);
        } else {
            env::remove_var("HOME");
        }
        if let Some(val) = orig_profile {
            env::set_var("USERPROFILE", val);
        } else {
            env::remove_var("USERPROFILE");
        }
        if let Some(val) = orig_appdata {
            env::set_var("APPDATA", val);
        } else {
            env::remove_var("APPDATA");
        }
        if let Some(val) = orig_xdg_config {
            env::set_var("XDG_CONFIG_HOME", val);
        } else {
            env::remove_var("XDG_CONFIG_HOME");
        }
    }
}
