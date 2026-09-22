#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ==========================================
// Inline & Reply Keyboards
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizontal_accuracy: Option<f64>,
}

impl Location {
    pub fn new(latitude: f64, longitude: f64) -> Result<Self, String> {
        let loc = Self {
            latitude,
            longitude,
            horizontal_accuracy: None,
        };
        loc.validate()?;
        Ok(loc)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.latitude.is_finite() {
            return Err("Location latitude must be a finite number".to_string());
        }
        if !(-90.0..=90.0).contains(&self.latitude) {
            return Err(format!(
                "Location latitude must be between -90.0 and 90.0; found {}",
                self.latitude
            ));
        }
        if !self.longitude.is_finite() {
            return Err("Location longitude must be a finite number".to_string());
        }
        if !(-180.0..=180.0).contains(&self.longitude) {
            return Err(format!(
                "Location longitude must be between -180.0 and 180.0; found {}",
                self.longitude
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forward_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_write_access: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwitchInlineQueryChosenChat {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_user_chats: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_bot_chats: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_group_chats: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_channel_chats: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum InputMedia {
    #[serde(rename = "photo")]
    Photo {
        media: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parse_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        show_caption_above_media: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        has_spoiler: Option<bool>,
    },
    #[serde(rename = "video")]
    Video {
        media: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parse_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        show_caption_above_media: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        width: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        height: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        has_spoiler: Option<bool>,
    },
    #[serde(rename = "animation")]
    Animation {
        media: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parse_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        show_caption_above_media: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        width: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        height: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        has_spoiler: Option<bool>,
    },
    #[serde(rename = "audio")]
    Audio {
        media: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parse_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        performer: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    #[serde(rename = "document")]
    Document {
        media: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parse_mode: Option<String>,
    },
    #[serde(rename = "voice_note", alias = "voice")]
    VoiceNote {
        media: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parse_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration: Option<i32>,
    },
}

impl InputMedia {
    pub fn photo(
        media: impl Into<String>,
        caption: Option<String>,
        parse_mode: Option<String>,
    ) -> Self {
        InputMedia::Photo {
            media: media.into(),
            caption,
            parse_mode,
            show_caption_above_media: None,
            has_spoiler: None,
        }
    }

    pub fn video(
        media: impl Into<String>,
        caption: Option<String>,
        parse_mode: Option<String>,
    ) -> Self {
        InputMedia::Video {
            media: media.into(),
            caption,
            parse_mode,
            show_caption_above_media: None,
            width: None,
            height: None,
            duration: None,
            has_spoiler: None,
        }
    }

    pub fn audio(
        media: impl Into<String>,
        caption: Option<String>,
        parse_mode: Option<String>,
        title: Option<String>,
        performer: Option<String>,
    ) -> Self {
        InputMedia::Audio {
            media: media.into(),
            caption,
            parse_mode,
            duration: None,
            performer,
            title,
        }
    }

    pub fn document(
        media: impl Into<String>,
        caption: Option<String>,
        parse_mode: Option<String>,
    ) -> Self {
        InputMedia::Document {
            media: media.into(),
            caption,
            parse_mode,
        }
    }

    pub fn animation(
        media: impl Into<String>,
        caption: Option<String>,
        parse_mode: Option<String>,
    ) -> Self {
        InputMedia::Animation {
            media: media.into(),
            caption,
            parse_mode,
            show_caption_above_media: None,
            width: None,
            height: None,
            duration: None,
            has_spoiler: None,
        }
    }

    pub fn voice_note(
        media: impl Into<String>,
        caption: Option<String>,
        parse_mode: Option<String>,
        duration: Option<i32>,
    ) -> Self {
        InputMedia::VoiceNote {
            media: media.into(),
            caption,
            parse_mode,
            duration,
        }
    }

    pub fn validate_media_group(media: &[Self]) -> Result<(), String> {
        if !(2..=10).contains(&media.len()) {
            return Err(format!(
                "sendMediaGroup requires 2-10 media items; found {}",
                media.len()
            ));
        }
        let compatible = match &media[0] {
            Self::Audio { .. } => media.iter().all(|item| matches!(item, Self::Audio { .. })),
            Self::Document { .. } => media
                .iter()
                .all(|item| matches!(item, Self::Document { .. })),
            Self::Photo { .. } | Self::Video { .. } => media
                .iter()
                .all(|item| matches!(item, Self::Photo { .. } | Self::Video { .. })),
            Self::Animation { .. } | Self::VoiceNote { .. } => false,
        };
        compatible.then_some(()).ok_or_else(|| {
            "sendMediaGroup requires audio-only or document-only albums; photos and videos may be combined"
                .to_string()
        })
    }

    pub fn media_url(&self) -> &str {
        match self {
            Self::Photo { media, .. }
            | Self::Video { media, .. }
            | Self::Animation { media, .. }
            | Self::Audio { media, .. }
            | Self::Document { media, .. }
            | Self::VoiceNote { media, .. } => media.as_str(),
        }
    }

    pub fn set_media_url(&mut self, new_media: impl Into<String>) {
        let val = new_media.into();
        match self {
            Self::Photo { media, .. }
            | Self::Video { media, .. }
            | Self::Animation { media, .. }
            | Self::Audio { media, .. }
            | Self::Document { media, .. }
            | Self::VoiceNote { media, .. } => *media = val,
        }
    }

    pub fn caption_text(&self) -> Option<&str> {
        match self {
            Self::Photo { caption, .. }
            | Self::Video { caption, .. }
            | Self::Animation { caption, .. }
            | Self::Audio { caption, .. }
            | Self::Document { caption, .. }
            | Self::VoiceNote { caption, .. } => caption.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputRichMessageMedia {
    pub id: String,
    pub media: InputMedia,
}

impl InputRichMessageMedia {
    pub fn new(id: impl Into<String>, media: InputMedia) -> Result<Self, String> {
        let id = id.into();
        Self::validate_id(&id)?;
        Ok(Self { id, media })
    }

    pub fn validate_id(id: &str) -> Result<(), String> {
        let count = id.chars().count();
        if !(1..=64).contains(&count) {
            return Err(format!(
                "InputRichMessageMedia ID must be 1-64 characters, found {count}"
            ));
        }
        if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!(
                "InputRichMessageMedia ID must contain only ASCII alphanumeric characters or underscores; found '{id}'"
            ));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        Self::validate_id(&self.id)?;
        if self.media.media_url().trim().is_empty() {
            return Err(format!(
                "InputRichMessageMedia ID '{}' has empty media URL",
                self.id
            ));
        }
        Ok(())
    }

    pub fn photo(
        id: impl Into<String>,
        media: impl Into<String>,
        caption: Option<String>,
    ) -> Result<Self, String> {
        Self::new(id, InputMedia::photo(media, caption, None))
    }

    pub fn video(
        id: impl Into<String>,
        media: impl Into<String>,
        caption: Option<String>,
    ) -> Result<Self, String> {
        Self::new(id, InputMedia::video(media, caption, None))
    }

    pub fn audio(
        id: impl Into<String>,
        media: impl Into<String>,
        title: Option<String>,
        performer: Option<String>,
        caption: Option<String>,
    ) -> Result<Self, String> {
        Self::new(
            id,
            InputMedia::audio(media, caption, None, title, performer),
        )
    }

    pub fn document(
        id: impl Into<String>,
        media: impl Into<String>,
        caption: Option<String>,
    ) -> Result<Self, String> {
        Self::new(id, InputMedia::document(media, caption, None))
    }

    pub fn animation(
        id: impl Into<String>,
        media: impl Into<String>,
        caption: Option<String>,
    ) -> Result<Self, String> {
        Self::new(id, InputMedia::animation(media, caption, None))
    }

    pub fn voice_note(
        id: impl Into<String>,
        media: impl Into<String>,
        caption: Option<String>,
        duration: Option<i32>,
    ) -> Result<Self, String> {
        Self::new(id, InputMedia::voice_note(media, caption, None, duration))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CopyTextButton {
    pub text: String,
}

impl CopyTextButton {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineKeyboardButton {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_app: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy_text: Option<CopyTextButton>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_url: Option<LoginUrl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch_inline_query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch_inline_query_current_chat: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch_inline_query_chosen_chat: Option<SwitchInlineQueryChosenChat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<Value>,
}

impl InlineKeyboardButton {
    pub fn callback(text: impl Into<String>, callback_data: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: None,
            callback_data: Some(callback_data.into()),
            url: None,
            web_app: None,
            copy_text: None,
            login_url: None,
            switch_inline_query: None,
            switch_inline_query_current_chat: None,
            switch_inline_query_chosen_chat: None,
            disabled: None,
        }
    }

    pub fn callback_styled(
        text: impl Into<String>,
        callback_data: impl Into<String>,
        style: impl Into<String>,
    ) -> Self {
        let mut button = Self::callback(text, callback_data);
        button.style = Some(style.into());
        button
    }

    pub fn url_btn(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: None,
            callback_data: None,
            url: Some(url.into()),
            web_app: None,
            copy_text: None,
            login_url: None,
            switch_inline_query: None,
            switch_inline_query_current_chat: None,
            switch_inline_query_chosen_chat: None,
            disabled: None,
        }
    }

    pub fn copy(text: impl Into<String>, copy_text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: None,
            callback_data: None,
            url: None,
            web_app: None,
            copy_text: Some(CopyTextButton::new(copy_text)),
            login_url: None,
            switch_inline_query: None,
            switch_inline_query_current_chat: None,
            switch_inline_query_chosen_chat: None,
            disabled: None,
        }
    }

    pub fn disabled(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: None,
            callback_data: None,
            url: None,
            web_app: None,
            copy_text: None,
            login_url: None,
            switch_inline_query: None,
            switch_inline_query_current_chat: None,
            switch_inline_query_chosen_chat: None,
            disabled: Some(serde_json::json!({})),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineKeyboardMarkup {
    pub inline_keyboard: Vec<Vec<InlineKeyboardButton>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_reply: Option<bool>,
}

impl InlineKeyboardMarkup {
    pub fn new(rows: Vec<Vec<InlineKeyboardButton>>) -> Self {
        Self {
            inline_keyboard: rows,
            force_reply: None,
        }
    }

    pub fn with_force_reply(mut self, force_reply: bool) -> Self {
        self.force_reply = Some(force_reply);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardButton {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_contact: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_location: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_app: Option<Value>,
}

impl KeyboardButton {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            request_contact: None,
            request_location: None,
            web_app: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyKeyboardMarkup {
    pub keyboard: Vec<Vec<KeyboardButton>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_persistent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resize_keyboard: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_time_keyboard: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_field_placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selective: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_reply: Option<bool>,
}

impl ReplyKeyboardMarkup {
    pub fn from_strings(
        rows: Vec<Vec<&str>>,
        is_persistent: bool,
        resize_keyboard: bool,
        placeholder: Option<&str>,
    ) -> Self {
        let keyboard = rows
            .into_iter()
            .map(|row| row.into_iter().map(KeyboardButton::new).collect())
            .collect();

        Self {
            keyboard,
            is_persistent: Some(is_persistent),
            resize_keyboard: Some(resize_keyboard),
            one_time_keyboard: Some(false),
            input_field_placeholder: placeholder.map(|s| s.to_string()),
            selective: None,
            force_reply: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReplyKeyboardRemove {
    pub remove_keyboard: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selective: Option<bool>,
}

impl ReplyKeyboardRemove {
    pub fn new() -> Self {
        Self {
            remove_keyboard: true,
            selective: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotCommand {
    pub command: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_ephemeral: Option<bool>,
}

impl BotCommand {
    pub fn new(command: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            description: description.into(),
            is_ephemeral: None,
        }
    }

    pub fn ephemeral(command: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            description: description.into(),
            is_ephemeral: Some(true),
        }
    }
}

// ==========================================
// Telegram Bot API 10.3: Rich Message Blocks
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RichTextButton {
    pub button: RichMessageButton,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RichMessageButton {
    pub text: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_app: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy_text: Option<CopyTextButton>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_url: Option<LoginUrl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch_inline_query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch_inline_query_current_chat: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch_inline_query_chosen_chat: Option<SwitchInlineQueryChosenChat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<Value>,
}

impl RichMessageButton {
    pub fn callback(text: impl Into<String>, callback_data: impl Into<String>) -> Self {
        Self {
            text: Value::String(text.into()),
            style: None,
            url: None,
            callback_data: Some(callback_data.into()),
            web_app: None,
            copy_text: None,
            login_url: None,
            switch_inline_query: None,
            switch_inline_query_current_chat: None,
            switch_inline_query_chosen_chat: None,
            disabled: None,
        }
    }

    pub fn callback_styled(
        text: impl Into<String>,
        callback_data: impl Into<String>,
        style: impl Into<String>,
    ) -> Self {
        let mut button = Self::callback(text, callback_data);
        button.style = Some(style.into());
        button
    }

    pub fn url(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            text: Value::String(text.into()),
            style: None,
            url: Some(url.into()),
            callback_data: None,
            web_app: None,
            copy_text: None,
            login_url: None,
            switch_inline_query: None,
            switch_inline_query_current_chat: None,
            switch_inline_query_chosen_chat: None,
            disabled: None,
        }
    }

    pub fn copy(text: impl Into<String>, copy_text: impl Into<String>) -> Self {
        Self {
            text: Value::String(text.into()),
            style: None,
            url: None,
            callback_data: None,
            web_app: None,
            copy_text: Some(CopyTextButton::new(copy_text)),
            login_url: None,
            switch_inline_query: None,
            switch_inline_query_current_chat: None,
            switch_inline_query_chosen_chat: None,
            disabled: None,
        }
    }

    pub fn disabled(text: impl Into<String>) -> Self {
        Self {
            text: Value::String(text.into()),
            style: None,
            url: None,
            callback_data: None,
            web_app: None,
            copy_text: None,
            login_url: None,
            switch_inline_query: None,
            switch_inline_query_current_chat: None,
            switch_inline_query_chosen_chat: None,
            disabled: Some(serde_json::json!({})),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let action_count = usize::from(self.url.is_some())
            + usize::from(self.callback_data.is_some())
            + usize::from(self.web_app.is_some())
            + usize::from(self.copy_text.is_some())
            + usize::from(self.login_url.is_some())
            + usize::from(self.switch_inline_query.is_some())
            + usize::from(self.switch_inline_query_current_chat.is_some())
            + usize::from(self.switch_inline_query_chosen_chat.is_some())
            + usize::from(self.disabled.is_some());
        if action_count != 1 {
            return Err(format!(
                "RichMessageButton must contain exactly one action, found {action_count}"
            ));
        }
        if let Some(copy_button) = &self.copy_text {
            let len = copy_button.text.chars().count();
            if len == 0 || len > 256 {
                return Err(format!(
                    "RichMessageButton copy_text must contain 1-256 characters, found {len}"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RichBlockCaption {
    pub text: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<Value>,
}

impl RichBlockCaption {
    pub fn new(text: Value) -> Self {
        Self { text, credit: None }
    }

    pub fn with_credit(text: Value, credit: Value) -> Self {
        Self {
            text,
            credit: Some(credit),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RichBlockTableCell {
    pub text: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_header: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    pub valign: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colspan: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rowspan: Option<usize>,
}

impl RichBlockTableCell {
    pub fn new(text: Value, is_header: bool, align: Option<&str>) -> Self {
        Self {
            text,
            is_header: if is_header { Some(true) } else { None },
            align: align.map(|a| a.to_string()),
            valign: "middle".to_string(),
            colspan: None,
            rowspan: None,
        }
    }

    pub fn text_only(text: &str, is_header: bool, align: Option<&str>) -> Self {
        Self::new(Value::String(text.to_string()), is_header, align)
    }

    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        value_to_text(&self.text, &mut out);
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RichBlockListItem {
    pub blocks: Vec<Value>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_checkbox: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_checked: Option<bool>,
}

impl RichBlockListItem {
    pub fn bullet(blocks: Vec<Value>) -> Self {
        Self {
            blocks,
            kind: None,
            value: None,
            has_checkbox: None,
            is_checked: None,
        }
    }

    pub fn ordered(blocks: Vec<Value>, value: Option<i64>) -> Self {
        Self {
            blocks,
            kind: Some("1".to_string()),
            value,
            has_checkbox: None,
            is_checked: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum RichBlock {
    #[serde(rename = "paragraph")]
    Paragraph { text: Value },

    #[serde(rename = "heading")]
    SectionHeading {
        text: Value,
        #[serde(rename = "size")]
        level: usize,
    },

    #[serde(rename = "pre")]
    Preformatted {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },

    #[serde(rename = "footer")]
    Footer { text: Value },

    #[serde(rename = "list")]
    List { items: Vec<RichBlockListItem> },

    #[serde(rename = "blockquote")]
    BlockQuotation {
        blocks: Vec<Value>, // [{"type": "paragraph", "text": ...}]
    },

    #[serde(rename = "expandable_blockquote")]
    ExpandableBlockQuotation {
        text: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        credit: Option<Value>,
    },

    #[serde(rename = "pullquote")]
    PullQuotation {
        text: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        credit: Option<Value>,
    },

    #[serde(rename = "divider")]
    Divider {},

    #[serde(rename = "mathematical_expression")]
    MathematicalExpression { expression: String },

    #[serde(rename = "table")]
    Table {
        cells: Vec<Vec<RichBlockTableCell>>,
        #[serde(skip)]
        has_header: bool,
        is_bordered: bool,
        is_striped: bool,
        is_compact: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
    },

    #[serde(rename = "buttons")]
    Buttons {
        buttons: Vec<RichMessageButton>,
        #[serde(skip_serializing_if = "Option::is_none")]
        align: Option<String>,
    },

    #[serde(rename = "document")]
    Document {
        document: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<RichBlockCaption>,
    },

    #[serde(rename = "photo")]
    Photo {
        photo: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<RichBlockCaption>,
    },

    #[serde(rename = "video")]
    Video {
        video: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<RichBlockCaption>,
    },

    #[serde(rename = "audio")]
    Audio {
        audio: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<RichBlockCaption>,
    },

    #[serde(rename = "voice_note")]
    VoiceNote {
        voice_note: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<RichBlockCaption>,
    },

    #[serde(rename = "animation")]
    Animation {
        animation: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<RichBlockCaption>,
    },

    #[serde(rename = "collage")]
    Collage {
        blocks: Vec<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<RichBlockCaption>,
    },

    #[serde(rename = "slideshow")]
    Slideshow {
        blocks: Vec<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<RichBlockCaption>,
    },

    #[serde(rename = "map")]
    Map {
        location: Location,
        #[serde(skip_serializing_if = "Option::is_none")]
        zoom: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        width: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        height: Option<i32>,
    },

    #[serde(rename = "details")]
    Details {
        summary: Value,
        blocks: Vec<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_open: Option<bool>,
    },

    #[serde(rename = "anchor")]
    Anchor { name: String },

    #[serde(rename = "thinking")]
    Thinking { text: Value },
}

impl RichBlock {
    pub fn is_media(&self) -> bool {
        matches!(
            self,
            RichBlock::Photo { .. }
                | RichBlock::Video { .. }
                | RichBlock::Audio { .. }
                | RichBlock::VoiceNote { .. }
                | RichBlock::Animation { .. }
                | RichBlock::Collage { .. }
                | RichBlock::Slideshow { .. }
                | RichBlock::Map { .. }
                | RichBlock::Document { .. }
        )
    }

    pub fn caption_text(&self) -> Option<String> {
        let cap = match self {
            RichBlock::Photo { caption, .. }
            | RichBlock::Video { caption, .. }
            | RichBlock::Audio { caption, .. }
            | RichBlock::VoiceNote { caption, .. }
            | RichBlock::Animation { caption, .. }
            | RichBlock::Collage { caption, .. }
            | RichBlock::Slideshow { caption, .. }
            | RichBlock::Document { caption, .. } => caption.as_ref()?,
            _ => return None,
        };
        match &cap.text {
            Value::String(s) => Some(s.clone()),
            Value::Array(arr) => {
                let mut out = String::new();
                for item in arr {
                    if let Some(s) = item.as_str() {
                        out.push_str(s);
                    } else if let Some(s) = item.get("text").and_then(Value::as_str) {
                        out.push_str(s);
                    }
                }
                if out.is_empty() {
                    None
                } else {
                    Some(out)
                }
            }
            _ => None,
        }
    }

    pub fn get_media_urls(&self) -> Vec<String> {
        let extract = |val: &Value| -> Option<String> {
            val.get("media")
                .and_then(Value::as_str)
                .or_else(|| val.as_str())
                .map(|s| s.to_string())
        };

        match self {
            RichBlock::Photo { photo, .. } => extract(photo).into_iter().collect(),
            RichBlock::Video { video, .. } => extract(video).into_iter().collect(),
            RichBlock::Audio { audio, .. } => extract(audio).into_iter().collect(),
            RichBlock::VoiceNote { voice_note, .. } => extract(voice_note).into_iter().collect(),
            RichBlock::Animation { animation, .. } => extract(animation).into_iter().collect(),
            RichBlock::Document { document, .. } => extract(document).into_iter().collect(),
            RichBlock::Collage { blocks, .. } | RichBlock::Slideshow { blocks, .. } => {
                let mut urls = Vec::new();
                for item in blocks {
                    if let Some(p) = item.get("photo") {
                        if let Some(u) = extract(p) {
                            urls.push(u);
                        }
                    } else if let Some(v) = item.get("video") {
                        if let Some(u) = extract(v) {
                            urls.push(u);
                        }
                    } else if let Some(u) = extract(item) {
                        urls.push(u);
                    }
                }
                urls
            }
            _ => Vec::new(),
        }
    }

    pub fn replace_media_urls<F: Fn(&str) -> Option<String>>(&mut self, replacer: &F) {
        let mutate = |val: &mut Value| {
            if let Some(obj) = val.as_object_mut() {
                if let Some(m) = obj.get("media").and_then(Value::as_str) {
                    if let Some(new_val) = replacer(m) {
                        obj.insert("media".to_string(), Value::String(new_val));
                    }
                }
            } else if let Some(s) = val.as_str() {
                if let Some(new_val) = replacer(s) {
                    *val = Value::String(new_val);
                }
            }
        };

        match self {
            RichBlock::Photo { photo, .. } => mutate(photo),
            RichBlock::Video { video, .. } => mutate(video),
            RichBlock::Audio { audio, .. } => mutate(audio),
            RichBlock::VoiceNote { voice_note, .. } => mutate(voice_note),
            RichBlock::Animation { animation, .. } => mutate(animation),
            RichBlock::Document { document, .. } => mutate(document),
            RichBlock::Collage { blocks, .. } | RichBlock::Slideshow { blocks, .. } => {
                for item in blocks.iter_mut() {
                    if let Some(p) = item.get_mut("photo") {
                        mutate(p);
                    } else if let Some(v) = item.get_mut("video") {
                        mutate(v);
                    } else {
                        mutate(item);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn extract_text(&self) -> String {
        let mut out = String::new();
        match self {
            RichBlock::Paragraph { text }
            | RichBlock::SectionHeading { text, .. }
            | RichBlock::Thinking { text }
            | RichBlock::Footer { text } => {
                value_to_text(text, &mut out);
            }
            RichBlock::Preformatted { text, .. } => {
                out.push_str(text);
            }
            RichBlock::BlockQuotation { blocks } => {
                for b in blocks {
                    value_to_text(b, &mut out);
                    out.push('\n');
                }
            }
            RichBlock::ExpandableBlockQuotation { text, credit }
            | RichBlock::PullQuotation { text, credit } => {
                value_to_text(text, &mut out);
                if let Some(c) = credit {
                    out.push_str(" — ");
                    value_to_text(c, &mut out);
                }
            }
            RichBlock::MathematicalExpression { expression } => {
                out.push_str(expression);
            }
            RichBlock::List { items } => {
                for item in items {
                    for b in &item.blocks {
                        value_to_text(b, &mut out);
                        out.push('\n');
                    }
                }
            }
            RichBlock::Table { cells, caption, .. } => {
                if let Some(cap) = caption {
                    out.push_str(cap);
                    out.push('\n');
                }
                for row in cells {
                    for cell in row {
                        value_to_text(&cell.text, &mut out);
                        out.push(' ');
                    }
                    out.push('\n');
                }
            }
            _ => {
                if let Some(cap) = self.caption_text() {
                    out.push_str(&cap);
                }
            }
        }
        out.trim().to_string()
    }

    pub fn map(location: Location, zoom: Option<i32>) -> Result<Self, String> {
        location.validate()?;
        if let Some(z) = zoom {
            if !(1..=20).contains(&z) {
                return Err(format!("Map zoom must be between 1 and 20; found {z}"));
            }
        }
        Ok(RichBlock::Map {
            location,
            zoom,
            width: None,
            height: None,
        })
    }

    pub fn map_coords(latitude: f64, longitude: f64, zoom: Option<i32>) -> Result<Self, String> {
        let location = Location::new(latitude, longitude)?;
        Self::map(location, zoom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct InputRichMessage {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<RichBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<Vec<InputRichMessageMedia>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_rtl: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_entity_detection: Option<bool>,
}

pub const RICH_MESSAGE_MAX_TEXT_CHARS: usize = 32_768;
pub const RICH_MESSAGE_MAX_BLOCKS: usize = 500;
pub const RICH_MESSAGE_MAX_NESTING: usize = 16;
pub const RICH_MESSAGE_MAX_MEDIA: usize = 50;
pub const RICH_MESSAGE_MAX_TABLE_COLUMNS: usize = 20;
pub const RICH_MESSAGE_MAX_BUTTONS_PER_ROW: usize = 8;

#[derive(Default)]
struct RichMessageStats {
    text_chars: usize,
    blocks: usize,
    max_depth: usize,
}

fn value_text_chars(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        Value::Array(values) => values.iter().map(value_text_chars).sum(),
        Value::Object(object) => object
            .iter()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "type"
                        | "url"
                        | "callback_data"
                        | "web_app"
                        | "style"
                        | "align"
                        | "valign"
                        | "language"
                        | "name"
                        | "document"
                )
            })
            .map(|(_, value)| value_text_chars(value))
            .sum(),
        _ => 0,
    }
}

pub(crate) fn value_to_text(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => out.push_str(text),
        Value::Array(values) => {
            for val in values {
                value_to_text(val, out);
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text") {
                value_to_text(text, out);
            } else {
                for (key, val) in object {
                    if !matches!(
                        key.as_str(),
                        "type"
                            | "url"
                            | "callback_data"
                            | "web_app"
                            | "style"
                            | "align"
                            | "valign"
                            | "language"
                            | "name"
                            | "document"
                    ) {
                        value_to_text(val, out);
                    }
                }
            }
        }
        _ => {}
    }
}

fn is_nested_block_type(kind: &str) -> bool {
    matches!(
        kind,
        "paragraph"
            | "heading"
            | "pre"
            | "footer"
            | "list"
            | "blockquote"
            | "expandable_blockquote"
            | "pullquote"
            | "divider"
            | "mathematical_expression"
            | "table"
            | "buttons"
            | "document"
            | "details"
            | "anchor"
            | "thinking"
            | "photo"
            | "video"
            | "audio"
            | "voice_note"
            | "animation"
            | "collage"
            | "slideshow"
            | "map"
    )
}

fn collect_nested_value_stats(value: &Value, depth: usize, stats: &mut RichMessageStats) {
    match value {
        Value::Array(values) => {
            for child in values {
                collect_nested_value_stats(child, depth, stats);
            }
        }
        Value::Object(object) => {
            let is_block = object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(is_nested_block_type);
            let child_depth = if is_block { depth + 1 } else { depth };
            if is_block {
                stats.blocks += 1;
                stats.max_depth = stats.max_depth.max(child_depth);
            }
            for child in object.values() {
                collect_nested_value_stats(child, child_depth, stats);
            }
        }
        _ => {}
    }
}

impl InputRichMessage {
    pub fn new(blocks: Vec<RichBlock>) -> Self {
        Self {
            blocks,
            html: None,
            markdown: None,
            media: None,
            is_rtl: None,
            skip_entity_detection: None,
        }
    }

    pub fn from_html(html: impl Into<String>, media: Option<Vec<InputRichMessageMedia>>) -> Self {
        Self {
            blocks: Vec::new(),
            html: Some(html.into()),
            markdown: None,
            media,
            is_rtl: None,
            skip_entity_detection: None,
        }
    }

    pub fn from_markdown(markdown: impl Into<String>) -> Self {
        Self {
            blocks: Vec::new(),
            html: None,
            markdown: Some(markdown.into()),
            media: None,
            is_rtl: None,
            skip_entity_detection: None,
        }
    }

    pub fn with_media(mut self, media: Vec<InputRichMessageMedia>) -> Self {
        self.media = Some(media);
        self
    }

    pub fn has_media(&self) -> bool {
        self.blocks.iter().any(|b| b.is_media())
            || self.media.as_ref().is_some_and(|m| !m.is_empty())
    }

    pub fn collect_media_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        for block in &self.blocks {
            urls.extend(block.get_media_urls());
        }
        if let Some(media) = &self.media {
            for item in media {
                urls.push(item.media.media_url().to_string());
            }
        }
        urls
    }

    pub fn replace_media_urls<F: Fn(&str) -> Option<String>>(&mut self, replacer: &F) {
        for block in &mut self.blocks {
            block.replace_media_urls(replacer);
        }
        if let Some(media) = &mut self.media {
            for item in media {
                if let Some(new_url) = replacer(item.media.media_url()) {
                    item.media.set_media_url(new_url);
                }
            }
        }
    }

    pub fn extract_plain_text(&self) -> String {
        if let Some(md) = &self.markdown {
            return md.clone();
        }
        if let Some(html) = &self.html {
            return html.clone();
        }
        let parts: Vec<String> = self
            .blocks
            .iter()
            .map(|b| b.extract_text())
            .filter(|s| !s.is_empty())
            .collect();
        parts.join("\n\n")
    }

    pub fn validate(&self) -> Result<(), String> {
        let representation_count = usize::from(!self.blocks.is_empty())
            + usize::from(self.html.is_some())
            + usize::from(self.markdown.is_some());
        if representation_count != 1 {
            return Err(format!(
                "InputRichMessage must contain exactly one of blocks, html, or markdown; found {representation_count}"
            ));
        }

        if let Some(media) = &self.media {
            if media.len() > RICH_MESSAGE_MAX_MEDIA {
                return Err(format!(
                    "Rich Message media count exceeds Telegram limit of {RICH_MESSAGE_MAX_MEDIA}"
                ));
            }
            let mut seen_ids = std::collections::HashSet::new();
            for item in media {
                item.validate()?;
                if !seen_ids.insert(&item.id) {
                    return Err(format!(
                        "InputRichMessage media IDs must be unique; duplicate ID found: '{}'",
                        item.id
                    ));
                }
            }
        }

        let mut stats = RichMessageStats::default();
        if let Some(html) = &self.html {
            stats.text_chars = html.chars().count();
        } else if let Some(markdown) = &self.markdown {
            stats.text_chars = markdown.chars().count();
        }

        for block in &self.blocks {
            stats.blocks += 1;
            stats.max_depth = stats.max_depth.max(1);
            match block {
                RichBlock::Paragraph { text }
                | RichBlock::SectionHeading { text, .. }
                | RichBlock::Thinking { text }
                | RichBlock::Footer { text } => {
                    stats.text_chars += value_text_chars(text);
                    collect_nested_value_stats(text, 1, &mut stats);
                }
                RichBlock::Preformatted { text, .. } => stats.text_chars += text.chars().count(),
                RichBlock::List { items } => {
                    stats.blocks += items.len();
                    if !items.is_empty() {
                        stats.max_depth = stats.max_depth.max(2);
                    }
                    for item in items {
                        for value in &item.blocks {
                            stats.text_chars += value_text_chars(value);
                            collect_nested_value_stats(value, 2, &mut stats);
                        }
                    }
                }
                RichBlock::BlockQuotation { blocks } => {
                    for value in blocks {
                        stats.text_chars += value_text_chars(value);
                        collect_nested_value_stats(value, 1, &mut stats);
                    }
                }
                RichBlock::ExpandableBlockQuotation { text, credit }
                | RichBlock::PullQuotation { text, credit } => {
                    stats.text_chars += value_text_chars(text);
                    if let Some(credit) = credit {
                        stats.text_chars += value_text_chars(credit);
                    }
                }
                RichBlock::Divider {} | RichBlock::Anchor { .. } => {}
                RichBlock::MathematicalExpression { expression } => {
                    stats.text_chars += expression.chars().count();
                }
                RichBlock::Table { cells, caption, .. } => {
                    stats.blocks += cells.len();
                    if !cells.is_empty() {
                        stats.max_depth = stats.max_depth.max(2);
                    }
                    if cells
                        .iter()
                        .any(|row| row.len() > RICH_MESSAGE_MAX_TABLE_COLUMNS)
                    {
                        return Err(format!(
                            "Rich Message table exceeds Telegram limit of {RICH_MESSAGE_MAX_TABLE_COLUMNS} columns"
                        ));
                    }
                    for row in cells {
                        for cell in row {
                            stats.text_chars += value_text_chars(&cell.text);
                            collect_nested_value_stats(&cell.text, 2, &mut stats);
                        }
                    }
                    if let Some(caption) = caption {
                        stats.text_chars += caption.chars().count();
                    }
                }
                RichBlock::Buttons { buttons, .. } => {
                    if buttons.is_empty() || buttons.len() > RICH_MESSAGE_MAX_BUTTONS_PER_ROW {
                        return Err(format!(
                            "Rich Message button row must contain 1-{RICH_MESSAGE_MAX_BUTTONS_PER_ROW} buttons"
                        ));
                    }
                    for button in buttons {
                        button.validate()?;
                        stats.text_chars += value_text_chars(&button.text);
                    }
                }
                RichBlock::Document { caption, .. }
                | RichBlock::Photo { caption, .. }
                | RichBlock::Video { caption, .. }
                | RichBlock::Audio { caption, .. }
                | RichBlock::VoiceNote { caption, .. }
                | RichBlock::Animation { caption, .. } => {
                    if let Some(caption) = caption {
                        stats.text_chars += value_text_chars(&caption.text);
                        if let Some(credit) = &caption.credit {
                            stats.text_chars += value_text_chars(credit);
                        }
                    }
                }
                RichBlock::Collage { blocks, caption }
                | RichBlock::Slideshow { blocks, caption } => {
                    stats.blocks += blocks.len();
                    for val in blocks {
                        stats.text_chars += value_text_chars(val);
                        collect_nested_value_stats(val, 1, &mut stats);
                    }
                    if let Some(caption) = caption {
                        stats.text_chars += value_text_chars(&caption.text);
                        if let Some(credit) = &caption.credit {
                            stats.text_chars += value_text_chars(credit);
                        }
                    }
                }
                RichBlock::Map { location, zoom, .. } => {
                    location.validate()?;
                    if let Some(z) = zoom {
                        if !(1..=20).contains(z) {
                            return Err(format!("Map zoom must be between 1 and 20; found {z}"));
                        }
                    }
                }
                RichBlock::Details {
                    summary, blocks, ..
                } => {
                    stats.text_chars += value_text_chars(summary);
                    for value in blocks {
                        stats.text_chars += value_text_chars(value);
                        collect_nested_value_stats(value, 1, &mut stats);
                    }
                }
            }
        }

        if stats.text_chars > RICH_MESSAGE_MAX_TEXT_CHARS {
            return Err(format!(
                "Rich Message text exceeds Telegram limit of {RICH_MESSAGE_MAX_TEXT_CHARS} characters"
            ));
        }
        if stats.blocks > RICH_MESSAGE_MAX_BLOCKS {
            return Err(format!(
                "Rich Message contains {} blocks; Telegram limit is {RICH_MESSAGE_MAX_BLOCKS}",
                stats.blocks
            ));
        }
        if stats.max_depth > RICH_MESSAGE_MAX_NESTING {
            return Err(format!(
                "Rich Message nesting depth {} exceeds Telegram limit of {RICH_MESSAGE_MAX_NESTING}",
                stats.max_depth
            ));
        }
        Ok(())
    }
}

pub fn deserialize_flexible_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct FlexibleI64Visitor;

    impl<'de> serde::de::Visitor<'de> for FlexibleI64Visitor {
        type Value = i64;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an integer or a string representing an integer")
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(v)
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            i64::try_from(v).map_err(serde::de::Error::custom)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            v.trim().parse::<i64>().map_err(serde::de::Error::custom)
        }
    }

    deserializer.deserialize_any(FlexibleI64Visitor)
}

pub fn deserialize_flexible_opt_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct FlexibleOptI64Visitor;

    impl<'de> serde::de::Visitor<'de> for FlexibleOptI64Visitor {
        type Value = Option<i64>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an optional integer or a string representing an integer")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserialize_flexible_i64(deserializer).map(Some)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }
    }

    deserializer.deserialize_option(FlexibleOptI64Visitor)
}

pub fn deserialize_flexible_i32<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct FlexibleI32Visitor;

    impl<'de> serde::de::Visitor<'de> for FlexibleI32Visitor {
        type Value = i32;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an integer or a string representing an integer")
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            i32::try_from(v).map_err(serde::de::Error::custom)
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            i32::try_from(v).map_err(serde::de::Error::custom)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            v.trim().parse::<i32>().map_err(serde::de::Error::custom)
        }
    }

    deserializer.deserialize_any(FlexibleI32Visitor)
}

pub fn deserialize_flexible_opt_i32<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct FlexibleOptI32Visitor;

    impl<'de> serde::de::Visitor<'de> for FlexibleOptI32Visitor {
        type Value = Option<i32>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an optional integer or a string representing an integer")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserialize_flexible_i32(deserializer).map(Some)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }
    }

    deserializer.deserialize_option(FlexibleOptI32Visitor)
}

// ==========================================
// Telegram Updates & Message Payloads
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub description: Option<String>,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_i64")]
    pub error_code: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Update {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub update_id: i64,
    pub message: Option<Message>,
    pub callback_query: Option<CallbackQuery>,
    pub stopped_message_generation: Option<MessageGenerationStopped>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<Poll>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageGenerationStopped {
    pub chat: Chat,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_i64")]
    pub message_thread_id: Option<i64>,
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub draft_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub message_id: i64,
    pub from: Option<User>,
    pub chat: Chat,
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub date: i64,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_i64")]
    pub message_thread_id: Option<i64>,
    pub receiver_user: Option<User>,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_i64")]
    pub ephemeral_message_id: Option<i64>,
    pub text: Option<String>,
    pub caption: Option<String>,
    pub photo: Option<Vec<PhotoSize>>,
    pub document: Option<Document>,
    pub voice: Option<Voice>,
    pub audio: Option<Audio>,
    pub video: Option<Video>,
    pub video_note: Option<VideoNote>,
    pub reply_to_message: Option<Box<Message>>,
    pub community_chat_joined: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<Poll>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyParameters {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_flexible_opt_i64"
    )]
    pub message_id: Option<i64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_flexible_opt_i64"
    )]
    pub ephemeral_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_sending_without_reply: Option<bool>,
}

impl ReplyParameters {
    pub fn new(message_id: i64) -> Self {
        Self {
            message_id: Some(message_id),
            ephemeral_message_id: None,
            allow_sending_without_reply: None,
        }
    }

    pub fn ephemeral(ephemeral_message_id: i64) -> Self {
        Self {
            message_id: None,
            ephemeral_message_id: Some(ephemeral_message_id),
            allow_sending_without_reply: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EphemeralMessageParameters {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub receiver_user_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_query_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replace_callback_query_message: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chat {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
    pub title: Option<String>,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    #[serde(default)]
    pub is_forum: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMember {
    pub status: String,
    pub user: Option<User>,
}

impl ChatMember {
    pub fn is_admin_or_creator(&self) -> bool {
        self.status == "administrator" || self.status == "creator"
    }

    pub fn is_administrator(&self) -> bool {
        self.status == "administrator"
    }

    pub fn is_creator(&self) -> bool {
        self.status == "creator"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhotoSize {
    pub file_id: String,
    pub file_unique_id: String,
    #[serde(deserialize_with = "deserialize_flexible_i32")]
    pub width: i32,
    #[serde(deserialize_with = "deserialize_flexible_i32")]
    pub height: i32,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_i64")]
    pub file_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub file_id: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_i64")]
    pub file_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voice {
    pub file_id: String,
    #[serde(deserialize_with = "deserialize_flexible_i32")]
    pub duration: i32,
    pub mime_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_i64")]
    pub file_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Audio {
    pub file_id: String,
    #[serde(deserialize_with = "deserialize_flexible_i32")]
    pub duration: i32,
    pub performer: Option<String>,
    pub title: Option<String>,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_i64")]
    pub file_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Video {
    pub file_id: String,
    #[serde(deserialize_with = "deserialize_flexible_i32")]
    pub width: i32,
    #[serde(deserialize_with = "deserialize_flexible_i32")]
    pub height: i32,
    #[serde(deserialize_with = "deserialize_flexible_i32")]
    pub duration: i32,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_i64")]
    pub file_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoNote {
    pub file_id: String,
    #[serde(deserialize_with = "deserialize_flexible_i32")]
    pub length: i32,
    #[serde(deserialize_with = "deserialize_flexible_i32")]
    pub duration: i32,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_i64")]
    pub file_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: User,
    pub message: Option<Message>,
    pub inline_message_id: Option<String>,
    pub data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub file_id: String,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_i64")]
    pub file_size: Option<i64>,
    pub file_path: Option<String>,
}

// ==========================================
// Poll & Quiz Models
// ==========================================

pub const QUIZ_MAX_QUESTION_CHARS: usize = 300;
pub const QUIZ_MIN_OPTIONS: usize = 2;
pub const QUIZ_MAX_OPTIONS: usize = 10;
pub const QUIZ_MAX_OPTION_CHARS: usize = 100;
pub const QUIZ_MAX_EXPLANATION_CHARS: usize = 200;
pub const QUIZ_MAX_EXPLANATION_LINE_BREAKS: usize = 2;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InputPollOption {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_parse_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_entities: Option<Vec<Value>>,
}

impl<'de> serde::Deserialize<'de> for InputPollOption {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawOption {
            text: String,
            #[serde(default)]
            text_parse_mode: Option<String>,
            #[serde(default)]
            text_entities: Option<Vec<Value>>,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Helper {
            Str(String),
            Obj(RawOption),
        }

        match Helper::deserialize(deserializer)? {
            Helper::Str(s) => Ok(InputPollOption::new(s)),
            Helper::Obj(obj) => Ok(InputPollOption {
                text: obj.text,
                text_parse_mode: obj.text_parse_mode,
                text_entities: obj.text_entities,
            }),
        }
    }
}

impl InputPollOption {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            text_parse_mode: None,
            text_entities: None,
        }
    }

    pub fn with_parse_mode(text: impl Into<String>, parse_mode: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            text_parse_mode: Some(parse_mode.into()),
            text_entities: None,
        }
    }
}

impl From<String> for InputPollOption {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl From<&str> for InputPollOption {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PollOption {
    pub text: String,
    #[serde(default, deserialize_with = "deserialize_flexible_i32")]
    pub voter_count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_entities: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Poll {
    pub id: String,
    pub question: String,
    pub options: Vec<PollOption>,
    #[serde(default, deserialize_with = "deserialize_flexible_i32")]
    pub total_voter_count: i32,
    #[serde(default)]
    pub is_closed: bool,
    #[serde(default)]
    pub is_anonymous: bool,
    #[serde(rename = "type")]
    pub poll_type: String,
    #[serde(default)]
    pub allows_multiple_answers: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_flexible_opt_i32"
    )]
    pub correct_option_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation_entities: Option<Vec<Value>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_flexible_opt_i32"
    )]
    pub open_period: Option<i32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_flexible_opt_i64"
    )]
    pub close_date: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question_entities: Option<Vec<Value>>,
}

pub fn validate_quiz(
    question: &str,
    options: &[InputPollOption],
    correct_option_id: i32,
    explanation: Option<&str>,
) -> Result<(), String> {
    let q_trimmed = question.trim();
    if q_trimmed.is_empty() {
        return Err("Quiz question cannot be empty".to_string());
    }
    let q_len = q_trimmed.chars().count();
    if q_len > QUIZ_MAX_QUESTION_CHARS {
        return Err(format!(
            "Quiz question exceeds maximum length of {QUIZ_MAX_QUESTION_CHARS} characters (found {q_len})"
        ));
    }
    if options.len() < QUIZ_MIN_OPTIONS || options.len() > QUIZ_MAX_OPTIONS {
        return Err(format!(
            "Quiz must have between {QUIZ_MIN_OPTIONS} and {QUIZ_MAX_OPTIONS} options (found {})",
            options.len()
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for (idx, opt) in options.iter().enumerate() {
        let opt_trimmed = opt.text.trim();
        if opt_trimmed.is_empty() {
            return Err(format!("Quiz option {} cannot be empty", idx + 1));
        }
        let opt_len = opt_trimmed.chars().count();
        if opt_len > QUIZ_MAX_OPTION_CHARS {
            return Err(format!(
                "Quiz option {} exceeds maximum length of {QUIZ_MAX_OPTION_CHARS} characters (found {opt_len})",
                idx + 1
            ));
        }
        if !seen.insert(opt_trimmed) {
            return Err(format!(
                "Quiz options must be unique (found duplicate: '{opt_trimmed}')"
            ));
        }
    }
    if correct_option_id < 0 || correct_option_id as usize >= options.len() {
        return Err(format!(
            "Quiz correct_option_id must be between 0 and {} (found {correct_option_id})",
            options.len().saturating_sub(1)
        ));
    }
    if let Some(exp) = explanation {
        let exp_trimmed = exp.trim();
        let exp_len = exp_trimmed.chars().count();
        if exp_len > QUIZ_MAX_EXPLANATION_CHARS {
            return Err(format!(
                "Quiz explanation exceeds maximum length of {QUIZ_MAX_EXPLANATION_CHARS} characters (found {exp_len})"
            ));
        }
        let normalized_exp = exp_trimmed.replace("\r\n", "\n");
        let line_breaks = normalized_exp
            .chars()
            .filter(|&c| c == '\n' || c == '\r')
            .count();
        if line_breaks > QUIZ_MAX_EXPLANATION_LINE_BREAKS {
            return Err(format!(
                "Quiz explanation cannot exceed {QUIZ_MAX_EXPLANATION_LINE_BREAKS} line breaks (found {line_breaks})"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_message_requires_exactly_one_representation() {
        let empty = InputRichMessage::default();
        assert!(empty.validate().is_err());

        let mut conflicting = InputRichMessage::new(vec![RichBlock::Paragraph {
            text: Value::String("hello".to_string()),
        }]);
        conflicting.markdown = Some("hello".to_string());
        assert!(conflicting.validate().is_err());

        let valid = InputRichMessage::new(vec![RichBlock::Paragraph {
            text: Value::String("hello".to_string()),
        }]);
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn rich_message_copy_text_button_validates_length_and_action() {
        let button = RichMessageButton::copy("Salin", "teks yang disalin");
        assert!(button.validate().is_ok());

        let value = serde_json::to_value(&button).expect("serialize button succeeds");
        assert_eq!(value["copy_text"]["text"], "teks yang disalin");

        let inline_button = InlineKeyboardButton::copy("Salin Prompt", "prompt text");
        let inline_value =
            serde_json::to_value(&inline_button).expect("serialize inline button succeeds");
        assert_eq!(inline_value["copy_text"]["text"], "prompt text");

        let mut invalid_button = RichMessageButton::copy("Salin", "a".repeat(257));
        assert!(invalid_button.validate().is_err());

        invalid_button.copy_text = Some(CopyTextButton::new(""));
        assert!(invalid_button.validate().is_err());
    }

    #[test]
    fn footer_and_pullquote_serialize_and_validate() {
        let message = InputRichMessage::new(vec![
            RichBlock::PullQuotation {
                text: Value::String("Kutipan penting".to_string()),
                credit: Some(Value::String("Penulis".to_string())),
            },
            RichBlock::Footer {
                text: Value::String("⚡ gpt-4o".to_string()),
            },
        ]);
        assert!(message.validate().is_ok());

        let serialized = serde_json::to_value(&message).expect("serialize message succeeds");
        assert_eq!(serialized["blocks"][0]["type"], "pullquote");
        assert_eq!(serialized["blocks"][0]["text"], "Kutipan penting");
        assert_eq!(serialized["blocks"][1]["type"], "footer");
        assert_eq!(serialized["blocks"][1]["text"], "⚡ gpt-4o");
    }

    #[test]
    fn rich_message_button_requires_exactly_one_action() {
        let mut button = RichMessageButton::callback("Open", "open");
        assert!(button.validate().is_ok());

        button.url = Some("https://example.com".to_string());
        assert!(button.validate().is_err());

        button.callback_data = None;
        assert!(button.validate().is_ok());

        button.url = None;
        assert!(button.validate().is_err());
    }

    #[test]
    fn disabled_inline_button_serializes_as_empty_object() {
        let value = serde_json::to_value(InlineKeyboardButton::disabled("Unavailable"))
            .expect("serialize disabled button succeeds");
        assert_eq!(value["disabled"], serde_json::json!({}));
        assert!(value.get("callback_data").is_none());
    }

    #[test]
    fn bot_api_10_3_stop_update_deserializes() {
        let update: Update = serde_json::from_value(serde_json::json!({
            "update_id": 42,
            "stopped_message_generation": {
                "chat": {"id": 7, "type": "private"},
                "draft_id": 99
            }
        }))
        .expect("deserialize stop update succeeds");
        let stopped = update
            .stopped_message_generation
            .expect("stopped_message_generation present");
        assert_eq!(stopped.chat.id, 7);
        assert_eq!(stopped.draft_id, 99);
    }

    #[test]
    fn rich_message_buttons_follow_10_3_shape() {
        let block = RichBlock::Buttons {
            buttons: vec![RichMessageButton::callback_styled(
                "Retry", "retry", "primary",
            )],
            align: Some("center".to_string()),
        };
        let value = serde_json::to_value(block).expect("serialize button block succeeds");
        assert_eq!(value["type"], "buttons");
        assert_eq!(value["buttons"][0]["callback_data"], "retry");
        assert_eq!(value["buttons"][0]["style"], "primary");
    }

    #[test]
    fn expandable_quote_follows_10_3_shape() {
        let block = RichBlock::ExpandableBlockQuotation {
            text: Value::String("detail".to_string()),
            credit: Some(Value::String("source".to_string())),
        };
        let value = serde_json::to_value(block).expect("serialize quote succeeds");
        assert_eq!(value["type"], "expandable_blockquote");
        assert_eq!(value["text"], "detail");
        assert_eq!(value["credit"], "source");
        assert!(value.get("blocks").is_none());
        assert!(value.get("is_open").is_none());
    }

    #[test]
    fn details_block_uses_summary_and_blocks() {
        let block = RichBlock::Details {
            summary: Value::String("More".to_string()),
            blocks: vec![serde_json::json!({"type": "paragraph", "text": "Body"})],
            is_open: Some(true),
        };
        let value = serde_json::to_value(block).expect("serialize details succeeds");
        assert_eq!(value["summary"], "More");
        assert!(value["blocks"].is_array());
        assert_eq!(value["is_open"], true);
    }

    #[test]
    fn rich_message_text_limit_is_enforced_at_boundary() {
        let at_limit = InputRichMessage::new(vec![RichBlock::Paragraph {
            text: Value::String("x".repeat(RICH_MESSAGE_MAX_TEXT_CHARS)),
        }]);
        assert!(at_limit.validate().is_ok());
        let over = InputRichMessage::new(vec![RichBlock::Paragraph {
            text: Value::String("x".repeat(RICH_MESSAGE_MAX_TEXT_CHARS + 1)),
        }]);
        assert!(over.validate().is_err());
    }

    #[test]
    fn rich_message_block_limit_is_enforced_at_boundary() {
        let paragraph = || RichBlock::Paragraph {
            text: Value::String("x".to_string()),
        };
        let at_limit =
            InputRichMessage::new((0..RICH_MESSAGE_MAX_BLOCKS).map(|_| paragraph()).collect());
        assert!(at_limit.validate().is_ok());
        let over =
            InputRichMessage::new((0..=RICH_MESSAGE_MAX_BLOCKS).map(|_| paragraph()).collect());
        assert!(over.validate().is_err());
    }

    #[test]
    fn rich_message_media_table_and_button_limits_are_local() {
        let mut message = InputRichMessage::new(vec![RichBlock::Paragraph {
            text: Value::String("ok".to_string()),
        }]);
        message.media = Some(
            (0..RICH_MESSAGE_MAX_MEDIA)
                .map(|i| {
                    InputRichMessageMedia::photo(
                        format!("pic_{i}"),
                        format!("https://example.com/pic_{i}.jpg"),
                        None,
                    )
                    .expect("valid media")
                })
                .collect(),
        );
        assert!(message.validate().is_ok());
        message.media.as_mut().expect("media vector present").push(
            InputRichMessageMedia::photo(
                format!("pic_{RICH_MESSAGE_MAX_MEDIA}"),
                "https://example.com/pic_extra.jpg",
                None,
            )
            .expect("valid media"),
        );
        assert!(message.validate().is_err());

        let table = |columns: usize| {
            InputRichMessage::new(vec![RichBlock::Table {
                cells: vec![(0..columns)
                    .map(|_| RichBlockTableCell::text_only("x", false, None))
                    .collect()],
                has_header: false,
                is_bordered: true,
                is_striped: false,
                is_compact: true,
                caption: None,
            }])
        };
        assert!(table(RICH_MESSAGE_MAX_TABLE_COLUMNS).validate().is_ok());
        assert!(table(RICH_MESSAGE_MAX_TABLE_COLUMNS + 1)
            .validate()
            .is_err());

        let buttons = |count: usize| {
            InputRichMessage::new(vec![RichBlock::Buttons {
                buttons: (0..count)
                    .map(|index| {
                        RichMessageButton::callback(format!("b{index}"), format!("c{index}"))
                    })
                    .collect(),
                align: None,
            }])
        };
        assert!(buttons(RICH_MESSAGE_MAX_BUTTONS_PER_ROW).validate().is_ok());
        assert!(buttons(RICH_MESSAGE_MAX_BUTTONS_PER_ROW + 1)
            .validate()
            .is_err());
    }

    fn nested_details(depth: usize) -> Value {
        if depth == 0 {
            return serde_json::json!({"type": "paragraph", "text": "leaf"});
        }
        serde_json::json!({
            "type": "details",
            "summary": "nested",
            "blocks": [nested_details(depth - 1)]
        })
    }

    #[test]
    fn rich_media_blocks_serialize_and_validate() {
        let msg = InputRichMessage::new(vec![
            RichBlock::Photo {
                photo: serde_json::json!({"type": "photo", "media": "attach://photo1"}),
                caption: Some(RichBlockCaption::new(Value::String(
                    "Pemandangan".to_string(),
                ))),
            },
            RichBlock::Video {
                video: serde_json::json!({"type": "video", "media": "attach://video1"}),
                caption: None,
            },
            RichBlock::Map {
                location: Location {
                    latitude: -5.147665,
                    longitude: 119.432732,
                    horizontal_accuracy: Some(10.0),
                },
                zoom: Some(15),
                width: Some(600),
                height: Some(400),
            },
        ]);
        assert!(msg.validate().is_ok());

        let val = serde_json::to_value(&msg).expect("serialize rich media msg succeeds");
        assert_eq!(val["blocks"][0]["type"], "photo");
        assert_eq!(val["blocks"][0]["caption"]["text"], "Pemandangan");
        assert_eq!(val["blocks"][1]["type"], "video");
        assert_eq!(val["blocks"][2]["type"], "map");
        assert_eq!(val["blocks"][2]["location"]["latitude"], -5.147665);
    }

    #[test]
    fn extended_button_actions_serialize_and_validate() {
        let mut btn = RichMessageButton::callback("Click", "data");
        assert!(btn.validate().is_ok());

        btn.callback_data = None;
        btn.login_url = Some(LoginUrl {
            url: "https://auth.example.com/login".to_string(),
            forward_text: Some("Log in".to_string()),
            bot_username: Some("xiao_bot".to_string()),
            request_write_access: Some(true),
        });
        assert!(btn.validate().is_ok());

        btn.login_url = None;
        btn.switch_inline_query_chosen_chat = Some(SwitchInlineQueryChosenChat {
            query: Some("search query".to_string()),
            allow_user_chats: Some(true),
            allow_bot_chats: Some(false),
            allow_group_chats: Some(true),
            allow_channel_chats: Some(false),
        });
        assert!(btn.validate().is_ok());

        let text_btn = RichTextButton {
            button: btn.clone(),
        };
        let text_val = serde_json::to_value(&text_btn).expect("serialize text_btn succeeds");
        assert!(text_val.get("button").is_some());
    }

    #[test]
    fn rich_message_nesting_limit_is_enforced() {
        // Top-level Details is depth 1, so fifteen nested block levels reaches 16.
        let at_limit = InputRichMessage::new(vec![RichBlock::Details {
            summary: Value::String("root".to_string()),
            blocks: vec![nested_details(RICH_MESSAGE_MAX_NESTING - 2)],
            is_open: None,
        }]);
        assert!(at_limit.validate().is_ok());
        let over = InputRichMessage::new(vec![RichBlock::Details {
            summary: Value::String("root".to_string()),
            blocks: vec![nested_details(RICH_MESSAGE_MAX_NESTING - 1)],
            is_open: None,
        }]);
        assert!(over.validate().is_err());
    }

    #[test]
    fn extract_plain_text_extracts_all_rich_block_types() {
        let msg = InputRichMessage::new(vec![
            RichBlock::Thinking {
                text: Value::String("🧩 Thinking...".to_string()),
            },
            RichBlock::Paragraph {
                text: serde_json::json!([
                    {"type": "bold", "text": "Halo"},
                    " dunia!"
                ]),
            },
            RichBlock::Preformatted {
                text: "println!(\"hello\");".to_string(),
                language: Some("rust".to_string()),
            },
        ]);
        let plain = msg.extract_plain_text();
        assert!(plain.contains("🧩 Thinking..."));
        assert!(plain.contains("Halo dunia!"));
        assert!(plain.contains("println!(\"hello\");"));
    }

    #[test]
    fn input_rich_message_media_id_validation() {
        // Valid IDs: 1 to 64 chars, ASCII alphanumeric and underscore
        assert!(InputRichMessageMedia::validate_id("a").is_ok());
        assert!(InputRichMessageMedia::validate_id("Z").is_ok());
        assert!(InputRichMessageMedia::validate_id("0").is_ok());
        assert!(InputRichMessageMedia::validate_id("_").is_ok());
        assert!(InputRichMessageMedia::validate_id("photo_1_preview").is_ok());
        assert!(InputRichMessageMedia::validate_id(&"x".repeat(64)).is_ok());

        // Invalid IDs: empty, > 64 chars, or containing invalid characters
        assert!(InputRichMessageMedia::validate_id("").is_err());
        assert!(InputRichMessageMedia::validate_id(&"x".repeat(65)).is_err());
        assert!(InputRichMessageMedia::validate_id("photo 1").is_err());
        assert!(InputRichMessageMedia::validate_id("photo-1").is_err());
        assert!(InputRichMessageMedia::validate_id("photo.jpg").is_err());
        assert!(InputRichMessageMedia::validate_id("pic@home").is_err());
        assert!(InputRichMessageMedia::validate_id("foto#1").is_err());
    }

    #[test]
    fn input_rich_message_media_constructors() {
        let photo = InputRichMessageMedia::photo(
            "p1",
            "https://example.com/pic.jpg",
            Some("Caption".to_string()),
        );
        assert!(photo.is_ok());
        let photo = photo.expect("valid photo");
        assert_eq!(photo.id, "p1");
        assert_eq!(photo.media.media_url(), "https://example.com/pic.jpg");

        let audio = InputRichMessageMedia::audio(
            "a1",
            "https://example.com/sound.mp3",
            Some("Title".to_string()),
            Some("Artist".to_string()),
            None,
        );
        assert!(audio.is_ok());
        let audio = audio.expect("valid audio");
        assert_eq!(audio.id, "a1");
        assert_eq!(audio.media.media_url(), "https://example.com/sound.mp3");

        let doc = InputRichMessageMedia::document("d1", "https://example.com/file.pdf", None);
        assert!(doc.is_ok());

        let vid = InputRichMessageMedia::video("v1", "https://example.com/vid.mp4", None);
        assert!(vid.is_ok());
    }

    #[test]
    fn input_rich_message_unique_media_ids_enforced() {
        let item1 = InputRichMessageMedia::photo("same_id", "https://example.com/1.jpg", None)
            .expect("valid");
        let item2 = InputRichMessageMedia::photo("same_id", "https://example.com/2.jpg", None)
            .expect("valid");

        let msg = InputRichMessage::from_html("<p>test</p>", Some(vec![item1, item2]));
        let err = msg
            .validate()
            .expect_err("Duplicate media IDs must be rejected");
        assert!(err.contains("unique"), "Error must mention unique: {err}");
        assert!(
            err.contains("same_id"),
            "Error must mention duplicated ID: {err}"
        );

        // Distinct IDs are valid
        let item3 = InputRichMessageMedia::photo("diff_id", "https://example.com/2.jpg", None)
            .expect("valid");
        let item1 = InputRichMessageMedia::photo("same_id", "https://example.com/1.jpg", None)
            .expect("valid");
        let valid_msg = InputRichMessage::from_html("<p>test</p>", Some(vec![item1, item3]));
        assert!(valid_msg.validate().is_ok());
    }

    #[test]
    fn input_rich_message_wire_serialization_matches_bot_api_10_2() {
        let photo = InputRichMessageMedia::photo(
            "pic1",
            "https://example.com/summit.jpg",
            Some("Summit view".to_string()),
        )
        .expect("valid");
        let msg = InputRichMessage::from_html(
            "<h3>Rinjani</h3><img src=\"tg://photo?id=pic1\"/>",
            Some(vec![photo]),
        );
        assert!(msg.validate().is_ok());

        let json_val = serde_json::to_value(&msg).expect("serialization succeeds");
        assert_eq!(
            json_val["html"],
            "<h3>Rinjani</h3><img src=\"tg://photo?id=pic1\"/>"
        );
        assert!(json_val["media"].is_array());
        let media_arr = json_val["media"].as_array().expect("media array");
        assert_eq!(media_arr.len(), 1);
        assert_eq!(media_arr[0]["id"], "pic1");
        assert_eq!(media_arr[0]["media"]["type"], "photo");
        assert_eq!(
            media_arr[0]["media"]["media"],
            "https://example.com/summit.jpg"
        );
        assert_eq!(media_arr[0]["media"]["caption"], "Summit view");

        // Roundtrip deserialization
        let deserialized: InputRichMessage =
            serde_json::from_value(json_val).expect("deserialization succeeds");
        assert_eq!(deserialized.html, msg.html);
        let d_media = deserialized.media.expect("deserialized media present");
        assert_eq!(d_media.len(), 1);
        assert_eq!(d_media[0].id, "pic1");
        assert_eq!(
            d_media[0].media.media_url(),
            "https://example.com/summit.jpg"
        );
    }

    #[test]
    fn location_and_tg_map_coordinates_validation() {
        // Valid coordinates
        let valid_coords = vec![
            (0.0, 0.0),
            (-90.0, -180.0),
            (90.0, 180.0),
            (-8.4113, 116.4573), // Rinjani
            (40.7128, -74.0060), // New York
        ];
        for (lat, lon) in valid_coords {
            let loc = Location::new(lat, lon);
            assert!(loc.is_ok(), "Coordinates ({lat}, {lon}) should be valid");
        }

        // Invalid latitude
        assert!(Location::new(90.0001, 0.0).is_err());
        assert!(Location::new(-90.0001, 0.0).is_err());
        assert!(Location::new(f64::NAN, 0.0).is_err());
        assert!(Location::new(f64::INFINITY, 0.0).is_err());

        // Invalid longitude
        assert!(Location::new(0.0, 180.0001).is_err());
        assert!(Location::new(0.0, -180.0001).is_err());
        assert!(Location::new(0.0, f64::NAN).is_err());
        assert!(Location::new(0.0, f64::NEG_INFINITY).is_err());

        // Map RichBlock validation
        let valid_map = InputRichMessage::new(vec![RichBlock::Map {
            location: Location {
                latitude: -8.4113,
                longitude: 116.4573,
                horizontal_accuracy: None,
            },
            zoom: Some(13),
            width: None,
            height: None,
        }]);
        assert!(valid_map.validate().is_ok());

        // Invalid zoom
        let invalid_zoom_zero = InputRichMessage::new(vec![RichBlock::Map {
            location: Location {
                latitude: -8.4113,
                longitude: 116.4573,
                horizontal_accuracy: None,
            },
            zoom: Some(0),
            width: None,
            height: None,
        }]);
        assert!(invalid_zoom_zero.validate().is_err());

        let invalid_zoom_high = InputRichMessage::new(vec![RichBlock::Map {
            location: Location {
                latitude: -8.4113,
                longitude: 116.4573,
                horizontal_accuracy: None,
            },
            zoom: Some(25),
            width: None,
            height: None,
        }]);
        assert!(invalid_zoom_high.validate().is_err());

        // Invalid location inside Map block
        let invalid_lat_map = InputRichMessage::new(vec![RichBlock::Map {
            location: Location {
                latitude: 99.0,
                longitude: 0.0,
                horizontal_accuracy: None,
            },
            zoom: Some(10),
            width: None,
            height: None,
        }]);
        assert!(invalid_lat_map.validate().is_err());
    }

    #[test]
    fn input_rich_message_partial_eq_and_animation_voice() {
        let photo1 = match InputRichMessageMedia::photo("p1", "https://example.com/1.jpg", None) {
            Ok(m) => m,
            Err(e) => panic!("valid photo failed: {e}"),
        };
        let photo2 = match InputRichMessageMedia::photo("p1", "https://example.com/1.jpg", None) {
            Ok(m) => m,
            Err(e) => panic!("valid photo failed: {e}"),
        };
        let photo3 = match InputRichMessageMedia::photo("p2", "https://example.com/2.jpg", None) {
            Ok(m) => m,
            Err(e) => panic!("valid photo failed: {e}"),
        };
        assert_eq!(photo1, photo2);
        assert_ne!(photo1, photo3);

        let anim = match InputRichMessageMedia::animation(
            "anim_1",
            "https://example.com/gif.mp4",
            Some("Animation".to_string()),
        ) {
            Ok(m) => m,
            Err(e) => panic!("valid animation failed: {e}"),
        };
        assert_eq!(anim.id, "anim_1");
        assert_eq!(anim.media.media_url(), "https://example.com/gif.mp4");

        let voice = match InputRichMessageMedia::voice_note(
            "voice_1",
            "https://example.com/voice.ogg",
            None,
            Some(15),
        ) {
            Ok(m) => m,
            Err(e) => panic!("valid voice failed: {e}"),
        };
        assert_eq!(voice.id, "voice_1");
        assert_eq!(voice.media.media_url(), "https://example.com/voice.ogg");

        let msg1 = InputRichMessage::from_html("<p>test</p>", Some(vec![photo1.clone()]));
        let msg2 = InputRichMessage::from_html("<p>test</p>", Some(vec![photo2]));
        let msg3 = InputRichMessage::from_html("<p>diff</p>", Some(vec![photo3]));
        assert_eq!(msg1, msg2);
        assert_ne!(msg1, msg3);
    }

    #[test]
    fn rich_block_map_helpers_and_validation() {
        let map_res = RichBlock::map_coords(-8.4113, 116.4573, Some(14));
        assert!(map_res.is_ok());
        if let Ok(RichBlock::Map { location, zoom, .. }) = map_res {
            assert_eq!(location.latitude, -8.4113);
            assert_eq!(location.longitude, 116.4573);
            assert_eq!(zoom, Some(14));
        } else {
            panic!("Expected RichBlock::Map variant");
        }

        // Exact boundary coordinates
        assert!(RichBlock::map_coords(-90.0, -180.0, Some(1)).is_ok());
        assert!(RichBlock::map_coords(90.0, 180.0, Some(20)).is_ok());

        // Out of boundary coordinates
        assert!(RichBlock::map_coords(-90.001, 0.0, None).is_err());
        assert!(RichBlock::map_coords(90.001, 0.0, None).is_err());
        assert!(RichBlock::map_coords(0.0, -180.001, None).is_err());
        assert!(RichBlock::map_coords(0.0, 180.001, None).is_err());

        // Non-finite coordinates
        assert!(RichBlock::map_coords(f64::NAN, 0.0, None).is_err());
        assert!(RichBlock::map_coords(0.0, f64::INFINITY, None).is_err());
        assert!(RichBlock::map_coords(0.0, f64::NEG_INFINITY, None).is_err());

        // Zoom boundaries
        assert!(RichBlock::map_coords(0.0, 0.0, Some(0)).is_err());
        assert!(RichBlock::map_coords(0.0, 0.0, Some(21)).is_err());
    }

    #[test]
    fn input_rich_message_media_count_boundary_50_limit() {
        let mut items_50 = Vec::new();
        for i in 0..50 {
            let item = match InputRichMessageMedia::photo(
                format!("id_{i}"),
                format!("https://example.com/{i}.jpg"),
                None,
            ) {
                Ok(item) => item,
                Err(e) => panic!("failed to create media item: {e}"),
            };
            items_50.push(item);
        }

        let msg_50 = InputRichMessage::from_html("<p>50 items</p>", Some(items_50.clone()));
        assert!(msg_50.validate().is_ok(), "50 media items must be accepted");

        let mut items_51 = items_50;
        let item_51 =
            match InputRichMessageMedia::photo("id_50", "https://example.com/50.jpg", None) {
                Ok(item) => item,
                Err(e) => panic!("failed to create media item: {e}"),
            };
        items_51.push(item_51);

        let msg_51 = InputRichMessage::from_html("<p>51 items</p>", Some(items_51));
        assert!(
            msg_51.validate().is_err(),
            "51 media items must be rejected"
        );
    }

    #[test]
    fn input_rich_message_media_rejects_empty_media_url() {
        let empty_photo = InputRichMessageMedia {
            id: "valid_id".to_string(),
            media: InputMedia::photo("", None, None),
        };
        assert!(
            empty_photo.validate().is_err(),
            "Empty media URL must be rejected"
        );
    }
}
