use std::io::{self, Write};

use crate::bot::client::TelegramBotClient;
use crate::cli::tui::terminal_interactive_select;
use crate::{
    get_configured_owner_id, get_configured_token, load_environment, save_env_kv, save_token_to_env,
};

#[derive(Debug, PartialEq, Eq)]
pub enum GatewayCliAction<'a> {
    Menu,
    Check,
    BindToken(Option<&'a str>),
    SetOwner(Option<&'a str>),
    Help,
    Unknown(&'a str),
}

pub fn parse_gateway_cli_action<'a>(
    action: Option<&'a str>,
    target: Option<&'a str>,
) -> GatewayCliAction<'a> {
    match action {
        None => GatewayCliAction::Menu,
        Some("check") | Some("test") | Some("status") => GatewayCliAction::Check,
        Some("token") | Some("bind") => GatewayCliAction::BindToken(target),
        Some("owner") => GatewayCliAction::SetOwner(target),
        Some("help") | Some("--help") | Some("-h") => GatewayCliAction::Help,
        Some(unknown) => GatewayCliAction::Unknown(unknown),
    }
}

pub(crate) async fn run_cli_gateway_menu() {
    load_environment();
    loop {
        let token = get_configured_token().unwrap_or_default();
        let owner_id = get_configured_owner_id();

        let val_tg = if token.is_empty() || token == "YOUR_TELEGRAM_BOT_TOKEN_HERE" {
            "\x1b[38;5;244m○ Not configured\x1b[0m".to_string()
        } else {
            let bot = TelegramBotClient::new(&token);
            match bot.get_me().await {
                Ok(resp) if resp.ok => {
                    let uname = resp
                        .result
                        .and_then(|i| i.username)
                        .unwrap_or_else(|| "Bot".to_string());
                    format!("\x1b[1;32m●\x1b[0m \x1b[1;37mOnline\x1b[0m \x1b[38;5;244m(@{uname} · Bot API 10.3)\x1b[0m")
                }
                _ => "\x1b[31m✖ Invalid Token\x1b[0m".to_string(),
            }
        };

        let val_wa = "\x1b[38;5;244m○ Not configured (Coming Soon)\x1b[0m";
        let val_sec = format!(
            "\x1b[38;5;252mOwner ID: \x1b[1;36m{}\x1b[0m \x1b[38;5;244m· Strict Whitelist\x1b[0m",
            owner_id
                .map(|i| i.to_string())
                .unwrap_or_else(|| "Not set".to_string())
        );

        let bar_width = crate::cli::tui::get_terminal_bar_width();
        let pkg_ver = env!("CARGO_PKG_VERSION");
        let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › Messaging Gateway Manager\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 32;
        let ver_str = format!("v{pkg_ver}");
        let ver_vis = crate::cli::tui::visible_width(&ver_str);
        let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
        let mini_header = format!(
            "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
            " ".repeat(pad),
            "─".repeat(bar_width.saturating_sub(4))
        );

        let hud_rows = [
            ("TELEGRAM GATEWAY", val_tg.as_str()),
            ("WHATSAPP GATEWAY", val_wa),
            ("SECURITY POLICY", val_sec.as_str()),
        ];
        let hud =
            crate::cli::tui::render_hud_box("REGISTERED MESSAGING GATEWAYS", &hud_rows, bar_width);

        let title = format!(
            "{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mSelect Gateway to Manage:\x1b[0m"
        );

        let items = vec![
            "Telegram Gateway           [ACTIVE]   (Bot Token, Ping, Daemon, Reset)".to_string(),
            "WhatsApp Gateway           [INACTIVE] (Multi-device pairing - Coming Soon)"
                .to_string(),
            "Global Security & Owner    [CONFIG]   (Set primary authorized Owner ID)".to_string(),
            "Back to Main Menu                     (Exit to Xiao Control Center)".to_string(),
        ];

        let sel = terminal_interactive_select(&title, &items, 0, false, None);

        let Some(idx) = sel else {
            break;
        };

        match idx {
            0 => {
                run_cli_gateway_telegram_submenu().await;
            }
            1 => {
                println!(
                    "\n  \x1b[38;2;6;182;212m●\x1b[0m \x1b[1;37mWhatsApp Gateway integration is currently in development.\x1b[0m"
                );
                println!("\x1b[38;5;244mComing in upcoming releases with Baileys / WhatsApp Web multi-device pairing.\x1b[0m\n");
                print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                let _ = io::stdout().flush();
                let mut tmp = String::new();
                let _ = io::stdin().read_line(&mut tmp);
            }
            2 => {
                run_cli_telegram_owner(None).await;
            }
            _ => break,
        }
    }
}

async fn run_cli_gateway_telegram_submenu() {
    loop {
        let token = get_configured_token().unwrap_or_default();
        let owner_id = get_configured_owner_id();

        let bar_width = crate::cli::tui::get_terminal_bar_width();
        let pkg_ver = env!("CARGO_PKG_VERSION");
        let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › Gateway › Telegram Gateway Config\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 40;
        let ver_str = format!("v{pkg_ver}");
        let ver_vis = crate::cli::tui::visible_width(&ver_str);
        let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
        let mini_header = format!(
            "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
            " ".repeat(pad),
            "─".repeat(bar_width.saturating_sub(4))
        );

        let (bot_status, token_masked) =
            if token.is_empty() || token == "YOUR_TELEGRAM_BOT_TOKEN_HERE" {
                (
                    "\x1b[38;5;244m○ Not configured\x1b[0m".to_string(),
                    "(not set)".to_string(),
                )
            } else {
                let masked = crate::cli::search::mask_api_key(&token);
                ("\x1b[1;32m● Configured\x1b[0m".to_string(), masked)
            };

        let owner_str = format!(
            "\x1b[1;36m{}\x1b[0m \x1b[38;5;244m· Strict Whitelist Active\x1b[0m",
            owner_id
                .map(|i| i.to_string())
                .unwrap_or_else(|| "Not set".to_string())
        );

        let hud_rows = [
            ("BOT STATUS", bot_status.as_str()),
            ("TOKEN MASKED", token_masked.as_str()),
            ("AUTH OWNER ID", owner_str.as_str()),
        ];
        let hud = crate::cli::tui::render_hud_box("TELEGRAM BOT TELEMETRY", &hud_rows, bar_width);

        let title =
            format!("{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mTelegram Actions:\x1b[0m");

        let actions = vec![
            "Ping Telegram API             (Check connection and latency)".to_string(),
            "Change Telegram Bot Token     (Bind new bot token from @BotFather)".to_string(),
            "Change Telegram Owner User ID  (Set authorized user Telegram ID)".to_string(),
            "Back to Gateway Menu          (Return to Messaging Gateways)".to_string(),
        ];

        let sel = terminal_interactive_select(&title, &actions, 0, false, None);
        let Some(choice) = sel else {
            break;
        };

        match choice {
            0 => {
                let _ = check_telegram_connection().await;
                print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                let _ = io::stdout().flush();
                let mut tmp = String::new();
                let _ = io::stdin().read_line(&mut tmp);
            }
            1 => {
                run_cli_telegram_bind(None).await;
            }
            2 => {
                run_cli_telegram_owner(None).await;
            }
            _ => break,
        }
    }
}

pub(crate) async fn run_cli_gateway_hub(action: Option<&str>, target: Option<&str>) {
    load_environment();
    match parse_gateway_cli_action(action, target) {
        GatewayCliAction::Menu => {
            run_cli_gateway_menu().await;
        }
        GatewayCliAction::Check => {
            run_cli_telegram_check().await;
        }
        GatewayCliAction::BindToken(tgt) => {
            run_cli_telegram_bind(tgt).await;
        }
        GatewayCliAction::SetOwner(tgt) => {
            run_cli_telegram_owner(tgt).await;
        }
        GatewayCliAction::Help => {
            let bar_width = crate::cli::tui::get_terminal_bar_width();
            crate::cli::tui::print_mini_header("Gateway › Command Reference");

            println!("\n  \x1b[1;37mUsage:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao gateway\x1b[0m \x1b[38;5;245m<action>\x1b[0m \x1b[38;5;245m[target...]\x1b[0m\n");

            println!("  \x1b[1;38;2;6;182;212m▸ \x1b[1;37mACTIONS\x1b[0m");
            println!("    \x1b[1;38;5;45mmenu\x1b[0m, \x1b[38;5;244m(none)\x1b[0m              \x1b[38;5;250mOpen interactive Gateway Manager (TUI)\x1b[0m");
            println!("    \x1b[1;38;5;45mcheck\x1b[0m                     \x1b[38;5;250mVerify bot token connectivity (getMe)\x1b[0m");
            println!("    \x1b[1;38;5;45mtoken\x1b[0m \x1b[38;5;245m<TOKEN>\x1b[0m             \x1b[38;5;250mBind and verify Telegram Bot Token\x1b[0m");
            println!("    \x1b[1;38;5;45mowner\x1b[0m, \x1b[1;38;5;45mid\x1b[0m \x1b[38;5;245m<ID>\x1b[0m            \x1b[38;5;250mSet Telegram Owner User ID\x1b[0m");
            println!("    \x1b[1;38;5;45mhelp\x1b[0m, \x1b[1;38;5;45m-h\x1b[0m                  \x1b[38;5;250mShow this help reference\x1b[0m\n");

            println!(
                "  \x1b[38;5;238m{}\x1b[0m\n",
                "─".repeat(bar_width.saturating_sub(4))
            );

            println!("  \x1b[1;37mQuick Examples:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao gateway check\x1b[0m                \x1b[38;5;242m# Test Telegram connection\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao gateway token <TOKEN>\x1b[0m        \x1b[38;5;242m# Bind new bot token\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao gateway owner 12345678\x1b[0m       \x1b[38;5;242m# Authorize owner ID\x1b[0m\n");
        }
        GatewayCliAction::Unknown(unknown) => {
            println!("\x1b[31m✖ Error: Subcommand 'gateway {unknown}' is unknown.\x1b[0m");
            println!("  Run 'xiao gateway help' or 'xiao help' for assistance.\n");
            std::process::exit(1);
        }
    }
}

pub(crate) async fn check_telegram_connection() -> bool {
    load_environment();
    crate::cli::tui::print_mini_header("Telegram Gateway Status");

    let token = get_configured_token().unwrap_or_default();
    if token.is_empty() || token == "YOUR_TELEGRAM_BOT_TOKEN_HERE" {
        println!("  \x1b[31m✖ BOT_TOKEN is not configured.\x1b[0m\n");
        return false;
    }

    let bot = TelegramBotClient::new(&token);
    match bot.get_me().await {
        Ok(resp) if resp.ok => {
            if let Some(info) = resp.result {
                let uname = info.username.unwrap_or_else(|| "Unknown".to_string());
                println!("  \x1b[1;32m✔ Status:\x1b[0m   Connected & Verified (API 10.3)");
                println!("  \x1b[1;37mBot Name:\x1b[0m {}", info.first_name);
                println!("  \x1b[1;37mUsername:\x1b[0m @{}", uname);
                println!("  \x1b[1;37mBot ID:\x1b[0m   {}", info.id);
                if let Some(owner) = get_configured_owner_id() {
                    println!("  \x1b[1;37mOwner ID:\x1b[0m {}", owner);
                } else {
                    println!("  \x1b[31m✖ OWNER_USER_ID is not configured.\x1b[0m");
                }
                println!();
                return true;
            }
            false
        }
        Ok(resp) => {
            println!(
                "  \x1b[31m✖ Invalid token ({:?})\x1b[0m\n",
                resp.description
            );
            false
        }
        Err(e) => {
            println!("  \x1b[31m✖ Failed to connect to Telegram API ({e})\x1b[0m\n");
            false
        }
    }
}

pub(crate) async fn run_cli_telegram_check() {
    if !check_telegram_connection().await {
        std::process::exit(1);
    }
}

pub(crate) async fn run_cli_telegram_bind(manual_token: Option<&str>) {
    load_environment();
    let token = if let Some(t) = manual_token {
        t.trim().to_string()
    } else {
        print!("\n\x1b[1;37mEnter Telegram Bot Token:\x1b[0m ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return;
        }
        input.trim().to_string()
    };

    if token.is_empty() {
        println!("\x1b[31m✖ Token cannot be empty.\x1b[0m\n");
        return;
    }

    println!("  \x1b[38;5;244mVerifying token...\x1b[0m");
    let bot = TelegramBotClient::new(&token);
    match bot.get_me().await {
        Ok(resp) if resp.ok => {
            let Some(info) = resp.result else {
                println!("  \x1b[31m✖ Failed to read bot data from Telegram.\x1b[0m\n");
                return;
            };
            let uname = info.username.unwrap_or_else(|| "Unknown".to_string());
            if let Err(e) = save_token_to_env(&token) {
                println!("  \x1b[31m✖ Failed to save token: {e}\x1b[0m\n");
            } else {
                println!(
                    "  \x1b[1;32m✔ Token valid! Connected to @{} ({})\x1b[0m\n",
                    uname, info.first_name
                );
            }
        }
        Ok(resp) => {
            println!("  \x1b[31m✖ Invalid token: {:?}\x1b[0m\n", resp.description);
        }
        Err(e) => {
            println!("  \x1b[31m✖ Connection error: {e}\x1b[0m\n");
        }
    }
}

pub(crate) async fn run_cli_telegram_owner(owner_arg: Option<&str>) {
    let owner = if let Some(value) = owner_arg {
        value.trim().parse::<i64>().ok()
    } else {
        print!("\n\x1b[1;37mEnter Telegram Owner User ID:\x1b[0m ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            None
        } else {
            input.trim().parse::<i64>().ok()
        }
    };

    match owner.filter(|value| *value > 0) {
        Some(owner_id) => match save_env_kv("OWNER_USER_ID", &owner_id.to_string()) {
            Ok(()) => println!("  \x1b[1;32m✔ Telegram Owner ID set to: {owner_id}\x1b[0m\n"),
            Err(error) => {
                println!("  \x1b[31m✖ Failed to save Owner ID: {error}\x1b[0m\n");
                std::process::exit(1);
            }
        },
        None => {
            println!("  \x1b[31m✖ Owner User ID must be a positive integer.\x1b[0m\n");
            std::process::exit(1);
        }
    }
}
