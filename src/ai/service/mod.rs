pub mod context;
pub mod generation;
pub mod image;
pub mod multimodal;
pub mod session;

#[cfg(test)]
mod tests;

use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex, RwLock};

#[allow(unused_imports)]
pub use self::context::{ContextMessageItem, ContextStats};
#[allow(unused_imports)]
pub use self::generation::{next_draft_id, GenerationGuard, GenerationInput, PendingToolCall};
#[allow(unused_imports)]
pub use self::image::{
    decode_generated_image_base64, download_generated_image, extract_image_from_chat_response,
    parse_data_uri_or_url, select_initial_image_protocol, ExtractedImageSource, GeneratedImage,
    ImageGenerationError, ImageGenerationErrorKind, ImageGenerationProtocol,
};
#[allow(unused_imports)]
pub use self::multimodal::{resolve_audio_file_and_mime, SpecialistObservationInput};
#[allow(unused_imports)]
pub use self::session::GenerationCancelSender;

#[allow(unused_imports)]
pub use crate::ai::capability::{ModelCapability, ModelMetadata};
#[allow(unused_imports)]
pub use crate::ai::routing::{
    GenerationModelSnapshot, ModelRole, ModelRoute, ModelRoutingConfig, ResolvedModelRoute,
    RouteOrigin,
};
#[allow(unused_imports)]
pub use crate::ai::storage::{
    load_app_setting, load_capability_registry, load_model_routing, load_provider_store,
    save_app_setting, save_provider_store, CapabilityKind, CapabilityRecord, CapabilityRegistry,
    CapabilityState, ChatSession, ProbeEvent, ProbeOutcome, ProviderConfig, ProviderStore,
};

pub(crate) type ActiveGenerations = Arc<RwLock<HashMap<(i64, i64), watch::Sender<bool>>>>;
pub(crate) type GenerationLockMap = Arc<RwLock<HashMap<(i64, i64), Arc<Mutex<()>>>>>;

pub(crate) const AI_PROVIDER_CONNECT_TIMEOUT_ENV: &str = "AI_PROVIDER_CONNECT_TIMEOUT_SECS";

pub(crate) fn bounded_timeout_secs(raw: Option<&str>, default_secs: u64) -> u64 {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default_secs)
        .min(600)
}

pub(crate) fn timeout_from_env(key: &str, default_secs: u64) -> Duration {
    let raw = std::env::var(key).ok();
    Duration::from_secs(bounded_timeout_secs(raw.as_deref(), default_secs))
}

pub(crate) fn provider_url(endpoint: &str, path: &str) -> String {
    format!(
        "{}/{}",
        endpoint.trim().trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

pub(crate) async fn read_bounded_response_bytes(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(format!("provider response exceeded {max_bytes} bytes"));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        let chunk = chunk.map_err(|_| "provider response stream failed".to_string())?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(format!("provider response exceeded {max_bytes} bytes"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) async fn read_bounded_json(response: reqwest::Response) -> Result<Value, String> {
    let bytes = read_bounded_response_bytes(response, 32 * 1024 * 1024).await?;
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid provider JSON: {error}"))
}

#[derive(Clone)]
pub struct AIChatService {
    pub(crate) client: Client,
    pub(crate) user_sessions: Arc<RwLock<HashMap<i64, Vec<ChatSession>>>>,
    pub(crate) active_session_id: Arc<RwLock<HashMap<i64, usize>>>,
    pub(crate) generation_locks: GenerationLockMap,
    pub(crate) session_locks: Arc<RwLock<HashMap<i64, Arc<Mutex<()>>>>>,
    pub(crate) active_generations: ActiveGenerations,
    pub(crate) provider_store: Arc<RwLock<ProviderStore>>,
    pub(crate) capability_registry: Arc<RwLock<CapabilityRegistry>>,
    pub(crate) model_routing: Arc<RwLock<ModelRoutingConfig>>,
    pub model_metadata: Arc<RwLock<HashMap<String, ModelMetadata>>>,
}

impl AIChatService {
    pub fn new() -> Self {
        crate::load_environment();
        for key in [
            "BOT_TOKEN",
            "OWNER_USER_ID",
            "ALLOWED_CHAT_IDS",
            "IMAGE_FALLBACK_PROVIDER",
        ] {
            if load_app_setting(key).is_none() {
                if let Ok(value) = std::env::var(key) {
                    if !value.trim().is_empty() {
                        if let Err(error) = save_app_setting(key, &value) {
                            eprintln!(
                                "[WARN] Failed to migrate environment setting {key}: {error}"
                            );
                        }
                    }
                }
            }
        }
        let provider_store = load_provider_store();
        let capability_registry = load_capability_registry();
        let model_routing = load_model_routing();
        let client = Client::builder()
            .connect_timeout(timeout_from_env(AI_PROVIDER_CONNECT_TIMEOUT_ENV, 10))
            .timeout(Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            user_sessions: Arc::new(RwLock::new(HashMap::new())),
            active_session_id: Arc::new(RwLock::new(HashMap::new())),
            generation_locks: Arc::new(RwLock::new(HashMap::new())),
            session_locks: Arc::new(RwLock::new(HashMap::new())),
            active_generations: Arc::new(RwLock::new(HashMap::new())),
            provider_store: Arc::new(RwLock::new(provider_store)),
            capability_registry: Arc::new(RwLock::new(capability_registry)),
            model_routing: Arc::new(RwLock::new(model_routing)),
            model_metadata: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for AIChatService {
    fn default() -> Self {
        Self::new()
    }
}
