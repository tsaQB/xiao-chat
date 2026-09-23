#![allow(dead_code)]

#[path = "../src/bot/models.rs"]
pub mod models;
pub mod bot {
    pub use crate::models;
}
#[path = "../src/parser.rs"]
mod parser;
#[path = "../src/ai/stream.rs"]
mod stream;

use models::{
    validate_quiz, InputMedia, InputPollOption, Poll, PollOption, RichBlock, RichMessageButton,
    RichTextButton, Update, QUIZ_MAX_EXPLANATION_CHARS, QUIZ_MAX_EXPLANATION_LINE_BREAKS,
    QUIZ_MAX_OPTIONS, QUIZ_MAX_OPTION_CHARS, QUIZ_MAX_QUESTION_CHARS, QUIZ_MIN_OPTIONS,
};
use stream::SseDecoder;

fn photo_media(id: &str) -> InputMedia {
    InputMedia::Photo {
        media: id.to_string(),
        caption: None,
        parse_mode: None,
        show_caption_above_media: None,
        has_spoiler: None,
    }
}

#[test]
fn voice_note_input_media_uses_bot_api_10_3_discriminator() {
    let media = InputMedia::VoiceNote {
        media: "file-id".to_string(),
        caption: None,
        parse_mode: None,
        duration: None,
    };
    let value = serde_json::to_value(media).expect("voice note should serialize");
    assert_eq!(value["type"], "voice_note");
}

#[test]
fn parsed_voice_note_uses_bot_api_10_3_nested_media_discriminator() {
    let blocks =
        parser::parse_markdown_to_rich_blocks("[voice: Rekaman](https://example.com/sample.ogg)");
    let Some(RichBlock::VoiceNote { voice_note, .. }) = blocks.first() else {
        panic!("expected parsed voice-note block");
    };
    assert_eq!(voice_note["type"], "voice_note");
}

#[test]
fn rich_text_button_includes_required_type_discriminator() {
    let rich_text = RichTextButton {
        button: RichMessageButton::callback("Retry", "retry"),
    };
    let value = serde_json::to_value(rich_text).expect("rich text button should serialize");
    assert_eq!(value["type"], "button");
    assert_eq!(value["button"]["callback_data"], "retry");
}

#[test]
fn malformed_sse_data_is_rejected_instead_of_silently_dropped() {
    let mut decoder = SseDecoder::default();
    let error = decoder
        .push(b"data: {not-json}\n\n")
        .expect_err("malformed SSE JSON must be surfaced as an error");
    assert!(error.contains("invalid JSON"), "unexpected error: {error}");
}

#[test]
fn media_group_requires_two_to_ten_album_compatible_items() {
    assert!(InputMedia::validate_media_group(&[photo_media("a")]).is_err());
    assert!(InputMedia::validate_media_group(&[photo_media("a"), photo_media("b")]).is_ok());
    assert!(InputMedia::validate_media_group(
        &(0..10)
            .map(|index| photo_media(&format!("p{index}")))
            .collect::<Vec<_>>()
    )
    .is_ok());
    assert!(InputMedia::validate_media_group(
        &(0..11)
            .map(|index| photo_media(&format!("p{index}")))
            .collect::<Vec<_>>()
    )
    .is_err());

    let animation = InputMedia::Animation {
        media: "anim".to_string(),
        caption: None,
        parse_mode: None,
        show_caption_above_media: None,
        width: None,
        height: None,
        duration: None,
        has_spoiler: None,
    };
    assert!(InputMedia::validate_media_group(&[photo_media("a"), animation]).is_err());

    let voice_note = InputMedia::VoiceNote {
        media: "voice".to_string(),
        caption: None,
        parse_mode: None,
        duration: None,
    };
    assert!(InputMedia::validate_media_group(&[photo_media("a"), voice_note]).is_err());
}

#[test]
fn media_group_keeps_audio_and_documents_in_homogeneous_albums() {
    let audio = InputMedia::audio("audio", None, None, None, None);
    let document = InputMedia::document("document", None, None);
    let video = InputMedia::video("video", None, None);

    assert!(InputMedia::validate_media_group(&[photo_media("photo"), video]).is_ok());
    assert!(InputMedia::validate_media_group(&[photo_media("photo"), audio.clone()]).is_err());
    assert!(InputMedia::validate_media_group(&[photo_media("photo"), document.clone()]).is_err());
    assert!(InputMedia::validate_media_group(&[audio.clone(), document.clone()]).is_err());
    assert!(InputMedia::validate_media_group(&[audio.clone(), audio]).is_ok());
    assert!(InputMedia::validate_media_group(&[document.clone(), document]).is_ok());
}

#[test]
fn permanent_send_paths_do_not_emit_draft_id_zero() {
    let source = include_str!("../src/bot/client.rs");
    assert!(
        !source.contains("\"draft_id\": 0"),
        "permanent wrapper sendMessage/sendRichMessage payloads must not include draft_id"
    );
}

#[test]
fn raw_transport_permanent_send_paths_do_not_emit_draft_id_zero() {
    let source = include_str!("../src/bot/client/raw.rs");
    assert!(
        !source.contains("\"draft_id\": 0"),
        "inner transport permanent sends must not retain the invalid draft_id sentinel"
    );
}

#[test]
fn raw_media_downloader_enforces_safe_outbound_url_policy() {
    let source = include_str!("../src/bot/client/raw.rs");
    let start = source
        .find("pub async fn download_media_bytes")
        .expect("raw downloader must exist");
    let tail = &source[start..];
    let end = tail
        .find("pub async fn send_photo(")
        .expect("send_photo should follow raw downloader");
    let body = &tail[..end];
    assert!(
        body.contains("resolve_download_url"),
        "external media fallback must resolve and validate outbound targets before requesting them"
    );
    assert!(
        body.contains("Policy::none()"),
        "external media fallback must disable redirects so a public URL cannot pivot to a private target"
    );
    assert!(
        body.contains(".no_proxy()"),
        "external media fallback must not let ambient proxies bypass DNS pinning"
    );
}

#[test]
fn parsed_expandable_blockquote_serializes_with_10_3_discriminator() {
    let blocks = parser::parse_markdown_to_rich_blocks("**> Catatan penting yang dapat dilipat");
    let Some(RichBlock::ExpandableBlockQuotation { .. }) = blocks.first() else {
        panic!("expected expandable blockquote");
    };
    let rich_message = models::InputRichMessage::new(blocks);
    assert!(rich_message.validate().is_ok());
    let value = serde_json::to_value(&rich_message).expect("should serialize rich message");
    assert_eq!(value["blocks"][0]["type"], "expandable_blockquote");
    assert_eq!(
        value["blocks"][0]["text"],
        "Catatan penting yang dapat dilipat"
    );
}

#[test]
fn parsed_rich_inline_spoiler_and_strikethrough_serialize() {
    let blocks = parser::parse_markdown_to_rich_blocks("Hasil: ||jawaban rahasia|| dan ~~coret~~");
    let rich_message = models::InputRichMessage::new(blocks);
    assert!(rich_message.validate().is_ok());
    let value = serde_json::to_value(&rich_message).expect("should serialize rich message");
    let serialized = value.to_string();
    assert!(serialized.contains(r#""type":"spoiler""#));
    assert!(serialized.contains(r#""type":"strikethrough""#));
}

#[test]
fn raw_client_exposes_explicit_delete_ephemeral_message() {
    let source = include_str!("../src/bot/client/raw.rs");
    assert!(
        source.contains("pub async fn delete_ephemeral_message("),
        "raw client must provide explicit delete_ephemeral_message API"
    );
    assert!(
        source.contains("\"deleteEphemeralMessage\""),
        "raw client must target Telegram deleteEphemeralMessage method"
    );
}

#[test]
fn parsed_collage_and_slideshow_serialize_with_10_3_discriminators() {
    let collage_block = RichBlock::Collage {
        blocks: vec![
            serde_json::json!({"type": "photo", "photo": {"type": "photo", "media": "https://example.com/p1.jpg"}}),
            serde_json::json!({"type": "photo", "photo": {"type": "photo", "media": "https://example.com/p2.jpg"}}),
        ],
        caption: Some(models::RichBlockCaption::new(serde_json::json!(
            "Galeri Foto"
        ))),
    };
    let slideshow_block = RichBlock::Slideshow {
        blocks: vec![
            serde_json::json!({"type": "photo", "photo": {"type": "photo", "media": "https://example.com/s1.jpg"}}),
            serde_json::json!({"type": "photo", "photo": {"type": "photo", "media": "https://example.com/s2.jpg"}}),
        ],
        caption: None,
    };
    let rich_message = models::InputRichMessage::new(vec![collage_block, slideshow_block]);
    assert!(rich_message.validate().is_ok());
    let value =
        serde_json::to_value(&rich_message).expect("should serialize collage and slideshow");
    assert_eq!(value["blocks"][0]["type"], "collage");
    assert_eq!(
        value["blocks"][0]["blocks"]
            .as_array()
            .expect("array of collage blocks")
            .len(),
        2
    );
    assert_eq!(value["blocks"][1]["type"], "slideshow");
    assert_eq!(
        value["blocks"][1]["blocks"]
            .as_array()
            .expect("array of slideshow blocks")
            .len(),
        2
    );
}

#[test]
fn telegram_command_registration_is_pure_zero_slash() {
    let source = include_str!("../src/bot/daemon.rs");
    let cmd_start = source
        .find("// Register Bot Commands")
        .expect("bot command registration block");
    let cmd_end = source[cmd_start..]
        .find("bot.set_my_commands")
        .map(|offset| cmd_start + offset)
        .expect("bot.set_my_commands call");
    let block = &source[cmd_start..cmd_end];
    assert!(block.contains("&empty_commands") || block.contains("&[]") || block.contains("vec![]"));
    assert!(!block.contains("\"start\""));
    assert!(!block.contains("\"menu\""));
    assert!(!block.contains("\"help\""));
    assert!(!block.contains("\"clear\""));
    assert!(!block.contains("\"image\""));
    assert!(!block.contains("\"session\""));
    assert!(!block.contains("\"context\""));
}

#[test]
fn chat_deserializes_is_forum_field() {
    let json_str = r#"{
        "id": -1001234567890,
        "type": "supergroup",
        "title": "Topic Workspace",
        "is_forum": true
    }"#;
    let chat: models::Chat =
        serde_json::from_str(json_str).expect("Chat with is_forum must deserialize");
    assert_eq!(chat.id, -1001234567890);
    assert_eq!(chat.is_forum, Some(true));
}

#[test]
fn chat_member_deserializes_and_validates_admin_status() {
    let admin_json = r#"{
        "status": "administrator",
        "user": {
            "id": 123456,
            "is_bot": true,
            "first_name": "xiao"
        }
    }"#;
    let admin: models::ChatMember =
        serde_json::from_str(admin_json).expect("admin ChatMember must deserialize");
    assert!(admin.is_admin_or_creator());
    assert!(admin.is_administrator());
    assert!(!admin.is_creator());

    let creator_json = r#"{
        "status": "creator",
        "user": {
            "id": 654321,
            "is_bot": false,
            "first_name": "owner"
        }
    }"#;
    let creator: models::ChatMember =
        serde_json::from_str(creator_json).expect("creator ChatMember must deserialize");
    assert!(creator.is_admin_or_creator());
    assert!(creator.is_creator());
    assert!(!creator.is_administrator());

    let member_json = r#"{
        "status": "member",
        "user": {
            "id": 999999,
            "is_bot": false,
            "first_name": "member"
        }
    }"#;
    let member: models::ChatMember =
        serde_json::from_str(member_json).expect("member ChatMember must deserialize");
    assert!(!member.is_admin_or_creator());
}

#[test]
fn input_rich_message_serializes_is_rtl_true_when_set() {
    let mut msg = models::InputRichMessage::new(vec![models::RichBlock::Paragraph {
        text: serde_json::Value::String("مرحبا".to_string()),
    }]);
    msg.is_rtl = Some(true);
    let val = serde_json::to_value(&msg).expect("rich message should serialize");
    assert_eq!(val["is_rtl"], true);
}

#[test]
fn input_rich_message_omits_is_rtl_when_none() {
    let msg = models::InputRichMessage::new(vec![models::RichBlock::Paragraph {
        text: serde_json::Value::String("Hello".to_string()),
    }]);
    let val = serde_json::to_value(&msg).expect("rich message should serialize");
    assert!(val.get("is_rtl").is_none());
}

#[test]
fn parser_emits_is_rtl_and_right_aligned_cells_for_arabic_table_with_hindi_numerals() {
    let table_md = "| الرقم | الاسم |\n| --- | --- |\n| ١ | أحمد |\n| ٢ | فاطمة |";
    let rich_msg = parser::build_full_rich_message(table_md, None);
    assert_eq!(rich_msg.is_rtl, Some(true));

    let Some(models::RichBlock::Table { cells, .. }) = rich_msg.blocks.first() else {
        panic!("expected rich table block");
    };

    assert_eq!(cells[0][0].align.as_deref(), Some("right"));
    assert_eq!(cells[0][1].align.as_deref(), Some("right"));
    assert_eq!(cells[1][0].align.as_deref(), Some("right"));
    assert_eq!(cells[1][1].align.as_deref(), Some("right"));

    let val = serde_json::to_value(&rich_msg).expect("serialization must succeed");
    assert_eq!(val["is_rtl"], true);
    assert_eq!(val["blocks"][0]["type"], "table");
    assert_eq!(val["blocks"][0]["cells"][0][0]["align"], "right");
}

#[test]
fn parser_honors_explicit_column_alignments_in_contract_wire_format() {
    let md = "| A | B | C |\n| :--- | :---: | ---: |\n| 1 | 2 | 3 |";
    let rich_msg = parser::build_full_rich_message(md, None);
    let val = serde_json::to_value(&rich_msg).expect("serialization must succeed");
    assert_eq!(val["blocks"][0]["cells"][0][0]["align"], "left");
    assert_eq!(val["blocks"][0]["cells"][0][1]["align"], "center");
    assert_eq!(val["blocks"][0]["cells"][0][2]["align"], "right");
    assert_eq!(val["blocks"][0]["cells"][1][0]["align"], "left");
    assert_eq!(val["blocks"][0]["cells"][1][1]["align"], "center");
    assert_eq!(val["blocks"][0]["cells"][1][2]["align"], "right");
}

#[test]
fn ephemeral_message_parameters_serializes_replace_callback_query_message() {
    let params = models::EphemeralMessageParameters {
        receiver_user_id: 123456,
        callback_query_id: Some("cq_1".to_string()),
        replace_callback_query_message: Some(true),
    };
    let val = serde_json::to_value(&params).expect("serialization must succeed");
    assert_eq!(val["receiver_user_id"], 123456);
    assert_eq!(val["callback_query_id"], "cq_1");
    assert_eq!(val["replace_callback_query_message"], true);

    let params_without = models::EphemeralMessageParameters {
        receiver_user_id: 123456,
        callback_query_id: None,
        replace_callback_query_message: None,
    };
    let val_without = serde_json::to_value(&params_without).expect("serialization must succeed");
    assert_eq!(val_without["receiver_user_id"], 123456);
    assert!(val_without.get("callback_query_id").is_none());
    assert!(val_without.get("replace_callback_query_message").is_none());
}

#[test]
fn input_poll_option_wire_format_serializes_and_deserializes() {
    let opt_plain = InputPollOption::new("Paris");
    let val_plain = serde_json::to_value(&opt_plain).expect("should serialize plain option");
    assert_eq!(val_plain["text"], "Paris");
    assert!(val_plain.get("text_parse_mode").is_none());
    assert!(val_plain.get("text_entities").is_none());

    let opt_html = InputPollOption::with_parse_mode("<b>Berlin</b>", "HTML");
    let val_html = serde_json::to_value(&opt_html).expect("should serialize html option");
    assert_eq!(val_html["text"], "<b>Berlin</b>");
    assert_eq!(val_html["text_parse_mode"], "HTML");

    let from_str: InputPollOption = "Rome".into();
    assert_eq!(from_str.text, "Rome");

    let from_string: InputPollOption = "Madrid".to_string().into();
    assert_eq!(from_string.text, "Madrid");

    let json_input = r#"{"text": "Tokyo"}"#;
    let opt_de: InputPollOption =
        serde_json::from_str(json_input).expect("should deserialize option");
    assert_eq!(opt_de.text, "Tokyo");
    assert_eq!(opt_de.text_parse_mode, None);

    let json_str_input = r#""Kyoto""#;
    let opt_from_str: InputPollOption =
        serde_json::from_str(json_str_input).expect("should deserialize plain string option");
    assert_eq!(opt_from_str.text, "Kyoto");
    assert_eq!(opt_from_str.text_parse_mode, None);
}

#[test]
fn quiz_character_limits_and_validation_rules() {
    let valid_options = vec![
        InputPollOption::new("Option A"),
        InputPollOption::new("Option B"),
    ];

    // 1. Happy path
    assert!(validate_quiz(
        "What is 2 + 2?",
        &valid_options,
        0,
        Some("Basic arithmetic")
    )
    .is_ok());

    // 2. Question boundaries (1..=300 chars)
    let q_300 = "a".repeat(QUIZ_MAX_QUESTION_CHARS);
    assert!(validate_quiz(&q_300, &valid_options, 0, None).is_ok());

    let q_301 = "a".repeat(QUIZ_MAX_QUESTION_CHARS + 1);
    let err_q = validate_quiz(&q_301, &valid_options, 0, None).expect_err("301 chars must fail");
    assert!(err_q.contains("300"));

    assert!(validate_quiz("", &valid_options, 0, None).is_err());
    assert!(validate_quiz("   ", &valid_options, 0, None).is_err());

    // 3. Option count boundaries (2..=10)
    let one_option = vec![InputPollOption::new("Only one")];
    let err_count_min =
        validate_quiz("Question?", &one_option, 0, None).expect_err("1 option must fail");
    assert!(err_count_min.contains(&QUIZ_MIN_OPTIONS.to_string()));

    let ten_options: Vec<InputPollOption> = (0..QUIZ_MAX_OPTIONS)
        .map(|i| InputPollOption::new(format!("Option {i}")))
        .collect();
    assert!(validate_quiz("Question?", &ten_options, 0, None).is_ok());

    let eleven_options: Vec<InputPollOption> = (0..=QUIZ_MAX_OPTIONS)
        .map(|i| InputPollOption::new(format!("Option {i}")))
        .collect();
    let err_count_max =
        validate_quiz("Question?", &eleven_options, 0, None).expect_err("11 options must fail");
    assert!(err_count_max.contains(&QUIZ_MAX_OPTIONS.to_string()));

    // 4. Option text character limits (1..=100 chars)
    let empty_text_option = vec![InputPollOption::new("Valid"), InputPollOption::new("   ")];
    assert!(validate_quiz("Question?", &empty_text_option, 0, None).is_err());

    let opt_100 = "x".repeat(QUIZ_MAX_OPTION_CHARS);
    let options_100 = vec![InputPollOption::new(opt_100), InputPollOption::new("B")];
    assert!(validate_quiz("Question?", &options_100, 0, None).is_ok());

    let opt_101 = "x".repeat(QUIZ_MAX_OPTION_CHARS + 1);
    let options_101 = vec![InputPollOption::new(opt_101), InputPollOption::new("B")];
    let err_opt_len =
        validate_quiz("Question?", &options_101, 0, None).expect_err("101 char option must fail");
    assert!(err_opt_len.contains(&QUIZ_MAX_OPTION_CHARS.to_string()));

    // 5. Correct option ID boundaries
    assert!(validate_quiz("Question?", &valid_options, -1, None).is_err());
    assert!(validate_quiz("Question?", &valid_options, 0, None).is_ok());
    assert!(validate_quiz("Question?", &valid_options, 1, None).is_ok());
    assert!(validate_quiz("Question?", &valid_options, 2, None).is_err());

    // 6. Explanation limits (<= 200 chars, <= 2 line breaks)
    assert_eq!(QUIZ_MAX_EXPLANATION_LINE_BREAKS, 2);
    let exp_200 = "e".repeat(QUIZ_MAX_EXPLANATION_CHARS);
    assert!(validate_quiz("Question?", &valid_options, 0, Some(&exp_200)).is_ok());

    let exp_201 = "e".repeat(QUIZ_MAX_EXPLANATION_CHARS + 1);
    let err_exp_len = validate_quiz("Question?", &valid_options, 0, Some(&exp_201))
        .expect_err("201 char explanation must fail");
    assert!(err_exp_len.contains(&QUIZ_MAX_EXPLANATION_CHARS.to_string()));

    let exp_2_breaks = "Line 1\nLine 2\nLine 3";
    assert!(validate_quiz("Question?", &valid_options, 0, Some(exp_2_breaks)).is_ok());

    let exp_3_breaks = "Line 1\nLine 2\nLine 3\nLine 4";
    let err_breaks = validate_quiz("Question?", &valid_options, 0, Some(exp_3_breaks))
        .expect_err("3 line breaks must fail");
    assert!(err_breaks.contains("line breaks"));

    let exp_3_cr_breaks = "Line 1\rLine 2\rLine 3\rLine 4";
    let err_cr_breaks = validate_quiz("Question?", &valid_options, 0, Some(exp_3_cr_breaks))
        .expect_err("3 CR line breaks must fail");
    assert!(err_cr_breaks.contains("line breaks"));

    // 7. Duplicate options rejection (Telegram requires options to be unique)
    let dup_options = vec![
        InputPollOption::new("Same Option"),
        InputPollOption::new("Same Option"),
    ];
    let err_dup = validate_quiz("Question?", &dup_options, 0, None)
        .expect_err("Duplicate option texts must fail");
    assert!(err_dup.contains("unique"));
}

#[test]
fn quiz_poll_and_poll_option_deserializes_telegram_wire_format() {
    let wire_json = r#"{
        "id": "5432109876",
        "question": "Berapakah hasil dari 2^10?",
        "options": [
            {"text": "512", "voter_count": 2},
            {"text": "1024", "voter_count": 8},
            {"text": "2048", "voter_count": 1}
        ],
        "total_voter_count": 11,
        "is_closed": false,
        "is_anonymous": false,
        "type": "quiz",
        "allows_multiple_answers": false,
        "correct_option_id": 1,
        "explanation": "2 pangkat 10 adalah 1024."
    }"#;

    let poll: Poll = serde_json::from_str(wire_json).expect("poll must deserialize");
    assert_eq!(poll.id, "5432109876");
    assert_eq!(poll.question, "Berapakah hasil dari 2^10?");
    assert_eq!(poll.poll_type, "quiz");
    assert_eq!(poll.total_voter_count, 11);
    assert!(!poll.is_closed);
    assert!(!poll.is_anonymous);
    assert!(!poll.allows_multiple_answers);
    assert_eq!(poll.correct_option_id, Some(1));
    assert_eq!(
        poll.explanation.as_deref(),
        Some("2 pangkat 10 adalah 1024.")
    );
    assert_eq!(poll.options.len(), 3);
    let opt_0: &PollOption = &poll.options[0];
    assert_eq!(opt_0.text, "512");
    assert_eq!(opt_0.voter_count, 2);
    assert_eq!(poll.options[1].text, "1024");
    assert_eq!(poll.options[1].voter_count, 8);
    assert_eq!(poll.options[2].text, "2048");
    assert_eq!(poll.options[2].voter_count, 1);
}

#[test]
fn update_wire_format_deserializes_poll_update() {
    let wire_update = r#"{
        "update_id": 999111,
        "poll": {
            "id": "poll_wire_1",
            "question": "Quiz in update?",
            "options": [
                {"text": "Yes", "voter_count": 5},
                {"text": "No", "voter_count": 0}
            ],
            "total_voter_count": 5,
            "type": "quiz"
        }
    }"#;
    let update: Update =
        serde_json::from_str(wire_update).expect("update with poll must deserialize");
    assert_eq!(update.update_id, 999111);
    let poll = update.poll.expect("poll must be present");
    assert_eq!(poll.id, "poll_wire_1");
    assert_eq!(poll.question, "Quiz in update?");
    assert_eq!(poll.options.len(), 2);
    // Verified default flags
    assert!(!poll.is_closed);
    assert!(!poll.is_anonymous);
    assert!(!poll.allows_multiple_answers);
}

#[test]
fn raw_and_main_client_expose_send_poll_with_delivery_context() {
    let raw_source = include_str!("../src/bot/client/raw.rs");
    assert!(
        raw_source.contains("pub async fn send_poll("),
        "raw::TelegramBotClient must expose pub async fn send_poll"
    );
    assert!(
        raw_source.contains("\"sendPoll\""),
        "raw::TelegramBotClient must post to sendPoll method"
    );
    assert!(
        raw_source.contains("Self::apply_delivery_context(&mut payload, true);"),
        "raw::TelegramBotClient::send_poll must apply delivery context"
    );

    let client_source = include_str!("../src/bot/client.rs");
    assert!(
        client_source.contains("pub async fn send_poll("),
        "TelegramBotClient must expose pub async fn send_poll"
    );
    assert!(
        client_source.contains("\"sendPoll\""),
        "TelegramBotClient must post to sendPoll method"
    );
    assert!(
        client_source.contains("Self::apply_delivery_context(&mut payload, true);"),
        "TelegramBotClient::send_poll must apply delivery context"
    );
}

#[test]
fn quiz_send_poll_payload_wire_format_conforms_to_bot_api() {
    let options = vec![
        InputPollOption::new("Option 1"),
        InputPollOption::new("Option 2"),
    ];
    let payload = serde_json::json!({
        "chat_id": 12345,
        "question": "What is the answer?",
        "options": options,
        "type": "quiz",
        "is_anonymous": false,
        "correct_option_id": 0,
        "explanation": "Because it is.",
        "reply_parameters": {
            "message_id": 999
        }
    });

    assert_eq!(payload["chat_id"], 12345);
    assert_eq!(payload["type"], "quiz");
    assert_eq!(payload["options"][0]["text"], "Option 1");
    assert_eq!(payload["options"][1]["text"], "Option 2");
    assert_eq!(payload["correct_option_id"], 0);
    assert_eq!(payload["explanation"], "Because it is.");
    assert_eq!(payload["reply_parameters"]["message_id"], 999);
}

#[test]
fn test_inline_interleaved_rich_media_placement_order() {
    // Tests Telegram Bot API in-line interleaved rich media placement (matching @richtextdemobot demo)
    // where media elements are interwoven between headings and paragraphs.
    let text = "# Media Demo\n\n\
                Paragraf pembuka observasi satwa liar.\n\n\
                <img src=\"https://example.com/tiger.jpg\" caption=\"Harimau Sumatera\"/>\n\n\
                Paragraf penjelasan lanjutan setelah foto harimau.\n\n\
                <audio src=\"https://example.com/roar.mp3\" title=\"Suara Auman\" performer=\"Satwa\"/>\n\n\
                Paragraf penutup berisi kesimpulan observasi.";

    let blocks = parser::parse_markdown_to_rich_blocks(text);
    assert_eq!(blocks.len(), 6);
    assert!(matches!(blocks[0], RichBlock::SectionHeading { .. }));
    assert!(matches!(blocks[1], RichBlock::Paragraph { .. }));
    assert!(matches!(blocks[2], RichBlock::Photo { .. }));
    assert!(matches!(blocks[3], RichBlock::Paragraph { .. }));
    assert!(matches!(blocks[4], RichBlock::Audio { .. }));
    assert!(matches!(blocks[5], RichBlock::Paragraph { .. }));

    // Verify wire format serialization retains order
    let rich_msg = models::InputRichMessage::new(blocks);
    assert!(rich_msg.validate().is_ok());
    let serialized = serde_json::to_value(&rich_msg).expect("serialize rich message");
    let json_blocks = serialized["blocks"].as_array().expect("blocks array");
    assert_eq!(json_blocks.len(), 6);
    assert_eq!(json_blocks[0]["type"], "heading");
    assert_eq!(json_blocks[1]["type"], "paragraph");
    assert_eq!(json_blocks[2]["type"], "photo");
    assert_eq!(json_blocks[3]["type"], "paragraph");
    assert_eq!(json_blocks[4]["type"], "audio");
    assert_eq!(json_blocks[5]["type"], "paragraph");
}
