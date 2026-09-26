use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

use crate::bot::client::TelegramBotClient;
use crate::cli::tui::terminal_interactive_select;
use crate::{
    get_configured_owner_id, get_configured_token, get_configured_whatsapp_owner,
    get_whatsapp_db_path, is_whatsapp_enabled, load_environment, save_env_kv, save_token_to_env,
};

#[derive(Debug, PartialEq, Eq)]
pub enum GatewayCliAction<'a> {
    Menu,
    Check,
    BindToken(Option<&'a str>),
    SetOwner(Option<&'a str>),
    WhatsApp(Option<&'a str>, Option<&'a str>),
    Help,
    Unknown(&'a str),
}

pub fn parse_gateway_cli_action<'a>(
    action: Option<&'a str>,
    target: Option<&'a str>,
    extra: Option<&'a str>,
) -> GatewayCliAction<'a> {
    match action {
        None => GatewayCliAction::Menu,
        Some("check") | Some("test") | Some("status") => GatewayCliAction::Check,
        Some("token") | Some("bind") => GatewayCliAction::BindToken(target),
        Some("owner") | Some("id") => GatewayCliAction::SetOwner(target),
        Some("wa") | Some("whatsapp") => GatewayCliAction::WhatsApp(target, extra),
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

        let wa_db_path = get_whatsapp_db_path();
        let wa_status = crate::gateway::whatsapp::WhatsAppGateway::check_status(&wa_db_path);
        let wa_owner = get_configured_whatsapp_owner();
        let wa_enabled = is_whatsapp_enabled();

        let val_wa = match wa_status {
            crate::gateway::whatsapp::WhatsAppStatus::Linked => {
                let owner_display = wa_owner.as_deref().unwrap_or("No owner set");
                format!("\x1b[1;32m●\x1b[0m \x1b[1;37mLinked\x1b[0m \x1b[38;5;244m(Multi-Device · +{owner_display})\x1b[0m")
            }
            crate::gateway::whatsapp::WhatsAppStatus::Unlinked => {
                if wa_enabled {
                    "\x1b[38;5;214m◐ Ready to Pair (Scan QR / Code)\x1b[0m".to_string()
                } else {
                    "\x1b[38;5;244m○ Not configured\x1b[0m".to_string()
                }
            }
            crate::gateway::whatsapp::WhatsAppStatus::Unconfigured => {
                "\x1b[38;5;244m○ Unconfigured\x1b[0m".to_string()
            }
        };

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
            ("WHATSAPP GATEWAY", val_wa.as_str()),
            ("SECURITY POLICY", val_sec.as_str()),
        ];
        let hud =
            crate::cli::tui::render_hud_box("REGISTERED MESSAGING GATEWAYS", &hud_rows, bar_width);

        let title = format!(
            "{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mSelect Gateway to Manage:\x1b[0m"
        );

        let wa_tag = if wa_status == crate::gateway::whatsapp::WhatsAppStatus::Linked {
            "[ACTIVE]  "
        } else {
            "[CONFIG]  "
        };

        let items = vec![
            "Telegram Gateway           [ACTIVE]   (Bot Token, Ping, Daemon, Reset)".to_string(),
            format!("WhatsApp Gateway           {wa_tag} (Multi-device pairing, QR, Status)"),
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
                run_cli_gateway_whatsapp_submenu().await;
            }
            2 => {
                run_cli_telegram_owner(None).await;
                crate::cli::tui::print_press_enter();
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
                crate::cli::tui::print_press_enter();
            }
            1 => {
                run_cli_telegram_bind(None).await;
                crate::cli::tui::print_press_enter();
            }
            2 => {
                run_cli_telegram_owner(None).await;
                crate::cli::tui::print_press_enter();
            }
            _ => break,
        }
    }
}

async fn run_cli_gateway_whatsapp_submenu() {
    loop {
        load_environment();
        let db_path = get_whatsapp_db_path();
        let status = crate::gateway::whatsapp::WhatsAppGateway::check_status(&db_path);
        let owner = get_configured_whatsapp_owner();
        let enabled = is_whatsapp_enabled();

        let bar_width = crate::cli::tui::get_terminal_bar_width();
        let pkg_ver = env!("CARGO_PKG_VERSION");
        let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › Gateway › WhatsApp Gateway Config\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 40;
        let ver_str = format!("v{pkg_ver}");
        let ver_vis = crate::cli::tui::visible_width(&ver_str);
        let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
        let mini_header = format!(
            "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
            " ".repeat(pad),
            "─".repeat(bar_width.saturating_sub(4))
        );

        let status_str = match status {
            crate::gateway::whatsapp::WhatsAppStatus::Linked => {
                "\x1b[1;32m●\x1b[0m \x1b[1;37mLinked\x1b[0m \x1b[38;5;244m(Multi-Device Session Active)\x1b[0m".to_string()
            }
            crate::gateway::whatsapp::WhatsAppStatus::Unlinked => {
                "\x1b[38;5;214m◐ Unlinked\x1b[0m \x1b[38;5;244m(Ready to Pair via QR or Code)\x1b[0m".to_string()
            }
            crate::gateway::whatsapp::WhatsAppStatus::Unconfigured => {
                "\x1b[38;5;244m○ Not configured\x1b[0m".to_string()
            }
        };

        let enabled_str = if enabled {
            "\x1b[1;32m● Enabled\x1b[0m \x1b[38;5;244m(Runs in daemon)\x1b[0m".to_string()
        } else {
            "\x1b[38;5;244m○ Disabled (WHATSAPP_ENABLED=false)\x1b[0m".to_string()
        };

        let owner_str = format!(
            "\x1b[1;36m{}\x1b[0m \x1b[38;5;244m· Strict Whitelist\x1b[0m",
            owner.as_deref().unwrap_or("Not set")
        );

        let db_path_display = db_path.to_string_lossy().to_string();
        let hud_rows = [
            ("SESSION STATUS", status_str.as_str()),
            ("GATEWAY STATE", enabled_str.as_str()),
            ("OWNER PHONE", owner_str.as_str()),
            ("STORAGE PATH", db_path_display.as_str()),
        ];
        let hud =
            crate::cli::tui::render_hud_box("WHATSAPP GATEWAY TELEMETRY", &hud_rows, bar_width);

        let title =
            format!("{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mWhatsApp Actions:\x1b[0m");

        let actions = vec![
            "Pair Device via QR Code        (Scan QR code in terminal to link device)".to_string(),
            "Pair Device via 8-Digit Code   (Link using phone number and pair code)".to_string(),
            "Set Owner Phone Number        (Set authorized WhatsApp number)".to_string(),
            "Toggle Gateway (Enable/Disable)(Switch WHATSAPP_ENABLED state)".to_string(),
            "Unlink / Logout Session       (Remove local SQLite session and logout)".to_string(),
            "Back to Gateway Menu          (Return to Messaging Gateways)".to_string(),
        ];

        let sel = terminal_interactive_select(&title, &actions, 0, false, None);
        let Some(choice) = sel else {
            break;
        };

        match choice {
            0 => {
                run_cli_whatsapp_pair(None).await;
                crate::cli::tui::print_press_enter();
            }
            1 => {
                print!("\n  \x1b[1;37mMasukkan nomor telepon (format internasional, contoh: 6281234567890):\x1b[0m ");
                let _ = io::stdout().flush();
                let mut phone = String::new();
                if io::stdin().read_line(&mut phone).is_ok() && !phone.trim().is_empty() {
                    let clean_phone: String =
                        phone.chars().filter(|c| c.is_ascii_digit()).collect();
                    run_cli_whatsapp_pair(Some(clean_phone)).await;
                }
                crate::cli::tui::print_press_enter();
            }
            2 => {
                run_cli_whatsapp_set_owner().await;
                crate::cli::tui::print_press_enter();
            }
            3 => {
                let new_state = if enabled { "false" } else { "true" };
                let _ = save_env_kv("WHATSAPP_ENABLED", new_state);
                println!(
                    "\n  \x1b[1;32m✔ WhatsApp Gateway diubah menjadi: {}\x1b[0m\n",
                    if new_state == "true" {
                        "ENABLED"
                    } else {
                        "DISABLED"
                    }
                );
                crate::cli::tui::print_press_enter();
            }
            4 => {
                if let Err(e) = crate::gateway::whatsapp::WhatsAppGateway::logout(&db_path) {
                    println!("\n  \x1b[31m✖ Gagal unlink session: {e}\x1b[0m\n");
                } else {
                    println!("\n  \x1b[1;32m✔ Sesi WhatsApp berhasil di-unlink/dihapus.\x1b[0m\n");
                }
                crate::cli::tui::print_press_enter();
            }
            _ => break,
        }
    }
}

async fn run_cli_whatsapp_pair(phone_login: Option<String>) {
    let db_path = get_whatsapp_db_path();
    let owner_number = get_configured_whatsapp_owner();
    let config = crate::gateway::whatsapp::WhatsAppConfig {
        db_path,
        owner_number,
        phone_login,
    };
    let ai_service = Arc::new(crate::ai::AIChatService::new());
    println!("\n  \x1b[1;37mMengkoneksikan ke server WhatsApp...\x1b[0m");
    println!("  \x1b[38;5;244mTekan Ctrl+C kapan saja untuk kembali ke menu.\x1b[0m\n");
    if let Err(e) = crate::gateway::whatsapp::WhatsAppGateway::start(config, ai_service).await {
        println!("\n  \x1b[31m✖ WhatsApp connection error: {e}\x1b[0m\n");
    }
}

async fn run_cli_whatsapp_set_owner() {
    print!(
        "\n  \x1b[1;37mMasukkan nomor telepon pemilik WhatsApp (contoh: 6281234567890):\x1b[0m "
    );
    let _ = io::stdout().flush();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() {
        let clean: String = input.chars().filter(|c| c.is_ascii_digit()).collect();
        if !clean.is_empty() {
            let _ = save_env_kv("WHATSAPP_OWNER_NUMBER", &clean);
            println!("\n  \x1b[1;32m✔ WhatsApp Owner Number berhasil disimpan: {clean}\x1b[0m\n");
        } else {
            println!("\n  \x1b[33mNomor tidak valid atau kosong.\x1b[0m\n");
        }
    }
}

pub(crate) async fn run_cli_gateway_hub(
    action: Option<&str>,
    target: Option<&str>,
    extra: Option<&str>,
) {
    load_environment();
    match parse_gateway_cli_action(action, target, extra) {
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
        GatewayCliAction::WhatsApp(subaction, param) => match subaction {
            None | Some("menu") => {
                run_cli_gateway_whatsapp_submenu().await;
            }
            Some("pair") | Some("qr") => {
                run_cli_whatsapp_pair(None).await;
            }
            Some("code") => {
                run_cli_whatsapp_pair(param.map(|s| s.to_string())).await;
            }
            Some("owner") => {
                if let Some(num) = param {
                    let clean: String = num.chars().filter(|c| c.is_ascii_digit()).collect();
                    let _ = save_env_kv("WHATSAPP_OWNER_NUMBER", &clean);
                    println!("  \x1b[1;32m✔ WhatsApp Owner Number set to: {clean}\x1b[0m\n");
                } else {
                    run_cli_whatsapp_set_owner().await;
                }
            }
            Some("status") | Some("check") => {
                let db_path = get_whatsapp_db_path();
                let status = crate::gateway::whatsapp::WhatsAppGateway::check_status(&db_path);
                let owner = get_configured_whatsapp_owner();
                let enabled = is_whatsapp_enabled();
                println!("\n  \x1b[1;37mWhatsApp Gateway Status:\x1b[0m");
                println!("    Status:  {:?}", status);
                println!("    Enabled: {}", enabled);
                println!(
                    "    Owner:   {}",
                    owner.unwrap_or_else(|| "Not set".to_string())
                );
                println!("    Storage: {}\n", db_path.display());
            }
            Some("unlink") | Some("logout") => {
                let db_path = get_whatsapp_db_path();
                if let Err(e) = crate::gateway::whatsapp::WhatsAppGateway::logout(&db_path) {
                    println!("  \x1b[31m✖ Gagal unlink session: {e}\x1b[0m\n");
                } else {
                    println!("  \x1b[1;32m✔ Sesi WhatsApp berhasil di-unlink/dihapus.\x1b[0m\n");
                }
            }
            Some(other) => {
                println!("\x1b[31m✖ Unknown WhatsApp action: '{other}'. Try 'xiao gateway wa [pair|code|owner|status|unlink]'.\x1b[0m\n");
            }
        },
        GatewayCliAction::Help => {
            let bar_width = crate::cli::tui::get_terminal_bar_width();
            crate::cli::tui::print_mini_header("Gateway › Command Reference");

            println!("\n  \x1b[1;37mUsage:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao gateway\x1b[0m \x1b[38;5;245m<action>\x1b[0m \x1b[38;5;245m[target...]\x1b[0m\n");

            println!("  \x1b[1;38;2;6;182;212m▸ \x1b[1;37mACTIONS\x1b[0m");
            println!("    \x1b[1;38;5;45mmenu\x1b[0m, \x1b[38;5;244m(none)\x1b[0m              \x1b[38;5;250mOpen interactive Gateway Manager (TUI)\x1b[0m");
            println!("    \x1b[1;38;5;45mcheck\x1b[0m, \x1b[1;38;5;45mtest\x1b[0m               \x1b[38;5;250mVerify bot token connectivity (getMe)\x1b[0m");
            println!("    \x1b[1;38;5;45mtoken\x1b[0m \x1b[38;5;245m<TOKEN>\x1b[0m             \x1b[38;5;250mBind and verify Telegram Bot Token\x1b[0m");
            println!("    \x1b[1;38;5;45mowner\x1b[0m, \x1b[1;38;5;45mid\x1b[0m \x1b[38;5;245m<ID>\x1b[0m            \x1b[38;5;250mSet Telegram Owner User ID\x1b[0m");
            println!("    \x1b[1;38;5;45mwa\x1b[0m \x1b[38;5;245m[pair|code|owner|status]\x1b[0m \x1b[38;5;250mWhatsApp Gateway management\x1b[0m");
            println!("    \x1b[1;38;5;45mhelp\x1b[0m, \x1b[1;38;5;45m-h\x1b[0m                  \x1b[38;5;250mShow this help reference\x1b[0m\n");

            println!(
                "  \x1b[38;5;238m{}\x1b[0m\n",
                "─".repeat(bar_width.saturating_sub(4))
            );

            println!("  \x1b[1;37mQuick Examples:\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao gateway check\x1b[0m                \x1b[38;5;242m# Test Telegram connection\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao gateway token <TOKEN>\x1b[0m        \x1b[38;5;242m# Bind new bot token\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao gateway owner 12345678\x1b[0m       \x1b[38;5;242m# Authorize owner ID\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao gateway wa pair\x1b[0m              \x1b[38;5;242m# Scan QR code for WhatsApp\x1b[0m");
            println!("    \x1b[1;38;5;45mxiao gateway wa code 628123...\x1b[0m    \x1b[38;5;242m# Pair using 8-digit code\x1b[0m\n");
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
    } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
        print!("\n\x1b[1;37mEnter Telegram Bot Token:\x1b[0m ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return;
        }
        input.trim().to_string()
    } else {
        println!("\n\x1b[31m✖ Error: <TOKEN> parameter is required.\x1b[0m");
        println!("  Usage: xiao gateway token <TOKEN>\n");
        return;
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
    } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
        print!("\n\x1b[1;37mEnter Telegram Owner User ID:\x1b[0m ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() || input.trim().is_empty() {
            println!("  \x1b[33mOperation cancelled.\x1b[0m\n");
            return;
        } else {
            input.trim().parse::<i64>().ok()
        }
    } else {
        println!("  \x1b[31m✖ Error: <ID> parameter is required.\x1b[0m");
        println!("  Usage: xiao gateway owner <ID> (or 'xiao gateway id <ID>')\n");
        return;
    };

    match owner.filter(|value| *value > 0) {
        Some(owner_id) => match save_env_kv("OWNER_USER_ID", &owner_id.to_string()) {
            Ok(()) => println!("  \x1b[1;32m✔ Telegram Owner ID set to: {owner_id}\x1b[0m\n"),
            Err(error) => {
                println!("  \x1b[31m✖ Failed to save Owner ID: {error}\x1b[0m\n");
            }
        },
        None => {
            println!("  \x1b[31m✖ Owner User ID must be a positive integer.\x1b[0m\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gateway_id_alias_parsing() {
        assert_eq!(
            parse_gateway_cli_action(Some("id"), Some("987654"), None),
            GatewayCliAction::SetOwner(Some("987654"))
        );
        assert_eq!(
            parse_gateway_cli_action(Some("owner"), Some("987654"), None),
            GatewayCliAction::SetOwner(Some("987654"))
        );
        assert_eq!(
            parse_gateway_cli_action(Some("token"), Some("test_token"), None),
            GatewayCliAction::BindToken(Some("test_token"))
        );
        assert_eq!(
            parse_gateway_cli_action(Some("check"), None, None),
            GatewayCliAction::Check
        );
        assert_eq!(
            parse_gateway_cli_action(Some("test"), None, None),
            GatewayCliAction::Check
        );
        assert_eq!(
            parse_gateway_cli_action(Some("status"), None, None),
            GatewayCliAction::Check
        );
        assert_eq!(
            parse_gateway_cli_action(Some("wa"), Some("pair"), None),
            GatewayCliAction::WhatsApp(Some("pair"), None)
        );
        assert_eq!(
            parse_gateway_cli_action(Some("wa"), Some("code"), Some("6281234567890")),
            GatewayCliAction::WhatsApp(Some("code"), Some("6281234567890"))
        );
    }

    #[tokio::test]
    async fn test_gateway_non_tty_safety() {
        run_cli_telegram_owner(None).await;
        run_cli_telegram_bind(None).await;
    }
}
