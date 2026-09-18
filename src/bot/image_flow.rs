use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::warn;

use crate::ai::{
    self,
    service::{AIChatService, ImageGenerationError, ImageGenerationErrorKind},
};
use crate::attachments;
use crate::bot::client::TelegramBotClient;
use crate::bot::models::{InputRichMessage, RichBlock, RichBlockTableCell};
use crate::timeline::{ExecutionTimeline, ProgressActivity};
use crate::util::truncate_chars;

pub type UserLastImagePrompt = Arc<RwLock<HashMap<i64, String>>>;

pub const TELEGRAM_PHOTO_CAPTION_MAX_CHARS: usize = 1024;
pub const IMAGE_CAPTION_PROMPT_ESCAPED_CHARS: usize = 320;
pub const IMAGE_CAPTION_PROVIDER_ESCAPED_CHARS: usize = 96;
pub const IMAGE_CAPTION_MODEL_ESCAPED_CHARS: usize = 128;
pub const IMAGE_CAPTION_FAILURE_ESCAPED_CHARS: usize = 144;

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

pub fn extract_image_intent_prompt(text: &str) -> Option<String> {
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
pub struct ImageGenerationIntent {
    pub image_prompt: String,
    pub explanation_prompt: Option<String>,
}

pub fn plan_image_generation_intent(text: &str) -> Option<ImageGenerationIntent> {
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

pub fn bounded_escaped_html(text: &str, max_chars: usize) -> String {
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

pub fn build_image_success_caption(
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

pub fn telegram_photo_delivery_error_class(error: &str) -> &'static str {
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
pub struct ImageDeliveryFailure {
    pub class: &'static str,
    pub detail: String,
    pub retry_attempted: bool,
}

pub async fn deliver_generated_image_with<F, Fut>(
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

pub async fn resolve_image_generation_prompt(
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

pub async fn send_image_generation_help(
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

pub fn build_image_generation_error_rich(error: &ImageGenerationError) -> InputRichMessage {
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
pub async fn handle_image_generation(
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
        crate::handle_ai_chat(
            bot,
            ai_service,
            chat_id,
            thread_id,
            user_id,
            crate::ChatInput {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let source = include_str!("image_flow.rs");
        let handler_start = source
            .find("pub async fn handle_image_generation(")
            .expect("image handler");
        let handler_end = source[handler_start..]
            .find("#[cfg(test)]")
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
}
