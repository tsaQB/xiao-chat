mod ai;
mod attachments;
mod bot;
mod cli;
mod document;
mod parser;
mod timeline;
mod util;

use std::collections::{HashMap, HashSet};
use std::env;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::bot::image_flow::UserLastImagePrompt;
use crate::bot::router::{delivery_context_for_update, handle_update, ChatRouteScope};
use ai::AIChatService;
use bot::client::TelegramBotClient;
use bot::models::{BotCommand, Update};

pub(crate) fn get_configured_owner_id() -> Option<i64> {
    load_environment();
    env::var("OWNER_USER_ID")
        .ok()
        .or_else(|| ai::service::load_app_setting("OWNER_USER_ID"))
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
}

fn parse_chat_ids_from_config(key: &str) -> HashSet<i64> {
    load_environment();
    env::var(key)
        .ok()
        .or_else(|| ai::service::load_app_setting(key))
        .unwrap_or_default()
        .split(',')
        .filter_map(|value| value.trim().parse::<i64>().ok())
        .collect()
}

fn get_allowed_chat_ids() -> HashSet<i64> {
    parse_chat_ids_from_config("ALLOWED_CHAT_IDS")
}

fn get_dedicated_chat_ids() -> HashSet<i64> {
    parse_chat_ids_from_config("DEDICATED_CHAT_IDS")
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

use cli::*;

// UI builders, handle_ai_chat, and command parsers are modularized in crate::bot::router

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ScopeKey {
    pub(crate) chat_id: i64,
    pub(crate) thread_id: i64,
}

impl ScopeKey {
    pub(crate) fn from_update(update: &Update) -> Self {
        if let Some(msg) = update.message.as_ref() {
            Self {
                chat_id: msg.chat.id,
                thread_id: msg.message_thread_id.unwrap_or(0),
            }
        } else if let Some(cb) = update.callback_query.as_ref() {
            if let Some(msg) = cb.message.as_ref() {
                Self {
                    chat_id: msg.chat.id,
                    thread_id: msg.message_thread_id.unwrap_or(0),
                }
            } else {
                Self {
                    chat_id: cb.from.id,
                    thread_id: 0,
                }
            }
        } else if let Some(stopped) = update.stopped_message_generation.as_ref() {
            Self {
                chat_id: stopped.chat.id,
                thread_id: 0,
            }
        } else {
            Self {
                chat_id: 0,
                thread_id: 0,
            }
        }
    }
}

#[derive(Clone)]
struct WorkerContext {
    bot: TelegramBotClient,
    ai_service: Arc<AIChatService>,
    user_last_image_prompt: UserLastImagePrompt,
    route_scope: Arc<ChatRouteScope>,
}

type ActiveScopes = Arc<tokio::sync::Mutex<HashMap<ScopeKey, tokio::sync::mpsc::Sender<Update>>>>;

async fn scoped_chat_worker(
    scope_key: ScopeKey,
    mut rx: tokio::sync::mpsc::Receiver<Update>,
    active_scopes: ActiveScopes,
    global_concurrency: Arc<tokio::sync::Semaphore>,
    ctx: WorkerContext,
) {
    loop {
        let update_opt = tokio::select! {
            msg = rx.recv() => msg,
            _ = tokio::time::sleep(Duration::from_secs(30)) => None,
        };

        match update_opt {
            Some(update) => {
                process_scoped_update(&global_concurrency, &ctx, update).await;
            }
            None => {
                let mut scopes = active_scopes.lock().await;
                match rx.try_recv() {
                    Ok(late_update) => {
                        drop(scopes);
                        process_scoped_update(&global_concurrency, &ctx, late_update).await;
                        continue;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        scopes.remove(&scope_key);
                        break;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        scopes.remove(&scope_key);
                        break;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScopedRetryPolicy {
    pub max_attempts: i64,
    pub backoff: Duration,
}

impl Default for ScopedRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 2,
            backoff: Duration::from_millis(1500),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryOutcome {
    Success,
    PoisonPill,
    ExceededMaxAttempts,
    ClaimFailed,
    Cancelled,
}

pub(crate) trait ScopedRetryStorage: Send + Sync {
    fn claim(&self) -> impl std::future::Future<Output = Option<i64>> + Send;
    fn retry(&self, reason: &'static str) -> impl std::future::Future<Output = bool> + Send;
    fn failed(&self, reason: &'static str) -> impl std::future::Future<Output = bool> + Send;
    fn processed(&self) -> impl std::future::Future<Output = bool> + Send;
}

struct DurableInboxStorage {
    update_id: i64,
}

impl ScopedRetryStorage for DurableInboxStorage {
    async fn claim(&self) -> Option<i64> {
        ai::storage::mark_telegram_processing_claim_async(self.update_id).await
    }
    async fn retry(&self, reason: &'static str) -> bool {
        ai::storage::mark_telegram_processing_retry_async(self.update_id, reason).await
    }
    async fn failed(&self, reason: &'static str) -> bool {
        ai::storage::mark_telegram_processing_failed_async(self.update_id, reason).await
    }
    async fn processed(&self) -> bool {
        ai::storage::mark_telegram_processed_async(self.update_id).await
    }
}

async fn execute_with_scoped_retry<S, F, Fut>(
    global_concurrency: &Arc<tokio::sync::Semaphore>,
    storage: &S,
    policy: ScopedRetryPolicy,
    mut make_task: F,
) -> RetryOutcome
where
    S: ScopedRetryStorage,
    F: FnMut() -> Fut + Send,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    loop {
        // Acquire global permit only while actively executing
        let permit = match global_concurrency.acquire().await {
            Ok(p) => p,
            Err(_) => return RetryOutcome::Cancelled,
        };

        // Seed/read attempt count directly from durable storage claim
        let attempt = match storage.claim().await {
            Some(a) => a,
            None => {
                drop(permit);
                return RetryOutcome::ClaimFailed;
            }
        };

        if attempt > policy.max_attempts {
            warn!(
                "Task melebihi batas percobaan ({attempt} attempts); mengarantina sebagai failed"
            );
            let _ = storage
                .failed("quarantined after exceeding max attempts")
                .await;
            drop(permit);
            return RetryOutcome::ExceededMaxAttempts;
        }

        let fut = make_task();
        let join_res = tokio::spawn(fut).await;

        match join_res {
            Ok(()) => {
                if !storage.processed().await {
                    warn!("Gagal menyelesaikan durable processing checkpoint");
                }
                drop(permit);
                return RetryOutcome::Success;
            }
            Err(join_err) if join_err.is_panic() => {
                // REQUIREMENT: RELEASE PERMIT BEFORE BACKOFF SLEEP
                drop(permit);

                if attempt < policy.max_attempts {
                    warn!(
                        "Task panic pada percobaan {attempt}/{}. Menandai retry dan backoff {:?} (permit dilepas)...",
                        policy.max_attempts, policy.backoff
                    );
                    let _ = storage
                        .retry("transient panic during processing, scheduled for retry")
                        .await;

                    // Backoff without holding any concurrency permit
                    tokio::time::sleep(policy.backoff).await;

                    // Next iteration re-acquires permit and re-claims via storage.claim()
                    continue;
                } else {
                    error!(
                        "Task gagal setelah {attempt} kali percobaan (poison pill). Mengarantina sebagai failed."
                    );
                    let _ = storage
                        .failed("quarantined after max consecutive panics")
                        .await;
                    return RetryOutcome::PoisonPill;
                }
            }
            Err(join_err) => {
                drop(permit);
                warn!("Task cancelled or aborted: {join_err}");
                return RetryOutcome::Cancelled;
            }
        }
    }
}

async fn process_scoped_update(
    global_concurrency: &Arc<tokio::sync::Semaphore>,
    ctx: &WorkerContext,
    update: Update,
) {
    let update_id = update.update_id;
    let storage = DurableInboxStorage { update_id };
    let policy = ScopedRetryPolicy::default();

    let worker_bot = ctx.bot.clone();
    let worker_ai = Arc::clone(&ctx.ai_service);
    let worker_last_image = Arc::clone(&ctx.user_last_image_prompt);
    let worker_route = Arc::clone(&ctx.route_scope);

    execute_with_scoped_retry(global_concurrency, &storage, policy, move || {
        let update_clone = update.clone();
        let worker_bot = worker_bot.clone();
        let worker_ai = Arc::clone(&worker_ai);
        let worker_last_image = Arc::clone(&worker_last_image);
        let worker_route = Arc::clone(&worker_route);

        async move {
            let delivery_context = delivery_context_for_update(&update_clone);
            TelegramBotClient::with_delivery_context(
                delivery_context,
                handle_update(
                    &worker_bot,
                    &worker_ai,
                    &worker_last_image,
                    &worker_route,
                    update_clone,
                ),
            )
            .await;
        }
    })
    .await;
}

async fn process_durable_update(
    bot: &TelegramBotClient,
    ai_service: &AIChatService,
    user_last_image_prompt: &UserLastImagePrompt,
    route_scope: &ChatRouteScope,
    update: Update,
) {
    let update_id = update.update_id;
    if !ai::storage::mark_telegram_processing_async(update_id).await {
        return;
    }

    let delivery_context = delivery_context_for_update(&update);
    TelegramBotClient::with_delivery_context(
        delivery_context,
        handle_update(bot, ai_service, user_last_image_prompt, route_scope, update),
    )
    .await;

    if !ai::storage::mark_telegram_processed_async(update_id).await {
        warn!("Gagal menyelesaikan durable Telegram inbox update {update_id}");
    }
}
// Document classification and update dispatching are modularized in crate::bot::router

// ==========================================
// Main Function & Polling Loop
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
            run_daemon(ai_service).await;
        }
        unknown => {
            println!("\x1b[31m✖ Error: Perintah '{unknown}' tidak dikenal. Jalankan 'xiao help' untuk bantuan.\x1b[0m");
            std::process::exit(1);
        }
    }
}

async fn bootstrap_bot(
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

fn spawn_workers(
    bot: TelegramBotClient,
    ai_service: Arc<AIChatService>,
    user_last_image_prompt: UserLastImagePrompt,
    route_scope: Arc<ChatRouteScope>,
) -> (
    tokio::sync::mpsc::Sender<Update>,
    tokio::task::JoinHandle<()>,
) {
    let (update_tx, mut update_rx) = tokio::sync::mpsc::channel::<Update>(64);
    let active_scopes: ActiveScopes = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let global_concurrency = Arc::new(tokio::sync::Semaphore::new(8));

    let worker_ctx = WorkerContext {
        bot,
        ai_service,
        user_last_image_prompt,
        route_scope,
    };
    let worker_active_scopes = Arc::clone(&active_scopes);
    let worker_global_concurrency = Arc::clone(&global_concurrency);

    let update_worker = tokio::spawn(async move {
        while let Some(update) = update_rx.recv().await {
            if update.stopped_message_generation.is_some() {
                // Native Stop bypasses worker queues for immediate zero-latency cancellation
                process_durable_update(
                    &worker_ctx.bot,
                    &worker_ctx.ai_service,
                    &worker_ctx.user_last_image_prompt,
                    &worker_ctx.route_scope,
                    update,
                )
                .await;
                continue;
            }

            let scope_key = ScopeKey::from_update(&update);
            let mut scopes = worker_active_scopes.lock().await;

            if let Some(sender) = scopes.get(&scope_key) {
                match sender.try_send(update) {
                    Ok(()) => continue,
                    Err(tokio::sync::mpsc::error::TrySendError::Full(rejected)) => {
                        warn!(
                            "Scope {:?} backlog penuh (>32 pesan); update {} dipertahankan berstatus pending di SQLite inbox",
                            scope_key, rejected.update_id
                        );
                        continue;
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(rejected)) => {
                        scopes.remove(&scope_key);
                        let (tx, rx) = tokio::sync::mpsc::channel::<Update>(32);
                        let _ = tx.try_send(rejected);
                        scopes.insert(scope_key, tx);
                        tokio::spawn(scoped_chat_worker(
                            scope_key,
                            rx,
                            Arc::clone(&worker_active_scopes),
                            Arc::clone(&worker_global_concurrency),
                            worker_ctx.clone(),
                        ));
                        continue;
                    }
                }
            }

            let (tx, rx) = tokio::sync::mpsc::channel::<Update>(32);
            let _ = tx.try_send(update);
            scopes.insert(scope_key, tx);
            tokio::spawn(scoped_chat_worker(
                scope_key,
                rx,
                Arc::clone(&worker_active_scopes),
                Arc::clone(&worker_global_concurrency),
                worker_ctx.clone(),
            ));
        }
    });

    (update_tx, update_worker)
}

async fn replay_durable_inbox(
    bot: &TelegramBotClient,
    ai_service: &Arc<AIChatService>,
    user_last_image_prompt: &UserLastImagePrompt,
    route_scope: &Arc<ChatRouteScope>,
    update_tx: &tokio::sync::mpsc::Sender<Update>,
) {
    let interrupted = ai::storage::recover_telegram_processing_async().await;
    if interrupted > 0 {
        warn!(
            "{interrupted} Telegram update berstatus processing dikembalikan ke pending untuk replay at-least-once; side effect eksternal sebelum crash dapat terulang"
        );
    }

    let mut replay_after_update_id = i64::MIN;
    loop {
        let replay_batch =
            ai::storage::pending_telegram_updates_after_async(replay_after_update_id, 500).await;
        if replay_batch.is_empty() {
            break;
        }
        for record in replay_batch {
            replay_after_update_id = record.update_id;
            if record.attempts >= 2 {
                warn!(
                    "Durable Telegram update {} sudah mencapai batas percobaan ({} attempts) saat startup; mengarantina sebagai failed",
                    record.update_id, record.attempts
                );
                let _ = ai::storage::mark_telegram_processing_failed_async(
                    record.update_id,
                    "quarantined after exceeding max attempts across restarts",
                )
                .await;
                continue;
            }
            match serde_json::from_str::<Update>(&record.payload_json) {
                Ok(update) => {
                    if update.stopped_message_generation.is_some() {
                        // Native Stop bypasses the queue for immediate cancellation.
                        process_durable_update(
                            bot,
                            ai_service,
                            user_last_image_prompt,
                            route_scope,
                            update,
                        )
                        .await;
                    } else if update_tx.send(update).await.is_err() {
                        error!("Update worker stopped while replaying durable inbox");
                        return;
                    }
                }
                Err(error) => {
                    warn!(
                        "Durable Telegram update {} tidak dapat didecode: {error}",
                        record.update_id
                    );
                    if ai::storage::mark_telegram_processing_async(record.update_id).await
                        && !ai::storage::mark_telegram_processed_async(record.update_id).await
                    {
                        warn!(
                        "Gagal menandai durable Telegram update {} yang invalid sebagai completed",
                        record.update_id
                    );
                    }
                }
            }
        }
    }
}

async fn poll_loop(
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

async fn run_daemon(ai_service: Arc<AIChatService>) {
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
    fn scope_key_extraction_for_messages_threads_and_callbacks() {
        use super::ScopeKey;
        use crate::bot::models::Update;

        // 1. Direct message (no thread)
        let update_direct: Update = serde_json::from_str(
            r#"{
                "update_id": 1,
                "message": {
                    "message_id": 10,
                    "date": 1234567,
                    "chat": {"id": 12345, "type": "private"},
                    "text": "hello"
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            ScopeKey::from_update(&update_direct),
            ScopeKey {
                chat_id: 12345,
                thread_id: 0
            }
        );

        // 2. Forum topic message (with thread)
        let update_topic: Update = serde_json::from_str(
            r#"{
                "update_id": 2,
                "message": {
                    "message_id": 11,
                    "message_thread_id": 99,
                    "date": 1234567,
                    "chat": {"id": -100123456, "type": "supergroup"},
                    "text": "in topic"
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            ScopeKey::from_update(&update_topic),
            ScopeKey {
                chat_id: -100123456,
                thread_id: 99
            }
        );

        // 3. Callback query with message
        let update_cb: Update = serde_json::from_str(
            r#"{
                "update_id": 3,
                "callback_query": {
                    "id": "cb1",
                    "from": {"id": 555, "is_bot": false, "first_name": "Test"},
                    "message": {
                        "message_id": 12,
                        "message_thread_id": 77,
                        "date": 1234567,
                        "chat": {"id": -100789, "type": "supergroup"}
                    }
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            ScopeKey::from_update(&update_cb),
            ScopeKey {
                chat_id: -100789,
                thread_id: 77
            }
        );
    }

    #[derive(Default)]
    struct MockRetryStorage {
        claim_count: Arc<std::sync::atomic::AtomicI64>,
        retry_count: Arc<std::sync::atomic::AtomicUsize>,
        failed_count: Arc<std::sync::atomic::AtomicUsize>,
        processed_count: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl super::ScopedRetryStorage for MockRetryStorage {
        async fn claim(&self) -> Option<i64> {
            Some(
                self.claim_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1,
            )
        }
        async fn retry(&self, _reason: &'static str) -> bool {
            self.retry_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        }
        async fn failed(&self, _reason: &'static str) -> bool {
            self.failed_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        }
        async fn processed(&self) -> bool {
            self.processed_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        }
    }

    #[tokio::test]
    async fn execute_with_scoped_retry_drops_permit_during_panic_backoff_runtime() {
        use super::{execute_with_scoped_retry, RetryOutcome, ScopedRetryPolicy};

        let sem = Arc::new(tokio::sync::Semaphore::new(1));
        let storage = MockRetryStorage::default();
        let policy = ScopedRetryPolicy {
            max_attempts: 2,
            backoff: Duration::from_millis(200),
        };

        let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let call_count_clone = Arc::clone(&call_count);
        let sem_worker = Arc::clone(&sem);

        let worker_handle = tokio::spawn(async move {
            execute_with_scoped_retry(&sem_worker, &storage, policy, move || {
                let count = call_count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    if count == 0 {
                        panic!("simulated transient panic on attempt 1");
                    }
                }
            })
            .await
        });

        // Give the task time to acquire permit, spawn handler, panic, and enter the 200ms backoff sleep
        tokio::time::sleep(Duration::from_millis(50)).await;

        // While the worker is sleeping in its backoff window, prove at runtime that the permit is free
        let acquired_during_backoff = match sem.try_acquire() {
            Ok(test_permit) => {
                // Drop our test permit so the worker can re-acquire it for attempt 2
                drop(test_permit);
                true
            }
            Err(_) => false,
        };

        let outcome = worker_handle.await.expect("worker task joins");

        assert!(
            acquired_during_backoff,
            "Semaphore permit must be free and acquirable by other tasks during backoff sleep"
        );
        assert_eq!(outcome, RetryOutcome::Success);
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn execute_with_scoped_retry_quarantines_poison_pill_after_consecutive_panics() {
        use super::{execute_with_scoped_retry, RetryOutcome, ScopedRetryPolicy};

        let sem = Arc::new(tokio::sync::Semaphore::new(1));
        let storage = MockRetryStorage::default();
        let claim_counter = Arc::clone(&storage.claim_count);
        let retry_counter = Arc::clone(&storage.retry_count);
        let failed_counter = Arc::clone(&storage.failed_count);
        let processed_counter = Arc::clone(&storage.processed_count);

        let policy = ScopedRetryPolicy {
            max_attempts: 2,
            backoff: Duration::from_millis(20),
        };

        let outcome = execute_with_scoped_retry(&sem, &storage, policy, || async {
            panic!("unrecoverable panic");
        })
        .await;

        assert_eq!(outcome, RetryOutcome::PoisonPill);
        assert_eq!(claim_counter.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(retry_counter.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(failed_counter.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            processed_counter.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }
}
