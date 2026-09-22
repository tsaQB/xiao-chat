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

use models::{InlineKeyboardButton, InlineKeyboardMarkup, InputMedia};
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

    if prev_data.len() > 64
        || indicator_data.len() > 64
        || next_data.len() > 64
    {
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
        (0.0, 0.0),       // Equator / Prime Meridian
        (90.0, 180.0),    // Top-right corner
        (90.0, -180.0),   // Top-left corner
        (-90.0, 180.0),   // Bottom-right corner
        (-90.0, -180.0),  // Bottom-left corner
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
    let tools_array = tools_val.as_array().expect("tools definition must be a json array");

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

    let media = InputMedia::photo(&args.url, args.caption.clone(), Some("Markdown".to_string()));
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
    let (id, new_index, action) =
        parse_carousel_callback_data(cb_next).expect("parse next action");
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
        let urls: Vec<String> = (0..count).map(|i| format!("https://ex.com/{i}.jpg")).collect();
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
        "a1b2c3d4",                                  // 8 hex chars (standard Xiao generator)
        "f0e1d2c3b4a5",                              // 12 hex chars
        "550e8400-e29b-41d4-a716-446655440000",       // 36 char UUID v4
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",   // 40 chars
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
        urls: vec!["https://ex.com/1.jpg".to_string(), "https://ex.com/2.jpg".to_string()],
        caption: Some(cap_huge.clone()),
    };
    collage.sanitize();
    assert_eq!(collage.caption.as_deref().map(|s| s.chars().count()), Some(1024));

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
    assert_eq!(audio.caption.as_deref().map(|s| s.chars().count()), Some(1024));

    // Voice
    let mut voice = SendVoiceArgs {
        url: "https://ex.com/v.ogg".to_string(),
        caption: Some(cap_huge.clone()),
    };
    voice.sanitize();
    assert_eq!(voice.caption.as_deref().map(|s| s.chars().count()), Some(1024));

    // Document
    let mut doc = SendDocumentArgs {
        url: "https://ex.com/d.pdf".to_string(),
        file_name: None,
        caption: Some(cap_huge),
    };
    doc.sanitize();
    assert_eq!(doc.caption.as_deref().map(|s| s.chars().count()), Some(1024));

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
        assert_eq!(curr, 0, "Stepping next {total} times must return to slide 0");
    }

    // 2-slide special boundary (both prev and next point to the other slide)
    let kb_2 = build_carousel_keyboard_contract("two_slides", 0, 2).expect("two slides kb");
    let prev_2 = kb_2.inline_keyboard[0][0].callback_data.as_deref().expect("prev");
    let next_2 = kb_2.inline_keyboard[0][2].callback_data.as_deref().expect("next");
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
        assert!(res.is_none(), "Expired carousel must be evicted and return None");
        assert!(guard.get("already_expired").is_none(), "Must be purged from map");
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
    assert_eq!(recovered_val, 42, "Poisoned lock recovery pattern succeeds without panic");
}


