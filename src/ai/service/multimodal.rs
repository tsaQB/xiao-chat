use reqwest::multipart::{Form, Part};
use serde_json::{json, Value};
use std::time::Duration;

use crate::util::truncate_chars;

use super::provider_url;
use super::read_bounded_json;
use super::AIChatService;
use crate::ai::routing::ResolvedModelRoute;
use crate::ai::storage::CapabilityRecord;

pub struct SpecialistObservationInput<'a> {
    pub prompt: &'a str,
    pub image_bytes: Option<&'a [u8]>,
    pub document_images: Option<&'a [Vec<u8>]>,
    pub mime_type: Option<&'a str>,
    pub video_bytes: Option<&'a [u8]>,
    pub video_mime: Option<&'a str>,
}

pub fn resolve_audio_file_and_mime(
    mime_type: Option<&str>,
    suggested_filename: Option<&str>,
) -> (&'static str, String) {
    let clean_mime = mime_type.unwrap_or("").trim().to_ascii_lowercase();
    let (mime_str, ext) =
        if clean_mime.starts_with("audio/ogg") || clean_mime.starts_with("application/ogg") {
            ("audio/ogg", "ogg")
        } else if clean_mime.starts_with("audio/opus") {
            ("audio/opus", "opus")
        } else if clean_mime.starts_with("audio/mpeg") || clean_mime.starts_with("audio/mp3") {
            ("audio/mpeg", "mp3")
        } else if clean_mime.starts_with("audio/x-m4a") {
            ("audio/x-m4a", "m4a")
        } else if clean_mime.starts_with("audio/mp4") || clean_mime.starts_with("audio/m4a") {
            ("audio/mp4", "m4a")
        } else if clean_mime.starts_with("audio/wav")
            || clean_mime.starts_with("audio/x-wav")
            || clean_mime.starts_with("audio/wave")
        {
            ("audio/wav", "wav")
        } else if clean_mime.starts_with("audio/flac") || clean_mime.starts_with("audio/x-flac") {
            ("audio/flac", "flac")
        } else if clean_mime.starts_with("audio/webm") {
            ("audio/webm", "webm")
        } else {
            let lower_fn = suggested_filename.unwrap_or("").to_ascii_lowercase();
            if lower_fn.ends_with(".ogg") || lower_fn.ends_with(".oga") {
                ("audio/ogg", "ogg")
            } else if lower_fn.ends_with(".opus") {
                ("audio/opus", "opus")
            } else if lower_fn.ends_with(".mp3") {
                ("audio/mpeg", "mp3")
            } else if lower_fn.ends_with(".m4a") {
                ("audio/mp4", "m4a")
            } else if lower_fn.ends_with(".wav") {
                ("audio/wav", "wav")
            } else if lower_fn.ends_with(".flac") {
                ("audio/flac", "flac")
            } else if lower_fn.ends_with(".webm") {
                ("audio/webm", "webm")
            } else {
                ("application/octet-stream", "bin")
            }
        };

    let filename = if let Some(suggested) = suggested_filename {
        let trimmed = suggested.trim();
        let safe_name: String = trimmed
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .take(64)
            .collect();
        if safe_name.to_ascii_lowercase().ends_with(&format!(".{ext}")) {
            safe_name
        } else if !safe_name.is_empty() && safe_name != "." {
            format!("{safe_name}.{ext}")
        } else {
            format!("audio.{ext}")
        }
    } else {
        format!("audio.{ext}")
    };

    (mime_str, filename)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeAudioFormatError {
    Unknown,
}

impl std::fmt::Display for NativeAudioFormatError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => write!(
                formatter,
                "audio format is unknown; Xiao will not guess a native input_audio format"
            ),
        }
    }
}

pub(crate) fn native_audio_input_format(
    mime_type: Option<&str>,
    file_name: Option<&str>,
) -> Result<&'static str, NativeAudioFormatError> {
    let clean_mime = mime_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let mime_result = match clean_mime.as_str() {
        "" => None,
        "audio/mpeg" | "audio/mp3" => Some(Ok("mp3")),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Some(Ok("wav")),
        "audio/ogg" | "application/ogg" => Some(Ok("ogg")),
        "audio/opus" => Some(Ok("opus")),
        "audio/mp4" => Some(Ok("mp4")),
        "audio/m4a" | "audio/x-m4a" | "audio/aac" => Some(Ok("m4a")),
        "audio/flac" | "audio/x-flac" => Some(Ok("flac")),
        "audio/webm" => Some(Ok("webm")),
        _ => None,
    };
    if let Some(result) = mime_result {
        return result;
    }

    let lower_name = file_name.unwrap_or("").trim().to_ascii_lowercase();
    if lower_name.ends_with(".mp3") {
        Ok("mp3")
    } else if lower_name.ends_with(".wav") {
        Ok("wav")
    } else if lower_name.ends_with(".ogg") || lower_name.ends_with(".oga") {
        Ok("ogg")
    } else if lower_name.ends_with(".opus") {
        Ok("opus")
    } else if lower_name.ends_with(".m4a") || lower_name.ends_with(".aac") {
        Ok("m4a")
    } else if lower_name.ends_with(".mp4") {
        Ok("mp4")
    } else if lower_name.ends_with(".flac") {
        Ok("flac")
    } else if lower_name.ends_with(".webm") {
        Ok("webm")
    } else {
        Err(NativeAudioFormatError::Unknown)
    }
}

pub(crate) fn native_audio_input_part(
    bytes: &[u8],
    mime_type: Option<&str>,
    file_name: Option<&str>,
) -> Result<Value, NativeAudioFormatError> {
    use base64::Engine;
    let format = native_audio_input_format(mime_type, file_name)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(json!({
        "type": "input_audio",
        "input_audio": {"data": encoded, "format": format}
    }))
}

pub(crate) fn historical_native_audio_input_part(
    bytes: &[u8],
    mime_type: &str,
    file_name: Option<&str>,
) -> Result<Value, NativeAudioFormatError> {
    native_audio_input_part(bytes, Some(mime_type), file_name)
}

pub(crate) fn resolved_runtime_media_mime(
    mime_type: Option<&str>,
    expected_prefix: &str,
    media_name: &str,
) -> Result<String, String> {
    let normalized = mime_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if normalized.starts_with(expected_prefix) {
        Ok(normalized)
    } else {
        Err(format!(
            "{media_name} MIME type is unknown or does not match the media kind; refusing to invent media metadata"
        ))
    }
}

pub(crate) fn media_data_url(
    bytes: &[u8],
    mime_type: Option<&str>,
    expected_prefix: &str,
    media_name: &str,
) -> Result<String, String> {
    use base64::Engine;
    let mime_type = resolved_runtime_media_mime(mime_type, expected_prefix, media_name)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime_type};base64,{encoded}"))
}

pub(crate) fn resolved_audio_persistence_mime(
    mime_type: Option<&str>,
    file_name: Option<&str>,
) -> String {
    let normalized = mime_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if normalized.starts_with("audio/") {
        normalized
    } else {
        resolve_audio_file_and_mime(None, file_name).0.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioExecutionMode {
    Native,
    Transcription,
}

pub(crate) fn select_audio_execution_mode(
    _capability: &CapabilityRecord,
    inherited_main: bool,
    mime_type: Option<&str>,
    file_name: Option<&str>,
) -> Result<AudioExecutionMode, String> {
    if inherited_main && native_audio_input_format(mime_type, file_name).is_ok() {
        Ok(AudioExecutionMode::Native)
    } else {
        Ok(AudioExecutionMode::Transcription)
    }
}

pub(crate) fn specialist_chat_payload(model: &str, content: Vec<Value>) -> Value {
    json!({
        "model": model,
        "messages": [{"role": "user", "content": content}],
        "stream": false,
        "max_tokens": 1200
    })
}

impl AIChatService {
    pub(crate) async fn run_specialist_observation(
        &self,
        route: &ResolvedModelRoute,
        input: SpecialistObservationInput<'_>,
    ) -> Result<String, String> {
        use base64::Engine;

        let SpecialistObservationInput {
            prompt,
            image_bytes,
            document_images,
            mime_type,
            video_bytes,
            video_mime,
        } = input;

        let mut content = vec![json!({
            "type": "text",
            "text": if prompt.trim().is_empty() {
                "Observe the supplied media accurately. Return a concise factual observation for another model to use. Do not answer beyond what is visible/audible in the media."
            } else {
                prompt
            }
        })];
        if let Some(pages) = document_images.filter(|pages| !pages.is_empty()) {
            for page in pages.iter().take(8) {
                let encoded = base64::engine::general_purpose::STANDARD.encode(page);
                content.push(json!({
                    "type": "image_url",
                    "image_url": {"url": format!("data:image/png;base64,{encoded}"), "detail": "high"}
                }));
            }
        } else if let Some(bytes) = video_bytes {
            let data_url = media_data_url(bytes, video_mime, "video/", "video")?;
            content.push(json!({
                "type": "image_url",
                "image_url": {"url": data_url}
            }));
        } else if let Some(bytes) = image_bytes {
            let data_url = media_data_url(bytes, mime_type, "image/", "image")?;
            content.push(json!({
                "type": "image_url",
                "image_url": {"url": data_url, "detail": "auto"}
            }));
        }

        let url = provider_url(&route.provider.endpoint, "chat/completions");
        let mut request = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&specialist_chat_payload(&route.model, content))
            .timeout(Duration::from_secs(90));
        if !route.provider.api_key.is_empty()
            && !["none", "-", "no", "null"]
                .iter()
                .any(|value| route.provider.api_key.eq_ignore_ascii_case(value))
        {
            request = request.header(
                "Authorization",
                format!("Bearer {}", route.provider.api_key),
            );
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                "specialist request timed out; capability remains unchanged".to_string()
            } else {
                "specialist request failed".to_string()
            }
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "specialist {} / {} returned HTTP {}",
                route.provider.name,
                route.model,
                response.status().as_u16()
            ));
        }
        let body = read_bounded_json(response).await?;
        let content = body
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"));
        let text = if let Some(text) = content.and_then(Value::as_str) {
            text.to_string()
        } else if let Some(parts) = content.and_then(Value::as_array) {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        } else {
            String::new()
        };
        let text = text.trim();
        if text.is_empty() {
            return Err("specialist returned an empty observation".to_string());
        }
        Ok(truncate_chars(text, 12_000))
    }

    pub(crate) async fn transcribe_audio_resolved(
        &self,
        route: &ResolvedModelRoute,
        audio_bytes: Vec<u8>,
        file_name: &str,
        mime_type: Option<&str>,
    ) -> Result<String, String> {
        let (safe_mime, safe_filename) = resolve_audio_file_and_mime(mime_type, Some(file_name));
        if audio_bytes.is_empty() || audio_bytes.len() > 20 * 1024 * 1024 {
            return Err("Audio kosong atau melebihi batas 20 MiB.".into());
        }
        if !safe_mime.starts_with("audio/") {
            return Err("Format audio tidak dapat direpresentasikan untuk transkripsi.".into());
        }
        let stt_url = provider_url(&route.provider.endpoint, "audio/transcriptions");
        let part = Part::bytes(audio_bytes.clone())
            .file_name(safe_filename)
            .mime_str(safe_mime)
            .map_err(|error| format!("multipart audio error: {error}"))?;
        let form = Form::new()
            .part("file", part)
            .text("model", route.model.clone());
        let mut request = self
            .client
            .post(stt_url)
            .multipart(form)
            .timeout(Duration::from_secs(90));
        if !route.provider.api_key.is_empty()
            && !["none", "-", "no", "null"]
                .iter()
                .any(|value| route.provider.api_key.eq_ignore_ascii_case(value))
        {
            request = request.header(
                "Authorization",
                format!("Bearer {}", route.provider.api_key),
            );
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                "audio transcription timed out; timeout is not Unsupported".to_string()
            } else {
                "audio transcription request failed".to_string()
            }
        })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            if status == 404 || status == 405 {
                // Fallback: If /audio/transcriptions is not supported (Protocol Mismatch) and
                // the audio can be formatted for multimodal chat, try transcribing via chat/completions.
                if let Ok(audio_part) =
                    native_audio_input_part(&audio_bytes, mime_type, Some(file_name))
                {
                    tracing::info!(
                        "Audio STT route {} / {} returned HTTP {}; falling back to multimodal chat transcription",
                        route.provider.name,
                        route.model,
                        status
                    );
                    return self
                        .transcribe_audio_via_chat_completions(route, audio_part)
                        .await;
                }

                return Err(format!(
                    "Audio STT route {} / {} returned HTTP {}: endpoint /audio/transcriptions is not supported by this provider protocol.",
                    route.provider.name, route.model, status
                ));
            }
            return Err(format!(
                "Audio STT {} / {} returned HTTP {}",
                route.provider.name, route.model, status
            ));
        }
        let body = read_bounded_json(response).await?;
        let text = body
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if text.is_empty() {
            return Err("audio transcription returned empty text".to_string());
        }
        Ok(truncate_chars(text, 32_000))
    }

    pub(crate) async fn transcribe_audio_via_chat_completions(
        &self,
        route: &ResolvedModelRoute,
        audio_part: Value,
    ) -> Result<String, String> {
        let content = vec![
            json!({
                "type": "text",
                "text": "Transcribe the following audio accurately word-for-word. Return only the verbatim transcript. Do not add commentary or formatting."
            }),
            audio_part,
        ];
        let payload = json!({
            "model": route.model,
            "messages": [{"role": "user", "content": content}],
            "stream": false,
            "max_tokens": 2048
        });

        let url = provider_url(&route.provider.endpoint, "chat/completions");
        let mut request = self
            .client
            .post(url)
            .json(&payload)
            .timeout(Duration::from_secs(90));
        if !route.provider.api_key.is_empty()
            && !["none", "-", "no", "null"]
                .iter()
                .any(|value| route.provider.api_key.eq_ignore_ascii_case(value))
        {
            request = request.header(
                "Authorization",
                format!("Bearer {}", route.provider.api_key),
            );
        }

        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                "multimodal audio transcription timed out".to_string()
            } else {
                "multimodal audio transcription request failed".to_string()
            }
        })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(format!(
                "Multimodal Audio STT {} / {} returned HTTP {}",
                route.provider.name, route.model, status
            ));
        }

        let body = read_bounded_json(response).await?;
        let text = body
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();

        if text.is_empty() {
            return Err("multimodal audio transcription returned empty text".to_string());
        }
        Ok(truncate_chars(text, 32_000))
    }
}
