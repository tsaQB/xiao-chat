use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::Path;
use tracing::warn;

use super::open_session_db;
use super::secrets::{
    create_secret_ref, read_secret_in_dir, remove_secret_in_dir, secret_store_dir,
    write_secret_in_dir,
};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    #[serde(default, skip_serializing)]
    pub api_key: String,
    #[serde(default)]
    pub api_key_ref: Option<String>,
    pub models: Vec<String>,
    pub active_model: String,
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("endpoint", &self.endpoint)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<empty>"
                } else {
                    "<redacted>"
                },
            )
            .field("api_key_ref", &self.api_key_ref)
            .field("models", &self.models)
            .field("active_model", &self.active_model)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderStore {
    pub active_id: Option<String>,
    pub providers: Vec<ProviderConfig>,
}

fn resolve_legacy_file_path(filename: &str) -> std::path::PathBuf {
    // 1. Check existing legacy file in %APPDATA% (Windows)
    #[cfg(windows)]
    if let Ok(appdata) = std::env::var("APPDATA") {
        let trimmed = appdata.trim();
        if !trimmed.is_empty() {
            let appdata_path = std::path::Path::new(trimmed);
            let p1 = appdata_path.join(filename);
            if p1.exists() {
                return p1;
            }
            let p2 = appdata_path.join("xiaoai").join(filename);
            if p2.exists() {
                return p2;
            }
            let p3 = appdata_path.join("xiao").join(filename);
            if p3.exists() {
                return p3;
            }
            let p4 = appdata_path.join("XiaoAI").join(filename);
            if p4.exists() {
                return p4;
            }
        }
    }

    // 2. Check existing in HOME or USERPROFILE
    let home_candidates = [
        std::env::var("HOME").ok(),
        std::env::var("USERPROFILE").ok(),
    ];
    for home_opt in home_candidates.into_iter().flatten() {
        let trimmed = home_opt.trim();
        if !trimmed.is_empty() {
            let home_path = std::path::Path::new(trimmed);
            let p1 = home_path.join(filename);
            if p1.exists() {
                return p1;
            }
            let p2 = home_path.join(".xiao").join(filename);
            if p2.exists() {
                return p2;
            }
            let p3 = home_path.join("xiao").join(filename);
            if p3.exists() {
                return p3;
            }
            let p4 = home_path.join(".xiaoai").join(filename);
            if p4.exists() {
                return p4;
            }
            let p5 = home_path.join("xiaoai").join(filename);
            if p5.exists() {
                return p5;
            }
            let p6 = home_path.join("XiaoAI").join(filename);
            if p6.exists() {
                return p6;
            }
        }
    }

    // 3. Check existing in current working directory
    let local = std::path::Path::new(filename);
    if local.exists() {
        return local.to_path_buf();
    }

    // 4. Default destination fallback: on Windows prefer %APPDATA%, then HOME/USERPROFILE, then relative
    #[cfg(windows)]
    if let Ok(appdata) = std::env::var("APPDATA") {
        let trimmed = appdata.trim();
        if !trimmed.is_empty() {
            return std::path::Path::new(trimmed).join(filename);
        }
    }

    let fallback_candidates = [
        std::env::var("HOME").ok(),
        std::env::var("USERPROFILE").ok(),
    ];
    for home_opt in fallback_candidates.into_iter().flatten() {
        let trimmed = home_opt.trim();
        if !trimmed.is_empty() {
            return std::path::Path::new(trimmed).join(filename);
        }
    }

    local.to_path_buf()
}

pub fn get_providers_store_path() -> std::path::PathBuf {
    resolve_legacy_file_path(".xiao_providers.json")
}

pub fn load_provider_store() -> ProviderStore {
    let mut store = load_provider_store_from_storage();
    if store.providers.is_empty() && seed_default_provider_from_env_if_empty(&mut store) {
        return store;
    }
    store
}

fn load_provider_store_from_storage() -> ProviderStore {
    if let Ok(conn) = open_session_db() {
        if let Ok(value) = conn.query_row(
            "SELECT value FROM settings WHERE key='provider_store'",
            [],
            |row| row.get::<_, String>(0),
        ) {
            if let Ok(store) = serde_json::from_str::<ProviderStore>(&value) {
                let has_legacy_plaintext = store
                    .providers
                    .iter()
                    .any(|provider| !provider.api_key.is_empty());
                if has_legacy_plaintext {
                    if let Err(error) = save_provider_state_db(&store) {
                        warn!("Failed to migrate plaintext provider secrets: {error}");
                        return store;
                    }
                    if let Ok(canonical) = conn.query_row(
                        "SELECT value FROM settings WHERE key='provider_store'",
                        [],
                        |row| row.get::<_, String>(0),
                    ) {
                        if let Ok(store) = serde_json::from_str::<ProviderStore>(&canonical) {
                            return hydrate_provider_store(store);
                        }
                    }
                }
                return hydrate_provider_store(store);
            }
        }
    }
    let p = get_providers_store_path();
    if p.exists() {
        if let Ok(content) = std::fs::read_to_string(&p) {
            if let Ok(store) = serde_json::from_str::<ProviderStore>(&content) {
                if save_provider_store(&store).is_ok() {
                    // The legacy JSON contains provider API keys in plaintext.
                    // Remove it only after the secret copy + SQLite reference
                    // migration committed successfully.
                    if let Err(err) = std::fs::remove_file(&p) {
                        warn!("Failed to remove migrated legacy provider file: {err}");
                    }
                    return load_provider_store_from_storage();
                }
                return store;
            }
        }
    }
    ProviderStore::default()
}

pub const DEFAULT_OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/v1";
pub const DEFAULT_OPENROUTER_MODEL: &str = "google/gemini-2.0-flash-001";

pub fn parse_auto_seed_endpoint(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.contains("YOUR_")
        || (!trimmed.starts_with("http://") && !trimmed.starts_with("https://"))
    {
        return None;
    }
    Some(trimmed.trim_end_matches('/').to_string())
}

pub fn seed_default_provider_from_env_if_empty(store: &mut ProviderStore) -> bool {
    crate::load_environment();
    let Ok(mut conn) = open_session_db() else {
        warn!("Failed to open session database for provider auto-seeding");
        return false;
    };
    seed_default_provider_from_env_if_empty_on_conn_and_dir(&mut conn, &secret_store_dir(), store)
}

pub(crate) fn seed_default_provider_from_env_if_empty_on_conn_and_dir(
    conn: &mut Connection,
    secrets_dir: &Path,
    store: &mut ProviderStore,
) -> bool {
    if !store.providers.is_empty() {
        return false;
    }

    let endpoint_raw = std::env::var("AI_ENDPOINT").ok();
    let endpoint = match endpoint_raw.as_deref() {
        Some(raw) => match parse_auto_seed_endpoint(raw) {
            Some(ep) => ep,
            None => return false,
        },
        None => DEFAULT_OPENROUTER_ENDPOINT.to_string(),
    };

    let api_key = match std::env::var("AI_API_KEY") {
        Ok(val) => {
            let trimmed = val.trim();
            if trimmed.contains("YOUR_") {
                return false;
            }
            if trimmed.is_empty() {
                "none".to_string()
            } else {
                trimmed.to_string()
            }
        }
        Err(_) => "none".to_string(),
    };

    let is_local = endpoint.contains("127.0.0.1")
        || endpoint.contains("localhost")
        || endpoint.contains("0.0.0.0");
    if !is_local && (api_key == "none" || api_key.is_empty()) {
        return false;
    }

    let active_model = match std::env::var("AI_MODEL") {
        Ok(val) => {
            let trimmed = val.trim();
            if trimmed.is_empty() {
                if endpoint == DEFAULT_OPENROUTER_ENDPOINT || endpoint.contains("openrouter.ai") {
                    DEFAULT_OPENROUTER_MODEL.to_string()
                } else {
                    "default".to_string()
                }
            } else {
                trimmed.to_string()
            }
        }
        Err(_) => {
            if endpoint == DEFAULT_OPENROUTER_ENDPOINT || endpoint.contains("openrouter.ai") {
                DEFAULT_OPENROUTER_MODEL.to_string()
            } else {
                "default".to_string()
            }
        }
    };

    let (name, id) =
        if endpoint == DEFAULT_OPENROUTER_ENDPOINT || endpoint.contains("openrouter.ai") {
            ("OpenRouter".to_string(), "openrouter".to_string())
        } else {
            let n = url::Url::parse(&endpoint)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_string()))
                .filter(|h| !h.is_empty())
                .unwrap_or_else(|| "Default Provider".to_string());
            (n, "env-default".to_string())
        };

    let new_provider = ProviderConfig {
        id: id.clone(),
        name,
        endpoint,
        api_key,
        api_key_ref: None,
        models: vec![active_model.clone()],
        active_model,
    };

    let candidate = ProviderStore {
        active_id: Some(id),
        providers: vec![new_provider],
    };

    if let Err(error) = save_provider_state_on_conn_and_dir(conn, secrets_dir, &candidate) {
        warn!("Failed to auto-seed provider store from environment: {error}");
        return false;
    }

    let Ok(value) = conn.query_row(
        "SELECT value FROM settings WHERE key='provider_store'",
        [],
        |row| row.get::<_, String>(0),
    ) else {
        warn!("Failed to read back persisted provider store after auto-seeding");
        return false;
    };
    let Ok(persisted) = serde_json::from_str::<ProviderStore>(&value) else {
        warn!("Failed to deserialize persisted provider store after auto-seeding");
        return false;
    };

    *store = hydrate_provider_store_in_dir(secrets_dir, persisted);
    true
}

pub fn save_provider_store(store: &ProviderStore) -> std::io::Result<()> {
    save_provider_state_db(store)
}

fn save_provider_state_db(store: &ProviderStore) -> std::io::Result<()> {
    let mut conn = open_session_db().map_err(|e| std::io::Error::other(e.to_string()))?;
    save_provider_state_on_conn_and_dir(&mut conn, &secret_store_dir(), store)
}

fn save_provider_state_on_conn_and_dir(
    conn: &mut Connection,
    secrets_dir: &Path,
    store: &ProviderStore,
) -> std::io::Result<()> {
    let existing_store = conn
        .query_row(
            "SELECT value FROM settings WHERE key='provider_store'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|value| serde_json::from_str::<ProviderStore>(&value).ok())
        .unwrap_or_default();
    let existing_refs: HashMap<String, String> = existing_store
        .providers
        .into_iter()
        .filter_map(|provider| {
            provider
                .api_key_ref
                .map(|secret_ref| (provider.id, secret_ref))
        })
        .collect();

    let mut sanitized = store.clone();
    let mut superseded_refs = Vec::new();
    let mut newly_written_refs: Vec<String> = Vec::new();
    for provider in &mut sanitized.providers {
        let old_ref = provider
            .api_key_ref
            .clone()
            .or_else(|| existing_refs.get(&provider.id).cloned());
        let has_secret = !provider.api_key.is_empty()
            && !["none", "-", "no"]
                .iter()
                .any(|sentinel| provider.api_key.eq_ignore_ascii_case(sentinel));

        if has_secret {
            let reusable = old_ref
                .as_deref()
                .and_then(|secret_ref| read_secret_in_dir(secrets_dir, secret_ref).ok())
                .is_some_and(|current| current == provider.api_key);
            if reusable {
                provider.api_key_ref = old_ref;
            } else {
                let new_ref = create_secret_ref("provider", &provider.id);
                if let Err(error) = write_secret_in_dir(secrets_dir, &new_ref, &provider.api_key) {
                    for secret_ref in &newly_written_refs {
                        remove_secret_in_dir(secrets_dir, secret_ref);
                    }
                    return Err(error);
                }
                match read_secret_in_dir(secrets_dir, &new_ref) {
                    Ok(verified) if verified == provider.api_key => {}
                    Ok(_) => {
                        remove_secret_in_dir(secrets_dir, &new_ref);
                        for secret_ref in &newly_written_refs {
                            remove_secret_in_dir(secrets_dir, secret_ref);
                        }
                        return Err(io::Error::other("provider secret verification mismatch"));
                    }
                    Err(error) => {
                        remove_secret_in_dir(secrets_dir, &new_ref);
                        for secret_ref in &newly_written_refs {
                            remove_secret_in_dir(secrets_dir, secret_ref);
                        }
                        return Err(error);
                    }
                }
                newly_written_refs.push(new_ref.clone());
                if let Some(old_ref) = old_ref {
                    superseded_refs.push(old_ref);
                }
                provider.api_key_ref = Some(new_ref);
            }
        } else {
            if let Some(old_ref) = old_ref {
                superseded_refs.push(old_ref);
            }
            provider.api_key_ref = None;
        }
        provider.api_key.clear();
    }

    let sanitized_ids: std::collections::HashSet<&str> =
        sanitized.providers.iter().map(|p| p.id.as_str()).collect();
    for (id, secret_ref) in &existing_refs {
        if !sanitized_ids.contains(id.as_str()) {
            superseded_refs.push(secret_ref.clone());
        }
    }

    let commit_result = (|| -> io::Result<()> {
        let json_str = serde_json::to_string_pretty(&sanitized).map_err(io::Error::other)?;
        let tx = conn
            .transaction()
            .map_err(|e| io::Error::other(e.to_string()))?;
        tx.execute(
            "INSERT INTO settings(key,value) VALUES('provider_store',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![json_str],
        )
        .map_err(|e| io::Error::other(e.to_string()))?;

        let active = sanitized
            .active_id
            .as_deref()
            .and_then(|id| {
                sanitized
                    .providers
                    .iter()
                    .find(|provider| provider.id == id)
            })
            .or_else(|| sanitized.providers.first());
        for (key, value) in [
            (
                "AI_ENDPOINT",
                active.map(|p| p.endpoint.as_str()).unwrap_or(""),
            ),
            (
                "AI_MODEL",
                active.map(|p| p.active_model.as_str()).unwrap_or(""),
            ),
        ] {
            tx.execute(
                "INSERT INTO settings(key,value) VALUES(?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![format!("app:{key}"), value],
            )
            .map_err(|e| io::Error::other(e.to_string()))?;
        }

        if let Some(secret_ref) = active.and_then(|provider| provider.api_key_ref.as_deref()) {
            tx.execute(
                "INSERT INTO settings(key,value) VALUES('app:AI_API_KEY_REF',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![secret_ref],
            )
            .map_err(|e| io::Error::other(e.to_string()))?;
        } else {
            tx.execute("DELETE FROM settings WHERE key='app:AI_API_KEY_REF'", [])
                .map_err(|e| io::Error::other(e.to_string()))?;
        }
        tx.execute("DELETE FROM settings WHERE key='app:AI_API_KEY'", [])
            .map_err(|e| io::Error::other(e.to_string()))?;
        tx.commit().map_err(|e| io::Error::other(e.to_string()))
    })();

    if let Err(error) = commit_result {
        for secret_ref in &newly_written_refs {
            remove_secret_in_dir(secrets_dir, secret_ref);
        }
        return Err(error);
    }

    let live_refs: std::collections::HashSet<&str> = sanitized
        .providers
        .iter()
        .filter_map(|provider| provider.api_key_ref.as_deref())
        .collect();
    superseded_refs.sort();
    superseded_refs.dedup();
    for secret_ref in superseded_refs {
        if !live_refs.contains(secret_ref.as_str()) {
            remove_secret_in_dir(secrets_dir, &secret_ref);
        }
    }
    Ok(())
}

fn hydrate_provider_store(store: ProviderStore) -> ProviderStore {
    hydrate_provider_store_in_dir(&secret_store_dir(), store)
}

fn hydrate_provider_store_in_dir(secrets_dir: &Path, mut store: ProviderStore) -> ProviderStore {
    for provider in &mut store.providers {
        provider.api_key = provider
            .api_key_ref
            .as_deref()
            .and_then(
                |secret_ref| match read_secret_in_dir(secrets_dir, secret_ref) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        warn!(
                            "Unable to load provider credential reference for '{}': {error}",
                            provider.id
                        );
                        None
                    }
                },
            )
            .unwrap_or_default();
    }
    store
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    TextChat,
    ImageInput,
    ImageGeneration,
    ImageEditing,
    AudioInput,
    AudioTranscription,
    VideoInput,
    NativeFileInput,
    Tools,
    StructuredOutput,
    Reasoning,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEvidenceSource {
    ProviderMetadata,
    ActiveProbe,
    KnownProviderProfile,
    UserOverride,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityEvidence {
    pub capability: CapabilityKind,
    pub source: CapabilityEvidenceSource,
    pub outcome: CapabilityState,
    pub checked_at: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceFreshness {
    Fresh,
    Stale,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeOutcome {
    Supported,
    Unsupported,
    Inconclusive,
    AuthFailed,
    RateLimited,
    Timeout,
    NetworkError,
    ProtocolMismatch,
    ProviderError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ProbeEvent {
    Started {
        capability: CapabilityKind,
    },
    Progress {
        capability: CapabilityKind,
        message: String,
    },
    Completed {
        capability: CapabilityKind,
        outcome: ProbeOutcome,
    },
    Skipped {
        capability: CapabilityKind,
        reason: String,
    },
    Persistence {
        saved: bool,
    },
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilityRecord {
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub context_window: Option<usize>,
    #[serde(default, alias = "supports_text")]
    pub supports_text_chat: Option<bool>,
    #[serde(default, alias = "supports_image")]
    pub supports_image_input: Option<bool>,
    #[serde(default)]
    pub supports_image_generation: Option<bool>,
    #[serde(default)]
    pub supports_image_editing: Option<bool>,
    #[serde(default, alias = "supports_audio")]
    pub supports_audio_input: Option<bool>,
    #[serde(default)]
    pub supports_audio_transcription: Option<bool>,
    #[serde(default, alias = "supports_video")]
    pub supports_video_input: Option<bool>,
    #[serde(default, alias = "supports_file_input")]
    pub supports_native_file_input: Option<bool>,
    #[serde(default)]
    pub supports_reasoning: Option<bool>,
    #[serde(default)]
    pub supports_tools: Option<bool>,
    #[serde(default)]
    pub supports_structured_output: Option<bool>,
    #[serde(default)]
    pub evidence: Vec<CapabilityEvidence>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub details: Vec<String>,
    #[serde(default)]
    pub checked_at: String,
}

impl CapabilityRecord {
    #[cfg(test)]
    fn timestamp_freshness(checked_at: &str, ttl: std::time::Duration) -> EvidenceFreshness {
        let Ok(checked_at) = chrono::DateTime::parse_from_rfc3339(checked_at) else {
            return EvidenceFreshness::Stale;
        };
        let age = chrono::Utc::now().signed_duration_since(checked_at.with_timezone(&chrono::Utc));
        if age.num_seconds() >= 0 && age.to_std().is_ok_and(|age| age <= ttl) {
            EvidenceFreshness::Fresh
        } else {
            EvidenceFreshness::Stale
        }
    }

    fn evidence_source_ttl(source: CapabilityEvidenceSource) -> Option<std::time::Duration> {
        match source {
            CapabilityEvidenceSource::ProviderMetadata => {
                Some(std::time::Duration::from_secs(6 * 60 * 60))
            }
            CapabilityEvidenceSource::ActiveProbe => {
                Some(std::time::Duration::from_secs(7 * 24 * 60 * 60))
            }
            CapabilityEvidenceSource::KnownProviderProfile => {
                Some(std::time::Duration::from_secs(30 * 24 * 60 * 60))
            }
            CapabilityEvidenceSource::UserOverride => None,
        }
    }

    fn evidence_source_precedence(source: CapabilityEvidenceSource) -> u8 {
        match source {
            CapabilityEvidenceSource::ProviderMetadata => 1,
            CapabilityEvidenceSource::KnownProviderProfile => 2,
            CapabilityEvidenceSource::ActiveProbe => 3,
            CapabilityEvidenceSource::UserOverride => 4,
        }
    }

    fn evidence_is_fresh(evidence: &CapabilityEvidence) -> bool {
        let Ok(checked_at) = chrono::DateTime::parse_from_rfc3339(&evidence.checked_at) else {
            return false;
        };
        let age = chrono::Utc::now().signed_duration_since(checked_at.with_timezone(&chrono::Utc));
        if age.num_seconds() < 0 {
            return false;
        }
        match Self::evidence_source_ttl(evidence.source) {
            Some(ttl) => age.to_std().is_ok_and(|age| age <= ttl),
            None => true,
        }
    }

    pub fn effective_evidence_for(
        &self,
        capability: CapabilityKind,
    ) -> Option<&CapabilityEvidence> {
        self.evidence
            .iter()
            .filter(|evidence| evidence.capability == capability)
            .filter(|evidence| Self::evidence_is_fresh(evidence))
            .max_by(|left, right| {
                let left_key = (
                    Self::evidence_source_precedence(left.source),
                    chrono::DateTime::parse_from_rfc3339(&left.checked_at)
                        .map(|timestamp| timestamp.timestamp_millis())
                        .unwrap_or(i64::MIN),
                );
                let right_key = (
                    Self::evidence_source_precedence(right.source),
                    chrono::DateTime::parse_from_rfc3339(&right.checked_at)
                        .map(|timestamp| timestamp.timestamp_millis())
                        .unwrap_or(i64::MIN),
                );
                left_key.cmp(&right_key)
            })
    }

    pub fn effective_state_for(&self, capability: CapabilityKind) -> CapabilityState {
        self.effective_evidence_for(capability)
            .map(|evidence| evidence.outcome)
            .unwrap_or(CapabilityState::Unknown)
    }

    #[cfg(test)]
    pub fn freshness_for(
        &self,
        capability: CapabilityKind,
        ttl: std::time::Duration,
    ) -> EvidenceFreshness {
        let latest = self
            .evidence
            .iter()
            .filter(|evidence| evidence.capability == capability)
            .filter_map(|evidence| {
                chrono::DateTime::parse_from_rfc3339(&evidence.checked_at)
                    .ok()
                    .map(|timestamp| (timestamp, evidence.checked_at.as_str()))
            })
            .max_by_key(|(timestamp, _)| timestamp.timestamp_millis())
            .map(|(_, checked_at)| checked_at);

        // Freshness is strictly capability-scoped. Unrelated metadata or
        // catalog refreshes must never re-authorize stale legacy or missing
        // capability evidence. If no typed evidence exists for this capability,
        // it is always Stale (fail-closed).
        match latest {
            Some(checked_at) => Self::timestamp_freshness(checked_at, ttl),
            None => EvidenceFreshness::Stale,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilityRegistry {
    pub models: Vec<CapabilityRecord>,
}

fn decode_model_routing(value: Option<&str>) -> (crate::ai::routing::ModelRoutingConfig, bool) {
    match value.and_then(|value| {
        serde_json::from_str::<crate::ai::routing::ModelRoutingConfig>(value).ok()
    }) {
        Some(config) => (config, false),
        None => (crate::ai::routing::ModelRoutingConfig::default(), true),
    }
}

pub fn load_model_routing() -> crate::ai::routing::ModelRoutingConfig {
    let stored = open_session_db().ok().and_then(|conn| {
        conn.query_row(
            "SELECT value FROM settings WHERE key='model_routing'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
    });
    let (config, needs_persist) = decode_model_routing(stored.as_deref());
    if needs_persist {
        if let Err(error) = save_model_routing(&config) {
            warn!("Failed to persist default model routing: {error}");
        }
    }
    config
}

pub fn save_model_routing(config: &crate::ai::routing::ModelRoutingConfig) -> std::io::Result<()> {
    let value = serde_json::to_string_pretty(config)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let conn = open_session_db().map_err(|error| std::io::Error::other(error.to_string()))?;
    conn.execute(
        "INSERT INTO settings(key,value) VALUES('model_routing',?1) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![value],
    )
    .map(|_| ())
    .map_err(|error| std::io::Error::other(error.to_string()))
}

pub(crate) async fn persist_model_routing(config: crate::ai::routing::ModelRoutingConfig) -> bool {
    match tokio::task::spawn_blocking(move || save_model_routing(&config)).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            warn!("Failed to persist model routing: {error}");
            false
        }
        Err(error) => {
            warn!("Model routing persistence task failed: {error}");
            false
        }
    }
}

pub fn get_capability_registry_path() -> std::path::PathBuf {
    resolve_legacy_file_path(".xiao_model_capabilities.json")
}

pub fn load_capability_registry() -> CapabilityRegistry {
    if let Ok(conn) = open_session_db() {
        if let Ok(value) = conn.query_row(
            "SELECT value FROM settings WHERE key='capability_registry'",
            [],
            |row| row.get::<_, String>(0),
        ) {
            if let Ok(registry) = serde_json::from_str(&value) {
                return registry;
            }
        }
    }
    let path = get_capability_registry_path();
    let registry = std::fs::read_to_string(path)
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default();
    if let Err(error) = save_capability_registry(&registry) {
        eprintln!("[WARN] Failed to migrate capability registry into SQLite: {error}");
    }
    registry
}

pub fn save_capability_registry(registry: &CapabilityRegistry) -> std::io::Result<()> {
    let value =
        serde_json::to_string_pretty(registry).map_err(|e| std::io::Error::other(e.to_string()))?;
    let conn = open_session_db().map_err(|e| std::io::Error::other(e.to_string()))?;
    conn.execute("INSERT INTO settings(key,value) VALUES('capability_registry',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![value])
        .map(|_| ())
        .map_err(|e| std::io::Error::other(e.to_string()))
}

pub(crate) async fn persist_capability_registry(registry: CapabilityRegistry) -> bool {
    match tokio::task::spawn_blocking(move || save_capability_registry(&registry)).await {
        Ok(Ok(())) => true,
        Ok(Err(err)) => {
            warn!("Failed to persist capability registry: {err}");
            false
        }
        Err(err) => {
            warn!("Capability persistence task failed: {err}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::routing::ModelRoutingConfig;
    use crate::ai::storage::secrets::secret_path_in_dir;

    #[test]
    fn provider_serialization_and_debug_never_include_raw_api_key() {
        let secret = "sk-test-super-secret";
        let provider = ProviderConfig {
            id: "p1".to_string(),
            name: "Provider".to_string(),
            endpoint: "https://example.invalid/v1".to_string(),
            api_key: secret.to_string(),
            api_key_ref: Some("secret://provider/p1/ref".to_string()),
            models: vec!["model".to_string()],
            active_model: "model".to_string(),
        };
        let json = serde_json::to_string(&ProviderStore {
            active_id: Some("p1".to_string()),
            providers: vec![provider.clone()],
        })
        .expect("serialize provider store succeeds");
        assert!(!json.contains(secret));
        assert!(json.contains("secret://provider/p1/ref"));
        assert!(!format!("{provider:?}").contains(secret));
    }

    #[test]
    fn missing_model_routing_migrates_to_main_model_defaults() {
        let (routing, needs_persist) = decode_model_routing(None);
        assert!(needs_persist);
        for role in crate::ai::routing::ModelRole::addon_roles() {
            assert_eq!(
                routing.route(role),
                Some(&crate::ai::routing::ModelRoute::MainModel)
            );
        }
    }

    #[test]
    fn valid_model_routing_does_not_request_rewrite() {
        let json = serde_json::to_string(&crate::ai::routing::ModelRoutingConfig::default())
            .expect("serialize default model routing succeeds");
        let (_, needs_persist) = decode_model_routing(Some(&json));
        assert!(!needs_persist);
    }

    #[test]
    fn capability_freshness_is_scoped_to_its_own_evidence() {
        let now = chrono::Utc::now().to_rfc3339();
        let old = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        let record = CapabilityRecord {
            supports_text_chat: Some(true),
            supports_image_generation: Some(true),
            checked_at: now.clone(),
            evidence: vec![
                CapabilityEvidence {
                    capability: CapabilityKind::TextChat,
                    source: CapabilityEvidenceSource::ActiveProbe,
                    outcome: CapabilityState::Supported,
                    checked_at: now,
                    detail: None,
                },
                CapabilityEvidence {
                    capability: CapabilityKind::ImageGeneration,
                    source: CapabilityEvidenceSource::ActiveProbe,
                    outcome: CapabilityState::Supported,
                    checked_at: old,
                    detail: None,
                },
            ],
            ..CapabilityRecord::default()
        };
        let ttl = std::time::Duration::from_secs(7 * 24 * 60 * 60);
        assert_eq!(
            record.freshness_for(CapabilityKind::TextChat, ttl),
            EvidenceFreshness::Fresh
        );
        assert_eq!(
            record.freshness_for(CapabilityKind::ImageGeneration, ttl),
            EvidenceFreshness::Stale
        );
        assert_eq!(
            record.freshness_for(CapabilityKind::AudioTranscription, ttl),
            EvidenceFreshness::Stale
        );
    }

    #[test]
    fn source_aware_freshness_uses_each_evidence_own_ttl() {
        let now = chrono::Utc::now();
        let record = CapabilityRecord {
            evidence: vec![
                CapabilityEvidence {
                    capability: CapabilityKind::ImageInput,
                    source: CapabilityEvidenceSource::ActiveProbe,
                    outcome: CapabilityState::Unsupported,
                    checked_at: (now - chrono::Duration::days(8)).to_rfc3339(),
                    detail: None,
                },
                CapabilityEvidence {
                    capability: CapabilityKind::ImageInput,
                    source: CapabilityEvidenceSource::ProviderMetadata,
                    outcome: CapabilityState::Supported,
                    checked_at: (now - chrono::Duration::hours(2)).to_rfc3339(),
                    detail: None,
                },
            ],
            ..CapabilityRecord::default()
        };
        assert_eq!(
            record.effective_state_for(CapabilityKind::ImageInput),
            CapabilityState::Supported
        );
    }

    #[test]
    fn fresh_active_probe_remains_authoritative_when_metadata_is_stale() {
        let now = chrono::Utc::now();
        let record = CapabilityRecord {
            evidence: vec![
                CapabilityEvidence {
                    capability: CapabilityKind::ImageInput,
                    source: CapabilityEvidenceSource::ActiveProbe,
                    outcome: CapabilityState::Supported,
                    checked_at: (now - chrono::Duration::days(1)).to_rfc3339(),
                    detail: None,
                },
                CapabilityEvidence {
                    capability: CapabilityKind::ImageInput,
                    source: CapabilityEvidenceSource::ProviderMetadata,
                    outcome: CapabilityState::Unsupported,
                    checked_at: (now - chrono::Duration::hours(7)).to_rfc3339(),
                    detail: None,
                },
            ],
            ..CapabilityRecord::default()
        };
        assert_eq!(
            record.effective_state_for(CapabilityKind::ImageInput),
            CapabilityState::Supported
        );
    }

    #[test]
    fn fresh_active_probe_overrides_fresh_metadata_deterministically() {
        let now = chrono::Utc::now().to_rfc3339();
        let record = CapabilityRecord {
            evidence: vec![
                CapabilityEvidence {
                    capability: CapabilityKind::AudioInput,
                    source: CapabilityEvidenceSource::ProviderMetadata,
                    outcome: CapabilityState::Supported,
                    checked_at: now.clone(),
                    detail: None,
                },
                CapabilityEvidence {
                    capability: CapabilityKind::AudioInput,
                    source: CapabilityEvidenceSource::ActiveProbe,
                    outcome: CapabilityState::Unsupported,
                    checked_at: now,
                    detail: None,
                },
            ],
            ..CapabilityRecord::default()
        };
        assert_eq!(
            record.effective_state_for(CapabilityKind::AudioInput),
            CapabilityState::Unsupported
        );
    }

    #[test]
    fn stale_metadata_does_not_inherit_active_probe_ttl() {
        let now = chrono::Utc::now();
        let record = CapabilityRecord {
            evidence: vec![
                CapabilityEvidence {
                    capability: CapabilityKind::VideoInput,
                    source: CapabilityEvidenceSource::ProviderMetadata,
                    outcome: CapabilityState::Supported,
                    checked_at: (now - chrono::Duration::hours(7)).to_rfc3339(),
                    detail: None,
                },
                CapabilityEvidence {
                    capability: CapabilityKind::VideoInput,
                    source: CapabilityEvidenceSource::ActiveProbe,
                    outcome: CapabilityState::Supported,
                    checked_at: (now - chrono::Duration::days(8)).to_rfc3339(),
                    detail: None,
                },
            ],
            ..CapabilityRecord::default()
        };
        assert_eq!(
            record.effective_state_for(CapabilityKind::VideoInput),
            CapabilityState::Unknown
        );
    }

    #[test]
    fn unrelated_fresh_probe_cannot_refresh_other_capability_metadata() {
        let now = chrono::Utc::now();
        let record = CapabilityRecord {
            evidence: vec![
                CapabilityEvidence {
                    capability: CapabilityKind::TextChat,
                    source: CapabilityEvidenceSource::ActiveProbe,
                    outcome: CapabilityState::Supported,
                    checked_at: now.to_rfc3339(),
                    detail: None,
                },
                CapabilityEvidence {
                    capability: CapabilityKind::ImageInput,
                    source: CapabilityEvidenceSource::ProviderMetadata,
                    outcome: CapabilityState::Supported,
                    checked_at: (now - chrono::Duration::hours(7)).to_rfc3339(),
                    detail: None,
                },
            ],
            ..CapabilityRecord::default()
        };
        assert_eq!(
            record.effective_state_for(CapabilityKind::ImageInput),
            CapabilityState::Unknown
        );
        assert_eq!(
            record.effective_state_for(CapabilityKind::TextChat),
            CapabilityState::Supported
        );
    }

    #[test]
    fn legacy_capability_fields_migrate_without_granting_new_capabilities() {
        let legacy = serde_json::json!({
            "provider_id": "legacy-provider",
            "provider_name": "Legacy",
            "model": "legacy-model",
            "supports_text": true,
            "supports_image": true,
            "supports_audio": true,
            "supports_video": false,
            "supports_file_input": true
        });
        let record: CapabilityRecord =
            serde_json::from_value(legacy).expect("deserialize capability record succeeds");

        assert_eq!(record.supports_text_chat, Some(true));
        assert_eq!(record.supports_image_input, Some(true));
        assert_eq!(record.supports_audio_input, Some(true));
        assert_eq!(record.supports_video_input, Some(false));
        assert_eq!(record.supports_native_file_input, Some(true));
        assert_eq!(record.supports_image_generation, None);
        assert_eq!(record.supports_image_editing, None);
        assert_eq!(record.supports_audio_transcription, None);
    }

    #[test]
    fn legacy_supports_image_without_evidence_and_stale_timestamp_remains_stale() {
        let old = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        let record = CapabilityRecord {
            supports_image_input: Some(true),
            evidence: Vec::new(),
            checked_at: old,
            ..CapabilityRecord::default()
        };
        let ttl = std::time::Duration::from_secs(7 * 24 * 60 * 60);
        assert_eq!(
            record.freshness_for(CapabilityKind::ImageInput, ttl),
            EvidenceFreshness::Stale
        );
    }

    #[test]
    fn legacy_audio_not_refreshed_by_unrelated_text_metadata() {
        let now = chrono::Utc::now().to_rfc3339();
        let record = CapabilityRecord {
            supports_audio_input: Some(true),
            evidence: vec![CapabilityEvidence {
                capability: CapabilityKind::TextChat,
                source: CapabilityEvidenceSource::ProviderMetadata,
                outcome: CapabilityState::Supported,
                checked_at: now.clone(),
                detail: None,
            }],
            checked_at: now,
            ..CapabilityRecord::default()
        };
        let ttl = std::time::Duration::from_secs(7 * 24 * 60 * 60);
        assert_eq!(
            record.freshness_for(CapabilityKind::AudioInput, ttl),
            EvidenceFreshness::Stale
        );
        assert_eq!(
            record.freshness_for(CapabilityKind::TextChat, ttl),
            EvidenceFreshness::Fresh
        );
    }

    #[test]
    fn fresh_provider_metadata_specifically_for_image_input_is_fresh() {
        let now = chrono::Utc::now().to_rfc3339();
        let record = CapabilityRecord {
            supports_image_input: Some(true),
            evidence: vec![CapabilityEvidence {
                capability: CapabilityKind::ImageInput,
                source: CapabilityEvidenceSource::ProviderMetadata,
                outcome: CapabilityState::Supported,
                checked_at: now.clone(),
                detail: Some("modalities: text,image".to_string()),
            }],
            checked_at: now,
            ..CapabilityRecord::default()
        };
        let ttl = std::time::Duration::from_secs(6 * 60 * 60);
        assert_eq!(
            record.freshness_for(CapabilityKind::ImageInput, ttl),
            EvidenceFreshness::Fresh
        );
    }

    #[test]
    fn fresh_text_chat_evidence_does_not_make_image_generation_fresh() {
        let now = chrono::Utc::now().to_rfc3339();
        let record = CapabilityRecord {
            supports_text_chat: Some(true),
            supports_image_generation: Some(true),
            evidence: vec![CapabilityEvidence {
                capability: CapabilityKind::TextChat,
                source: CapabilityEvidenceSource::ActiveProbe,
                outcome: CapabilityState::Supported,
                checked_at: now.clone(),
                detail: None,
            }],
            checked_at: now,
            ..CapabilityRecord::default()
        };
        let ttl = std::time::Duration::from_secs(7 * 24 * 60 * 60);
        assert_eq!(
            record.freshness_for(CapabilityKind::ImageGeneration, ttl),
            EvidenceFreshness::Stale
        );
    }

    #[test]
    fn stale_active_probe_for_image_generation_with_fresh_model_catalog_metadata_stays_stale() {
        let now = chrono::Utc::now().to_rfc3339();
        let old = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        let record = CapabilityRecord {
            supports_image_generation: Some(true),
            evidence: vec![CapabilityEvidence {
                capability: CapabilityKind::ImageGeneration,
                source: CapabilityEvidenceSource::ActiveProbe,
                outcome: CapabilityState::Supported,
                checked_at: old,
                detail: None,
            }],
            checked_at: now,
            ..CapabilityRecord::default()
        };
        let ttl = std::time::Duration::from_secs(7 * 24 * 60 * 60);
        assert_eq!(
            record.freshness_for(CapabilityKind::ImageGeneration, ttl),
            EvidenceFreshness::Stale
        );
    }

    #[test]
    fn model_routing_additive_setup_preserves_custom_routes() {
        let mut config = ModelRoutingConfig::default();
        let _ = config.set_route(
            crate::ai::routing::ModelRole::Vision,
            crate::ai::routing::ModelRoute::Disabled,
        );
        let _ = config.set_route(
            crate::ai::routing::ModelRole::ImageGeneration,
            crate::ai::routing::ModelRoute::Specific {
                provider_id: "prov_custom".to_string(),
                model: "flux-pro".to_string(),
            },
        );

        // Simulate additive setup: only set routes for roles that are None
        for role in crate::ai::routing::ModelRole::addon_roles() {
            if config.route(role).is_none() {
                let _ = config.set_route(role, crate::ai::routing::ModelRoute::MainModel);
            }
        }

        assert_eq!(
            config.route(crate::ai::routing::ModelRole::Vision),
            Some(&crate::ai::routing::ModelRoute::Disabled)
        );
        assert_eq!(
            config.route(crate::ai::routing::ModelRole::ImageGeneration),
            Some(&crate::ai::routing::ModelRoute::Specific {
                provider_id: "prov_custom".to_string(),
                model: "flux-pro".to_string(),
            })
        );
        assert_eq!(
            config.route(crate::ai::routing::ModelRole::Video),
            Some(&crate::ai::routing::ModelRoute::MainModel)
        );
        assert_eq!(
            config.route(crate::ai::routing::ModelRole::AudioStt),
            Some(&crate::ai::routing::ModelRoute::MainModel)
        );
        assert_eq!(
            config.route(crate::ai::routing::ModelRole::Curator),
            Some(&crate::ai::routing::ModelRoute::MainModel)
        );
    }

    use crate::ai::storage::ENV_TEST_LOCK;

    struct EnvCleanupGuard {
        vars: Vec<(&'static str, Option<String>)>,
    }

    impl Drop for EnvCleanupGuard {
        fn drop(&mut self) {
            for (key, original) in &self.vars {
                match original {
                    Some(val) => std::env::set_var(key, val),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn settings_test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open_in_memory succeeds");
        conn.execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .expect("execute_batch succeeds");
        conn
    }

    fn run_with_isolated_ai_env<F>(
        endpoint: Option<&str>,
        api_key: Option<&str>,
        model: Option<&str>,
        test_fn: F,
    ) where
        F: FnOnce(&mut Connection, &Path),
    {
        let _lock = ENV_TEST_LOCK.lock().expect("ENV_TEST_LOCK poisoned");
        let _guard = EnvCleanupGuard {
            vars: vec![
                ("AI_ENDPOINT", std::env::var("AI_ENDPOINT").ok()),
                ("AI_API_KEY", std::env::var("AI_API_KEY").ok()),
                ("AI_MODEL", std::env::var("AI_MODEL").ok()),
            ],
        };

        match endpoint {
            Some(ep) => std::env::set_var("AI_ENDPOINT", ep),
            None => std::env::remove_var("AI_ENDPOINT"),
        }
        match api_key {
            Some(key) => std::env::set_var("AI_API_KEY", key),
            None => std::env::remove_var("AI_API_KEY"),
        }
        match model {
            Some(m) => std::env::set_var("AI_MODEL", m),
            None => std::env::remove_var("AI_MODEL"),
        }

        let secrets_dir = std::env::temp_dir().join(format!(
            "xiaoai-autoseed-test-{}-{:x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let mut conn = settings_test_conn();

        test_fn(&mut conn, &secrets_dir);

        let _ = std::fs::remove_dir_all(secrets_dir);
    }

    #[test]
    fn test_auto_seed_provider_from_env_when_empty() {
        run_with_isolated_ai_env(
            Some("https://api.openai.com/v1/"),
            Some("sk-test-secret-key-12345"),
            Some("gpt-4o"),
            |conn, secrets_dir| {
                let mut store = ProviderStore::default();
                let seeded = seed_default_provider_from_env_if_empty_on_conn_and_dir(
                    conn,
                    secrets_dir,
                    &mut store,
                );
                assert!(seeded);
                assert_eq!(store.providers.len(), 1);
                assert_eq!(store.active_id, Some("env-default".to_string()));

                let p = &store.providers[0];
                assert_eq!(p.id, "env-default");
                assert_eq!(p.endpoint, "https://api.openai.com/v1");
                assert_eq!(p.name, "api.openai.com");
                assert_eq!(p.active_model, "gpt-4o");
                assert_eq!(p.models, vec!["gpt-4o".to_string()]);
                assert_eq!(p.api_key, "sk-test-secret-key-12345");

                let secret_ref = p.api_key_ref.as_ref().expect("api_key_ref populated");
                assert!(secret_ref.starts_with("secret://provider/env-default/"));

                let secret_path = secret_path_in_dir(secrets_dir, secret_ref)
                    .expect("secret path in dir succeeds");
                assert!(secret_path.exists());
                assert_eq!(
                    read_secret_in_dir(secrets_dir, secret_ref)
                        .expect("read secret in dir succeeds"),
                    "sk-test-secret-key-12345"
                );

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let file_mode = std::fs::metadata(&secret_path)
                        .expect("metadata succeeds")
                        .permissions()
                        .mode()
                        & 0o777;
                    assert_eq!(file_mode, 0o600);
                    let dir_mode = std::fs::metadata(secrets_dir)
                        .expect("metadata succeeds")
                        .permissions()
                        .mode()
                        & 0o777;
                    assert_eq!(dir_mode, 0o700);
                }

                let persisted_json: String = conn
                    .query_row(
                        "SELECT value FROM settings WHERE key='provider_store'",
                        [],
                        |row| row.get(0),
                    )
                    .expect("query provider_store succeeds");
                assert!(!persisted_json.contains("sk-test-secret-key-12345"));
                assert!(persisted_json.contains(secret_ref));

                let ep: String = conn
                    .query_row(
                        "SELECT value FROM settings WHERE key='app:AI_ENDPOINT'",
                        [],
                        |row| row.get(0),
                    )
                    .expect("query app:AI_ENDPOINT succeeds");
                assert_eq!(ep, "https://api.openai.com/v1");

                let model: String = conn
                    .query_row(
                        "SELECT value FROM settings WHERE key='app:AI_MODEL'",
                        [],
                        |row| row.get(0),
                    )
                    .expect("query app:AI_MODEL succeeds");
                assert_eq!(model, "gpt-4o");

                let key_ref: String = conn
                    .query_row(
                        "SELECT value FROM settings WHERE key='app:AI_API_KEY_REF'",
                        [],
                        |row| row.get(0),
                    )
                    .expect("query app:AI_API_KEY_REF succeeds");
                assert_eq!(key_ref, *secret_ref);

                let plaintext_key_count: usize = conn
                    .query_row(
                        "SELECT COUNT(*) FROM settings WHERE key='app:AI_API_KEY'",
                        [],
                        |row| row.get::<_, i64>(0).map(|c| c as usize),
                    )
                    .expect("query count app:AI_API_KEY succeeds");
                assert_eq!(plaintext_key_count, 0);
            },
        );
    }

    #[test]
    fn test_auto_seed_does_not_override_existing() {
        run_with_isolated_ai_env(
            Some("https://new-endpoint.ai/v1"),
            Some("new-key"),
            Some("new-model"),
            |conn, secrets_dir| {
                let mut store = ProviderStore {
                    active_id: Some("existing".to_string()),
                    providers: vec![ProviderConfig {
                        id: "existing".to_string(),
                        name: "Existing".to_string(),
                        endpoint: "https://existing.ai/v1".to_string(),
                        api_key: "existing-key".to_string(),
                        api_key_ref: None,
                        models: vec!["existing-model".to_string()],
                        active_model: "existing-model".to_string(),
                    }],
                };

                let seeded = seed_default_provider_from_env_if_empty_on_conn_and_dir(
                    conn,
                    secrets_dir,
                    &mut store,
                );
                assert!(!seeded);
                assert_eq!(store.providers.len(), 1);
                assert_eq!(store.providers[0].id, "existing");
                assert_eq!(store.providers[0].endpoint, "https://existing.ai/v1");
            },
        );
    }

    #[test]
    fn test_auto_seed_ignores_empty_or_dummy_endpoint() {
        // 1. Dummy placeholder
        run_with_isolated_ai_env(
            Some("http://YOUR_API_ENDPOINT_HERE/v1"),
            Some("sk-test"),
            Some("default"),
            |conn, secrets_dir| {
                let mut store = ProviderStore::default();
                assert!(!seed_default_provider_from_env_if_empty_on_conn_and_dir(
                    conn,
                    secrets_dir,
                    &mut store,
                ));
                assert!(store.providers.is_empty());
            },
        );

        // 2. Empty string
        run_with_isolated_ai_env(
            Some("   "),
            Some("sk-test"),
            Some("default"),
            |conn, secrets_dir| {
                let mut store = ProviderStore::default();
                assert!(!seed_default_provider_from_env_if_empty_on_conn_and_dir(
                    conn,
                    secrets_dir,
                    &mut store,
                ));
                assert!(store.providers.is_empty());
            },
        );

        // 3. Non-HTTP scheme
        run_with_isolated_ai_env(
            Some("ftp://api.example.com/v1"),
            Some("sk-test"),
            Some("default"),
            |conn, secrets_dir| {
                let mut store = ProviderStore::default();
                assert!(!seed_default_provider_from_env_if_empty_on_conn_and_dir(
                    conn,
                    secrets_dir,
                    &mut store,
                ));
                assert!(store.providers.is_empty());
            },
        );
    }

    #[test]
    fn test_auto_seed_defaults_api_key_and_model_when_omitted() {
        run_with_isolated_ai_env(
            Some("http://127.0.0.1:11434/v1"),
            None,
            None,
            |conn, secrets_dir| {
                let mut store = ProviderStore::default();
                let seeded = seed_default_provider_from_env_if_empty_on_conn_and_dir(
                    conn,
                    secrets_dir,
                    &mut store,
                );
                assert!(seeded);
                assert_eq!(store.providers.len(), 1);

                let p = &store.providers[0];
                assert_eq!(p.id, "env-default");
                assert_eq!(p.endpoint, "http://127.0.0.1:11434/v1");
                assert_eq!(p.name, "127.0.0.1");
                assert_eq!(p.api_key, ""); // "none" is unauthenticated
                assert_eq!(p.api_key_ref, None);
                assert_eq!(p.active_model, "default");
                assert_eq!(p.models, vec!["default".to_string()]);
            },
        );
    }

    #[test]
    fn test_auto_seed_openrouter_defaults_when_only_api_key_is_provided() {
        run_with_isolated_ai_env(
            None,
            Some("sk-or-v1-testkey123"),
            None,
            |conn, secrets_dir| {
                let mut store = ProviderStore::default();
                let seeded = seed_default_provider_from_env_if_empty_on_conn_and_dir(
                    conn,
                    secrets_dir,
                    &mut store,
                );
                assert!(seeded);
                assert_eq!(store.providers.len(), 1);
                assert_eq!(store.active_id, Some("openrouter".to_string()));

                let p = &store.providers[0];
                assert_eq!(p.id, "openrouter");
                assert_eq!(p.endpoint, DEFAULT_OPENROUTER_ENDPOINT);
                assert_eq!(p.name, "OpenRouter");
                assert_eq!(p.active_model, DEFAULT_OPENROUTER_MODEL);
                assert_eq!(p.models, vec![DEFAULT_OPENROUTER_MODEL.to_string()]);
                assert_eq!(p.api_key, "sk-or-v1-testkey123");
            },
        );
    }

    #[test]
    fn test_auto_seed_openrouter_requires_api_key_and_rejects_placeholder() {
        // Placeholder key rejected
        run_with_isolated_ai_env(
            None,
            Some("YOUR_OPENROUTER_API_KEY_HERE"),
            None,
            |conn, secrets_dir| {
                let mut store = ProviderStore::default();
                assert!(!seed_default_provider_from_env_if_empty_on_conn_and_dir(
                    conn,
                    secrets_dir,
                    &mut store,
                ));
                assert!(store.providers.is_empty());
            },
        );

        // Missing key rejected for remote OpenRouter
        run_with_isolated_ai_env(None, None, None, |conn, secrets_dir| {
            let mut store = ProviderStore::default();
            assert!(!seed_default_provider_from_env_if_empty_on_conn_and_dir(
                conn,
                secrets_dir,
                &mut store,
            ));
            assert!(store.providers.is_empty());
        });
    }

    #[test]
    fn test_auto_seed_failure_leaves_store_intact_and_unmutated() {
        run_with_isolated_ai_env(
            Some("https://api.openai.com/v1"),
            Some("sk-secret"),
            Some("gpt-4o"),
            |conn, _valid_secrets_dir| {
                let invalid_secrets_file = std::env::temp_dir().join(format!(
                    "xiaoai-invalid-secrets-{}-{:x}",
                    std::process::id(),
                    rand::random::<u64>()
                ));
                std::fs::write(&invalid_secrets_file, b"not a directory")
                    .expect("write invalid secrets dummy succeeds");

                let mut store = ProviderStore::default();
                let seeded = seed_default_provider_from_env_if_empty_on_conn_and_dir(
                    conn,
                    &invalid_secrets_file,
                    &mut store,
                );
                assert!(!seeded);
                assert!(store.providers.is_empty());
                assert_eq!(store.active_id, None);

                let _ = std::fs::remove_file(invalid_secrets_file);
            },
        );
    }

    #[test]
    fn resolve_legacy_file_path_finds_existing_file_in_appdata_or_profile() {
        let _lock = ENV_TEST_LOCK.lock().expect("ENV_TEST_LOCK poisoned");
        let orig_appdata = std::env::var("APPDATA").ok();
        let orig_home = std::env::var("HOME").ok();
        let orig_profile = std::env::var("USERPROFILE").ok();

        let temp_dir = std::env::temp_dir().join(format!(
            "xiaoai-legacy-test-{}-{:x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let test_file = temp_dir.join(".xiao_providers.json");
        std::fs::write(&test_file, b"{}").expect("write dummy provider json succeeds");

        std::env::set_var("USERPROFILE", &temp_dir);
        std::env::remove_var("HOME");
        std::env::remove_var("APPDATA");

        let resolved = get_providers_store_path();
        assert_eq!(resolved, test_file);

        let cap_path = get_capability_registry_path();
        assert_eq!(cap_path, temp_dir.join(".xiao_model_capabilities.json"));

        let _ = std::fs::remove_file(test_file);

        // Also verify finding inside .xiao subfolder
        let dot_xiao_dir = temp_dir.join(".xiao");
        let _ = std::fs::create_dir_all(&dot_xiao_dir);
        let nested_file = dot_xiao_dir.join(".xiao_providers.json");
        std::fs::write(&nested_file, b"{}").expect("write nested provider json succeeds");
        let resolved_nested = get_providers_store_path();
        assert_eq!(resolved_nested, nested_file);
        let _ = std::fs::remove_file(nested_file);
        let _ = std::fs::remove_dir_all(dot_xiao_dir);

        let _ = std::fs::remove_dir_all(temp_dir);

        if let Some(val) = orig_appdata {
            std::env::set_var("APPDATA", val);
        } else {
            std::env::remove_var("APPDATA");
        }
        if let Some(val) = orig_home {
            std::env::set_var("HOME", val);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(val) = orig_profile {
            std::env::set_var("USERPROFILE", val);
        } else {
            std::env::remove_var("USERPROFILE");
        }
    }
}
