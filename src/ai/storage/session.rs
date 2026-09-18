use chrono::Local;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{open_session_db, run_db};

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TopicScope {
    pub chat_id: i64,
    pub thread_id: i64,
}

#[cfg(test)]
impl TopicScope {
    pub fn new(chat_id: i64, thread_id: i64) -> Self {
        Self { chat_id, thread_id }
    }

    pub fn is_private(&self) -> bool {
        self.thread_id == 0 && self.chat_id > 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: usize,
    pub name: String,
    pub messages: Vec<ChatMessage>,
    pub created_at: String,
    #[serde(default)]
    pub revision: u64,
}

fn load_sessions_db(user_id: i64) -> rusqlite::Result<Vec<ChatSession>> {
    let conn = open_session_db()?;
    let mut stmt = conn.prepare(
        "SELECT session_id,name,created_at,revision FROM sessions WHERE user_id=?1 ORDER BY session_id",
    )?;
    let mut rows = stmt.query(params![user_id])?;
    let mut sessions = Vec::new();
    while let Some(row) = rows.next()? {
        let id: usize = row.get(0)?;
        let mut session = ChatSession {
            id,
            name: row.get(1)?,
            messages: Vec::new(),
            created_at: row.get(2)?,
            revision: row.get(3)?,
        };
        let mut msg_stmt = conn.prepare(
            "SELECT role,content FROM messages WHERE user_id=?1 AND session_id=?2 ORDER BY rowid",
        )?;
        let mut msg_rows = msg_stmt.query(params![user_id, id])?;
        while let Some(msg) = msg_rows.next()? {
            let content: String = msg.get(1)?;
            session.messages.push(ChatMessage {
                role: msg.get(0)?,
                content: serde_json::from_str(&content).unwrap_or(Value::String(content)),
            });
        }
        sessions.push(session);
    }
    Ok(sessions)
}

pub(crate) fn compute_next_session_id(stored_next: Option<usize>, max_existing: usize) -> usize {
    stored_next
        .unwrap_or_else(|| max_existing.saturating_add(1))
        .max(max_existing.saturating_add(1))
        .max(1)
}

pub(crate) fn legacy_active_session_id(
    legacy_index: Option<usize>,
    sessions: &[ChatSession],
) -> Option<usize> {
    if sessions.is_empty() {
        return None;
    }
    Some(
        legacy_index
            .and_then(|index| sessions.get(index).map(|session| session.id))
            .unwrap_or(sessions[0].id),
    )
}

fn allocate_session_id_tx(tx: &rusqlite::Transaction<'_>, user_id: i64) -> rusqlite::Result<usize> {
    let max_existing: usize = tx.query_row(
        "SELECT COALESCE(MAX(session_id),0) FROM sessions WHERE user_id=?1",
        params![user_id],
        |row| row.get(0),
    )?;
    let stored_next = tx
        .query_row(
            "SELECT next_session_id FROM session_counters WHERE user_id=?1",
            params![user_id],
            |row| row.get::<_, usize>(0),
        )
        .ok();
    let next_id = compute_next_session_id(stored_next, max_existing);
    tx.execute(
        "INSERT INTO session_counters(user_id,next_session_id) VALUES(?1,?2)
         ON CONFLICT(user_id) DO UPDATE SET next_session_id=excluded.next_session_id",
        params![user_id, next_id.saturating_add(1)],
    )?;
    Ok(next_id)
}

fn replace_session_messages_if_revision_db(
    user_id: i64,
    expected_revision: u64,
    session: &ChatSession,
) -> rusqlite::Result<bool> {
    let mut conn = open_session_db()?;
    replace_session_messages_if_revision_on_conn(&mut conn, user_id, expected_revision, session)
}

fn replace_session_messages_if_revision_on_conn(
    conn: &mut Connection,
    user_id: i64,
    expected_revision: u64,
    session: &ChatSession,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    let now = Local::now().to_rfc3339();
    let changed = tx.execute(
        "UPDATE sessions SET name=?3,updated_at=?4,revision=?5
         WHERE user_id=?1 AND session_id=?2 AND revision=?6",
        params![
            user_id,
            session.id,
            session.name,
            now,
            session.revision,
            expected_revision
        ],
    )?;
    if changed != 1 {
        return Ok(false);
    }
    tx.execute(
        "DELETE FROM messages WHERE user_id=?1 AND session_id=?2",
        params![user_id, session.id],
    )?;
    for message in &session.messages {
        let content = serde_json::to_string(&message.content).unwrap_or_default();
        tx.execute(
            "INSERT INTO messages(user_id,session_id,role,content,created_at) VALUES(?1,?2,?3,?4,?5)",
            params![user_id, session.id, message.role, content, now],
        )?;
    }
    tx.commit()?;
    Ok(true)
}

#[cfg(test)]
fn append_session_messages_on_conn(
    conn: &mut Connection,
    user_id: i64,
    expected_revision: u64,
    session: &ChatSession,
    messages: &[ChatMessage],
) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    let now = Local::now().to_rfc3339();
    let changed = tx.execute(
        "UPDATE sessions SET name=?3,updated_at=?4 WHERE user_id=?1 AND session_id=?2 AND revision=?5",
        params![user_id, session.id, session.name, now, expected_revision],
    )?;
    if changed != 1 {
        return Ok(false);
    }
    for message in messages {
        let content = serde_json::to_string(&message.content).unwrap_or_default();
        tx.execute(
            "INSERT INTO messages(user_id,session_id,role,content,created_at) VALUES(?1,?2,?3,?4,?5)",
            params![user_id, session.id, message.role, content, now],
        )?;
    }
    tx.commit()?;
    Ok(true)
}

fn save_active_session_db(user_id: i64, session_id: usize) -> rusqlite::Result<()> {
    let conn = open_session_db()?;
    conn.execute(
        "INSERT INTO active_sessions(user_id,session_id) VALUES(?1,?2)
         ON CONFLICT(user_id) DO UPDATE SET session_id=excluded.session_id",
        params![user_id, session_id],
    )?;
    Ok(())
}

fn switch_active_session_db(user_id: i64, session_id: usize) -> rusqlite::Result<bool> {
    let mut conn = open_session_db()?;
    let tx = conn.transaction()?;
    let exists = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE user_id=?1 AND session_id=?2)",
        params![user_id, session_id],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO active_sessions(user_id,session_id) VALUES(?1,?2)
         ON CONFLICT(user_id) DO UPDATE SET session_id=excluded.session_id",
        params![user_id, session_id],
    )?;
    tx.commit()?;
    Ok(true)
}

fn create_session_and_activate_db(
    user_id: i64,
    name: &str,
    created_at: &str,
) -> rusqlite::Result<ChatSession> {
    let mut conn = open_session_db()?;
    let tx = conn.transaction()?;
    let session_id = allocate_session_id_tx(&tx, user_id)?;
    let now = Local::now().to_rfc3339();
    tx.execute(
        "INSERT INTO sessions(user_id,session_id,name,created_at,updated_at,revision)
         VALUES(?1,?2,?3,?4,?5,0)",
        params![user_id, session_id, name, created_at, now],
    )?;
    tx.execute(
        "INSERT INTO active_sessions(user_id,session_id) VALUES(?1,?2)
         ON CONFLICT(user_id) DO UPDATE SET session_id=excluded.session_id",
        params![user_id, session_id],
    )?;
    tx.commit()?;
    Ok(ChatSession {
        id: session_id,
        name: name.to_string(),
        messages: Vec::new(),
        created_at: created_at.to_string(),
        revision: 0,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct RemoveSessionOutcome {
    pub new_active_id: usize,
    pub replacement: Option<ChatSession>,
}

impl RemoveSessionOutcome {
    pub fn is_last_session_reset(&self) -> bool {
        self.replacement.is_some()
    }
}

fn remove_session_transaction_db(
    user_id: i64,
    session_id: usize,
    replacement_name: &str,
    replacement_created_at: &str,
) -> rusqlite::Result<Option<RemoveSessionOutcome>> {
    let mut conn = open_session_db()?;
    remove_session_transaction_on_conn(
        &mut conn,
        user_id,
        session_id,
        replacement_name,
        replacement_created_at,
    )
}

fn remove_session_transaction_on_conn(
    conn: &mut Connection,
    user_id: i64,
    session_id: usize,
    replacement_name: &str,
    replacement_created_at: &str,
) -> rusqlite::Result<Option<RemoveSessionOutcome>> {
    let tx = conn.transaction()?;
    let exists = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE user_id=?1 AND session_id=?2)",
        params![user_id, session_id],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists {
        return Ok(None);
    }
    let count: usize = tx.query_row(
        "SELECT COUNT(*) FROM sessions WHERE user_id=?1",
        params![user_id],
        |row| row.get(0),
    )?;
    let current_active = tx
        .query_row(
            "SELECT session_id FROM active_sessions WHERE user_id=?1",
            params![user_id],
            |row| row.get::<_, usize>(0),
        )
        .ok();

    let replacement = if count == 1 {
        let replacement_id = allocate_session_id_tx(&tx, user_id)?;
        let now = Local::now().to_rfc3339();
        tx.execute(
            "INSERT INTO sessions(user_id,session_id,name,created_at,updated_at,revision)
             VALUES(?1,?2,?3,?4,?5,0)",
            params![
                user_id,
                replacement_id,
                replacement_name,
                replacement_created_at,
                now
            ],
        )?;
        Some(ChatSession {
            id: replacement_id,
            name: replacement_name.to_string(),
            messages: Vec::new(),
            created_at: replacement_created_at.to_string(),
            revision: 0,
        })
    } else {
        None
    };

    tx.execute(
        "DELETE FROM messages WHERE user_id=?1 AND session_id=?2",
        params![user_id, session_id],
    )?;
    tx.execute(
        "DELETE FROM sessions WHERE user_id=?1 AND session_id=?2",
        params![user_id, session_id],
    )?;

    let new_active_id = if let Some(replacement) = &replacement {
        replacement.id
    } else if current_active == Some(session_id)
        || current_active.is_none()
        || !current_active.is_some_and(|active_id| {
            tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE user_id=?1 AND session_id=?2)",
                params![user_id, active_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
        })
    {
        tx.query_row(
            "SELECT session_id FROM sessions WHERE user_id=?1 ORDER BY session_id LIMIT 1",
            params![user_id],
            |row| row.get::<_, usize>(0),
        )?
    } else {
        current_active.unwrap_or_default()
    };

    tx.execute(
        "INSERT INTO active_sessions(user_id,session_id) VALUES(?1,?2)
         ON CONFLICT(user_id) DO UPDATE SET session_id=excluded.session_id",
        params![user_id, new_active_id],
    )?;
    tx.commit()?;
    Ok(Some(RemoveSessionOutcome {
        new_active_id,
        replacement,
    }))
}

fn ensure_session_identity_v2_db(user_id: i64, sessions: &[ChatSession]) -> rusqlite::Result<()> {
    if sessions.is_empty() {
        return Ok(());
    }
    let conn = open_session_db()?;
    let marker = format!("session_identity_v2:{user_id}");
    let migrated = conn
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            params![&marker],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .is_some();

    if !migrated {
        let legacy_value = conn
            .query_row(
                "SELECT session_id FROM active_sessions WHERE user_id=?1",
                params![user_id],
                |row| row.get::<_, usize>(0),
            )
            .ok();
        let stable_id = legacy_active_session_id(legacy_value, sessions).unwrap_or(sessions[0].id);
        save_active_session_db(user_id, stable_id)?;
        conn.execute(
            "INSERT INTO settings(key,value) VALUES(?1,'1') ON CONFLICT(key) DO UPDATE SET value='1'",
            params![&marker],
        )?;
    }

    let next_id = sessions
        .iter()
        .map(|session| session.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .max(1);
    conn.execute(
        "INSERT INTO session_counters(user_id,next_session_id) VALUES(?1,?2)
         ON CONFLICT(user_id) DO UPDATE SET next_session_id=MAX(next_session_id,excluded.next_session_id)",
        params![user_id, next_id],
    )?;
    Ok(())
}

pub(crate) async fn load_sessions_db_async(user_id: i64) -> Vec<ChatSession> {
    run_db("load_sessions", move || load_sessions_db(user_id))
        .await
        .unwrap_or_default()
}

pub(crate) async fn replace_session_messages_if_revision_db_async(
    user_id: i64,
    expected_revision: u64,
    session: ChatSession,
) -> Option<bool> {
    run_db("replace_session_messages_if_revision", move || {
        replace_session_messages_if_revision_db(user_id, expected_revision, &session)
    })
    .await
}

pub(crate) async fn switch_active_session_db_async(
    user_id: i64,
    session_id: usize,
) -> Option<bool> {
    run_db("switch_active_session", move || {
        switch_active_session_db(user_id, session_id)
    })
    .await
}

pub(crate) async fn create_session_and_activate_db_async(
    user_id: i64,
    name: String,
    created_at: String,
) -> Option<ChatSession> {
    run_db("create_session_and_activate", move || {
        create_session_and_activate_db(user_id, &name, &created_at)
    })
    .await
}

pub(crate) async fn remove_session_transaction_db_async(
    user_id: i64,
    session_id: usize,
    replacement_name: String,
    replacement_created_at: String,
) -> Option<Option<RemoveSessionOutcome>> {
    run_db("remove_session_transaction", move || {
        remove_session_transaction_db(
            user_id,
            session_id,
            &replacement_name,
            &replacement_created_at,
        )
    })
    .await
}

pub(crate) async fn ensure_session_identity_v2_db_async(
    user_id: i64,
    sessions: Vec<ChatSession>,
) -> bool {
    run_db("ensure_session_identity_v2", move || {
        ensure_session_identity_v2_db(user_id, &sessions)
    })
    .await
    .is_some()
}

pub(crate) async fn load_active_session_id_db_async(user_id: i64) -> Option<usize> {
    run_db("load_active_session", move || {
        let conn = open_session_db()?;
        conn.query_row(
            "SELECT session_id FROM active_sessions WHERE user_id=?1",
            params![user_id],
            |row| row.get::<_, usize>(0),
        )
    })
    .await
}

pub fn load_scoped_messages(
    chat_id: i64,
    thread_id: i64,
    limit: usize,
) -> rusqlite::Result<Vec<ChatMessage>> {
    let conn = open_session_db()?;
    load_scoped_messages_on_conn(&conn, chat_id, thread_id, limit)
}

fn load_scoped_messages_on_conn(
    conn: &Connection,
    chat_id: i64,
    thread_id: i64,
    limit: usize,
) -> rusqlite::Result<Vec<ChatMessage>> {
    let mut stmt = conn.prepare(
        "SELECT role, content FROM (
            SELECT rowid, role, content FROM messages
            WHERE chat_id = ?1 AND thread_id = ?2
            ORDER BY rowid DESC
            LIMIT ?3
        ) ORDER BY rowid ASC",
    )?;
    let rows = stmt.query_map(params![chat_id, thread_id, limit as i64], |row| {
        let role: String = row.get(0)?;
        let content: String = row.get(1)?;
        let content_val = serde_json::from_str(&content).unwrap_or(Value::String(content));
        Ok(ChatMessage {
            role,
            content: content_val,
        })
    })?;
    rows.collect()
}

pub fn count_scoped_messages(chat_id: i64, thread_id: i64) -> rusqlite::Result<usize> {
    let conn = open_session_db()?;
    count_scoped_messages_on_conn(&conn, chat_id, thread_id)
}

fn count_scoped_messages_on_conn(
    conn: &Connection,
    chat_id: i64,
    thread_id: i64,
) -> rusqlite::Result<usize> {
    conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE chat_id = ?1 AND thread_id = ?2",
        params![chat_id, thread_id],
        |row| row.get(0),
    )
}

pub fn save_scoped_message(
    chat_id: i64,
    thread_id: i64,
    user_id: i64,
    role: &str,
    content: &str,
) -> rusqlite::Result<()> {
    let conn = open_session_db()?;
    save_scoped_message_on_conn(&conn, chat_id, thread_id, user_id, role, content)
}

fn save_scoped_message_on_conn(
    conn: &Connection,
    chat_id: i64,
    thread_id: i64,
    user_id: i64,
    role: &str,
    content: &str,
) -> rusqlite::Result<()> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO messages(user_id, session_id, chat_id, thread_id, role, content, created_at)
         VALUES(?1, 0, ?2, ?3, ?4, ?5, ?6)",
        params![user_id, chat_id, thread_id, role, content, now],
    )?;
    Ok(())
}

pub async fn load_scoped_messages_async(
    chat_id: i64,
    thread_id: i64,
    limit: usize,
) -> Vec<ChatMessage> {
    run_db("load_scoped_messages", move || {
        load_scoped_messages(chat_id, thread_id, limit)
    })
    .await
    .unwrap_or_default()
}

pub async fn count_scoped_messages_async(chat_id: i64, thread_id: i64) -> usize {
    run_db("count_scoped_messages", move || {
        count_scoped_messages(chat_id, thread_id)
    })
    .await
    .unwrap_or_default()
}

pub async fn save_scoped_message_async(
    chat_id: i64,
    thread_id: i64,
    user_id: i64,
    role: String,
    content: String,
) -> bool {
    run_db("save_scoped_message", move || {
        save_scoped_message(chat_id, thread_id, user_id, &role, &content)
    })
    .await
    .is_some()
}

pub fn clear_scoped_messages(chat_id: i64, thread_id: i64) -> rusqlite::Result<()> {
    let conn = open_session_db()?;
    conn.execute(
        "DELETE FROM messages WHERE chat_id=?1 AND thread_id=?2",
        params![chat_id, thread_id],
    )?;
    conn.execute(
        "DELETE FROM scoped_summaries WHERE chat_id=?1 AND thread_id=?2",
        params![chat_id, thread_id],
    )?;
    Ok(())
}

pub async fn clear_scoped_messages_async(chat_id: i64, thread_id: i64) -> bool {
    run_db("clear_scoped_messages", move || {
        clear_scoped_messages(chat_id, thread_id)
    })
    .await
    .is_some()
}

pub fn get_scoped_summary(chat_id: i64, thread_id: i64) -> rusqlite::Result<Option<String>> {
    let conn = open_session_db()?;
    get_scoped_summary_on_conn(&conn, chat_id, thread_id)
}

fn get_scoped_summary_on_conn(
    conn: &Connection,
    chat_id: i64,
    thread_id: i64,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT summary FROM scoped_summaries WHERE chat_id=?1 AND thread_id=?2",
        params![chat_id, thread_id],
        |row| row.get(0),
    )
    .optional()
}

pub fn save_scoped_summary(chat_id: i64, thread_id: i64, summary: &str) -> rusqlite::Result<()> {
    let conn = open_session_db()?;
    save_scoped_summary_on_conn(&conn, chat_id, thread_id, summary)
}

fn save_scoped_summary_on_conn(
    conn: &Connection,
    chat_id: i64,
    thread_id: i64,
    summary: &str,
) -> rusqlite::Result<()> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO scoped_summaries(chat_id, thread_id, summary, updated_at) VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(chat_id, thread_id) DO UPDATE SET summary=excluded.summary, updated_at=excluded.updated_at",
        params![chat_id, thread_id, summary, now],
    )?;
    Ok(())
}

pub async fn get_scoped_summary_async(chat_id: i64, thread_id: i64) -> Option<String> {
    run_db("get_scoped_summary", move || {
        get_scoped_summary(chat_id, thread_id)
    })
    .await
    .flatten()
}

pub async fn save_scoped_summary_async(chat_id: i64, thread_id: i64, summary: String) -> bool {
    run_db("save_scoped_summary", move || {
        save_scoped_summary(chat_id, thread_id, &summary)
    })
    .await
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                user_id INTEGER NOT NULL, session_id INTEGER NOT NULL, name TEXT NOT NULL,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL, revision INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(user_id, session_id)
            );
            CREATE TABLE messages (
                user_id INTEGER NOT NULL, session_id INTEGER NOT NULL DEFAULT 0,
                chat_id INTEGER NOT NULL DEFAULT 0, thread_id INTEGER NOT NULL DEFAULT 0,
                role TEXT NOT NULL, content TEXT NOT NULL, created_at TEXT NOT NULL
            );
            CREATE TABLE active_sessions (user_id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL);
            CREATE TABLE session_counters (user_id INTEGER PRIMARY KEY, next_session_id INTEGER NOT NULL);
            CREATE TABLE telegram_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE telegram_inbox (
                update_id INTEGER PRIMARY KEY, payload_json TEXT NOT NULL, status TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0, received_at TEXT NOT NULL, last_error TEXT
            );
            CREATE TABLE user_memories (
                user_id INTEGER NOT NULL,
                key TEXT NOT NULL,
                fact TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(user_id, key)
            );
            CREATE TABLE scoped_summaries (
                chat_id INTEGER NOT NULL,
                thread_id INTEGER NOT NULL DEFAULT 0,
                summary TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(chat_id, thread_id)
            );
            CREATE INDEX idx_messages_chat_thread ON messages(chat_id, thread_id);
            CREATE INDEX idx_user_memories_user ON user_memories(user_id);",
        )
        .unwrap();
        conn
    }

    fn seed_session(conn: &Connection, revision: u64) {
        conn.execute(
            "INSERT INTO sessions(user_id,session_id,name,created_at,updated_at,revision)
             VALUES(7,3,'Original','now','now',?1)",
            params![revision],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO active_sessions(user_id,session_id) VALUES(7,3)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages(user_id,session_id,role,content,created_at)
             VALUES(7,3,'user','\"hello\"','now')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn clear_transaction_failure_leaves_revision_and_history_unchanged() {
        let mut conn = session_test_conn();
        seed_session(&conn, 4);
        conn.execute_batch(
            "CREATE TRIGGER fail_clear BEFORE DELETE ON messages
             BEGIN SELECT RAISE(ABORT, 'clear failpoint'); END;",
        )
        .unwrap();
        let candidate = ChatSession {
            id: 3,
            name: "Original".to_string(),
            messages: Vec::new(),
            created_at: "now".to_string(),
            revision: 5,
        };
        assert!(replace_session_messages_if_revision_on_conn(&mut conn, 7, 4, &candidate).is_err());
        let revision: u64 = conn
            .query_row(
                "SELECT revision FROM sessions WHERE user_id=7 AND session_id=3",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let messages: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE user_id=7 AND session_id=3",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(revision, 4);
        assert_eq!(messages, 1);
    }

    #[test]
    fn append_failure_does_not_publish_partial_durable_turn() {
        let mut conn = session_test_conn();
        seed_session(&conn, 2);
        conn.execute_batch(
            "CREATE TRIGGER fail_append BEFORE INSERT ON messages
             BEGIN SELECT RAISE(ABORT, 'append failpoint'); END;",
        )
        .unwrap();
        let candidate = ChatSession {
            id: 3,
            name: "Candidate title".to_string(),
            messages: Vec::new(),
            created_at: "now".to_string(),
            revision: 2,
        };
        let appended = vec![ChatMessage {
            role: "assistant".to_string(),
            content: Value::String("answer".to_string()),
        }];
        assert!(append_session_messages_on_conn(&mut conn, 7, 2, &candidate, &appended).is_err());
        let name: String = conn
            .query_row(
                "SELECT name FROM sessions WHERE user_id=7 AND session_id=3",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let messages: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE user_id=7 AND session_id=3",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name, "Original");
        assert_eq!(messages, 1);
    }

    #[test]
    fn stale_generation_revision_is_rejected_without_writes() {
        let mut conn = session_test_conn();
        seed_session(&conn, 9);
        let candidate = ChatSession {
            id: 3,
            name: "Original".to_string(),
            messages: Vec::new(),
            created_at: "now".to_string(),
            revision: 8,
        };
        let result = append_session_messages_on_conn(&mut conn, 7, 8, &candidate, &[]).unwrap();
        assert!(!result);
        let messages: usize = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(messages, 1);
    }

    #[test]
    fn delete_failure_leaves_session_active_state_and_counter_intact() {
        let mut conn = session_test_conn();
        seed_session(&conn, 1);
        conn.execute(
            "INSERT INTO session_counters(user_id,next_session_id) VALUES(7,4)",
            [],
        )
        .unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_delete BEFORE DELETE ON sessions
             BEGIN SELECT RAISE(ABORT, 'delete failpoint'); END;",
        )
        .unwrap();
        assert!(
            remove_session_transaction_on_conn(&mut conn, 7, 3, "Replacement", "later").is_err()
        );
        let session_count: usize = conn
            .query_row("SELECT COUNT(*) FROM sessions WHERE user_id=7", [], |row| {
                row.get(0)
            })
            .unwrap();
        let active: usize = conn
            .query_row(
                "SELECT session_id FROM active_sessions WHERE user_id=7",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let next_id: usize = conn
            .query_row(
                "SELECT next_session_id FROM session_counters WHERE user_id=7",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(session_count, 1);
        assert_eq!(active, 3);
        assert_eq!(next_id, 4);
    }

    #[test]
    fn test_scoped_messages_and_threads() {
        let conn = session_test_conn();
        // Chat 100, Thread 0 (private chat)
        save_scoped_message_on_conn(&conn, 100, 0, 100, "user", "\"Halo!\"").unwrap();
        save_scoped_message_on_conn(&conn, 100, 0, 100, "assistant", "\"Hai ada apa?\"").unwrap();
        // Supergroup -100123, Thread 42
        save_scoped_message_on_conn(&conn, -100123, 42, 100, "user", "\"Topik 42\"").unwrap();
        // Supergroup -100123, Thread 99
        save_scoped_message_on_conn(&conn, -100123, 99, 100, "user", "\"Topik 99\"").unwrap();

        assert_eq!(count_scoped_messages_on_conn(&conn, 100, 0).unwrap(), 2);
        assert_eq!(
            count_scoped_messages_on_conn(&conn, -100123, 42).unwrap(),
            1
        );
        assert_eq!(
            count_scoped_messages_on_conn(&conn, -100123, 99).unwrap(),
            1
        );

        let msgs = load_scoped_messages_on_conn(&conn, 100, 0, 10).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, Value::String("Halo!".to_string()));
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, Value::String("Hai ada apa?".to_string()));

        let thread_msgs = load_scoped_messages_on_conn(&conn, -100123, 42, 10).unwrap();
        assert_eq!(thread_msgs.len(), 1);
        assert_eq!(
            thread_msgs[0].content,
            Value::String("Topik 42".to_string())
        );
    }

    #[test]
    fn test_scoped_summary_crud() {
        let conn = session_test_conn();
        assert_eq!(get_scoped_summary_on_conn(&conn, 100, 0).unwrap(), None);

        save_scoped_summary_on_conn(&conn, 100, 0, "Discussed Rust async patterns").unwrap();
        assert_eq!(
            get_scoped_summary_on_conn(&conn, 100, 0).unwrap(),
            Some("Discussed Rust async patterns".to_string())
        );

        // Different thread
        assert_eq!(get_scoped_summary_on_conn(&conn, 100, 42).unwrap(), None);
        save_scoped_summary_on_conn(&conn, 100, 42, "Topic 42 summary").unwrap();
        assert_eq!(
            get_scoped_summary_on_conn(&conn, 100, 42).unwrap(),
            Some("Topic 42 summary".to_string())
        );

        // Upsert
        save_scoped_summary_on_conn(&conn, 100, 0, "Updated summary").unwrap();
        assert_eq!(
            get_scoped_summary_on_conn(&conn, 100, 0).unwrap(),
            Some("Updated summary".to_string())
        );
    }

    #[test]
    fn test_topic_scope_helpers() {
        let private_scope = TopicScope::new(12345, 0);
        assert!(private_scope.is_private());
        assert_eq!(private_scope.chat_id, 12345);
        assert_eq!(private_scope.thread_id, 0);

        let group_topic_scope = TopicScope::new(-1001234567, 99);
        assert!(!group_topic_scope.is_private());
        assert_eq!(group_topic_scope.chat_id, -1001234567);
        assert_eq!(group_topic_scope.thread_id, 99);
    }
}
