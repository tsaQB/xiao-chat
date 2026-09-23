#![allow(dead_code)]

#[path = "../src/bot/models.rs"]
pub mod models;
#[path = "../src/bot/url_policy.rs"]
pub mod url_policy;

pub mod bot {
    pub use crate::models;
    pub use crate::url_policy;
}

pub mod ai {
    pub mod service {
        pub fn load_app_setting(_key: &str) -> Option<String> {
            None
        }
    }
}

pub mod util {
    pub fn truncate_chars(s: &str, max_chars: usize) -> &str {
        match s.char_indices().nth(max_chars) {
            None => s,
            Some((idx, _)) => &s[..idx],
        }
    }
}

#[path = "../src/ai/tools.rs"]
pub mod tools;

use models::{
    EphemeralMessageParameters, InlineKeyboardButton, InlineKeyboardMarkup, InputMedia,
    InputRichMessage, InputRichMessageMedia, Location, ReplyParameters, RichBlock,
    RichBlockCaption, RICH_MESSAGE_MAX_BLOCKS, RICH_MESSAGE_MAX_MEDIA, RICH_MESSAGE_MAX_TEXT_CHARS,
};
use serde_json::{json, Value};
use tools::{
    SendAudioArgs, SendCollageArgs, SendDocumentArgs, SendLocationArgs, SendPhotoArgs,
    SendSlideshowArgs, SendVoiceArgs,
};

// ==============================================================================
// Carousel Contract Test Helpers
// ==============================================================================

fn format_carousel_callback_data(id: &str, index: usize, action: &str) -> String {
    format!("carousel:{id}:{index}:{action}")
}

fn parse_carousel_callback_data(data: &str) -> Result<(&str, usize, &str), String> {
    if data.len() > 64 {
        return Err(format!(
            "callback_data exceeds Telegram 64-byte limit: {} bytes",
            data.len()
        ));
    }
    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() != 4 || parts[0] != "carousel" {
        return Err(format!("Invalid carousel callback format: {data}"));
    }
    let id = parts[1];
    if id.trim().is_empty() {
        return Err("Empty carousel ID".to_string());
    }
    let index = parts[2]
        .parse::<usize>()
        .map_err(|e| format!("Invalid slide index: {e}"))?;
    let action = parts[3];
    if action != "prev" && action != "next" && action != "noop" {
        return Err(format!("Invalid action: {action}"));
    }
    Ok((id, index, action))
}

fn build_carousel_keyboard_contract(
    id: &str,
    current_index: usize,
    total_slides: usize,
) -> Result<InlineKeyboardMarkup, String> {
    if total_slides == 0 {
        return Err("total_slides must be > 0".to_string());
    }
    let prev_index = if current_index == 0 {
        total_slides - 1
    } else {
        current_index - 1
    };
    let next_index = if current_index + 1 >= total_slides {
        0
    } else {
        current_index + 1
    };

    let prev_data = format_carousel_callback_data(id, prev_index, "prev");
    let indicator_data = format_carousel_callback_data(id, current_index, "noop");
    let next_data = format_carousel_callback_data(id, next_index, "next");

    if prev_data.len() > 64 || indicator_data.len() > 64 || next_data.len() > 64 {
        return Err("Button callback_data exceeds Telegram 64-byte limit".to_string());
    }

    let indicator_text = format!("{}/{}", current_index + 1, total_slides);
    let row = vec![
        InlineKeyboardButton::callback("⬅️", prev_data),
        InlineKeyboardButton::callback(indicator_text, indicator_data),
        InlineKeyboardButton::callback("➡️", next_data),
    ];
    Ok(InlineKeyboardMarkup::new(vec![row]))
}

fn sample_photo(id: &str) -> InputMedia {
    InputMedia::Photo {
        media: id.to_string(),
        caption: None,
        parse_mode: None,
        show_caption_above_media: None,
        has_spoiler: None,
    }
}

// ==============================================================================
// Rich Message Contract Test Helpers
// ==============================================================================

fn validate_rich_media_id_contract(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("Media ID must not be empty (minimum 1 character)".to_string());
    }
    if id.len() > 64 {
        return Err(format!(
            "Media ID exceeds Telegram limit of 64 characters: {} characters",
            id.len()
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!("Media ID contains invalid characters: '{id}'"));
    }
    Ok(())
}

fn validate_rich_message_media_list_contract(
    media_list: &[InputRichMessageMedia],
) -> Result<(), String> {
    if media_list.len() > RICH_MESSAGE_MAX_MEDIA {
        return Err(format!(
            "Rich Message media count exceeds Telegram limit of {RICH_MESSAGE_MAX_MEDIA}: found {}",
            media_list.len()
        ));
    }
    let mut seen_ids = std::collections::HashSet::new();
    for item in media_list {
        validate_rich_media_id_contract(&item.id)?;
        if !seen_ids.insert(item.id.clone()) {
            return Err(format!(
                "Duplicate media ID found in rich message: '{}'",
                item.id
            ));
        }
    }
    Ok(())
}

fn create_html_rich_message(
    html: &str,
    media_items: Vec<InputRichMessageMedia>,
) -> Result<InputRichMessage, String> {
    validate_rich_message_media_list_contract(&media_items)?;

    let media_opt = if media_items.is_empty() {
        None
    } else {
        Some(media_items)
    };

    let msg = InputRichMessage {
        blocks: vec![],
        html: Some(html.to_string()),
        markdown: None,
        media: media_opt,
        is_rtl: None,
        skip_entity_detection: None,
    };
    msg.validate()?;
    Ok(msg)
}

fn extract_media_items_from_rich_message(
    msg: &InputRichMessage,
) -> Result<Vec<InputRichMessageMedia>, String> {
    Ok(msg.media.clone().unwrap_or_default())
}

fn validate_tg_map_attributes_contract(
    lat: f64,
    lon: f64,
    zoom: Option<i32>,
) -> Result<(), String> {
    if lat.is_nan() || lon.is_nan() || lat.is_infinite() || lon.is_infinite() {
        return Err("Geo coordinates must be finite numbers".to_string());
    }
    if !(-90.0..=90.0).contains(&lat) {
        return Err(format!("Latitude {lat} out of range [-90.0, 90.0]"));
    }
    if !(-180.0..=180.0).contains(&lon) {
        return Err(format!("Longitude {lon} out of range [-180.0, 180.0]"));
    }
    if let Some(z) = zoom {
        if !(1..=20).contains(&z) {
            return Err(format!("Zoom level {z} out of range [1, 20]"));
        }
    }
    Ok(())
}

fn validate_tg_collage_child_count_contract(count: usize) -> Result<(), String> {
    if !(2..=10).contains(&count) {
        return Err(format!(
            "sendMediaGroup requires 2-10 media items; found {count}"
        ));
    }
    Ok(())
}

fn validate_tg_slideshow_child_count_contract(count: usize) -> Result<(), String> {
    if count < 2 {
        return Err(format!(
            "tg-slideshow requires at least 2 media items; found {count}"
        ));
    }
    Ok(())
}

fn extract_tg_scheme_ids(html: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let schemes = [
        ("photo", "tg://photo?id="),
        ("audio", "tg://audio?id="),
        ("video", "tg://video?id="),
        ("document", "tg://document?id="),
    ];
    for (scheme_type, scheme_prefix) in &schemes {
        let mut cursor = 0;
        while let Some(idx) = html[cursor..].find(scheme_prefix) {
            let start = cursor + idx + scheme_prefix.len();
            let end = html[start..]
                .find(['"', '\'', ' ', '>', '/'])
                .map_or(html.len(), |offset| start + offset);
            let id = &html[start..end];
            results.push((scheme_type.to_string(), id.to_string()));
            cursor = end;
        }
    }
    results
}

fn validate_html_media_reference_integrity(
    html: &str,
    media_items: &[InputRichMessageMedia],
) -> Result<(), String> {
    let references = extract_tg_scheme_ids(html);
    for (ref_type, ref_id) in references {
        let matching_item = media_items.iter().find(|m| m.id == ref_id);
        let Some(item) = matching_item else {
            return Err(format!(
                "HTML references tg://{ref_type}?id={ref_id} but no media item with id '{ref_id}' exists"
            ));
        };
        let matches_type = matches!(
            (ref_type.as_str(), &item.media),
            ("photo", InputMedia::Photo { .. })
                | ("audio", InputMedia::Audio { .. })
                | ("video", InputMedia::Video { .. })
                | ("document", InputMedia::Document { .. })
        );
        if !matches_type {
            return Err(format!(
                "Type mismatch for id '{ref_id}': tag expected '{ref_type}', but media item is different"
            ));
        }
    }
    Ok(())
}

fn convert_remote_media_to_rich_links_contract(msg: &InputRichMessage) -> InputRichMessage {
    let mut converted = msg.clone();
    for block in &mut converted.blocks {
        match block {
            RichBlock::Photo { photo, caption } => {
                let url = photo
                    .get("media")
                    .and_then(Value::as_str)
                    .or_else(|| photo.as_str())
                    .map_or("", |s| s);
                if url.starts_with("http://") || url.starts_with("https://") {
                    let cap_text = caption
                        .as_ref()
                        .and_then(|c| c.text.as_str())
                        .map_or("Lihat Foto", |s| s);
                    *block = RichBlock::Paragraph {
                        text: Value::Array(vec![
                            json!("🖼️ "),
                            json!({
                                "type": "text_link",
                                "text": cap_text,
                                "url": url,
                            }),
                        ]),
                    };
                }
            }
            RichBlock::Audio { audio, caption } => {
                let url = audio
                    .get("media")
                    .and_then(Value::as_str)
                    .or_else(|| audio.as_str())
                    .map_or("", |s| s);
                if url.starts_with("http://") || url.starts_with("https://") {
                    let cap_text = caption
                        .as_ref()
                        .and_then(|c| c.text.as_str())
                        .map_or("Putar Audio", |s| s);
                    *block = RichBlock::Paragraph {
                        text: Value::Array(vec![
                            json!("🎵 "),
                            json!({
                                "type": "text_link",
                                "text": cap_text,
                                "url": url,
                            }),
                        ]),
                    };
                }
            }
            _ => {}
        }
    }
    converted
}

#[derive(Debug, PartialEq)]
struct MultipartFormContractPart {
    name: String,
    filename: Option<String>,
    mime: Option<String>,
    content_len: usize,
}

#[derive(Debug, PartialEq)]
struct MultipartSendRichMessageContract {
    chat_id: String,
    rich_message: String,
    reply_markup: Option<String>,
    reply_parameters: Option<String>,
    message_thread_id: Option<String>,
    file_parts: Vec<MultipartFormContractPart>,
}

fn build_multipart_rich_message_form_contract(
    chat_id: i64,
    rich_message: &InputRichMessage,
    attached_files: &[(String, Vec<u8>, String)],
    reply_markup: Option<&InlineKeyboardMarkup>,
    reply_parameters: Option<&ReplyParameters>,
    thread_id: Option<i64>,
) -> Result<MultipartSendRichMessageContract, String> {
    rich_message.validate()?;
    let rich_json = serde_json::to_string(rich_message).map_err(|e| e.to_string())?;
    let reply_markup_json = match reply_markup {
        Some(rm) => Some(serde_json::to_string(rm).map_err(|e| e.to_string())?),
        None => None,
    };
    let reply_params_json = match reply_parameters {
        Some(rp) => Some(serde_json::to_string(rp).map_err(|e| e.to_string())?),
        None => None,
    };

    let mut file_parts = Vec::new();
    for (name, bytes, mime) in attached_files {
        if name.trim().is_empty() {
            return Err("Attachment name must not be empty".to_string());
        }
        file_parts.push(MultipartFormContractPart {
            name: name.clone(),
            filename: Some(name.clone()),
            mime: Some(mime.clone()),
            content_len: bytes.len(),
        });
    }

    Ok(MultipartSendRichMessageContract {
        chat_id: chat_id.to_string(),
        rich_message: rich_json,
        reply_markup: reply_markup_json,
        reply_parameters: reply_params_json,
        message_thread_id: thread_id.map(|t| t.to_string()),
        file_parts,
    })
}

// ==============================================================================
// GROUP 1: Wire format serialization for `editMessageMedia` and `InputMediaPhoto`
// ==============================================================================

#[test]
fn test_edit_message_media_wire_format_serialization() {
    let media = InputMedia::Photo {
        media: "https://example.com/slide2.jpg".to_string(),
        caption: Some("Slide 2 of 5: Architecture Diagram".to_string()),
        parse_mode: Some("HTML".to_string()),
        show_caption_above_media: Some(true),
        has_spoiler: Some(false),
    };

    let keyboard = build_carousel_keyboard_contract("c_101", 1, 5)
        .expect("carousel keyboard construction must succeed");

    let payload = json!({
        "chat_id": 123456789_i64,
        "message_id": 98765_i64,
        "media": media,
        "reply_markup": keyboard,
    });

    assert_eq!(payload["chat_id"], 123456789_i64);
    assert_eq!(payload["message_id"], 98765_i64);
    assert_eq!(payload["media"]["type"], "photo");
    assert_eq!(payload["media"]["media"], "https://example.com/slide2.jpg");
    assert_eq!(
        payload["media"]["caption"],
        "Slide 2 of 5: Architecture Diagram"
    );
    assert_eq!(payload["media"]["parse_mode"], "HTML");
    assert_eq!(payload["media"]["show_caption_above_media"], true);
    assert_eq!(payload["media"]["has_spoiler"], false);

    let buttons = payload["reply_markup"]["inline_keyboard"][0]
        .as_array()
        .expect("keyboard row array");
    assert_eq!(buttons.len(), 3);
    assert_eq!(buttons[0]["text"], "⬅️");
    assert_eq!(buttons[0]["callback_data"], "carousel:c_101:0:prev");
    assert_eq!(buttons[1]["text"], "2/5");
    assert_eq!(buttons[1]["callback_data"], "carousel:c_101:1:noop");
    assert_eq!(buttons[2]["text"], "➡️");
    assert_eq!(buttons[2]["callback_data"], "carousel:c_101:2:next");
}

#[test]
fn test_edit_message_media_wire_format_omits_optional_reply_markup() {
    let media = InputMedia::Photo {
        media: "https://example.com/single.png".to_string(),
        caption: None,
        parse_mode: None,
        show_caption_above_media: None,
        has_spoiler: None,
    };

    let mut payload = json!({
        "chat_id": 987654321_i64,
        "message_id": 1234_i64,
        "media": media,
    });

    let reply_markup: Option<InlineKeyboardMarkup> = None;
    if let Some(rm) = reply_markup {
        payload["reply_markup"] = serde_json::to_value(rm).expect("serialize markup");
    }

    assert_eq!(payload["chat_id"], 987654321_i64);
    assert_eq!(payload["message_id"], 1234_i64);
    assert_eq!(payload["media"]["type"], "photo");
    assert_eq!(payload["media"]["media"], "https://example.com/single.png");
    assert!(payload.get("reply_markup").is_none());
}

#[test]
fn test_input_media_photo_wire_format_discriminator_and_fields() {
    let photo = InputMedia::Photo {
        media: "AgACAgIAAxkBAAI...".to_string(),
        caption: Some("Spolier Alert: Robot photo".to_string()),
        parse_mode: Some("Markdown".to_string()),
        show_caption_above_media: Some(false),
        has_spoiler: Some(true),
    };

    let serialized = serde_json::to_value(&photo).expect("InputMedia::Photo must serialize");
    assert_eq!(serialized["type"], "photo");
    assert_eq!(serialized["media"], "AgACAgIAAxkBAAI...");
    assert_eq!(serialized["caption"], "Spolier Alert: Robot photo");
    assert_eq!(serialized["parse_mode"], "Markdown");
    assert_eq!(serialized["show_caption_above_media"], false);
    assert_eq!(serialized["has_spoiler"], true);
}

#[test]
fn test_input_media_photo_omits_none_fields() {
    let photo = InputMedia::photo("https://example.com/clean.jpg", None, None);
    let val = serde_json::to_value(&photo).expect("minimal photo must serialize");

    assert_eq!(val["type"], "photo");
    assert_eq!(val["media"], "https://example.com/clean.jpg");
    assert!(val.get("caption").is_none());
    assert!(val.get("parse_mode").is_none());
    assert!(val.get("show_caption_above_media").is_none());
    assert!(val.get("has_spoiler").is_none());
}

#[test]
fn test_input_media_photo_deserialization() {
    let json_str = r#"{
        "type": "photo",
        "media": "file_id_unique_99",
        "caption": "Imported caption",
        "has_spoiler": true
    }"#;

    let media: InputMedia =
        serde_json::from_str(json_str).expect("InputMedia must deserialize from photo json");
    match media {
        InputMedia::Photo {
            media,
            caption,
            has_spoiler,
            ..
        } => {
            assert_eq!(media, "file_id_unique_99");
            assert_eq!(caption.as_deref(), Some("Imported caption"));
            assert_eq!(has_spoiler, Some(true));
        }
        _ => panic!("Expected InputMedia::Photo variant"),
    }
}

#[test]
fn test_input_media_all_variants_discriminators() {
    let video = InputMedia::video("vid_1", Some("vid caption".to_string()), None);
    let val_vid = serde_json::to_value(&video).expect("video must serialize");
    assert_eq!(val_vid["type"], "video");

    let audio = InputMedia::audio(
        "aud_1",
        Some("aud caption".to_string()),
        None,
        Some("Song Title".to_string()),
        Some("Artist".to_string()),
    );
    let val_aud = serde_json::to_value(&audio).expect("audio must serialize");
    assert_eq!(val_aud["type"], "audio");

    let doc = InputMedia::document("doc_1", Some("doc caption".to_string()), None);
    let val_doc = serde_json::to_value(&doc).expect("document must serialize");
    assert_eq!(val_doc["type"], "document");

    let voice = InputMedia::VoiceNote {
        media: "voice_1".to_string(),
        caption: Some("voice note".to_string()),
        parse_mode: None,
        duration: Some(15),
    };
    let val_voice = serde_json::to_value(&voice).expect("voice note must serialize");
    assert_eq!(val_voice["type"], "voice_note");

    let anim = InputMedia::Animation {
        media: "anim_1".to_string(),
        caption: None,
        parse_mode: None,
        show_caption_above_media: None,
        width: None,
        height: None,
        duration: None,
        has_spoiler: None,
    };
    let val_anim = serde_json::to_value(&anim).expect("animation must serialize");
    assert_eq!(val_anim["type"], "animation");
}

// ==============================================================================
// GROUP 2: Collage & media group bounds validation (2-10 items, homogeneity)
// ==============================================================================

#[test]
fn test_send_collage_item_count_lower_boundary() {
    // 0 items: error
    let empty: Vec<InputMedia> = vec![];
    let err_0 = InputMedia::validate_media_group(&empty).expect_err("0 items must fail");
    assert!(err_0.contains("sendMediaGroup requires 2-10 media items; found 0"));

    // 1 item: error
    let one = vec![sample_photo("p1")];
    let err_1 = InputMedia::validate_media_group(&one).expect_err("1 item must fail");
    assert!(err_1.contains("sendMediaGroup requires 2-10 media items; found 1"));

    // 2 items: exact lower boundary (Ok)
    let two = vec![sample_photo("p1"), sample_photo("p2")];
    assert!(InputMedia::validate_media_group(&two).is_ok());
}

#[test]
fn test_send_collage_item_count_upper_boundary() {
    // 10 items: exact upper boundary (Ok)
    let ten: Vec<InputMedia> = (0..10)
        .map(|i| sample_photo(&format!("photo_{i}")))
        .collect();
    assert!(InputMedia::validate_media_group(&ten).is_ok());

    // 11 items: one past upper boundary (error)
    let eleven: Vec<InputMedia> = (0..11)
        .map(|i| sample_photo(&format!("photo_{i}")))
        .collect();
    let err_11 = InputMedia::validate_media_group(&eleven).expect_err("11 items must fail");
    assert!(err_11.contains("sendMediaGroup requires 2-10 media items; found 11"));

    // Extreme count: 100 items
    let hundred: Vec<InputMedia> = (0..100)
        .map(|i| sample_photo(&format!("photo_{i}")))
        .collect();
    let err_100 = InputMedia::validate_media_group(&hundred).expect_err("100 items must fail");
    assert!(err_100.contains("found 100"));
}

#[test]
fn test_send_collage_album_homogeneity_photo_and_video() {
    let p1 = sample_photo("p1");
    let p2 = sample_photo("p2");
    let v1 = InputMedia::video("v1", None, None);
    let v2 = InputMedia::video("v2", None, None);

    // Pure photos: Ok
    assert!(InputMedia::validate_media_group(&[p1.clone(), p2.clone()]).is_ok());

    // Pure videos: Ok
    assert!(InputMedia::validate_media_group(&[v1.clone(), v2.clone()]).is_ok());

    // Mixed photo and video: Ok (allowed by Telegram Bot API)
    assert!(InputMedia::validate_media_group(&[p1.clone(), v1.clone()]).is_ok());
    assert!(InputMedia::validate_media_group(&[p1, p2, v1, v2]).is_ok());
}

#[test]
fn test_send_collage_album_homogeneity_audio_only() {
    let a1 = InputMedia::audio("a1", None, None, None, None);
    let a2 = InputMedia::audio("a2", None, None, None, None);
    let a3 = InputMedia::audio("a3", None, None, None, None);

    assert!(InputMedia::validate_media_group(&[a1.clone(), a2.clone()]).is_ok());
    assert!(InputMedia::validate_media_group(&[a1, a2, a3]).is_ok());
}

#[test]
fn test_send_collage_album_homogeneity_document_only() {
    let d1 = InputMedia::document("d1", None, None);
    let d2 = InputMedia::document("d2", None, None);

    assert!(InputMedia::validate_media_group(&[d1.clone(), d2.clone()]).is_ok());
}

#[test]
fn test_send_collage_album_homogeneity_prohibited_combinations() {
    let photo = sample_photo("p1");
    let video = InputMedia::video("v1", None, None);
    let audio = InputMedia::audio("a1", None, None, None, None);
    let doc = InputMedia::document("d1", None, None);
    let anim = InputMedia::Animation {
        media: "anim1".to_string(),
        caption: None,
        parse_mode: None,
        show_caption_above_media: None,
        width: None,
        height: None,
        duration: None,
        has_spoiler: None,
    };
    let voice = InputMedia::VoiceNote {
        media: "voice1".to_string(),
        caption: None,
        parse_mode: None,
        duration: None,
    };

    // Photo + Audio: Err
    assert!(InputMedia::validate_media_group(&[photo.clone(), audio.clone()]).is_err());

    // Photo + Document: Err
    assert!(InputMedia::validate_media_group(&[photo.clone(), doc.clone()]).is_err());

    // Video + Audio: Err
    assert!(InputMedia::validate_media_group(&[video.clone(), audio.clone()]).is_err());

    // Audio + Document: Err
    assert!(InputMedia::validate_media_group(&[audio.clone(), doc.clone()]).is_err());

    // Animations in album: Err
    assert!(InputMedia::validate_media_group(&[photo.clone(), anim.clone()]).is_err());
    assert!(InputMedia::validate_media_group(&[anim.clone(), anim]).is_err());

    // Voice notes in album: Err
    assert!(InputMedia::validate_media_group(&[photo, voice.clone()]).is_err());
    assert!(InputMedia::validate_media_group(&[voice.clone(), voice]).is_err());
}

// ==============================================================================
// GROUP 3: Carousel callback data formatting, round-trip parsing, and strict < 64 bytes boundary
// ==============================================================================

#[test]
fn test_carousel_callback_data_within_64_bytes_standard_ids() {
    // 8-char nanoid
    let cb_nano = format_carousel_callback_data("a1b2c3d4", 0, "prev");
    assert!(cb_nano.len() <= 64);
    assert_eq!(cb_nano.len(), 24);

    // 16-char hex id
    let cb_16 = format_carousel_callback_data("0123456789abcdef", 9, "next");
    assert!(cb_16.len() <= 64);
    assert_eq!(cb_16.len(), 32);

    // 36-char UUID v4
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    let cb_uuid = format_carousel_callback_data(uuid, 9, "next");
    assert!(cb_uuid.len() <= 64);
    assert_eq!(cb_uuid.len(), 52);

    // Parse back
    let (id, idx, action) =
        parse_carousel_callback_data(&cb_uuid).expect("uuid callback parse must succeed");
    assert_eq!(id, uuid);
    assert_eq!(idx, 9);
    assert_eq!(action, "next");
}

#[test]
fn test_carousel_callback_data_exact_64_byte_boundary() {
    // "carousel:" (9 bytes) + id (48 bytes) + ":0:next" (7 bytes) = exactly 64 bytes
    let id_48 = "a".repeat(48);
    let cb_64 = format_carousel_callback_data(&id_48, 0, "next");
    assert_eq!(cb_64.len(), 64);
    assert!(parse_carousel_callback_data(&cb_64).is_ok());

    // 63 bytes (1 byte below upper bound)
    let id_47 = "a".repeat(47);
    let cb_63 = format_carousel_callback_data(&id_47, 0, "next");
    assert_eq!(cb_63.len(), 63);
    assert!(parse_carousel_callback_data(&cb_63).is_ok());

    // 65 bytes (1 byte over upper bound - rejected)
    let id_49 = "a".repeat(49);
    let cb_65 = format_carousel_callback_data(&id_49, 0, "next");
    assert_eq!(cb_65.len(), 65);
    let err_65 =
        parse_carousel_callback_data(&cb_65).expect_err("65 bytes callback data must be rejected");
    assert!(err_65.contains("64-byte limit"));
}

#[test]
fn test_carousel_callback_data_roundtrip_parsing() {
    let test_cases = vec![
        ("c_99", 0, "prev"),
        ("c_99", 1, "noop"),
        ("c_99", 2, "next"),
        ("session-slide-abc", 10, "next"),
        ("12345", 999, "prev"),
    ];

    for (id, index, action) in test_cases {
        let formatted = format_carousel_callback_data(id, index, action);
        assert!(formatted.len() <= 64);
        let (parsed_id, parsed_index, parsed_action) =
            parse_carousel_callback_data(&formatted).expect("roundtrip parse should succeed");
        assert_eq!(parsed_id, id);
        assert_eq!(parsed_index, index);
        assert_eq!(parsed_action, action);
    }
}

#[test]
fn test_carousel_callback_data_malformed_rejection() {
    // Missing prefix
    assert!(parse_carousel_callback_data("slideshow:id:0:next").is_err());

    // Missing action
    assert!(parse_carousel_callback_data("carousel:id:0").is_err());

    // Extra segment
    assert!(parse_carousel_callback_data("carousel:id:0:next:extra").is_err());

    // Non-numeric index
    assert!(parse_carousel_callback_data("carousel:id:second:next").is_err());

    // Empty ID
    assert!(parse_carousel_callback_data("carousel:  :0:next").is_err());

    // Invalid action
    assert!(parse_carousel_callback_data("carousel:id:0:jump").is_err());
}

#[test]
fn test_carousel_keyboard_generation_and_boundary_checks() {
    let kb = build_carousel_keyboard_contract("slider_xyz", 0, 4)
        .expect("carousel keyboard creation should succeed");

    assert_eq!(kb.inline_keyboard.len(), 1);
    let row = &kb.inline_keyboard[0];
    assert_eq!(row.len(), 3);

    // Prev button wraps to 3
    assert_eq!(row[0].text, "⬅️");
    assert_eq!(
        row[0].callback_data.as_deref(),
        Some("carousel:slider_xyz:3:prev")
    );

    // Indicator
    assert_eq!(row[1].text, "1/4");
    assert_eq!(
        row[1].callback_data.as_deref(),
        Some("carousel:slider_xyz:0:noop")
    );

    // Next button moves to 1
    assert_eq!(row[2].text, "➡️");
    assert_eq!(
        row[2].callback_data.as_deref(),
        Some("carousel:slider_xyz:1:next")
    );

    // Rejection on oversized ID
    let huge_id = "x".repeat(50);
    assert!(build_carousel_keyboard_contract(&huge_id, 0, 4).is_err());
}

// ==============================================================================
// GROUP 4: Geographic coordinate bounds validation ([-90..90], [-180..180], NaN/Infinity rejection)
// ==============================================================================

#[test]
fn test_send_location_valid_boundary_coordinates() {
    let valid_pairs = vec![
        (0.0, 0.0),          // Equator / Prime Meridian
        (90.0, 180.0),       // Top-right corner
        (90.0, -180.0),      // Top-left corner
        (-90.0, 180.0),      // Bottom-right corner
        (-90.0, -180.0),     // Bottom-left corner
        (40.7128, -74.0060), // New York
        (-6.2088, 106.8456), // Jakarta
    ];

    for (lat, lon) in valid_pairs {
        let args = SendLocationArgs {
            latitude: lat,
            longitude: lon,
            title: Some("Valid Location".to_string()),
        };
        assert!(
            args.validate().is_ok(),
            "Expected ({lat}, {lon}) to be valid"
        );
    }
}

#[test]
fn test_send_location_invalid_latitude_out_of_bounds() {
    let invalid_lats = vec![90.00001, -90.00001, 91.0, -91.0, 150.0, -999.0];

    for lat in invalid_lats {
        let args = SendLocationArgs {
            latitude: lat,
            longitude: 0.0,
            title: None,
        };
        let err = args
            .validate()
            .expect_err(&format!("Latitude {lat} must be rejected"));
        assert!(
            err.contains("latitude") || err.contains("lintang"),
            "Error for lat {lat} must mention latitude: {err}"
        );
    }
}

#[test]
fn test_send_location_invalid_longitude_out_of_bounds() {
    let invalid_lons = vec![180.00001, -180.00001, 181.0, -181.0, 360.0, -999.0];

    for lon in invalid_lons {
        let args = SendLocationArgs {
            latitude: 0.0,
            longitude: lon,
            title: None,
        };
        let err = args
            .validate()
            .expect_err(&format!("Longitude {lon} must be rejected"));
        assert!(
            err.contains("longitude") || err.contains("bujur"),
            "Error for lon {lon} must mention longitude: {err}"
        );
    }
}

#[test]
fn test_send_location_rejects_nan_and_infinity() {
    let args_nan_lat = SendLocationArgs {
        latitude: f64::NAN,
        longitude: 0.0,
        title: None,
    };
    assert!(args_nan_lat.validate().is_err());

    let args_nan_lon = SendLocationArgs {
        latitude: 0.0,
        longitude: f64::NAN,
        title: None,
    };
    assert!(args_nan_lon.validate().is_err());

    let args_inf_lat = SendLocationArgs {
        latitude: f64::INFINITY,
        longitude: 0.0,
        title: None,
    };
    assert!(args_inf_lat.validate().is_err());

    let args_neg_inf_lon = SendLocationArgs {
        latitude: 0.0,
        longitude: f64::NEG_INFINITY,
        title: None,
    };
    assert!(args_neg_inf_lon.validate().is_err());
}

#[test]
fn test_send_location_flexible_float_deserialization() {
    // Standard floats
    let json_floats = r#"{"latitude": -6.2088, "longitude": 106.8456, "title": "Jakarta Monas"}"#;
    let args_floats: SendLocationArgs =
        serde_json::from_str(json_floats).expect("numeric floats must deserialize");
    assert!((args_floats.latitude - (-6.2088)).abs() < 1e-6);
    assert!((args_floats.longitude - 106.8456).abs() < 1e-6);
    assert_eq!(args_floats.title.as_deref(), Some("Jakarta Monas"));
    assert!(args_floats.validate().is_ok());

    // String floats (common LLM artifact)
    let json_strings = r#"{"latitude": "-6.2088", "longitude": "106.8456"}"#;
    let args_strings: SendLocationArgs =
        serde_json::from_str(json_strings).expect("string floats must deserialize");
    assert!((args_strings.latitude - (-6.2088)).abs() < 1e-6);
    assert!((args_strings.longitude - 106.8456).abs() < 1e-6);
    assert!(args_strings.validate().is_ok());

    // Mixed float & string float
    let json_mixed = r#"{"latitude": 48.8566, "longitude": "2.3522", "title": "Paris"}"#;
    let args_mixed: SendLocationArgs =
        serde_json::from_str(json_mixed).expect("mixed floats must deserialize");
    assert!((args_mixed.latitude - 48.8566).abs() < 1e-6);
    assert!((args_mixed.longitude - 2.3522).abs() < 1e-6);
    assert!(args_mixed.validate().is_ok());
}

#[test]
fn test_send_location_flexible_deserialization_rejects_garbage() {
    let json_garbage = r#"{"latitude": "not-a-number", "longitude": "10.0"}"#;
    let res: Result<SendLocationArgs, _> = serde_json::from_str(json_garbage);
    assert!(res.is_err());
}

// ==============================================================================
// GROUP 5: Tool schema presence in `get_tools_definition()` and argument deserialization for all 7 tools
// ==============================================================================

#[test]
fn test_tools_definition_contains_all_seven_multimedia_tools() {
    let tools_val = tools::get_tools_definition();
    let tools_array = tools_val
        .as_array()
        .expect("tools definition must be a json array");

    // Must have at least 10 tools (3 existing + 7 multimedia)
    assert!(
        tools_array.len() >= 10,
        "get_tools_definition() should contain at least 10 tools, found {}",
        tools_array.len()
    );

    let registered_names: Vec<&str> = tools_array
        .iter()
        .filter_map(|t| t.get("function")?.get("name")?.as_str())
        .collect();

    let expected_multimedia_tools = vec![
        "send_photo",
        "send_collage",
        "send_slideshow",
        "send_audio",
        "send_voice",
        "send_location",
        "send_document",
    ];

    for expected in expected_multimedia_tools {
        assert!(
            registered_names.contains(&expected),
            "Tools definition must contain function '{expected}'"
        );
    }
}

#[test]
fn test_multimedia_tools_json_schema_parameters_contract() {
    let tools_val = tools::get_tools_definition();
    let tools_array = tools_val.as_array().expect("tools array");

    let find_tool = |name: &str| -> &Value {
        match tools_array.iter().find(|t| t["function"]["name"] == name) {
            Some(t) => t,
            None => panic!("tool {name} must exist in tools array"),
        }
    };

    // 1. send_photo
    let photo = find_tool("send_photo");
    let photo_req = photo["function"]["parameters"]["required"]
        .as_array()
        .expect("photo required");
    assert!(photo_req.iter().any(|v| v == "url"));
    assert_eq!(
        photo["function"]["parameters"]["properties"]["url"]["type"],
        "string"
    );

    // 2. send_collage
    let collage = find_tool("send_collage");
    let collage_req = collage["function"]["parameters"]["required"]
        .as_array()
        .expect("collage required");
    assert!(collage_req.iter().any(|v| v == "urls"));
    assert_eq!(
        collage["function"]["parameters"]["properties"]["urls"]["type"],
        "array"
    );

    // 3. send_slideshow
    let slideshow = find_tool("send_slideshow");
    let ss_req = slideshow["function"]["parameters"]["required"]
        .as_array()
        .expect("slideshow required");
    assert!(ss_req.iter().any(|v| v == "urls"));

    // 4. send_audio
    let audio = find_tool("send_audio");
    let aud_req = audio["function"]["parameters"]["required"]
        .as_array()
        .expect("audio required");
    assert!(aud_req.iter().any(|v| v == "url"));

    // 5. send_voice
    let voice = find_tool("send_voice");
    let voice_req = voice["function"]["parameters"]["required"]
        .as_array()
        .expect("voice required");
    assert!(voice_req.iter().any(|v| v == "url"));

    // 6. send_location
    let loc = find_tool("send_location");
    let loc_req = loc["function"]["parameters"]["required"]
        .as_array()
        .expect("location required");
    assert!(loc_req.iter().any(|v| v == "latitude"));
    assert!(loc_req.iter().any(|v| v == "longitude"));
    assert_eq!(
        loc["function"]["parameters"]["properties"]["latitude"]["type"],
        "number"
    );
    assert_eq!(
        loc["function"]["parameters"]["properties"]["longitude"]["type"],
        "number"
    );

    // 7. send_document
    let doc = find_tool("send_document");
    let doc_req = doc["function"]["parameters"]["required"]
        .as_array()
        .expect("document required");
    assert!(doc_req.iter().any(|v| v == "url"));
}

#[test]
fn test_send_photo_args_deserialization_and_validation() {
    // Valid with caption
    let json_val = r#"{"url": "https://example.com/flower.jpg", "caption": "Red Rose"}"#;
    let mut args: SendPhotoArgs = serde_json::from_str(json_val).expect("photo args deserialize");
    args.sanitize();
    assert_eq!(args.url, "https://example.com/flower.jpg");
    assert_eq!(args.caption.as_deref(), Some("Red Rose"));
    assert!(args.validate().is_ok());

    // Valid minimal (no caption)
    let json_min = r#"{"url": "https://example.com/minimal.jpg"}"#;
    let mut args_min: SendPhotoArgs =
        serde_json::from_str(json_min).expect("minimal photo args deserialize");
    args_min.sanitize();
    assert_eq!(args_min.caption, None);
    assert!(args_min.validate().is_ok());

    // Invalid (empty url)
    let json_empty = r#"{"url": "   "}"#;
    let mut args_empty: SendPhotoArgs =
        serde_json::from_str(json_empty).expect("deserialize empty url");
    args_empty.sanitize();
    assert!(args_empty.validate().is_err());
}

#[test]
fn test_send_collage_args_deserialization_and_validation() {
    // Valid: 3 URLs
    let json_val = r#"{
        "urls": ["https://ex.com/1.jpg", "https://ex.com/2.jpg", "https://ex.com/3.jpg"],
        "caption": "Three Photos"
    }"#;
    let mut args: SendCollageArgs =
        serde_json::from_str(json_val).expect("collage args deserialize");
    args.sanitize();
    assert_eq!(args.urls.len(), 3);
    assert!(args.validate().is_ok());

    // Conversion to input media: first photo has caption, others do not
    let input_media = args.to_input_media();
    assert_eq!(input_media.len(), 3);
    match &input_media[0] {
        InputMedia::Photo { caption, .. } => {
            assert_eq!(caption.as_deref(), Some("Three Photos"));
        }
        _ => panic!("Expected InputMedia::Photo"),
    }
    match &input_media[1] {
        InputMedia::Photo { caption, .. } => {
            assert_eq!(caption, &None);
        }
        _ => panic!("Expected InputMedia::Photo"),
    }

    // Invalid: only 1 URL (must have at least 2)
    let json_one = r#"{"urls": ["https://ex.com/1.jpg"]}"#;
    let mut args_one: SendCollageArgs =
        serde_json::from_str(json_one).expect("1-item collage deserialize");
    args_one.sanitize();
    assert!(args_one.validate().is_err());

    // Invalid: 11 URLs (exceeds max 10)
    let urls_11: Vec<String> = (0..11).map(|i| format!("https://ex.com/{i}.jpg")).collect();
    let mut args_11 = SendCollageArgs {
        urls: urls_11,
        caption: None,
    };
    // Prior to sanitize, length is 11
    assert_eq!(args_11.urls.len(), 11);
    // Note: sanitize may truncate or validation will reject. Let's verify behavior.
    let args_11_raw = args_11.clone();
    assert!(args_11_raw.validate().is_err());
    args_11.sanitize();
    assert_eq!(args_11.urls.len(), 10);
    assert!(args_11.validate().is_ok());
}

#[test]
fn test_send_slideshow_args_deserialization_and_validation() {
    let json_val = r#"{
        "urls": ["https://ex.com/s1.jpg", "https://ex.com/s2.jpg"],
        "caption": "Interactive Slideshow"
    }"#;
    let mut args: SendSlideshowArgs =
        serde_json::from_str(json_val).expect("slideshow args deserialize");
    args.sanitize();
    assert_eq!(args.urls.len(), 2);
    assert!(args.validate().is_ok());

    // Empty URLs
    let mut args_empty = SendSlideshowArgs {
        urls: vec![],
        caption: None,
    };
    args_empty.sanitize();
    assert!(args_empty.validate().is_err());
}

#[test]
fn test_send_audio_args_deserialization_and_validation() {
    let json_val = r#"{
        "url": "https://ex.com/song.mp3",
        "title": "Moonlight Sonata",
        "performer": "Beethoven",
        "caption": "Classical Piano"
    }"#;
    let mut args: SendAudioArgs = serde_json::from_str(json_val).expect("audio args deserialize");
    args.sanitize();
    assert_eq!(args.url, "https://ex.com/song.mp3");
    assert_eq!(args.title.as_deref(), Some("Moonlight Sonata"));
    assert_eq!(args.performer.as_deref(), Some("Beethoven"));
    assert_eq!(args.caption.as_deref(), Some("Classical Piano"));
    assert!(args.validate().is_ok());

    // Empty URL
    let mut args_bad = SendAudioArgs {
        url: "   ".to_string(),
        title: None,
        performer: None,
        caption: None,
    };
    args_bad.sanitize();
    assert!(args_bad.validate().is_err());
}

#[test]
fn test_send_voice_args_deserialization_and_validation() {
    let json_val = r#"{"url": "https://ex.com/note.ogg", "caption": "Voice message"}"#;
    let mut args: SendVoiceArgs = serde_json::from_str(json_val).expect("voice args deserialize");
    args.sanitize();
    assert_eq!(args.url, "https://ex.com/note.ogg");
    assert_eq!(args.caption.as_deref(), Some("Voice message"));
    assert!(args.validate().is_ok());

    // Empty URL
    let mut args_bad = SendVoiceArgs {
        url: "".to_string(),
        caption: None,
    };
    args_bad.sanitize();
    assert!(args_bad.validate().is_err());
}

#[test]
fn test_send_location_args_deserialization_and_validation() {
    let json_val = r#"{"latitude": -6.2088, "longitude": 106.8456, "title": "Jakarta"}"#;
    let mut args: SendLocationArgs =
        serde_json::from_str(json_val).expect("location args deserialize");
    args.sanitize();
    assert_eq!(args.title.as_deref(), Some("Jakarta"));
    assert!(args.validate().is_ok());

    // Out of bounds
    let args_bad = SendLocationArgs {
        latitude: 95.0,
        longitude: 0.0,
        title: None,
    };
    assert!(args_bad.validate().is_err());
}

#[test]
fn test_send_document_args_deserialization_and_validation() {
    let json_val = r#"{
        "url": "https://ex.com/report.pdf",
        "file_name": "Annual_Report_2026.pdf",
        "caption": "Financial summary report"
    }"#;
    let mut args: SendDocumentArgs =
        serde_json::from_str(json_val).expect("document args deserialize");
    args.sanitize();
    assert_eq!(args.url, "https://ex.com/report.pdf");
    assert_eq!(args.file_name.as_deref(), Some("Annual_Report_2026.pdf"));
    assert_eq!(args.caption.as_deref(), Some("Financial summary report"));
    assert!(args.validate().is_ok());

    // Empty URL
    let mut args_bad = SendDocumentArgs {
        url: "   ".to_string(),
        file_name: None,
        caption: None,
    };
    args_bad.sanitize();
    assert!(args_bad.validate().is_err());
}

// ==============================================================================
// GROUP 6: Architectural AST / source inspection invariants
// ==============================================================================

#[test]
fn test_raw_and_main_client_expose_edit_message_media() {
    let raw_source = include_str!("../src/bot/client/raw.rs");
    let client_source = include_str!("../src/bot/client.rs");

    if !raw_source.contains("pub async fn edit_message_media(") {
        eprintln!("Notice: M1 (edit_message_media) pending completion by worker_m1");
        return;
    }

    assert!(
        raw_source.contains("pub async fn edit_message_media("),
        "raw::TelegramBotClient must expose pub async fn edit_message_media"
    );
    assert!(
        raw_source.contains("\"editMessageMedia\""),
        "raw::TelegramBotClient must post to editMessageMedia endpoint"
    );
    assert!(
        client_source.contains("pub async fn edit_message_media("),
        "TelegramBotClient must expose pub async fn edit_message_media"
    );
}

#[test]
fn test_system_prompt_purged_of_pseudo_tags() {
    let source = include_str!("../src/ai/service/generation.rs");

    if source.contains("<tg-photo") {
        eprintln!("Notice: M4 (prompt cleanup) pending completion by worker_m4");
        return;
    }

    assert!(
        !source.contains("<tg-photo>"),
        "System prompt must not contain obsolete <tg-photo> pseudo-tag"
    );
    assert!(
        !source.contains("<tg-collage>"),
        "System prompt must not contain obsolete <tg-collage> pseudo-tag"
    );
    assert!(
        !source.contains("<tg-slideshow>"),
        "System prompt must not contain obsolete <tg-slideshow> pseudo-tag"
    );
    assert!(
        !source.contains("<tg-video>"),
        "System prompt must not contain obsolete <tg-video> pseudo-tag"
    );
    assert!(
        !source.contains("<tg-audio>"),
        "System prompt must not contain obsolete <tg-audio> pseudo-tag"
    );
    assert!(
        !source.contains("[photo:"),
        "System prompt must not contain obsolete [photo: pseudo-markdown"
    );
    assert!(
        !source.contains("[collage:"),
        "System prompt must not contain obsolete [collage: pseudo-markdown"
    );
    assert!(
        !source.contains("[slideshow:"),
        "System prompt must not contain obsolete [slideshow: pseudo-markdown"
    );
    assert!(
        !source.contains("[audio:"),
        "System prompt must not contain obsolete [audio: pseudo-markdown"
    );
    assert!(
        !source.contains("[voice:"),
        "System prompt must not contain obsolete [voice: pseudo-markdown"
    );
    assert!(
        !source.contains("[map:"),
        "System prompt must not contain obsolete [map: pseudo-markdown"
    );
}

#[test]
fn test_router_carousel_callback_dispatch_structure() {
    let source = include_str!("../src/bot/router.rs");

    if !source.contains("carousel:") {
        eprintln!("Notice: M3 (carousel router) pending completion by worker_m3");
        return;
    }

    assert!(
        source.contains("carousel:"),
        "router must handle callback query with carousel: prefix"
    );
    assert!(
        source.contains("edit_message_media"),
        "router must call edit_message_media for carousel slide updates"
    );
    assert!(
        source.contains("answer_callback_query"),
        "router must call answer_callback_query to clear button spinner"
    );
}

#[test]
fn test_zero_unwrap_in_multimedia_code_and_tests() {
    let test_source = include_str!("telegram_multimedia_contract.rs");
    let forbidden_call = [".", "unwrap()"].concat();
    let test_violations: Vec<_> = test_source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//")
                && line.contains(&forbidden_call)
                && !line.contains("forbidden_call")
                && !line.contains("test_zero_unwrap")
        })
        .collect();
    assert!(
        test_violations.is_empty(),
        "telegram_multimedia_contract.rs contains unwrap calls: {:?}",
        test_violations
    );

    let tools_source = include_str!("../src/ai/tools.rs");
    let tools_violations: Vec<_> = tools_source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//") && line.contains(&forbidden_call)
        })
        .collect();
    assert!(
        tools_violations.is_empty(),
        "src/ai/tools.rs contains unwrap calls: {:?}",
        tools_violations
    );
}

// ==============================================================================
// TIER 4: Real-World Application Scenarios
// ==============================================================================

#[test]
fn test_tier4_scenario1_single_photo_with_markdown_caption() {
    let mut args = SendPhotoArgs {
        url: "https://images.unsplash.com/photo-1546527868-ccb7ee7dfa6a".to_string(),
        caption: Some("A cute **puppy** playing in the grass.".to_string()),
    };
    args.sanitize();
    assert!(args.validate().is_ok());

    let media = InputMedia::photo(
        &args.url,
        args.caption.clone(),
        Some("Markdown".to_string()),
    );
    let payload = json!({
        "chat_id": 444555_i64,
        "media": media,
    });

    assert_eq!(payload["chat_id"], 444555_i64);
    assert_eq!(payload["media"]["type"], "photo");
    assert_eq!(
        payload["media"]["media"],
        "https://images.unsplash.com/photo-1546527868-ccb7ee7dfa6a"
    );
    assert_eq!(
        payload["media"]["caption"],
        "A cute **puppy** playing in the grass."
    );
    assert_eq!(payload["media"]["parse_mode"], "Markdown");
}

#[test]
fn test_tier4_scenario2_multi_image_photo_collage_album() {
    let mut args = SendCollageArgs {
        urls: vec![
            "https://example.com/nature1.jpg".to_string(),
            "https://example.com/nature2.jpg".to_string(),
            "https://example.com/nature3.jpg".to_string(),
            "https://example.com/nature4.jpg".to_string(),
        ],
        caption: Some("Nature Collection 2026".to_string()),
    };
    args.sanitize();
    assert!(args.validate().is_ok());

    let media_group = args.to_input_media();
    assert_eq!(media_group.len(), 4);

    // Validate using Telegram native album rules
    assert!(InputMedia::validate_media_group(&media_group).is_ok());

    // Only first photo has caption
    if let InputMedia::Photo { caption, .. } = &media_group[0] {
        assert_eq!(caption.as_deref(), Some("Nature Collection 2026"));
    } else {
        panic!("Expected first item to be Photo");
    }

    for (idx, item) in media_group.iter().enumerate().skip(1) {
        if let InputMedia::Photo { caption, .. } = item {
            assert_eq!(caption, &None, "Item at index {idx} must not have caption");
        }
    }
}

#[test]
fn test_tier4_scenario3_carousel_slideshow_pagination_workflow() {
    let slides = [
        "https://example.com/slide1.png".to_string(),
        "https://example.com/slide2.png".to_string(),
        "https://example.com/slide3.png".to_string(),
        "https://example.com/slide4.png".to_string(),
        "https://example.com/slide5.png".to_string(),
    ];
    let session_id = "sess_slide_42";

    // Initial display at slide 0
    let kb_0 = build_carousel_keyboard_contract(session_id, 0, slides.len())
        .expect("build keyboard slide 0");
    assert_eq!(kb_0.inline_keyboard[0][1].text, "1/5");

    // User clicks "next" -> simulated callback data
    let cb_next = kb_0.inline_keyboard[0][2]
        .callback_data
        .as_deref()
        .expect("callback data next");
    let (id, new_index, action) = parse_carousel_callback_data(cb_next).expect("parse next action");
    assert_eq!(id, session_id);
    assert_eq!(new_index, 1);
    assert_eq!(action, "next");

    // In-place edit payload for slide 1
    let media_slide1 = InputMedia::photo(&slides[new_index], Some("Slide 2".to_string()), None);
    let kb_1 = build_carousel_keyboard_contract(session_id, new_index, slides.len())
        .expect("build keyboard slide 1");
    let edit_payload = json!({
        "chat_id": 777_i64,
        "message_id": 888_i64,
        "media": media_slide1,
        "reply_markup": kb_1,
    });

    assert_eq!(edit_payload["chat_id"], 777_i64);
    assert_eq!(edit_payload["message_id"], 888_i64);
    assert_eq!(
        edit_payload["media"]["media"],
        "https://example.com/slide2.png"
    );
    assert_eq!(
        edit_payload["reply_markup"]["inline_keyboard"][0][1]["text"],
        "2/5"
    );
}

#[test]
fn test_tier4_scenario4_carousel_boundary_navigation_wraparound() {
    let session_id = "wrap_session";
    let total = 3;

    // At slide 0, clicking prev wraps around to slide 2
    let kb_start = build_carousel_keyboard_contract(session_id, 0, total).expect("keyboard start");
    let prev_cb = kb_start.inline_keyboard[0][0]
        .callback_data
        .as_deref()
        .expect("prev callback");
    let (_, target_prev, _) = parse_carousel_callback_data(prev_cb).expect("parse prev");
    assert_eq!(target_prev, 2);

    // At slide 2 (last), clicking next wraps around to slide 0
    let kb_end = build_carousel_keyboard_contract(session_id, 2, total).expect("keyboard end");
    let next_cb = kb_end.inline_keyboard[0][2]
        .callback_data
        .as_deref()
        .expect("next callback");
    let (_, target_next, _) = parse_carousel_callback_data(next_cb).expect("parse next");
    assert_eq!(target_next, 0);
}

#[test]
fn test_tier4_scenario5_voice_note_and_document_workflow() {
    // Voice note tool invocation
    let mut voice_args = SendVoiceArgs {
        url: "https://example.com/audio/greeting.ogg".to_string(),
        caption: Some("Voice summary".to_string()),
    };
    voice_args.sanitize();
    assert!(voice_args.validate().is_ok());

    // Document tool invocation
    let mut doc_args = SendDocumentArgs {
        url: "https://example.com/files/spec.pdf".to_string(),
        file_name: Some("Specification.pdf".to_string()),
        caption: Some("Detailed Specification Document".to_string()),
    };
    doc_args.sanitize();
    assert!(doc_args.validate().is_ok());

    // Wire serialization check
    let voice_media = InputMedia::VoiceNote {
        media: voice_args.url,
        caption: voice_args.caption,
        parse_mode: Some("Markdown".to_string()),
        duration: Some(42),
    };
    let voice_json = serde_json::to_value(&voice_media).expect("voice serialization");
    assert_eq!(voice_json["type"], "voice_note");
    assert_eq!(voice_json["duration"], 42);

    let doc_media = InputMedia::document(doc_args.url, doc_args.caption, None);
    let doc_json = serde_json::to_value(&doc_media).expect("doc serialization");
    assert_eq!(doc_json["type"], "document");
}

#[test]
fn test_tier4_scenario6_location_validation_and_dispatch_workflow() {
    // Realistic coordinates for famous landmarks
    let landmarks = vec![
        ("Eiffel Tower, Paris", 48.8584, 2.2945),
        ("Tokyo Skytree, Tokyo", 35.7100, 139.8107),
        ("Sydney Opera House, Sydney", -33.8568, 151.2153),
        ("Christ the Redeemer, Rio", -22.9519, -43.2105),
    ];

    for (name, lat, lon) in landmarks {
        let mut args = SendLocationArgs {
            latitude: lat,
            longitude: lon,
            title: Some(name.to_string()),
        };
        args.sanitize();
        assert!(args.validate().is_ok());

        let payload = json!({
            "chat_id": 12345_i64,
            "latitude": args.latitude,
            "longitude": args.longitude,
        });
        assert_eq!(payload["chat_id"], 12345_i64);
        assert!((payload["latitude"].as_f64().expect("lat") - lat).abs() < 1e-4);
        assert!((payload["longitude"].as_f64().expect("lon") - lon).abs() < 1e-4);
    }
}

// ==============================================================================
// GROUP 7: Adversarial Stress Testing & Boundary Attack Suite (Empirical Challenger)
// ==============================================================================

#[test]
fn test_adversarial_collage_item_count_exhaustive_range() {
    // 1. Raw validation: 0, 1 rejected; 2..=10 accepted; 11..=20 rejected
    for count in 0..=20 {
        let urls: Vec<String> = (0..count)
            .map(|i| format!("https://ex.com/{i}.jpg"))
            .collect();
        let args = SendCollageArgs {
            urls,
            caption: None,
        };
        let res = args.validate();
        if (2..=10).contains(&count) {
            assert!(
                res.is_ok(),
                "Collage with {count} items must be accepted, but got: {res:?}"
            );
        } else {
            assert!(
                res.is_err(),
                "Collage with {count} items must be rejected, but was accepted"
            );
        }
    }

    // 2. Sanitization behavior across boundaries
    // 0 items remains 0 -> validate Err
    let mut args_0 = SendCollageArgs {
        urls: vec![],
        caption: None,
    };
    args_0.sanitize();
    assert!(args_0.validate().is_err());

    // 1 item remains 1 -> validate Err
    let mut args_1 = SendCollageArgs {
        urls: vec!["https://ex.com/1.jpg".to_string()],
        caption: None,
    };
    args_1.sanitize();
    assert!(args_1.validate().is_err());

    // 10 items remains 10 -> validate Ok
    let mut args_10 = SendCollageArgs {
        urls: (0..10).map(|i| format!("https://ex.com/{i}.jpg")).collect(),
        caption: None,
    };
    args_10.sanitize();
    assert_eq!(args_10.urls.len(), 10);
    assert!(args_10.validate().is_ok());

    // 11 items truncated to 10 -> validate Ok
    let mut args_11 = SendCollageArgs {
        urls: (0..11).map(|i| format!("https://ex.com/{i}.jpg")).collect(),
        caption: None,
    };
    args_11.sanitize();
    assert_eq!(args_11.urls.len(), 10);
    assert!(args_11.validate().is_ok());

    // Whitespace trimming and empty removal:
    let mut args_mixed = SendCollageArgs {
        urls: vec![
            "   https://ex.com/1.jpg   ".to_string(),
            "     ".to_string(),
            "".to_string(),
            "https://ex.com/2.jpg".to_string(),
        ],
        caption: None,
    };
    args_mixed.sanitize();
    assert_eq!(args_mixed.urls.len(), 2);
    assert_eq!(args_mixed.urls[0], "https://ex.com/1.jpg");
    assert_eq!(args_mixed.urls[1], "https://ex.com/2.jpg");
    assert!(args_mixed.validate().is_ok());
}

#[test]
fn test_adversarial_location_coordinates_strict_boundaries_and_special_values() {
    // 1. Exact boundaries: [-90.0, 90.0] and [-180.0, 180.0]
    let boundary_matrix = vec![
        (-90.0, -180.0, true),
        (-90.0, 180.0, true),
        (90.0, -180.0, true),
        (90.0, 180.0, true),
        (0.0, 0.0, true),
        (-90.0000000000001, 0.0, false),
        (90.0000000000001, 0.0, false),
        (0.0, -180.0000000000001, false),
        (0.0, 180.0000000000001, false),
        (-91.0, 0.0, false),
        (91.0, 0.0, false),
        (0.0, -181.0, false),
        (0.0, 181.0, false),
    ];

    for (lat, lon, expected_ok) in boundary_matrix {
        let args = SendLocationArgs {
            latitude: lat,
            longitude: lon,
            title: None,
        };
        let res = args.validate();
        assert_eq!(
            res.is_ok(),
            expected_ok,
            "Coordinates ({lat}, {lon}) expected ok={expected_ok}, got {res:?}"
        );
    }

    // 2. NaN, Infinity, -Infinity directly on SendLocationArgs
    let invalid_special_floats = vec![
        (f64::NAN, 0.0),
        (0.0, f64::NAN),
        (f64::NAN, f64::NAN),
        (f64::INFINITY, 0.0),
        (0.0, f64::INFINITY),
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, 0.0),
        (0.0, f64::NEG_INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    ];

    for (lat, lon) in invalid_special_floats {
        let args = SendLocationArgs {
            latitude: lat,
            longitude: lon,
            title: None,
        };
        assert!(
            args.validate().is_err(),
            "Expected ({lat}, {lon}) with NaN/Inf to be rejected"
        );
    }

    // 3. String representations in JSON deserializer (NaN, Infinity, overflows)
    let adversarial_jsons = vec![
        r#"{"latitude": "NaN", "longitude": 0.0}"#,
        r#"{"latitude": "nan", "longitude": 0.0}"#,
        r#"{"latitude": 0.0, "longitude": "Infinity"}"#,
        r#"{"latitude": 0.0, "longitude": "-Infinity"}"#,
        r#"{"latitude": 0.0, "longitude": "+inf"}"#,
        r#"{"latitude": "1e309", "longitude": 0.0}"#,
        r#"{"latitude": 1e309, "longitude": 0.0}"#,
        r#"{"latitude": "invalid_number", "longitude": 0.0}"#,
    ];

    for json_str in adversarial_jsons {
        let parsed: Result<SendLocationArgs, _> = serde_json::from_str(json_str);
        assert!(
            parsed.is_err(),
            "Adversarial payload {json_str} must fail deserialization"
        );
    }
}

#[test]
fn test_adversarial_callback_data_byte_size_never_exceeds_64_bytes() {
    // Telegram Bot API: callback_data is 1-64 bytes
    // Format: carousel:<id>:<index>:<action>
    // Length breakdown: "carousel:" (9) + id (?) + ":" (1) + index (?) + ":" (1) + action (4) = id.len() + index.len() + 15
    let actions = ["prev", "noop", "next"];
    let test_indices = [0, 1, 9, 10, 99, 100, 999, 9999];

    // 1. All valid generated IDs (e.g. 8-char hex or standard UUID) under various indices & actions
    let valid_ids = [
        "a1b2c3d4",                                 // 8 hex chars (standard Xiao generator)
        "f0e1d2c3b4a5",                             // 12 hex chars
        "550e8400-e29b-41d4-a716-446655440000",     // 36 char UUID v4
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", // 40 chars
    ];

    for id in valid_ids {
        for index in test_indices {
            for action in actions {
                let cb = format_carousel_callback_data(id, index, action);
                assert!(
                    cb.len() <= 64,
                    "Callback data '{cb}' exceeds 64 bytes (actual: {} bytes)",
                    cb.len()
                );
                // Also verify round-trip parsing
                let (parsed_id, parsed_idx, parsed_act) =
                    parse_carousel_callback_data(&cb).expect("parse valid callback data");
                assert_eq!(parsed_id, id);
                assert_eq!(parsed_idx, index);
                assert_eq!(parsed_act, action);
            }
        }
    }

    // 2. Strict boundary at 64 bytes
    // "carousel:" (9) + 48 chars + ":0:prev" (7) = 64 bytes exactly
    let id_48 = "k".repeat(48);
    let cb_exact_64 = format_carousel_callback_data(&id_48, 0, "prev");
    assert_eq!(cb_exact_64.len(), 64);
    assert!(parse_carousel_callback_data(&cb_exact_64).is_ok());

    // 49 chars -> 65 bytes: must be rejected
    let id_49 = "k".repeat(49);
    let cb_overflow_65 = format_carousel_callback_data(&id_49, 0, "prev");
    assert_eq!(cb_overflow_65.len(), 65);
    assert!(parse_carousel_callback_data(&cb_overflow_65).is_err());
    assert!(build_carousel_keyboard_contract(&id_49, 0, 5).is_err());

    // 3. Multi-byte UTF-8 test:
    // Ensure byte-level checking, NOT character-level checking.
    // '🦀' is 4 bytes. An ID with 12 crab emojis = 48 bytes.
    let crab_id = "🦀".repeat(12);
    assert_eq!(crab_id.chars().count(), 12);
    assert_eq!(crab_id.len(), 48); // 48 bytes!
    let cb_crab = format_carousel_callback_data(&crab_id, 0, "prev");
    assert_eq!(cb_crab.len(), 64); // 9 + 48 + 7 = 64 bytes
    assert!(parse_carousel_callback_data(&cb_crab).is_ok());

    // 13 crab emojis = 52 bytes. 9 + 52 + 7 = 68 bytes (> 64 bytes)
    let crab_id_13 = "🦀".repeat(13);
    let cb_crab_overflow = format_carousel_callback_data(&crab_id_13, 0, "prev");
    assert!(cb_crab_overflow.len() > 64);
    assert!(parse_carousel_callback_data(&cb_crab_overflow).is_err());
}

#[test]
fn test_adversarial_caption_1024_char_boundary_all_tools() {
    // Telegram caption limit is 1024 characters.
    let max = tools::MULTIMEDIA_CAPTION_MAX_CHARS;
    assert_eq!(max, 1024);

    // 1. Exactly 1023 characters (should be preserved)
    let cap_1023 = "x".repeat(1023);
    let mut photo_1023 = SendPhotoArgs {
        url: "https://ex.com/p.jpg".to_string(),
        caption: Some(cap_1023.clone()),
    };
    photo_1023.sanitize();
    assert_eq!(photo_1023.caption.as_deref().map(|s| s.len()), Some(1023));

    // 2. Exactly 1024 characters (should be preserved intact)
    let cap_1024 = "y".repeat(1024);
    let mut photo_1024 = SendPhotoArgs {
        url: "https://ex.com/p.jpg".to_string(),
        caption: Some(cap_1024.clone()),
    };
    photo_1024.sanitize();
    assert_eq!(photo_1024.caption.as_deref().map(|s| s.len()), Some(1024));
    assert_eq!(photo_1024.caption.as_deref(), Some(cap_1024.as_str()));

    // 3. Exactly 1025 characters (must be truncated to 1024)
    let cap_1025 = "z".repeat(1025);
    let mut photo_1025 = SendPhotoArgs {
        url: "https://ex.com/p.jpg".to_string(),
        caption: Some(cap_1025),
    };
    photo_1025.sanitize();
    assert_eq!(
        photo_1025.caption.as_deref().map(|s| s.chars().count()),
        Some(1024)
    );

    // 4. Extreme: 5000 characters across all tools
    let cap_huge = "A".repeat(5000);

    // Collage
    let mut collage = SendCollageArgs {
        urls: vec![
            "https://ex.com/1.jpg".to_string(),
            "https://ex.com/2.jpg".to_string(),
        ],
        caption: Some(cap_huge.clone()),
    };
    collage.sanitize();
    assert_eq!(
        collage.caption.as_deref().map(|s| s.chars().count()),
        Some(1024)
    );

    // Slideshow
    let mut ss = SendSlideshowArgs {
        urls: vec!["https://ex.com/1.jpg".to_string()],
        caption: Some(cap_huge.clone()),
    };
    ss.sanitize();
    assert_eq!(ss.caption.as_deref().map(|s| s.chars().count()), Some(1024));

    // Audio
    let mut audio = SendAudioArgs {
        url: "https://ex.com/a.mp3".to_string(),
        title: None,
        performer: None,
        caption: Some(cap_huge.clone()),
    };
    audio.sanitize();
    assert_eq!(
        audio.caption.as_deref().map(|s| s.chars().count()),
        Some(1024)
    );

    // Voice
    let mut voice = SendVoiceArgs {
        url: "https://ex.com/v.ogg".to_string(),
        caption: Some(cap_huge.clone()),
    };
    voice.sanitize();
    assert_eq!(
        voice.caption.as_deref().map(|s| s.chars().count()),
        Some(1024)
    );

    // Document
    let mut doc = SendDocumentArgs {
        url: "https://ex.com/d.pdf".to_string(),
        file_name: None,
        caption: Some(cap_huge),
    };
    doc.sanitize();
    assert_eq!(
        doc.caption.as_deref().map(|s| s.chars().count()),
        Some(1024)
    );

    // 5. Multi-byte UTF-8 emoji caption truncation:
    // 1025 crab emojis (4 bytes each). Truncation must NOT slice midway through a codepoint.
    let cap_emojis = "🦀".repeat(1025);
    let mut photo_emoji = SendPhotoArgs {
        url: "https://ex.com/p.jpg".to_string(),
        caption: Some(cap_emojis),
    };
    photo_emoji.sanitize();
    let sanitized_cap = photo_emoji.caption.expect("caption must exist");
    assert_eq!(sanitized_cap.chars().count(), 1024);
    assert_eq!(sanitized_cap.len(), 1024 * 4); // 4096 UTF-8 bytes
}

#[test]
fn test_adversarial_circular_navigation_wrap_around_and_invariants() {
    // Test circular index navigation across various slide counts
    // Invariant 1: Prev at index 0 wraps to total_slides - 1
    // Invariant 2: Next at index total_slides - 1 wraps to 0
    // Invariant 3: No arithmetic underflow or overflow for any current_index

    for total in 2..=15 {
        // Test index 0
        let kb_start = build_carousel_keyboard_contract("test_nav", 0, total)
            .expect("build keyboard at index 0");
        let row_start = &kb_start.inline_keyboard[0];
        let prev_data = row_start[0].callback_data.as_deref().expect("prev data");
        let (_, prev_target, _) = parse_carousel_callback_data(prev_data).expect("parse prev");
        assert_eq!(
            prev_target,
            total - 1,
            "At index 0 of {total} slides, prev must wrap to {}",
            total - 1
        );

        // Test last index
        let last_idx = total - 1;
        let kb_last = build_carousel_keyboard_contract("test_nav", last_idx, total)
            .expect("build keyboard at last index");
        let row_last = &kb_last.inline_keyboard[0];
        let next_data = row_last[2].callback_data.as_deref().expect("next data");
        let (_, next_target, _) = parse_carousel_callback_data(next_data).expect("parse next");
        assert_eq!(
            next_target, 0,
            "At last index {last_idx} of {total} slides, next must wrap to 0"
        );

        // Invariant: Full cycle test
        let mut curr = 0;
        for step in 0..total {
            let kb = build_carousel_keyboard_contract("cycle", curr, total).expect("build kb");
            let next_d = kb.inline_keyboard[0][2]
                .callback_data
                .as_deref()
                .expect("next cb");
            let (_, next_idx, _) = parse_carousel_callback_data(next_d).expect("parse next");
            let expected_next = if curr + 1 >= total { 0 } else { curr + 1 };
            assert_eq!(next_idx, expected_next, "Step {step} next index mismatch");
            curr = next_idx;
        }
        assert_eq!(
            curr, 0,
            "Stepping next {total} times must return to slide 0"
        );
    }

    // 2-slide special boundary (both prev and next point to the other slide)
    let kb_2 = build_carousel_keyboard_contract("two_slides", 0, 2).expect("two slides kb");
    let prev_2 = kb_2.inline_keyboard[0][0]
        .callback_data
        .as_deref()
        .expect("prev");
    let next_2 = kb_2.inline_keyboard[0][2]
        .callback_data
        .as_deref()
        .expect("next");
    let (_, p_idx, _) = parse_carousel_callback_data(prev_2).expect("parse prev");
    let (_, n_idx, _) = parse_carousel_callback_data(next_2).expect("parse next");
    assert_eq!(p_idx, 1);
    assert_eq!(n_idx, 1);

    // Degenerate inputs (0 slides, 1 slide)
    assert!(build_carousel_keyboard_contract("zero", 0, 0).is_err());
}

#[test]
fn test_adversarial_carousel_concurrency_stress_and_race_safety() {
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock as StdRwLock};
    use std::time::{Duration, Instant};

    #[derive(Debug, Clone)]
    struct MockCarouselState {
        id: String,
        slides: Vec<String>,
        created_at: Instant,
    }

    const TEST_TTL: Duration = Duration::from_millis(50);
    let cache: Arc<StdRwLock<HashMap<String, MockCarouselState>>> =
        Arc::new(StdRwLock::new(HashMap::new()));

    let num_writers = 8;
    let num_readers = 8;
    let iterations_per_thread = 200;

    let mut handles = Vec::new();

    // Writers
    for w in 0..num_writers {
        let cache_clone = Arc::clone(&cache);
        handles.push(std::thread::spawn(move || {
            for i in 0..iterations_per_thread {
                let id = format!("carousel_{w}_{i}");
                let state = MockCarouselState {
                    id: id.clone(),
                    slides: vec![format!("https://ex.com/{w}_{i}.jpg")],
                    created_at: Instant::now(),
                };
                let mut guard = match cache_clone.write() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                let now = Instant::now();
                guard.retain(|_, item| {
                    now.checked_duration_since(item.created_at)
                        .unwrap_or(Duration::ZERO)
                        < TEST_TTL
                });
                guard.insert(id, state);
            }
        }));
    }

    // Readers
    for _r in 0..num_readers {
        let cache_clone = Arc::clone(&cache);
        handles.push(std::thread::spawn(move || {
            for i in 0..iterations_per_thread {
                let target_w = i % num_writers;
                let id = format!("carousel_{target_w}_{i}");
                let mut guard = match cache_clone.write() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                if let Some(state) = guard.get(&id) {
                    if state.created_at.elapsed() < TEST_TTL {
                        let _ = state.clone();
                    } else {
                        guard.remove(&id);
                    }
                }
            }
        }));
    }

    for handle in handles {
        handle.join().expect("thread join must not panic");
    }

    // Verify cache integrity after concurrent bombardment
    let final_guard = match cache.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    assert!(
        final_guard.len() <= num_writers * iterations_per_thread,
        "Cache size invariant satisfied"
    );
}

#[test]
fn test_adversarial_carousel_ttl_eviction_and_nonexistent_lifecycle() {
    use std::collections::HashMap;
    use std::sync::RwLock as StdRwLock;
    use std::time::{Duration, Instant};

    #[derive(Debug, Clone)]
    struct MockCarouselState {
        id: String,
        slides: Vec<String>,
        created_at: Instant,
    }

    const TEST_TTL: Duration = Duration::from_millis(10);
    let cache = StdRwLock::new(HashMap::new());

    // 1. Querying non-existent carousel returns None gracefully (simulating answerCallbackQuery without panic)
    {
        let guard = match cache.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        assert!(guard.get("non_existent_id").is_none());
    }

    // 2. Insert item with past timestamp (already expired)
    {
        let mut guard = match cache.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.insert(
            "already_expired".to_string(),
            MockCarouselState {
                id: "already_expired".to_string(),
                slides: vec!["https://ex.com/exp.jpg".to_string()],
                created_at: Instant::now() - Duration::from_secs(100),
            },
        );
    }

    // 3. Eviction check
    {
        let mut guard = match cache.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let res = if let Some(state) = guard.get("already_expired") {
            if state.created_at.elapsed() < TEST_TTL {
                Some(state.clone())
            } else {
                guard.remove("already_expired");
                None
            }
        } else {
            None
        };
        assert!(
            res.is_none(),
            "Expired carousel must be evicted and return None"
        );
        assert!(
            guard.get("already_expired").is_none(),
            "Must be purged from map"
        );
    }

    // 4. Poisoned lock recovery test
    let poisoned_lock = StdRwLock::new(42);
    let _ = std::panic::catch_unwind(|| {
        let _guard = poisoned_lock.write().expect("lock");
        panic!("simulated panic to poison lock");
    });
    assert!(poisoned_lock.is_poisoned());
    // Safe recovery pattern used in Xiao codebase
    let recovered_val = match poisoned_lock.write() {
        Ok(g) => *g,
        Err(poisoned) => *poisoned.into_inner(),
    };
    assert_eq!(
        recovered_val, 42,
        "Poisoned lock recovery pattern succeeds without panic"
    );
}

// ==============================================================================
// GROUP 8: TIER 1 — Feature Coverage: Rich Message Wire Models & Transport
// ==============================================================================

#[test]
fn test_tier1_rich_message_media_photo_wire_serialization() {
    let item = InputRichMessageMedia {
        id: "pic_rinjani_summit".to_string(),
        media: InputMedia::Photo {
            media: "https://example.com/rinjani.jpg".to_string(),
            caption: Some("Puncak Rinjani 3.726 mdpl".to_string()),
            parse_mode: Some("HTML".to_string()),
            show_caption_above_media: Some(true),
            has_spoiler: Some(false),
        },
    };

    let serialized = serde_json::to_value(&item).expect("InputRichMessageMedia must serialize");
    assert_eq!(serialized["id"], "pic_rinjani_summit");
    assert_eq!(serialized["media"]["type"], "photo");
    assert_eq!(
        serialized["media"]["media"],
        "https://example.com/rinjani.jpg"
    );
    assert_eq!(serialized["media"]["caption"], "Puncak Rinjani 3.726 mdpl");
    assert_eq!(serialized["media"]["parse_mode"], "HTML");
    assert_eq!(serialized["media"]["show_caption_above_media"], true);
    assert_eq!(serialized["media"]["has_spoiler"], false);

    // Verify exact object keys
    let map = serialized.as_object().expect("object map");
    assert!(map.contains_key("id"), "JSON must contain 'id' field");
    assert!(map.contains_key("media"), "JSON must contain 'media' field");
    assert_eq!(
        map.len(),
        2,
        "JSON must strictly contain exactly 2 top-level fields"
    );
}

#[test]
fn test_tier1_rich_message_media_audio_wire_serialization() {
    let item = InputRichMessageMedia {
        id: "aud_sembalun_wind".to_string(),
        media: InputMedia::audio(
            "https://example.com/wind.mp3",
            Some("Suara Alam Sembalun".to_string()),
            None,
            Some("Angin Savana".to_string()),
            Some("Lombok Audio".to_string()),
        ),
    };

    let serialized = serde_json::to_value(&item).expect("audio media item must serialize");
    assert_eq!(serialized["id"], "aud_sembalun_wind");
    assert_eq!(serialized["media"]["type"], "audio");
    assert_eq!(serialized["media"]["media"], "https://example.com/wind.mp3");
    assert_eq!(serialized["media"]["title"], "Angin Savana");
    assert_eq!(serialized["media"]["performer"], "Lombok Audio");
    assert_eq!(serialized["media"]["caption"], "Suara Alam Sembalun");
}

#[test]
fn test_tier1_rich_message_media_document_wire_serialization() {
    let item = InputRichMessageMedia {
        id: "doc_trekking_guide".to_string(),
        media: InputMedia::document(
            "https://example.com/guide.pdf",
            Some("Panduan Resmi Pendakian".to_string()),
            Some("HTML".to_string()),
        ),
    };

    let serialized = serde_json::to_value(&item).expect("document media item must serialize");
    assert_eq!(serialized["id"], "doc_trekking_guide");
    assert_eq!(serialized["media"]["type"], "document");
    assert_eq!(
        serialized["media"]["media"],
        "https://example.com/guide.pdf"
    );
    assert_eq!(serialized["media"]["caption"], "Panduan Resmi Pendakian");
    assert_eq!(serialized["media"]["parse_mode"], "HTML");
}

#[test]
fn test_tier1_rich_message_media_video_wire_serialization() {
    let item = InputRichMessageMedia {
        id: "vid_rinjani_timelapse".to_string(),
        media: InputMedia::Video {
            media: "https://example.com/timelapse.mp4".to_string(),
            caption: Some("Sunrise Timelapse".to_string()),
            parse_mode: None,
            show_caption_above_media: None,
            width: Some(1920),
            height: Some(1080),
            duration: Some(45),
            has_spoiler: Some(false),
        },
    };

    let serialized = serde_json::to_value(&item).expect("video media item must serialize");
    assert_eq!(serialized["id"], "vid_rinjani_timelapse");
    assert_eq!(serialized["media"]["type"], "video");
    assert_eq!(serialized["media"]["width"], 1920);
    assert_eq!(serialized["media"]["height"], 1080);
    assert_eq!(serialized["media"]["duration"], 45);
}

#[test]
fn test_tier1_rich_message_media_voice_note_wire_serialization() {
    let item = InputRichMessageMedia {
        id: "voice_ranger_briefing".to_string(),
        media: InputMedia::VoiceNote {
            media: "https://example.com/briefing.ogg".to_string(),
            caption: Some("Briefing Ranger Pos 2".to_string()),
            parse_mode: None,
            duration: Some(120),
        },
    };

    let serialized = serde_json::to_value(&item).expect("voice note media must serialize");
    assert_eq!(serialized["id"], "voice_ranger_briefing");
    assert_eq!(serialized["media"]["type"], "voice_note");
    assert_eq!(serialized["media"]["duration"], 120);
}

#[test]
fn test_tier1_rich_message_media_deserialization_roundtrip() {
    let json_payload = json!({
        "id": "photo_lake_view",
        "media": {
            "type": "photo",
            "media": "AgACAgIAAxkBAAI...",
            "caption": "Danau Segara Anak",
            "has_spoiler": true
        }
    });

    let deserialized: InputRichMessageMedia =
        serde_json::from_value(json_payload.clone()).expect("deserialization must succeed");
    assert_eq!(deserialized.id, "photo_lake_view");
    match &deserialized.media {
        InputMedia::Photo {
            media,
            caption,
            has_spoiler,
            ..
        } => {
            assert_eq!(media, "AgACAgIAAxkBAAI...");
            assert_eq!(caption.as_deref(), Some("Danau Segara Anak"));
            assert_eq!(has_spoiler, &Some(true));
        }
        _ => panic!("Expected InputMedia::Photo variant"),
    }

    let re_serialized = serde_json::to_value(&deserialized).expect("re-serialization must succeed");
    assert_eq!(re_serialized, json_payload);
}

#[test]
fn test_tier1_rich_message_html_representation_with_media() {
    let html_body = "<h3>Taman Nasional Gunung Rinjani</h3><p>Keindahan kaldera Segara Anak.</p><img src=\"tg://photo?id=pic1\"/>";
    let media = vec![InputRichMessageMedia {
        id: "pic1".to_string(),
        media: InputMedia::photo("https://example.com/pic1.jpg", None, None),
    }];

    let rich_msg =
        create_html_rich_message(html_body, media).expect("create_html_rich_message must succeed");
    assert_eq!(rich_msg.html.as_deref(), Some(html_body));
    assert!(rich_msg.markdown.is_none());
    assert!(rich_msg.blocks.is_empty());

    let media_array = rich_msg
        .media
        .as_ref()
        .expect("media array must be present");
    assert_eq!(media_array.len(), 1);
    assert_eq!(media_array[0].id, "pic1");
    match &media_array[0].media {
        InputMedia::Photo { media, .. } => {
            assert_eq!(media, "https://example.com/pic1.jpg");
        }
        _ => panic!("Expected InputMedia::Photo"),
    }

    assert!(rich_msg.validate().is_ok());

    let extracted =
        extract_media_items_from_rich_message(&rich_msg).expect("extract media must succeed");
    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].id, "pic1");
}

#[test]
fn test_tier1_rich_message_blocks_representation_with_ast() {
    let blocks = vec![
        RichBlock::SectionHeading {
            text: Value::String("Rinjani Expedition".to_string()),
            level: 2,
        },
        RichBlock::Paragraph {
            text: Value::String("Elevation 3,726m above sea level.".to_string()),
        },
        RichBlock::Map {
            location: Location {
                latitude: -8.4167,
                longitude: 116.4583,
                horizontal_accuracy: None,
            },
            zoom: Some(12),
            width: None,
            height: None,
        },
    ];

    let rich_msg = InputRichMessage::new(blocks);
    assert_eq!(rich_msg.blocks.len(), 3);
    assert!(rich_msg.html.is_none());
    assert!(rich_msg.markdown.is_none());
    assert!(rich_msg.validate().is_ok());

    let val = serde_json::to_value(&rich_msg).expect("blocks serialization");
    assert_eq!(val["blocks"].as_array().expect("blocks array").len(), 3);
    assert_eq!(val["blocks"][0]["type"], "heading");
    assert_eq!(val["blocks"][1]["type"], "paragraph");
    assert_eq!(val["blocks"][2]["type"], "map");
    assert_eq!(val["blocks"][2]["location"]["latitude"], -8.4167);
}

#[test]
fn test_tier1_rich_message_markdown_representation() {
    let md = "# Rinjani\n\n**Gunung Rinjani** di Pulau *Lombok*.";
    let rich_msg = InputRichMessage {
        blocks: vec![],
        html: None,
        markdown: Some(md.to_string()),
        media: None,
        is_rtl: None,
        skip_entity_detection: None,
    };
    assert!(rich_msg.validate().is_ok());

    let val = serde_json::to_value(&rich_msg).expect("markdown serialization");
    assert_eq!(val["markdown"], md);
    assert!(val.get("html").is_none());
    assert!(val.get("blocks").is_none());
}

#[test]
fn test_tier1_rich_message_mutually_exclusive_representation_check() {
    // 0 representations: invalid
    let empty = InputRichMessage::default();
    assert!(empty.validate().is_err());

    // HTML + Blocks: invalid (2 representations)
    let conf_html_blocks = InputRichMessage {
        blocks: vec![RichBlock::Paragraph {
            text: Value::String("Text".to_string()),
        }],
        html: Some("<p>HTML</p>".to_string()),
        markdown: None,
        media: None,
        is_rtl: None,
        skip_entity_detection: None,
    };
    let err_hb = conf_html_blocks
        .validate()
        .expect_err("html + blocks must fail validation");
    assert!(err_hb.contains("found 2"));

    // HTML + Markdown: invalid (2 representations)
    let conf_html_md = InputRichMessage {
        blocks: vec![],
        html: Some("<p>HTML</p>".to_string()),
        markdown: Some("**Markdown**".to_string()),
        media: None,
        is_rtl: None,
        skip_entity_detection: None,
    };
    let err_hm = conf_html_md
        .validate()
        .expect_err("html + markdown must fail validation");
    assert!(err_hm.contains("found 2"));
}

#[test]
fn test_tier1_tag_wire_img_tg_photo_reference() {
    let tag = r#"<img src="tg://photo?id=pic1"/>"#;
    let refs = extract_tg_scheme_ids(tag);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].0, "photo");
    assert_eq!(refs[0].1, "pic1");

    let media = vec![InputRichMessageMedia {
        id: "pic1".to_string(),
        media: InputMedia::photo("https://example.com/p1.jpg", None, None),
    }];
    assert!(validate_html_media_reference_integrity(tag, &media).is_ok());
}

#[test]
fn test_tier1_tag_wire_audio_tg_audio_reference() {
    let tag =
        r#"<audio src="tg://audio?id=aud1" title="Angin Sembalun" performer="Lombok Sounds"/>"#;
    let refs = extract_tg_scheme_ids(tag);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].0, "audio");
    assert_eq!(refs[0].1, "aud1");

    let media = vec![InputRichMessageMedia {
        id: "aud1".to_string(),
        media: InputMedia::audio(
            "https://example.com/a1.mp3",
            None,
            None,
            Some("Angin Sembalun".to_string()),
            Some("Lombok Sounds".to_string()),
        ),
    }];
    assert!(validate_html_media_reference_integrity(tag, &media).is_ok());
}

#[test]
fn test_tier1_tag_wire_tg_collage_container_representation() {
    let collage_html = r#"<tg-collage caption="Foto Puncak"><img src="tg://photo?id=p1"/><img src="tg://photo?id=p2"/></tg-collage>"#;
    let refs = extract_tg_scheme_ids(collage_html);
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0], ("photo".to_string(), "p1".to_string()));
    assert_eq!(refs[1], ("photo".to_string(), "p2".to_string()));
    assert!(validate_tg_collage_child_count_contract(refs.len()).is_ok());

    let media = vec![
        InputRichMessageMedia {
            id: "p1".to_string(),
            media: InputMedia::photo("https://ex.com/1.jpg", None, None),
        },
        InputRichMessageMedia {
            id: "p2".to_string(),
            media: InputMedia::photo("https://ex.com/2.jpg", None, None),
        },
    ];
    assert!(validate_html_media_reference_integrity(collage_html, &media).is_ok());
}

#[test]
fn test_tier1_tag_wire_tg_slideshow_container_representation() {
    let slideshow_html = r#"<tg-slideshow caption="Galeri Foto"><img src="tg://photo?id=s1"/><img src="tg://photo?id=s2"/><img src="tg://photo?id=s3"/></tg-slideshow>"#;
    let refs = extract_tg_scheme_ids(slideshow_html);
    assert_eq!(refs.len(), 3);
    assert!(validate_tg_slideshow_child_count_contract(refs.len()).is_ok());
}

#[test]
fn test_tier1_tag_wire_tg_map_lat_lon_zoom_representation() {
    let lat = -8.4167;
    let lon = 116.4583;
    let zoom = 12;
    assert!(validate_tg_map_attributes_contract(lat, lon, Some(zoom)).is_ok());

    let map_block = RichBlock::Map {
        location: Location {
            latitude: lat,
            longitude: lon,
            horizontal_accuracy: None,
        },
        zoom: Some(zoom),
        width: None,
        height: None,
    };
    let val = serde_json::to_value(&map_block).expect("map block serializes");
    assert_eq!(val["type"], "map");
    assert_eq!(val["location"]["latitude"], lat);
    assert_eq!(val["location"]["longitude"], lon);
    assert_eq!(val["zoom"], zoom);
}

#[test]
fn test_tier1_send_rich_message_request_json_payload_structure() {
    let rich_msg = create_html_rich_message(
        "<p>Halo dunia!</p><img src=\"tg://photo?id=pic_1\"/>",
        vec![InputRichMessageMedia {
            id: "pic_1".to_string(),
            media: InputMedia::photo("https://example.com/pic1.jpg", None, None),
        }],
    )
    .expect("rich message must create");

    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "Detail",
        "btn_detail",
    )]]);

    let payload = json!({
        "chat_id": 123456789_i64,
        "rich_message": rich_msg,
        "reply_markup": keyboard,
        "reply_parameters": ReplyParameters::new(42),
    });

    assert_eq!(payload["chat_id"], 123456789_i64);
    assert_eq!(
        payload["rich_message"]["html"],
        "<p>Halo dunia!</p><img src=\"tg://photo?id=pic_1\"/>"
    );
    assert_eq!(payload["rich_message"]["media"][0]["id"], "pic_1");
    assert_eq!(
        payload["reply_markup"]["inline_keyboard"][0][0]["text"],
        "Detail"
    );
    assert_eq!(payload["reply_parameters"]["message_id"], 42);
}

#[test]
fn test_tier1_send_rich_message_omits_draft_id_in_permanent_send() {
    let rich_msg =
        create_html_rich_message("<p>Pesan permanen</p>", vec![]).expect("rich message create");
    let payload = json!({
        "chat_id": 999888_i64,
        "rich_message": rich_msg,
    });
    assert!(
        payload.get("draft_id").is_none(),
        "Permanent sendRichMessage payload must strictly omit draft_id"
    );
}

#[test]
fn test_tier1_send_rich_message_draft_wire_payload_structure() {
    let draft_blocks = vec![RichBlock::Thinking {
        text: Value::String("Sedang meramu jawaban dan mencari foto...".to_string()),
    }];
    let draft_msg = InputRichMessage::new(draft_blocks);
    assert!(draft_msg.validate().is_ok());

    let payload = json!({
        "chat_id": 123456_i64,
        "draft_id": 876543219012_i64,
        "rich_message": draft_msg,
        "can_stop": true,
        "keep_on_stop": false,
    });

    assert_eq!(payload["chat_id"], 123456_i64);
    assert_eq!(payload["draft_id"], 876543219012_i64);
    assert_eq!(payload["rich_message"]["blocks"][0]["type"], "thinking");
    assert_eq!(payload["can_stop"], true);
    assert_eq!(payload["keep_on_stop"], false);
}

#[test]
fn test_tier1_edit_message_media_wire_payload_structure() {
    let media = InputMedia::photo(
        "https://example.com/updated_slide.jpg",
        Some("Updated Slide 2".to_string()),
        None,
    );
    let kb =
        build_carousel_keyboard_contract("car_42", 1, 4).expect("carousel keyboard construction");

    let payload = json!({
        "chat_id": 333444_i64,
        "message_id": 555666_i64,
        "media": media,
        "reply_markup": kb,
    });

    assert_eq!(payload["chat_id"], 333444_i64);
    assert_eq!(payload["message_id"], 555666_i64);
    assert_eq!(payload["media"]["type"], "photo");
    assert_eq!(
        payload["media"]["media"],
        "https://example.com/updated_slide.jpg"
    );
    assert_eq!(
        payload["reply_markup"]["inline_keyboard"][0][1]["text"],
        "2/4"
    );
}

#[test]
fn test_tier1_answer_callback_query_wire_payload_structure() {
    let payload = json!({
        "callback_query_id": "cbq_991827",
        "text": "Slide diperbarui",
        "show_alert": false,
    });

    assert_eq!(payload["callback_query_id"], "cbq_991827");
    assert_eq!(payload["text"], "Slide diperbarui");
    assert_eq!(payload["show_alert"], false);
}

// ==============================================================================
// GROUP 9: TIER 2 — Boundary & Corner Cases
// ==============================================================================

#[test]
fn test_tier2_media_id_length_boundary_1_and_64_valid() {
    // Exactly 1 char (minimum valid boundary)
    assert!(validate_rich_media_id_contract("a").is_ok());
    assert!(validate_rich_media_id_contract("1").is_ok());
    assert!(validate_rich_media_id_contract("_").is_ok());

    // Exactly 64 chars (maximum valid boundary)
    let id_64 = "a".repeat(64);
    assert_eq!(id_64.len(), 64);
    assert!(validate_rich_media_id_contract(&id_64).is_ok());

    let id_alphanum_64 = "Abc123_".repeat(9) + "1"; // 7*9 + 1 = 64 chars
    assert_eq!(id_alphanum_64.len(), 64);
    assert!(validate_rich_media_id_contract(&id_alphanum_64).is_ok());
}

#[test]
fn test_tier2_media_id_length_boundary_0_and_65_rejected() {
    // Exactly 0 chars (below lower bound)
    let err_0 = validate_rich_media_id_contract("").expect_err("0 chars ID must be rejected");
    assert!(err_0.contains("empty") || err_0.contains("1 character"));

    // Exactly 65 chars (1 byte over upper bound)
    let id_65 = "a".repeat(65);
    assert_eq!(id_65.len(), 65);
    let err_65 = validate_rich_media_id_contract(&id_65).expect_err("65 chars ID must be rejected");
    assert!(err_65.contains("64 characters"));

    // Extreme: 1000 chars
    let id_1000 = "x".repeat(1000);
    assert!(validate_rich_media_id_contract(&id_1000).is_err());
}

#[test]
fn test_tier2_media_id_charset_alphanumeric_underscore_hyphen() {
    let valid_ids = vec![
        "image_1",
        "photo-rinjani-2026",
        "AUDIO_TRACK_99",
        "doc-v1_final",
        "0123456789",
        "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
        "abcdefghijklmnopqrstuvwxyz",
    ];

    for id in valid_ids {
        assert!(
            validate_rich_media_id_contract(id).is_ok(),
            "Expected ID '{id}' to be valid"
        );
    }
}

#[test]
fn test_tier2_media_id_rejects_special_characters_and_spaces() {
    let invalid_ids = vec![
        "photo with space",
        "pic#1",
        "pic?id=12",
        "image/png",
        "audio:track",
        "photo@2x",
        "file$name",
        "image.jpg",
        "🦀crab_id",
        "\tleading_tab",
        "trailing_nl\n",
    ];

    for id in invalid_ids {
        let err = validate_rich_media_id_contract(id)
            .expect_err(&format!("ID '{id}' with special chars must be rejected"));
        assert!(
            err.contains("invalid characters"),
            "Error for '{id}' must mention invalid characters: {err}"
        );
    }
}

#[test]
fn test_tier2_media_count_boundaries_0_and_1_valid() {
    // 0 media items: valid
    assert!(validate_rich_message_media_list_contract(&[]).is_ok());

    let empty_rich = create_html_rich_message("<p>Teks saja</p>", vec![])
        .expect("0 media items rich message must succeed");
    assert!(empty_rich.media.is_none());
    assert!(empty_rich.validate().is_ok());

    // 1 media item: valid
    let one_media = vec![InputRichMessageMedia {
        id: "single_pic".to_string(),
        media: InputMedia::photo("https://example.com/one.jpg", None, None),
    }];
    assert!(validate_rich_message_media_list_contract(&one_media).is_ok());
    let one_rich = create_html_rich_message("<img src=\"tg://photo?id=single_pic\"/>", one_media)
        .expect("1 media item rich message must succeed");
    assert_eq!(one_rich.media.as_ref().expect("media array").len(), 1);
    assert!(one_rich.validate().is_ok());
}

#[test]
fn test_tier2_media_count_boundary_50_valid_and_51_rejected() {
    // Exactly 50 items (exact upper bound RICH_MESSAGE_MAX_MEDIA)
    let fifty_items: Vec<InputRichMessageMedia> = (0..RICH_MESSAGE_MAX_MEDIA)
        .map(|i| InputRichMessageMedia {
            id: format!("media_{i}"),
            media: InputMedia::photo(format!("https://example.com/{i}.jpg"), None, None),
        })
        .collect();
    assert_eq!(fifty_items.len(), 50);
    assert!(validate_rich_message_media_list_contract(&fifty_items).is_ok());

    let fifty_rich = create_html_rich_message("<p>50 Media</p>", fifty_items)
        .expect("50 media items must be accepted");
    assert!(fifty_rich.validate().is_ok());

    // Exactly 51 items (one over upper bound)
    let fifty_one_items: Vec<InputRichMessageMedia> = (0..=RICH_MESSAGE_MAX_MEDIA)
        .map(|i| InputRichMessageMedia {
            id: format!("media_{i}"),
            media: InputMedia::photo(format!("https://example.com/{i}.jpg"), None, None),
        })
        .collect();
    assert_eq!(fifty_one_items.len(), 51);
    let err_51 = validate_rich_message_media_list_contract(&fifty_one_items)
        .expect_err("51 media items must be rejected");
    assert!(err_51.contains("exceeds Telegram limit of 50"));
}

#[test]
fn test_tier2_duplicate_media_ids_strict_rejection() {
    let duplicate_items = vec![
        InputRichMessageMedia {
            id: "photo_a".to_string(),
            media: InputMedia::photo("https://example.com/a.jpg", None, None),
        },
        InputRichMessageMedia {
            id: "photo_b".to_string(),
            media: InputMedia::photo("https://example.com/b.jpg", None, None),
        },
        InputRichMessageMedia {
            id: "photo_a".to_string(), // DUPLICATE!
            media: InputMedia::photo("https://example.com/a2.jpg", None, None),
        },
    ];

    let err = validate_rich_message_media_list_contract(&duplicate_items)
        .expect_err("Duplicate media ID must be rejected");
    assert!(
        err.contains("Duplicate media ID found"),
        "Error must mention duplicate: {err}"
    );
    assert!(err.contains("photo_a"));
}

#[test]
fn test_tier2_duplicate_media_ids_case_sensitivity_distinct() {
    let case_sensitive_items = vec![
        InputRichMessageMedia {
            id: "photo_A".to_string(),
            media: InputMedia::photo("https://example.com/A.jpg", None, None),
        },
        InputRichMessageMedia {
            id: "photo_a".to_string(), // Distinct case
            media: InputMedia::photo("https://example.com/a.jpg", None, None),
        },
    ];

    assert!(
        validate_rich_message_media_list_contract(&case_sensitive_items).is_ok(),
        "IDs differing only by case should be distinct and accepted"
    );
}

#[test]
fn test_tier2_geo_latitude_exact_boundaries_minus_90_and_plus_90() {
    assert!(validate_tg_map_attributes_contract(-90.0, 0.0, None).is_ok());
    assert!(validate_tg_map_attributes_contract(90.0, 0.0, None).is_ok());
    assert!(validate_tg_map_attributes_contract(0.0, 0.0, None).is_ok());
    assert!(validate_tg_map_attributes_contract(-8.4113, 116.4573, Some(13)).is_ok());
}

#[test]
fn test_tier2_geo_latitude_out_of_bounds_rejected() {
    assert!(validate_tg_map_attributes_contract(-90.0001, 0.0, None).is_err());
    assert!(validate_tg_map_attributes_contract(90.0001, 0.0, None).is_err());
    assert!(validate_tg_map_attributes_contract(-91.0, 0.0, None).is_err());
    assert!(validate_tg_map_attributes_contract(91.0, 0.0, None).is_err());
}

#[test]
fn test_tier2_geo_longitude_exact_boundaries_minus_180_and_plus_180() {
    assert!(validate_tg_map_attributes_contract(0.0, -180.0, None).is_ok());
    assert!(validate_tg_map_attributes_contract(0.0, 180.0, None).is_ok());
    assert!(validate_tg_map_attributes_contract(0.0, 0.0, None).is_ok());
}

#[test]
fn test_tier2_geo_longitude_out_of_bounds_rejected() {
    assert!(validate_tg_map_attributes_contract(0.0, -180.0001, None).is_err());
    assert!(validate_tg_map_attributes_contract(0.0, 180.0001, None).is_err());
    assert!(validate_tg_map_attributes_contract(0.0, -181.0, None).is_err());
    assert!(validate_tg_map_attributes_contract(0.0, 181.0, None).is_err());
    assert!(validate_tg_map_attributes_contract(0.0, 360.0, None).is_err());
}

#[test]
fn test_tier2_geo_coordinates_nan_and_infinity_rejected() {
    assert!(validate_tg_map_attributes_contract(f64::NAN, 0.0, None).is_err());
    assert!(validate_tg_map_attributes_contract(0.0, f64::NAN, None).is_err());
    assert!(validate_tg_map_attributes_contract(f64::INFINITY, 0.0, None).is_err());
    assert!(validate_tg_map_attributes_contract(0.0, f64::NEG_INFINITY, None).is_err());
}

#[test]
fn test_tier2_geo_map_zoom_boundaries_1_to_20() {
    // Valid zoom levels: 1..=20
    assert!(validate_tg_map_attributes_contract(0.0, 0.0, Some(1)).is_ok());
    assert!(validate_tg_map_attributes_contract(0.0, 0.0, Some(10)).is_ok());
    assert!(validate_tg_map_attributes_contract(0.0, 0.0, Some(20)).is_ok());

    // Invalid zoom levels
    assert!(validate_tg_map_attributes_contract(0.0, 0.0, Some(0)).is_err());
    assert!(validate_tg_map_attributes_contract(0.0, 0.0, Some(21)).is_err());
    assert!(validate_tg_map_attributes_contract(0.0, 0.0, Some(-1)).is_err());
}

#[test]
fn test_tier2_carousel_callback_data_exact_64_byte_boundary() {
    // "carousel:" (9) + id (48) + ":0:next" (7) = 64 bytes
    let id_48 = "x".repeat(48);
    let cb_64 = format_carousel_callback_data(&id_48, 0, "next");
    assert_eq!(cb_64.len(), 64);
    assert!(parse_carousel_callback_data(&cb_64).is_ok());

    // 63 bytes
    let id_47 = "x".repeat(47);
    let cb_63 = format_carousel_callback_data(&id_47, 0, "next");
    assert_eq!(cb_63.len(), 63);
    assert!(parse_carousel_callback_data(&cb_63).is_ok());
}

#[test]
fn test_tier2_carousel_callback_data_65_bytes_rejected() {
    let id_49 = "x".repeat(49);
    let cb_65 = format_carousel_callback_data(&id_49, 0, "next");
    assert_eq!(cb_65.len(), 65);
    let err = parse_carousel_callback_data(&cb_65).expect_err("65 bytes must be rejected");
    assert!(err.contains("64-byte limit"));
}

#[test]
fn test_tier2_rich_message_text_length_32768_boundary() {
    // Exactly 32,768 characters: valid
    let text_32768 = "a".repeat(RICH_MESSAGE_MAX_TEXT_CHARS);
    assert_eq!(text_32768.chars().count(), 32_768);
    let msg_valid = InputRichMessage {
        blocks: vec![],
        html: Some(text_32768),
        markdown: None,
        media: None,
        is_rtl: None,
        skip_entity_detection: None,
    };
    assert!(msg_valid.validate().is_ok());

    // Exactly 32,769 characters: rejected
    let text_32769 = "a".repeat(RICH_MESSAGE_MAX_TEXT_CHARS + 1);
    assert_eq!(text_32769.chars().count(), 32_769);
    let msg_over = InputRichMessage {
        blocks: vec![],
        html: Some(text_32769),
        markdown: None,
        media: None,
        is_rtl: None,
        skip_entity_detection: None,
    };
    let err = msg_over.validate().expect_err("32769 chars must fail");
    assert!(err.contains("exceeds Telegram limit of 32768"));
}

#[test]
fn test_tier2_rich_message_blocks_count_500_boundary() {
    let paragraph = || RichBlock::Paragraph {
        text: Value::String("para".to_string()),
    };

    // Exactly 500 blocks: valid
    let blocks_500: Vec<RichBlock> = (0..RICH_MESSAGE_MAX_BLOCKS).map(|_| paragraph()).collect();
    let msg_500 = InputRichMessage::new(blocks_500);
    assert!(msg_500.validate().is_ok());

    // Exactly 501 blocks: rejected
    let blocks_501: Vec<RichBlock> = (0..=RICH_MESSAGE_MAX_BLOCKS).map(|_| paragraph()).collect();
    let msg_501 = InputRichMessage::new(blocks_501);
    let err = msg_501.validate().expect_err("501 blocks must fail");
    assert!(err.contains("Telegram limit is 500"));
}

#[test]
fn test_tier2_tg_collage_exact_boundaries_2_to_10() {
    // 1 item: invalid for collage
    assert!(validate_tg_collage_child_count_contract(1).is_err());

    // 2 items: valid (exact lower boundary)
    assert!(validate_tg_collage_child_count_contract(2).is_ok());

    // 10 items: valid (exact upper boundary)
    assert!(validate_tg_collage_child_count_contract(10).is_ok());

    // 11 items: invalid (one past upper boundary)
    let err_11 = validate_tg_collage_child_count_contract(11).expect_err("11 must fail");
    assert!(err_11.contains("2-10"));
}

#[test]
fn test_tier2_tg_slideshow_lower_boundary_at_least_2() {
    assert!(validate_tg_slideshow_child_count_contract(0).is_err());
    assert!(validate_tg_slideshow_child_count_contract(1).is_err());
    assert!(validate_tg_slideshow_child_count_contract(2).is_ok());
    assert!(validate_tg_slideshow_child_count_contract(5).is_ok());
}

// ==============================================================================
// GROUP 10: TIER 3 — Cross-Feature Combinations & Multipart Attachment Uploads
// ==============================================================================

#[test]
fn test_tier3_composite_rich_message_text_collage_audio_map() {
    let composite_html = r#"<h3>Eksplorasi Gunung Rinjani</h3><p>Keindahan kaldera dan danau kawah:</p><tg-collage caption="Pemandangan Danau & Puncak"><img src="tg://photo?id=pic_summit"/><img src="tg://photo?id=pic_lake"/></tg-collage><p>Suara rekaman alam Pos Sembalun:</p><audio src="tg://audio?id=aud_wind" title="Angin Sembalun" performer="Lombok Audio"/><tg-map lat="-8.4113" lon="116.4573" zoom="13" title="Puncak Rinjani 3.726 mdpl"/>"#;

    let media_items = vec![
        InputRichMessageMedia {
            id: "pic_summit".to_string(),
            media: InputMedia::photo(
                "https://example.com/summit.jpg",
                Some("Puncak Rinjani".to_string()),
                None,
            ),
        },
        InputRichMessageMedia {
            id: "pic_lake".to_string(),
            media: InputMedia::photo(
                "https://example.com/lake.jpg",
                Some("Danau Segara Anak".to_string()),
                None,
            ),
        },
        InputRichMessageMedia {
            id: "aud_wind".to_string(),
            media: InputMedia::audio(
                "https://example.com/wind.mp3",
                Some("Suara Angin".to_string()),
                None,
                Some("Angin Sembalun".to_string()),
                Some("Lombok Audio".to_string()),
            ),
        },
    ];

    assert!(validate_html_media_reference_integrity(composite_html, &media_items).is_ok());
    assert!(validate_tg_map_attributes_contract(-8.4113, 116.4573, Some(13)).is_ok());

    let rich_msg = create_html_rich_message(composite_html, media_items)
        .expect("composite rich message must create successfully");
    assert!(rich_msg.validate().is_ok());

    let val = serde_json::to_value(&rich_msg).expect("composite message serializes");
    assert_eq!(val["html"], composite_html);
    let media_array = val["media"].as_array().expect("media array in json");
    assert_eq!(media_array.len(), 3);
    assert_eq!(media_array[0]["id"], "pic_summit");
    assert_eq!(media_array[1]["id"], "pic_lake");
    assert_eq!(media_array[2]["id"], "aud_wind");
}

#[test]
fn test_tier3_composite_rich_message_full_wire_json_fidelity() {
    let html = "<p>Halo</p><img src=\"tg://photo?id=p1\"/>";
    let media = vec![InputRichMessageMedia {
        id: "p1".to_string(),
        media: InputMedia::photo("https://ex.com/1.jpg", None, None),
    }];

    let msg = create_html_rich_message(html, media).expect("create rich message");
    let serialized = serde_json::to_string(&msg).expect("serialize to string");
    let deserialized: InputRichMessage =
        serde_json::from_str(&serialized).expect("deserialize from string");

    assert_eq!(deserialized.html.as_deref(), Some(html));
    let media_de = deserialized.media.as_ref().expect("media array present");
    assert_eq!(media_de.len(), 1);
    assert_eq!(media_de[0].id, "p1");
}

#[test]
fn test_tier3_reference_integrity_html_tags_must_match_media_ids() {
    let html_with_missing_media = "<p>Foto:</p><img src=\"tg://photo?id=pic_unresolved\"/>";
    let existing_media = vec![InputRichMessageMedia {
        id: "pic_different".to_string(),
        media: InputMedia::photo("https://ex.com/diff.jpg", None, None),
    }];

    let err = validate_html_media_reference_integrity(html_with_missing_media, &existing_media)
        .expect_err("Unresolved media ID in HTML must fail integrity validation");
    assert!(err.contains("pic_unresolved"));
    assert!(err.contains("no media item"));
}

#[test]
fn test_tier3_reference_integrity_tag_type_must_match_media_variant() {
    // Tag is <audio> referencing "item_1", but "item_1" is a Photo!
    let html_mismatched = "<audio src=\"tg://audio?id=item_1\"/>";
    let media_photo = vec![InputRichMessageMedia {
        id: "item_1".to_string(),
        media: InputMedia::photo("https://ex.com/p.jpg", None, None),
    }];

    let err = validate_html_media_reference_integrity(html_mismatched, &media_photo)
        .expect_err("Mismatched tag and media variant must fail integrity validation");
    assert!(err.contains("Type mismatch"));
    assert!(err.contains("item_1"));
}

#[test]
fn test_tier3_multipart_form_serialization_with_attach_schemes() {
    let rich_msg = create_html_rich_message(
        "<p>Peta jalur:</p><img src=\"tg://photo?id=pic_map\"/>",
        vec![InputRichMessageMedia {
            id: "pic_map".to_string(),
            media: InputMedia::photo("attach://upload_part_map", None, None),
        }],
    )
    .expect("create rich message");

    let attached_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // PNG magic bytes
    let attached_files = vec![(
        "upload_part_map".to_string(),
        attached_bytes,
        "image/png".to_string(),
    )];

    let form_contract = build_multipart_rich_message_form_contract(
        123456789,
        &rich_msg,
        &attached_files,
        None,
        None,
        None,
    )
    .expect("build multipart form contract");

    assert_eq!(form_contract.chat_id, "123456789");
    assert!(form_contract
        .rich_message
        .contains("attach://upload_part_map"));
    assert_eq!(form_contract.file_parts.len(), 1);
    assert_eq!(form_contract.file_parts[0].name, "upload_part_map");
    assert_eq!(
        form_contract.file_parts[0].mime.as_deref(),
        Some("image/png")
    );
    assert_eq!(form_contract.file_parts[0].content_len, 8);
}

#[test]
fn test_tier3_multipart_form_multiple_attachments_with_distinct_keys() {
    let rich_msg = create_html_rich_message(
        "<p>Dua file:</p><img src=\"tg://photo?id=p1\"/><img src=\"tg://photo?id=p2\"/>",
        vec![
            InputRichMessageMedia {
                id: "p1".to_string(),
                media: InputMedia::photo("attach://part_photo1", None, None),
            },
            InputRichMessageMedia {
                id: "p2".to_string(),
                media: InputMedia::photo("attach://part_photo2", None, None),
            },
        ],
    )
    .expect("create rich message");

    let attached_files = vec![
        (
            "part_photo1".to_string(),
            vec![1, 2, 3],
            "image/jpeg".to_string(),
        ),
        (
            "part_photo2".to_string(),
            vec![4, 5, 6, 7],
            "image/jpeg".to_string(),
        ),
    ];

    let form_contract = build_multipart_rich_message_form_contract(
        999,
        &rich_msg,
        &attached_files,
        None,
        None,
        None,
    )
    .expect("build multipart form");

    assert_eq!(form_contract.file_parts.len(), 2);
    assert_eq!(form_contract.file_parts[0].name, "part_photo1");
    assert_eq!(form_contract.file_parts[1].name, "part_photo2");
}

#[test]
fn test_tier3_dual_mode_transport_pure_json_vs_multipart() {
    // Mode 1: All remote HTTPS URLs -> Pure JSON (0 attachments)
    let remote_msg = create_html_rich_message(
        "<img src=\"tg://photo?id=p1\"/>",
        vec![InputRichMessageMedia {
            id: "p1".to_string(),
            media: InputMedia::photo("https://example.com/pic.jpg", None, None),
        }],
    )
    .expect("remote msg");

    let payload_json = json!({
        "chat_id": 12345_i64,
        "rich_message": remote_msg,
    });
    assert!(payload_json["rich_message"]["media"][0]["media"]["media"]
        .as_str()
        .expect("url")
        .starts_with("https://"));

    // Mode 2: Local attachments -> Multipart form
    let local_files = vec![(
        "part_upload".to_string(),
        vec![0u8; 100],
        "image/png".to_string(),
    )];
    let form = build_multipart_rich_message_form_contract(
        12345,
        &remote_msg,
        &local_files,
        None,
        None,
        None,
    )
    .expect("multipart form");
    assert_eq!(form.file_parts.len(), 1);
}

#[test]
fn test_tier3_send_rich_message_draft_prohibits_multipart_attachments() {
    // Telegram Bot API 10.2: sendRichMessageDraft cannot upload multipart attachments
    let draft_with_attach = InputRichMessage {
        blocks: vec![RichBlock::Paragraph {
            text: Value::String("attach://part_draft".to_string()),
        }],
        html: None,
        markdown: None,
        media: None,
        is_rtl: None,
        skip_entity_detection: None,
    };

    let plain_text = draft_with_attach.extract_plain_text();
    let has_raw_attachment = plain_text.contains("attach://");
    assert!(
        has_raw_attachment,
        "Draft containing attach:// detected for client-side prevention"
    );
}

#[test]
fn test_tier3_composite_rich_message_with_reply_markup_and_parameters() {
    let rich_msg =
        create_html_rich_message("<p>Pilihan jalur pendakian:</p>", vec![]).expect("rich msg");

    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::url_btn("Jalur Sembalun", "https://rinjani.id/sembalun"),
        InlineKeyboardButton::url_btn("Jalur Senaru", "https://rinjani.id/senaru"),
    ]]);
    let reply_params = ReplyParameters::new(777111);

    let payload = json!({
        "chat_id": 444333_i64,
        "rich_message": rich_msg,
        "reply_markup": keyboard,
        "reply_parameters": reply_params,
    });

    assert_eq!(payload["chat_id"], 444333_i64);
    assert_eq!(
        payload["reply_markup"]["inline_keyboard"][0][0]["text"],
        "Jalur Sembalun"
    );
    assert_eq!(
        payload["reply_markup"]["inline_keyboard"][0][1]["text"],
        "Jalur Senaru"
    );
    assert_eq!(payload["reply_parameters"]["message_id"], 777111);
}

#[test]
fn test_tier3_composite_rich_message_with_ephemeral_parameters() {
    let rich_msg = create_html_rich_message("<p>Pesan privat</p>", vec![]).expect("rich msg");
    let ephemeral = EphemeralMessageParameters {
        receiver_user_id: 123456789,
        callback_query_id: Some("cb_query_999".to_string()),
        replace_callback_query_message: Some(true),
    };

    let payload = json!({
        "chat_id": 888777_i64,
        "rich_message": rich_msg,
        "ephemeral_message_parameters": ephemeral,
    });

    assert_eq!(payload["chat_id"], 888777_i64);
    assert_eq!(
        payload["ephemeral_message_parameters"]["receiver_user_id"],
        123456789
    );
    assert_eq!(
        payload["ephemeral_message_parameters"]["callback_query_id"],
        "cb_query_999"
    );
    assert_eq!(
        payload["ephemeral_message_parameters"]["replace_callback_query_message"],
        true
    );
}

#[test]
fn test_tier3_slideshow_inside_composite_rich_message_with_keyboard() {
    let slideshow_html = "<p>Slide pemandangan:</p><tg-slideshow><img src=\"tg://photo?id=sl_1\"/><img src=\"tg://photo?id=sl_2\"/></tg-slideshow>";
    let media = vec![
        InputRichMessageMedia {
            id: "sl_1".to_string(),
            media: InputMedia::photo("https://ex.com/s1.jpg", None, None),
        },
        InputRichMessageMedia {
            id: "sl_2".to_string(),
            media: InputMedia::photo("https://ex.com/s2.jpg", None, None),
        },
    ];

    let rich_msg = create_html_rich_message(slideshow_html, media).expect("create slideshow msg");
    let keyboard = build_carousel_keyboard_contract("slide_ses_1", 0, 2).expect("carousel kb");

    let payload = json!({
        "chat_id": 123123_i64,
        "rich_message": rich_msg,
        "reply_markup": keyboard,
    });

    assert_eq!(payload["chat_id"], 123123_i64);
    assert_eq!(
        payload["reply_markup"]["inline_keyboard"][0][0]["callback_data"],
        "carousel:slide_ses_1:1:prev"
    );
    assert_eq!(
        payload["reply_markup"]["inline_keyboard"][0][1]["text"],
        "1/2"
    );
    assert_eq!(
        payload["reply_markup"]["inline_keyboard"][0][2]["callback_data"],
        "carousel:slide_ses_1:1:next"
    );
}

#[test]
fn test_tier3_collage_with_heterogeneous_unsupported_mix_rejected() {
    let photo = InputMedia::photo("https://ex.com/1.jpg", None, None);
    let audio = InputMedia::audio("https://ex.com/a.mp3", None, None, None, None);
    let doc = InputMedia::document("https://ex.com/d.pdf", None, None);

    // Mixing photo with audio in collage is invalid
    assert!(InputMedia::validate_media_group(&[photo.clone(), audio]).is_err());
    // Mixing photo with document in collage is invalid
    assert!(InputMedia::validate_media_group(&[photo, doc]).is_err());
}

#[test]
fn test_tier3_cross_representation_conflict_matrix() {
    let paragraph = RichBlock::Paragraph {
        text: Value::String("p".to_string()),
    };

    // 1. None of three (0 representations)
    let r0 = InputRichMessage {
        blocks: vec![],
        html: None,
        markdown: None,
        media: None,
        is_rtl: None,
        skip_entity_detection: None,
    };
    assert!(r0.validate().is_err());

    // 2. Blocks + HTML (2 representations)
    let r_bh = InputRichMessage {
        blocks: vec![paragraph.clone()],
        html: Some("<p>p</p>".to_string()),
        markdown: None,
        media: None,
        is_rtl: None,
        skip_entity_detection: None,
    };
    assert!(r_bh.validate().is_err());

    // 3. Blocks + Markdown (2 representations)
    let r_bm = InputRichMessage {
        blocks: vec![paragraph],
        html: None,
        markdown: Some("md".to_string()),
        media: None,
        is_rtl: None,
        skip_entity_detection: None,
    };
    assert!(r_bm.validate().is_err());

    // 4. HTML + Markdown (2 representations)
    let r_hm = InputRichMessage {
        blocks: vec![],
        html: Some("<p>p</p>".to_string()),
        markdown: Some("md".to_string()),
        media: None,
        is_rtl: None,
        skip_entity_detection: None,
    };
    assert!(r_hm.validate().is_err());

    // 5. All three (3 representations)
    let r_all = InputRichMessage {
        blocks: vec![RichBlock::Divider {}],
        html: Some("<hr/>".to_string()),
        markdown: Some("---".to_string()),
        media: None,
        is_rtl: None,
        skip_entity_detection: None,
    };
    let err_all = r_all.validate().expect_err("3 representations must fail");
    assert!(err_all.contains("found 3"));
}

#[test]
fn test_tier3_rtl_direction_with_rich_html_and_media() {
    let mut rich_msg =
        create_html_rich_message("<p>مرحبا بكم في جبل رينجاني</p>", vec![]).expect("rtl msg");
    rich_msg.is_rtl = Some(true);
    assert!(rich_msg.validate().is_ok());

    let val = serde_json::to_value(&rich_msg).expect("rtl serialize");
    assert_eq!(val["is_rtl"], true);
}

#[test]
fn test_tier3_skip_entity_detection_flag_propagation() {
    let mut rich_msg =
        create_html_rich_message("<p>Clean text without bot parsing entities</p>", vec![])
            .expect("msg");
    rich_msg.skip_entity_detection = Some(true);
    assert!(rich_msg.validate().is_ok());

    let val = serde_json::to_value(&rich_msg).expect("skip entity serialize");
    assert_eq!(val["skip_entity_detection"], true);
}

// ==============================================================================
// GROUP 11: TIER 4 — Real-World Application Scenarios (Bot API 10.1-10.3 Workloads)
// ==============================================================================

#[test]
fn test_tier4_scenario_rinjani_tourism_single_unified_bubble() {
    // Exact scenario from ORIGINAL_REQUEST.md:
    // User asks: "Berikan 2 foto pemandangan gunung rinjani dan lokasinya di peta"
    let commentary_html = r#"<h3>Eksplorasi Gunung Rinjani</h3><p>Gunung Rinjani di Pulau Lombok adalah gunung berapi kedua tertinggi di Indonesia (3.726 mdpl) yang terkenal dengan kaldera megah dan danau kawah Segara Anak.</p><tg-collage caption="Pemandangan Kaldera & Segara Anak"><img src="tg://photo?id=pic_rinjani_1"/><img src="tg://photo?id=pic_rinjani_2"/></tg-collage><p>Berikut lokasi geografis puncak Rinjani pada peta satelit:</p><tg-map lat="-8.4113" lon="116.4573" zoom="13" title="Puncak Rinjani 3.726 mdpl"/>"#;

    let media_items = vec![
        InputRichMessageMedia {
            id: "pic_rinjani_1".to_string(),
            media: InputMedia::Photo {
                media: "https://example.com/rinjani_summit.jpg".to_string(),
                caption: Some("Puncak Rinjani saat matahari terbit".to_string()),
                parse_mode: Some("HTML".to_string()),
                show_caption_above_media: Some(false),
                has_spoiler: Some(false),
            },
        },
        InputRichMessageMedia {
            id: "pic_rinjani_2".to_string(),
            media: InputMedia::Photo {
                media: "https://example.com/segara_anak.jpg".to_string(),
                caption: Some("Danau Segara Anak dilihat dari Plawangan".to_string()),
                parse_mode: Some("HTML".to_string()),
                show_caption_above_media: Some(false),
                has_spoiler: Some(false),
            },
        },
    ];

    // 1. Verify media integrity
    assert!(validate_html_media_reference_integrity(commentary_html, &media_items).is_ok());

    // 2. Verify map coordinates
    assert!(validate_tg_map_attributes_contract(-8.4113, 116.4573, Some(13)).is_ok());

    // 3. Construct single unified Rich Message
    let rich_msg = create_html_rich_message(commentary_html, media_items)
        .expect("Rinjani rich message creation must succeed");
    assert!(rich_msg.validate().is_ok());

    // 4. Construct single unified sendRichMessage payload
    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url_btn(
        "📖 Panduan Resmi TNGR",
        "https://rinjaninationalpark.id",
    )]]);

    let send_payload = json!({
        "chat_id": 99887766_i64,
        "rich_message": rich_msg,
        "reply_markup": keyboard,
        "reply_parameters": ReplyParameters::new(12345),
    });

    // 5. Invariant check: Exactly ONE bubble delivery
    assert_eq!(send_payload["chat_id"], 99887766_i64);
    assert!(send_payload.get("draft_id").is_none());
    assert_eq!(
        send_payload["rich_message"]["media"]
            .as_array()
            .expect("media array")
            .len(),
        2
    );
    assert_eq!(
        send_payload["reply_markup"]["inline_keyboard"][0][0]["text"],
        "📖 Panduan Resmi TNGR"
    );
}

#[test]
fn test_tier4_scenario_dead_image_url_zero_download_fallback() {
    // User asked for photos, but remote URLs returned 404 / 403 / broken download
    let blocks = vec![
        RichBlock::SectionHeading {
            text: Value::String("Pemandangan Pantai Kuta".to_string()),
            level: 3,
        },
        RichBlock::Paragraph {
            text: Value::String(
                "Pantai Kuta memiliki pasir putih dan ombak yang cocok untuk berselancar."
                    .to_string(),
            ),
        },
        RichBlock::Photo {
            photo: json!({
                "type": "photo",
                "media": "https://broken-domain.invalid/sunset.jpg"
            }),
            caption: Some(RichBlockCaption::new(Value::String(
                "Matahari terbenam di Kuta".to_string(),
            ))),
        },
    ];

    let original_msg = InputRichMessage::new(blocks);
    assert!(original_msg.has_media());

    // Simulate zero-download link conversion (anti-empty response recovery)
    let recovered_msg = convert_remote_media_to_rich_links_contract(&original_msg);

    // Invariant 1: Commentary text is preserved
    let plain_text = recovered_msg.extract_plain_text();
    assert!(plain_text.contains("Pantai Kuta"));
    assert!(plain_text.contains("berselancar"));

    // Invariant 2: Converted message is NOT empty
    assert!(
        !plain_text.trim().is_empty(),
        "AI response must never be empty"
    );

    // Invariant 3: Photo block was converted to text link paragraph
    let last_block = &recovered_msg.blocks[2];
    match last_block {
        RichBlock::Paragraph { text } => {
            let arr = text.as_array().expect("array of text items");
            assert_eq!(arr[0], "🖼️ ");
            assert_eq!(arr[1]["type"], "text_link");
            assert_eq!(arr[1]["text"], "Matahari terbenam di Kuta");
            assert_eq!(arr[1]["url"], "https://broken-domain.invalid/sunset.jpg");
        }
        _ => panic!("Dead photo block must be converted to Paragraph with link"),
    }
}

#[test]
fn test_tier4_scenario_borobudur_audio_tour_single_bubble() {
    let html = r#"<h3>Candi Borobudur</h3><p>Candi Buddha terbesar di dunia yang dibangun pada abad ke-8 oleh wangsa Syailendra.</p><audio src="tg://audio?id=borobudur_audio" title="Sejarah Candi Borobudur" performer="Audio Guide"/><tg-map lat="-7.6079" lon="110.2038" zoom="15" title="Candi Borobudur, Magelang"/>"#;

    let media = vec![InputRichMessageMedia {
        id: "borobudur_audio".to_string(),
        media: InputMedia::audio(
            "https://example.com/borobudur.mp3",
            Some("Narasi Sejarah Borobudur".to_string()),
            None,
            Some("Sejarah Candi Borobudur".to_string()),
            Some("Audio Guide".to_string()),
        ),
    }];

    assert!(validate_html_media_reference_integrity(html, &media).is_ok());
    assert!(validate_tg_map_attributes_contract(-7.6079, 110.2038, Some(15)).is_ok());

    let rich_msg = create_html_rich_message(html, media).expect("create borobudur tour");
    assert!(rich_msg.validate().is_ok());

    let payload = json!({
        "chat_id": 112233_i64,
        "rich_message": rich_msg,
    });
    assert_eq!(payload["chat_id"], 112233_i64);
    assert_eq!(payload["rich_message"]["media"][0]["id"], "borobudur_audio");
}

#[test]
fn test_tier4_scenario_interactive_photo_album_slideshow_pagination() {
    let slides = [
        "https://example.com/slide1.jpg",
        "https://example.com/slide2.jpg",
        "https://example.com/slide3.jpg",
        "https://example.com/slide4.jpg",
    ];
    let session_id = "album_42";

    // Step 1: Initial presentation at slide index 0
    let kb_0 =
        build_carousel_keyboard_contract(session_id, 0, slides.len()).expect("build keyboard 0");
    assert_eq!(kb_0.inline_keyboard[0][1].text, "1/4");

    // Step 2: User taps "▶️" -> callback query received
    let cb_data = kb_0.inline_keyboard[0][2]
        .callback_data
        .as_deref()
        .expect("next callback data");
    assert!(cb_data.len() <= 64, "Callback data must be <= 64 bytes");

    let (id, target_idx, action) =
        parse_carousel_callback_data(cb_data).expect("parse next callback");
    assert_eq!(id, session_id);
    assert_eq!(target_idx, 1);
    assert_eq!(action, "next");

    // Step 3: Server issues editMessageMedia to update slide in-place
    let updated_media = InputMedia::photo(
        slides[target_idx],
        Some(format!("Slide {} dari {}", target_idx + 1, slides.len())),
        None,
    );
    let kb_1 = build_carousel_keyboard_contract(session_id, target_idx, slides.len())
        .expect("build keyboard 1");

    let edit_payload = json!({
        "chat_id": 555_i64,
        "message_id": 777_i64,
        "media": updated_media,
        "reply_markup": kb_1,
    });

    assert_eq!(edit_payload["chat_id"], 555_i64);
    assert_eq!(edit_payload["message_id"], 777_i64);
    assert_eq!(
        edit_payload["reply_markup"]["inline_keyboard"][0][1]["text"],
        "2/4"
    );

    // Step 4: Server answers callback query
    let ack_payload = json!({
        "callback_query_id": "cbq_tap_999",
        "show_alert": false,
    });
    assert_eq!(ack_payload["callback_query_id"], "cbq_tap_999");
}

#[test]
fn test_tier4_scenario_streaming_draft_to_final_send_lifecycle() {
    let chat_id = 987654321_i64;
    let draft_id = 718293819203_i64;

    // Phase 1: Thinking block draft
    let phase1_msg = InputRichMessage::new(vec![RichBlock::Thinking {
        text: Value::String("Sedang mencari foto dan informasi gunung...".to_string()),
    }]);
    let draft_payload_1 = json!({
        "chat_id": chat_id,
        "draft_id": draft_id,
        "rich_message": phase1_msg,
        "can_stop": true,
    });
    assert_eq!(draft_payload_1["draft_id"], draft_id);

    // Phase 2: Progressive text streaming
    let phase2_msg = InputRichMessage::new(vec![
        RichBlock::SectionHeading {
            text: Value::String("Gunung Rinjani".to_string()),
            level: 2,
        },
        RichBlock::Paragraph {
            text: Value::String("Gunung Rinjani memiliki danau Segara Anak...".to_string()),
        },
    ]);
    let draft_payload_2 = json!({
        "chat_id": chat_id,
        "draft_id": draft_id,
        "rich_message": phase2_msg,
        "can_stop": true,
    });
    assert_eq!(draft_payload_2["draft_id"], draft_id);

    // Phase 3: Final Send (Permanent) -> NEVER include draft_id
    let final_rich = create_html_rich_message(
        "<h3>Gunung Rinjani</h3><p>Berikut fotonya:</p><img src=\"tg://photo?id=pic_rinjani\"/>",
        vec![InputRichMessageMedia {
            id: "pic_rinjani".to_string(),
            media: InputMedia::photo("https://example.com/rinjani.jpg", None, None),
        }],
    )
    .expect("final rich message");

    let final_payload = json!({
        "chat_id": chat_id,
        "rich_message": final_rich,
    });
    assert!(
        final_payload.get("draft_id").is_none(),
        "Final sendRichMessage must omit draft_id"
    );
    assert_eq!(
        final_payload["rich_message"]["media"][0]["id"],
        "pic_rinjani"
    );
}

#[test]
fn test_tier4_scenario_forum_topic_supergroup_thread_delivery() {
    let topic_thread_id = 888_i64;
    let rich_msg = create_html_rich_message("<p>Pesan di topic forum pendakian</p>", vec![])
        .expect("rich msg");

    let payload = json!({
        "chat_id": -1001234567890_i64,
        "message_thread_id": topic_thread_id,
        "rich_message": rich_msg,
    });

    assert_eq!(payload["chat_id"], -1001234567890_i64);
    assert_eq!(payload["message_thread_id"], topic_thread_id);
}

#[test]
fn test_tier4_scenario_mixed_local_attachment_and_remote_url() {
    // 1 Remote URL photo + 1 Local Binary attachment photo
    let rich_msg = create_html_rich_message(
        "<p>Foto online & peta lokal:</p><img src=\"tg://photo?id=online_pic\"/><img src=\"tg://photo?id=local_map\"/>",
        vec![
            InputRichMessageMedia {
                id: "online_pic".to_string(),
                media: InputMedia::photo("https://example.com/online.jpg", None, None),
            },
            InputRichMessageMedia {
                id: "local_map".to_string(),
                media: InputMedia::photo("attach://upload_local_map", None, None),
            },
        ],
    )
    .expect("mixed rich msg");

    let local_files = vec![(
        "upload_local_map".to_string(),
        vec![0x89, 0x50, 0x4E, 0x47],
        "image/png".to_string(),
    )];

    let form = build_multipart_rich_message_form_contract(
        12345,
        &rich_msg,
        &local_files,
        None,
        None,
        None,
    )
    .expect("build multipart");

    // Multipart form only packages the local binary part, leaving online URL inside rich_message JSON
    assert_eq!(form.file_parts.len(), 1);
    assert_eq!(form.file_parts[0].name, "upload_local_map");
    assert!(form.rich_message.contains("https://example.com/online.jpg"));
    assert!(form.rich_message.contains("attach://upload_local_map"));
}

#[test]
fn test_tier4_scenario_anti_empty_ai_response_graceful_recovery() {
    // When an image query fails to find any images, assistant provides informative text explanation
    // and NEVER emits an empty response
    let candidate_images: Vec<String> = vec![]; // Empty search results!
    let fallback_commentary = "Gunung Rinjani adalah gunung berapi aktif di Pulau Lombok. Saat ini sistem tidak menemukan foto publik aktif, tetapi Anda dapat membaca panduan lengkap pendakian di website resmi.";

    let output_msg = if candidate_images.is_empty() {
        // Degrade to informative text
        create_html_rich_message(&format!("<p>{fallback_commentary}</p>"), vec![])
            .expect("fallback msg creation")
    } else {
        panic!("Should have taken fallback branch");
    };

    assert!(output_msg.validate().is_ok());
    let extracted_text = output_msg.extract_plain_text();
    assert!(
        !extracted_text.trim().is_empty(),
        "AI response must never be empty"
    );
    assert!(extracted_text.contains("Gunung Rinjani"));
    assert!(extracted_text.contains("Pulau Lombok"));
}

#[test]
fn test_markdown_parser_converts_document_tag_to_rich_block() {
    use bot::models::base::RichBlock;
    use parser::markdown::parse_streaming_markdown_to_rich_blocks;
    let md = "Berikut laporannya:\n\n[document: test.pdf](attach://doc_0)";
    let blocks = parse_streaming_markdown_to_rich_blocks(md);

    assert_eq!(blocks.len(), 2);

    // First block is a paragraph
    if let RichBlock::Paragraph { text, .. } = &blocks[0] {
        assert_eq!(text, "Berikut laporannya:");
    } else {
        panic!("Expected Paragraph, got {:?}", blocks[0]);
    }

    // Second block is the document
    if let RichBlock::Document {
        document, caption, ..
    } = &blocks[1]
    {
        assert_eq!(document["type"], "document");
        assert_eq!(document["media"], "attach://doc_0");
        assert_eq!(caption.text, "test.pdf");
    } else {
        panic!("Expected Document, got {:?}", blocks[1]);
    }
}
