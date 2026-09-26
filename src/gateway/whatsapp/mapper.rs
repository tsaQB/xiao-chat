use crate::gateway::{ChannelKind, InboundEnvelope};
use std::hash::{Hash, Hasher};

/// Mengonversi format JID pengguna WhatsApp (misal "6281234567890@s.whatsapp.net")
/// menjadi integer bertanda 64-bit (`i64`) untuk kompatibilitas skema database XiaoBot.
pub fn parse_user_jid_to_i64(jid_str: &str) -> Option<i64> {
    let user_part = jid_str.split('@').next()?.trim();
    let digits: String = user_part.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<i64>().ok().filter(|&id| id > 0)
}

/// Mengonversi format JID grup WhatsApp (misal "1203631234567890@g.us")
/// menjadi integer negatif 64-bit (`-i64`), konsisten dengan konvensi ID chat grup XiaoBot.
pub fn parse_group_jid_to_i64(jid_str: &str) -> i64 {
    let group_part = jid_str.split('@').next().unwrap_or(jid_str).trim();
    let digits: String = group_part.chars().filter(|c| c.is_ascii_digit()).collect();
    if let Ok(num) = digits.parse::<i64>() {
        -num.abs()
    } else {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        group_part.hash(&mut hasher);
        -(hasher.finish() as i64).abs()
    }
}

/// Memvalidasi otorisasi pengirim berdasarkan Hardened Single-Owner Boundary.
///
/// Kebijakan Keamanan (AGENTS.md §4.1):
/// Hanya nomor pemilik (`owner_number`) yang diizinkan berinteraksi dengan bot.
/// Pesan dari nomor lain diabaikan seketika (silent drop) tanpa log identitas
/// ataupun respon (*zero information leakage*).
pub fn is_sender_authorized(sender_id: i64, owner_number: Option<&str>) -> bool {
    let Some(owner) = owner_number else {
        return false;
    };

    let clean_owner: String = owner.chars().filter(|c| c.is_ascii_digit()).collect();
    if clean_owner.is_empty() {
        return false;
    }

    if let Ok(owner_id) = clean_owner.parse::<i64>() {
        sender_id == owner_id
    } else {
        false
    }
}

/// Membangun `InboundEnvelope` dari data pesan WhatsApp yang tervalidasi.
#[allow(clippy::too_many_arguments)]
pub fn build_inbound_envelope(
    sender_id: i64,
    chat_id: i64,
    text: String,
    sender_name: Option<String>,
    is_group: bool,
    reply_to_id: Option<i64>,
    image_bytes: Option<Vec<u8>>,
    audio_bytes: Option<Vec<u8>>,
    doc_bytes: Option<Vec<u8>>,
    doc_name: Option<String>,
    mime_type: Option<String>,
) -> InboundEnvelope {
    InboundEnvelope {
        channel: ChannelKind::WhatsApp,
        sender_id,
        chat_id,
        thread_id: 0,
        text,
        reply_to_id,
        sender_name,
        is_group,
        image_bytes,
        audio_bytes,
        doc_bytes,
        doc_name,
        mime_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_user_jid() {
        let jid = "6281234567890@s.whatsapp.net";
        assert_eq!(parse_user_jid_to_i64(jid), Some(6281234567890));

        let invalid = "invalid_user@s.whatsapp.net";
        assert_eq!(parse_user_jid_to_i64(invalid), None);
    }

    #[test]
    fn test_parse_group_jid() {
        let group_jid = "120363028384910293@g.us";
        let group_id = parse_group_jid_to_i64(group_jid);
        assert!(group_id < 0);
        assert_eq!(group_id, -120363028384910293);
    }

    #[test]
    fn test_single_owner_authorization() {
        let owner = "6281234567890";

        // Nomor pemilik sah -> diizinkan
        assert!(is_sender_authorized(6281234567890, Some(owner)));

        // Nomor orang asing -> ditolak seketika (silent drop)
        assert!(!is_sender_authorized(6289999999999, Some(owner)));

        // Tanpa pemilik dikonfigurasi -> ditolak semua
        assert!(!is_sender_authorized(6281234567890, None));
    }
}
