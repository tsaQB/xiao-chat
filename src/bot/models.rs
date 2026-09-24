#![allow(dead_code)]
#![allow(unused_imports)]

#[path = "models/base.rs"]
mod base;

pub use base::{
    deserialize_flexible_i32, deserialize_flexible_i64, deserialize_flexible_opt_i32,
    deserialize_flexible_opt_i64, validate_quiz, ApiResponse, Audio, BotCommand, CallbackQuery,
    Chat, ChatMember, CopyTextButton, Document, EphemeralMessageParameters, FileInfo,
    InlineKeyboardButton, InlineKeyboardMarkup, InputMedia, InputPollOption, InputRichMessage,
    InputRichMessageMedia, KeyboardButton, Location, LoginUrl, Message, MessageGenerationStopped,
    PhotoSize, Poll, PollOption, ReplyKeyboardMarkup, ReplyKeyboardRemove, ReplyParameters,
    RichBlock, RichBlockCaption, RichBlockListItem, RichBlockTableCell, RichMessageButton,
    StagedDocument, SwitchInlineQueryChosenChat, Update, User, Video, VideoNote, Voice,
    QUIZ_MAX_EXPLANATION_CHARS, QUIZ_MAX_EXPLANATION_LINE_BREAKS, QUIZ_MAX_OPTIONS,
    QUIZ_MAX_OPTION_CHARS, QUIZ_MAX_QUESTION_CHARS, QUIZ_MIN_OPTIONS, RICH_MESSAGE_MAX_BLOCKS,
    RICH_MESSAGE_MAX_BUTTONS_PER_ROW, RICH_MESSAGE_MAX_MEDIA, RICH_MESSAGE_MAX_NESTING,
    RICH_MESSAGE_MAX_TABLE_COLUMNS, RICH_MESSAGE_MAX_TEXT_CHARS,
};

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RichTextButton {
    pub button: RichMessageButton,
}

impl Serialize for RichTextButton {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("RichTextButton", 2)?;
        state.serialize_field("type", "button")?;
        state.serialize_field("button", &self.button)?;
        state.end()
    }
}
