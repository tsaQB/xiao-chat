use chrono::Local;
use rusqlite::{params, Connection};

use super::{open_session_db, run_db};

pub fn get_user_memories(user_id: i64) -> rusqlite::Result<Vec<(String, String)>> {
    let conn = open_session_db()?;
    get_user_memories_on_conn(&conn, user_id)
}

fn get_user_memories_on_conn(
    conn: &Connection,
    user_id: i64,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt =
        conn.prepare("SELECT key, fact FROM user_memories WHERE user_id = ?1 ORDER BY key ASC")?;
    let rows = stmt.query_map(params![user_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

pub fn save_user_memory(user_id: i64, key: &str, fact: &str) -> rusqlite::Result<()> {
    let conn = open_session_db()?;
    save_user_memory_on_conn(&conn, user_id, key, fact)
}

fn save_user_memory_on_conn(
    conn: &Connection,
    user_id: i64,
    key: &str,
    fact: &str,
) -> rusqlite::Result<()> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO user_memories(user_id, key, fact, updated_at) VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(user_id, key) DO UPDATE SET fact = excluded.fact, updated_at = excluded.updated_at",
        params![user_id, key, fact, now],
    )?;
    Ok(())
}

pub fn delete_user_memory(user_id: i64, key: &str) -> rusqlite::Result<()> {
    let conn = open_session_db()?;
    delete_user_memory_on_conn(&conn, user_id, key)
}

fn delete_user_memory_on_conn(conn: &Connection, user_id: i64, key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM user_memories WHERE user_id = ?1 AND key = ?2",
        params![user_id, key],
    )?;
    Ok(())
}

pub fn clear_user_memories(user_id: i64) -> rusqlite::Result<()> {
    let conn = open_session_db()?;
    clear_user_memories_on_conn(&conn, user_id)
}

fn clear_user_memories_on_conn(conn: &Connection, user_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM user_memories WHERE user_id = ?1",
        params![user_id],
    )?;
    Ok(())
}

pub async fn get_user_memories_async(user_id: i64) -> Vec<(String, String)> {
    run_db("get_user_memories", move || get_user_memories(user_id))
        .await
        .unwrap_or_default()
}

pub async fn save_user_memory_async(user_id: i64, key: String, fact: String) -> bool {
    run_db("save_user_memory", move || {
        save_user_memory(user_id, &key, &fact)
    })
    .await
    .is_some()
}

pub async fn delete_user_memory_async(user_id: i64, key: String) -> bool {
    run_db("delete_user_memory", move || {
        delete_user_memory(user_id, &key)
    })
    .await
    .is_some()
}

pub async fn clear_user_memories_async(user_id: i64) -> bool {
    run_db("clear_user_memories", move || clear_user_memories(user_id))
        .await
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE user_memories (
                user_id INTEGER NOT NULL,
                key TEXT NOT NULL,
                fact TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(user_id, key)
            );
            CREATE INDEX idx_user_memories_user ON user_memories(user_id);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_user_memories_crud() {
        let conn = memory_test_conn();
        let user_id = 999;
        assert!(get_user_memories_on_conn(&conn, user_id)
            .unwrap()
            .is_empty());

        save_user_memory_on_conn(&conn, user_id, "Name", "Alice").unwrap();
        save_user_memory_on_conn(&conn, user_id, "Language", "Rust").unwrap();

        let memories = get_user_memories_on_conn(&conn, user_id).unwrap();
        assert_eq!(memories.len(), 2);
        assert_eq!(memories[0], ("Language".to_string(), "Rust".to_string()));
        assert_eq!(memories[1], ("Name".to_string(), "Alice".to_string()));

        // Upsert
        save_user_memory_on_conn(&conn, user_id, "Language", "Rust & Go").unwrap();
        let memories = get_user_memories_on_conn(&conn, user_id).unwrap();
        assert_eq!(memories.len(), 2);
        assert_eq!(
            memories[0],
            ("Language".to_string(), "Rust & Go".to_string())
        );

        // Delete single
        delete_user_memory_on_conn(&conn, user_id, "Language").unwrap();
        let memories = get_user_memories_on_conn(&conn, user_id).unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0], ("Name".to_string(), "Alice".to_string()));

        // Clear all
        clear_user_memories_on_conn(&conn, user_id).unwrap();
        let memories = get_user_memories_on_conn(&conn, user_id).unwrap();
        assert!(memories.is_empty());
    }
}
