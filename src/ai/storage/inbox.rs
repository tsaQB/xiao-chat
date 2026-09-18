use chrono::Local;
use rusqlite::{params, Connection};

use super::{open_session_db, run_db};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramInboxRecord {
    pub update_id: i64,
    pub payload_json: String,
    pub attempts: i64,
}

fn load_telegram_offset_db() -> rusqlite::Result<Option<i64>> {
    let conn = open_session_db()?;
    let value = conn
        .query_row(
            "SELECT value FROM telegram_state WHERE key='offset'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();
    Ok(value.and_then(|value| value.parse::<i64>().ok()))
}

fn enqueue_telegram_update_db(update_id: i64, payload_json: &str) -> rusqlite::Result<bool> {
    let mut conn = open_session_db()?;
    enqueue_telegram_update_on_conn(&mut conn, update_id, payload_json)
}

fn enqueue_telegram_update_on_conn(
    conn: &mut Connection,
    update_id: i64,
    payload_json: &str,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO telegram_inbox(update_id,payload_json,status,attempts,received_at)
         VALUES(?1,?2,'pending',0,?3)",
        params![update_id, payload_json, Local::now().to_rfc3339()],
    )? == 1;
    tx.execute(
        "INSERT INTO telegram_state(key,value) VALUES('offset',?1)
         ON CONFLICT(key) DO UPDATE SET value=
           CASE
             WHEN CAST(excluded.value AS INTEGER) > CAST(value AS INTEGER)
             THEN excluded.value ELSE value
           END",
        params![update_id.saturating_add(1).to_string()],
    )?;
    // Completed tombstones deduplicate a repeated Telegram delivery. Keep a
    // bounded recent window so the inbox itself cannot grow forever.
    tx.execute(
        "DELETE FROM telegram_inbox
         WHERE status='completed' AND update_id < (
           SELECT COALESCE(MAX(update_id),0) - 5000 FROM telegram_inbox
         )",
        [],
    )?;
    tx.commit()?;
    Ok(inserted)
}

fn recover_telegram_processing_db() -> rusqlite::Result<usize> {
    let conn = open_session_db()?;
    recover_telegram_processing_on_conn(&conn)
}

fn pending_telegram_updates_after_db(
    after_update_id: i64,
    limit: usize,
) -> rusqlite::Result<Vec<TelegramInboxRecord>> {
    let conn = open_session_db()?;
    pending_telegram_updates_after_on_conn(&conn, after_update_id, limit)
}

fn pending_telegram_updates_after_on_conn(
    conn: &Connection,
    after_update_id: i64,
    limit: usize,
) -> rusqlite::Result<Vec<TelegramInboxRecord>> {
    let mut stmt = conn.prepare(
        "SELECT update_id,payload_json,attempts
         FROM telegram_inbox
         WHERE status='pending' AND update_id>?1
         ORDER BY update_id
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![after_update_id, limit as i64], |row| {
        Ok(TelegramInboxRecord {
            update_id: row.get(0)?,
            payload_json: row.get(1)?,
            attempts: row.get(2)?,
        })
    })?;
    rows.collect()
}

fn recover_telegram_processing_on_conn(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE telegram_inbox
         SET status='pending',last_error='recovered after daemon stopped while processing'
         WHERE status='processing'",
        [],
    )
}

fn mark_telegram_processing_claim_on_conn(
    conn: &Connection,
    update_id: i64,
) -> rusqlite::Result<Option<i64>> {
    // Keep the payload until the handler reaches its completed checkpoint. If
    // XiaoAI crashes immediately after this claim, startup recovery can safely
    // make the update pending again instead of losing it forever. This gives
    // the inbox explicit at-least-once processing semantics; a crash after an
    // external side effect but before completion can still repeat that effect.
    let affected = conn.execute(
        "UPDATE telegram_inbox
         SET status='processing',attempts=attempts+1,last_error=NULL
         WHERE update_id=?1 AND status='pending'",
        params![update_id],
    )?;
    if affected == 0 {
        return Ok(None);
    }
    let attempts: i64 = conn.query_row(
        "SELECT attempts FROM telegram_inbox WHERE update_id=?1",
        params![update_id],
        |row| row.get(0),
    )?;
    Ok(Some(attempts))
}

fn mark_telegram_processing_claim_db(update_id: i64) -> rusqlite::Result<Option<i64>> {
    let conn = open_session_db()?;
    mark_telegram_processing_claim_on_conn(&conn, update_id)
}

#[cfg(test)]
fn mark_telegram_processing_on_conn(conn: &Connection, update_id: i64) -> rusqlite::Result<bool> {
    Ok(mark_telegram_processing_claim_on_conn(conn, update_id)?.is_some())
}

fn mark_telegram_processing_retry_on_conn(
    conn: &Connection,
    update_id: i64,
    reason: &str,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE telegram_inbox
         SET status='pending',last_error=?2
         WHERE update_id=?1 AND status='processing'",
        params![update_id, reason],
    )? == 1)
}

fn mark_telegram_processing_retry_db(update_id: i64, reason: &str) -> rusqlite::Result<bool> {
    let conn = open_session_db()?;
    mark_telegram_processing_retry_on_conn(&conn, update_id, reason)
}

fn mark_telegram_processing_failed_on_conn(
    conn: &Connection,
    update_id: i64,
    reason: &str,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE telegram_inbox
         SET status='failed',last_error=?2
         WHERE update_id=?1 AND status='processing'",
        params![update_id, reason],
    )? == 1)
}

fn mark_telegram_processing_failed_db(update_id: i64, reason: &str) -> rusqlite::Result<bool> {
    let conn = open_session_db()?;
    mark_telegram_processing_failed_on_conn(&conn, update_id, reason)
}

fn mark_telegram_processed_db(update_id: i64) -> rusqlite::Result<bool> {
    let conn = open_session_db()?;
    mark_telegram_processed_on_conn(&conn, update_id)
}

fn mark_telegram_processed_on_conn(conn: &Connection, update_id: i64) -> rusqlite::Result<bool> {
    let scrubbed = serde_json::json!({
        "update_id": update_id,
        "payload": "redacted_after_completion"
    })
    .to_string();
    Ok(conn.execute(
        "UPDATE telegram_inbox
         SET status='completed',payload_json=?2,last_error=NULL
         WHERE update_id=?1 AND status='processing'",
        params![update_id, scrubbed],
    )? == 1)
}

pub async fn load_telegram_offset_async() -> Option<i64> {
    run_db("load_telegram_offset", load_telegram_offset_db)
        .await
        .flatten()
}

pub async fn enqueue_telegram_update_async(update_id: i64, payload_json: String) -> Option<bool> {
    run_db("enqueue_telegram_update", move || {
        enqueue_telegram_update_db(update_id, &payload_json)
    })
    .await
}

pub async fn pending_telegram_updates_after_async(
    after_update_id: i64,
    limit: usize,
) -> Vec<TelegramInboxRecord> {
    run_db("pending_telegram_updates_after", move || {
        pending_telegram_updates_after_db(after_update_id, limit)
    })
    .await
    .unwrap_or_default()
}

pub async fn recover_telegram_processing_async() -> usize {
    run_db(
        "recover_telegram_processing",
        recover_telegram_processing_db,
    )
    .await
    .unwrap_or_default()
}

pub async fn mark_telegram_processing_claim_async(update_id: i64) -> Option<i64> {
    run_db("mark_telegram_processing_claim", move || {
        mark_telegram_processing_claim_db(update_id)
    })
    .await
    .flatten()
}

pub async fn mark_telegram_processing_async(update_id: i64) -> bool {
    mark_telegram_processing_claim_async(update_id)
        .await
        .is_some()
}

pub async fn mark_telegram_processing_retry_async(update_id: i64, reason: &str) -> bool {
    let reason = reason.to_string();
    run_db("mark_telegram_processing_retry", move || {
        mark_telegram_processing_retry_db(update_id, &reason)
    })
    .await
    .unwrap_or(false)
}

pub async fn mark_telegram_processing_failed_async(update_id: i64, reason: &str) -> bool {
    let reason = reason.to_string();
    run_db("mark_telegram_processing_failed", move || {
        mark_telegram_processing_failed_db(update_id, &reason)
    })
    .await
    .unwrap_or(false)
}

pub async fn mark_telegram_processed_async(update_id: i64) -> bool {
    run_db("mark_telegram_processed", move || {
        mark_telegram_processed_db(update_id)
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbox_test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE telegram_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE telegram_inbox (
                update_id INTEGER PRIMARY KEY, payload_json TEXT NOT NULL, status TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0, received_at TEXT NOT NULL, last_error TEXT
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn telegram_claim_crash_is_recoverable_and_completed_updates_deduplicate() {
        let mut conn = inbox_test_conn();
        let payload = r#"{"update_id":42,"message":{"text":"hello"}}"#;
        assert!(enqueue_telegram_update_on_conn(&mut conn, 42, payload).unwrap());
        assert!(mark_telegram_processing_on_conn(&conn, 42).unwrap());
        let claimed: (String, String) = conn
            .query_row(
                "SELECT status,payload_json FROM telegram_inbox WHERE update_id=42",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(claimed.0, "processing");
        assert_eq!(claimed.1, payload);

        assert_eq!(recover_telegram_processing_on_conn(&conn).unwrap(), 1);
        let recovered: String = conn
            .query_row(
                "SELECT status FROM telegram_inbox WHERE update_id=42",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recovered, "pending");

        assert!(mark_telegram_processing_on_conn(&conn, 42).unwrap());
        assert!(mark_telegram_processed_on_conn(&conn, 42).unwrap());
        assert_eq!(recover_telegram_processing_on_conn(&conn).unwrap(), 0);
        assert!(!enqueue_telegram_update_on_conn(&mut conn, 42, payload).unwrap());
        let completed: (String, String) = conn
            .query_row(
                "SELECT status,payload_json FROM telegram_inbox WHERE update_id=42",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(completed.0, "completed");
        assert!(!completed.1.contains("hello"));
    }

    #[test]
    fn telegram_pending_updates_page_past_first_500_without_replaying_queued_rows() {
        let mut conn = inbox_test_conn();
        for update_id in 1..=501 {
            let payload = format!(r#"{{"update_id":{update_id}}}"#);
            assert!(enqueue_telegram_update_on_conn(&mut conn, update_id, &payload).unwrap());
        }

        let first = pending_telegram_updates_after_on_conn(&conn, i64::MIN, 500).unwrap();
        assert_eq!(first.len(), 500);
        assert_eq!(first.first().map(|record| record.update_id), Some(1));
        assert_eq!(first.last().map(|record| record.update_id), Some(500));

        let second = pending_telegram_updates_after_on_conn(&conn, 500, 500).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].update_id, 501);
    }

    #[test]
    fn telegram_pending_updates_replayed_and_marked_processing_then_processed() {
        let mut conn = inbox_test_conn();
        assert!(enqueue_telegram_update_on_conn(&mut conn, 10, r#"{"update_id":10}"#).unwrap());
        assert!(enqueue_telegram_update_on_conn(&mut conn, 11, r#"{"update_id":11}"#).unwrap());

        let pending = pending_telegram_updates_after_on_conn(&conn, i64::MIN, 10).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].update_id, 10);
        assert_eq!(pending[1].update_id, 11);

        assert!(mark_telegram_processing_on_conn(&conn, 10).unwrap());
        let pending_after_first =
            pending_telegram_updates_after_on_conn(&conn, i64::MIN, 10).unwrap();
        assert_eq!(pending_after_first.len(), 1);
        assert_eq!(pending_after_first[0].update_id, 11);

        assert!(mark_telegram_processed_on_conn(&conn, 10).unwrap());
        assert_eq!(
            pending_telegram_updates_after_on_conn(&conn, i64::MIN, 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn telegram_inbox_attempts_increment_on_claim_and_retry_preserves_count() {
        let mut conn = inbox_test_conn();
        assert!(enqueue_telegram_update_on_conn(&mut conn, 100, r#"{"update_id":100}"#).unwrap());

        // First claim: attempts becomes 1
        assert_eq!(
            mark_telegram_processing_claim_on_conn(&conn, 100).unwrap(),
            Some(1)
        );

        // Mark retry: status becomes pending again
        assert!(mark_telegram_processing_retry_on_conn(&conn, 100, "transient error").unwrap());

        let pending = pending_telegram_updates_after_on_conn(&conn, 99, 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].update_id, 100);
        assert_eq!(pending[0].attempts, 1);

        // Second claim: attempts becomes 2
        assert_eq!(
            mark_telegram_processing_claim_on_conn(&conn, 100).unwrap(),
            Some(2)
        );

        // Mark failed (quarantine): status becomes failed
        assert!(mark_telegram_processing_failed_on_conn(&conn, 100, "poison pill").unwrap());

        // Failed records do not show in pending
        let pending_after_fail = pending_telegram_updates_after_on_conn(&conn, 99, 10).unwrap();
        assert!(pending_after_fail.is_empty());
    }
}
