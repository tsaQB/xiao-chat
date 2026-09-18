pub mod inbox;
pub mod memory;
pub mod provider;
pub mod secrets;
pub mod session;

pub use inbox::*;
pub use memory::*;
pub use provider::*;
pub use secrets::*;
pub use session::*;

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
    let base = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    base.join(".local/share/xiaoai")
}

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
        let _guard = DB_INIT_MUTEX.lock().unwrap();
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

    static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn xiao_data_dir_honors_env_override() {
        let _lock = ENV_TEST_LOCK.lock().unwrap();
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
}
