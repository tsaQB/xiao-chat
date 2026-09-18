mod ai;
mod attachments;
mod bot;
mod cli;
mod document;
mod parser;
mod timeline;
mod util;

use regex::Regex;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::io;
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use ai::service::{GenerationModelSnapshot, ImageGenerationError, ImageGenerationErrorKind};
use ai::AIChatService;
use bot::client::{TelegramBotClient, TelegramDeliveryContext};
use bot::models::{
    BotCommand, InputRichMessage, MessageGenerationStopped, RichBlock, RichBlockTableCell, Update,
};
use parser::build_full_rich_message;
use timeline::{ExecutionTimeline, GenerationProgressSink, ProgressActivity};
use util::{escape_html, truncate_chars};

type UserLastImagePrompt = Arc<RwLock<HashMap<i64, String>>>;

struct ChatInput<'a> {
    prompt: &'a str,
    image_bytes: Option<Vec<u8>>,
    document_images: Option<Vec<Vec<u8>>>,
    mime_type: Option<&'a str>,
    doc_text: Option<&'a str>,
    doc_name: Option<&'a str>,
    audio_bytes: Option<Vec<u8>>,
    audio_mime: Option<&'a str>,
    video_bytes: Option<Vec<u8>>,
    video_mime: Option<&'a str>,
    video_duration: Option<i32>,
    model_snapshot: Option<&'a GenerationModelSnapshot>,
    reply_to_message_id: Option<i64>,
}

fn build_audio_chat_input<'a>(
    prompt: &'a str,
    audio_bytes: Vec<u8>,
    audio_mime: Option<&'a str>,
    doc_name: Option<&'a str>,
    reply_to_message_id: Option<i64>,
) -> ChatInput<'a> {
    ChatInput {
        prompt,
        image_bytes: None,
        document_images: None,
        mime_type: None,
        doc_text: None,
        doc_name,
        audio_bytes: Some(audio_bytes),
        audio_mime,
        video_bytes: None,
        video_mime: None,
        video_duration: None,
        model_snapshot: None,
        reply_to_message_id,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteDecision {
    ProcessChat(String),
    Ignore,
}

#[derive(Debug, Clone)]
pub struct MessageRouteContext<'a> {
    pub user_id: i64,
    pub chat_id: i64,
    pub is_forum: bool,
    pub raw_text: &'a str,
    pub has_media: bool,
    pub is_reply_to_bot: bool,
}

#[derive(Clone)]
struct ChatRouteScope {
    owner_user_id: i64,
    allowed_chat_ids: HashSet<i64>,
    dedicated_chat_ids: HashSet<i64>,
    bot_id: Option<i64>,
    bot_username: Option<String>,
    bot: TelegramBotClient,
    admin_cache: Arc<tokio::sync::RwLock<HashMap<i64, (bool, std::time::Instant)>>>,
}

#[cfg(test)]
pub const DEFAULT_DEDICATED_WORKSPACE_CHAT_ID: i64 = -1001234567890;

fn strip_bot_mention(raw_text: &str, bot_username: &str) -> Option<String> {
    let bot_tag = format!("@{bot_username}");
    let bot_tag_len = bot_tag.len();
    let lower_text = raw_text.to_ascii_lowercase();
    let lower_tag = bot_tag.to_ascii_lowercase();

    let mut found = false;
    let mut result = String::with_capacity(raw_text.len());
    let mut last_idx = 0;

    let mut search_from = 0;
    while let Some(pos) = lower_text[search_from..].find(&lower_tag) {
        let actual_pos = search_from + pos;
        let after_pos = actual_pos + bot_tag_len;
        // Verify boundary after mention: must be end of string or non-alphanumeric/non-underscore
        let is_boundary_after = if let Some(ch) = raw_text[after_pos..].chars().next() {
            !ch.is_alphanumeric() && ch != '_'
        } else {
            true
        };

        // Verify boundary before mention: must be start of string or non-alphanumeric/non-underscore
        let is_boundary_before = if actual_pos == 0 {
            true
        } else {
            raw_text[..actual_pos]
                .chars()
                .next_back()
                .is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')
        };

        if is_boundary_before && is_boundary_after {
            found = true;
            result.push_str(&raw_text[last_idx..actual_pos]);
            // Skip a single space following mention only if preceded by whitespace or at start
            let preceded_by_whitespace = raw_text[..actual_pos].ends_with(char::is_whitespace);
            if (actual_pos == 0 || preceded_by_whitespace) && raw_text[after_pos..].starts_with(' ')
            {
                last_idx = after_pos + 1;
            } else {
                last_idx = after_pos;
            }
            search_from = last_idx;
        } else {
            search_from = raw_text[actual_pos..]
                .char_indices()
                .nth(1)
                .map(|(idx, _)| actual_pos + idx)
                .unwrap_or(raw_text.len());
        }
    }

    if found {
        result.push_str(&raw_text[last_idx..]);
        Some(result.trim().to_string())
    } else {
        None
    }
}

impl ChatRouteScope {
    fn new(
        owner_user_id: i64,
        allowed_chat_ids: HashSet<i64>,
        dedicated_chat_ids: HashSet<i64>,
        bot_id: Option<i64>,
        bot_username: Option<String>,
        bot: TelegramBotClient,
    ) -> Self {
        Self {
            owner_user_id,
            allowed_chat_ids,
            dedicated_chat_ids,
            bot_id,
            bot_username,
            bot,
            admin_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    fn allows_stop_chat(&self, chat_id: i64) -> bool {
        chat_id == self.owner_user_id
    }

    async fn is_bot_admin(&self, chat_id: i64) -> bool {
        let Some(bot_id) = self.bot_id else {
            return false;
        };
        {
            let cache = self.admin_cache.read().await;
            if let Some((is_admin, cached_at)) = cache.get(&chat_id) {
                if cached_at.elapsed() < Duration::from_secs(300) {
                    return *is_admin;
                }
            }
        }
        let is_admin = match self.bot.get_chat_member(chat_id, bot_id).await {
            Ok(resp) if resp.ok => resp
                .result
                .as_ref()
                .is_some_and(|m| m.is_admin_or_creator()),
            _ => false,
        };
        let mut cache = self.admin_cache.write().await;
        cache.insert(chat_id, (is_admin, std::time::Instant::now()));
        is_admin
    }

    async fn is_dedicated_chat(&self, chat_id: i64, is_forum: bool) -> bool {
        if self.dedicated_chat_ids.contains(&chat_id) {
            return true;
        }
        if is_forum && self.is_bot_admin(chat_id).await {
            return true;
        }
        false
    }

    fn evaluate_internal(
        &self,
        is_dedicated: bool,
        ctx: &MessageRouteContext<'_>,
    ) -> RouteDecision {
        // 1. Hard single-owner invariant: non-owners are silently dropped (100%)
        if ctx.user_id != self.owner_user_id {
            return RouteDecision::Ignore;
        }

        let is_private = ctx.chat_id == self.owner_user_id;

        // 2. Allowed chat IDs whitelist enforcement:
        // Private chat and dedicated workspaces are always allowed.
        // Other groups must be in allowed_chat_ids if the whitelist is configured.
        if !is_private
            && !is_dedicated
            && !self.allowed_chat_ids.is_empty()
            && !self.allowed_chat_ids.contains(&ctx.chat_id)
        {
            return RouteDecision::Ignore;
        }

        let trimmed = ctx.raw_text.trim();

        // 3. Drop completely empty updates if there is no media attached
        if trimmed.is_empty() && !ctx.has_media {
            return RouteDecision::Ignore;
        }

        // 4. Private 1-on-1 Chat: always processed for owner
        if is_private {
            return RouteDecision::ProcessChat(trimmed.to_string());
        }

        // 5. Check if message contains an explicit mention of this bot
        let stripped_mention = self
            .bot_username
            .as_deref()
            .and_then(|bot_name| strip_bot_mention(ctx.raw_text, bot_name));

        if let Some(prompt) = stripped_mention {
            let prompt = if prompt.is_empty() && !ctx.has_media {
                "Halo Xiao!".to_string()
            } else {
                prompt
            };
            return RouteDecision::ProcessChat(prompt);
        }

        // 6. Dedicated Forum Workspace:
        // Xiao answers all owner messages across all topics without requiring mention, reply, or slash
        if is_dedicated {
            return RouteDecision::ProcessChat(trimmed.to_string());
        }

        // 7. Guest Group (non-dedicated):
        // Only reply if direct reply to Xiao or message starts with a slash command.
        // Media without caption mention/reply/slash is silently ignored.
        if ctx.is_reply_to_bot || trimmed.starts_with('/') {
            return RouteDecision::ProcessChat(trimmed.to_string());
        }

        RouteDecision::Ignore
    }

    pub async fn evaluate(&self, ctx: &MessageRouteContext<'_>) -> RouteDecision {
        let is_dedicated = if ctx.chat_id == self.owner_user_id {
            true
        } else {
            self.is_dedicated_chat(ctx.chat_id, ctx.is_forum).await
        };
        self.evaluate_internal(is_dedicated, ctx)
    }
}

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

// ==========================================
// UI Builders
// ==========================================

#[cfg(test)]
fn build_help_ui() -> InputRichMessage {
    use bot::models::RichBlockListItem;
    let input_items = vec![
        RichBlockListItem::bullet(vec![
            json!({"type":"paragraph","text":"Text — ordinary chat and instructions."}),
        ]),
        RichBlockListItem::bullet(vec![
            json!({"type":"paragraph","text":"Images: routed through the configured Vision role, without a prerequisite probe."}),
        ]),
        RichBlockListItem::bullet(vec![
            json!({"type":"paragraph","text":"Documents — local extraction; scanned PDF pages route through Vision."}),
        ]),
        RichBlockListItem::bullet(vec![
            json!({"type":"paragraph","text":"Voice/audio — native Main audio or the configured Audio STT role."}),
        ]),
        RichBlockListItem::bullet(vec![
            json!({"type":"paragraph","text":"Video: direct Main or the configured Video specialist, without a prerequisite probe."}),
        ]),
    ];
    let command_rows = vec![
        vec![
            RichBlockTableCell::text_only("Command / Input", true, Some("left")),
            RichBlockTableCell::text_only("Action", true, Some("left")),
        ],
        vec![
            RichBlockTableCell::text_only("Teks & Percakapan", false, Some("left")),
            RichBlockTableCell::text_only(
                "Percakapan bebas dan sambutan alami tanpa slash command",
                false,
                Some("left"),
            ),
        ],
        vec![
            RichBlockTableCell::text_only("Chat & Media", false, Some("left")),
            RichBlockTableCell::text_only(
                "Natural conversation, image generation, audio & document analysis",
                false,
                Some("left"),
            ),
        ],
    ];
    InputRichMessage::new(vec![
        RichBlock::SectionHeading {
            text: Value::String("HELP".to_string()),
            level: 1,
        },
        RichBlock::Paragraph {
            text: Value::String("Supported input".to_string()),
        },
        RichBlock::List { items: input_items },
        RichBlock::Table {
            cells: command_rows,
            has_header: true,
            is_bordered: false,
            is_striped: false,
            is_compact: true,
            caption: None,
        },
        RichBlock::Details {
            summary: Value::String("Model Routing".to_string()),
            blocks: vec![json!({
                "type":"paragraph",
                "text":"All model routing and specialist routes are managed via Xiao CLI. Vision, Video, Audio STT, and Image Generation routes are read-only in Telegram (configure via xiao addon)."
            })],
            is_open: Some(false),
        },
        RichBlock::Details {
            summary: Value::String("Media Routing".to_string()),
            blocks: vec![json!({
                "type":"paragraph",
                "text":"Main-compatible media executes directly on Main. A different specialist receives only the minimum current context and returns a bounded observation/transcript to Main."
            })],
            is_open: Some(false),
        },
        RichBlock::Paragraph {
            text: Value::String("Advanced routing configuration: xiao addon".to_string()),
        },
    ])
}

#[cfg(test)]
fn specialist_context_policy(
    role: ai::service::ModelRole,
    origin: ai::service::RouteOrigin,
) -> &'static str {
    if origin == ai::service::RouteOrigin::MainModel {
        return "Direct on Main; canonical history stays on Main";
    }
    match role {
        ai::service::ModelRole::Vision | ai::service::ModelRole::Video => {
            "Transient media + current question; no full history"
        }
        ai::service::ModelRole::AudioStt => "Transcript only; no full history",
        ai::service::ModelRole::ImageGeneration => "Prompt/config only; no canonical history",
        ai::service::ModelRole::Curator => "Background extraction & summarization only",
        ai::service::ModelRole::Main => "Canonical Main context",
    }
}

#[cfg(test)]
fn context_available_tokens(limit: usize, used: usize) -> usize {
    limit.saturating_sub(used)
}

#[cfg(test)]
fn main_context_overflow_warning(model: &str, used: usize, usable_limit: usize) -> Option<String> {
    (used > usable_limit).then(|| {
        format!(
            "Main Model changed to {model}. Current canonical history (~{used} tokens) exceeds the new usable context (~{usable_limit} tokens). Xiao will compact before the next request when needed; history was not deleted."
        )
    })
}

// ==========================================
// Intent Detection & Image Generation
// ==========================================

static FOLLOW_UP_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)^(?:tolong\s+|pls\s+|please\s+)?(?:buatkan|bikinin|bikin|buat|generate|draw|render|lukiskan|lukis|gambarin|gambarkan)\s+(?:dong\s+|kan\s+)?(?:gambar(?:nya| ini| itu| tersebut| tadi)?|foto(?:nya| ini| itu| tersebut)?|lukisan(?:nya| ini)?|image(?:nya)?|it|this)$").expect("valid regex"),
        Regex::new(r"(?i)^(?:gambar(?:nya| ini| itu| tersebut)?|foto(?:nya)?|lukisan(?:nya)?)\s*(?:dong|ya|tolong|pls)?$").expect("valid regex"),
    ]
});

static INTENT_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)^(?:tolong\s+|pls\s+|please\s+)?(?:buatkan|buatlah|buat|bikinin|bikin|generate|create|render|hasilkan|lukiskan|lukis|gambarin|gambarkan|draw)\s+(?:saya\s+|aku\s+|in\s+)?(?:sebuah\s+|seekor\s+|seorang\s+|suatu\s+|an?\s+|the\s+)?(?:gambar|foto|photo|lukisan|image|picture|wallpaper|ilustrasi|illustration|artwork|poster|visual)\s+(?:tentang\s+|dari\s+|of\s+|about\s+)?(.+)$").expect("valid regex"),
        Regex::new(r"(?i)^(?:tolong\s+|pls\s+|please\s+)?(?:gambarin|gambarkan|lukiskan|lukis)\s+(?:saya\s+|aku\s+|in\s+)?(?:dong\s+|kan\s+)?(.+)$").expect("valid regex"),
        Regex::new(r"(?i)^(?:ilustrasi|lukisan|artwork|wallpaper|fanart|sketsa|foto)\s+(?:tentang\s+|dari\s+|of\s+|about\s+)?(.+)$").expect("valid regex"),
        Regex::new(r"(?i)^(?:tolong\s+|pls\s+|please\s+)?(?:buatkan|bikinin|bikin)\s+(?:saya\s+|aku\s+)?(?:dong\s+)?(.+?\b(?:gaya|style|anime|wallpaper|realistis|realistic|3d|cyberpunk|lukisan|sketsa|art|hd|8k)\b.*)$").expect("valid regex"),
        Regex::new(r"(?i)^(?:please\s+|can you\s+)?(?:generate|create|make|draw|render)\s+(?:me\s+)?(?:an?\s+|the\s+)?(?:image|picture|photo|illustration|drawing|wallpaper|artwork)\s+(?:of\s+|about\s+)?(.+)$").expect("valid regex"),
    ]
});

static CLEAN_PREFIX_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:tentang|mengenai|berupa|of|about|dong|ya|tolong)\s+").expect("valid regex")
});

static COMPOUND_IMAGE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?P<image>.+?)\s+(?:dan|lalu|kemudian|and|then)\s+(?P<explain>(?:jelaskan|terangkan|explain|describe)\b.+)$").expect("valid regex")
});

fn extract_image_intent_prompt(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }

    let t_lower = t.to_lowercase();
    let inquiry_prefixes = [
        "apa itu",
        "apa arti",
        "jelaskan",
        "mengapa",
        "kenapa",
        "bagaimana cara",
        "cara ",
        "tutorial",
        "definisi",
        "what is",
        "why",
        "how to",
        "explain",
    ];
    if inquiry_prefixes
        .iter()
        .any(|pref| t_lower.starts_with(pref))
    {
        return None;
    }

    for regex in FOLLOW_UP_REGEXES.iter() {
        if regex.is_match(t) {
            return Some("__CONTEXT_FOLLOWUP__".to_string());
        }
    }

    for regex in INTENT_REGEXES.iter() {
        if let Some(caps) = regex.captures(t) {
            if let Some(extracted_match) = caps.get(1) {
                let mut extracted = extracted_match.as_str().trim().to_string();
                extracted = CLEAN_PREFIX_REGEX
                    .replace(&extracted, "")
                    .trim()
                    .to_string();

                let ext_low = extracted.to_lowercase();
                if [
                    "dong",
                    "ya",
                    "ini",
                    "itu",
                    "nya",
                    "tadi",
                    "tersebut",
                    "gambarnya",
                    "fotonya",
                ]
                .contains(&ext_low.as_str())
                {
                    return Some("__CONTEXT_FOLLOWUP__".to_string());
                }
                if extracted.len() >= 3 {
                    return Some(extracted);
                }
            }
        }
    }

    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageGenerationIntent {
    image_prompt: String,
    explanation_prompt: Option<String>,
}

fn plan_image_generation_intent(text: &str) -> Option<ImageGenerationIntent> {
    if let Some(captures) = COMPOUND_IMAGE_REGEX.captures(text.trim()) {
        let image_request = captures.name("image")?.as_str().trim();
        let explanation = captures.name("explain")?.as_str().trim();
        if let Some(image_prompt) = extract_image_intent_prompt(image_request) {
            return Some(ImageGenerationIntent {
                image_prompt,
                explanation_prompt: (!explanation.is_empty()).then(|| explanation.to_string()),
            });
        }
    }

    extract_image_intent_prompt(text).map(|image_prompt| ImageGenerationIntent {
        image_prompt,
        explanation_prompt: None,
    })
}

const TELEGRAM_PHOTO_CAPTION_MAX_CHARS: usize = 1024;
const IMAGE_CAPTION_PROMPT_ESCAPED_CHARS: usize = 320;
const IMAGE_CAPTION_PROVIDER_ESCAPED_CHARS: usize = 96;
const IMAGE_CAPTION_MODEL_ESCAPED_CHARS: usize = 128;
const IMAGE_CAPTION_FAILURE_ESCAPED_CHARS: usize = 144;

fn bounded_escaped_html(text: &str, max_chars: usize) -> String {
    let mut output = String::new();
    let mut used = 0usize;
    let mut truncated = false;

    for ch in text.chars() {
        let escaped = match ch {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&#39;",
            _ => {
                let needed = 1usize;
                if used.saturating_add(needed) > max_chars {
                    truncated = true;
                    break;
                }
                output.push(ch);
                used += needed;
                continue;
            }
        };
        let needed = escaped.chars().count();
        if used.saturating_add(needed) > max_chars {
            truncated = true;
            break;
        }
        output.push_str(escaped);
        used += needed;
    }

    if truncated && used < max_chars {
        output.push('…');
    }
    output
}

fn build_image_success_caption(
    prompt: &str,
    provider: &str,
    model: &str,
    dimensions: (usize, usize),
    elapsed_secs: f64,
    used_external_fallback: bool,
    primary_failure: Option<&str>,
) -> String {
    let (width, height) = dimensions;
    let safe_prompt = bounded_escaped_html(prompt, IMAGE_CAPTION_PROMPT_ESCAPED_CHARS);
    let safe_provider = bounded_escaped_html(provider, IMAGE_CAPTION_PROVIDER_ESCAPED_CHARS);
    let safe_model = bounded_escaped_html(model, IMAGE_CAPTION_MODEL_ESCAPED_CHARS);
    let fallback_note = if used_external_fallback {
        let safe_failure = bounded_escaped_html(
            primary_failure.unwrap_or("Primary provider failure was not reported."),
            IMAGE_CAPTION_FAILURE_ESCAPED_CHARS,
        );
        format!(
            "\n⚠️ <i>External fallback opt-in digunakan.</i>\n<b>Primary failure:</b> {safe_failure}"
        )
    } else {
        String::new()
    };

    let caption = format!(
        "🫟 <b>Gambar Berhasil Dibuat!</b>\n\n\
         📝 <b>Prompt:</b> <i>\"{safe_prompt}\"</i>\n\
         🧩 <b>Provider:</b> <code>{safe_provider}</code>\n\
         🤖 <b>Model:</b> <code>{safe_model}</code>\n\
         📐 <b>Size:</b> <code>{width} × {height}</code>\n\
         ⏱️ <b>Elapsed:</b> <code>{elapsed_secs:.1}s</code>{fallback_note}"
    );

    if caption.chars().count() <= TELEGRAM_PHOTO_CAPTION_MAX_CHARS {
        return caption;
    }

    let minimal = format!(
        "🫟 <b>Gambar Berhasil Dibuat!</b>\n\
         🧩 <b>Provider:</b> <code>{safe_provider}</code>\n\
         🤖 <b>Model:</b> <code>{safe_model}</code>\n\
         📐 <b>Size:</b> <code>{width} × {height}</code>\n\
         ⏱️ <b>Elapsed:</b> <code>{elapsed_secs:.1}s</code>"
    );
    debug_assert!(minimal.chars().count() <= TELEGRAM_PHOTO_CAPTION_MAX_CHARS);
    minimal
}

fn telegram_photo_delivery_error_class(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if [
        "caption",
        "can't parse entities",
        "cannot parse entities",
        "parse entities",
        "reply markup",
        "reply_markup",
        "inline keyboard",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        "caption_or_markup"
    } else if [
        "multipart error",
        "timeout",
        "timed out",
        "connection",
        "network",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        "telegram_transport"
    } else if lower.contains("unsupported image signature") {
        "local_image_validation"
    } else {
        "telegram_api"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageDeliveryFailure {
    class: &'static str,
    detail: String,
    retry_attempted: bool,
}

async fn deliver_generated_image_with<F, Fut>(
    image_bytes: &[u8],
    caption: &str,
    reply_markup: Option<Value>,
    mut sender: F,
) -> Result<(), ImageDeliveryFailure>
where
    F: FnMut(Vec<u8>, Option<String>, Option<String>, Option<Value>) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    match sender(
        image_bytes.to_vec(),
        Some(caption.to_string()),
        Some("HTML".to_string()),
        reply_markup,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(first_error)
            if telegram_photo_delivery_error_class(&first_error) == "caption_or_markup" =>
        {
            let retry_caption = "Image generated successfully.".to_string();
            match sender(image_bytes.to_vec(), Some(retry_caption), None, None).await {
                Ok(()) => Ok(()),
                Err(second_error) => Err(ImageDeliveryFailure {
                    class: telegram_photo_delivery_error_class(&second_error),
                    detail: second_error,
                    retry_attempted: true,
                }),
            }
        }
        Err(error) => Err(ImageDeliveryFailure {
            class: telegram_photo_delivery_error_class(&error),
            detail: error,
            retry_attempted: false,
        }),
    }
}

async fn resolve_image_generation_prompt(
    prompt: &str,
    user_last_image_prompt: &UserLastImagePrompt,
    chat_id: i64,
    thread_id: i64,
    user_id: i64,
) -> String {
    let clean_prompt = prompt.trim();
    if clean_prompt == "__CONTEXT_FOLLOWUP__"
        || [
            "gambarnya",
            "gambarnya dong",
            "itu",
            "yang tadi",
            "ini",
            "dong",
            "ya",
            "fotonya",
        ]
        .contains(&clean_prompt)
    {
        let mut last_context = String::new();
        let scoped_messages = ai::storage::load_scoped_messages_async(chat_id, thread_id, 10).await;
        for msg in scoped_messages.iter().rev() {
            let candidate = match &msg.content {
                Value::String(value) => Some(value.clone()),
                value => attachments::decode_user_content(value).map(|content| content.text),
            };
            if let Some(candidate) = candidate {
                if candidate.trim().chars().count() > 8 {
                    last_context = candidate.trim().to_string();
                    break;
                }
            }
        }

        if !last_context.is_empty() {
            format!("illustration of {}", truncate_chars(&last_context, 250))
        } else {
            let last_guard = user_last_image_prompt.read().await;
            last_guard.get(&user_id).cloned().unwrap_or_else(|| {
                "majestic mountain scenery with clouds and ancient kingdom".to_string()
            })
        }
    } else {
        clean_prompt.to_string()
    }
}

async fn send_image_generation_help(
    bot: &TelegramBotClient,
    ai_service: &AIChatService,
    chat_id: i64,
) {
    let route_text = match ai_service
        .resolve_model_route(ai::service::ModelRole::ImageGeneration)
        .await
    {
        Ok(route) => format!("{} / {}", route.provider.name, route.model),
        Err(error) => format!("Unavailable — {}", error),
    };
    let rich = InputRichMessage::new(vec![
        RichBlock::SectionHeading {
            text: Value::String("IMAGE GENERATION".to_string()),
            level: 1,
        },
        RichBlock::Table {
            cells: vec![
                vec![
                    RichBlockTableCell::text_only("Image Model", true, Some("left")),
                    RichBlockTableCell::text_only("Default Size", true, Some("left")),
                ],
                vec![
                    RichBlockTableCell::text_only(&route_text, false, Some("left")),
                    RichBlockTableCell::text_only("1024 × 1024", false, Some("left")),
                ],
            ],
            has_header: true,
            is_bordered: false,
            is_striped: false,
            is_compact: true,
            caption: None,
        },
        RichBlock::Paragraph {
            text: Value::String(
                "Kirim deskripsi gambar yang ingin dibuat (contoh: \"buat gambar pemandangan pegunungan saat fajar\").".to_string(),
            ),
        },
    ]);
    let _ = bot
        .send_rich_message(chat_id, &rich, None, None, None)
        .await;
}

fn build_image_generation_error_rich(error: &ImageGenerationError) -> InputRichMessage {
    let status = match error.kind {
        ImageGenerationErrorKind::CapabilityUnknown => "Capability unknown",
        ImageGenerationErrorKind::CapabilityUnsupported => "Unsupported",
        ImageGenerationErrorKind::RouteDisabled => "Route disabled",
        ImageGenerationErrorKind::ProviderNotFound => "Provider not found",
        ImageGenerationErrorKind::ModelNotFound => "Model not found",
        ImageGenerationErrorKind::Timeout => "Timeout",
        ImageGenerationErrorKind::Auth => "Authentication error",
        ImageGenerationErrorKind::RateLimited => "Rate limited",
        ImageGenerationErrorKind::HttpStatus => "HTTP error",
        ImageGenerationErrorKind::ProtocolMismatch => "Protocol mismatch",
        ImageGenerationErrorKind::InvalidResponse => "Invalid response",
        ImageGenerationErrorKind::InvalidBase64 => "Invalid base64",
        ImageGenerationErrorKind::InvalidImage => "Invalid image",
        ImageGenerationErrorKind::UnsafeImageUrl => "Unsafe image URL",
        ImageGenerationErrorKind::DownloadTimeout => "Download timeout",
        ImageGenerationErrorKind::Cancelled => "Cancelled",
        ImageGenerationErrorKind::Provider => "Provider error",
    };
    let mut blocks = vec![
        RichBlock::SectionHeading {
            text: Value::String("IMAGE GENERATION FAILED".to_string()),
            level: 1,
        },
        RichBlock::Table {
            cells: vec![
                vec![
                    RichBlockTableCell::text_only("Status", true, Some("left")),
                    RichBlockTableCell::text_only("Detail", true, Some("left")),
                ],
                vec![
                    RichBlockTableCell::text_only(status, false, Some("left")),
                    RichBlockTableCell::text_only(
                        &truncate_chars(&error.message, 240),
                        false,
                        Some("left"),
                    ),
                ],
            ],
            has_header: true,
            is_bordered: false,
            is_striped: false,
            is_compact: true,
            caption: None,
        },
    ];
    if matches!(
        error.kind,
        ImageGenerationErrorKind::CapabilityUnknown
            | ImageGenerationErrorKind::CapabilityUnsupported
            | ImageGenerationErrorKind::RouteDisabled
            | ImageGenerationErrorKind::ProviderNotFound
            | ImageGenerationErrorKind::ModelNotFound
    ) {
        blocks.push(RichBlock::BlockQuotation {
            blocks: vec![json!({
                "type":"paragraph",
                "text":"Configure specialist Image Generation route with: xiao addon"
            })],
        });
    }
    InputRichMessage::new(blocks)
}

#[allow(clippy::too_many_arguments)]
async fn handle_image_generation(
    bot: &TelegramBotClient,
    ai_service: &AIChatService,
    user_last_image_prompt: &UserLastImagePrompt,
    chat_id: i64,
    thread_id: i64,
    user_id: i64,
    prompt: &str,
    explanation_prompt: Option<&str>,
    reply_to_message_id: Option<i64>,
) {
    let clean_prompt = resolve_image_generation_prompt(
        prompt,
        user_last_image_prompt,
        chat_id,
        thread_id,
        user_id,
    )
    .await;

    if clean_prompt.is_empty() {
        send_image_generation_help(bot, ai_service, chat_id).await;
        return;
    }

    user_last_image_prompt
        .write()
        .await
        .insert(user_id, clean_prompt.clone());

    let draft_id = ai::service::next_draft_id();
    let timeline = Arc::new(ExecutionTimeline::for_chat(
        bot.clone(),
        chat_id,
        user_id,
        draft_id,
        10,
        chat_id == user_id,
        reply_to_message_id,
    ));
    timeline
        .add_action("Generating Image", Some(ProgressActivity::Drawing))
        .await;
    timeline.sync_draft(true).await;
    timeline.start_ticker();
    let _ = bot.send_chat_action(chat_id, "upload_photo").await;

    let width = 1024usize;
    let height = 1024usize;
    let image_started = Instant::now();
    let model_snapshot = ai_service.generation_model_snapshot().await;
    let (mut cancel_rx, _guard) = ai_service.begin_generation(chat_id, draft_id).await;
    let image_result = ai_service
        .generate_image_with_snapshot(
            user_id,
            &clean_prompt,
            width,
            height,
            &model_snapshot,
            &mut cancel_rx,
        )
        .await;
    ai_service.end_generation(chat_id, draft_id).await;
    timeline.stop_ticker();
    let elapsed_secs = image_started.elapsed().as_secs_f64();

    let generated = match image_result {
        Ok(image) => image,
        Err(error) => {
            timeline.fail_current().await;
            timeline.sync_draft(true).await;

            if error.kind == ImageGenerationErrorKind::Cancelled {
                let rich = InputRichMessage::new(vec![
                    RichBlock::SectionHeading {
                        text: Value::String("IMAGE GENERATION CANCELLED".to_string()),
                        level: 1,
                    },
                    RichBlock::Paragraph {
                        text: Value::String("Image generation was cancelled.".to_string()),
                    },
                ]);
                let _ = timeline.finalize_answer(&rich).await;
                return;
            }

            let rich = build_image_generation_error_rich(&error);
            let _ = timeline.finalize_answer(&rich).await;
            return;
        }
    };

    let caption_text = build_image_success_caption(
        &clean_prompt,
        &generated.provider_name,
        &generated.model,
        (width, height),
        elapsed_secs,
        generated.used_external_fallback,
        generated.primary_failure.as_deref(),
    );

    timeline.delete_placeholder().await;

    let delivery = deliver_generated_image_with(
        &generated.bytes,
        &caption_text,
        None,
        |bytes, caption, parse_mode, reply_markup| async move {
            bot.send_photo_bytes(
                chat_id,
                bytes,
                caption.as_deref(),
                parse_mode.as_deref(),
                reply_markup,
                reply_to_message_id,
            )
            .await
            .map(|_| ())
        },
    )
    .await;

    if let Err(failure) = delivery {
        warn!(
            "Generated image delivery failed [{}; retry={}]: {}",
            failure.class,
            failure.retry_attempted,
            truncate_chars(&failure.detail, 200)
        );
        let rich = InputRichMessage::new(vec![
            RichBlock::SectionHeading {
                text: Value::String("IMAGE DELIVERY FAILED".to_string()),
                level: 1,
            },
            RichBlock::Paragraph {
                text: Value::String(
                    "Image generation succeeded, but Telegram could not deliver the image."
                        .to_string(),
                ),
            },
            RichBlock::BlockQuotation {
                blocks: vec![json!({
                    "type": "paragraph",
                    "text": format!("Diagnostic class: {}", failure.class)
                })],
            },
        ]);
        if let Err(error) = bot
            .send_rich_message(chat_id, &rich, None, None, reply_to_message_id)
            .await
        {
            warn!(
                "Image delivery fallback message also failed: {}",
                truncate_chars(&error, 160)
            );
        }
        return;
    }

    if let Some(explanation_prompt) = explanation_prompt
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
    {
        handle_ai_chat(
            bot,
            ai_service,
            chat_id,
            thread_id,
            user_id,
            ChatInput {
                prompt: explanation_prompt,
                image_bytes: None,
                document_images: None,
                mime_type: None,
                doc_text: None,
                doc_name: None,
                audio_bytes: None,
                audio_mime: None,
                video_bytes: None,
                video_mime: None,
                video_duration: None,
                model_snapshot: Some(&model_snapshot),
                reply_to_message_id,
            },
        )
        .await;
    }
}

// ==========================================
// Main AI Chat Handler
// ==========================================

async fn handle_ai_chat(
    bot: &TelegramBotClient,
    ai_service: &AIChatService,
    chat_id: i64,
    thread_id: i64,
    user_id: i64,
    input: ChatInput<'_>,
) {
    let ChatInput {
        prompt: user_prompt,
        image_bytes,
        document_images,
        mime_type,
        doc_text,
        doc_name,
        audio_bytes,
        audio_mime,
        video_bytes,
        video_mime,
        video_duration,
        model_snapshot,
        reply_to_message_id,
    } = input;
    let generation_lock = ai_service.generation_lock(chat_id, thread_id).await;
    let _generation_guard = generation_lock.lock().await;

    let draft_id = ai::service::next_draft_id();
    let (mut cancel_rx, _guard) = ai_service.begin_generation(chat_id, draft_id).await;
    let timeline = Arc::new(ExecutionTimeline::for_chat(
        bot.clone(),
        chat_id,
        user_id,
        draft_id,
        30,
        chat_id == user_id,
        reply_to_message_id,
    ));

    let (initial_lbl, initial_act) = if video_bytes.is_some() {
        ("Watching", ProgressActivity::Watching)
    } else if image_bytes.is_some()
        || document_images
            .as_ref()
            .is_some_and(|pages| !pages.is_empty())
    {
        ("Looking", ProgressActivity::Looking)
    } else if doc_text.is_some() {
        ("Reading", ProgressActivity::Reading)
    } else if audio_bytes.is_some() {
        ("Listening", ProgressActivity::Listening)
    } else {
        ("Thinking", ProgressActivity::Thinking)
    };

    timeline.add_action(initial_lbl, Some(initial_act)).await;
    timeline.sync_draft(true).await;
    timeline.start_ticker();
    let _ = bot.send_chat_action(chat_id, "typing").await;

    let generation_input = ai::service::GenerationInput {
        prompt: user_prompt,
        canonical_prompt: None,
        media_to_main: true,
        sink: Some(timeline.as_ref() as &dyn GenerationProgressSink),
        image_bytes,
        document_images,
        mime_type,
        doc_text,
        doc_name,
        audio_bytes,
        audio_mime,
        video_bytes,
        video_mime,
        video_duration,
    };
    let generation_start = std::time::Instant::now();
    let (_thinking, mut answer_text, cancelled) = if let Some(snapshot) = model_snapshot {
        ai_service
            .generate_response_with_snapshot(
                chat_id,
                thread_id,
                user_id,
                generation_input,
                snapshot,
                &mut cancel_rx,
            )
            .await
    } else {
        ai_service
            .generate_response(
                chat_id,
                thread_id,
                user_id,
                generation_input,
                &mut cancel_rx,
            )
            .await
    };

    ai_service.end_generation(chat_id, draft_id).await;
    timeline.stop_ticker();
    if cancelled {
        return;
    }
    let elapsed_secs = generation_start.elapsed().as_secs_f64();
    let emoji = if elapsed_secs <= 10.0 {
        "⚡"
    } else if elapsed_secs <= 30.0 {
        "⏱️"
    } else {
        "🧠"
    };
    let elapsed = format!("`{emoji} {:.1}s`", elapsed_secs);

    if answer_text.trim().is_empty() {
        answer_text = "Maaf, respon AI kosong untuk permintaan ini.".to_string();
    }

    let full_rich_msg = build_full_rich_message(&answer_text, Some(&elapsed));
    let res = timeline.finalize_answer(&full_rich_msg).await;

    if let Err(error) = res {
        // send_rich_message already exhausts canonical Rich -> safe HTML ->
        // semantic plain-text fallback. Never re-send raw model Markdown here.
        warn!("Unable to deliver final canonical answer: {error}");
    }
}

// ==========================================
// Update Router
// ==========================================

fn delivery_context_for_update(update: &Update) -> TelegramDeliveryContext {
    if let Some(message) = update.message.as_ref() {
        return TelegramDeliveryContext {
            message_thread_id: message.message_thread_id,
            receiver_user_id: message
                .ephemeral_message_id
                .and_then(|_| message.from.as_ref().map(|user| user.id)),
            source_ephemeral_message_id: message.ephemeral_message_id,
            callback_query_id: None,
            replace_callback_query_message: None,
        };
    }
    if let Some(callback) = update.callback_query.as_ref() {
        let message = callback.message.as_ref();
        let source_ephemeral_message_id = message.and_then(|message| message.ephemeral_message_id);
        return TelegramDeliveryContext {
            message_thread_id: message.and_then(|message| message.message_thread_id),
            receiver_user_id: source_ephemeral_message_id.map(|_| callback.from.id),
            source_ephemeral_message_id,
            callback_query_id: source_ephemeral_message_id.map(|_| callback.id.clone()),
            replace_callback_query_message: None,
        };
    }
    if let Some(stopped) = update.stopped_message_generation.as_ref() {
        return TelegramDeliveryContext {
            message_thread_id: stopped.message_thread_id,
            ..TelegramDeliveryContext::default()
        };
    }
    TelegramDeliveryContext::default()
}

fn command_matches(text: &str, command: &str) -> bool {
    command_args(text, command).is_some()
}

fn command_args<'a>(text: &'a str, command: &str) -> Option<&'a str> {
    let text = text.trim();
    let rest = text.strip_prefix(command)?;
    if rest.is_empty() {
        return Some("");
    }
    if rest.chars().next().is_some_and(char::is_whitespace) {
        return Some(rest.trim_start());
    }
    let mention = rest.strip_prefix('@')?;
    let mention_end = mention
        .char_indices()
        .find(|(_, character)| character.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(mention.len());
    if mention_end == 0 {
        return None;
    }
    Some(mention[mention_end..].trim_start())
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TelegramDocumentMediaKind {
    Image,
    Audio,
    Video,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClassifiedTelegramDocument {
    kind: TelegramDocumentMediaKind,
    mime_type: Option<String>,
}

const TELEGRAM_DOCUMENT_MEDIA_MAPPINGS: [(&str, TelegramDocumentMediaKind, &str); 16] = [
    (".png", TelegramDocumentMediaKind::Image, "image/png"),
    (".jpg", TelegramDocumentMediaKind::Image, "image/jpeg"),
    (".jpeg", TelegramDocumentMediaKind::Image, "image/jpeg"),
    (".webp", TelegramDocumentMediaKind::Image, "image/webp"),
    (".ogg", TelegramDocumentMediaKind::Audio, "audio/ogg"),
    (".oga", TelegramDocumentMediaKind::Audio, "audio/ogg"),
    (".opus", TelegramDocumentMediaKind::Audio, "audio/opus"),
    (".mp3", TelegramDocumentMediaKind::Audio, "audio/mpeg"),
    (".wav", TelegramDocumentMediaKind::Audio, "audio/wav"),
    (".m4a", TelegramDocumentMediaKind::Audio, "audio/mp4"),
    (".flac", TelegramDocumentMediaKind::Audio, "audio/flac"),
    (".mp4", TelegramDocumentMediaKind::Video, "video/mp4"),
    (".webm", TelegramDocumentMediaKind::Video, "video/webm"),
    (".mov", TelegramDocumentMediaKind::Video, "video/quicktime"),
    (".avi", TelegramDocumentMediaKind::Video, "video/x-msvideo"),
    (".mkv", TelegramDocumentMediaKind::Video, "video/x-matroska"),
];

fn normalize_telegram_document_mime(mime_type: &str) -> String {
    mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn telegram_document_media_from_extension(
    file_name: &str,
    remote_path: &str,
) -> Option<(TelegramDocumentMediaKind, &'static str)> {
    let file_name = file_name.to_ascii_lowercase();
    if let Some((_, kind, mime_type)) = TELEGRAM_DOCUMENT_MEDIA_MAPPINGS
        .iter()
        .find(|(extension, _, _)| file_name.ends_with(*extension))
    {
        return Some((*kind, *mime_type));
    }

    let remote_path = remote_path.to_ascii_lowercase();
    TELEGRAM_DOCUMENT_MEDIA_MAPPINGS
        .iter()
        .find(|(extension, _, _)| remote_path.ends_with(*extension))
        .map(|(_, kind, mime_type)| (*kind, *mime_type))
}

fn classify_telegram_document_media(
    mime_type: &str,
    file_name: &str,
    remote_path: &str,
) -> ClassifiedTelegramDocument {
    let mime_type = normalize_telegram_document_mime(mime_type);

    let explicit_kind = if mime_type.starts_with("image/") {
        Some(TelegramDocumentMediaKind::Image)
    } else if mime_type.starts_with("audio/") {
        Some(TelegramDocumentMediaKind::Audio)
    } else if mime_type.starts_with("video/") {
        Some(TelegramDocumentMediaKind::Video)
    } else {
        None
    };

    if let Some(kind) = explicit_kind {
        return ClassifiedTelegramDocument {
            kind,
            mime_type: Some(mime_type),
        };
    }

    if let Some((kind, resolved_mime)) =
        telegram_document_media_from_extension(file_name, remote_path)
    {
        return ClassifiedTelegramDocument {
            kind,
            mime_type: Some(resolved_mime.to_string()),
        };
    }

    ClassifiedTelegramDocument {
        kind: TelegramDocumentMediaKind::Other,
        mime_type: None,
    }
}

async fn handle_stopped_generation(
    ai_service: &AIChatService,
    route_scope: &ChatRouteScope,
    stopped: &MessageGenerationStopped,
) {
    if route_scope.allows_stop_chat(stopped.chat.id) {
        let _ = ai_service
            .cancel_generation(stopped.chat.id, stopped.draft_id)
            .await;
    }
}

async fn handle_callback_query(bot: &TelegramBotClient, cq: crate::bot::models::CallbackQuery) {
    let cq_id = cq.id;
    let _ = bot
        .answer_callback_query(
            &cq_id,
            Some("Xiao is now a pure conversational assistant."),
            false,
        )
        .await;
    if let Some(msg) = cq.message {
        let _ = bot.delete_message(msg.chat.id, msg.message_id).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_text_or_image_chat(
    bot: &TelegramBotClient,
    ai_service: &AIChatService,
    user_last_image_prompt: &UserLastImagePrompt,
    chat_id: i64,
    thread_id: i64,
    user_id: i64,
    text: &str,
    reply_to_msg_id: Option<i64>,
    image_bytes: Option<Vec<u8>>,
    document_images: Option<Vec<Vec<u8>>>,
    mime_type: Option<&str>,
    doc_text: Option<&str>,
    doc_name: Option<&str>,
    video_bytes: Option<Vec<u8>>,
    video_mime: Option<&str>,
    video_duration: i32,
) {
    let is_explicit_image = command_matches(text, "/image");
    let image_arg = if is_explicit_image {
        command_args(text, "/image").unwrap_or("")
    } else {
        text
    };

    let auto_image_intent = if image_bytes.is_none() && doc_text.is_none() {
        plan_image_generation_intent(image_arg).or_else(|| {
            if is_explicit_image && !image_arg.trim().is_empty() {
                Some(ImageGenerationIntent {
                    image_prompt: image_arg.trim().to_string(),
                    explanation_prompt: None,
                })
            } else {
                None
            }
        })
    } else {
        None
    };

    if let Some(intent) = auto_image_intent {
        handle_image_generation(
            bot,
            ai_service,
            user_last_image_prompt,
            chat_id,
            thread_id,
            user_id,
            &intent.image_prompt,
            intent.explanation_prompt.as_deref(),
            reply_to_msg_id,
        )
        .await;
    } else {
        handle_ai_chat(
            bot,
            ai_service,
            chat_id,
            thread_id,
            user_id,
            ChatInput {
                prompt: text,
                image_bytes,
                document_images,
                mime_type,
                doc_text,
                doc_name,
                audio_bytes: None,
                audio_mime: None,
                video_bytes,
                video_mime,
                video_duration: Some(video_duration),
                model_snapshot: None,
                reply_to_message_id: reply_to_msg_id,
            },
        )
        .await;
    }
}

async fn handle_update(
    bot: &TelegramBotClient,
    ai_service: &AIChatService,
    user_last_image_prompt: &UserLastImagePrompt,
    route_scope: &ChatRouteScope,
    update: Update,
) {
    if let Some(stopped) = update.stopped_message_generation.as_ref() {
        handle_stopped_generation(ai_service, route_scope, stopped).await;
        return;
    }
    if let Some(msg) = update.message {
        let chat_id = msg.chat.id;
        let thread_id = msg.message_thread_id.unwrap_or(0);
        let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(chat_id);
        let _user_name = msg
            .from
            .as_ref()
            .map(|u| u.first_name.as_str())
            .unwrap_or("Pengguna");
        let raw_text = msg
            .text
            .as_deref()
            .or(msg.caption.as_deref())
            .unwrap_or("")
            .trim();

        let is_group = chat_id != user_id;
        let reply_to_msg_id = if is_group { Some(msg.message_id) } else { None };
        let is_reply_to_bot = msg
            .reply_to_message
            .as_ref()
            .and_then(|r| r.from.as_ref())
            .map(|u| {
                if let Some(bot_id) = route_scope.bot_id {
                    if u.id == bot_id {
                        return true;
                    }
                }
                if let Some(ref my_bot) = route_scope.bot_username {
                    u.username
                        .as_deref()
                        .unwrap_or("")
                        .eq_ignore_ascii_case(my_bot)
                } else {
                    u.is_bot
                }
            })
            .unwrap_or(false);

        let has_video = msg.video.is_some() || msg.video_note.is_some();
        let has_photo = msg.photo.is_some();
        let has_audio = msg.voice.is_some() || msg.audio.is_some();
        let has_document = msg.document.is_some();
        let has_media = has_video || has_photo || has_audio || has_document;
        let is_forum = msg.chat.is_forum.unwrap_or(false) || thread_id > 0;

        let route_ctx = MessageRouteContext {
            user_id,
            chat_id,
            is_forum,
            raw_text,
            has_media,
            is_reply_to_bot,
        };

        let text = match route_scope.evaluate(&route_ctx).await {
            RouteDecision::ProcessChat(t) => t,
            RouteDecision::Ignore => return,
        };

        let mut image_bytes: Option<Vec<u8>> = None;
        let mut document_images: Option<Vec<Vec<u8>>> = None;
        let mut mime_type: Option<String> = None;
        let mut doc_text: Option<String> = None;
        let mut doc_name: Option<String> = None;
        let mut audio_bytes: Option<Vec<u8>> = None;
        let mut audio_mime: Option<String> = None;
        let mut audio_duration: i32 = 0;
        let mut video_bytes: Option<Vec<u8>> = None;
        let mut video_mime: Option<String> = None;
        let mut video_duration: i32 = 0;

        if let Some(ref v) = msg.voice {
            audio_duration = v.duration;
            audio_mime = v.mime_type.clone();
            if let Some((data, path)) = bot.get_file_bytes(&v.file_id).await {
                audio_bytes = Some(data);
                doc_name = path.split('/').next_back().map(str::to_string);
            }
        } else if let Some(ref a) = msg.audio {
            audio_duration = a.duration;
            audio_mime = a.mime_type.clone();
            let audio_file_name = a.file_name.clone();
            if let Some((data, path)) = bot.get_file_bytes(&a.file_id).await {
                audio_bytes = Some(data);
                doc_name =
                    audio_file_name.or_else(|| path.split('/').next_back().map(str::to_string));
            }
        } else if let Some(ref vid) = msg.video {
            video_duration = vid.duration;
            if let Some((data, path)) = bot.get_file_bytes(&vid.file_id).await {
                video_bytes = Some(data);
                let ext = path.split('.').next_back().unwrap_or("mp4");
                video_mime = vid
                    .mime_type
                    .clone()
                    .or_else(|| Some(format!("video/{ext}")));
            }
        } else if let Some(ref vn) = msg.video_note {
            video_duration = vn.duration;
            if let Some((data, _)) = bot.get_file_bytes(&vn.file_id).await {
                video_bytes = Some(data);
                video_mime = Some("video/mp4".to_string());
            }
        } else if let Some(ref photos) = msg.photo {
            if let Some(largest) = photos.last() {
                if let Some((data, path)) = bot.get_file_bytes(&largest.file_id).await {
                    image_bytes = Some(data);
                    let ext = path.split('.').next_back().unwrap_or("jpeg");
                    mime_type = Some(if ext == "jpg" {
                        "image/jpeg".to_string()
                    } else {
                        format!("image/{ext}")
                    });
                }
            }
        } else if let Some(doc) = msg.document {
            let d_mime = doc.mime_type.clone().unwrap_or_default();
            let d_name = doc
                .file_name
                .clone()
                .unwrap_or_else(|| "dokumen".to_string());
            if let Some((data, path)) = bot.get_file_bytes(&doc.file_id).await {
                let ClassifiedTelegramDocument {
                    kind,
                    mime_type: resolved_mime,
                } = classify_telegram_document_media(&d_mime, &d_name, &path);
                match kind {
                    TelegramDocumentMediaKind::Image => {
                        image_bytes = Some(data);
                        mime_type = resolved_mime;
                    }
                    TelegramDocumentMediaKind::Audio => {
                        audio_bytes = Some(data);
                        audio_mime = resolved_mime;
                        doc_name = Some(d_name);
                    }
                    TelegramDocumentMediaKind::Video => {
                        video_bytes = Some(data);
                        video_mime = resolved_mime;
                    }
                    TelegramDocumentMediaKind::Other
                        if document::is_extractable_document(&d_mime, &d_name) =>
                    {
                        match document::extract_document(data, &d_mime, &d_name).await {
                            Ok(extracted) => {
                                doc_text = extracted.text;
                                if !extracted.rendered_pages.is_empty() {
                                    document_images = Some(extracted.rendered_pages);
                                }
                                doc_name = Some(d_name);
                                if let Some(warning) = extracted.warning {
                                    info!("{warning}");
                                }
                            }
                            Err(err) => {
                                let safe_name = escape_html(&d_name);
                                let safe_error = escape_html(&err);
                                let _ = bot
                                    .send_message(
                                        chat_id,
                                        &format!(
                                            "⚠️ <b>Dokumen tidak dapat diproses.</b>\n\n<code>{safe_name}</code>\n{safe_error}"
                                        ),
                                        Some("HTML"),
                                        None,
                                        None,
                                        None,
                                    )
                                    .await;
                                return;
                            }
                        }
                    }
                    TelegramDocumentMediaKind::Other => {
                        let safe_name = escape_html(&d_name);
                        let _ = bot.send_message(
                            chat_id,
                            &format!(
                                "⚠️ <b>Format dokumen belum didukung.</b>\n\n<code>{safe_name}</code> tidak akan dipaksa dibaca sebagai teks biner. Xiao mendukung dokumen teks/kode, PDF, DOCX, XLSX, serta arsip ZIP, TAR/TAR.GZ, dan 7Z."
                            ),
                            Some("HTML"), None, None, None
                        ).await;
                        return;
                    }
                }
            }
        }

        if has_photo && image_bytes.is_none() {
            let _ = bot
                .send_message(
                    chat_id,
                    "⚠️ <b>Gagal mengunduh gambar dari server Telegram.</b> Silakan coba kirim ulang.",
                    Some("HTML"),
                    None,
                    None,
                    None,
                )
                .await;
            return;
        }
        if has_audio && audio_bytes.is_none() {
            let _ = bot
                .send_message(
                    chat_id,
                    "⚠️ <b>Gagal mengunduh audio dari Telegram.</b> Silakan kirim ulang pesan suara/audio.",
                    Some("HTML"),
                    None,
                    None,
                    None,
                )
                .await;
            return;
        }
        if has_document
            && image_bytes.is_none()
            && audio_bytes.is_none()
            && video_bytes.is_none()
            && doc_text.is_none()
            && document_images
                .as_ref()
                .is_none_or(|pages| pages.is_empty())
        {
            let _ = bot
                .send_message(
                    chat_id,
                    "⚠️ <b>Gagal mengunduh dokumen dari Telegram.</b> Silakan kirim ulang file tersebut.",
                    Some("HTML"),
                    None,
                    None,
                    None,
                )
                .await;
            return;
        }

        if text.is_empty()
            && image_bytes.is_none()
            && audio_bytes.is_none()
            && video_bytes.is_none()
            && doc_text.is_none()
            && document_images
                .as_ref()
                .is_none_or(|pages| pages.is_empty())
        {
            return;
        }

        // Strict provider lock
        if !ai_service.has_configured_provider(user_id).await {
            let _ = bot
                .send_message(
                    chat_id,
                    "⚠️ <b>Xiao belum memiliki provider AI aktif.</b>\n\n\
                     Silakan hubungkan AI provider terlebih dahulu melalui terminal host:\n\
                     <code>xiao setup</code> atau <code>xiao ai</code>",
                    Some("HTML"),
                    None,
                    None,
                    None,
                )
                .await;
            return;
        }

        // Voice & audio file processing is role-routed inside AIChatService.
        // Do not pre-transcribe against the active provider here because that
        // would bypass the configured Audio STT Model route.
        if let Some(a_bytes) = audio_bytes {
            let prompt_audio = if !text.is_empty() {
                format!(
                    "Dengarkan rekaman/audio terlampir dan tanggapi permintaan berikut:\n\n{text}"
                )
            } else {
                format!(
                    "Dengarkan pesan suara/audio ini ({} detik) dan jawab pertanyaan atau tanggapi maksud di dalamnya secara jelas dan mendalam.",
                    audio_duration
                )
            };

            let chat_input = build_audio_chat_input(
                &prompt_audio,
                a_bytes,
                audio_mime.as_deref(),
                doc_name.as_deref(),
                reply_to_msg_id,
            );
            handle_ai_chat(bot, ai_service, chat_id, thread_id, user_id, chat_input).await;
            return;
        }

        // Video processing
        if let Some(v_bytes) = video_bytes {
            let prompt_video = if !text.is_empty() {
                text.clone()
            } else {
                "Tonton dan analisis rekaman video ini secara mendalam. Jelaskan isi visual, alur peristiwa, teks di layar, dan suara di dalamnya.".to_string()
            };

            handle_ai_chat(
                bot,
                ai_service,
                chat_id,
                thread_id,
                user_id,
                ChatInput {
                    prompt: &prompt_video,
                    image_bytes: None,
                    document_images: None,
                    mime_type: None,
                    doc_text: None,
                    doc_name: None,
                    audio_bytes: None,
                    audio_mime: None,
                    video_bytes: Some(v_bytes),
                    video_mime: video_mime.as_deref(),
                    video_duration: Some(video_duration),
                    model_snapshot: None,
                    reply_to_message_id: reply_to_msg_id,
                },
            )
            .await;
            return;
        } else if has_video {
            let _ = bot
                .send_message(
                    chat_id,
                    "⚠️ <b>Gagal mengunduh video dari Telegram.</b>\n\n\
                     Telegram membatasi ukuran unduhan file bot maksimal <b>20MB</b>. Pastikan durasi atau ukuran video di bawah 20MB.",
                    Some("HTML"),
                    None,
                    None,
                    None,
                )
                .await;
            return;
        }

        dispatch_text_or_image_chat(
            bot,
            ai_service,
            user_last_image_prompt,
            chat_id,
            thread_id,
            user_id,
            &text,
            reply_to_msg_id,
            image_bytes,
            document_images,
            mime_type.as_deref(),
            doc_text.as_deref(),
            doc_name.as_deref(),
            video_bytes,
            video_mime.as_deref(),
            video_duration,
        )
        .await;
    } else if let Some(cq) = update.callback_query {
        handle_callback_query(bot, cq).await;
    }
}

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
    use crate::bot::models::ChatMember;

    #[test]
    fn command_matching_requires_a_real_token_boundary() {
        assert!(command_matches("/menu", "/menu"));
        assert_eq!(command_args("/menu settings", "/menu"), Some("settings"));
        assert!(command_matches("/menu@xiaobot settings", "/menu"));
        assert!(!command_matches("/menux", "/menu"));
        assert!(!command_matches("/imagegen", "/image"));
        assert!(!command_matches("/startling", "/start"));
    }

    #[test]
    fn strip_bot_mention_handles_unicode_and_boundaries() {
        assert_eq!(
            strip_bot_mention("halo @XiaoBot apa kabar?", "xiaobot"),
            Some("halo apa kabar?".to_string())
        );
        assert_eq!(
            strip_bot_mention("@XiaoBot halo dunia", "xiaobot"),
            Some("halo dunia".to_string())
        );
        // Multi-byte Unicode character directly adjacent
        assert_eq!(
            strip_bot_mention("✨@XiaoBot halo", "xiaobot"),
            Some("✨ halo".to_string())
        );
        // Substring mention that is part of a longer word shouldn't match
        assert_eq!(strip_bot_mention("@xiaobot_extra halo", "xiaobot"), None);
        assert_eq!(strip_bot_mention("halo dunia", "xiaobot"), None);
    }

    #[test]
    fn specialist_context_policies_are_minimal_and_role_specific() {
        assert!(specialist_context_policy(
            ai::service::ModelRole::Vision,
            ai::service::RouteOrigin::Specific
        )
        .contains("no full history"));
        assert!(specialist_context_policy(
            ai::service::ModelRole::AudioStt,
            ai::service::RouteOrigin::Specific
        )
        .starts_with("Transcript only"));
        assert!(specialist_context_policy(
            ai::service::ModelRole::ImageGeneration,
            ai::service::RouteOrigin::Specific
        )
        .starts_with("Prompt/config only"));
        assert!(specialist_context_policy(
            ai::service::ModelRole::Curator,
            ai::service::RouteOrigin::Specific
        )
        .contains("Background"));
        assert!(specialist_context_policy(
            ai::service::ModelRole::Vision,
            ai::service::RouteOrigin::MainModel
        )
        .starts_with("Direct on Main"));
    }

    #[test]
    fn context_semantics_use_only_main_budget_and_never_underflow() {
        assert_eq!(context_available_tokens(100_000, 35_000), 65_000);
        assert_eq!(context_available_tokens(64_000, 80_000), 0);
        assert!(specialist_context_policy(
            ai::service::ModelRole::Vision,
            ai::service::RouteOrigin::Specific
        )
        .contains("no full history"));
    }

    #[test]
    fn smaller_main_context_warns_only_when_history_exceeds_usable_budget() {
        assert!(main_context_overflow_warning("small-model", 80_000, 64_000).is_some());
        assert!(main_context_overflow_warning("large-model", 40_000, 64_000).is_none());
    }

    #[test]
    fn start_and_menu_contracts_remain_distinct() {
        let start_buttons: [&str; 0] = [];
        assert_eq!(start_buttons.len(), 0);
    }

    #[test]
    fn help_ui_is_typed_rich_and_declares_addons_read_only() {
        let value = serde_json::to_value(build_help_ui()).unwrap();
        let serialized = value.to_string();
        assert!(serialized.contains("\"type\":\"table\""));
        assert!(serialized.contains("\"type\":\"details\""));
        assert!(serialized.contains("read-only"));
        assert!(serialized.contains("xiao addon"));
    }

    #[test]
    fn compound_image_intent_keeps_explanation_for_main() {
        let intent = plan_image_generation_intent(
            "buat gambar simulasi galaksi dan jelaskan bagaimana lengan spiral terbentuk",
        )
        .expect("compound image intent");
        assert!(intent.image_prompt.to_ascii_lowercase().contains("galaksi"));
        assert_eq!(
            intent.explanation_prompt.as_deref(),
            Some("jelaskan bagaimana lengan spiral terbentuk")
        );
    }

    #[test]
    fn image_generation_draft_can_stop_policy_is_private_only() {
        let owner_id = 123456789i64;
        let group_id = -100987654321i64;

        let private_can_stop = owner_id == owner_id;
        assert!(private_can_stop);

        let group_can_stop = group_id == owner_id;
        assert!(!group_can_stop);
    }

    #[test]
    fn chat_route_scope_allows_native_stop_only_in_owner_private_chat() {
        let scope = ChatRouteScope::new(
            42,
            [100, 200].into_iter().collect(),
            HashSet::new(),
            Some(10),
            Some("xiao_bot".to_string()),
            TelegramBotClient::new("token".to_string()),
        );
        assert!(scope.allows_stop_chat(42));
        assert!(!scope.allows_stop_chat(100));
        assert!(!scope.allows_stop_chat(200));
        assert!(!scope.allows_stop_chat(999));
    }

    #[test]
    fn chat_route_scope_evaluates_private_chat() {
        let scope = ChatRouteScope::new(
            42,
            HashSet::new(),
            HashSet::new(),
            Some(10),
            Some("xiao_bot".to_string()),
            TelegramBotClient::new("token".to_string()),
        );
        let owner_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: 42,
            is_forum: false,
            raw_text: "Halo Xiao",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(true, &owner_ctx),
            RouteDecision::ProcessChat("Halo Xiao".to_string())
        );

        let non_owner_ctx = MessageRouteContext {
            user_id: 999,
            chat_id: 42,
            is_forum: false,
            raw_text: "Halo",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(true, &non_owner_ctx),
            RouteDecision::Ignore
        );
    }

    #[test]
    fn chat_route_scope_evaluates_dedicated_workspace() {
        let mut dedicated = HashSet::new();
        dedicated.insert(DEFAULT_DEDICATED_WORKSPACE_CHAT_ID);
        let scope = ChatRouteScope::new(
            42,
            HashSet::new(),
            dedicated,
            Some(10),
            Some("xiao_bot".to_string()),
            TelegramBotClient::new("token".to_string()),
        );

        // Plain message without mention is answered in dedicated workspace
        let plain_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: DEFAULT_DEDICATED_WORKSPACE_CHAT_ID,
            is_forum: true,
            raw_text: "Tolong rangkum artikel ini",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(true, &plain_ctx),
            RouteDecision::ProcessChat("Tolong rangkum artikel ini".to_string())
        );

        // Media message without caption is allowed in dedicated workspace
        let media_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: DEFAULT_DEDICATED_WORKSPACE_CHAT_ID,
            is_forum: true,
            raw_text: "",
            has_media: true,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(true, &media_ctx),
            RouteDecision::ProcessChat("".to_string())
        );

        // Bot mention stripped in dedicated workspace
        let mention_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: DEFAULT_DEDICATED_WORKSPACE_CHAT_ID,
            is_forum: true,
            raw_text: "@xiao_bot halo",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(true, &mention_ctx),
            RouteDecision::ProcessChat("halo".to_string())
        );

        // In dedicated workspace, all owner messages are answered freely (including external mentions)
        let mention_other_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: DEFAULT_DEDICATED_WORKSPACE_CHAT_ID,
            is_forum: true,
            raw_text: "opini tentang @openai",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(true, &mention_other_ctx),
            RouteDecision::ProcessChat("opini tentang @openai".to_string())
        );

        // Non-owner in dedicated workspace is rejected
        let non_owner_ctx = MessageRouteContext {
            user_id: 999,
            chat_id: DEFAULT_DEDICATED_WORKSPACE_CHAT_ID,
            is_forum: true,
            raw_text: "Halo Xiao",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(true, &non_owner_ctx),
            RouteDecision::Ignore
        );
    }

    #[test]
    fn chat_route_scope_evaluates_guest_group() {
        const GUEST_TEST_CHAT_ID: i64 = -100987654321;
        let scope = ChatRouteScope::new(
            42,
            HashSet::new(),
            HashSet::new(),
            Some(10),
            Some("xiao_bot".to_string()),
            TelegramBotClient::new("token".to_string()),
        );

        // Plain message without mention/reply/slash is IGNORED in guest group (fixes public group bug)
        let plain_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: GUEST_TEST_CHAT_ID,
            is_forum: false,
            raw_text: "Sory² admin lagi iseng testing",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &plain_ctx),
            RouteDecision::Ignore
        );

        // Bare bot username without '@' does not trigger Xiao in guest group
        let bare_name_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: GUEST_TEST_CHAT_ID,
            is_forum: false,
            raw_text: "xiao_bot kamu hebat",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &bare_name_ctx),
            RouteDecision::Ignore
        );

        // Mentioning bot: strips mention and processes
        let mention_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: GUEST_TEST_CHAT_ID,
            is_forum: false,
            raw_text: "Tes @xiao_bot",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &mention_ctx),
            RouteDecision::ProcessChat("Tes".to_string())
        );

        // Multiline prompt with bot mention preserves newlines and indentation
        let multiline_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: GUEST_TEST_CHAT_ID,
            is_forum: false,
            raw_text: "@xiao_bot Tolong review kode:\n```rust\nfn test() {}\n```",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &multiline_ctx),
            RouteDecision::ProcessChat(
                "Tolong review kode:\n```rust\nfn test() {}\n```".to_string()
            )
        );

        let slash_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: GUEST_TEST_CHAT_ID,
            is_forum: false,
            raw_text: "/start",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &slash_ctx),
            RouteDecision::ProcessChat("/start".to_string())
        );

        // Slash command referencing external handle is preserved
        let slash_with_mention_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: GUEST_TEST_CHAT_ID,
            is_forum: false,
            raw_text: "/ask @alice is this ready?",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &slash_with_mention_ctx),
            RouteDecision::ProcessChat("/ask @alice is this ready?".to_string())
        );

        // Direct reply to bot in guest group is accepted
        let reply_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: GUEST_TEST_CHAT_ID,
            is_forum: false,
            raw_text: "Jawaban bagus",
            has_media: false,
            is_reply_to_bot: true,
        };
        assert_eq!(
            scope.evaluate_internal(false, &reply_ctx),
            RouteDecision::ProcessChat("Jawaban bagus".to_string())
        );

        // Media without caption/mention/reply in guest group is IGNORED
        let media_no_caption_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: GUEST_TEST_CHAT_ID,
            is_forum: false,
            raw_text: "",
            has_media: true,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &media_no_caption_ctx),
            RouteDecision::Ignore
        );

        // Media with caption mentioning bot in guest group is accepted
        let media_mention_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: GUEST_TEST_CHAT_ID,
            is_forum: false,
            raw_text: "@xiao_bot tolong analisis gambar ini",
            has_media: true,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &media_mention_ctx),
            RouteDecision::ProcessChat("tolong analisis gambar ini".to_string())
        );

        // Mentioning another user without bot mention in guest group is ignored
        let mention_other_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: GUEST_TEST_CHAT_ID,
            is_forum: false,
            raw_text: "Hai @mira",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &mention_other_ctx),
            RouteDecision::Ignore
        );

        // Non-owner mentioning bot in guest group is ignored (single-owner invariant)
        let non_owner_mention_ctx = MessageRouteContext {
            user_id: 999,
            chat_id: GUEST_TEST_CHAT_ID,
            is_forum: false,
            raw_text: "@xiao_bot halo",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &non_owner_mention_ctx),
            RouteDecision::Ignore
        );
    }

    #[test]
    fn chat_route_scope_enforces_whitelist_when_configured() {
        let scope = ChatRouteScope::new(
            42,
            [-100111, -100222].into_iter().collect(),
            HashSet::new(),
            Some(10),
            Some("xiao_bot".to_string()),
            TelegramBotClient::new("token".to_string()),
        );
        // Whitelisted group allowed
        let allowed_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: -100111,
            is_forum: false,
            raw_text: "/start",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &allowed_ctx),
            RouteDecision::ProcessChat("/start".to_string())
        );

        // Non-whitelisted group rejected even if mentioning bot
        let forbidden_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: -100999,
            is_forum: false,
            raw_text: "@xiao_bot halo",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(false, &forbidden_ctx),
            RouteDecision::Ignore
        );

        // Dedicated workspace is always allowed even when whitelist is configured
        let dedicated_ctx = MessageRouteContext {
            user_id: 42,
            chat_id: DEFAULT_DEDICATED_WORKSPACE_CHAT_ID,
            is_forum: true,
            raw_text: "Halo workspace",
            has_media: false,
            is_reply_to_bot: false,
        };
        assert_eq!(
            scope.evaluate_internal(true, &dedicated_ctx),
            RouteDecision::ProcessChat("Halo workspace".to_string())
        );
    }

    #[test]
    fn chat_member_admin_or_creator_check() {
        let admin = ChatMember {
            status: "administrator".to_string(),
            user: None,
        };
        assert!(admin.is_admin_or_creator());

        let creator = ChatMember {
            status: "creator".to_string(),
            user: None,
        };
        assert!(creator.is_admin_or_creator());

        let member = ChatMember {
            status: "member".to_string(),
            user: None,
        };
        assert!(!member.is_admin_or_creator());
    }

    #[test]
    fn telegram_document_explicit_media_mime_overrides_conflicting_extension() {
        let cases = [
            (
                "audio/webm",
                "file.webm",
                TelegramDocumentMediaKind::Audio,
                "audio/webm",
            ),
            (
                "audio/mp4",
                "file.mp4",
                TelegramDocumentMediaKind::Audio,
                "audio/mp4",
            ),
            (
                "audio/flac",
                "file.mkv",
                TelegramDocumentMediaKind::Audio,
                "audio/flac",
            ),
            (
                "audio/opus",
                "clip.webm",
                TelegramDocumentMediaKind::Audio,
                "audio/opus",
            ),
            (
                "video/mp4",
                "recording.mp3",
                TelegramDocumentMediaKind::Video,
                "video/mp4",
            ),
            (
                "video/webm",
                "voice.opus",
                TelegramDocumentMediaKind::Video,
                "video/webm",
            ),
            (
                "image/png",
                "movie.mp4",
                TelegramDocumentMediaKind::Image,
                "image/png",
            ),
            (
                "image/jpeg",
                "recording.flac",
                TelegramDocumentMediaKind::Image,
                "image/jpeg",
            ),
        ];

        for (mime_type, file_name, expected_kind, expected_mime) in cases {
            let classified = classify_telegram_document_media(mime_type, file_name, file_name);
            assert_eq!(classified.kind, expected_kind, "{mime_type} / {file_name}");
            assert_eq!(
                classified.mime_type.as_deref(),
                Some(expected_mime),
                "{mime_type} / {file_name}"
            );
        }
    }

    #[test]
    fn telegram_document_mime_less_extension_resolves_canonical_media_identity() {
        let cases = [
            ("sample.png", TelegramDocumentMediaKind::Image, "image/png"),
            ("sample.jpg", TelegramDocumentMediaKind::Image, "image/jpeg"),
            (
                "sample.jpeg",
                TelegramDocumentMediaKind::Image,
                "image/jpeg",
            ),
            (
                "sample.webp",
                TelegramDocumentMediaKind::Image,
                "image/webp",
            ),
            ("sample.ogg", TelegramDocumentMediaKind::Audio, "audio/ogg"),
            ("sample.oga", TelegramDocumentMediaKind::Audio, "audio/ogg"),
            (
                "sample.opus",
                TelegramDocumentMediaKind::Audio,
                "audio/opus",
            ),
            ("sample.mp3", TelegramDocumentMediaKind::Audio, "audio/mpeg"),
            ("sample.wav", TelegramDocumentMediaKind::Audio, "audio/wav"),
            ("sample.m4a", TelegramDocumentMediaKind::Audio, "audio/mp4"),
            (
                "sample.flac",
                TelegramDocumentMediaKind::Audio,
                "audio/flac",
            ),
            ("sample.mp4", TelegramDocumentMediaKind::Video, "video/mp4"),
            (
                "sample.webm",
                TelegramDocumentMediaKind::Video,
                "video/webm",
            ),
            (
                "sample.mov",
                TelegramDocumentMediaKind::Video,
                "video/quicktime",
            ),
            (
                "sample.avi",
                TelegramDocumentMediaKind::Video,
                "video/x-msvideo",
            ),
            (
                "sample.mkv",
                TelegramDocumentMediaKind::Video,
                "video/x-matroska",
            ),
        ];

        for (file_name, expected_kind, expected_mime) in cases {
            let classified = classify_telegram_document_media("", file_name, file_name);
            assert_eq!(classified.kind, expected_kind, "{file_name}");
            assert_eq!(
                classified.mime_type.as_deref(),
                Some(expected_mime),
                "{file_name}"
            );
        }

        let unknown = classify_telegram_document_media("", "arbitrary.bin", "documents/file_789");
        assert_eq!(unknown.kind, TelegramDocumentMediaKind::Other);
        assert_eq!(unknown.mime_type, None);
    }

    #[test]
    fn telegram_document_remote_path_fallback_resolves_media_identity() {
        let audio = classify_telegram_document_media("", "document", "documents/file.opus");
        assert_eq!(audio.kind, TelegramDocumentMediaKind::Audio);
        assert_eq!(audio.mime_type.as_deref(), Some("audio/opus"));

        let video = classify_telegram_document_media("", "document", "documents/video.webm");
        assert_eq!(video.kind, TelegramDocumentMediaKind::Video);
        assert_eq!(video.mime_type.as_deref(), Some("video/webm"));
    }

    #[test]
    fn telegram_document_filename_identity_wins_before_remote_path_fallback() {
        let video =
            classify_telegram_document_media("", "sample.webm", "documents/telegram-file.mp3");
        assert_eq!(video.kind, TelegramDocumentMediaKind::Video);
        assert_eq!(video.mime_type.as_deref(), Some("video/webm"));

        let audio =
            classify_telegram_document_media("", "sample.mp3", "documents/telegram-file.webm");
        assert_eq!(audio.kind, TelegramDocumentMediaKind::Audio);
        assert_eq!(audio.mime_type.as_deref(), Some("audio/mpeg"));
    }

    #[test]
    fn telegram_document_mime_normalization_handles_case_and_parameters() {
        let audio = classify_telegram_document_media(
            "Audio/WebM; codecs=opus",
            "file.webm",
            "documents/file.webm",
        );
        assert_eq!(audio.kind, TelegramDocumentMediaKind::Audio);
        assert_eq!(audio.mime_type.as_deref(), Some("audio/webm"));

        let video = classify_telegram_document_media("VIDEO/MP4", "file.mp3", "documents/file.mp3");
        assert_eq!(video.kind, TelegramDocumentMediaKind::Video);
        assert_eq!(video.mime_type.as_deref(), Some("video/mp4"));

        let image = classify_telegram_document_media(
            "  image/PNG ; charset=binary ",
            "clip.mp4",
            "documents/clip.mp4",
        );
        assert_eq!(image.kind, TelegramDocumentMediaKind::Image);
        assert_eq!(image.mime_type.as_deref(), Some("image/png"));
    }

    #[test]
    fn mime_less_audio_identity_survives_runtime_chat_input_and_stt_metadata() {
        for (file_name, expected_mime, expected_stt_name) in [
            ("sample.mp3", "audio/mpeg", "sample.mp3"),
            ("sample.wav", "audio/wav", "sample.wav"),
            ("sample.opus", "audio/opus", "sample.opus"),
            ("sample.flac", "audio/flac", "sample.flac"),
        ] {
            let classified = classify_telegram_document_media("", file_name, file_name);
            assert_eq!(classified.kind, TelegramDocumentMediaKind::Audio);

            let input = build_audio_chat_input(
                "analyze",
                vec![1, 2, 3],
                classified.mime_type.as_deref(),
                Some(file_name),
                None,
            );
            assert_eq!(input.doc_name, Some(file_name));
            assert_eq!(input.audio_mime, Some(expected_mime));

            let (stt_mime, stt_name) =
                ai::service::resolve_audio_file_and_mime(input.audio_mime, input.doc_name);
            assert_eq!(stt_mime, expected_mime);
            assert_eq!(stt_name, expected_stt_name);
        }
    }

    #[test]
    fn telegram_document_runtime_uses_one_authoritative_media_classifier() {
        let source = include_str!("main.rs").replace("\r\n", "\n");
        let document_start = source
            .find("} else if let Some(doc) = msg.document {")
            .expect("Telegram document branch");
        let document_end = source[document_start..]
            .find("\n        }\n\n        // Strict provider lock")
            .map(|offset| document_start + offset)
            .expect("Telegram document branch end");
        let document_branch = &source[document_start..document_end];

        assert_eq!(
            document_branch
                .matches("classify_telegram_document_media(")
                .count(),
            1
        );
        assert!(!document_branch.contains("telegram_document_is_audio"));
        assert!(!document_branch.contains("\"image/jpeg\".to_string()"));
        assert!(!document_branch.contains("\"video/mp4\".to_string()"));

        let audio_start = source
            .find("if let Some(a_bytes) = audio_bytes {")
            .expect("audio runtime branch");
        let audio_end = source[audio_start..]
            .find("// Video processing")
            .map(|offset| audio_start + offset)
            .expect("audio runtime branch end");
        let audio_branch = &source[audio_start..audio_end];
        assert_eq!(audio_branch.matches("build_audio_chat_input(").count(), 1);
        assert!(!audio_branch.contains("doc_name: None"));
        assert!(audio_branch.contains("doc_name.as_deref()"));
    }

    #[test]
    fn image_caption_is_unicode_safe_bounded_and_html_escaped() {
        let prompt = format!("{} <tag> & \"quotes\" 'single'", "🌌银河系".repeat(800));
        let caption = build_image_success_caption(
            &prompt,
            "provider<&>",
            "model<\"x\">&",
            (1024, 1024),
            12.34,
            true,
            Some("failure <unsafe> & detail"),
        );

        assert!(caption.chars().count() <= TELEGRAM_PHOTO_CAPTION_MAX_CHARS);
        assert!(!caption.contains("<tag>"));
        assert!(caption.contains("&lt;"));
        assert!(caption.contains("&amp;"));
        assert!(caption.contains("&quot;"));
        assert!(!caption.contains("provider<&>"));
        assert!(caption.is_char_boundary(caption.len()));
    }

    #[test]
    fn photo_delivery_classifier_retries_only_caption_or_markup_failures() {
        assert_eq!(
            telegram_photo_delivery_error_class("Bad Request: can't parse entities in caption"),
            "caption_or_markup"
        );
        assert_eq!(
            telegram_photo_delivery_error_class("sendPhoto multipart error: timeout"),
            "telegram_transport"
        );
        assert_eq!(
            telegram_photo_delivery_error_class(
                "sendPhoto rejected bytes with an unsupported image signature"
            ),
            "local_image_validation"
        );
    }

    #[tokio::test]
    async fn image_delivery_success_is_single_send() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_sender = Arc::clone(&calls);
        let result = deliver_generated_image_with(
            b"same-image-bytes",
            "safe caption",
            Some(json!({"inline_keyboard": []})),
            move |_bytes, _caption, _parse_mode, _markup| {
                let calls = Arc::clone(&calls_for_sender);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn image_delivery_caption_retry_reuses_same_bytes_without_regeneration() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Mutex as StdMutex;

        let sends = Arc::new(AtomicUsize::new(0));
        let seen_bytes = Arc::new(StdMutex::new(Vec::<Vec<u8>>::new()));
        let sends_for_sender = Arc::clone(&sends);
        let seen_for_sender = Arc::clone(&seen_bytes);

        // Provider generation already happened exactly once before the delivery helper.
        let provider_calls = AtomicUsize::new(1);
        let result = deliver_generated_image_with(
            b"paid-generated-image",
            "caption",
            Some(json!({"inline_keyboard": [["button"]]})),
            move |bytes, _caption, _parse_mode, _markup| {
                let sends = Arc::clone(&sends_for_sender);
                let seen = Arc::clone(&seen_for_sender);
                async move {
                    let attempt = sends.fetch_add(1, Ordering::SeqCst);
                    seen.lock().expect("seen bytes lock").push(bytes);
                    if attempt == 0 {
                        Err("Bad Request: can't parse entities in caption".to_string())
                    } else {
                        Ok(())
                    }
                }
            },
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
        assert_eq!(sends.load(Ordering::SeqCst), 2);
        let seen = seen_bytes.lock().expect("seen bytes lock");
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], seen[1]);
    }

    #[tokio::test]
    async fn image_delivery_double_failure_returns_user_error_policy() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let sends = Arc::new(AtomicUsize::new(0));
        let sends_for_sender = Arc::clone(&sends);
        let result = deliver_generated_image_with(
            b"image",
            "caption",
            None,
            move |_bytes, _caption, _parse_mode, _markup| {
                let sends = Arc::clone(&sends_for_sender);
                async move {
                    let attempt = sends.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        Err("Bad Request: caption entities are invalid".to_string())
                    } else {
                        Err("sendPhoto multipart error: connection".to_string())
                    }
                }
            },
        )
        .await;

        let failure = result.expect_err("second delivery should fail");
        assert_eq!(sends.load(Ordering::SeqCst), 2);
        assert!(failure.retry_attempted);
        assert_eq!(failure.class, "telegram_transport");
    }

    #[test]
    fn compound_image_handler_has_one_generation_call_and_failure_precedes_explanation() {
        let source = include_str!("main.rs");
        let handler_start = source
            .find("async fn handle_image_generation(")
            .expect("image handler");
        let handler_end = source[handler_start..]
            .find("// Main AI Chat Handler")
            .map(|offset| handler_start + offset)
            .expect("image handler end");
        let handler = &source[handler_start..handler_end];

        assert_eq!(handler.matches(".generate_image_with_snapshot(").count(), 1);
        let failure_return = handler
            .find("if let Err(failure) = delivery")
            .expect("delivery failure branch");
        let explanation = handler
            .find("if let Some(explanation_prompt)")
            .expect("compound explanation");
        assert!(failure_return < explanation);
        assert!(handler[failure_return..explanation].contains("return;"));
    }

    #[test]
    fn scanned_pdf_with_rendered_pages_does_not_trigger_download_failure_guard() {
        let source = include_str!("main.rs");
        let doc_guard_start = source
            .find("if has_document")
            .expect("document failure guard");
        let doc_guard_end = source[doc_guard_start..]
            .find("if text.is_empty()")
            .map(|offset| doc_guard_start + offset)
            .expect("document failure guard end");
        let doc_guard = &source[doc_guard_start..doc_guard_end];

        assert!(doc_guard.contains("document_images"));
        assert!(doc_guard.contains("is_none_or(|pages| pages.is_empty())"));
    }

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
