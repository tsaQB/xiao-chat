use crate::gateway::DeliverySink;
use crate::parser::whatsapp::{chunk_whatsapp_message, format_for_whatsapp};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, warn};
use whatsapp_rust::client::Client;
use whatsapp_rust::prelude::*;

#[derive(Clone)]
pub struct WhatsAppDeliverySink {
    client: Arc<Client>,
    jid_cache: Arc<RwLock<HashMap<i64, Jid>>>,
}

impl WhatsAppDeliverySink {
    pub fn new(client: Arc<Client>) -> Self {
        Self {
            client,
            jid_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Menyimpan asosiasi chat_id i64 ke JID asli WhatsApp.
    ///
    /// Menghindari masalah rekonstruksi lossy pada group JID yang memuat karakter non-digit atau hyphen.
    pub async fn remember_jid(&self, chat_id: i64, jid: Jid) {
        let mut cache = self.jid_cache.write().await;
        cache.insert(chat_id, jid);
    }

    /// Mengonversi chat_id XiaoBot (positif untuk DM, negatif untuk grup)
    /// menjadi JID WhatsApp yang valid, memprioritaskan cache JID asli.
    pub async fn resolve_jid(&self, chat_id: i64) -> Option<Jid> {
        if let Some(cached) = self.jid_cache.read().await.get(&chat_id) {
            return Some(cached.clone());
        }
        Self::id_to_jid(chat_id)
    }

    /// Fallback konversi statis chat_id ke format JID standar.
    pub fn id_to_jid(chat_id: i64) -> Option<Jid> {
        let jid_str = if chat_id > 0 {
            format!("{chat_id}@s.whatsapp.net")
        } else if chat_id < 0 {
            format!("{}@g.us", chat_id.abs())
        } else {
            return None;
        };
        Jid::from_str(&jid_str).ok()
    }

    /// Menghentikan indikator mengetik / composing
    pub async fn stop_typing(&self, chat_id: i64) {
        if let Some(jid) = self.resolve_jid(chat_id).await {
            let _ = self.client.chatstate().send_paused(&jid).await;
        }
    }
}

impl DeliverySink for WhatsAppDeliverySink {
    async fn indicate_typing(&self, chat_id: i64) -> Result<(), String> {
        let Some(jid) = self.resolve_jid(chat_id).await else {
            return Err(format!("Invalid chat_id {chat_id} for WhatsApp JID"));
        };
        if let Err(e) = self.client.chatstate().send_composing(&jid).await {
            warn!("Failed to send composing indicator to {jid}: {e}");
        }
        Ok(())
    }

    async fn send_text(
        &self,
        chat_id: i64,
        text: &str,
        _reply_to: Option<i64>,
    ) -> Result<i64, String> {
        let Some(jid) = self.resolve_jid(chat_id).await else {
            return Err(format!("Invalid chat_id {chat_id} for WhatsApp JID"));
        };

        // Format teks Markdown CommonMark ke format WhatsApp Markdown (*bold*, _italic_, ~strike~)
        let formatted = format_for_whatsapp(text);

        // Pecah pesan jika melebihi batas 3.500 karakter per gelembung chat
        let chunks = chunk_whatsapp_message(&formatted, 3500);

        for chunk in chunks {
            let msg = wa::Message::text(chunk);
            if let Err(e) = self.client.send_message(&jid, msg).await {
                error!("Failed to send WhatsApp message to {jid}: {e}");
                return Err(format!("WhatsApp send error: {e}"));
            }
        }

        Ok(chrono::Utc::now().timestamp_millis())
    }
}
