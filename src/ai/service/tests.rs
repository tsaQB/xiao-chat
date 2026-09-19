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
