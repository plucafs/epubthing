use eframe::egui;

use crate::app::fonts;
use crate::config::{AppConfig, TextAlign, Theme};

/// Shows the settings window. Returns whether the window should remain open.
pub fn show(ctx: &egui::Context, show: &mut bool, config: &mut AppConfig, config_dirty: &mut bool) {
    let mut open = *show;
    egui::Window::new("Settings")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([320.0, 500.0])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    ui.heading("General");
                    ui.add_space(4.0);

                    if ui
                        .checkbox(&mut config.open_last_file, "Open last file on startup")
                        .changed()
                    {
                        *config_dirty = true;
                    }

                    if ui
                        .checkbox(&mut config.save_reading_position, "Save reading position")
                        .changed()
                    {
                        *config_dirty = true;
                    }

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("UI Scale:");
                        if ui
                            .add(egui::Slider::new(&mut config.ui_scale, 0.75..=3.0).step_by(0.05))
                            .changed()
                        {
                            *config_dirty = true;
                        }
                        ui.label(format!("{:.2}", config.ui_scale));
                    });

                    ui.separator();

                    ui.heading("App Theme");
                    ui.add_space(4.0);

                    if ui
                        .radio_value(&mut config.app_theme, Theme::Dark, "Dark")
                        .changed()
                    {
                        *config_dirty = true;
                    }
                    if ui
                        .radio_value(&mut config.app_theme, Theme::Light, "Light")
                        .changed()
                    {
                        *config_dirty = true;
                    }

                    ui.separator();

                    ui.heading("Font");
                    ui.add_space(4.0);

                    egui::ComboBox::from_label("Font family")
                        .selected_text(&config.font_family)
                        .show_ui(ui, |ui| {
                            for name in fonts::font_families() {
                                if ui
                                    .selectable_value(
                                        &mut config.font_family,
                                        name.to_owned(),
                                        name,
                                    )
                                    .changed()
                                {
                                    *config_dirty = true;
                                }
                            }
                        });

                    ui.separator();

                    ui.heading("Text Box");
                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.label("Font size:");
                        if ui
                            .add(egui::Slider::new(&mut config.font_size, 10.0..=32.0).step_by(1.0))
                            .changed()
                        {
                            *config_dirty = true;
                        }
                        ui.label(format!("{:.0}", config.font_size));
                    });

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Width (ch):");
                        if ui
                            .add(
                                egui::Slider::new(&mut config.text_width_ch, 30.0..=120.0)
                                    .step_by(5.0),
                            )
                            .changed()
                        {
                            *config_dirty = true;
                        }
                        ui.label(format!("{:.0}ch", config.text_width_ch));
                    });

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Scroll speed:");
                        if ui
                            .add(
                                egui::Slider::new(&mut config.scroll_speed, 5.0..=100.0)
                                    .step_by(5.0),
                            )
                            .changed()
                        {
                            *config_dirty = true;
                        }
                        ui.label(format!("{:.0}", config.scroll_speed));
                    });

                    ui.add_space(4.0);
                    ui.label("Alignment:");
                    ui.add_space(2.0);
                    if ui
                        .radio_value(
                            &mut config.text_align,
                            TextAlign::Left,
                            TextAlign::Left.label(),
                        )
                        .changed()
                    {
                        *config_dirty = true;
                    }
                    if ui
                        .radio_value(
                            &mut config.text_align,
                            TextAlign::Center,
                            TextAlign::Center.label(),
                        )
                        .changed()
                    {
                        *config_dirty = true;
                    }
                    if ui
                        .radio_value(
                            &mut config.text_align,
                            TextAlign::Right,
                            TextAlign::Right.label(),
                        )
                        .changed()
                    {
                        *config_dirty = true;
                    }

                    ui.add_space(4.0);
                    if ui
                        .checkbox(&mut config.show_minimap, "Show scroll minimap")
                        .changed()
                    {
                        *config_dirty = true;
                    }

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Font color:");
                        let mut c = config.font_color32();
                        if ui.color_edit_button_srgba(&mut c).changed() {
                            config.font_color = [c.r(), c.g(), c.b(), c.a()];
                            *config_dirty = true;
                        }
                    });

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Background color:");
                        let mut c = config.bg_color32();
                        if ui.color_edit_button_srgba(&mut c).changed() {
                            config.bg_color = [c.r(), c.g(), c.b(), c.a()];
                            *config_dirty = true;
                        }
                    });

                    ui.add_space(12.0);

                    if ui.button("Default").clicked() {
                        let defaults = AppConfig::default();
                        config.ui_scale = defaults.ui_scale;
                        config.text_width_ch = defaults.text_width_ch;
                        config.font_size = defaults.font_size;
                        config.font_color = defaults.font_color;
                        config.bg_color = defaults.bg_color;
                        config.app_theme = defaults.app_theme;
                        config.font_family = defaults.font_family;
                        config.text_align = defaults.text_align;
                        config.scroll_speed = defaults.scroll_speed;
                        config.save_reading_position = defaults.save_reading_position;
                        config.show_minimap = defaults.show_minimap;
                        *config_dirty = true;
                    }
                });
        });
    if !open {
        *show = false;
    }
}
