use rusqlite::{params, Connection};
use std::io::{self, Write};
use tracing::warn;

use super::{harden_dir_mode, harden_file_mode, open_session_db, xiao_data_dir};

const SECRET_SCHEME_PREFIX: &str = "secret://";

pub(crate) fn secret_store_dir() -> std::path::PathBuf {
    xiao_data_dir().join("secrets")
}

pub(crate) fn secret_path_in_dir(
    dir: &std::path::Path,
    secret_ref: &str,
) -> io::Result<std::path::PathBuf> {
    if !secret_ref.starts_with(SECRET_SCHEME_PREFIX) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid XiaoAI secret reference",
        ));
    }
    use base64::Engine;
    let name = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret_ref.as_bytes());
    Ok(dir.join(name))
}

pub(crate) fn create_secret_ref(namespace: &str, id: &str) -> String {
    use rand::Rng;
    let nonce: u64 = rand::thread_rng().gen();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let safe_id: String = id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    format!("secret://{namespace}/{safe_id}/{now:x}-{nonce:x}")
}

pub(crate) fn write_secret_in_dir(
    dir: &std::path::Path,
    secret_ref: &str,
    value: &str,
) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    harden_dir_mode(dir);
    let final_path = secret_path_in_dir(dir, secret_ref)?;
    let tmp_path = dir.join(format!(
        ".tmp-{}-{:x}",
        std::process::id(),
        rand::random::<u64>()
    ));

    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp_path)?;
    if let Err(error) = (|| -> io::Result<()> {
        file.write_all(value.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp_path, &final_path)?;
        harden_file_mode(&final_path);
        if let Ok(dir_handle) = std::fs::File::open(dir) {
            let _ = dir_handle.sync_all();
        }
        Ok(())
    })() {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn write_secret(secret_ref: &str, value: &str) -> io::Result<()> {
    write_secret_in_dir(&secret_store_dir(), secret_ref, value)
}

pub(crate) fn read_secret_in_dir(dir: &std::path::Path, secret_ref: &str) -> io::Result<String> {
    let path = secret_path_in_dir(dir, secret_ref)?;
    harden_file_mode(&path);
    std::fs::read_to_string(path)
}

pub(crate) fn read_secret(secret_ref: &str) -> io::Result<String> {
    read_secret_in_dir(&secret_store_dir(), secret_ref)
}

pub(crate) fn remove_secret_in_dir(dir: &std::path::Path, secret_ref: &str) {
    let Ok(path) = secret_path_in_dir(dir, secret_ref) else {
        return;
    };
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != io::ErrorKind::NotFound {
            warn!("Failed to remove superseded XiaoAI secret: {error}");
        }
    }
}

pub(crate) fn remove_secret(secret_ref: &str) {
    remove_secret_in_dir(&secret_store_dir(), secret_ref);
}

fn secret_setting_namespace(key: &str) -> Option<&'static str> {
    match key {
        "BOT_TOKEN" => Some("telegram"),
        "AI_API_KEY" => Some("app-provider"),
        _ => None,
    }
}

fn migrate_legacy_secret_setting_on_conn(
    conn: &mut Connection,
    secret_dir: &std::path::Path,
    key: &str,
    namespace: &str,
    legacy: &str,
) -> io::Result<String> {
    let ref_key = format!("app:{key}_REF");
    let raw_key = format!("app:{key}");
    let secret_ref = create_secret_ref(namespace, "main");
    write_secret_in_dir(secret_dir, &secret_ref, legacy)?;

    // First commit only the reference. The legacy plaintext remains available
    // until the newly written secret has been read back successfully.
    let reference_commit = (|| -> io::Result<()> {
        let tx = conn
            .transaction()
            .map_err(|error| io::Error::other(error.to_string()))?;
        tx.execute(
            "INSERT INTO settings(key,value) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![&ref_key, &secret_ref],
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        tx.commit()
            .map_err(|error| io::Error::other(error.to_string()))
    })();
    if let Err(error) = reference_commit {
        remove_secret_in_dir(secret_dir, &secret_ref);
        return Err(error);
    }

    let verified = match read_secret_in_dir(secret_dir, &secret_ref) {
        Ok(value) if value == legacy => value,
        Ok(_) => {
            if let Err(cleanup_error) =
                conn.execute("DELETE FROM settings WHERE key=?1", params![&ref_key])
            {
                warn!("Failed to roll back unverified secret reference {ref_key}: {cleanup_error}");
            }
            remove_secret_in_dir(secret_dir, &secret_ref);
            return Err(io::Error::other("secret migration verification mismatch"));
        }
        Err(error) => {
            if let Err(cleanup_error) =
                conn.execute("DELETE FROM settings WHERE key=?1", params![&ref_key])
            {
                warn!("Failed to roll back unreadable secret reference {ref_key}: {cleanup_error}");
            }
            remove_secret_in_dir(secret_dir, &secret_ref);
            return Err(error);
        }
    };

    // Plaintext removal is intentionally a second durable step after read-back
    // verification. If this delete fails, keeping the legacy row is safer than
    // losing the credential; the reference remains valid and can be retried.
    conn.execute("DELETE FROM settings WHERE key=?1", params![&raw_key])
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(verified)
}

fn load_secret_app_setting(key: &str, namespace: &str) -> Option<String> {
    let mut conn = open_session_db().ok()?;
    let ref_key = format!("app:{key}_REF");
    if let Ok(secret_ref) = conn.query_row(
        "SELECT value FROM settings WHERE key=?1",
        params![&ref_key],
        |row| row.get::<_, String>(0),
    ) {
        match read_secret(&secret_ref) {
            Ok(value) => return Some(value),
            Err(error) => {
                // A legacy plaintext row may still exist if an earlier migration
                // committed the reference but crashed before cleanup. Fall back
                // to it rather than losing access to the credential.
                warn!("Unable to resolve secret setting {key}: {error}; checking legacy fallback");
            }
        }
    }

    let raw_key = format!("app:{key}");
    let legacy = conn
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            params![&raw_key],
            |row| row.get::<_, String>(0),
        )
        .ok()?;
    if legacy.is_empty() {
        return Some(legacy);
    }

    match migrate_legacy_secret_setting_on_conn(
        &mut conn,
        &secret_store_dir(),
        key,
        namespace,
        &legacy,
    ) {
        Ok(value) => Some(value),
        Err(error) => {
            warn!("Failed to migrate legacy secret setting {key}: {error}");
            Some(legacy)
        }
    }
}

fn save_secret_app_setting(key: &str, namespace: &str, value: &str) -> io::Result<()> {
    let mut conn = open_session_db().map_err(|error| io::Error::other(error.to_string()))?;
    let ref_key = format!("app:{key}_REF");
    let raw_key = format!("app:{key}");
    let old_ref = conn
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            params![&ref_key],
            |row| row.get::<_, String>(0),
        )
        .ok();

    let reusable = old_ref
        .as_deref()
        .and_then(|secret_ref| read_secret(secret_ref).ok())
        .is_some_and(|current| current == value);
    let new_ref = if value.is_empty() {
        None
    } else if reusable {
        old_ref.clone()
    } else {
        let secret_ref = create_secret_ref(namespace, "main");
        write_secret(&secret_ref, value)?;
        let verified = read_secret(&secret_ref)?;
        if verified != value {
            remove_secret(&secret_ref);
            return Err(io::Error::other("secret write verification mismatch"));
        }
        Some(secret_ref)
    };

    let commit_result = (|| -> io::Result<()> {
        let tx = conn
            .transaction()
            .map_err(|error| io::Error::other(error.to_string()))?;
        if let Some(secret_ref) = new_ref.as_deref() {
            tx.execute(
                "INSERT INTO settings(key,value) VALUES(?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![&ref_key, secret_ref],
            )
            .map_err(|error| io::Error::other(error.to_string()))?;
        } else {
            tx.execute("DELETE FROM settings WHERE key=?1", params![&ref_key])
                .map_err(|error| io::Error::other(error.to_string()))?;
        }
        tx.execute("DELETE FROM settings WHERE key=?1", params![&raw_key])
            .map_err(|error| io::Error::other(error.to_string()))?;
        tx.commit()
            .map_err(|error| io::Error::other(error.to_string()))
    })();
    if let Err(error) = commit_result {
        if new_ref.as_deref() != old_ref.as_deref() {
            if let Some(secret_ref) = new_ref.as_deref() {
                remove_secret(secret_ref);
            }
        }
        return Err(error);
    }
    if old_ref.as_deref() != new_ref.as_deref() {
        if let Some(secret_ref) = old_ref.as_deref() {
            remove_secret(secret_ref);
        }
    }
    Ok(())
}

pub fn load_app_setting(key: &str) -> Option<String> {
    if let Some(namespace) = secret_setting_namespace(key) {
        return load_secret_app_setting(key, namespace);
    }
    open_session_db()
        .ok()?
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            params![format!("app:{key}")],
            |row| row.get(0),
        )
        .ok()
}

pub fn save_app_setting(key: &str, value: &str) -> std::io::Result<()> {
    if let Some(namespace) = secret_setting_namespace(key) {
        return save_secret_app_setting(key, namespace, value);
    }
    let conn = open_session_db().map_err(|e| std::io::Error::other(e.to_string()))?;
    conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("app:{key}"), value])
        .map(|_| ())
        .map_err(|e| std::io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_bot_token_migration_is_lossless_and_removes_plaintext_only_after_reference() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .unwrap();
        let token = "123456:test-secret-token";
        conn.execute(
            "INSERT INTO settings(key,value) VALUES('app:BOT_TOKEN',?1)",
            params![token],
        )
        .unwrap();

        let dir = std::env::temp_dir().join(format!(
            "xiaoai-secret-migration-{}-{:x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let migrated =
            migrate_legacy_secret_setting_on_conn(&mut conn, &dir, "BOT_TOKEN", "telegram", token)
                .unwrap();
        assert_eq!(migrated, token);

        let raw_count: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key='app:BOT_TOKEN'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw_count, 0);
        let secret_ref: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key='app:BOT_TOKEN_REF'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(secret_ref.starts_with("secret://telegram/"));
        assert_eq!(read_secret_in_dir(&dir, &secret_ref).unwrap(), token);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn local_secret_store_uses_private_directory_and_file_modes() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "xiaoai-secret-modes-{}-{:x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let secret_ref = "secret://test/mode-check";
        write_secret_in_dir(&dir, secret_ref, "secret").unwrap();
        let path = secret_path_in_dir(&dir, secret_ref).unwrap();

        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_secret_storage_write_aborts_without_persisting() {
        let invalid_dir = std::env::temp_dir().join(format!(
            "xiaoai-secret-parent-file-{}-{:x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::write(&invalid_dir, b"not a directory").unwrap();
        let secret_ref = "secret://test/fail-check";
        let res = write_secret_in_dir(&invalid_dir, secret_ref, "test");
        assert!(res.is_err());
        let _ = std::fs::remove_file(invalid_dir);
    }
}
