use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::attachments::{encode_user_content, persist_attachment, AttachmentRef};
use crate::timeline::{GenerationProgressSink, ProgressActivity};
use crate::util::truncate_chars;

use super::context::{estimate_stored_content_tokens, estimate_text_tokens};
use super::multimodal::{
    media_data_url, native_audio_input_part, resolved_audio_persistence_mime,
    resolved_runtime_media_mime, select_audio_execution_mode, AudioExecutionMode,
    SpecialistObservationInput,
};
use super::{provider_url, AIChatService, ActiveGenerations};
use crate::ai::capability::model_metadata_key;
use crate::ai::http::{is_retryable_status, retry_delay, MAX_PROVIDER_ATTEMPTS};
use crate::ai::routing::{GenerationModelSnapshot, ModelRole, ResolvedModelRoute, RouteOrigin};
use crate::ai::storage::{
    count_scoped_messages_async, get_scoped_summary_async, get_user_memories_async,
    load_scoped_messages_async, save_scoped_message_async, save_scoped_summary_async,
    save_user_memory_async, CapabilityKind, CapabilityState, ProviderConfig,
};
use crate::ai::stream::{SseDecoder, StreamEvent};

pub(crate) const MAX_STREAM_VISIBLE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_STREAM_REASONING_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_STREAM_WIRE_BYTES: usize = 32 * 1024 * 1024;

pub(crate) use crate::bot::models::StagedDocument;

/// (thinking_text, answer_text, staged_documents, cancelled)
pub(crate) type ChatGenerationResult = (Option<String>, String, Vec<StagedDocument>, bool);

static NEXT_DRAFT_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(100_000);

pub fn next_draft_id() -> i64 {
    NEXT_DRAFT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub(crate) fn push_bounded(target: &mut String, chunk: &str, max_bytes: usize) -> bool {
    if target.len().saturating_add(chunk.len()) > max_bytes {
        return false;
    }
    target.push_str(chunk);
    true
}

pub(crate) fn canonical_persisted_prompt<'a>(
    canonical: Option<&'a str>,
    runtime_prompt: &'a str,
) -> &'a str {
    canonical.unwrap_or(runtime_prompt)
}

pub(crate) fn max_output_tokens_for_model(model: &str) -> usize {
    let lower = model.to_ascii_lowercase();
    if lower.contains("claude") {
        64_000
    } else if lower.contains("gemini")
        || ["o1", "o3", "gpt-4o", "gpt-5", "sol", "terra", "luna"]
            .iter()
            .any(|needle| lower.contains(needle))
    {
        65_536
    } else {
        16_384
    }
}

pub(crate) fn cancelled_chat_result(
    sink: Option<&dyn GenerationProgressSink>,
) -> ChatGenerationResult {
    if let Some(sink) = sink {
        sink.on_failure("Stopped by user", false);
    }
    (
        None,
        "⏹️ Generasi dihentikan oleh pengguna.".to_string(),
        Vec::new(),
        true,
    )
}

#[derive(Default, Clone)]
pub struct PendingToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

fn extract_leaked_tool_calls(raw: &str) -> Vec<PendingToolCall> {
    static RE_TOOL_CALL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r#"(?is)<(?:tool_call|function_call)\b[^>]*>(.*?)</(?:tool_call|function_call)>"#,
        )
        .expect("valid static regex")
    });

    let mut result = Vec::new();
    for cap in RE_TOOL_CALL.captures_iter(raw) {
        if let Some(inner) = cap.get(1) {
            let s = inner.as_str().trim();
            if let Ok(v) = serde_json::from_str::<Value>(s) {
                if let Some(name) = v.get("name").and_then(Value::as_str) {
                    let args = if let Some(args_str) = v.get("arguments").and_then(Value::as_str) {
                        args_str.to_string()
                    } else if let Some(args_val) = v.get("arguments") {
                        args_val.to_string()
                    } else {
                        String::new()
                    };
                    let id = v
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("call_leaked")
                        .to_string();
                    result.push(PendingToolCall {
                        id,
                        name: name.to_string(),
                        arguments: args,
                    });
                }
            } else if let Ok(arr) = serde_json::from_str::<Vec<Value>>(s) {
                for v in arr {
                    if let Some(name) = v.get("name").and_then(Value::as_str) {
                        let args =
                            if let Some(args_str) = v.get("arguments").and_then(Value::as_str) {
                                args_str.to_string()
                            } else if let Some(args_val) = v.get("arguments") {
                                args_val.to_string()
                            } else {
                                String::new()
                            };
                        let id = v
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("call_leaked")
                            .to_string();
                        result.push(PendingToolCall {
                            id,
                            name: name.to_string(),
                            arguments: args,
                        });
                    }
                }
            }
        }
    }
    result
}

pub struct GenerationGuard {
    active_generations: ActiveGenerations,
    chat_id: i64,
    draft_id: i64,
}

impl GenerationGuard {
    pub fn new(active_generations: ActiveGenerations, chat_id: i64, draft_id: i64) -> Self {
        Self {
            active_generations,
            chat_id,
            draft_id,
        }
    }
}

impl Drop for GenerationGuard {
    fn drop(&mut self) {
        let key = (self.chat_id, self.draft_id);
        if let Ok(mut map) = self.active_generations.try_write() {
            map.remove(&key);
        } else if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let active = self.active_generations.clone();
            handle.spawn(async move {
                active.write().await.remove(&key);
            });
        }
    }
}

pub struct GenerationInput<'a> {
    pub prompt: &'a str,
    pub canonical_prompt: Option<&'a str>,
    pub media_to_main: bool,
    pub sink: Option<&'a (dyn GenerationProgressSink + 'a)>,
    pub image_bytes: Option<Vec<u8>>,
    pub document_images: Option<Vec<Vec<u8>>>,
    pub mime_type: Option<&'a str>,
    pub doc_text: Option<&'a str>,
    pub doc_name: Option<&'a str>,
    pub audio_bytes: Option<Vec<u8>>,
    pub audio_mime: Option<&'a str>,
    pub video_bytes: Option<Vec<u8>>,
    pub video_mime: Option<&'a str>,
    pub video_duration: Option<i32>,
    pub bot: Option<crate::bot::client::TelegramBotClient>,
    pub reply_to_message_id: Option<i64>,
}

impl AIChatService {
    pub async fn generate_response(
        &self,
        chat_id: i64,
        thread_id: i64,
        user_id: i64,
        input: GenerationInput<'_>,
        cancel_rx: &mut watch::Receiver<bool>,
    ) -> ChatGenerationResult {
        let snapshot = self.generation_model_snapshot().await;
        self.generate_response_with_snapshot(
            chat_id, thread_id, user_id, input, &snapshot, cancel_rx,
        )
        .await
    }

    pub(crate) async fn generate_response_with_snapshot(
        &self,
        chat_id: i64,
        thread_id: i64,
        user_id: i64,
        input: GenerationInput<'_>,
        snapshot: &GenerationModelSnapshot,
        cancel_rx: &mut watch::Receiver<bool>,
    ) -> ChatGenerationResult {
        if thread_id > 0
            && crate::bot::client::TelegramBotClient::current_delivery_context()
                .message_thread_id
                .is_none()
        {
            let mut ctx = crate::bot::client::TelegramBotClient::current_delivery_context();
            ctx.message_thread_id = Some(thread_id);
            crate::bot::client::TelegramBotClient::with_delivery_context(
                ctx,
                self.generate_response_with_snapshot_inner(
                    chat_id, thread_id, user_id, input, snapshot, cancel_rx,
                ),
            )
            .await
        } else {
            self.generate_response_with_snapshot_inner(
                chat_id, thread_id, user_id, input, snapshot, cancel_rx,
            )
            .await
        }
    }

    async fn generate_response_with_snapshot_inner(
        &self,
        chat_id: i64,
        thread_id: i64,
        user_id: i64,
        input: GenerationInput<'_>,
        snapshot: &GenerationModelSnapshot,
        cancel_rx: &mut watch::Receiver<bool>,
    ) -> ChatGenerationResult {
        let GenerationInput {
            prompt,
            canonical_prompt: _,
            media_to_main: _,
            sink,
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
            bot,
            reply_to_message_id,
        } = input;

        if *cancel_rx.borrow() {
            return cancelled_chat_result(sink);
        }

        let main = match Self::resolve_model_route_from_snapshot(snapshot, ModelRole::Main) {
            Ok(route) => route,
            Err(error) => {
                return (
                    None,
                    format!("Main Model is unavailable: {error}"),
                    Vec::new(),
                    false,
                )
            }
        };

        let has_vision = image_bytes.is_some()
            || document_images
                .as_ref()
                .is_some_and(|pages| !pages.is_empty());
        let role = if has_vision {
            Some(ModelRole::Vision)
        } else if video_bytes.is_some() {
            Some(ModelRole::Video)
        } else if audio_bytes.is_some() {
            Some(ModelRole::AudioStt)
        } else {
            None
        };

        let Some(role) = role else {
            return self
                .generate_response_on_main(
                    chat_id,
                    thread_id,
                    user_id,
                    &main,
                    snapshot,
                    GenerationInput {
                        prompt,
                        canonical_prompt: None,
                        media_to_main: true,
                        sink,
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
                        bot,
                        reply_to_message_id,
                    },
                    cancel_rx,
                )
                .await;
        };

        let specialist = match Self::resolve_model_route_from_snapshot(snapshot, role) {
            Ok(route) => route,
            Err(error) => {
                return (
                    None,
                    format!("{} unavailable: {error}", role.display_name()),
                    Vec::new(),
                    false,
                )
            }
        };

        let same_as_main =
            specialist.provider.id == main.provider.id && specialist.model == main.model;

        if role == ModelRole::AudioStt {
            let inherited_main = same_as_main && specialist.route_origin == RouteOrigin::MainModel;
            let audio_mode = match select_audio_execution_mode(
                &specialist.capability,
                inherited_main,
                audio_mime,
                doc_name,
            ) {
                Ok(mode) => mode,
                Err(error) => return (None, error, Vec::new(), false),
            };

            if audio_mode == AudioExecutionMode::Native {
                return self
                    .generate_response_on_main(
                        chat_id,
                        thread_id,
                        user_id,
                        &main,
                        snapshot,
                        GenerationInput {
                            prompt,
                            canonical_prompt: None,
                            media_to_main: true,
                            sink,
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
                            bot,
                            reply_to_message_id,
                        },
                        cancel_rx,
                    )
                    .await;
            }

            let Some(bytes) = audio_bytes.clone() else {
                return (
                    None,
                    "Audio input is missing.".to_string(),
                    Vec::new(),
                    false,
                );
            };
            let transcript_result = tokio::select! {
                changed = cancel_rx.changed() => {
                    if changed.is_ok() && *cancel_rx.borrow() {
                        return cancelled_chat_result(sink);
                    }
                    Err("Kanal pembatalan audio ditutup.".to_string())
                }
                result = self.transcribe_audio_resolved(
                    &specialist,
                    bytes,
                    doc_name.unwrap_or("audio"),
                    audio_mime,
                ) => result
            };
            let transcript = match transcript_result {
                Ok(transcript) => transcript,
                Err(error) => return (None, error, Vec::new(), false),
            };
            let synthesis_prompt = if prompt.trim().is_empty() {
                format!("Transcript from Audio STT specialist:\n\n{transcript}\n\nRespond to the user based on this transcript.")
            } else {
                format!(
                    "User request:\n{prompt}\n\nTranscript from Audio STT specialist:\n{transcript}\n\nAnswer the user request using the transcript as an execution artifact."
                )
            };
            return self
                .generate_response_on_main(
                    chat_id,
                    thread_id,
                    user_id,
                    &main,
                    snapshot,
                    GenerationInput {
                        prompt: &synthesis_prompt,
                        canonical_prompt: Some(prompt),
                        media_to_main: false,
                        sink,
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
                        bot,
                        reply_to_message_id,
                    },
                    cancel_rx,
                )
                .await;
        }

        if same_as_main && specialist.route_origin == RouteOrigin::MainModel {
            return self
                .generate_response_on_main(
                    chat_id,
                    thread_id,
                    user_id,
                    &main,
                    snapshot,
                    GenerationInput {
                        prompt,
                        canonical_prompt: None,
                        media_to_main: true,
                        sink,
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
                        bot,
                        reply_to_message_id,
                    },
                    cancel_rx,
                )
                .await;
        }

        let observation_result = tokio::select! {
            changed = cancel_rx.changed() => {
                if changed.is_ok() && *cancel_rx.borrow() {
                    return cancelled_chat_result(sink);
                }
                Err("Kanal pembatalan media ditutup.".to_string())
            }
            result = self.run_specialist_observation(
                &specialist,
                SpecialistObservationInput {
                    prompt,
                    image_bytes: image_bytes.as_deref(),
                    document_images: document_images.as_deref(),
                    mime_type,
                    video_bytes: video_bytes.as_deref(),
                    video_mime,
                },
            ) => result
        };
        let observation = match observation_result {
            Ok(observation) => observation,
            Err(error) => return (None, error, Vec::new(), false),
        };
        let synthesis_prompt = format!(
            "User request:\n{}\n\nBounded {} observation from {} / {}:\n{}\n\nUse the observation as an execution artifact. Do not claim access to media beyond it.",
            if prompt.trim().is_empty() { "Analyze the supplied media." } else { prompt },
            role.display_name(),
            specialist.provider.name,
            specialist.model,
            observation
        );
        self.generate_response_on_main(
            chat_id,
            thread_id,
            user_id,
            &main,
            snapshot,
            GenerationInput {
                prompt: &synthesis_prompt,
                canonical_prompt: Some(prompt),
                media_to_main: false,
                sink,
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
                bot,
                reply_to_message_id,
            },
            cancel_rx,
        )
        .await
    }
}

// Multimodal attachments are stored outside SQLite and referenced from
// the user message. If the append fails, only newly created references
// are cleaned up; pre-existing media stays intact.
#[allow(clippy::too_many_arguments)]
async fn persist_runtime_attachments(
    chat_id: i64,
    thread_id: i64,
    document_images: Option<&[Vec<u8>]>,
    image_bytes: Option<&[u8]>,
    mime_type: Option<&str>,
    audio_bytes: Option<&[u8]>,
    audio_mime: Option<&str>,
    doc_name: Option<&str>,
    video_bytes: Option<&[u8]>,
    video_mime: Option<&str>,
) -> Vec<AttachmentRef> {
    let mut attachment_refs = Vec::new();
    if let Some(pages) = document_images {
        for (index, page) in pages.iter().enumerate() {
            let page_name = format!(
                "{} page {}",
                doc_name.unwrap_or("PDF scan"),
                index.saturating_add(1)
            );
            match persist_attachment(
                chat_id,
                thread_id,
                "document_page",
                "image/png",
                Some(&page_name),
                page,
            )
            .await
            {
                Ok(reference) => attachment_refs.push(reference),
                Err(err) => warn!("Failed to persist rendered PDF page: {err}"),
            }
        }
    } else if let Some(bytes) = image_bytes {
        match resolved_runtime_media_mime(mime_type, "image/", "persisted image") {
            Ok(resolved_mime) => {
                match persist_attachment(chat_id, thread_id, "image", &resolved_mime, None, bytes)
                    .await
                {
                    Ok(reference) => attachment_refs.push(reference),
                    Err(err) => warn!("Failed to persist image attachment: {err}"),
                }
            }
            Err(err) => warn!("Refusing to persist image with false media identity: {err}"),
        }
    } else if let Some(bytes) = audio_bytes {
        let resolved_mime = resolved_audio_persistence_mime(audio_mime, doc_name);
        match persist_attachment(chat_id, thread_id, "audio", &resolved_mime, doc_name, bytes).await
        {
            Ok(reference) => attachment_refs.push(reference),
            Err(err) => warn!("Failed to persist audio attachment: {err}"),
        }
    } else if let Some(bytes) = video_bytes {
        match resolved_runtime_media_mime(video_mime, "video/", "persisted video") {
            Ok(resolved_mime) => {
                match persist_attachment(chat_id, thread_id, "video", &resolved_mime, None, bytes)
                    .await
                {
                    Ok(reference) => attachment_refs.push(reference),
                    Err(err) => warn!("Failed to persist video attachment: {err}"),
                }
            }
            Err(err) => warn!("Refusing to persist video with false media identity: {err}"),
        }
    }
    attachment_refs
}

impl AIChatService {
    #[allow(clippy::too_many_arguments)]
    async fn generate_response_on_main(
        &self,
        chat_id: i64,
        thread_id: i64,
        user_id: i64,
        main_route: &ResolvedModelRoute,
        snapshot: &GenerationModelSnapshot,
        input: GenerationInput<'_>,
        cancel_rx: &mut watch::Receiver<bool>,
    ) -> ChatGenerationResult {
        let GenerationInput {
            prompt,
            canonical_prompt,
            media_to_main,
            sink,
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
            bot,
            reply_to_message_id,
        } = input;

        let provider = &main_route.provider;
        let mut model = main_route.model.as_str();

        if crate::ai::service::is_dedicated_image_generation_model(model) {
            let fallback_text_model = provider
                .models
                .iter()
                .find(|m| !crate::ai::service::is_dedicated_image_generation_model(m));
            if let Some(fallback) = fallback_text_model {
                warn!(
                    chat_id,
                    configured_model = %model,
                    fallback_model = %fallback,
                    "Main model is a dedicated image generation model; falling back to conversational text model"
                );
                model = fallback.as_str();
            } else {
                return (
                    None,
                    format!(
                        "⚠️ Model `{model}` adalah model khusus pembuatan gambar (Image Generation), bukan model percakapan teks.\n\nSilakan gunakan perintah `/image [deskripsi]` untuk membuat gambar, atau alihkan model utama ke model percakapan teks (seperti `gemini-3.8-flash-high`) melalui menu `/model`."
                    ),
                    Vec::new(),
                    false,
                );
            }
        }

        let mut clean_prompt = prompt.trim().to_string();
        if let Some(doc) = doc_text {
            let d_name = doc_name.unwrap_or("Dokumen");
            let doc_header = format!("[Dokumen Terlampir: {d_name}]\n{}\n\n", doc.trim());
            clean_prompt = if clean_prompt.is_empty() {
                format!("{doc_header}Baca, analisis, dan jelaskan isi dokumen ini.")
            } else {
                format!("{doc_header}{clean_prompt}")
            };
        } else if document_images
            .as_ref()
            .is_some_and(|pages| !pages.is_empty())
            && clean_prompt.is_empty()
        {
            let d_name = doc_name.unwrap_or("PDF scan");
            clean_prompt = format!(
                "Baca dan analisis halaman hasil render dari dokumen '{d_name}'. Lakukan OCR visual pada teks yang terlihat dan jelaskan isi dokumen secara akurat."
            );
        } else if video_bytes.is_some() && clean_prompt.is_empty() {
            let dur_str = video_duration
                .map(|d| format!(" ({d} detik)"))
                .unwrap_or_default();
            clean_prompt = format!("Tonton dan analisis rekaman video ini{dur_str} secara mendalam. Jelaskan isi visual, alur peristiwa, teks di layar, dan suara di dalamnya.");
        } else if image_bytes.is_some() && clean_prompt.is_empty() {
            clean_prompt = "Jelaskan dan analisis gambar ini secara detail.".to_string();
        } else if audio_bytes.is_some() && clean_prompt.is_empty() {
            clean_prompt = "Dengarkan rekaman suara ini dan jawab pertanyaan atau instruksi di dalamnya secara lengkap.".to_string();
        }
        let canonical_history_prompt = canonical_prompt.map(str::to_string);

        let resolved_capability = self
            .resolved_model_capability(&provider.endpoint, model)
            .await;
        let metadata_max_completion_tokens = self
            .model_metadata
            .read()
            .await
            .get(&model_metadata_key(&provider.endpoint, model))
            .and_then(|metadata| metadata.max_completion_tokens);
        let mut max_output_tokens = max_output_tokens_for_model(model)
            .min(resolved_capability.context_limit.saturating_div(2).max(1));
        if let Some(limit) = metadata_max_completion_tokens.filter(|limit| *limit > 0) {
            max_output_tokens = max_output_tokens.min(limit);
        }
        let max_prompt_tokens = resolved_capability
            .context_limit
            .saturating_sub(max_output_tokens)
            .saturating_sub(2_048)
            .max(1);
        if estimate_text_tokens(&clean_prompt) > max_prompt_tokens {
            let max_chars = max_prompt_tokens.saturating_mul(4);
            clean_prompt = truncate_chars(&clean_prompt, max_chars);
            clean_prompt.push_str("\n\n[Input dipotong Xiao agar muat di context window model.]");
        }
        let enhanced_prompt = clean_prompt.clone();

        let reserved_tokens = max_output_tokens
            .saturating_add(estimate_text_tokens(&enhanced_prompt))
            .saturating_add(2_048);
        let history_budget = resolved_capability
            .context_limit
            .saturating_sub(reserved_tokens);

        let scoped_messages = load_scoped_messages_async(chat_id, thread_id, 20).await;
        let mut selected_history = Vec::new();
        let mut used_history_tokens = 0usize;
        for message in scoped_messages.iter().rev() {
            let estimated = estimate_stored_content_tokens(&message.content).saturating_add(8);
            if !selected_history.is_empty()
                && used_history_tokens.saturating_add(estimated) > history_budget
            {
                break;
            }
            if estimated > history_budget && selected_history.is_empty() {
                continue;
            }
            used_history_tokens = used_history_tokens.saturating_add(estimated);
            selected_history.push(message.clone());
        }
        selected_history.reverse();

        let mut history = Vec::with_capacity(selected_history.len());
        for message in &selected_history {
            let content = if message.role == "user" {
                self.rehydrate_history_content(chat_id, thread_id, &message.content, snapshot)
                    .await
            } else {
                message.content.clone()
            };
            history.push(json!({ "role": message.role, "content": content }));
        }

        let user_memories = get_user_memories_async(user_id).await;
        let mut system_text = "Kamu adalah Xiao, asisten AI yang cerdas, komunikatif, dan ramah. \
                    Gunakan input multimodal hanya ketika input tersebut benar-benar disediakan dan endpoint/model mendukungnya. \
                    Dokumen Xiao diekstrak menjadi teks bila memungkinkan; PDF scan dapat diberikan sebagai halaman hasil render untuk OCR visual. \
                    Lakukan penalaran secara internal dan berikan hanya jawaban yang berguna bagi pengguna; jangan menampilkan chain-of-thought tersembunyi. \
                    Gunakan gaya bahasa yang alami dan format teks yang elegan. \
                    Jika membuat tabel atau data berkolom, gunakan Markdown Table standar agar Xiao dapat merendernya secara rapi. \
                    Jika pengguna meminta atau membutuhkan konten visual, foto, gambar, logo, lambang/ikon, album kolase, tayangan slide, berkas audio/musik, rekaman suara, lokasi peta, dokumen berkas, pembuatan file/arsip langsung, atau kuis interaktif, SELALU panggil tool resmi yang sesuai (`send_photo`, `send_collage`, `send_slideshow`, `send_audio`, `send_voice`, `send_location`, `send_document`, `create_document`, `create_quiz`). Jika Anda membutuhkan URL gambar untuk memanggil tool foto/kolase, gunakan tool `web_search` terlebih dahulu untuk memperoleh URL gambar raster terverifikasi (.jpg, .png, .webp). \
                    Khusus untuk pembuatan berkas/dokumen (termasuk dokumen PDF `.pdf`, berkas HTML `.html`, script kode, data CSV/JSON, atau teks): Anda MEMILIKI kemampuan membuat dan mengirimkannya secara langsung via tool `create_document`. JANGAN PERNAH menolak permintaan pembuatan PDF atau dokumen dengan alasan teknis seperti tidak memiliki Puppeteer, wkhtmltopdf, WeasyPrint, atau browser engine. SELALU panggil tool `create_document` dengan nama berkas yang diminta (contoh: `laporan.pdf`, `profil.html`, dsb) dan isi konten dokumen tersebut. \
                    Ketika Anda memanggil tool multimedia atau pembuatan dokumen, tool akan menyiapkan media dan mengembalikan tag media yang siap disematkan. WAJIB sematkan tag media tersebut langsung di tengah-tengah penjelasan teks pada posisi yang paling relevan (misalnya di bawah heading pembuka atau di antara paragraf narasi) agar tampil elegan di dalam gelembung pesan utama Xiao. Jangan mengarang URL atau tag media fiktif tanpa memanggil tool terlebih dahulu. \
                    Untuk tautan video streaming eksternal (seperti YouTube, Vimeo, Twitch), sertakan tautan teks Markdown standar [Judul Video](https://...) agar Telegram otomatis memunculkan rich link preview interaktif. \
                    Jika pengguna meminta kuis interaktif, latihan soal, atau tebak-tebakan, selalu panggil tool `create_quiz` (gunakan parameter `preamble` terformat Markdown jika ada materi pengantar, studi kasus, atau potongan kode sebelum kuis).\n\
                    Jangan pernah menampilkan tag internal seperti <think>, <thought>, <tool_call>, atau blok JSON raw ke pengguna.\n\
                    Jika pengguna mengirim '/start' atau salam pembuka di awal sesi baru, sambut mereka dengan hangat, ramah, dan ringkas sebagai asisten AI Xiao tanpa menyebut-nyebut perintah slash. \
                    Jika pengguna mengirim '/start' ketika percakapan sudah berjalan, berikan rangkuman ringkas mengenai hal-hal yang telah dibahas sebelumnya dan tanyakan kelanjutannya secara natural.".to_string();

        if !user_memories.is_empty() {
            system_text.push_str("\n\n[User Profile (Long-Term Memory)]:\n");
            for (key, fact) in &user_memories {
                system_text.push_str(&format!("- {key}: {fact}\n"));
            }
        }

        if let Some(summary) = get_scoped_summary_async(chat_id, thread_id).await {
            system_text.push_str(&format!(
                "\n\n[Previous Conversation Summary for this Topic]:\n{summary}\n"
            ));
        }

        let mut messages = vec![json!({
            "role": "system",
            "content": system_text
        })];

        messages.extend(history);

        if media_to_main {
            if let Some(pages) = document_images.as_ref().filter(|pages| !pages.is_empty()) {
                use base64::Engine;
                let mut content = vec![json!({ "type": "text", "text": enhanced_prompt })];
                for page in pages {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(page);
                    content.push(json!({
                        "type": "image_url",
                        "image_url": {
                            "url": format!("data:image/png;base64,{encoded}"),
                            "detail": "high"
                        }
                    }));
                }
                messages.push(json!({ "role": "user", "content": content }));
            } else if let Some(v_bytes) = video_bytes.as_ref() {
                let data_url = match media_data_url(v_bytes, video_mime, "video/", "video") {
                    Ok(data_url) => data_url,
                    Err(error) => return (None, error, Vec::new(), false),
                };
                messages.push(json!({
                    "role": "user",
                    "content": [
                        { "type": "text", "text": enhanced_prompt },
                        { "type": "image_url", "image_url": { "url": data_url } }
                    ]
                }));
            } else if let Some(i_bytes) = image_bytes.as_ref() {
                let data_url = match media_data_url(i_bytes, mime_type, "image/", "image") {
                    Ok(data_url) => data_url,
                    Err(error) => return (None, error, Vec::new(), false),
                };
                messages.push(json!({
                    "role": "user",
                    "content": [
                        { "type": "text", "text": enhanced_prompt },
                        { "type": "image_url", "image_url": { "url": data_url, "detail": "auto" } }
                    ]
                }));
            } else if let Some(a_bytes) = audio_bytes.as_ref() {
                let audio_part = match native_audio_input_part(a_bytes, audio_mime, doc_name) {
                    Ok(part) => part,
                    Err(error) => {
                        return (
                            None,
                            format!(
                                "Native audio payload was blocked because its format cannot be represented safely: {error}."
                            ),
                        Vec::new(),
                        false,
                        )
                    }
                };
                messages.push(json!({
                    "role": "user",
                    "content": [
                        { "type": "text", "text": enhanced_prompt },
                        audio_part
                    ]
                }));
            } else {
                messages.push(json!({
                    "role": "user",
                    "content": enhanced_prompt
                }));
            }
        } else {
            messages.push(json!({
                "role": "user",
                "content": enhanced_prompt
            }));
        }

        let cap_record = self.capability_record(&provider.endpoint, model).await;
        let supports_tools = cap_record
            .as_ref()
            .map(|r| r.effective_state_for(CapabilityKind::Tools) != CapabilityState::Unsupported)
            .unwrap_or(true);

        let url = provider_url(&provider.endpoint, "chat/completions");
        let mut payload = json!({
            "model": model,
            "messages": messages,
            "stream": true,
        });
        if metadata_max_completion_tokens.is_some() {
            payload["max_completion_tokens"] = json!(max_output_tokens);
        } else {
            payload["max_tokens"] = json!(max_output_tokens);
        }

        let use_auth = !provider.api_key.is_empty()
            && !["none", "-", "no"]
                .iter()
                .any(|k| provider.api_key.eq_ignore_ascii_case(k));

        let mut accumulated_raw = String::new();
        let mut accumulated_reasoning = String::new();
        let mut cancelled = false;
        let mut stream_bounded = false;
        let mut stream_interrupted = false;
        let mut has_started_answer = false;
        let mut staged_media_tags: Vec<String> = Vec::new();
        let mut staged_documents: Vec<StagedDocument> = Vec::new();
        let mut has_executed_multimedia_or_quiz = false;

        for turn in 0..3 {
            accumulated_raw.clear();
            accumulated_reasoning.clear();
            let mut accumulated_tool_calls: Vec<PendingToolCall> = Vec::new();
            let mut streamed_wire_bytes = 0usize;
            let mut stream_done = false;
            has_started_answer = false;

            let is_final_turn = turn >= 2;
            if supports_tools && !has_executed_multimedia_or_quiz && !is_final_turn {
                payload["tools"] = crate::ai::tools::get_tools_definition();
            } else if let Some(obj) = payload.as_object_mut() {
                obj.remove("tools");
            }

            let mut response = None;
            let mut terminal_transport_failure = false;
            for attempt in 0..MAX_PROVIDER_ATTEMPTS {
                let mut req = self
                    .client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .json(&payload)
                    .timeout(Duration::from_secs(180));
                if use_auth {
                    req = req.header("Authorization", format!("Bearer {}", provider.api_key));
                }

                let mut send_future = Box::pin(req.send());
                let send_result = tokio::select! {
                    changed = cancel_rx.changed() => {
                        if changed.is_ok() && *cancel_rx.borrow() {
                            return cancelled_chat_result(sink);
                        }
                        send_future.as_mut().await
                    }
                    result = send_future.as_mut() => result,
                };

                match send_result {
                    Ok(resp)
                        if is_retryable_status(resp.status())
                            && attempt + 1 < MAX_PROVIDER_ATTEMPTS =>
                    {
                        let status = resp.status();
                        let delay = retry_delay(resp.headers(), attempt);
                        drop(resp);
                        warn!(
                            "Transient provider status {}; retrying attempt {}/{}",
                            status.as_u16(),
                            attempt + 2,
                            MAX_PROVIDER_ATTEMPTS
                        );
                        tokio::select! {
                            _ = tokio::time::sleep(delay) => {}
                            changed = cancel_rx.changed() => {
                                if changed.is_ok() && *cancel_rx.borrow() {
                                    return cancelled_chat_result(sink);
                                }
                            }
                        }
                    }
                    Ok(resp) if resp.status().as_u16() == 400 && payload.get("tools").is_some() => {
                        drop(resp);
                        warn!(
                            "Provider rejected tools parameter (HTTP 400); retrying without tools"
                        );
                        if let Some(obj) = payload.as_object_mut() {
                            obj.remove("tools");
                        }
                        continue;
                    }
                    Ok(resp) => {
                        response = Some(resp);
                        break;
                    }
                    Err(e) => {
                        let retryable_transport = e.is_timeout() || e.is_connect();
                        if retryable_transport && attempt + 1 < MAX_PROVIDER_ATTEMPTS {
                            let delay = Duration::from_millis(
                                500_u64.saturating_mul(1_u64 << attempt.min(5)),
                            );
                            warn!(
                                "Transient provider transport failure; retrying attempt {}/{}",
                                attempt + 2,
                                MAX_PROVIDER_ATTEMPTS
                            );
                            tokio::select! {
                                _ = tokio::time::sleep(delay) => {}
                                changed = cancel_rx.changed() => {
                                    if changed.is_ok() && *cancel_rx.borrow() {
                                        return cancelled_chat_result(sink);
                                    }
                                }
                            }
                        } else {
                            error!(
                                "Error sending AI completion request: {}",
                                if e.is_timeout() {
                                    "timeout"
                                } else {
                                    "transport failure"
                                }
                            );
                            terminal_transport_failure = true;
                            break;
                        }
                    }
                }
            }

            let Some(resp) = response else {
                if let Some(s) = sink {
                    s.on_failure("Provider connection failed", true);
                }
                return (
                    None,
                    if terminal_transport_failure {
                        "⚠️ Terjadi kendala saat memproses jawaban AI.".to_string()
                    } else {
                        "⚠️ Provider tidak merespons setelah beberapa percobaan.".to_string()
                    },
                    Vec::new(),
                    false,
                );
            };

            if !resp.status().is_success() {
                let status_code = resp.status().as_u16();
                drop(resp);
                error!("AI endpoint returned status {status_code}");
                if let Some(s) = sink {
                    s.on_failure(&format!("API status {status_code}"), true);
                }
                return (
                    None,
                    format!("⚠️ Gagal menghubungi AI proxy: {status_code}"),
                    Vec::new(),
                    false,
                );
            }

            let stream = resp.bytes_stream();
            tokio::pin!(stream);
            let mut decoder = SseDecoder::default();

            'streaming: while !stream_done {
                if *cancel_rx.borrow() {
                    cancelled = true;
                    break 'streaming;
                }

                let next_item = tokio::select! {
                    changed = cancel_rx.changed() => {
                        if changed.is_ok() && *cancel_rx.borrow() {
                            cancelled = true;
                            None
                        } else {
                            stream.next().await
                        }
                    }
                    item = stream.next() => item,
                };

                let Some(item) = next_item else {
                    break;
                };
                let bytes = match item {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        stream_interrupted = true;
                        warn!("AI response stream interrupted");
                        break;
                    }
                };
                streamed_wire_bytes = streamed_wire_bytes.saturating_add(bytes.len());
                if streamed_wire_bytes > MAX_STREAM_WIRE_BYTES {
                    stream_bounded = true;
                    stream_interrupted = true;
                    warn!("AI response exceeded XiaoAI's absolute streamed payload limit");
                    break;
                }

                let events = match decoder.push(&bytes) {
                    Ok(events) => events,
                    Err(error) => {
                        stream_interrupted = true;
                        warn!("AI response SSE decode failed: {error}");
                        break;
                    }
                };
                for event in events {
                    match event {
                        StreamEvent::Done => {
                            stream_done = true;
                            break;
                        }
                        StreamEvent::Json(data) => {
                            let Some(delta) = data
                                .get("choices")
                                .and_then(|choices| choices.get(0))
                                .and_then(|choice| choice.get("delta"))
                            else {
                                continue;
                            };

                            if let Some(tool_calls) =
                                delta.get("tool_calls").and_then(Value::as_array)
                            {
                                for tc in tool_calls {
                                    let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0)
                                        as usize;
                                    while accumulated_tool_calls.len() <= index {
                                        accumulated_tool_calls.push(PendingToolCall::default());
                                    }
                                    if let Some(id) = tc.get("id").and_then(Value::as_str) {
                                        accumulated_tool_calls[index].id.push_str(id);
                                    }
                                    if let Some(func) = tc.get("function") {
                                        if let Some(name) = func.get("name").and_then(Value::as_str)
                                        {
                                            accumulated_tool_calls[index].name.push_str(name);
                                        }
                                        if let Some(args) =
                                            func.get("arguments").and_then(Value::as_str)
                                        {
                                            accumulated_tool_calls[index].arguments.push_str(args);
                                        }
                                    }
                                }
                                if has_started_answer {
                                    has_started_answer = false;
                                    if let Some(s) = sink {
                                        s.on_action("Searching", Some(ProgressActivity::Searching));
                                    }
                                }
                            }

                            let reasoning_chunk = delta
                                .get("reasoning_content")
                                .or_else(|| delta.get("reasoning"))
                                .or_else(|| delta.get("thought"))
                                .or_else(|| delta.get("thinking"))
                                .and_then(Value::as_str);
                            if let Some(reasoning_chunk) = reasoning_chunk {
                                if !push_bounded(
                                    &mut accumulated_reasoning,
                                    reasoning_chunk,
                                    MAX_STREAM_REASONING_BYTES,
                                ) {
                                    stream_bounded = true;
                                    stream_interrupted = true;
                                    warn!("AI reasoning exceeded XiaoAI's absolute output limit");
                                    break 'streaming;
                                }
                            }

                            let content_chunk =
                                delta.get("content").and_then(Value::as_str).unwrap_or("");
                            if content_chunk.is_empty() {
                                continue;
                            }
                            if !push_bounded(
                                &mut accumulated_raw,
                                content_chunk,
                                MAX_STREAM_VISIBLE_BYTES,
                            ) {
                                stream_bounded = true;
                                stream_interrupted = true;
                                warn!("AI answer exceeded XiaoAI's absolute output limit");
                                break 'streaming;
                            }

                            let visible_partial =
                                crate::parser::markdown::sanitize_leaked_llm_artifacts(
                                    &accumulated_raw,
                                )
                                .trim()
                                .to_string();

                            if !visible_partial.is_empty() && accumulated_tool_calls.is_empty() {
                                let is_tool_preamble = turn == 0
                                    && payload.get("tools").is_some()
                                    && crate::ai::tools::is_suppressed_tool_preamble_stream(
                                        &visible_partial,
                                    );

                                if !is_tool_preamble {
                                    if !has_started_answer {
                                        has_started_answer = true;
                                    }
                                    if let Some(s) = sink {
                                        s.on_partial_answer(&visible_partial);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !stream_done && !cancelled && !stream_interrupted {
                match decoder.finish() {
                    Ok(events) => {
                        for event in events {
                            if matches!(event, StreamEvent::Done) {
                                stream_done = true;
                            }
                        }
                        if !stream_done {
                            stream_interrupted = true;
                        }
                    }
                    Err(error) => {
                        warn!("AI response SSE final decode failed: {error}");
                        stream_interrupted = true;
                    }
                }
            }

            if accumulated_tool_calls.is_empty() && !accumulated_raw.is_empty() {
                let leaked = extract_leaked_tool_calls(&accumulated_raw);
                if !leaked.is_empty() {
                    accumulated_tool_calls = leaked;
                }
            }

            if turn < 2 && !accumulated_tool_calls.is_empty() && !cancelled {
                let mut tool_results = Vec::new();
                let mut quiz_sent = false;
                let mut quiz_history_summary: Option<String> = None;
                for tc in accumulated_tool_calls.iter() {
                    let name = tc.name.trim();
                    let tool_id = if tc.id.is_empty() {
                        "call_default".to_string()
                    } else {
                        tc.id.clone()
                    };
                    let result = if name == "web_search" {
                        if let Some(s) = sink {
                            s.on_action("Searching", Some(ProgressActivity::Searching));
                        }
                        let parsed_query = serde_json::from_str::<Value>(&tc.arguments)
                            .ok()
                            .and_then(|v| {
                                v.get("query")
                                    .and_then(Value::as_str)
                                    .map(|s| s.to_string())
                            })
                            .unwrap_or_else(|| tc.arguments.clone());

                        tokio::select! {
                            changed = cancel_rx.changed() => {
                                if changed.is_ok() && *cancel_rx.borrow() {
                                    cancelled = true;
                                }
                                "Pencarian dibatalkan.".to_string()
                            }
                            res = crate::ai::tools::execute_web_search(&parsed_query) => res,
                        }
                    } else if name == "fetch_url" {
                        if let Some(s) = sink {
                            s.on_action("Fetching", Some(ProgressActivity::Fetching));
                        }
                        let parsed_url = serde_json::from_str::<Value>(&tc.arguments)
                            .ok()
                            .and_then(|v| {
                                v.get("url").and_then(Value::as_str).map(|s| s.to_string())
                            })
                            .unwrap_or_else(|| tc.arguments.clone());

                        tokio::select! {
                            changed = cancel_rx.changed() => {
                                if changed.is_ok() && *cancel_rx.borrow() {
                                    cancelled = true;
                                }
                                "Pengambilan web dibatalkan.".to_string()
                            }
                            res = crate::ai::tools::fetch_web_content(&parsed_url) => {
                                res.unwrap_or_else(|e| format!("Gagal membaca URL: {e}"))
                            }
                        }
                    } else if name == "create_quiz" {
                        if let Some(s) = sink {
                            s.on_action("Quiz", Some(ProgressActivity::Quiz));
                        }
                        match serde_json::from_str::<crate::ai::tools::CreateQuizArgs>(
                            &tc.arguments,
                        ) {
                            Ok(mut args) => {
                                args.sanitize();
                                match args.validate() {
                                    Ok(correct_id) => {
                                        if let Some(bot_client) = &bot {
                                            let (preamble_msg_id, preamble_err) = if let Some(
                                                preamble,
                                            ) =
                                                &args.preamble
                                            {
                                                let rich_preamble =
                                                    crate::parser::build_full_rich_message(
                                                        preamble, None,
                                                    );
                                                match bot_client
                                                    .send_rich_message(
                                                        chat_id,
                                                        &rich_preamble,
                                                    None,
                                                    None,
                                                        reply_to_message_id,
                                                    )
                                                    .await
                                                {
                                                    Ok(res) => {
                                                        let pid = res
                                                            .get("message_id")
                                                            .and_then(Value::as_i64)
                                                            .or_else(|| {
                                                                res.get("result")
                                                                    .and_then(|r| r.get("message_id"))
                                                                    .and_then(Value::as_i64)
                                                            });
                                                        if let Some(pid) = pid {
                                                            (Some(pid), None)
                                                    } else {
                                                            (None, Some(format!("Gagal mendapatkan ID pesan pengantar kuis dari Telegram: {res}")))
                                                        }
                                                    }
                                                    Err(err) => {
                                                        (None, Some(format!("Gagal mengirim pesan pengantar kuis ke Telegram: {err}")))
                                                    }
                                                }
                                            } else {
                                                (None, None)
                                            };

                                            if let Some(err) = preamble_err {
                                                err
                                            } else {
                                                let poll_reply_to =
                                                    preamble_msg_id.or(reply_to_message_id);
                                                let input_options: Vec<
                                                    crate::bot::models::InputPollOption,
                                                > = args
                                                    .options
                                                    .iter()
                                                    .map(|opt| {
                                                        crate::bot::models::InputPollOption::new(
                                                            opt.as_str(),
                                                        )
                                                    })
                                                    .collect();

                                                let is_anon = args.is_anonymous.unwrap_or(false);
                                                match bot_client
                                                    .send_poll(
                                                        chat_id,
                                                        &args.question,
                                                        &input_options,
                                                        Some(is_anon),
                                                        Some("quiz"),
                                                        Some(correct_id),
                                                        args.explanation.as_deref(),
                                                        None,
                                                        poll_reply_to,
                                                    )
                                                    .await
                                                {
                                                    Ok(_poll_res) => {
                                                        quiz_sent = true;
                                                        let mut summary = String::new();
                                                        if let Some(pre) = &args.preamble {
                                                            summary.push_str(pre);
                                                            summary.push_str("\n\n");
                                                        }
                                                        summary.push_str(&format!(
                                                            "📊 **Kuis**: {}\n",
                                                            args.question
                                                        ));
                                                        for (i, opt) in
                                                            args.options.iter().enumerate()
                                                        {
                                                            let mark = if i as i32 == correct_id {
                                                                " (Benar)"
                                                            } else {
                                                                ""
                                                            };
                                                            summary.push_str(&format!(
                                                                "{}. {opt}{mark}\n",
                                                                i + 1
                                                            ));
                                                        }
                                                        if let Some(exp) = &args.explanation {
                                                            summary.push_str(&format!(
                                                                "\n💡 Penjelasan: {exp}\n"
                                                            ));
                                                        }
                                                        if let Some(existing) =
                                                            &mut quiz_history_summary
                                                        {
                                                            existing.push_str("\n\n---\n\n");
                                                            existing.push_str(&summary);
                                                        } else {
                                                            quiz_history_summary = Some(summary);
                                                        }
                                                        "Kuis native Telegram berhasil dikirim ke obrolan.".to_string()
                                                    }
                                                    Err(err) => {
                                                        if let Some(pid) = preamble_msg_id {
                                                            let _ = bot_client
                                                                .delete_message(chat_id, pid)
                                                                .await;
                                                        }
                                                        format!("Gagal mengirim kuis native ke Telegram: {err}")
                                                    }
                                                }
                                            }
                                        } else {
                                            let mut output = String::new();
                                            if let Some(pre) = &args.preamble {
                                                output.push_str(pre);
                                                output.push_str("\n\n");
                                            }
                                            output.push_str(&format!(
                                                "📊 **Kuis**: {}\n\n",
                                                args.question
                                            ));
                                            for (i, opt) in args.options.iter().enumerate() {
                                                let marker = if i as i32 == correct_id {
                                                    "✅"
                                                } else {
                                                    "⚪"
                                                };
                                                output.push_str(&format!(
                                                    "{marker} {}. {opt}\n",
                                                    i + 1
                                                ));
                                            }
                                            if let Some(exp) = &args.explanation {
                                                output
                                                    .push_str(&format!("\n💡 Penjelasan: {exp}\n"));
                                            }
                                            output
                                        }
                                    }
                                    Err(validation_err) => {
                                        format!("Validasi kuis gagal: {validation_err}")
                                    }
                                }
                            }
                            Err(parse_err) => {
                                format!("Format argumen kuis tidak valid: {parse_err}")
                            }
                        }
                    } else if name == "send_photo" {
                        if let Some(s) = sink {
                            s.on_action("Photo", Some(ProgressActivity::Drawing));
                        }
                        match serde_json::from_str::<crate::ai::tools::SendPhotoArgs>(&tc.arguments)
                        {
                            Ok(mut args) => {
                                args.sanitize();
                                match args.validate() {
                                    Ok(()) => {
                                        let tag = if let Some(caption) = &args.caption {
                                            format!(
                                                r#"<img src="{}" caption="{}"/>"#,
                                                args.url,
                                                caption.replace('"', "&quot;")
                                            )
                                        } else {
                                            format!(r#"<img src="{}"/>"#, args.url)
                                        };
                                        staged_media_tags.push(tag.clone());
                                        format!("Foto telah disiapkan. Tag media: {tag}\nAnda DAPAT menyematkan tag media ini langsung di tengah-tengah teks penjelasan pada posisi yang paling relevan (misal di bawah heading atau di antara paragraf), atau biarkan Xiao menampilkannya secara otomatis. Sekarang berikan penjelasan naratif yang lengkap dan jelas.")
                                    }
                                    Err(validation_err) => {
                                        format!("Validasi foto gagal: {validation_err}")
                                    }
                                }
                            }
                            Err(parse_err) => {
                                format!("Format argumen send_photo tidak valid: {parse_err}")
                            }
                        }
                    } else if name == "send_collage" {
                        if let Some(s) = sink {
                            s.on_action("Collage", Some(ProgressActivity::Drawing));
                        }
                        match serde_json::from_str::<crate::ai::tools::SendCollageArgs>(
                            &tc.arguments,
                        ) {
                            Ok(mut args) => {
                                args.sanitize();
                                match args.validate() {
                                    Ok(()) => {
                                        let caption_attr = if let Some(caption) = &args.caption {
                                            format!(
                                                r#" caption="{}""#,
                                                caption.replace('"', "&quot;")
                                            )
                                        } else {
                                            String::new()
                                        };
                                        let img_tags = args
                                            .urls
                                            .iter()
                                            .map(|u| format!(r#"<img src="{u}"/>"#))
                                            .collect::<Vec<_>>()
                                            .join("");
                                        let collage_tag = format!(
                                            r#"<tg-collage{caption_attr}>{img_tags}</tg-collage>"#
                                        );
                                        staged_media_tags.push(collage_tag.clone());
                                        format!("Album kolase foto telah disiapkan. Tag media: {collage_tag}\nAnda DAPAT menyematkan tag media ini langsung di tengah-tengah penjelasan pada bagian yang paling sesuai. Sekarang berikan penjelasan naratif yang lengkap dan jelas.")
                                    }
                                    Err(validation_err) => {
                                        format!("Validasi kolase foto gagal: {validation_err}")
                                    }
                                }
                            }
                            Err(parse_err) => {
                                format!("Format argumen send_collage tidak valid: {parse_err}")
                            }
                        }
                    } else if name == "send_slideshow" {
                        if let Some(s) = sink {
                            s.on_action("Slideshow", Some(ProgressActivity::Drawing));
                        }
                        match serde_json::from_str::<crate::ai::tools::SendSlideshowArgs>(
                            &tc.arguments,
                        ) {
                            Ok(mut args) => {
                                args.sanitize();
                                match args.validate() {
                                    Ok(()) => {
                                        let caption_attr = if let Some(caption) = &args.caption {
                                            format!(
                                                r#" caption="{}""#,
                                                caption.replace('"', "&quot;")
                                            )
                                        } else {
                                            String::new()
                                        };
                                        let img_tags = args
                                            .urls
                                            .iter()
                                            .map(|u| format!(r#"<img src="{u}"/>"#))
                                            .collect::<Vec<_>>()
                                            .join("");
                                        let slideshow_tag = format!(
                                            r#"<tg-slideshow{caption_attr}>{img_tags}</tg-slideshow>"#
                                        );
                                        staged_media_tags.push(slideshow_tag.clone());
                                        format!("Tayangan slide interaktif telah disiapkan. Tag media: {slideshow_tag}\nAnda DAPAT menyematkan tag media ini langsung di tengah-tengah penjelasan pada bagian yang paling sesuai. Sekarang berikan penjelasan naratif yang lengkap dan jelas.")
                                    }
                                    Err(validation_err) => {
                                        format!("Validasi tayangan slide gagal: {validation_err}")
                                    }
                                }
                            }
                            Err(parse_err) => {
                                format!("Format argumen send_slideshow tidak valid: {parse_err}")
                            }
                        }
                    } else if name == "send_audio" {
                        if let Some(s) = sink {
                            s.on_action("Audio", Some(ProgressActivity::Listening));
                        }
                        match serde_json::from_str::<crate::ai::tools::SendAudioArgs>(&tc.arguments)
                        {
                            Ok(mut args) => {
                                args.sanitize();
                                match args.validate() {
                                    Ok(()) => {
                                        let mut attrs = Vec::new();
                                        if let Some(title) = &args.title {
                                            attrs.push(format!(
                                                r#"title="{}""#,
                                                title.replace('"', "&quot;")
                                            ));
                                        }
                                        if let Some(performer) = &args.performer {
                                            attrs.push(format!(
                                                r#"performer="{}""#,
                                                performer.replace('"', "&quot;")
                                            ));
                                        }
                                        if let Some(caption) = &args.caption {
                                            attrs.push(format!(
                                                r#"caption="{}""#,
                                                caption.replace('"', "&quot;")
                                            ));
                                        }
                                        let extra = if attrs.is_empty() {
                                            String::new()
                                        } else {
                                            format!(" {}", attrs.join(" "))
                                        };
                                        let audio_tag =
                                            format!(r#"<audio src="{}"{extra}/>"#, args.url);
                                        staged_media_tags.push(audio_tag.clone());
                                        format!("Audio telah disiapkan. Tag media: {audio_tag}\nAnda DAPAT menyematkan tag media ini langsung di tengah penjelasan teks pada posisi yang relevan. Sekarang berikan penjelasan naratif yang lengkap dan jelas.")
                                    }
                                    Err(validation_err) => {
                                        format!("Validasi audio gagal: {validation_err}")
                                    }
                                }
                            }
                            Err(parse_err) => {
                                format!("Format argumen send_audio tidak valid: {parse_err}")
                            }
                        }
                    } else if name == "send_voice" {
                        if let Some(s) = sink {
                            s.on_action("Voice", Some(ProgressActivity::Listening));
                        }
                        match serde_json::from_str::<crate::ai::tools::SendVoiceArgs>(&tc.arguments)
                        {
                            Ok(mut args) => {
                                args.sanitize();
                                match args.validate() {
                                    Ok(()) => {
                                        let title =
                                            args.caption.as_deref().unwrap_or("Pesan Suara");
                                        let voice_tag = format!("[rekaman: {title}]({})", args.url);
                                        staged_media_tags.push(voice_tag.clone());
                                        format!("Pesan suara telah disiapkan. Tag media: {voice_tag}\nAnda DAPAT menyematkan tag media ini langsung di tengah penjelasan teks pada posisi yang relevan. Sekarang berikan penjelasan naratif yang lengkap dan jelas.")
                                    }
                                    Err(validation_err) => {
                                        format!("Validasi pesan suara gagal: {validation_err}")
                                    }
                                }
                            }
                            Err(parse_err) => {
                                format!("Format argumen send_voice tidak valid: {parse_err}")
                            }
                        }
                    } else if name == "send_location" {
                        if let Some(s) = sink {
                            s.on_action("Location", Some(ProgressActivity::Looking));
                        }
                        match serde_json::from_str::<crate::ai::tools::SendLocationArgs>(
                            &tc.arguments,
                        ) {
                            Ok(mut args) => {
                                args.sanitize();
                                match args.validate() {
                                    Ok(()) => {
                                        let title_attr = if let Some(title) = &args.title {
                                            format!(r#" title="{}""#, title.replace('"', "&quot;"))
                                        } else {
                                            String::new()
                                        };
                                        let map_tag = format!(
                                            r#"<tg-map lat="{}" lon="{}" zoom="13"{title_attr}/>"#,
                                            args.latitude, args.longitude
                                        );
                                        staged_media_tags.push(map_tag.clone());
                                        format!("Peta lokasi telah disiapkan. Tag media: {map_tag}\nAnda DAPAT menyematkan tag media ini langsung di tengah penjelasan teks pada posisi yang relevan. Sekarang berikan penjelasan naratif yang lengkap dan jelas.")
                                    }
                                    Err(validation_err) => {
                                        format!("Validasi lokasi gagal: {validation_err}")
                                    }
                                }
                            }
                            Err(parse_err) => {
                                format!("Format argumen send_location tidak valid: {parse_err}")
                            }
                        }
                    } else if name == "send_document" {
                        if let Some(s) = sink {
                            s.on_action("Document", Some(ProgressActivity::Reading));
                        }
                        match serde_json::from_str::<crate::ai::tools::SendDocumentArgs>(
                            &tc.arguments,
                        ) {
                            Ok(mut args) => {
                                args.sanitize();
                                match args.validate() {
                                    Ok(()) => {
                                        let file_name =
                                            args.file_name.as_deref().unwrap_or("Dokumen");
                                        let doc_tag =
                                            format!("[document: {file_name}]({})", args.url);
                                        staged_media_tags.push(doc_tag.clone());
                                        format!("Dokumen telah disiapkan. Tag media: {doc_tag}\nAnda DAPAT menyematkan tag media ini langsung di tengah penjelasan teks pada posisi yang relevan. Sekarang berikan penjelasan naratif yang lengkap dan jelas.")
                                    }
                                    Err(validation_err) => {
                                        format!("Validasi dokumen gagal: {validation_err}")
                                    }
                                }
                            }
                            Err(parse_err) => {
                                format!("Format argumen send_document tidak valid: {parse_err}")
                            }
                        }
                    } else if name == "create_document" {
                        if let Some(s) = sink {
                            s.on_action("Document", Some(ProgressActivity::Reading));
                        }
                        match serde_json::from_str::<crate::ai::tools::CreateDocumentArgs>(
                            &tc.arguments,
                        ) {
                            Ok(mut args) => {
                                args.sanitize();
                                match args.validate() {
                                    Ok(()) => {
                                        let (final_bytes, final_filename, mime_type) =
                                            args.into_payload();

                                        let attach_key = format!("doc_{}", staged_documents.len());
                                        let staged_doc = StagedDocument::new(
                                            attach_key,
                                            final_bytes,
                                            mime_type,
                                            final_filename,
                                        );
                                        let doc_tag = staged_doc.markdown_tag();
                                        staged_documents.push(staged_doc);

                                        format!(
                                            "Dokumen '{}' telah berhasil disiapkan di memori. Tag media Telegram: {}\nWAJIB sematkan tag media {} ini langsung di dalam teks jawaban/penjelasan Anda pada posisi yang paling relevan. Berikan penjelasan naratif yang lengkap dan jelas mengenai dokumen ini kepada pengguna.",
                                            staged_documents.last().map(|d| d.filename.as_str()).unwrap_or(""),
                                            doc_tag,
                                            doc_tag
                                        )
                                    }
                                    Err(validation_err) => {
                                        format!("Validasi dokumen gagal: {validation_err}")
                                    }
                                }
                            }
                            Err(parse_err) => {
                                format!("Format argumen create_document tidak valid: {parse_err}")
                            }
                        }
                    } else {
                        format!("Tool '{name}' tidak didukung.")
                    };
                    tool_results.push((tool_id, name.to_string(), tc.arguments.clone(), result));
                    if cancelled {
                        break;
                    }
                }

                if cancelled {
                    break;
                }

                if quiz_sent {
                    let attachment_refs = persist_runtime_attachments(
                        chat_id,
                        thread_id,
                        document_images.as_deref(),
                        image_bytes.as_deref(),
                        mime_type,
                        audio_bytes.as_deref(),
                        audio_mime,
                        doc_name,
                        video_bytes.as_deref(),
                        video_mime,
                    )
                    .await;

                    let user_message_content = encode_user_content(
                        canonical_persisted_prompt(
                            canonical_history_prompt.as_deref(),
                            &clean_prompt,
                        ),
                        attachment_refs,
                    );
                    let user_content_str =
                        serde_json::to_string(&user_message_content).unwrap_or_default();

                    save_scoped_message_async(
                        chat_id,
                        thread_id,
                        user_id,
                        "user".to_string(),
                        user_content_str,
                    )
                    .await;

                    let assistant_content = quiz_history_summary
                        .unwrap_or_else(|| "[Kuis Native Telegram]".to_string());
                    save_scoped_message_async(
                        chat_id,
                        thread_id,
                        user_id,
                        "assistant".to_string(),
                        assistant_content.clone(),
                    )
                    .await;

                    let service_clone = self.clone();
                    let prompt_for_bg = clean_prompt.clone();
                    tokio::spawn(async move {
                        service_clone
                            .process_background_memory_turn(
                                user_id,
                                chat_id,
                                thread_id,
                                &prompt_for_bg,
                                &assistant_content,
                            )
                            .await;
                    });

                    if let Some(s) = sink {
                        s.on_complete();
                    }

                    return (
                        if !accumulated_reasoning.is_empty() {
                            Some(accumulated_reasoning.trim().to_string())
                        } else {
                            None
                        },
                        "[QUIZ_SENT]".to_string(),
                        Vec::new(),
                        false,
                    );
                }

                let tool_calls_json = tool_results
                    .iter()
                    .map(|(id, name, args, _)| {
                        json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": args
                            }
                        })
                    })
                    .collect::<Vec<_>>();

                messages.push(json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": tool_calls_json
                }));

                for (id, _, _, res) in &tool_results {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": res
                    }));
                }

                let has_quiz_or_media = tool_results.iter().any(|(_, name, _, _)| {
                    name == "create_quiz"
                        || name == "send_photo"
                        || name == "send_collage"
                        || name == "send_slideshow"
                        || name == "send_audio"
                        || name == "send_voice"
                        || name == "send_location"
                        || name == "send_document"
                        || name == "create_document"
                });
                if has_quiz_or_media {
                    has_executed_multimedia_or_quiz = true;
                }

                let follow_up_prompt = if has_executed_multimedia_or_quiz {
                    "Berdasarkan media yang telah disiapkan di atas, berikan penjelasan naratif yang kaya, informatif, dan lengkap untuk menjawab pertanyaan pengguna."
                } else if turn >= 1 {
                    "Berdasarkan seluruh hasil pencarian dan informasi di atas, berikan penjelasan naratif yang lengkap, informatif, dan jelas untuk menjawab pertanyaan pengguna. Jika ada tautan foto/gambar atau sumber terverifikasi, sertakan tautan tersebut."
                } else {
                    "Berdasarkan hasil pencarian dan informasi di atas, jika pengguna meminta foto/gambar/logo dan Anda menemukan URL gambar raster terverifikasi (.jpg, .png, .webp) di bagian [URL Foto/Gambar Raster Terverifikasi], Anda WAJIB memanggil tool multimedia resmi (seperti send_photo untuk satu gambar, atau send_collage / send_slideshow untuk beberapa gambar) menggunakan URL tersebut agar media tampil langsung di gelembung pesan Telegram. Jika tidak ada URL gambar raster yang valid, berikan penjelasan naratif yang lengkap dan jelas beserta tautan sumber yang relevan."
                };
                messages.push(json!({
                    "role": "user",
                    "content": follow_up_prompt
                }));

                payload["messages"] = json!(messages);
                let next_is_final = turn >= 1;
                if has_executed_multimedia_or_quiz || next_is_final {
                    if let Some(obj) = payload.as_object_mut() {
                        obj.remove("tools");
                    }
                } else if supports_tools {
                    payload["tools"] = crate::ai::tools::get_tools_definition();
                }

                if let Some(s) = sink {
                    s.on_action("Summarizing", Some(ProgressActivity::Summarizing));
                }

                continue;
            }

            break;
        }

        // Post-process final output
        let (extracted_thinking, mut answer_text) =
            crate::parser::markdown::extract_thinking_and_answer(&accumulated_raw);
        let thinking_text = if !accumulated_reasoning.is_empty() {
            Some(accumulated_reasoning.trim().to_string())
        } else {
            extracted_thinking
        };

        if !staged_documents.is_empty() {
            let mut missing_tags = Vec::new();
            for doc in &staged_documents {
                let tag_needle = doc.attach_uri();
                if !answer_text.contains(&tag_needle) {
                    missing_tags.push(doc.markdown_tag());
                }
            }
            if !missing_tags.is_empty() {
                if answer_text.trim().is_empty() {
                    answer_text = format!(
                        "Berikut adalah dokumen yang Anda minta:\n\n{}",
                        missing_tags.join("\n\n")
                    );
                } else {
                    answer_text.push_str("\n\n");
                    answer_text.push_str(&missing_tags.join("\n\n"));
                }
            }
        }

        if !staged_media_tags.is_empty() {
            if answer_text.trim().is_empty() {
                answer_text = staged_media_tags.join("\n\n");
            } else {
                let mut missing_tags = Vec::new();
                for tag in &staged_media_tags {
                    let is_already_present = if tag.starts_with("<tg-collage") {
                        answer_text.contains("<tg-collage") || answer_text.contains("kolase")
                    } else if tag.starts_with("<tg-slideshow") {
                        answer_text.contains("<tg-slideshow") || answer_text.contains("carousel")
                    } else if let Some(src) = tag
                        .split(r#"src=""#)
                        .nth(1)
                        .and_then(|s| s.split('"').next())
                    {
                        !src.is_empty() && answer_text.contains(src)
                    } else if let Some(url) =
                        tag.split("](").nth(1).and_then(|s| s.split(')').next())
                    {
                        !url.is_empty() && answer_text.contains(url)
                    } else if tag.starts_with("<tg-map") {
                        answer_text.contains("<tg-map")
                            || (tag.contains(r#"lat=""#) && {
                                let lat = tag
                                    .split(r#"lat=""#)
                                    .nth(1)
                                    .and_then(|s| s.split('"').next())
                                    .unwrap_or("");
                                !lat.is_empty() && answer_text.contains(lat)
                            })
                    } else {
                        answer_text.contains(tag)
                    };

                    if !is_already_present {
                        missing_tags.push(tag.as_str());
                    }
                }
                if !missing_tags.is_empty() {
                    let header = missing_tags.join("\n\n");
                    answer_text = format!("{header}\n\n{}", answer_text.trim());
                }
            }
        }

        if cancelled {
            if answer_text.trim().is_empty() {
                answer_text = "⏹️ Generasi dihentikan oleh pengguna.".to_string();
            } else {
                answer_text.push_str("\n\n_⏹️ Generasi dihentikan oleh pengguna._");
            }
        } else if stream_bounded {
            if answer_text.trim().is_empty() {
                answer_text =
                    "⚠️ Respons provider melewati batas ukuran aman XiaoAI dan dihentikan."
                        .to_string();
            } else {
                answer_text.push_str(
                    "\n\n_⚠️ Respons dihentikan karena melewati batas ukuran aman XiaoAI._",
                );
            }
        } else if stream_interrupted {
            if answer_text.trim().is_empty() {
                answer_text = "⚠️ Stream provider terputus sebelum jawaban diterima.".to_string();
            } else {
                answer_text
                    .push_str("\n\n_⚠️ Stream provider terputus; jawaban mungkin tidak lengkap._");
            }
        } else if answer_text.trim().is_empty() {
            if !staged_media_tags.is_empty() {
                answer_text = staged_media_tags.join("\n\n");
            } else if !accumulated_reasoning.is_empty() {
                answer_text = "Maaf, Xiao telah memproses permintaan ini namun model tidak menghasilkan teks jawaban. Silakan coba ulangi pertanyaan dengan instruksi yang lebih jelas.".to_string();
            } else {
                answer_text = "Maaf, Xiao tidak dapat menemukan informasi yang diminta saat ini. Silakan coba ulangi pertanyaan dengan lebih spesifik.".to_string();
            }
        }

        if let Some(s) = sink {
            if cancelled {
                s.on_partial_answer(&answer_text);
                s.on_failure("Stopped by user", false);
            } else if stream_interrupted {
                s.on_partial_answer(&answer_text);
                s.on_failure(
                    if stream_bounded {
                        "Provider output exceeded safety limit"
                    } else {
                        "Provider stream interrupted"
                    },
                    true,
                );
            } else {
                if !has_started_answer {
                    s.on_action("Writing", Some(ProgressActivity::Writing));
                }
                s.on_complete();
            }
        }

        // Cancelled/interrupted output is presentation-only. Do not make a
        // partial answer canonical history: retry/follow-up context must only
        // see completed assistant turns.
        if cancelled || stream_interrupted {
            return (thinking_text, answer_text, staged_documents, cancelled);
        }

        // Persist runtime attachments to storage
        let attachment_refs = persist_runtime_attachments(
            chat_id,
            thread_id,
            document_images.as_deref(),
            image_bytes.as_deref(),
            mime_type,
            audio_bytes.as_deref(),
            audio_mime,
            doc_name,
            video_bytes.as_deref(),
            video_mime,
        )
        .await;

        let user_message_content = encode_user_content(
            canonical_persisted_prompt(canonical_history_prompt.as_deref(), &clean_prompt),
            attachment_refs.clone(),
        );
        let user_content_str = serde_json::to_string(&user_message_content).unwrap_or_default();
        if cancelled {
            return (thinking_text, answer_text, staged_documents, true);
        }
        let assistant_content_str = answer_text.clone();

        save_scoped_message_async(
            chat_id,
            thread_id,
            user_id,
            "user".to_string(),
            user_content_str,
        )
        .await;
        save_scoped_message_async(
            chat_id,
            thread_id,
            user_id,
            "assistant".to_string(),
            assistant_content_str,
        )
        .await;

        let service_clone = self.clone();
        let prompt_for_bg = clean_prompt.clone();
        let answer_for_bg = answer_text.clone();
        tokio::spawn(async move {
            service_clone
                .process_background_memory_turn(
                    user_id,
                    chat_id,
                    thread_id,
                    &prompt_for_bg,
                    &answer_for_bg,
                )
                .await;
        });

        (thinking_text, answer_text, staged_documents, cancelled)
    }

    async fn process_background_memory_turn(
        &self,
        user_id: i64,
        chat_id: i64,
        thread_id: i64,
        user_prompt: &str,
        assistant_answer: &str,
    ) {
        let curator_route = match self.resolve_model_route(ModelRole::Curator).await {
            Ok(route) => route,
            Err(e) => {
                debug!("Curator model route disabled or unavailable: {e}");
                return;
            }
        };

        let total_count = count_scoped_messages_async(chat_id, thread_id).await;

        let text_lower = user_prompt.to_ascii_lowercase();
        let personal_hints = [
            "nama saya",
            "namaku",
            "my name",
            "saya suka",
            "prefer",
            "tech stack",
            "proyek",
            "project",
            "bekerja sebagai",
            "tinggal di",
            "bahasa",
            "panggil aku",
            "i am",
            "i work",
            "i live",
            "my favorite",
            "hobi",
        ];
        let has_hint = personal_hints.iter().any(|hint| text_lower.contains(hint));
        let has_significant_data = has_hint || user_prompt.chars().count() > 80;
        let should_extract = has_significant_data || total_count <= 6 || (total_count % 6 == 0);

        if should_extract {
            self.extract_user_facts_background(
                user_id,
                &curator_route.provider,
                &curator_route.model,
                user_prompt,
                assistant_answer,
            )
            .await;
        }

        if total_count > 20 && (total_count % 20 == 0) {
            self.summarize_older_history_background(
                user_id,
                chat_id,
                thread_id,
                &curator_route.provider,
                &curator_route.model,
            )
            .await;
        }
    }

    async fn request_completion_text(
        &self,
        provider: &ProviderConfig,
        payload: &Value,
    ) -> Option<String> {
        let url = provider_url(&provider.endpoint, "chat/completions");
        let mut req = self.client.post(&url).json(payload);
        if !provider.api_key.is_empty()
            && !["none", "-", "no"]
                .iter()
                .any(|k| provider.api_key.eq_ignore_ascii_case(k))
        {
            req = req.bearer_auth(&provider.api_key);
        }

        let resp = req.timeout(Duration::from_secs(30)).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        let body = resp.json::<Value>().await.ok()?;
        body.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
    }

    async fn extract_user_facts_background(
        &self,
        user_id: i64,
        provider: &ProviderConfig,
        model: &str,
        user_prompt: &str,
        assistant_answer: &str,
    ) {
        let existing_memories = get_user_memories_async(user_id).await;
        let mut existing_context = String::new();
        if !existing_memories.is_empty() {
            existing_context.push_str("Current known user facts:\n");
            for (k, f) in &existing_memories {
                existing_context.push_str(&format!("- {k}: {f}\n"));
            }
            existing_context.push('\n');
        }

        let extract_prompt = format!(
            "{}Recent conversation:\nUser input: \"{}\"\nAssistant reply: \"{}\"\n\n\
            Analyze the interaction and extract or update persistent personal profile facts about the user \
            (e.g., Name/Callsign, Preferred Language, Tech Stack, Ongoing Projects, Key Preferences, Style, Role/Work, Location). \
            If a statement updates or contradicts a previously known fact, output the updated fact with the matching key to supersede it. \
            Respond ONLY with a valid JSON array of objects with \"key\" and \"fact\" properties. \
            Example: [{{\"key\": \"Name\", \"fact\": \"Alex\"}}, {{\"key\": \"Tech Stack\", \"fact\": \"Rust, Linux, Termux\"}}]. \
            If there are no personal facts about the user or nothing new/updated, return an empty array []. \
            Do not output any markdown formatting, thoughts, or explanations, only the raw JSON array.",
            existing_context,
            truncate_chars(user_prompt, 800),
            truncate_chars(assistant_answer, 400),
        );

        let payload = json!({
            "model": model,
            "messages": [
                {
                    "role": "system",
                    "content": "You are a concise background fact extraction engine. Always output only valid JSON without code fences or extra text."
                },
                {
                    "role": "user",
                    "content": extract_prompt
                }
            ],
            "temperature": 0.1,
            "max_tokens": 400,
            "stream": false
        });

        let content = match self.request_completion_text(provider, &payload).await {
            Some(c) => c,
            None => return,
        };

        let clean_json = if let Some(stripped) = content.strip_prefix("```json") {
            stripped.trim_end_matches("```").trim()
        } else if let Some(stripped) = content.strip_prefix("```") {
            stripped.trim_end_matches("```").trim()
        } else {
            content.as_str()
        };

        let json_str =
            if let (Some(start), Some(end)) = (clean_json.find('['), clean_json.rfind(']')) {
                if start < end {
                    &clean_json[start..=end]
                } else {
                    clean_json
                }
            } else {
                clean_json
            };

        #[derive(Deserialize)]
        struct ExtractedFact {
            key: String,
            fact: String,
        }

        if let Ok(facts) = serde_json::from_str::<Vec<ExtractedFact>>(json_str) {
            for f in facts {
                let k = f.key.trim().to_string();
                let v = f.fact.trim().to_string();
                if !k.is_empty() && !v.is_empty() && k.len() <= 64 && v.len() <= 500 {
                    save_user_memory_async(user_id, k, v).await;
                }
            }
        }
    }

    async fn summarize_older_history_background(
        &self,
        user_id: i64,
        chat_id: i64,
        thread_id: i64,
        provider: &ProviderConfig,
        model: &str,
    ) {
        let messages = load_scoped_messages_async(chat_id, thread_id, 30).await;
        if messages.len() < 10 {
            return;
        }
        let mut turns_text = String::new();
        for m in &messages[..messages.len().saturating_sub(10)] {
            let preview = match &m.content {
                Value::String(s) => s.as_str(),
                val => val.as_str().unwrap_or(""),
            };
            turns_text.push_str(&format!("{}: {}\n", m.role, truncate_chars(preview, 200)));
        }
        let existing_summary = get_scoped_summary_async(chat_id, thread_id).await;
        let mut context_text = String::new();
        if let Some(ref prev) = existing_summary {
            context_text.push_str(&format!(
                "Previous summary of earlier discussion:\n{prev}\n\n"
            ));
        }
        context_text.push_str(&format!("Recent conversation turns:\n{turns_text}\n\n"));

        let summary_prompt = format!(
            "{}Update and condense the ongoing context, key discussions, and decisions in 2-3 concise sentences. Output only the plain summary.",
            truncate_chars(&context_text, 3500)
        );
        let payload = json!({
            "model": model,
            "messages": [
                { "role": "system", "content": "You are a concise conversational summarizer." },
                { "role": "user", "content": summary_prompt }
            ],
            "temperature": 0.2,
            "max_tokens": 250,
            "stream": false
        });

        if let Some(summary) = self.request_completion_text(provider, &payload).await {
            if !summary.is_empty() {
                save_scoped_summary_async(chat_id, thread_id, summary.clone()).await;

                // Also extract any lingering user profile facts from older turns into Tier 1
                self.extract_user_facts_background(user_id, provider, model, &turns_text, &summary)
                    .await;
            }
        }
    }
}
