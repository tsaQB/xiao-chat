use super::context::*;
use super::generation::*;
use super::image::*;
use super::multimodal::*;
use super::session::signal_generation_cancel;
use super::*;
use crate::ai::routing::*;
use crate::ai::storage::*;
use crate::bot::url_policy::is_unsafe_remote_ip;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, RwLock};

fn session(id: usize) -> ChatSession {
    ChatSession {
        id,
        name: format!("Session {id}"),
        messages: Vec::new(),
        created_at: "now".to_string(),
        revision: 0,
    }
}

#[test]
fn legacy_active_index_maps_to_stable_session_id() {
    let sessions = vec![session(3), session(8), session(20)];
    assert_eq!(
        crate::ai::storage::legacy_active_session_id(Some(1), &sessions),
        Some(8)
    );
    assert_eq!(
        crate::ai::storage::legacy_active_session_id(Some(99), &sessions),
        Some(3)
    );
}

#[test]
fn session_id_counter_never_reuses_deleted_high_water_mark() {
    assert_eq!(crate::ai::storage::compute_next_session_id(Some(21), 8), 21);
    assert_eq!(crate::ai::storage::compute_next_session_id(Some(4), 8), 9);
    assert_eq!(crate::ai::storage::compute_next_session_id(None, 8), 9);
}

fn evidence_record(
    kind: CapabilityKind,
    outcome: CapabilityState,
    age: chrono::Duration,
) -> CapabilityRecord {
    let mut record = CapabilityRecord::default();
    match kind {
        CapabilityKind::ImageInput => {
            record.supports_image_input = Some(outcome == CapabilityState::Supported)
        }
        CapabilityKind::AudioInput => {
            record.supports_audio_input = Some(outcome == CapabilityState::Supported)
        }
        CapabilityKind::AudioTranscription => {
            record.supports_audio_transcription = Some(outcome == CapabilityState::Supported)
        }
        CapabilityKind::VideoInput => {
            record.supports_video_input = Some(outcome == CapabilityState::Supported)
        }
        _ => {}
    }
    record
        .evidence
        .push(crate::ai::storage::CapabilityEvidence {
            capability: kind,
            source: crate::ai::storage::CapabilityEvidenceSource::ActiveProbe,
            outcome,
            checked_at: (chrono::Utc::now() - age).to_rfc3339(),
            detail: None,
        });
    record
}

#[test]
fn history_replay_uses_routes_not_diagnostic_evidence() {
    let provider = ProviderConfig {
        id: "main".into(),
        name: "Main".into(),
        endpoint: "https://a.example/v1".into(),
        api_key: String::new(),
        api_key_ref: None,
        models: vec!["model".into()],
        active_model: "model".into(),
    };
    let mut snapshot = GenerationModelSnapshot {
        provider_store: ProviderStore {
            active_id: Some(provider.id.clone()),
            providers: vec![provider.clone()],
        },
        routing: crate::ai::routing::ModelRoutingConfig::default(),
        capabilities: crate::ai::storage::CapabilityRegistry { models: vec![] },
    };
    for (kind, role, modality, part) in [
        (
            "image",
            ModelRole::Vision,
            CapabilityKind::ImageInput,
            json!({"type":"image_url", "image_url":{"url":"data:image/png;base64,aW1hZ2U="}}),
        ),
        (
            "video",
            ModelRole::Video,
            CapabilityKind::VideoInput,
            json!({"type":"image_url", "image_url":{"url":"data:video/mp4;base64,dmlkZW8="}}),
        ),
        (
            "audio",
            ModelRole::AudioStt,
            CapabilityKind::AudioInput,
            json!({"type":"input_audio", "input_audio":{"format":"mp3", "data":"YXVkaW8="}}),
        ),
    ] {
        for state in [
            CapabilityState::Unknown,
            CapabilityState::Supported,
            CapabilityState::Unsupported,
        ] {
            for age in [chrono::Duration::hours(1), chrono::Duration::days(90)] {
                let mut record = evidence_record(modality, state, age);
                record.provider_id = provider.endpoint.clone();
                record.model = provider.active_model.clone();
                snapshot.capabilities.models = vec![record];
                snapshot
                    .routing
                    .set_route(role, ModelRoute::MainModel)
                    .expect("set_route succeeds");
                assert!(history_attachment_authorized(&snapshot, kind));
                let legacy = json!([part.clone()]);
                assert_eq!(sanitize_legacy_history(&legacy, &snapshot), legacy);
                for route in [
                    ModelRoute::Disabled,
                    ModelRoute::Specific {
                        provider_id: provider.id.clone(),
                        model: "model".into(),
                    },
                ] {
                    snapshot
                        .routing
                        .set_route(role, route)
                        .expect("set_route succeeds");
                    assert!(!history_attachment_authorized(&snapshot, kind));
                    assert_ne!(sanitize_legacy_history(&legacy, &snapshot), legacy);
                }
            }
        }
        snapshot
            .routing
            .set_route(role, ModelRoute::MainModel)
            .expect("set_route succeeds");
    }
    let unsafe_legacy = json!([
        {"type":"image_url", "image_url":{"url":"http://127.0.0.1/private"}},
        {"type":"image_url", "image_url":{"url":"data:image/png;base64,%%%"}},
        {"type":"input_audio", "input_audio":{"format":"exe", "data":"YQ=="}}
    ]);
    assert!(sanitize_legacy_history(&unsafe_legacy, &snapshot)
        .as_array()
        .expect("sanitized legacy is array")
        .iter()
        .all(|part| part["type"] == "text"));
}

#[test]
fn media_data_urls_preserve_resolved_mime_and_fail_closed() {
    for (mime_type, expected_prefix) in [
        ("image/png", "data:image/png;base64,"),
        ("image/webp", "data:image/webp;base64,"),
        ("video/mp4", "data:video/mp4;base64,"),
        ("video/webm", "data:video/webm;base64,"),
        ("video/x-matroska", "data:video/x-matroska;base64,"),
    ] {
        let expected_kind = if mime_type.starts_with("image/") {
            "image/"
        } else {
            "video/"
        };
        let data_url = media_data_url(b"media", Some(mime_type), expected_kind, "test media")
            .expect("media_data_url succeeds");
        assert!(data_url.starts_with(expected_prefix), "{mime_type}");
    }

    assert!(media_data_url(b"media", None, "image/", "image").is_err());
    assert!(media_data_url(b"media", Some("video/webm"), "image/", "image").is_err());
    assert!(media_data_url(b"media", Some("image/png"), "video/", "video").is_err());
}

#[test]
fn audio_persistence_mime_recovers_known_filename_without_overwriting_explicit_mime() {
    assert_eq!(
        resolved_audio_persistence_mime(None, Some("sample.mp3")),
        "audio/mpeg"
    );
    assert_eq!(
        resolved_audio_persistence_mime(None, Some("sample.wav")),
        "audio/wav"
    );
    assert_eq!(
        resolved_audio_persistence_mime(None, Some("sample.opus")),
        "audio/opus"
    );
    assert_eq!(
        resolved_audio_persistence_mime(None, Some("sample.flac")),
        "audio/flac"
    );
    assert_eq!(
        resolved_audio_persistence_mime(Some("Audio/WebM; codecs=opus"), Some("sample.mp3")),
        "audio/webm"
    );
}

#[test]
fn native_audio_format_mapping_is_protocol_safe() {
    for mime in ["audio/mpeg", "audio/mp3"] {
        assert_eq!(native_audio_input_format(Some(mime), None), Ok("mp3"));
    }
    for mime in ["audio/wav", "audio/x-wav", "audio/wave"] {
        assert_eq!(native_audio_input_format(Some(mime), None), Ok("wav"));
    }
    for mime in ["audio/ogg", "application/ogg"] {
        assert_eq!(native_audio_input_format(Some(mime), None), Ok("ogg"));
    }
    assert_eq!(
        native_audio_input_format(Some("audio/opus"), None),
        Ok("opus")
    );
    assert_eq!(
        native_audio_input_format(Some("audio/m4a"), None),
        Ok("m4a")
    );
    assert_eq!(
        native_audio_input_format(Some("audio/flac"), None),
        Ok("flac")
    );
    assert_eq!(
        native_audio_input_format(Some("audio/webm"), None),
        Ok("webm")
    );

    assert_eq!(
        native_audio_input_format(None, Some("voice.wav")),
        Ok("wav")
    );
    assert_eq!(
        native_audio_input_format(None, Some("voice.ogg")),
        Ok("ogg")
    );
    assert_eq!(
        native_audio_input_format(None, Some("voice.oga")),
        Ok("ogg")
    );
    assert_eq!(
        native_audio_input_format(None, Some("track.m4a")),
        Ok("m4a")
    );
    assert_eq!(
        native_audio_input_format(Some("application/octet-stream"), Some("voice.mp3")),
        Ok("mp3")
    );
    assert!(matches!(
        native_audio_input_format(Some("application/octet-stream"), Some("voice.bin")),
        Err(NativeAudioFormatError::Unknown)
    ));
    assert!(matches!(
        native_audio_input_format(None, None),
        Err(NativeAudioFormatError::Unknown)
    ));
}

#[test]
fn current_and_historical_native_audio_parts_share_safe_mapping() {
    let current = native_audio_input_part(b"audio", Some("audio/mpeg"), Some("clip.mp3"))
        .expect("mp3 should be native-safe");
    assert_eq!(
        current
            .get("input_audio")
            .and_then(|value| value.get("format"))
            .and_then(Value::as_str),
        Some("mp3")
    );

    let historical = historical_native_audio_input_part(b"audio", "audio/x-wav", Some("clip.wav"))
        .expect("wav should be native-safe");
    assert_eq!(
        historical
            .get("input_audio")
            .and_then(|value| value.get("format"))
            .and_then(Value::as_str),
        Some("wav")
    );

    let ogg_part = historical_native_audio_input_part(b"audio", "audio/ogg", Some("voice.ogg"))
        .expect("ogg should be native-safe");
    assert_eq!(
        ogg_part
            .get("input_audio")
            .and_then(|value| value.get("format"))
            .and_then(Value::as_str),
        Some("ogg")
    );
}

#[test]
fn audio_execution_uses_route_and_format_not_evidence() {
    fn combined(
        native: (CapabilityState, chrono::Duration),
        stt: (CapabilityState, chrono::Duration),
    ) -> CapabilityRecord {
        let mut record = evidence_record(CapabilityKind::AudioInput, native.0, native.1);
        let stt_record = evidence_record(CapabilityKind::AudioTranscription, stt.0, stt.1);
        record.evidence.extend(stt_record.evidence);
        record.supports_audio_transcription = stt_record.supports_audio_transcription;
        record
    }

    let fresh = chrono::Duration::hours(1);
    let stale = chrono::Duration::days(8);

    assert_eq!(
        select_audio_execution_mode(
            &combined(
                (CapabilityState::Supported, fresh),
                (CapabilityState::Supported, fresh),
            ),
            true,
            Some("audio/mpeg"),
            Some("clip.mp3"),
        ),
        Ok(AudioExecutionMode::Native)
    );

    assert_eq!(
        select_audio_execution_mode(
            &combined(
                (CapabilityState::Supported, fresh),
                (CapabilityState::Supported, fresh),
            ),
            true,
            Some("audio/ogg"),
            Some("voice.ogg"),
        ),
        Ok(AudioExecutionMode::Native)
    );

    assert_eq!(
        select_audio_execution_mode(
            &combined(
                (CapabilityState::Supported, fresh),
                (CapabilityState::Supported, fresh),
            ),
            true,
            Some("audio/opus"),
            Some("sample.opus"),
        ),
        Ok(AudioExecutionMode::Native)
    );

    assert_eq!(
        select_audio_execution_mode(
            &combined(
                (CapabilityState::Supported, stale),
                (CapabilityState::Supported, fresh),
            ),
            true,
            Some("audio/mpeg"),
            Some("clip.mp3"),
        ),
        Ok(AudioExecutionMode::Native)
    );

    assert_eq!(
        select_audio_execution_mode(
            &combined(
                (CapabilityState::Unsupported, fresh),
                (CapabilityState::Supported, fresh),
            ),
            true,
            Some("audio/wav"),
            Some("clip.wav"),
        ),
        Ok(AudioExecutionMode::Native)
    );

    assert_eq!(
        select_audio_execution_mode(
            &combined(
                (CapabilityState::Supported, stale),
                (CapabilityState::Supported, stale),
            ),
            true,
            Some("audio/mp3"),
            Some("clip.mp3"),
        ),
        Ok(AudioExecutionMode::Native)
    );
    for inherited_main in [false, true] {
        assert_eq!(
            select_audio_execution_mode(&CapabilityRecord::default(), inherited_main, None, None,),
            Ok(AudioExecutionMode::Transcription)
        );
    }
    assert_eq!(
        select_audio_execution_mode(
            &CapabilityRecord::default(),
            false,
            Some("audio/mp3"),
            Some("clip.mp3"),
        ),
        Ok(AudioExecutionMode::Transcription)
    );
}

#[test]
fn stream_accumulation_has_absolute_bounds() {
    let mut visible = String::new();
    assert!(push_bounded(&mut visible, "abc", 3));
    assert!(!push_bounded(&mut visible, "d", 3));
    assert_eq!(visible, "abc");

    let mut reasoning = String::new();
    assert!(push_bounded(&mut reasoning, "🧠", 4));
    assert!(!push_bounded(&mut reasoning, "x", 4));
    assert_eq!(reasoning, "🧠");
}

#[test]
fn selected_image_model_is_propagated_to_openai_images_payload() {
    let payload = ImageGenerationProtocol::OpenAiImages.payload(
        "black-forest-labs/FLUX.1-schnell",
        "galaxy",
        1024,
        1024,
    );
    assert_eq!(
        payload.get("model").and_then(Value::as_str),
        Some("black-forest-labs/FLUX.1-schnell")
    );
    assert_eq!(
        payload.get("size").and_then(Value::as_str),
        Some("1024x1024")
    );
}

#[test]
fn timeout_configuration_is_scoped_and_safely_bounded() {
    assert_eq!(
        AI_PROVIDER_CONNECT_TIMEOUT_ENV,
        "AI_PROVIDER_CONNECT_TIMEOUT_SECS"
    );
    assert_eq!(
        IMAGE_PROVIDER_CONNECT_TIMEOUT_ENV,
        "IMAGE_PROVIDER_CONNECT_TIMEOUT_SECS"
    );
    assert_eq!(
        IMAGE_GENERATION_TIMEOUT_ENV,
        "IMAGE_GENERATION_TIMEOUT_SECS"
    );
    assert_eq!(IMAGE_DOWNLOAD_TIMEOUT_ENV, "IMAGE_DOWNLOAD_TIMEOUT_SECS");
    assert_ne!(
        AI_PROVIDER_CONNECT_TIMEOUT_ENV,
        IMAGE_PROVIDER_CONNECT_TIMEOUT_ENV
    );

    assert_eq!(bounded_timeout_secs(None, 120), 120);
    assert_eq!(bounded_timeout_secs(Some("0"), 120), 120);
    assert_eq!(bounded_timeout_secs(Some("bad"), 120), 120);
    assert_eq!(bounded_timeout_secs(Some("75"), 120), 75);
    assert_eq!(bounded_timeout_secs(Some("99999"), 120), 600);
}

#[test]
fn fallback_result_retains_primary_failure_provenance() {
    let image = GeneratedImage {
        bytes: b"\x89PNG\r\n\x1a\nrest".to_vec(),
        provider_name: "Pollinations fallback".to_string(),
        model: "flux".to_string(),
        used_external_fallback: true,
        primary_failure: Some("Primary provider returned HTTP 503".to_string()),
    };
    assert!(image.used_external_fallback);
    assert_eq!(
        image.primary_failure.as_deref(),
        Some("Primary provider returned HTTP 503")
    );
}

#[test]
fn external_image_fallback_is_explicit_opt_in_only() {
    assert!(external_image_fallback_enabled("pollinations"));
    assert!(external_image_fallback_enabled(" POLLINATIONS "));
    assert!(!external_image_fallback_enabled("auto"));
    assert!(!external_image_fallback_enabled(""));
    assert!(!external_image_fallback_enabled("none"));
    assert!(!external_image_fallback_enabled("false"));
    assert!(!external_image_fallback_enabled("off"));
}

#[test]
fn specialist_payload_contains_only_the_current_user_message() {
    let payload = specialist_chat_payload(
        "vision-model",
        vec![json!({"type":"text","text":"current question"})],
    );
    let messages = payload
        .get("messages")
        .and_then(Value::as_array)
        .expect("messages array present");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].get("role").and_then(Value::as_str),
        Some("user")
    );
    assert_eq!(
        payload.get("model").and_then(Value::as_str),
        Some("vision-model")
    );
}

#[test]
fn specialist_runtime_prompt_does_not_replace_canonical_user_prompt() {
    assert_eq!(
        canonical_persisted_prompt(
            Some("what is in this image?"),
            "internal specialist synthesis"
        ),
        "what is in this image?"
    );
    assert_eq!(
        canonical_persisted_prompt(None, "ordinary chat"),
        "ordinary chat"
    );
}

#[test]
fn generated_image_base64_rejects_oversized_input_before_decode() {
    let oversized = "A".repeat(
        MAX_GENERATED_IMAGE_BYTES
            .saturating_mul(4)
            .div_ceil(3)
            .saturating_add(16),
    );
    let error = decode_generated_image_base64(&oversized).expect_err("oversized image should fail");
    assert_eq!(error.kind, ImageGenerationErrorKind::InvalidImage);
}

#[test]
fn generated_image_base64_validation_is_typed() {
    let error =
        decode_generated_image_base64("%%%not-base64%%%").expect_err("invalid base64 should fail");
    assert_eq!(error.kind, ImageGenerationErrorKind::InvalidBase64);

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nrest");
    assert!(decode_generated_image_base64(&encoded).is_ok());
}

#[test]
fn generated_image_url_validation_rejects_unsafe_schemes_and_private_ips() {
    assert_eq!(
        parse_generated_image_url("file:///etc/passwd")
            .expect_err("file url should be rejected")
            .kind,
        ImageGenerationErrorKind::UnsafeImageUrl
    );
    assert!(parse_generated_image_url("https://example.com/image.png").is_ok());
    assert!(is_unsafe_remote_ip(
        "127.0.0.1".parse().expect("valid ip literal")
    ));
    assert!(is_unsafe_remote_ip(
        "10.1.2.3".parse().expect("valid ip literal")
    ));
    assert!(is_unsafe_remote_ip(
        "100.64.0.1".parse().expect("valid ip literal")
    ));
    assert!(is_unsafe_remote_ip(
        "::1".parse().expect("valid ip literal")
    ));
    assert!(is_unsafe_remote_ip(
        "::ffff:127.0.0.1".parse().expect("valid ip literal")
    ));
    assert!(!is_unsafe_remote_ip(
        "1.1.1.1".parse().expect("valid ip literal")
    ));
}

#[test]
fn image_timeout_is_a_typed_timeout_not_unsupported() {
    let error = timeout_image_error("Image Generation Model", Duration::from_secs(120));
    assert_eq!(error.kind, ImageGenerationErrorKind::Timeout);
    assert!(error.message.contains("120"));
}

#[tokio::test]
async fn generation_cancel_signal_reaches_registered_receiver() {
    let (sender, mut receiver) = watch::channel(false);
    assert!(signal_generation_cancel(Some(sender)));
    receiver.changed().await.expect("receiver changed");
    assert!(*receiver.borrow());
}

#[test]
fn generated_image_validation_rejects_non_image_bytes() {
    assert!(validate_generated_image_bytes(b"not an image").is_err());
    assert!(validate_generated_image_bytes(b"\x89PNG\r\n\x1a\nrest").is_ok());
}

#[test]
fn image_route_errors_keep_capability_and_route_failures_distinct() {
    assert_eq!(
        classify_image_route_error("Image Generation Model is Disabled"),
        ImageGenerationErrorKind::RouteDisabled
    );
    assert_eq!(
        classify_image_route_error("Image Generation Model is explicitly Unsupported"),
        ImageGenerationErrorKind::CapabilityUnsupported
    );
    assert_eq!(
        classify_image_route_error("Image Generation Model capability is Unknown"),
        ImageGenerationErrorKind::CapabilityUnknown
    );
}

#[test]
fn audio_mime_mapping_covers_all_standard_formats() {
    let (mime, name) = resolve_audio_file_and_mime(Some("audio/ogg"), Some("voice"));
    assert_eq!(mime, "audio/ogg");
    assert_eq!(name, "voice.ogg");

    let (mime, name) = resolve_audio_file_and_mime(Some("audio/opus"), Some("note"));
    assert_eq!(mime, "audio/opus");
    assert_eq!(name, "note.opus");

    let (mime, name) = resolve_audio_file_and_mime(Some("audio/mpeg"), Some("speech"));
    assert_eq!(mime, "audio/mpeg");
    assert_eq!(name, "speech.mp3");

    let (mime, name) = resolve_audio_file_and_mime(Some("audio/mp4"), Some("recording"));
    assert_eq!(mime, "audio/mp4");
    assert_eq!(name, "recording.m4a");

    let (mime, name) = resolve_audio_file_and_mime(Some("audio/x-m4a"), Some("memo"));
    assert_eq!(mime, "audio/x-m4a");
    assert_eq!(name, "memo.m4a");

    let (mime, name) = resolve_audio_file_and_mime(Some("audio/wav"), Some("sample"));
    assert_eq!(mime, "audio/wav");
    assert_eq!(name, "sample.wav");

    let (mime, name) = resolve_audio_file_and_mime(Some("audio/x-wav"), Some("test"));
    assert_eq!(mime, "audio/wav");
    assert_eq!(name, "test.wav");

    let (mime, name) = resolve_audio_file_and_mime(Some("application/custom"), Some("data"));
    assert_eq!(mime, "application/octet-stream");
    assert_eq!(name, "data.bin");
    assert_ne!(mime, "audio/ogg");
}

#[test]
fn mime_less_audio_resolver_drives_native_and_stt_formats_truthfully() {
    let cases = [
        ("sample.mp3", "audio/mpeg", "sample.mp3"),
        ("sample.wav", "audio/wav", "sample.wav"),
        ("sample.opus", "audio/opus", "sample.opus"),
        ("sample.flac", "audio/flac", "sample.flac"),
    ];

    for (file_name, expected_mime, expected_name) in cases {
        let (mime_type, safe_name) = resolve_audio_file_and_mime(None, Some(file_name));
        assert_eq!(mime_type, expected_mime, "{file_name}");
        assert_eq!(safe_name, expected_name, "{file_name}");
    }

    assert_eq!(
        native_audio_input_format(Some("audio/mpeg"), Some("sample.mp3")),
        Ok("mp3")
    );
    assert_eq!(
        native_audio_input_format(Some("audio/wav"), Some("sample.wav")),
        Ok("wav")
    );
    assert_eq!(
        native_audio_input_format(Some("audio/opus"), Some("sample.opus")),
        Ok("opus")
    );
    assert_eq!(
        native_audio_input_format(Some("audio/flac"), Some("sample.flac")),
        Ok("flac")
    );

    let (mime_type, safe_name) =
        resolve_audio_file_and_mime(Some("audio/webm"), Some("sample.webm"));
    assert_eq!(mime_type, "audio/webm");
    assert_eq!(safe_name, "sample.webm");
}

#[test]
fn persistence_path_has_no_false_media_default() {
    let source = include_str!("generation.rs");
    let persistence_start = source
        .find("// Multimodal attachments are stored outside SQLite")
        .expect("attachment persistence block");
    let persistence_end = source[persistence_start..]
        .find("let user_message_content = encode_user_content")
        .map(|offset| persistence_start + offset)
        .expect("attachment persistence block end");
    let persistence = &source[persistence_start..persistence_end];

    assert!(!persistence.contains("mime_type.unwrap_or(\"image/jpeg\")"));
    assert!(!persistence.contains("video_mime.unwrap_or(\"video/mp4\")"));
    assert!(!persistence.contains("audio_mime.unwrap_or(\"application/octet-stream\")"));
    assert!(persistence.contains("resolved_audio_persistence_mime(audio_mime, doc_name)"));
}

fn isolated_service(provider: ProviderConfig) -> AIChatService {
    AIChatService {
        client: Client::builder()
            .no_proxy()
            .build()
            .expect("build client succeeds"),
        user_sessions: Default::default(),
        active_session_id: Default::default(),
        generation_locks: Default::default(),
        session_locks: Default::default(),
        active_generations: Default::default(),
        provider_store: Arc::new(RwLock::new(ProviderStore {
            active_id: Some(provider.id.clone()),
            providers: vec![provider],
        })),
        capability_registry: Default::default(),
        model_routing: Default::default(),
        model_metadata: Default::default(),
    }
}

#[tokio::test]
async fn transcription_uses_selected_transport_without_probe_or_real_state() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener succeeds");
    let address = listener.local_addr().expect("local_addr succeeds");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for status in ["200 OK", "415 Unsupported Media Type"] {
            let (mut socket, _) = listener.accept().await.expect("accept socket succeeds");
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0u8; 4096];
                let count = socket
                    .read(&mut buffer)
                    .await
                    .expect("read socket succeeds");
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                assert!(bytes.len() < 128 * 1024);
                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length").then(|| {
                                value.trim().parse::<usize>().expect("valid content-length")
                            })
                        })
                        .expect("content-length header found");
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).expect("valid utf8 request body"));
            let body = r#"{"text":"sample transcript"}"#;
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response succeeds");
        }
        requests
    });
    let provider = ProviderConfig {
        id: "selected".into(),
        name: "Selected".into(),
        endpoint: format!("http://{address}/v1"),
        api_key: String::new(),
        api_key_ref: None,
        models: vec!["new-model".into()],
        active_model: "new-model".into(),
    };
    let service = isolated_service(provider.clone());
    let snapshot = service.generation_model_snapshot().await;
    let route = AIChatService::resolve_model_route_from_snapshot(&snapshot, ModelRole::AudioStt)
        .expect("resolve audio stt route succeeds");
    {
        let mut live = service.provider_store.write().await;
        live.providers[0].endpoint = "http://127.0.0.1:1/changed".into();
        live.providers[0].active_model = "later-model".into();
    }
    assert_eq!(
        service
            .transcribe_audio_resolved(
                &route,
                b"sample".to_vec(),
                "sample.mp3",
                Some("audio/mpeg"),
            )
            .await
            .expect("transcribe audio succeeds"),
        "sample transcript"
    );
    let error = service
        .transcribe_audio_resolved(&route, b"sample".to_vec(), "sample.mp3", Some("audio/mpeg"))
        .await
        .expect_err("unconfigured audio provider should fail");
    assert!(error.contains("415"));
    assert!(!error.contains("probe"));
    assert!(service.capability_registry.read().await.models.is_empty());
    let requests = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server timeout not exceeded")
        .expect("server task join succeeds");
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert!(request.starts_with("POST /v1/audio/transcriptions "));
        assert!(request.contains("new-model"));
        assert!(!request.contains("later-model"));
    }
    let mut disabled = snapshot.clone();
    disabled.routing.audio_stt = ModelRoute::Disabled;
    disabled.routing.image_gen = ModelRoute::Disabled;
    assert!(
        AIChatService::resolve_model_route_from_snapshot(&disabled, ModelRole::AudioStt,).is_err()
    );
    let (_cancel, mut receiver) = watch::channel(false);
    let error = service
        .generate_image_with_snapshot(0, "test", 64, 64, &disabled, &mut receiver)
        .await
        .expect_err("disabled route should fail");
    assert_eq!(error.kind, ImageGenerationErrorKind::RouteDisabled);
    assert!(service
        .transcribe_audio_resolved(&route, vec![], "unsafe.bin", None,)
        .await
        .is_err());
}

#[test]
fn main_route_snapshot_keeps_provider_model_and_capability_stable() {
    let route = ResolvedModelRoute {
        provider: ProviderConfig {
            id: "prov-a".to_string(),
            name: "Provider A".to_string(),
            endpoint: "https://a.example/v1".to_string(),
            api_key: String::new(),
            api_key_ref: None,
            models: vec!["model-a".to_string()],
            active_model: "model-a".to_string(),
        },
        model: "model-a".to_string(),
        capability: CapabilityRecord {
            provider_id: "https://a.example/v1".to_string(),
            model: "model-a".to_string(),
            supports_text_chat: Some(true),
            ..CapabilityRecord::default()
        },
        route_origin: RouteOrigin::Main,
    };
    assert_eq!(route.provider.id, "prov-a");
    assert_eq!(route.model, "model-a");
    assert_eq!(route.capability.supports_text_chat, Some(true));
}

#[tokio::test]
async fn generation_guard_removes_active_generation_on_drop() {
    let active: ActiveGenerations = Arc::new(RwLock::new(HashMap::new()));
    let (tx, _rx) = tokio::sync::watch::channel(false);
    active.write().await.insert((123, 456), tx);

    {
        let _guard = GenerationGuard::new(active.clone(), 123, 456);
        assert!(active.read().await.contains_key(&(123, 456)));
    }

    assert!(!active.read().await.contains_key(&(123, 456)));
}

#[tokio::test]
async fn generation_guard_cleans_up_on_drop() {
    let active: ActiveGenerations = Arc::new(RwLock::new(HashMap::new()));
    let (tx, _rx) = tokio::sync::watch::channel(false);
    active.write().await.insert((123, 789), tx);

    {
        let _guard = GenerationGuard::new(active.clone(), 123, 789);
    }

    assert!(!active.read().await.contains_key(&(123, 789)));
}

#[test]
fn next_draft_id_is_strictly_monotonic() {
    let id1 = next_draft_id();
    let id2 = next_draft_id();
    let id3 = next_draft_id();
    assert!(id2 > id1);
    assert!(id3 > id2);
}

#[test]
fn protocol_selection_heuristic_identifies_chat_models() {
    assert_eq!(
        select_initial_image_protocol("gemini-3.1-flash-image"),
        ImageGenerationProtocol::ChatCompletionsMultimodal
    );
    assert_eq!(
        select_initial_image_protocol("gemini-2.5-flash-image-preview"),
        ImageGenerationProtocol::ChatCompletionsMultimodal
    );
    assert_eq!(
        select_initial_image_protocol("imagen-3.0-generate-002"),
        ImageGenerationProtocol::ChatCompletionsMultimodal
    );
    assert_eq!(
        select_initial_image_protocol("dall-e-3"),
        ImageGenerationProtocol::OpenAiImages
    );
    assert_eq!(
        select_initial_image_protocol("flux-schnell"),
        ImageGenerationProtocol::OpenAiImages
    );
}

#[test]
fn dedicated_image_generation_model_classifier_works() {
    assert!(is_dedicated_image_generation_model(
        "gemini-3.1-flash-image"
    ));
    assert!(is_dedicated_image_generation_model(
        "gemini-2.5-flash-image-preview"
    ));
    assert!(is_dedicated_image_generation_model("gpt-image-2"));
    assert!(is_dedicated_image_generation_model("gpt-image-1.5"));
    assert!(is_dedicated_image_generation_model("grok-imagine-image"));
    assert!(is_dedicated_image_generation_model(
        "grok-imagine-image-quality"
    ));
    assert!(is_dedicated_image_generation_model("dall-e-3"));
    assert!(is_dedicated_image_generation_model("imagen-3"));
    assert!(is_dedicated_image_generation_model("flux-pro"));
    assert!(is_dedicated_image_generation_model("stable-diffusion-xl"));

    assert!(!is_dedicated_image_generation_model(
        "gemini-3.8-flash-high"
    ));
    assert!(!is_dedicated_image_generation_model("gemini-3-flash"));
    assert!(!is_dedicated_image_generation_model("claude-sonnet-4-6"));
    assert!(!is_dedicated_image_generation_model(
        "claude-opus-4-6-thinking"
    ));
    assert!(!is_dedicated_image_generation_model("gpt-5.5"));
    assert!(!is_dedicated_image_generation_model("gpt-6-sol"));
}

#[test]
fn chat_completions_multimodal_extracts_images_array() {
    let body = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "images": [{
                    "image_url": {
                        "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
                    }
                }]
            }
        }]
    });
    let source = extract_image_from_chat_response(&body).expect("extract succeeds");
    match source {
        ExtractedImageSource::Base64(b64) => {
            assert!(b64.starts_with("iVBORw0KGgoAAA"));
        }
        ExtractedImageSource::Url(_) => panic!("expected base64"),
    }
}

#[test]
fn chat_completions_multimodal_extracts_markdown_base64() {
    let body = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Ini gambar yang kamu minta:\n\n![hasil gambar](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==)\n\nSemoga suka!"
            }
        }]
    });
    let source = extract_image_from_chat_response(&body).expect("extract succeeds");
    match source {
        ExtractedImageSource::Base64(b64) => {
            assert!(b64.starts_with("iVBORw0KGgoAAA"));
        }
        ExtractedImageSource::Url(_) => panic!("expected base64"),
    }
}

#[test]
fn chat_completions_multimodal_extracts_direct_data_uri() {
    let body = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
            }
        }]
    });
    let source = extract_image_from_chat_response(&body).expect("extract succeeds");
    match source {
        ExtractedImageSource::Base64(b64) => {
            assert!(b64.starts_with("iVBORw0KGgoAAA"));
        }
        ExtractedImageSource::Url(_) => panic!("expected base64"),
    }
}

#[tokio::test]
async fn image_generation_adaptive_fallback_on_400() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener succeeds");
    let address = listener.local_addr().expect("local_addr succeeds");

    let server = tokio::spawn(async move {
        let mut request_paths = Vec::new();

        // Request 1: /images/generations -> return 400 Bad Request unsupported endpoint
        {
            let (mut socket, _) = listener.accept().await.expect("accept 1 succeeds");
            let mut buffer = [0u8; 4096];
            let count = socket.read(&mut buffer).await.expect("read 1 succeeds");
            let req_str = String::from_utf8_lossy(&buffer[..count]);
            let first_line = req_str.lines().next().unwrap_or_default().to_string();
            request_paths.push(first_line);

            let body = r#"{"error":{"message":"Endpoint /images/generations is not supported for this model"}}"#;
            let response = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write 1 succeeds");
        }

        // Request 2: /chat/completions -> return 200 OK with images array
        {
            let (mut socket, _) = listener.accept().await.expect("accept 2 succeeds");
            let mut buffer = [0u8; 4096];
            let count = socket.read(&mut buffer).await.expect("read 2 succeeds");
            let req_str = String::from_utf8_lossy(&buffer[..count]);
            let first_line = req_str.lines().next().unwrap_or_default().to_string();
            request_paths.push(first_line);

            let body = r#"{"choices":[{"message":{"images":[{"image_url":{"url":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="}}]}}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write 2 succeeds");
        }

        request_paths
    });

    let provider = ProviderConfig {
        id: "adaptive-test".into(),
        name: "Adaptive Test".into(),
        endpoint: format!("http://{address}/v1"),
        api_key: String::new(),
        api_key_ref: None,
        models: vec!["custom-image-model".into()],
        active_model: "custom-image-model".into(),
    };
    let service = isolated_service(provider.clone());
    let mut snapshot = service.generation_model_snapshot().await;
    snapshot.routing.image_gen = ModelRoute::MainModel;

    let (_cancel, mut receiver) = watch::channel(false);
    let result = service
        .generate_image_with_snapshot(0, "a cute kitten", 1024, 1024, &snapshot, &mut receiver)
        .await
        .expect("image generation adaptive fallback succeeds");

    assert_eq!(result.provider_name, "Adaptive Test");
    assert_eq!(result.model, "custom-image-model");
    assert!(!result.used_external_fallback);
    assert!(!result.bytes.is_empty());

    let paths = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server completed")
        .expect("server join succeeds");

    assert_eq!(paths.len(), 2);
    assert!(paths[0].contains("/images/generations"));
    assert!(paths[1].contains("/chat/completions"));
}

#[test]
fn test_create_quiz_sanitization_and_validation() {
    use crate::ai::tools::CreateQuizArgs;

    let mut args = CreateQuizArgs {
        question: "  ".to_string() + &"Q".repeat(350) + "  ",
        options: (0..15)
            .map(|i| "  ".to_string() + &"Opt".repeat(40) + &format!("_{i}"))
            .collect(),
        correct_option_id: Some(99),
        explanation: Some(
            "  Line 1\nLine 2\nLine 3\nLine 4\nLine 5  ".to_string() + &"E".repeat(250),
        ),
        preamble: Some("  Preamble text  ".to_string()),
        is_anonymous: None,
    };

    args.sanitize();

    // Question truncated to 300
    assert_eq!(args.question.chars().count(), 300);

    // Options truncated to 10
    assert_eq!(args.options.len(), 10);

    // Each option text truncated to 100
    for opt in &args.options {
        assert!(opt.chars().count() <= 100);
    }

    // correct_option_id clamped to valid range 0..10
    assert_eq!(args.correct_option_id, Some(9));

    // Explanation truncated to <= 200 chars and <= 2 line breaks
    let exp = args.explanation.as_ref().expect("explanation present");
    assert!(exp.chars().count() <= 200);
    let line_breaks = exp.chars().filter(|&c| c == '\n').count();
    assert!(line_breaks <= 2);

    // Preamble trimmed
    assert_eq!(args.preamble.as_deref(), Some("Preamble text"));

    // Validation passes after sanitization
    assert!(args.validate().is_ok());

    // Validation fails if options < 2
    let mut invalid = args.clone();
    invalid.options = vec!["Single".to_string()];
    assert!(invalid.validate().is_err());

    // Validation fails if question is empty
    let mut invalid_q = args.clone();
    invalid_q.question = "".to_string();
    assert!(invalid_q.validate().is_err());

    // Explanation with CRLF and > 2 line breaks is normalized without stray \r
    let mut crlf_args = args.clone();
    crlf_args.explanation = Some("Line 1\r\nLine 2\r\nLine 3\r\nLine 4\r\nLine 5".to_string());
    crlf_args.sanitize();
    let crlf_exp = crlf_args.explanation.as_ref().expect("explanation present");
    assert!(!crlf_exp.contains('\r'));
    assert_eq!(crlf_exp.chars().filter(|&c| c == '\n').count(), 2);
    assert!(crlf_exp.contains("Line 3 Line 4 Line 5"));

    // Explanation with only whitespace is sanitized to None
    let mut whitespace_exp_args = args.clone();
    whitespace_exp_args.explanation = Some("    \n\t  ".to_string());
    whitespace_exp_args.sanitize();
    assert!(whitespace_exp_args.explanation.is_none());
}

#[tokio::test]
async fn test_create_quiz_without_preamble_sends_single_bubble_and_emits_sink() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // 1. Mock Telegram Server
    let tg_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind tg listener succeeds");
    let tg_address = tg_listener.local_addr().expect("tg addr succeeds");

    let tg_server = tokio::spawn(async move {
        let (mut socket, _) = tg_listener.accept().await.expect("accept tg 1 succeeds");
        let mut data = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = socket.read(&mut buf).await.expect("read chunk");
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            if data.windows(4).any(|w| w == b"\r\n\r\n") {
                // Read content-length if any
                let header_str = String::from_utf8_lossy(&data);
                let content_len = header_str
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                    .and_then(|l| l.split(':').nth(1))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let body_pos = data
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|p| p + 4)
                    .unwrap_or(0);
                if data.len() >= body_pos + content_len {
                    break;
                }
            }
        }
        let req_str = String::from_utf8_lossy(&data).to_string();
        let first_line = req_str.lines().next().unwrap_or("").to_string();
        let body_pos = data
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .unwrap_or(0);
        let payload: Value = serde_json::from_slice(&data[body_pos..]).unwrap_or(json!({}));

        let resp_body = r#"{"ok":true,"result":{"message_id":7001,"poll":{"id":"poll_7001"}}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
            resp_body.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write tg resp");

        vec![(first_line, payload)]
    });

    // 2. Mock AI LLM Server
    let ai_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ai listener succeeds");
    let ai_address = ai_listener.local_addr().expect("ai addr succeeds");

    let ai_server = tokio::spawn(async move {
        let (mut socket, _) = ai_listener.accept().await.expect("accept ai succeeds");
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.expect("read ai request");
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }

        let sse = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_quiz_nopre\",\"type\":\"function\",\"function\":{\"name\":\"create_quiz\",\"arguments\":\"{\\\"question\\\":\\\"Berapa 1+1?\\\",\\\"options\\\":[\\\"1\\\",\\\"2\\\"],\\\"correct_option_id\\\":1}\"}}]}}]}\n\ndata: [DONE]\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",
            sse.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write sse");
        let _ = socket.shutdown().await;
    });

    let bot_client = crate::bot::client::TelegramBotClient::with_base_url(
        "test_tok",
        format!("http://{tg_address}"),
    );
    let provider = ProviderConfig {
        id: "main-quiz-test".into(),
        name: "Main Quiz Test".into(),
        endpoint: format!("http://{ai_address}/v1"),
        api_key: String::new(),
        api_key_ref: None,
        models: vec!["quiz-model".into()],
        active_model: "quiz-model".into(),
    };
    let service = isolated_service(provider.clone());
    let snapshot = service.generation_model_snapshot().await;

    struct TestProgressSink {
        actions: std::sync::Mutex<Vec<(String, Option<crate::timeline::ProgressActivity>)>>,
    }
    impl crate::timeline::GenerationProgressSink for TestProgressSink {
        fn on_action(&self, label: &str, activity: Option<crate::timeline::ProgressActivity>) {
            if let Ok(mut l) = self.actions.lock() {
                l.push((label.to_string(), activity));
            }
        }
        fn on_partial_answer(&self, _text: &str) {}
        fn on_failure(&self, _error: &str, _force_sync: bool) {}
        fn on_complete(&self) {}
    }

    let sink = TestProgressSink {
        actions: std::sync::Mutex::new(Vec::new()),
    };

    let (_cancel, mut receiver) = watch::channel(false);
    let gen_input = GenerationInput {
        prompt: "Buatkan kuis 1+1",
        canonical_prompt: None,
        media_to_main: true,
        sink: Some(&sink),
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
        bot: Some(bot_client),
        reply_to_message_id: Some(100),
    };

    let (_thinking, answer, _staged_docs, cancelled) = service
        .generate_response_with_snapshot(1234, 0, 1234, gen_input, &snapshot, &mut receiver)
        .await;

    assert!(!cancelled);
    assert_eq!(answer, "[QUIZ_SENT]");

    // Verify progress sink emitted "Quiz"
    let recorded_actions = sink.actions.lock().expect("lock actions").clone();
    assert!(
        recorded_actions.iter().any(
            |(lbl, act)| lbl == "Quiz" && *act == Some(crate::timeline::ProgressActivity::Quiz)
        ),
        "Progress sink must emit Quiz action"
    );

    // Verify exactly 1 request to Telegram (sendPoll)
    let tg_requests = tg_server.await.expect("tg server join");
    assert_eq!(
        tg_requests.len(),
        1,
        "Quiz without preamble must send exactly 1 bubble"
    );
    let (req_line, payload) = &tg_requests[0];
    assert!(req_line.contains("POST /sendPoll"));
    assert_eq!(payload["type"], "quiz");
    assert_eq!(payload["question"], "Berapa 1+1?");
    assert_eq!(payload["correct_option_id"], 1);

    // Verify session history recorded both the user prompt and the assistant quiz summary
    let messages = crate::ai::storage::load_scoped_messages_async(1234, 0, 10).await;
    assert!(
        messages
            .iter()
            .any(|m| m.role == "user" && m.content.to_string().contains("Buatkan kuis 1+1")),
        "User prompt must be saved in session messages"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.role == "assistant" && m.content.to_string().contains("Berapa 1+1?")),
        "Assistant quiz summary must be saved in session messages"
    );

    ai_server.await.expect("ai server join");
}

#[tokio::test]
async fn test_create_quiz_with_preamble_sends_two_connected_messages() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // 1. Mock Telegram Server (handles sendRichMessage, then sendPoll)
    let tg_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind tg listener succeeds");
    let tg_address = tg_listener.local_addr().expect("tg addr succeeds");

    let tg_server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for req_idx in 0..2 {
            let (mut socket, _) = tg_listener.accept().await.expect("accept tg succeeds");
            let mut data = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.expect("read chunk");
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                if data.windows(4).any(|w| w == b"\r\n\r\n") {
                    let header_str = String::from_utf8_lossy(&data);
                    let content_len = header_str
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let body_pos = data
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .map(|p| p + 4)
                        .unwrap_or(0);
                    if data.len() >= body_pos + content_len {
                        break;
                    }
                }
            }
            let req_str = String::from_utf8_lossy(&data).to_string();
            let first_line = req_str.lines().next().unwrap_or("").to_string();
            let body_pos = data
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|p| p + 4)
                .unwrap_or(0);
            let payload: Value = serde_json::from_slice(&data[body_pos..]).unwrap_or(json!({}));

            let msg_id = if req_idx == 0 { 8001 } else { 8002 };
            let resp_body = format!(r#"{{"ok":true,"result":{{"message_id":{msg_id}}}}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
                resp_body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write tg resp");
            requests.push((first_line, payload));
        }

        requests
    });

    // 2. Mock AI LLM Server
    let ai_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ai listener succeeds");
    let ai_address = ai_listener.local_addr().expect("ai addr succeeds");

    let ai_server = tokio::spawn(async move {
        let (mut socket, _) = ai_listener.accept().await.expect("accept ai succeeds");
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.expect("read ai request");
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }

        let sse = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_quiz_pre\",\"type\":\"function\",\"function\":{\"name\":\"create_quiz\",\"arguments\":\"{\\\"preamble\\\":\\\"Perhatikan cuplikan kode ini:\\\\n```rust\\\\nfn main() {}\\\\n```\\\",\\\"question\\\":\\\"Berapakah outputnya?\\\",\\\"options\\\":[\\\"0\\\",\\\"1\\\"],\\\"correct_option_id\\\":0}\"}}]}}]}\n\ndata: [DONE]\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",
            sse.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write sse");
        let _ = socket.shutdown().await;
    });

    let bot_client = crate::bot::client::TelegramBotClient::with_base_url(
        "test_tok",
        format!("http://{tg_address}"),
    );
    let provider = ProviderConfig {
        id: "main-quiz-pre-test".into(),
        name: "Main Quiz Pre Test".into(),
        endpoint: format!("http://{ai_address}/v1"),
        api_key: String::new(),
        api_key_ref: None,
        models: vec!["quiz-model".into()],
        active_model: "quiz-model".into(),
    };
    let service = isolated_service(provider.clone());
    let snapshot = service.generation_model_snapshot().await;

    let (_cancel, mut receiver) = watch::channel(false);
    let gen_input = GenerationInput {
        prompt: "Buatkan kuis dengan studi kasus",
        canonical_prompt: None,
        media_to_main: true,
        sink: None,
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
        bot: Some(bot_client),
        reply_to_message_id: Some(100),
    };

    let (_thinking, answer, _staged_docs, cancelled) = service
        .generate_response_with_snapshot(1234, 0, 1234, gen_input, &snapshot, &mut receiver)
        .await;

    assert!(!cancelled);
    assert_eq!(answer, "[QUIZ_SENT]");

    // Verify exactly 2 requests to Telegram connected by reply_parameters
    let tg_requests = tg_server.await.expect("tg server join");
    assert_eq!(
        tg_requests.len(),
        2,
        "Quiz with preamble must send 2 connected messages"
    );

    // Message 1: Preamble (sendRichMessage)
    let (req1_line, payload1) = &tg_requests[0];
    assert!(req1_line.contains("POST /sendRichMessage"));
    assert_eq!(payload1["chat_id"], 1234);

    // Message 2: Native Quiz (sendPoll) linked to message_id 8001
    let (req2_line, payload2) = &tg_requests[1];
    assert!(req2_line.contains("POST /sendPoll"));
    assert_eq!(payload2["type"], "quiz");
    assert_eq!(payload2["question"], "Berapakah outputnya?");
    assert_eq!(
        payload2["reply_parameters"]["message_id"], 8001,
        "Quiz must be linked to preamble message_id"
    );

    // Verify session history recorded both user prompt and assistant preamble+quiz
    let messages = crate::ai::storage::load_scoped_messages_async(1234, 0, 10).await;
    assert!(
        messages.iter().any(|m| m.role == "user"
            && m.content
                .to_string()
                .contains("Buatkan kuis dengan studi kasus")),
        "User prompt must be saved in session messages"
    );
    assert!(
        messages.iter().any(|m| m.role == "assistant"
            && m.content
                .to_string()
                .contains("Perhatikan cuplikan kode ini")
            && m.content.to_string().contains("Berapakah outputnya?")),
        "Assistant quiz summary must contain both preamble and quiz question"
    );

    ai_server.await.expect("ai server join");
}

#[tokio::test]
async fn test_create_quiz_in_forum_topic_preserves_thread_id() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let tg_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind tg listener succeeds");
    let tg_address = tg_listener.local_addr().expect("tg addr succeeds");

    let tg_server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for req_idx in 0..2 {
            let (mut socket, _) = tg_listener.accept().await.expect("accept tg succeeds");
            let mut data = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.expect("read chunk");
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                if data.windows(4).any(|w| w == b"\r\n\r\n") {
                    let header_str = String::from_utf8_lossy(&data);
                    let content_len = header_str
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let body_pos = data
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .map(|p| p + 4)
                        .unwrap_or(0);
                    if data.len() >= body_pos + content_len {
                        break;
                    }
                }
            }
            let req_str = String::from_utf8_lossy(&data).to_string();
            let first_line = req_str.lines().next().unwrap_or("").to_string();
            let body_pos = data
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|p| p + 4)
                .unwrap_or(0);
            let payload: Value = serde_json::from_slice(&data[body_pos..]).unwrap_or(json!({}));

            let msg_id = if req_idx == 0 { 8801 } else { 8802 };
            let resp_body = format!(r#"{{"ok":true,"result":{{"message_id":{msg_id}}}}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
                resp_body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write tg resp");
            requests.push((first_line, payload));
        }

        requests
    });

    let ai_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ai listener succeeds");
    let ai_address = ai_listener.local_addr().expect("ai addr succeeds");

    let ai_server = tokio::spawn(async move {
        let (mut socket, _) = ai_listener.accept().await.expect("accept ai succeeds");
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.expect("read ai request");
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }

        let sse = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_quiz_topic\",\"type\":\"function\",\"function\":{\"name\":\"create_quiz\",\"arguments\":\"{\\\"preamble\\\":\\\"Materi Topik:\\\\nInfo topik forum\\\",\\\"question\\\":\\\"Pertanyaan Topik?\\\",\\\"options\\\":[\\\"A\\\",\\\"B\\\"],\\\"correct_option_id\\\":0}\"}}]}}]}\n\ndata: [DONE]\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",
            sse.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write sse");
        let _ = socket.shutdown().await;
    });

    let bot_client = crate::bot::client::TelegramBotClient::with_base_url(
        "test_tok",
        format!("http://{tg_address}"),
    );
    let provider = ProviderConfig {
        id: "main-quiz-topic-test".into(),
        name: "Main Quiz Topic Test".into(),
        endpoint: format!("http://{ai_address}/v1"),
        api_key: String::new(),
        api_key_ref: None,
        models: vec!["quiz-model".into()],
        active_model: "quiz-model".into(),
    };
    let service = isolated_service(provider.clone());
    let snapshot = service.generation_model_snapshot().await;

    let (_cancel, mut receiver) = watch::channel(false);
    let gen_input = GenerationInput {
        prompt: "Buat kuis dalam topik forum",
        canonical_prompt: None,
        media_to_main: true,
        sink: None,
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
        bot: Some(bot_client),
        reply_to_message_id: Some(100),
    };

    // thread_id = 9988 represents a Telegram forum topic
    let (_thinking, answer, _staged_docs, cancelled) = service
        .generate_response_with_snapshot(1234, 9988, 1234, gen_input, &snapshot, &mut receiver)
        .await;

    assert!(!cancelled);
    assert_eq!(answer, "[QUIZ_SENT]");

    let tg_requests = tg_server.await.expect("tg server join");
    assert_eq!(
        tg_requests.len(),
        2,
        "Quiz with preamble must send 2 connected messages"
    );

    // Message 1: Preamble must include message_thread_id = 9988
    let (req1_line, payload1) = &tg_requests[0];
    assert!(req1_line.contains("POST /sendRichMessage"));
    assert_eq!(
        payload1["message_thread_id"], 9988,
        "Preamble must preserve message_thread_id"
    );

    // Message 2: sendPoll must include message_thread_id = 9988 AND reply to preamble 8801
    let (req2_line, payload2) = &tg_requests[1];
    assert!(req2_line.contains("POST /sendPoll"));
    assert_eq!(
        payload2["message_thread_id"], 9988,
        "sendPoll must preserve message_thread_id"
    );
    assert_eq!(
        payload2["reply_parameters"]["message_id"], 8801,
        "sendPoll must link to preamble message_id"
    );

    ai_server.await.expect("ai server join");
}

#[tokio::test]
async fn test_create_quiz_deletes_orphan_preamble_if_poll_fails() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let tg_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind tg listener succeeds");
    let tg_address = tg_listener.local_addr().expect("tg addr succeeds");

    let tg_server = tokio::spawn(async move {
        let mut requests = Vec::new();
        // 3 requests expected: sendRichMessage (succeeds), sendPoll (fails 400), deleteMessage (cleans up preamble)
        for req_idx in 0..3 {
            let (mut socket, _) = tg_listener.accept().await.expect("accept tg succeeds");
            let mut data = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.expect("read chunk");
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                if data.windows(4).any(|w| w == b"\r\n\r\n") {
                    let header_str = String::from_utf8_lossy(&data);
                    let content_len = header_str
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let body_pos = data
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .map(|p| p + 4)
                        .unwrap_or(0);
                    if data.len() >= body_pos + content_len {
                        break;
                    }
                }
            }
            let req_str = String::from_utf8_lossy(&data).to_string();
            let first_line = req_str.lines().next().unwrap_or("").to_string();
            let body_pos = data
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|p| p + 4)
                .unwrap_or(0);
            let payload: Value = serde_json::from_slice(&data[body_pos..]).unwrap_or(json!({}));

            let (status, resp_body) = if req_idx == 0 {
                // 1. sendRichMessage succeeds with message_id 9001
                (
                    "200 OK",
                    r#"{"ok":true,"result":{"message_id":9001}}"#.to_string(),
                )
            } else if req_idx == 1 {
                // 2. sendPoll fails with 400 Bad Request
                (
                    "400 Bad Request",
                    r#"{"ok":false,"error_code":400,"description":"Bad Request: poll failed"}"#
                        .to_string(),
                )
            } else {
                // 3. deleteMessage succeeds
                ("200 OK", r#"{"ok":true,"result":true}"#.to_string())
            };

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
                resp_body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write tg resp");
            requests.push((first_line, payload));
        }

        requests
    });

    let ai_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ai listener succeeds");
    let ai_address = ai_listener.local_addr().expect("ai addr succeeds");

    let ai_server = tokio::spawn(async move {
        // AI turn 1: generates create_quiz tool call
        let (mut socket, _) = ai_listener.accept().await.expect("accept ai succeeds");
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.expect("read ai request");
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }

        let sse = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_quiz_fail\",\"type\":\"function\",\"function\":{\"name\":\"create_quiz\",\"arguments\":\"{\\\"preamble\\\":\\\"Preamble yang akan dibersihkan\\\",\\\"question\\\":\\\"Pertanyaan?\\\",\\\"options\\\":[\\\"A\\\",\\\"B\\\"],\\\"correct_option_id\\\":0}\"}}]}}]}\n\ndata: [DONE]\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",
            sse.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write sse");
        let _ = socket.shutdown().await;

        // AI turn 2: model responds to tool error
        let (mut socket2, _) = ai_listener.accept().await.expect("accept ai 2 succeeds");
        let mut bytes2 = Vec::new();
        loop {
            let mut buf = [0u8; 4096];
            let n = socket2.read(&mut buf).await.expect("read ai request 2");
            if n == 0 {
                break;
            }
            bytes2.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes2.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes2[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if bytes2.len() >= end + 4 + length {
                    break;
                }
            }
        }

        let sse2 = "data: {\"choices\":[{\"delta\":{\"content\":\"Maaf kuis gagal dikirim.\"}}]}\n\ndata: [DONE]\n\n";
        let response2 = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse2}",
            sse2.len()
        );
        socket2
            .write_all(response2.as_bytes())
            .await
            .expect("write sse 2");
        let _ = socket2.shutdown().await;
    });

    let bot_client = crate::bot::client::TelegramBotClient::with_base_url(
        "test_tok",
        format!("http://{tg_address}"),
    );
    let provider = ProviderConfig {
        id: "main-quiz-orphan-test".into(),
        name: "Main Quiz Orphan Test".into(),
        endpoint: format!("http://{ai_address}/v1"),
        api_key: String::new(),
        api_key_ref: None,
        models: vec!["quiz-model".into()],
        active_model: "quiz-model".into(),
    };
    let service = isolated_service(provider.clone());
    let snapshot = service.generation_model_snapshot().await;

    let (_cancel, mut receiver) = watch::channel(false);
    let gen_input = GenerationInput {
        prompt: "Buatkan kuis uji pembersihan preamble",
        canonical_prompt: None,
        media_to_main: true,
        sink: None,
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
        bot: Some(bot_client),
        reply_to_message_id: None,
    };

    let (_thinking, answer, _staged_docs, cancelled) = service
        .generate_response_with_snapshot(1234, 0, 1234, gen_input, &snapshot, &mut receiver)
        .await;

    assert!(!cancelled);
    assert!(answer.contains("Maaf kuis gagal dikirim"));

    let tg_requests = tg_server.await.expect("tg server join");
    assert_eq!(
        tg_requests.len(),
        3,
        "Must send preamble, attempt poll, and delete orphan preamble on poll failure"
    );

    // Request 1: sendRichMessage
    assert!(tg_requests[0].0.contains("POST /sendRichMessage"));
    // Request 2: sendPoll
    assert!(tg_requests[1].0.contains("POST /sendPoll"));
    // Request 3: deleteMessage with message_id = 9001
    assert!(tg_requests[2].0.contains("POST /deleteMessage"));
    assert_eq!(
        tg_requests[2].1["message_id"], 9001,
        "Must delete orphan preamble message_id 9001"
    );

    ai_server.await.expect("ai server join");
}

#[tokio::test]
async fn test_create_quiz_aborts_if_preamble_fails_and_does_not_send_poll() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let tg_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind tg listener succeeds");
    let tg_address = tg_listener.local_addr().expect("tg addr succeeds");

    let tg_server = tokio::spawn(async move {
        let mut requests = Vec::new();
        // We only expect 1 request: sendRichMessage (which fails). sendPoll must NOT be called.
        let (mut socket, _) = tg_listener.accept().await.expect("accept tg succeeds");
        let mut data = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = socket.read(&mut buf).await.expect("read chunk");
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            if data.windows(4).any(|w| w == b"\r\n\r\n") {
                let header_str = String::from_utf8_lossy(&data);
                let content_len = header_str
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                    .and_then(|l| l.split(':').nth(1))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let body_pos = data
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|p| p + 4)
                    .unwrap_or(0);
                if data.len() >= body_pos + content_len {
                    break;
                }
            }
        }
        let req_str = String::from_utf8_lossy(&data).to_string();
        let first_line = req_str.lines().next().unwrap_or("").to_string();
        let body_pos = data
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .unwrap_or(0);
        let payload: Value = serde_json::from_slice(&data[body_pos..]).unwrap_or(json!({}));

        // sendRichMessage fails with 400 Bad Request
        let resp_body =
            r#"{"ok":false,"error_code":400,"description":"Bad Request: preamble failed"}"#;
        let response = format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
            resp_body.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write tg resp");
        requests.push((first_line, payload));

        requests
    });

    let ai_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ai listener succeeds");
    let ai_address = ai_listener.local_addr().expect("ai addr succeeds");

    let ai_server = tokio::spawn(async move {
        // AI turn 1: generates create_quiz tool call with preamble
        let (mut socket, _) = ai_listener.accept().await.expect("accept ai succeeds");
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.expect("read ai request");
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }

        let sse = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_preamble_fail\",\"type\":\"function\",\"function\":{\"name\":\"create_quiz\",\"arguments\":\"{\\\"preamble\\\":\\\"Preamble gagal kirim\\\",\\\"question\\\":\\\"Pertanyaan?\\\",\\\"options\\\":[\\\"A\\\",\\\"B\\\"],\\\"correct_option_id\\\":0}\"}}]}}]}\n\ndata: [DONE]\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",
            sse.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write sse");
        let _ = socket.shutdown().await;

        // AI turn 2: model receives preamble error and apologizes
        let (mut socket2, _) = ai_listener.accept().await.expect("accept ai 2 succeeds");
        let mut bytes2 = Vec::new();
        loop {
            let mut buf = [0u8; 4096];
            let n = socket2.read(&mut buf).await.expect("read ai request 2");
            if n == 0 {
                break;
            }
            bytes2.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes2.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes2[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if bytes2.len() >= end + 4 + length {
                    break;
                }
            }
        }

        let sse2 = "data: {\"choices\":[{\"delta\":{\"content\":\"Maaf, pesan pengantar kuis gagal terkirim.\"}}]}\n\ndata: [DONE]\n\n";
        let response2 = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse2}",
            sse2.len()
        );
        socket2
            .write_all(response2.as_bytes())
            .await
            .expect("write sse 2");
        let _ = socket2.shutdown().await;
    });

    let bot_client = crate::bot::client::TelegramBotClient::with_base_url(
        "test_tok",
        format!("http://{tg_address}"),
    );
    let provider = ProviderConfig {
        id: "main-quiz-pre-fail-test".into(),
        name: "Main Quiz Pre Fail Test".into(),
        endpoint: format!("http://{ai_address}/v1"),
        api_key: String::new(),
        api_key_ref: None,
        models: vec!["quiz-model".into()],
        active_model: "quiz-model".into(),
    };
    let service = isolated_service(provider.clone());
    let snapshot = service.generation_model_snapshot().await;

    let (_cancel, mut receiver) = watch::channel(false);
    let gen_input = GenerationInput {
        prompt: "Buatkan kuis uji gagal preamble",
        canonical_prompt: None,
        media_to_main: true,
        sink: None,
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
        bot: Some(bot_client),
        reply_to_message_id: None,
    };

    let (_thinking, answer, _staged_docs, cancelled) = service
        .generate_response_with_snapshot(1234, 0, 1234, gen_input, &snapshot, &mut receiver)
        .await;

    assert!(!cancelled);
    assert!(answer.contains("pesan pengantar kuis gagal terkirim"));

    let tg_requests = tg_server.await.expect("tg server join");
    assert_eq!(
        tg_requests.len(),
        1,
        "Must only attempt sendRichMessage and abort before sendPoll"
    );
    assert!(tg_requests[0].0.contains("POST /sendRichMessage"));

    ai_server.await.expect("ai server join");
}
