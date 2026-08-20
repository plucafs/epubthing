use eframe::egui::Color32;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::text_align::TextAlign;
use super::theme::Theme;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub open_last_file: bool,
    pub last_file: Option<String>,
    pub ui_scale: f32,
    pub text_width_ch: f32,
    pub font_size: f32,
    pub font_color: [u8; 4],
    pub bg_color: [u8; 4],
    pub app_theme: Theme,
    pub font_family: String,
    #[serde(default)]
    pub recent_files: Vec<String>,
    #[serde(default)]
    pub text_align: TextAlign,
    #[serde(default)]
    pub show_toc: bool,
    #[serde(default)]
    pub show_conversation: bool,
    #[serde(default = "default_true")]
    pub save_reading_position: bool,
    #[serde(default = "default_true")]
    pub show_chapter_progress: bool,
    #[serde(default = "default_true")]
    pub show_reading_time: bool,
    #[serde(default = "default_true")]
    pub show_minimap: bool,
    #[serde(default = "default_scroll_speed")]
    pub scroll_speed: f32,
    #[serde(default)]
    pub reading_positions: HashMap<String, ReadingPosition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReadingPosition {
    pub chapter: usize,
    pub scroll_offset: f32,
}

fn default_true() -> bool {
    true
}

fn default_scroll_speed() -> f32 {
    10.0
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            open_last_file: true,
            last_file: None,
            ui_scale: 1.40,
            text_width_ch: 70.0,
            font_size: 16.0,
            font_color: [220, 220, 220, 255],
            bg_color: [35, 35, 40, 0],
            app_theme: Theme::Dark,
            font_family: "Inter".into(),
            recent_files: Vec::new(),
            text_align: TextAlign::Center,
            show_toc: true,
            show_conversation: true,
            save_reading_position: true,
            show_chapter_progress: true,
            show_reading_time: true,
            show_minimap: true,
            scroll_speed: default_scroll_speed(),
            reading_positions: HashMap::new(),
        }
    }
}

impl AppConfig {
    pub fn font_color32(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(
            self.font_color[0],
            self.font_color[1],
            self.font_color[2],
            self.font_color[3],
        )
    }

    pub fn bg_color32(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(
            self.bg_color[0],
            self.bg_color[1],
            self.bg_color[2],
            self.bg_color[3],
        )
    }
}
