use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::bot::client::{TelegramBotClient, TelegramDeliveryContext};
use crate::bot::models::{InputRichMessage, RichBlock};
use crate::parser::parse_streaming_markdown_to_rich_blocks;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    Active,
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressActivity {
    Thinking,
    Looking,
    Reading,
    Searching,
    Fetching,
    Writing,
    Listening,
    Drawing,
    Watching,
    Summarizing,
    Quiz,
}

impl ProgressActivity {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Thinking => "Thinking",
            Self::Looking => "Looking",
            Self::Reading => "Reading",
            Self::Searching => "Searching",
            Self::Fetching => "Fetching",
            Self::Writing => "Writing",
            Self::Listening => "Listening",
            Self::Drawing => "Generating Image",
            Self::Watching => "Watching",
            Self::Summarizing => "Summarizing",
            Self::Quiz => "Quiz",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Thinking => "🧩",
            Self::Looking => "👀",
            Self::Reading => "📖",
            Self::Searching => "🔎",
            Self::Fetching => "🌐",
            Self::Writing => "🪶",
            Self::Listening => "🎧",
            Self::Drawing => "🫟",
            Self::Watching => "🎥",
            Self::Summarizing => "✨",
            Self::Quiz => "📊",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProgressItem {
    pub label: String,
    pub activity: ProgressActivity,
    pub state: ProgressState,
}

pub trait GenerationProgressSink: Send + Sync {
    fn on_action(&self, label: &str, activity: Option<ProgressActivity>);
    fn on_partial_answer(&self, text: &str);
    fn on_failure(&self, error: &str, force_sync: bool);
    fn on_complete(&self);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineMode {
    PrivateDraft { draft_id: i64, can_stop: bool },
    GroupProgressive,
}

struct TimelineState {
    items: Vec<ProgressItem>,
    partial_answer: String,
    last_sync_time: Option<Instant>,
    stopped: bool,
    is_failed: bool,
    placeholder_message_id: Option<i64>,
    last_synced_status: Option<String>,
    ticker_epoch: u64,
}

impl TimelineState {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            partial_answer: String::new(),
            last_sync_time: None,
            stopped: false,
            is_failed: false,
            placeholder_message_id: None,
            last_synced_status: None,
            ticker_epoch: 0,
        }
    }

    fn add_action(&mut self, label: String, activity: Option<ProgressActivity>, max_items: usize) {
        let act = activity.unwrap_or(ProgressActivity::Thinking);
        for it in self.items.iter_mut() {
            if it.state == ProgressState::Active {
                it.state = ProgressState::Done;
            }
        }
        self.items.push(ProgressItem {
            label,
            activity: act,
            state: ProgressState::Active,
        });
        if self.items.len() > max_items {
            let drain_count = self.items.len() - max_items;
            self.items.drain(0..drain_count);
        }
    }

    fn fail_current(&mut self) {
        for it in self.items.iter_mut().rev() {
            if it.state == ProgressState::Active {
                it.state = ProgressState::Failed;
                break;
            }
        }
    }

    fn finish_all(&mut self, final_state: ProgressState) {
        self.stopped = true;
        for it in self.items.iter_mut() {
            if it.state == ProgressState::Active {
                it.state = final_state;
            }
        }
    }

    fn set_partial_answer(&mut self, text: &str) {
        self.partial_answer.clear();
        self.partial_answer.push_str(text);
    }

    fn render_status_line(&self) -> (String, bool) {
        let mut status_line = "🧩 Thinking".to_string();
        let mut is_active = true;

        for it in self.items.iter().rev() {
            if it.state == ProgressState::Active {
                let lbl = if it.label.is_empty() {
                    it.activity.display_name()
                } else {
                    &it.label
                };
                status_line = if lbl.starts_with(it.activity.icon()) {
                    lbl.to_string()
                } else {
                    format!("{} {}", it.activity.icon(), lbl)
                };
                break;
            } else if it.state == ProgressState::Failed {
                status_line = format!("✗ {} Failed", it.activity.display_name());
                is_active = false;
                break;
            }
        }
        (status_line, is_active)
    }

    fn render_current_status(&self, start_time: Instant, include_elapsed: bool) -> String {
        let (status_line, is_active) = self.render_status_line();
        if !include_elapsed {
            return status_line;
        }

        let elapsed = start_time.elapsed().as_secs();
        if is_active {
            let dots = match elapsed % 3 {
                0 => "•",
                1 => "••",
                _ => "•••",
            };
            format!("{status_line}\n{elapsed}s {dots}")
        } else {
            format!("{status_line}\n{elapsed}s")
        }
    }
}

struct TimelineInner {
    bot: TelegramBotClient,
    chat_id: i64,
    max_items: usize,
    mode: TimelineMode,
    reply_to_message_id: Option<i64>,
    start_time: Instant,
    delivery_context: TelegramDeliveryContext,
    state: std::sync::Mutex<TimelineState>,
    sync_lock: Mutex<()>,
}

impl TimelineInner {
    fn lock_state(&self) -> std::sync::MutexGuard<'_, TimelineState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Clone)]
pub struct ExecutionTimeline {
    inner: Arc<TimelineInner>,
}

impl ExecutionTimeline {
    pub fn for_chat(
        bot: TelegramBotClient,
        chat_id: i64,
        user_id: i64,
        draft_id: i64,
        max_items: usize,
        can_stop: bool,
        reply_to_message_id: Option<i64>,
    ) -> Self {
        if chat_id == user_id {
            Self::with_mode(
                bot,
                chat_id,
                max_items,
                TimelineMode::PrivateDraft { draft_id, can_stop },
                reply_to_message_id,
            )
        } else {
            Self::with_mode(
                bot,
                chat_id,
                max_items,
                TimelineMode::GroupProgressive,
                reply_to_message_id,
            )
        }
    }

    pub fn new(
        bot: TelegramBotClient,
        chat_id: i64,
        max_items: usize,
        mode: TimelineMode,
        reply_to_message_id: Option<i64>,
        delivery_context: TelegramDeliveryContext,
    ) -> Self {
        Self {
            inner: Arc::new(TimelineInner {
                bot,
                chat_id,
                max_items,
                mode,
                reply_to_message_id,
                start_time: Instant::now(),
                delivery_context,
                state: std::sync::Mutex::new(TimelineState::new()),
                sync_lock: Mutex::new(()),
            }),
        }
    }

    pub fn with_mode(
        bot: TelegramBotClient,
        chat_id: i64,
        max_items: usize,
        mode: TimelineMode,
        reply_to_message_id: Option<i64>,
    ) -> Self {
        Self::new(
            bot,
            chat_id,
            max_items,
            mode,
            reply_to_message_id,
            TelegramBotClient::current_delivery_context(),
        )
    }

    #[cfg(test)]
    pub fn mode(&self) -> &TimelineMode {
        &self.inner.mode
    }

    #[cfg(test)]
    pub fn reply_to_message_id(&self) -> Option<i64> {
        self.inner.reply_to_message_id
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, TimelineState> {
        self.inner.lock_state()
    }

    #[cfg(test)]
    pub fn placeholder_message_id(&self) -> Option<i64> {
        self.lock_state().placeholder_message_id
    }

    pub async fn add_action(&self, label: impl Into<String>, activity: Option<ProgressActivity>) {
        let lbl = label.into();
        self.lock_state()
            .add_action(lbl, activity, self.inner.max_items);
    }

    pub async fn fail_current(&self) {
        let mut state = self.lock_state();
        state.fail_current();
        state.stopped = true;
    }

    pub fn stop_ticker(&self) {
        self.lock_state().stopped = true;
    }

    pub fn start_ticker(&self) {
        let my_epoch = {
            let mut state = self.lock_state();
            state.stopped = false;
            state.ticker_epoch = state.ticker_epoch.wrapping_add(1);
            state.ticker_epoch
        };

        // 1. Private chat draft ticker: animate draft every 1000ms
        if matches!(self.inner.mode, TimelineMode::PrivateDraft { .. }) {
            let inner_weak = std::sync::Arc::downgrade(&self.inner);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                    let Some(inner) = inner_weak.upgrade() else {
                        break;
                    };
                    {
                        let state = inner.lock_state();
                        if state.stopped || state.ticker_epoch != my_epoch {
                            break;
                        }
                    }
                    let tl = ExecutionTimeline { inner };
                    TelegramBotClient::with_delivery_context(
                        tl.inner.delivery_context.clone(),
                        tl.sync_draft(false),
                    )
                    .await;
                }
            });
        }

        // 2. Chat action heartbeat: keep Telegram status (e.g. typing) alive every 4000ms
        // until generation completes or stops.
        let inner_weak = std::sync::Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(4000)).await;
                let Some(inner) = inner_weak.upgrade() else {
                    break;
                };
                let action = {
                    let state = inner.lock_state();
                    if state.stopped || state.ticker_epoch != my_epoch {
                        break;
                    }
                    let act = state.items.last().map(|it| it.activity);
                    match act {
                        Some(ProgressActivity::Drawing) => "upload_photo",
                        Some(ProgressActivity::Watching) => "record_video",
                        Some(ProgressActivity::Listening) => "record_voice",
                        _ => "typing",
                    }
                };
                let _ = TelegramBotClient::with_delivery_context(
                    inner.delivery_context.clone(),
                    inner.bot.send_chat_action(inner.chat_id, action),
                )
                .await;
            }
        });
    }

    pub fn trigger_sync(&self, force: bool) {
        let timeline = self.clone();
        tokio::spawn(async move {
            TelegramBotClient::with_delivery_context(
                timeline.inner.delivery_context.clone(),
                timeline.sync_draft(force),
            )
            .await;
        });
    }

    pub async fn sync_draft(&self, force: bool) {
        let _guard = if force {
            self.inner.sync_lock.lock().await
        } else {
            let Ok(guard) = self.inner.sync_lock.try_lock() else {
                return;
            };
            guard
        };

        let now = Instant::now();
        let (status, partial, placeholder_msg_id) = {
            let mut state = self.lock_state();
            if state.stopped && !force {
                return;
            }

            let min_interval_ms = match self.inner.mode {
                TimelineMode::PrivateDraft { .. } => 1200,
                TimelineMode::GroupProgressive => 2500,
            };

            let is_group = matches!(self.inner.mode, TimelineMode::GroupProgressive);
            let cur_status = state.render_current_status(self.inner.start_time, !is_group);

            if is_group
                && state.placeholder_message_id.is_some()
                && state.last_synced_status.as_deref() == Some(&cur_status)
            {
                return;
            }

            if !force
                && state
                    .last_sync_time
                    .is_some_and(|t| now.duration_since(t).as_millis() < min_interval_ms)
            {
                return;
            }

            state.last_sync_time = Some(now);
            state.last_synced_status = Some(cur_status.clone());

            (
                cur_status,
                state.partial_answer.clone(),
                state.placeholder_message_id,
            )
        };

        let is_group = matches!(self.inner.mode, TimelineMode::GroupProgressive);
        let mut blocks = Vec::new();
        if !is_group && !partial.trim().is_empty() {
            // The model streams Markdown into private drafts. Feed the accumulated
            // answer through the semantic parser.
            blocks.extend(parse_streaming_markdown_to_rich_blocks(&partial));
        }

        // When blocks is empty (in groups, or in private drafts before the answer starts),
        // display the live activity/status block with emoji (e.g. 🧩 Thinking, 🔎 Searching, 🪶 Writing).
        if blocks.is_empty() {
            blocks.push(RichBlock::Thinking {
                text: Value::String(status),
            });
        }
        let mut rich_message = InputRichMessage::new(blocks);
        crate::parser::rtl::apply_rtl_direction(&mut rich_message, &partial);

        match &self.inner.mode {
            TimelineMode::PrivateDraft { draft_id, can_stop } => {
                if let Err(e) = self
                    .inner
                    .bot
                    .send_rich_message_draft(
                        self.inner.chat_id,
                        *draft_id,
                        &rich_message,
                        *can_stop,
                        false,
                    )
                    .await
                {
                    debug!("Failed to sync draft update: {e}");
                }
            }
            TimelineMode::GroupProgressive => match placeholder_msg_id {
                None => {
                    match self
                        .inner
                        .bot
                        .send_rich_message(
                            self.inner.chat_id,
                            &rich_message,
                            None,
                            None,
                            self.inner.reply_to_message_id,
                        )
                        .await
                    {
                        Ok(resp) => {
                            let msg_id =
                                resp.get("message_id").and_then(Value::as_i64).or_else(|| {
                                    resp.get("result")
                                        .and_then(|r| r.get("message_id"))
                                        .and_then(Value::as_i64)
                                });
                            if let Some(id) = msg_id {
                                let mut state = self.lock_state();
                                if state.stopped {
                                    let bot = self.inner.bot.clone();
                                    let chat_id = self.inner.chat_id;
                                    tokio::spawn(async move {
                                        let _ = bot.delete_message(chat_id, id).await;
                                    });
                                } else {
                                    state.placeholder_message_id = Some(id);
                                }
                            }
                        }
                        Err(err) => {
                            debug!("Failed to send group progressive placeholder: {err}");
                        }
                    }
                }
                Some(msg_id) => {
                    if let Err(err) = self
                        .inner
                        .bot
                        .edit_rich_message(self.inner.chat_id, msg_id, &rich_message, None)
                        .await
                    {
                        debug!("Failed to edit group progressive rich message: {err}");
                    }
                }
            },
        }
    }

    pub async fn finalize_answer_with_media(
        &self,
        full_rich_msg: &crate::bot::models::InputRichMessage,
        attached_files: Vec<(String, Vec<u8>, String, String)>,
    ) -> Result<serde_json::Value, String> {
        let _sync_guard = self.inner.sync_lock.lock().await;
        let (placeholder_msg_id, is_failed) = {
            let mut state = self.lock_state();
            state.stopped = true;
            (state.placeholder_message_id.take(), state.is_failed)
        };

        if is_failed {
            return Ok(serde_json::json!({"ok": true, "failed": true}));
        }

        let reply_to_msg_id = self.inner.reply_to_message_id;
        let reply_markup: Option<serde_json::Value> = None;

        // If there's an existing placeholder message (in groups or streaming drafts), delete it first because editMessageMedia/editMessageText does not support full sendRichMessage multipart upload
        if let Some(msg_id) = placeholder_msg_id {
            let _ = self
                .inner
                .bot
                .delete_message(self.inner.chat_id, msg_id)
                .await;
        }

        if !attached_files.is_empty() {
            self.inner
                .bot
                .send_rich_message_with_media_params(
                    self.inner.chat_id,
                    full_rich_msg,
                    attached_files,
                    reply_markup,
                    None,
                    reply_to_msg_id,
                )
                .await
        } else {
            self.inner
                .bot
                .send_rich_message(
                    self.inner.chat_id,
                    full_rich_msg,
                    reply_markup,
                    None,
                    reply_to_msg_id,
                )
                .await
        }
    }

    pub async fn finalize_answer(
        &self,
        full_rich_msg: &crate::bot::models::InputRichMessage,
    ) -> Result<serde_json::Value, String> {
        self.finalize_answer_with_media(full_rich_msg, Vec::new())
            .await
    }

    pub async fn delete_placeholder(&self) {
        let _sync_guard = self.inner.sync_lock.lock().await;
        let placeholder_msg_id = {
            let mut state = self.lock_state();
            state.stopped = true;
            state.placeholder_message_id.take()
        };

        if let Some(msg_id) = placeholder_msg_id {
            let _ = self
                .inner
                .bot
                .delete_message(self.inner.chat_id, msg_id)
                .await;
        }
    }
}

impl GenerationProgressSink for ExecutionTimeline {
    fn on_action(&self, label: &str, activity: Option<ProgressActivity>) {
        {
            let mut state = self.lock_state();
            state.partial_answer.clear();
            state.add_action(label.to_string(), activity, self.inner.max_items);
        }
        self.trigger_sync(true);
    }

    fn on_partial_answer(&self, text: &str) {
        let (should_sync, is_first_writing) = {
            let mut state = self.lock_state();
            let is_already_writing = state.items.last().is_some_and(|it| {
                it.activity == ProgressActivity::Writing && it.state == ProgressState::Active
            });
            let first_writing = !is_already_writing;
            if first_writing {
                state.add_action(
                    "Writing".to_string(),
                    Some(ProgressActivity::Writing),
                    self.inner.max_items,
                );
            }
            state.set_partial_answer(text);
            let now = Instant::now();
            let min_ms = match self.inner.mode {
                TimelineMode::PrivateDraft { .. } => 1200,
                TimelineMode::GroupProgressive => 2500,
            };
            let can_sync = state
                .last_sync_time
                .is_none_or(|t| now.duration_since(t).as_millis() >= min_ms);
            (can_sync, first_writing)
        };

        match self.inner.mode {
            TimelineMode::PrivateDraft { .. } => {
                if should_sync {
                    self.trigger_sync(false);
                }
            }
            TimelineMode::GroupProgressive => {
                // In groups, partial text is never streamed. Sync immediately only
                // on the first transition to the Writing state.
                if is_first_writing {
                    self.trigger_sync(true);
                }
            }
        }
    }

    fn on_failure(&self, error: &str, force_sync: bool) {
        let (placeholder_msg_id, is_group) = {
            let mut state = self.lock_state();
            state.fail_current();
            state.stopped = true;
            state.is_failed = true;
            (
                state.placeholder_message_id.take(),
                matches!(self.inner.mode, TimelineMode::GroupProgressive),
            )
        };

        if let Some(msg_id) = placeholder_msg_id {
            let bot = self.inner.bot.clone();
            let chat_id = self.inner.chat_id;
            let err_msg = format!("⚠️ {error}");
            let err_rich = InputRichMessage::new(vec![RichBlock::Paragraph {
                text: Value::String(err_msg),
            }]);
            let delivery = self.inner.delivery_context.clone();
            tokio::spawn(async move {
                TelegramBotClient::with_delivery_context(delivery, async move {
                    let _ = bot
                        .edit_rich_message(chat_id, msg_id, &err_rich, None)
                        .await;
                })
                .await;
            });
        } else if is_group {
            let bot = self.inner.bot.clone();
            let chat_id = self.inner.chat_id;
            let reply_to = self.inner.reply_to_message_id;
            let err_msg = format!("⚠️ {error}");
            let err_rich = InputRichMessage::new(vec![RichBlock::Paragraph {
                text: Value::String(err_msg),
            }]);
            let delivery = self.inner.delivery_context.clone();
            tokio::spawn(async move {
                TelegramBotClient::with_delivery_context(delivery, async move {
                    let _ = bot
                        .send_rich_message(chat_id, &err_rich, None, None, reply_to)
                        .await;
                })
                .await;
            });
        } else if force_sync {
            self.trigger_sync(true);
        }
    }

    fn on_complete(&self) {
        self.inner.lock_state().finish_all(ProgressState::Done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_markdown_uses_native_rich_blocks() {
        let partial = "## Dua Gaya yang Bertarung\n\nOrbit itu **jatuh terus-menerus**.\n\n---\n\n1. **Gravitasi Bumi** — tarik ke bawah\n2. **Kecepatan tangensial** — dorong ke samping";
        let blocks = parse_streaming_markdown_to_rich_blocks(partial);
        let wire = serde_json::to_string(&blocks).expect("serialize blocks succeeds");

        assert!(blocks
            .iter()
            .any(|block| matches!(block, RichBlock::SectionHeading { .. })));
        assert!(blocks
            .iter()
            .any(|block| matches!(block, RichBlock::Divider { .. })));
        assert!(blocks
            .iter()
            .any(|block| matches!(block, RichBlock::List { .. })));
        assert!(wire.contains("\"type\":\"bold\""));
        assert!(!wire.contains("## Dua Gaya"));
        assert!(!wire.contains("**jatuh terus-menerus**"));
    }

    #[test]
    fn streaming_draft_omits_thinking_header_when_answer_starts() {
        let partial = "Xiao adalah saya sendiri—asisten AI yang sedang kamu ajak mengobrol!";
        let mut blocks = Vec::new();
        if !partial.trim().is_empty() {
            blocks.extend(parse_streaming_markdown_to_rich_blocks(partial));
        }
        if blocks.is_empty() {
            blocks.push(RichBlock::Thinking {
                text: Value::String("🪶 Writing\n8s".to_string()),
            });
        }
        assert!(!blocks
            .iter()
            .any(|b| matches!(b, RichBlock::Thinking { .. })));
        assert!(blocks
            .iter()
            .any(|b| matches!(b, RichBlock::Paragraph { .. })));
    }

    #[test]
    fn streaming_draft_shows_thinking_header_when_answer_empty() {
        let partial = "";
        let mut blocks = Vec::new();
        if !partial.trim().is_empty() {
            blocks.extend(parse_streaming_markdown_to_rich_blocks(partial));
        }
        if blocks.is_empty() {
            blocks.push(RichBlock::Thinking {
                text: Value::String("🧩 Thinking\n3s".to_string()),
            });
        }
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], RichBlock::Thinking { .. }));
    }

    #[test]
    fn group_placeholder_only_shows_status_without_streaming_partial_answer() {
        let partial = "Jawaban lengkap yang sedang di-generate oleh model...";
        let is_group = true;
        let mut blocks = Vec::new();
        if !is_group && !partial.trim().is_empty() {
            blocks.extend(parse_streaming_markdown_to_rich_blocks(partial));
        }
        if blocks.is_empty() {
            blocks.push(RichBlock::Thinking {
                text: Value::String("🪶 Writing\n5s •••".to_string()),
            });
        }
        // In group mode, blocks must ONLY contain RichBlock::Thinking and NO partial text blocks
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], RichBlock::Thinking { .. }));
    }

    #[test]
    fn render_current_status_cycles_loading_dots() {
        let mut state = TimelineState::new();
        state.add_action("Thinking".to_string(), Some(ProgressActivity::Thinking), 10);

        let now = Instant::now();
        let s0 = state.render_current_status(now, true);
        assert!(s0.contains("0s •"));

        let s1 = state.render_current_status(
            now.checked_sub(std::time::Duration::from_secs(1))
                .expect("instant subtraction succeeds"),
            true,
        );
        assert!(s1.contains("1s ••"));

        let s2 = state.render_current_status(
            now.checked_sub(std::time::Duration::from_secs(2))
                .expect("instant subtraction succeeds"),
            true,
        );
        assert!(s2.contains("2s •••"));

        let s3 = state.render_current_status(
            now.checked_sub(std::time::Duration::from_secs(3))
                .expect("instant subtraction succeeds"),
            true,
        );
        assert!(s3.contains("3s •"));
    }

    #[test]
    fn render_current_status_without_elapsed_is_single_line_without_dots() {
        let mut state = TimelineState::new();
        state.add_action("Thinking".to_string(), Some(ProgressActivity::Thinking), 10);
        let s = state.render_current_status(Instant::now(), false);
        assert_eq!(s, "🧩 Thinking");
        assert!(!s.contains('\n'));
        assert!(!s.contains('•'));

        state.add_action("Writing".to_string(), Some(ProgressActivity::Writing), 10);
        let s2 = state.render_current_status(Instant::now(), false);
        assert_eq!(s2, "🪶 Writing");
        assert!(!s2.contains('\n'));
    }

    #[test]
    fn summarizing_activity_renders_sparkles_and_label() {
        let mut state = TimelineState::new();
        state.add_action(
            "Summarizing".to_string(),
            Some(ProgressActivity::Summarizing),
            10,
        );
        let s = state.render_current_status(Instant::now(), true);
        assert!(s.contains("✨ Summarizing"));
    }

    #[test]
    fn for_chat_selects_private_vs_group_timeline_mode() {
        let bot = TelegramBotClient::new("dummy");
        let private_tl = ExecutionTimeline::for_chat(bot.clone(), 12345, 12345, 1, 10, true, None);
        assert!(matches!(
            private_tl.mode(),
            TimelineMode::PrivateDraft {
                draft_id: 1,
                can_stop: true
            }
        ));
        assert_eq!(private_tl.placeholder_message_id(), None);
        assert_eq!(private_tl.reply_to_message_id(), None);

        let group_tl =
            ExecutionTimeline::for_chat(bot, -1001234567, 12345, 2, 10, false, Some(999));
        assert!(matches!(group_tl.mode(), TimelineMode::GroupProgressive));
        assert_eq!(group_tl.placeholder_message_id(), None);
        assert_eq!(group_tl.reply_to_message_id(), Some(999));
    }

    #[tokio::test]
    async fn finalize_answer_skips_when_failed() {
        let bot = TelegramBotClient::new("dummy");
        let tl = ExecutionTimeline::for_chat(bot, -1001234567, 12345, 2, 10, false, Some(999));
        tl.on_failure("Network timeout", false);
        let rich = InputRichMessage::new(vec![]);
        let res = tl.finalize_answer(&rich).await;
        assert!(res.is_ok());
        let val = res.expect("finalize_answer succeeds");
        assert_eq!(val.get("failed").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn draft_input_rich_message_detects_rtl_streaming_content() {
        let partial = "مرحبا بالعالم";
        let mut rich_message = InputRichMessage::new(vec![RichBlock::Paragraph {
            text: Value::String(partial.to_string()),
        }]);
        crate::parser::rtl::apply_rtl_direction(&mut rich_message, partial);
        assert_eq!(rich_message.is_rtl, Some(true));

        let ltr_partial = "Hello world";
        let mut ltr_msg = InputRichMessage::new(vec![RichBlock::Paragraph {
            text: Value::String(ltr_partial.to_string()),
        }]);
        crate::parser::rtl::apply_rtl_direction(&mut ltr_msg, ltr_partial);
        assert!(ltr_msg.is_rtl.is_none());
    }

    #[tokio::test]
    async fn ticker_exits_gracefully_when_timeline_is_dropped() {
        let bot = TelegramBotClient::new("dummy");
        let weak_inner = {
            let tl = ExecutionTimeline::for_chat(bot, 12345, 12345, 1, 10, true, None);
            tl.start_ticker();
            std::sync::Arc::downgrade(&tl.inner)
            // tl is dropped here
        };
        // Give background tasks a moment to awaken
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        // Strong count should be 0 because background tasks only held Weak references
        assert!(weak_inner.upgrade().is_none());
    }
}
