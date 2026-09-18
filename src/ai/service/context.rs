use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::warn;

use crate::attachments::{decode_user_content, load_attachment};
use crate::util::truncate_chars_with_ellipsis;

use super::generation::max_output_tokens_for_model;
use super::multimodal::{historical_native_audio_input_part, media_data_url};
use super::AIChatService;
use crate::ai::capability::{model_metadata_key, ModelCapability};
use crate::ai::routing::{GenerationModelSnapshot, ModelRole, RouteOrigin};
use crate::ai::storage::load_scoped_messages_async;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMessageItem {
    pub index: usize,
    pub role: String,
    pub preview: String,
    pub chars: usize,
    pub tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextStats {
    pub session_name: String,
    pub session_id: usize,
    pub created_at: String,
    pub model_name: String,
    pub capabilities: ModelCapability,
    pub limit_tokens: usize,
    pub limit_str: String,
    pub total_messages: usize,
    pub total_turns: usize,
    pub attachment_count: usize,
    pub total_tokens: usize,
    pub output_reserve_tokens: usize,
    pub total_chars: usize,
    pub usage_pct: f64,
    pub progress_bar: String,
    pub messages_breakdown: Vec<ContextMessageItem>,
}

pub(crate) fn estimate_text_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4).max(1)
}

pub(crate) fn estimate_stored_content_tokens(content: &Value) -> usize {
    if let Some(persisted) = decode_user_content(content) {
        let media_cost = persisted.attachments.len().saturating_mul(1_500);
        return estimate_text_tokens(&persisted.text).saturating_add(media_cost);
    }
    match content {
        Value::String(text) => estimate_text_tokens(text),
        value => estimate_text_tokens(&value.to_string()),
    }
}

pub(crate) fn history_attachment_authorized(
    snapshot: &GenerationModelSnapshot,
    attachment_kind: &str,
) -> bool {
    let role = match attachment_kind {
        "image" | "document_page" => ModelRole::Vision,
        "audio" => ModelRole::AudioStt,
        "video" => ModelRole::Video,
        _ => return false,
    };
    AIChatService::resolve_model_route_from_snapshot(snapshot, role)
        .is_ok_and(|route| route.route_origin == RouteOrigin::MainModel)
}

pub(crate) fn sanitize_legacy_history(value: &Value, snapshot: &GenerationModelSnapshot) -> Value {
    use base64::Engine;

    let Some(parts) = value.as_array() else {
        return value.as_str().map_or_else(
            || Value::String("[Unrecognized historical content omitted.]".into()),
            |text| Value::String(text.to_string()),
        );
    };
    let mut budget = 12 * 1024 * 1024usize;
    let content = parts.iter().map(|part| {
        let valid = match part.get("type").and_then(Value::as_str) {
            Some("text") => part.get("text").is_some_and(Value::is_string),
            Some("image_url") => {
                part.pointer("/image_url/url").and_then(Value::as_str).is_some_and(|url| {
                    let Some((header, encoded)) = url.split_once(";base64,") else {
                        return false;
                    };
                    let Some(mime) = header.strip_prefix("data:") else {
                        return false;
                    };
                    let kind = if mime.starts_with("video/") { "video" } else { "image" };
                    if !history_attachment_authorized(snapshot, kind)
                        || encoded.len() > budget.saturating_mul(4).div_ceil(3)
                    {
                        return false;
                    }
                    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
                        return false;
                    };
                    let prefix = if kind == "video" { "video/" } else { "image/" };
                    if bytes.is_empty() || bytes.len() > budget
                        || media_data_url(&bytes, Some(mime), prefix, "historical media").is_err()
                    {
                        return false;
                    }
                    budget -= bytes.len();
                    true
                })
            }
            Some("input_audio") if history_attachment_authorized(snapshot, "audio") => {
                let format = part.pointer("/input_audio/format").and_then(Value::as_str).unwrap_or("");
                let encoded = part.pointer("/input_audio/data").and_then(Value::as_str).unwrap_or("");
                if !matches!(format, "mp3" | "wav" | "ogg" | "opus" | "mp4" | "m4a" | "flac" | "webm")
                    || encoded.len() > budget.saturating_mul(4).div_ceil(3)
                {
                    false
                } else if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                    if bytes.is_empty() || bytes.len() > budget {
                        false
                    } else {
                        budget -= bytes.len();
                        true
                    }
                } else {
                    false
                }
            }
            _ => false,
        };
        if valid {
            part.clone()
        } else {
            json!({"type": "text", "text": "[Historical media omitted: route unavailable, specialist-only, or unsafe payload.]"})
        }
    }).collect();
    Value::Array(content)
}

impl AIChatService {
    pub(crate) async fn rehydrate_history_content(
        &self,
        chat_id: i64,
        thread_id: i64,
        value: &Value,
        snapshot: &GenerationModelSnapshot,
    ) -> Value {
        let Some(persisted) = decode_user_content(value) else {
            return sanitize_legacy_history(value, snapshot);
        };

        let mut text = persisted.text;
        let mut parts = Vec::new();
        let mut total_loaded = 0usize;
        for attachment in persisted.attachments {
            let allowed = history_attachment_authorized(snapshot, attachment.kind.as_str());
            if !allowed {
                text.push_str(&format!(
                    "\n[Attachment '{}' omitted because its route is disabled, unavailable, or assigned to a specialist.]",
                    attachment.name.as_deref().unwrap_or(&attachment.kind)
                ));
                continue;
            }

            let bytes = match load_attachment(chat_id, thread_id, &attachment).await {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("Unable to reload persisted attachment: {err}");
                    text.push_str("\n[Previously attached media is no longer available.]");
                    continue;
                }
            };
            total_loaded = total_loaded.saturating_add(bytes.len());
            if total_loaded > 12 * 1024 * 1024 {
                text.push_str(
                    "\n[Older attachments omitted because the history media budget was reached.]",
                );
                break;
            }

            match attachment.kind.as_str() {
                "image" | "document_page" => {
                    match media_data_url(
                        &bytes,
                        Some(&attachment.mime_type),
                        "image/",
                        "historical image",
                    ) {
                        Ok(data_url) => parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": data_url,
                                "detail": "auto"
                            }
                        })),
                        Err(error) => text.push_str(&format!(
                            "\n[Historical image '{}' omitted: {error}.]",
                            attachment.name.as_deref().unwrap_or("image")
                        )),
                    }
                }
                "audio" => {
                    match historical_native_audio_input_part(
                        &bytes,
                        &attachment.mime_type,
                        attachment.name.as_deref(),
                    ) {
                        Ok(part) => parts.push(part),
                        Err(error) => {
                            text.push_str(&format!(
                                "\n[Historical audio '{}' omitted from native history: {error}.]",
                                attachment.name.as_deref().unwrap_or("audio")
                            ));
                        }
                    }
                }
                "video" => {
                    match media_data_url(
                        &bytes,
                        Some(&attachment.mime_type),
                        "video/",
                        "historical video",
                    ) {
                        Ok(data_url) => parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": data_url
                            }
                        })),
                        Err(error) => text.push_str(&format!(
                            "\n[Historical video '{}' omitted: {error}.]",
                            attachment.name.as_deref().unwrap_or("video")
                        )),
                    }
                }
                _ => {}
            }
        }

        if parts.is_empty() {
            Value::String(text)
        } else {
            let mut content = vec![json!({"type": "text", "text": text})];
            content.extend(parts);
            Value::Array(content)
        }
    }

    pub async fn get_scoped_context_stats(
        &self,
        chat_id: i64,
        thread_id: i64,
        user_id: i64,
    ) -> ContextStats {
        let scoped_messages = load_scoped_messages_async(chat_id, thread_id, 50).await;
        let active_model = self.get_user_model(user_id).await;
        let endpoint = self
            .get_active_provider(user_id)
            .await
            .map(|provider| provider.endpoint)
            .unwrap_or_default();
        let cap = self
            .resolved_model_capability(&endpoint, &active_model)
            .await;
        let limit_tokens = cap.context_limit;
        let limit_str = cap.context_str.clone();

        let mut total_chars = 0;
        let mut msg_stats = Vec::new();

        for (i, m) in scoped_messages.iter().enumerate() {
            let c_str = match &m.content {
                Value::String(s) => s.clone(),
                value => decode_user_content(value)
                    .map(|persisted| {
                        if persisted.attachments.is_empty() {
                            persisted.text
                        } else {
                            format!(
                                "{}\n[{} persisted attachment(s)]",
                                persisted.text,
                                persisted.attachments.len()
                            )
                        }
                    })
                    .unwrap_or_else(|| value.to_string()),
            };
            let chars = c_str.chars().count();
            let toks = estimate_text_tokens(&c_str);
            total_chars += chars;

            let preview = truncate_chars_with_ellipsis(&c_str, 90);

            msg_stats.push(ContextMessageItem {
                index: i + 1,
                role: m.role.clone(),
                preview,
                chars,
                tokens: toks,
            });
        }

        let attachment_count = scoped_messages
            .iter()
            .filter_map(|message| decode_user_content(&message.content))
            .map(|persisted| persisted.attachments.len())
            .sum();
        let total_tokens = scoped_messages
            .iter()
            .map(|message| estimate_stored_content_tokens(&message.content))
            .sum();
        let mut output_reserve_tokens =
            max_output_tokens_for_model(&active_model).min(limit_tokens.saturating_div(2).max(1));
        if let Some(metadata_limit) = self
            .model_metadata
            .read()
            .await
            .get(&model_metadata_key(&endpoint, &active_model))
            .and_then(|metadata| metadata.max_completion_tokens)
            .filter(|limit| *limit > 0)
        {
            output_reserve_tokens = output_reserve_tokens.min(metadata_limit);
        }
        let usage_pct = ((total_tokens as f64 / limit_tokens.max(1) as f64) * 100.0).min(100.0);

        let mut filled_blocks = (usage_pct / 10.0).floor() as usize;
        if usage_pct > 0.0 && filled_blocks == 0 {
            filled_blocks = 1;
        }
        filled_blocks = filled_blocks.min(10);
        let bar = format!(
            "{}{}",
            "█".repeat(filled_blocks),
            "░".repeat(10 - filled_blocks)
        );

        let session_name = if chat_id == user_id && thread_id == 0 {
            "Main Conversation".to_string()
        } else if thread_id != 0 {
            format!("Topic #{thread_id}")
        } else {
            format!("Chat #{chat_id}")
        };

        ContextStats {
            session_name,
            session_id: 0,
            created_at: "Eternal".to_string(),
            model_name: active_model,
            capabilities: cap,
            limit_tokens,
            limit_str,
            total_messages: scoped_messages.len(),
            total_turns: scoped_messages.len().div_ceil(2),
            attachment_count,
            total_tokens,
            output_reserve_tokens,
            total_chars,
            usage_pct,
            progress_bar: bar,
            messages_breakdown: msg_stats,
        }
    }
}
