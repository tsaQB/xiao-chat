use std::path::{Path, PathBuf};

pub mod client;
pub mod delivery;
pub mod mapper;

pub use delivery::WhatsAppDeliverySink;

#[derive(Debug, Clone)]
pub struct WhatsAppConfig {
    pub db_path: PathBuf,
    pub owner_number: Option<String>,
    pub phone_login: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhatsAppStatus {
    Unconfigured,
    Linked,
    Unlinked,
}

pub struct WhatsAppGateway;

impl WhatsAppGateway {
    /// Memeriksa status keberadaan sesi login WhatsApp pada path database lokal.
    ///
    /// Menghindari false-positive dengan memverifikasi bahwa tabel `device`
    /// benar-benar memuat kredensial sesi multi-device yang telah terverifikasi (`account IS NOT NULL`).
    pub fn check_status(db_path: &Path) -> WhatsAppStatus {
        if !db_path.exists() {
            return WhatsAppStatus::Unlinked;
        }

        let conn = match rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(c) => c,
            Err(_) => return WhatsAppStatus::Unlinked,
        };

        let table_exists: bool = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='device'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count > 0)
            .unwrap_or(false);

        if !table_exists {
            return WhatsAppStatus::Unlinked;
        }

        let is_linked: bool = conn
            .query_row(
                "SELECT count(*) FROM device WHERE account IS NOT NULL AND length(account) > 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count > 0)
            .unwrap_or(false);

        if is_linked {
            WhatsAppStatus::Linked
        } else {
            WhatsAppStatus::Unlinked
        }
    }

    /// Menghapus file database sesi WhatsApp (Logout / Unlink).
    pub fn logout(db_path: &Path) -> std::io::Result<()> {
        if db_path.exists() {
            std::fs::remove_file(db_path)?;
            let wal = format!("{}-wal", db_path.display());
            let shm = format!("{}-shm", db_path.display());
            let _ = std::fs::remove_file(wal);
            let _ = std::fs::remove_file(shm);
        }
        Ok(())
    }

    /// Menjalankan loop client WhatsApp
    pub async fn start(
        config: WhatsAppConfig,
        ai_service: std::sync::Arc<crate::ai::AIChatService>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        client::WhatsAppClientRunner::run(config, ai_service, None).await
    }
}
