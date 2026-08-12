use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub id: String,
    pub text: String,
    pub is_highlight: bool,
    pub chapter_idx: usize,
    pub timestamp: u64,
    #[serde(default)]
    pub is_me_message: bool,
}

impl ConversationMessage {
    pub fn new(text: String, is_highlight: bool, chapter_idx: usize) -> Self {
        let id = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        Self {
            id,
            text,
            is_highlight,
            chapter_idx,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            is_me_message: false,
        }
    }

    pub fn new_me(text: String, chapter_idx: usize) -> Self {
        let mut msg = Self::new(text, false, chapter_idx);
        msg.is_me_message = true;
        msg
    }

    pub fn new_author(text: String, chapter_idx: usize) -> Self {
        Self::new(text, false, chapter_idx)
    }

    pub fn formatted_timestamp(&self) -> Option<String> {
        if self.timestamp == 0 {
            return None;
        }

        Local
            .timestamp_opt(self.timestamp as i64, 0)
            .single()
            .map(|date_time| date_time.format("%d/%m/%Y - %H:%M").to_string())
    }
}
