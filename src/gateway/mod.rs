pub mod whatsapp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Telegram,
    WhatsApp,
    Terminal,
}

#[derive(Debug, Clone)]
pub struct InboundEnvelope {
    pub channel: ChannelKind,
    pub sender_id: i64,
    pub chat_id: i64,
    pub thread_id: i64,
    pub text: String,
    pub reply_to_id: Option<i64>,
    pub sender_name: Option<String>,
    pub is_group: bool,
    pub image_bytes: Option<Vec<u8>>,
    pub audio_bytes: Option<Vec<u8>>,
    pub doc_bytes: Option<Vec<u8>>,
    pub doc_name: Option<String>,
    pub mime_type: Option<String>,
}

#[allow(async_fn_in_trait)]
pub trait DeliverySink: Send + Sync {
    async fn indicate_typing(&self, chat_id: i64) -> Result<(), String>;
    async fn send_text(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i64>,
    ) -> Result<i64, String>;
}
