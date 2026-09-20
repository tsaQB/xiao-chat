use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::ai::{self, AIChatService};
use crate::bot::client::TelegramBotClient;
use crate::bot::image_flow::UserLastImagePrompt;
use crate::bot::models::{BotCommand, Update};
use crate::bot::router::ChatRouteScope;
use crate::bot::worker::{process_durable_update, replay_durable_inbox, spawn_workers};
use crate::cli::get_or_prompt_token;
use crate::get_configured_owner_id;
use crate::load_environment;

pub(crate) fn parse_chat_ids_from_str(raw: &str) -> HashSet<i64> {
    raw.split(',')
        .filter_map(|value| value.trim().parse::<i64>().ok())
        .collect()
}

pub(crate) fn parse_chat_ids_from_config(key: &str) -> HashSet<i64> {
    load_environment();
    let raw = env::var(key)
        .ok()
        .or_else(|| ai::service::load_app_setting(key))
        .unwrap_or_default();
    parse_chat_ids_from_str(&raw)
}

pub(crate) fn get_allowed_chat_ids() -> HashSet<i64> {
    parse_chat_ids_from_config("ALLOWED_CHAT_IDS")
}

pub(crate) fn get_dedicated_chat_ids() -> HashSet<i64> {
    parse_chat_ids_from_config("DEDICATED_CHAT_IDS")
}

pub async fn bootstrap_bot(
    ai_service: &AIChatService,
) -> (TelegramBotClient, Arc<ChatRouteScope>, UserLastImagePrompt) {
    let Some(token) = get_or_prompt_token(ai_service).await else {
        std::process::exit(1);
    };

    let Some(owner_user_id) = get_configured_owner_id() else {
        error!("OWNER_USER_ID belum dikonfigurasi. Jalankan `xiao gateway` atau `xiao setup`.");
        std::process::exit(1);
    };

    let bot = TelegramBotClient::new(token);
    let user_last_image_prompt: UserLastImagePrompt = Arc::new(RwLock::new(HashMap::new()));

    // Test connection & get bot identity
    let (bot_id, bot_username) = match bot.get_me().await {
        Ok(resp) if resp.ok => {
            let Some(bot_info) = resp.result else {
                error!("Telegram getMe returned ok=true without a result");
                std::process::exit(1);
            };
            let bar_width = crate::cli::tui::get_terminal_bar_width();
            let uname = bot_info.username.as_deref().unwrap_or("XiaoBot");
            let first_name = bot_info.first_name.as_str();
            let bot_val = format!(
                "\x1b[38;2;16;185;129m●\x1b[0m \x1b[1;37m@{uname}\x1b[0m \x1b[38;5;244m({first_name})\x1b[0m"
            );
            let proto_val = "\x1b[38;2;6;182;212m●\x1b[0m \x1b[1;37mTelegram Bot API 10.3\x1b[0m \x1b[38;5;244m(Rich Messages + Drafts)\x1b[0m";
            let timeline_val = "\x1b[38;2;139;92;246m●\x1b[0m \x1b[1;37mStreaming Timeline\x1b[0m \x1b[38;5;244m· Native Stop Button Active\x1b[0m";
            let engine_val = "\x1b[38;2;16;185;129m●\x1b[0m \x1b[1;37mOpenAI-Compatible Core\x1b[0m \x1b[38;5;244m· Long-Polling Active\x1b[0m";

            let daemon_rows = [
                ("BOT NAME", bot_val.as_str()),
                ("PROTOCOL", proto_val),
                ("RUNTIME", timeline_val),
                ("ENGINE", engine_val),
            ];
            crate::cli::tui::print_mini_header("Daemon Service");
            let hud =
                crate::cli::tui::render_hud_box("TELEGRAM DAEMON ACTIVE", &daemon_rows, bar_width);
            println!("{hud}");
            println!("\n  \x1b[38;5;244mService actively running. Press \x1b[1;37m[Ctrl+C]\x1b[0m \x1b[38;5;244mto stop daemon.\x1b[0m\n");
            (Some(bot_info.id), bot_info.username)
        }
        Ok(resp) => {
            error!(
                "Gagal terhubung ke Telegram Bot API: {:?}",
                resp.description
            );
            std::process::exit(1);
        }
        Err(e) => {
            error!("HTTP connection error: {e}");
            std::process::exit(1);
        }
    };

    let route_scope = Arc::new(ChatRouteScope::new(
        owner_user_id,
        get_allowed_chat_ids(),
        get_dedicated_chat_ids(),
        bot_id,
        bot_username,
        bot.clone(),
    ));

    // Register Bot Commands - Clear all commands for pure zero-slash conversational gateway
    let empty_commands: Vec<BotCommand> = vec![];

    if let Err(e) = bot.set_my_commands(&empty_commands).await {
        warn!("Gagal mengosongkan bot commands di Telegram: {e}");
    } else {
        info!("Bot commands berhasil dikosongkan (pure zero-slash gateway).");
    }

    (bot, route_scope, user_last_image_prompt)
}

pub async fn poll_loop(
    bot: &TelegramBotClient,
    ai_service: &Arc<AIChatService>,
    user_last_image_prompt: &UserLastImagePrompt,
    route_scope: &Arc<ChatRouteScope>,
    update_tx: &tokio::sync::mpsc::Sender<Update>,
) {
    let mut offset = ai::storage::load_telegram_offset_async().await;
    info!("Memulai polling pesan dengan durable control/generation queues...");

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| warn!("Gagal mendaftarkan SIGTERM handler: {e}"))
        .ok();

    #[cfg(unix)]
    let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        .map_err(|e| warn!("Gagal mendaftarkan SIGHUP handler: {e}"))
        .ok();

    #[cfg(windows)]
    let mut ctrl_close = tokio::signal::windows::ctrl_close()
        .map_err(|e| warn!("Gagal mendaftarkan CTRL_CLOSE handler: {e}"))
        .ok();

    #[cfg(windows)]
    let mut ctrl_shutdown = tokio::signal::windows::ctrl_shutdown()
        .map_err(|e| warn!("Gagal mendaftarkan CTRL_SHUTDOWN handler: {e}"))
        .ok();

    #[cfg(windows)]
    let mut ctrl_logoff = tokio::signal::windows::ctrl_logoff()
        .map_err(|e| warn!("Gagal mendaftarkan CTRL_LOGOFF handler: {e}"))
        .ok();

    #[cfg(windows)]
    let mut ctrl_break = tokio::signal::windows::ctrl_break()
        .map_err(|e| warn!("Gagal mendaftarkan CTRL_BREAK handler: {e}"))
        .ok();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\nReceived shutdown signal (SIGINT). Shutting down gracefully.");
                break;
            }
            _ = async {
                #[cfg(unix)]
                {
                    tokio::select! {
                        _ = async {
                            if let Some(ref mut sig) = sigterm {
                                sig.recv().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {}
                        _ = async {
                            if let Some(ref mut sig) = sighup {
                                sig.recv().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {}
                    }
                }
                #[cfg(windows)]
                {
                    tokio::select! {
                        _ = async {
                            if let Some(ref mut sig) = ctrl_close {
                                sig.recv().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {}
                        _ = async {
                            if let Some(ref mut sig) = ctrl_shutdown {
                                sig.recv().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {}
                        _ = async {
                            if let Some(ref mut sig) = ctrl_logoff {
                                sig.recv().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {}
                        _ = async {
                            if let Some(ref mut sig) = ctrl_break {
                                sig.recv().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {}
                    }
                }
                #[cfg(not(any(unix, windows)))]
                std::future::pending::<()>().await;
            } => {
                println!("\nReceived termination signal. Shutting down gracefully.");
                break;
            }
            updates_res = bot.get_updates(
                offset,
                100,
                20,
                Some(vec![
                    "message".to_string(),
                    "callback_query".to_string(),
                    "stopped_message_generation".to_string(),
                ]),
            ) => {
                match updates_res {
                    Ok(resp) if resp.ok => {
                        if let Some(updates) = resp.result {
                            for update in updates {
                                let update_id = update.update_id;
                                let payload_json = match serde_json::to_string(&update) {
                                    Ok(payload) => payload,
                                    Err(error) => {
                                        error!("Gagal serialize Telegram update {update_id}: {error}");
                                        break;
                                    }
                                };
                                let Some(accepted) = ai::storage::enqueue_telegram_update_async(
                                    update_id,
                                    payload_json,
                                )
                                .await
                                else {
                                    // Never acknowledge a later Telegram update if the durable
                                    // acceptance transaction for this update failed.
                                    error!(
                                        "Durable Telegram intake gagal untuk update {update_id}; offset tidak dimajukan"
                                    );
                                    break;
                                };
                                offset = Some(update_id.saturating_add(1));
                                if !accepted {
                                    continue;
                                }

                                if update.stopped_message_generation.is_some() {
                                    // Native Stop bypasses the queue so cancellation cannot be
                                    // blocked by queued generation work.
                                    process_durable_update(
                                        bot,
                                        ai_service,
                                        user_last_image_prompt,
                                        route_scope,
                                        update,
                                    )
                                    .await;
                                } else if update_tx.send(update).await.is_err() {
                                    error!("Update worker stopped unexpectedly");
                                    return;
                                }
                            }
                        }
                    }
                    Ok(resp) => {
                        warn!("Telegram polling update not ok: {:?}", resp.description);
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    Err(e) => {
                        error!("Polling network error: {e}");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        }
    }
}

pub async fn run_daemon(ai_service: Arc<AIChatService>) {
    tracing_subscriber::fmt::init();

    let (bot, route_scope, user_last_image_prompt) = bootstrap_bot(&ai_service).await;

    let (update_tx, update_worker) = spawn_workers(
        bot.clone(),
        Arc::clone(&ai_service),
        Arc::clone(&user_last_image_prompt),
        Arc::clone(&route_scope),
    );

    replay_durable_inbox(
        &bot,
        &ai_service,
        &user_last_image_prompt,
        &route_scope,
        &update_tx,
    )
    .await;

    poll_loop(
        &bot,
        &ai_service,
        &user_last_image_prompt,
        &route_scope,
        &update_tx,
    )
    .await;

    ai_service.cancel_all_generations().await;
    drop(update_tx);

    match tokio::time::timeout(Duration::from_secs(5), update_worker).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => warn!("Update worker terminated with error: {err}"),
        Err(_) => warn!("Update worker did not stop within shutdown grace period"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chat_ids_from_str_splits_correctly() {
        let ids = parse_chat_ids_from_str("12345, -1009988, 54321, invalid, 0");
        assert!(ids.contains(&12345));
        assert!(ids.contains(&-1009988));
        assert!(ids.contains(&54321));
        assert!(ids.contains(&0));
        assert_eq!(ids.len(), 4);
    }
}
