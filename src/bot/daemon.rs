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
            println!(
                "\n🚀 xiao @{} online menggunakan Telegram Bot API 10.3!",
                bot_info.username.as_deref().unwrap_or_default()
            );
            println!("⚡ Streaming Timeline + Native Stop Active!");
            println!("🌐 Custom OpenAI-Compatible Provider Setup Active (via CLI)\n");
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

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n🛑 Menerima sinyal berhenti (SIGINT). Bot dimatikan secara aman.");
                break;
            }
            _ = async {
                #[cfg(unix)]
                if let Some(ref mut sig) = sigterm {
                    sig.recv().await;
                    return;
                }
                std::future::pending::<()>().await;
            } => {
                println!("\n🛑 Menerima sinyal terminasi (SIGTERM). Bot dimatikan secara aman.");
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
