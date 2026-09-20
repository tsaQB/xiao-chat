pub mod inbox;
pub mod memory;
pub mod provider;
pub mod secrets;
pub mod session;

#[allow(unused_imports)]
pub use inbox::{
    enqueue_telegram_update_async, load_telegram_offset_async, mark_telegram_processed_async,
    mark_telegram_processing_async, mark_telegram_processing_claim_async,
    mark_telegram_processing_failed_async, mark_telegram_processing_retry_async,
    pending_telegram_updates_after_async, recover_telegram_processing_async, TelegramInboxRecord,
};
#[allow(unused_imports)]
pub use memory::{
    clear_user_memories, clear_user_memories_async, delete_user_memory, delete_user_memory_async,
    get_user_memories, get_user_memories_async, save_user_memory, save_user_memory_async,
};
#[allow(unused_imports)]
pub use provider::{
    get_capability_registry_path, get_providers_store_path, load_capability_registry,
    load_model_routing, load_provider_store, parse_auto_seed_endpoint, save_capability_registry,
    save_model_routing, save_provider_store, seed_default_provider_from_env_if_empty,
    CapabilityEvidence, CapabilityEvidenceSource, CapabilityKind, CapabilityRecord,
    CapabilityRegistry, CapabilityState, ProbeEvent, ProbeOutcome, ProviderConfig, ProviderStore,
    DEFAULT_OPENROUTER_ENDPOINT, DEFAULT_OPENROUTER_MODEL,
};
#[allow(unused_imports)]
pub(crate) use provider::{persist_capability_registry, persist_model_routing};
#[allow(unused_imports)]
pub use secrets::{load_app_setting, save_app_setting};
#[allow(unused_imports)]
pub use session::{
    clear_scoped_messages, clear_scoped_messages_async, count_scoped_messages,
    count_scoped_messages_async, get_scoped_summary, get_scoped_summary_async,
    load_scoped_messages, load_scoped_messages_async, save_scoped_message,
    save_scoped_message_async, save_scoped_summary, save_scoped_summary_async, ChatMessage,
    ChatSession,
};
#[allow(unused_imports)]
pub(crate) use session::{
    compute_next_session_id, create_session_and_activate_db_async,
    ensure_session_identity_v2_db_async, legacy_active_session_id, load_active_session_id_db_async,
    load_sessions_db_async, remove_session_transaction_db_async,
    replace_session_messages_if_revision_db_async, switch_active_session_db_async,
    RemoveSessionOutcome,
};

#[cfg(test)]
#[allow(unused_imports)]
pub use provider::EvidenceFreshness;
#[cfg(test)]
#[allow(unused_imports)]
pub use session::TopicScope;
#[cfg(test)]
pub(crate) use tests::ENV_TEST_LOCK;

use rusqlite::Connection;
use std::path::Path;
use std::sync::OnceLock;
use tracing::warn;

pub(crate) fn xiao_data_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("XIAO_DATA_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return std::path::PathBuf::from(trimmed);
        }
    }
    #[cfg(windows)]
    if let Ok(appdata) = std::env::var("APPDATA") {
        let trimmed = appdata.trim();
        if !trimmed.is_empty() {
            return std::path::PathBuf::from(trimmed).join("xiaoai");
        }
    }
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        let trimmed = xdg_data.trim();
        if !trimmed.is_empty() {
            return std::path::PathBuf::from(trimmed).join("xiaoai");
        }
    }
    let base = std::env::var("HOME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join(".local/share/xiaoai")
}

#[cfg(unix)]
pub(crate) fn harden_dir_mode(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o700);
        if let Err(err) = std::fs::set_permissions(path, permissions) {
            warn!("Failed to harden XiaoAI data directory permissions: {err}");
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn harden_dir_mode(_path: &std::path::Path) {}

#[cfg(unix)]
pub(crate) fn harden_file_mode(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        if let Err(err) = std::fs::set_permissions(path, permissions) {
            warn!("Failed to harden XiaoAI data file permissions: {err}");
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn harden_file_mode(_path: &std::path::Path) {}

pub(crate) fn session_db_path() -> std::path::PathBuf {
    xiao_data_dir().join("xiaoai.db")
}

// PRAGMA table_info is safe against SQL injection here because `table` is

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == column);
    if !exists {
        match conn.execute_batch(alter_sql) {
            Ok(()) => {}
            Err(err) => {
                let msg = err.to_string();
                if !msg.contains("duplicate column name") {
                    return Err(err);
                }
            }
        }
    }
    Ok(())
}

static DB_INIT: OnceLock<()> = OnceLock::new();
static DB_INIT_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn ensure_database_initialized(conn: &Connection, path: &Path) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;
        CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS sessions (
            user_id INTEGER NOT NULL, session_id INTEGER NOT NULL, name TEXT NOT NULL,
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL, revision INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(user_id, session_id)
        );
        CREATE TABLE IF NOT EXISTS messages (
            user_id INTEGER NOT NULL, session_id INTEGER NOT NULL DEFAULT 0,
            chat_id INTEGER NOT NULL DEFAULT 0, thread_id INTEGER NOT NULL DEFAULT 0,
            role TEXT NOT NULL, content TEXT NOT NULL, created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS active_sessions (
            user_id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS session_counters (
            user_id INTEGER PRIMARY KEY, next_session_id INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS telegram_state (
            key TEXT PRIMARY KEY, value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS telegram_inbox (
            update_id INTEGER PRIMARY KEY,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            received_at TEXT NOT NULL,
            last_error TEXT
        );
        CREATE TABLE IF NOT EXISTS user_memories (
            user_id INTEGER NOT NULL,
            key TEXT NOT NULL,
            fact TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(user_id, key)
        );
        CREATE TABLE IF NOT EXISTS scoped_summaries (
            chat_id INTEGER NOT NULL,
            thread_id INTEGER NOT NULL DEFAULT 0,
            summary TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(chat_id, thread_id)
        );
        CREATE INDEX IF NOT EXISTS idx_messages_user_session ON messages(user_id, session_id);
        CREATE INDEX IF NOT EXISTS idx_user_memories_user ON user_memories(user_id);
        CREATE INDEX IF NOT EXISTS idx_telegram_inbox_status_update_id ON telegram_inbox(status, update_id);",
    )?;
    ensure_column(
        conn,
        "sessions",
        "revision",
        "ALTER TABLE sessions ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;",
    )?;
    ensure_column(
        conn,
        "messages",
        "chat_id",
        "ALTER TABLE messages ADD COLUMN chat_id INTEGER NOT NULL DEFAULT 0;",
    )?;
    ensure_column(
        conn,
        "messages",
        "thread_id",
        "ALTER TABLE messages ADD COLUMN thread_id INTEGER NOT NULL DEFAULT 0;",
    )?;
    ensure_column(
        conn,
        "telegram_inbox",
        "attempts",
        "ALTER TABLE telegram_inbox ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;",
    )?;
    let _ = conn.execute(
        "UPDATE messages SET chat_id = user_id WHERE chat_id = 0 AND user_id != 0;",
        [],
    );
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_messages_chat_thread ON messages(chat_id, thread_id);",
    )?;
    // WAL/SHM files may be created lazily. The private 0700 parent directory
    // is the primary boundary; harden sidecars whenever they already exist.
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        harden_file_mode(&parent.join(format!("{name}-wal")));
        harden_file_mode(&parent.join(format!("{name}-shm")));
    }
    Ok(())
}

pub(crate) fn open_session_db() -> rusqlite::Result<Connection> {
    let path = session_db_path();
    if DB_INIT.get().is_none() {
        let _guard = DB_INIT_MUTEX.lock().expect("DB_INIT_MUTEX poisoned");
        if DB_INIT.get().is_none() {
            if let Some(parent) = path.parent() {
                if let Err(err) = std::fs::create_dir_all(parent) {
                    warn!("Failed to create XiaoAI data directory: {err}");
                }
                harden_dir_mode(parent);
            }
            let conn = Connection::open(&path)?;
            harden_file_mode(&path);
            ensure_database_initialized(&conn, &path)?;
            let _ = DB_INIT.set(());
            return Ok(conn);
        }
    }
    let conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
    Ok(conn)
}

async fn run_db<T, F>(operation: &'static str, task: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> rusqlite::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(task).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(err)) => {
            warn!("SQLite operation {operation} failed: {err}");
            None
        }
        Err(err) => {
            warn!("SQLite task {operation} failed: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    pub(crate) static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn xiao_data_dir_honors_env_override() {
        let _lock = ENV_TEST_LOCK.lock().expect("ENV_TEST_LOCK poisoned");
        let original = std::env::var("XIAO_DATA_DIR").ok();
        let custom_dir = "/tmp/test_xiao_custom_dir";
        std::env::set_var("XIAO_DATA_DIR", custom_dir);
        assert_eq!(xiao_data_dir(), std::path::PathBuf::from(custom_dir));

        if let Some(val) = original {
            std::env::set_var("XIAO_DATA_DIR", val);
        } else {
            std::env::remove_var("XIAO_DATA_DIR");
        }
    }

    #[test]
    #[cfg(windows)]
    fn xiao_data_dir_resolves_appdata_on_windows() {
        let _lock = ENV_TEST_LOCK.lock().expect("ENV_TEST_LOCK poisoned");
        let orig_xiao = std::env::var("XIAO_DATA_DIR").ok();
        let orig_appdata = std::env::var("APPDATA").ok();
        std::env::remove_var("XIAO_DATA_DIR");
        std::env::set_var("APPDATA", r"C:\TestAppData\Roaming");

        assert_eq!(
            xiao_data_dir(),
            std::path::PathBuf::from(r"C:\TestAppData\Roaming\xiaoai")
        );

        if let Some(val) = orig_xiao {
            std::env::set_var("XIAO_DATA_DIR", val);
        } else {
            std::env::remove_var("XIAO_DATA_DIR");
        }
        if let Some(val) = orig_appdata {
            std::env::set_var("APPDATA", val);
        } else {
            std::env::remove_var("APPDATA");
        }
    }

    #[test]
    fn xiao_data_dir_resolves_xdg_data_home_when_configured() {
        let _lock = ENV_TEST_LOCK.lock().expect("ENV_TEST_LOCK poisoned");
        let orig_xiao = std::env::var("XIAO_DATA_DIR").ok();
        #[cfg(windows)]
        let orig_appdata = std::env::var("APPDATA").ok();
        let orig_xdg = std::env::var("XDG_DATA_HOME").ok();

        std::env::remove_var("XIAO_DATA_DIR");
        #[cfg(windows)]
        std::env::remove_var("APPDATA");

        let custom_xdg = std::env::temp_dir().join(format!(
            "xiaoai-xdg-test-{}-{:x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::env::set_var("XDG_DATA_HOME", &custom_xdg);

        assert_eq!(xiao_data_dir(), custom_xdg.join("xiaoai"));

        if let Some(val) = orig_xiao {
            std::env::set_var("XIAO_DATA_DIR", val);
        } else {
            std::env::remove_var("XIAO_DATA_DIR");
        }
        #[cfg(windows)]
        if let Some(val) = orig_appdata {
            std::env::set_var("APPDATA", val);
        } else {
            std::env::remove_var("APPDATA");
        }
        if let Some(val) = orig_xdg {
            std::env::set_var("XDG_DATA_HOME", val);
        } else {
            std::env::remove_var("XDG_DATA_HOME");
        }
    }

    #[test]
    fn xiao_data_dir_falls_back_to_userprofile_or_home() {
        let _lock = ENV_TEST_LOCK.lock().expect("ENV_TEST_LOCK poisoned");
        let orig_xiao = std::env::var("XIAO_DATA_DIR").ok();
        let orig_appdata = std::env::var("APPDATA").ok();
        let orig_xdg = std::env::var("XDG_DATA_HOME").ok();
        let orig_home = std::env::var("HOME").ok();
        let orig_profile = std::env::var("USERPROFILE").ok();

        std::env::remove_var("XIAO_DATA_DIR");
        std::env::remove_var("APPDATA");
        std::env::remove_var("XDG_DATA_HOME");
        std::env::remove_var("HOME");
        let test_profile = std::env::temp_dir().join("test_user_profile");
        std::env::set_var("USERPROFILE", &test_profile);

        assert_eq!(xiao_data_dir(), test_profile.join(".local/share/xiaoai"));

        if let Some(val) = orig_xiao {
            std::env::set_var("XIAO_DATA_DIR", val);
        } else {
            std::env::remove_var("XIAO_DATA_DIR");
        }
        if let Some(val) = orig_appdata {
            std::env::set_var("APPDATA", val);
        } else {
            std::env::remove_var("APPDATA");
        }
        if let Some(val) = orig_xdg {
            std::env::set_var("XDG_DATA_HOME", val);
        } else {
            std::env::remove_var("XDG_DATA_HOME");
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

    #[test]
    fn xiao_data_dir_handles_empty_home_and_falls_back_to_userprofile() {
        let _lock = ENV_TEST_LOCK.lock().expect("ENV_TEST_LOCK poisoned");
        let orig_xiao = std::env::var("XIAO_DATA_DIR").ok();
        let orig_appdata = std::env::var("APPDATA").ok();
        let orig_xdg = std::env::var("XDG_DATA_HOME").ok();
        let orig_home = std::env::var("HOME").ok();
        let orig_profile = std::env::var("USERPROFILE").ok();

        std::env::remove_var("XIAO_DATA_DIR");
        std::env::remove_var("APPDATA");
        std::env::remove_var("XDG_DATA_HOME");
        std::env::set_var("HOME", "   ");
        let test_profile = std::env::temp_dir().join("test_user_profile_empty_home");
        std::env::set_var("USERPROFILE", &test_profile);

        assert_eq!(xiao_data_dir(), test_profile.join(".local/share/xiaoai"));

        if let Some(val) = orig_xiao {
            std::env::set_var("XIAO_DATA_DIR", val);
        } else {
            std::env::remove_var("XIAO_DATA_DIR");
        }
        if let Some(val) = orig_appdata {
            std::env::set_var("APPDATA", val);
        } else {
            std::env::remove_var("APPDATA");
        }
        if let Some(val) = orig_xdg {
            std::env::set_var("XDG_DATA_HOME", val);
        } else {
            std::env::remove_var("XDG_DATA_HOME");
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
