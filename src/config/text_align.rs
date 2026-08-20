use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl TextAlign {
    #[allow(dead_code)] // Column alignment will be restored with the markdown reader.
    pub fn to_egui(&self) -> egui::Align {
        match self {
            TextAlign::Left => egui::Align::LEFT,
            TextAlign::Center => egui::Align::Center,
            TextAlign::Right => egui::Align::RIGHT,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            TextAlign::Left => "Left",
            TextAlign::Center => "Center",
            TextAlign::Right => "Right",
        }
    }
}
