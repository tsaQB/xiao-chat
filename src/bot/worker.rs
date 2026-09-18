use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, warn};

use crate::ai::{self, AIChatService};
use crate::bot::client::TelegramBotClient;
use crate::bot::image_flow::UserLastImagePrompt;
use crate::bot::models::Update;
use crate::bot::router::{delivery_context_for_update, handle_update, ChatRouteScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeKey {
    pub chat_id: i64,
    pub thread_id: i64,
}

impl ScopeKey {
    pub fn from_update(update: &Update) -> Self {
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
pub struct WorkerContext {
    pub bot: TelegramBotClient,
    pub ai_service: Arc<AIChatService>,
    pub user_last_image_prompt: UserLastImagePrompt,
    pub route_scope: Arc<ChatRouteScope>,
}

pub type ActiveScopes =
    Arc<tokio::sync::Mutex<HashMap<ScopeKey, tokio::sync::mpsc::Sender<Update>>>>;

pub async fn scoped_chat_worker(
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
pub struct ScopedRetryPolicy {
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
pub enum RetryOutcome {
    Success,
    PoisonPill,
    ExceededMaxAttempts,
    ClaimFailed,
    Cancelled,
}

pub trait ScopedRetryStorage: Send + Sync {
    fn claim(&self) -> impl std::future::Future<Output = Option<i64>> + Send;
    fn retry(&self, reason: &'static str) -> impl std::future::Future<Output = bool> + Send;
    fn failed(&self, reason: &'static str) -> impl std::future::Future<Output = bool> + Send;
    fn processed(&self) -> impl std::future::Future<Output = bool> + Send;
}

pub struct DurableInboxStorage {
    pub update_id: i64,
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

pub async fn execute_with_scoped_retry<S, F, Fut>(
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

pub async fn process_scoped_update(
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

pub async fn process_durable_update(
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

pub fn spawn_workers(
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

pub async fn replay_durable_inbox(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_key_extraction_for_messages_threads_and_callbacks() {
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
        .expect("deserialize update succeeds");
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
        .expect("deserialize update succeeds");
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
        .expect("deserialize update succeeds");
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

    impl ScopedRetryStorage for MockRetryStorage {
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
