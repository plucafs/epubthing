mod app;
mod config;
mod conversation;
mod ui;

use eframe::egui;
use eframe::egui::Color32;
use epubthing::{resolve_path, ContentSegment};

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use app::fonts;
use app::loading::{self as loading_mod, LoadedDocument, LoadingTask};
use app::search::{self as search_mod, ChapterSearchData, SearchState};
use config::{config_path, load_config, save_config, AppConfig, ReadingPosition};
use conversation::{
    export_conversation, import_conversation, load_conversation, save_conversation,
    ConversationMessage,
};

// ─── Config (see config/ module) ───────────────────────────────────────────

// ─── Entry point ───────────────────────────────────────────────────────────

fn main() -> Result<(), eframe::Error> {
    let cli_path = std::env::args().nth(1);
    let config = load_config();

    let startup_path = cli_path.or_else(|| {
        if config.open_last_file {
            config.last_file.clone()
        } else {
            None
        }
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_title("epubthing"),
        ..Default::default()
    };

    eframe::run_native(
        "epubthing",
        options,
        Box::new(move |_cc| {
            // Don't load EPUB here -- defer to first frame so UI shows immediately
            let mut app = EpubApp::new(config);
            app.pending_startup_load = startup_path;
            Ok(Box::new(app))
        }),
    )
}

fn open_directory(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer");

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        return Err(std::io::Error::other(
            "opening folders is unsupported on this platform",
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        command.arg(path).spawn()?;
        Ok(())
    }
}

// ─── App state ─────────────────────────────────────────────────────────────

struct EpubApp {
    document: Option<LoadedDocument>,
    current_chapter: usize,
    current_file_path: Option<String>,
    show_toc: bool,
    show_help: bool,
    show_settings: bool,
    config_dirty: bool,
    error_message: Option<String>,
    status_message: Option<String>,
    status_message_time: Option<std::time::Instant>,
    config: AppConfig,
    textures: HashMap<String, egui::TextureHandle>,
    loaded_font: String,
    font_loaded: bool,
    /// Path to load on first frame (deferred from startup)
    pending_startup_load: Option<String>,
    /// Search filter for the table of contents
    toc_search: String,
    /// Whether the window is in fullscreen mode
    is_fullscreen: bool,
    /// Whether the recent files dialog is open
    show_recent: bool,
    /// Whether the conversation panel is open
    show_conversation: bool,
    /// Search filter for the recent files dialog
    recent_search: String,
    /// Current selection index in the filtered recent files list
    recent_selection: usize,
    // --- Conversation ---
    conversation_messages: Vec<ConversationMessage>,
    conversation_input: String,
    /// false = Author mode (left), true = Me mode (right)
    conversation_me_mode: bool,
    /// Flag to scroll conversation to bottom after sending
    conversation_scroll_to_bottom: bool,
    reset_chapter_scroll: bool,
    chapter_progress_pct: u32,
    remaining_reading_minutes: u64,
    remaining_chapter_minutes: u64,
    search: SearchState,
    search_enter_pressed: bool,
    /// Background loading task
    loading: Option<LoadingTask>,
    /// Cached word count per chapter (indexed same as chapters)
    chapter_word_counts: Vec<usize>,
}

impl EpubApp {
    fn new(config: AppConfig) -> Self {
        Self {
            document: None,
            current_chapter: 0,
            current_file_path: None,
            show_toc: config.show_toc,
            show_conversation: config.show_conversation,
            show_help: false,
            show_settings: false,
            config_dirty: false,
            error_message: None,
            status_message: None,
            status_message_time: None,
            config,
            textures: HashMap::new(),
            loaded_font: String::new(),
            font_loaded: false,
            pending_startup_load: None,
            toc_search: String::new(),
            is_fullscreen: false,
            show_recent: false,
            recent_search: String::new(),
            recent_selection: 0,
            conversation_messages: Vec::new(),
            conversation_input: String::new(),
            conversation_me_mode: true, // Default to Me mode
            conversation_scroll_to_bottom: false,
            reset_chapter_scroll: false,
            chapter_progress_pct: 0,
            remaining_reading_minutes: 0,
            remaining_chapter_minutes: 0,
            search: SearchState::default(),
            search_enter_pressed: false,
            loading: None,
            chapter_word_counts: Vec::new(),
        }
    }

    /// Starts background loading of an EPUB from a file path.
    fn start_loading_from_path(&mut self, path: &str) {
        self.cancel_loading();
        self.document = None;
        self.error_message = None;
        self.textures.clear();

        let path_owned = path.to_string();
        self.current_file_path = Some(path_owned.clone());
        self.config.last_file = Some(path_owned.clone());
        save_config(&self.config);

        let cancel = Arc::new(AtomicBool::new(false));
        self.loading = Some(loading_mod::start_loading_from_path(path, cancel));
    }

    /// Starts background loading of an EPUB from bytes (e.g. drag-and-drop).
    fn start_loading_from_bytes(&mut self, name: &str, bytes: Vec<u8>) {
        self.cancel_loading();
        self.document = None;
        self.error_message = None;
        self.textures.clear();
        self.current_file_path = None;

        let cancel = Arc::new(AtomicBool::new(false));
        self.loading = Some(loading_mod::start_loading_from_bytes(name, bytes, cancel));
    }

    /// Cancels the current background load, if any.
    fn cancel_loading(&mut self) {
        if let Some(ref loading) = self.loading {
            loading.cancel();
        }
        self.loading = None;
    }

    /// Adds a file path to the front of the recent files list (deduplicated, max 20).
    fn add_to_recent_files(&mut self, path: &str) {
        self.config.recent_files.retain(|p| p != path);
        self.config.recent_files.insert(0, path.to_string());
        self.config.recent_files.truncate(20);
        save_config(&self.config);
    }

    /// Removes a path from the recent files list.
    fn remove_from_recent_files(&mut self, path: &str) {
        self.config.recent_files.retain(|p| p != path);
        save_config(&self.config);
    }

    /// Sets the current chapter and saves the reading position.
    fn set_chapter(&mut self, chapter: usize) {
        if self.current_chapter != chapter {
            self.reset_chapter_scroll = true;
        }
        self.current_chapter = chapter;
        self.save_reading_position();
    }

    /// Saves the current reading position for the current book.
    fn save_reading_position(&mut self) {
        if !self.config.save_reading_position {
            return;
        }
        if let Some(ref path) = self.current_file_path.clone() {
            let pos = ReadingPosition {
                chapter: self.current_chapter,
                scroll_offset: 0.0, // Scroll offset will be updated by the UI
            };
            self.config.reading_positions.insert(path.clone(), pos);
            save_config(&self.config);
            self.status_message = Some("Position saved".into());
            self.status_message_time = Some(std::time::Instant::now());
        }
    }

    /// Checks if a background load has completed and applies the result.
    /// Handles multiple receives for incremental loading: the first message
    /// contains only chapter 0, and a second message replaces it with the
    /// full document once all chapters are parsed.
    fn poll_loading(&mut self) {
        if self.loading.is_none() {
            return;
        }

        let mut got_first = false;
        let mut first_path: Option<String> = None;
        let mut first_chapter_count: usize = 0;

        loop {
            let result = self.loading.as_ref().unwrap().receiver.try_recv();
            match result {
                Ok(Ok(doc)) => {
                    if !got_first {
                        got_first = true;
                        first_path = self.current_file_path.clone();
                        first_chapter_count = doc.chapters.len();
                    }
                    self.document = Some(doc);
                    self.chapter_word_counts = self
                        .document
                        .as_ref()
                        .unwrap()
                        .chapters
                        .iter()
                        .map(|ch| search_mod::word_count(&ch.segments))
                        .collect();
                }
                Ok(Err(e)) => {
                    if e != "Cancelled" {
                        self.error_message = Some(format!("Error: {}", e));
                    }
                    self.loading = None;
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.loading = None;
                    break;
                }
            }
        }

        if got_first {
            if let Some(ref path) = first_path {
                self.add_to_recent_files(path);
                self.conversation_messages = load_conversation(path);
                if self.config.save_reading_position {
                    if let Some(pos) = self.config.reading_positions.get(path) {
                        self.current_chapter =
                            pos.chapter.min(first_chapter_count.saturating_sub(1));
                    } else {
                        self.current_chapter = 0;
                    }
                } else {
                    self.current_chapter = 0;
                }
            } else {
                self.current_chapter = 0;
            }
            self.status_message = None;
        }
    }

    fn restart_app(&self) {
        if let Ok(exe) = std::env::current_exe() {
            let mut cmd = std::process::Command::new(exe);
            if let Some(ref path) = self.current_file_path {
                cmd.arg(path);
            }
            let _ = cmd.spawn();
        }
        std::process::exit(0);
    }

    fn render_chapter_content(
        ui: &mut egui::Ui,
        doc: &LoadedDocument,
        index: usize,
        color: Color32,
        font_size: f32,
        textures: &mut HashMap<String, egui::TextureHandle>,
        highlighted_segments: Option<&Vec<ContentSegment>>,
        chapter_to_set: &mut Option<usize>,
        search_need_scroll: &mut bool,
        font_loaded: bool,
    ) {
        let chapter = match doc.chapters.get(index) {
            Some(c) => c,
            None => return,
        };

        let segments_to_render = highlighted_segments.unwrap_or(&chapter.segments);

        for segment in segments_to_render {
            match segment {
                ContentSegment::StyledText(spans) => {
                    ui.horizontal_wrapped(|ui| {
                        for span in spans {
                            if span.text == "\n" {
                                ui.end_row();
                                ui.add_space(4.0);
                                continue;
                            }

                            let heading_size = match span.heading_level {
                                1 => font_size * 1.5,
                                2 => font_size * 1.25,
                                3 => font_size * 1.1,
                                _ => font_size,
                            };

                            let mut rich = egui::RichText::new(&span.text);

                            if span.bold || span.heading_level > 0 {
                                rich = rich.strong();
                            }
                            if span.italic {
                                rich = rich.italics();
                            }
                            if span.underline {
                                rich = rich.underline();
                            }
                            if let Some([r, g, b, a]) = span.color {
                                rich = rich.color(Color32::from_rgba_unmultiplied(r, g, b, a));
                            } else if span.link_url.is_some() {
                                rich = rich.color(ui.visuals().hyperlink_color);
                            } else if span.heading_level == 0 {
                                rich = rich.color(color);
                            }

                            let font_family = if font_loaded {
                                fonts::reader_font_family()
                            } else {
                                egui::FontFamily::Proportional
                            };
                            rich = rich.font(egui::FontId::new(heading_size, font_family));

                            let response =
                                ui.add(egui::Label::new(rich).sense(if span.link_url.is_some() {
                                    egui::Sense::click()
                                } else {
                                    egui::Sense::hover()
                                }));
                            if *search_need_scroll
                                && span.color == Some(search_mod::ACTIVE_MATCH_COLOR)
                            {
                                response.scroll_to_me(Some(egui::Align::Center));
                                *search_need_scroll = false;
                            }
                            if response.clicked() {
                                if let Some(link) = &span.link_url {
                                    if link.starts_with("http://")
                                        || link.starts_with("https://")
                                        || link.starts_with("mailto:")
                                    {
                                        ui.ctx().open_url(egui::OpenUrl::new_tab(link));
                                    } else {
                                        let current_href = &chapter.href;
                                        let base = current_href
                                            .rsplit_once('/')
                                            .map_or("", |(directory, _)| directory);
                                        let target = resolve_path(base, link);
                                        if let Some(target_index) =
                                            doc.chapters.iter().position(|candidate| {
                                                resolve_path("", &candidate.href) == target
                                            })
                                        {
                                            *chapter_to_set = Some(target_index);
                                        }
                                    }
                                }
                            }
                        }
                    });
                }
                ContentSegment::Image { href } => {
                    if let Some(bytes) = chapter.image_data.get(href) {
                        if !textures.contains_key(href) {
                            let img = match image::load_from_memory(bytes) {
                                Ok(img) => img,
                                Err(error) => {
                                    ui.colored_label(
                                        ui.visuals().error_fg_color,
                                        format!("[Image decode failed: {}]", error),
                                    );
                                    continue;
                                }
                            };
                            let rgba = img.to_rgba8();
                            let size = [rgba.width() as usize, rgba.height() as usize];
                            let pixels: Vec<egui::Color32> = rgba
                                .pixels()
                                .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
                                .collect();
                            let color_img = egui::ColorImage { size, pixels };
                            let handle = ui.ctx().load_texture(
                                href.clone(),
                                color_img,
                                egui::TextureOptions::default(),
                            );
                            textures.insert(href.clone(), handle);
                        }

                        ui.add_space(6.0);
                        if let Some(handle) = textures.get(href) {
                            let available = ui.available_width();
                            ui.add(egui::Image::new(handle).max_width(available));
                        }
                        ui.add_space(4.0);
                    } else if let Some(error) = chapter.image_errors.get(href) {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            format!("[{}: {}]", error, href),
                        );
                    }
                }
            }
        }
    }
}

// ─── GUI ───────────────────────────────────────────────────────────────────

impl eframe::App for EpubApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply theme and scale
        ctx.set_pixels_per_point(self.config.ui_scale);
        ctx.set_visuals(self.config.app_theme.visuals());

        // Load font if changed
        if self.loaded_font != self.config.font_family {
            self.font_loaded = fonts::load_font(ctx, &self.config.font_family);
            if !self.font_loaded {
                eprintln!("Warning: Font '{}' not found, using default font", self.config.font_family);
            }
            self.loaded_font = self.config.font_family.clone();
        }

        // --- Deferred startup load: fire AFTER first frame is shown ---
        if let Some(path) = self.pending_startup_load.take() {
            self.start_loading_from_path(&path);
        }

        // --- Poll background loading ---
        let prev_chapter = self.current_chapter;
        self.poll_loading();
        if self.current_chapter != prev_chapter {
            self.search.dirty = true;
        }

        // --- Recompute search if needed ---
        if self.search.show && self.search.dirty {
            if let Some(ref doc) = self.document {
                if let Some(chapter) = doc.chapters.get(self.current_chapter) {
                    let ch_data = ChapterSearchData {
                        segments: &chapter.segments,
                    };
                    search_mod::recompute(&mut self.search, &ch_data);
                }
            } else {
                self.search.dirty = false;
            }
        }

        // If a load is in progress, keep repainting so we poll the channel
        if self.loading.is_some() {
            ctx.request_repaint();
        }

        // Keyboard input
        let chapter_before_input = self.current_chapter;

        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Q)) {
            ctx.send_viewport_cmd(egui::viewport::ViewportCommand::Close);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            self.is_fullscreen = !self.is_fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.is_fullscreen))
        };
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Escape) {
                self.show_help = false;
                self.show_settings = false;
                self.search.close();
            }
            // if i.key_pressed(egui::Key::ArrowLeft) && self.current_chapter > 0 {
            //     self.current_chapter -= 1;
            // }
            // if i.key_pressed(egui::Key::ArrowRight)
            //     && self
            //         .document
            //         .as_ref()
            //         .map_or(false, |d| self.current_chapter + 1 < d.chapters.len())
            // {
            //     self.current_chapter += 1;
            // }
            // Ctrl+,: open settings
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Comma) {
                self.show_settings = !self.show_settings;
            }
            // Ctrl+F: open search bar
            if i.modifiers.ctrl && i.key_pressed(egui::Key::F) {
                self.search.show = true;
                if self.search.show {
                    if let Some(ref doc) = self.document {
                        if let Some(chapter) = doc.chapters.get(self.current_chapter) {
                            let ch_data = ChapterSearchData {
                                segments: &chapter.segments,
                            };
                            search_mod::recompute(&mut self.search, &ch_data);
                        }
                    }
                } else {
                    self.search.clear();
                }
            }
            // Search navigation
            if self.search.show {
                let advance = |slf: &mut Self| {
                    if let Some(ref doc) = slf.document {
                        if let Some(chapter) = doc.chapters.get(slf.current_chapter) {
                            let ch_data = ChapterSearchData {
                                segments: &chapter.segments,
                            };
                            slf.search.next_match(&ch_data);
                        }
                    }
                };
                let go_back = |slf: &mut Self| {
                    if let Some(ref doc) = slf.document {
                        if let Some(chapter) = doc.chapters.get(slf.current_chapter) {
                            let ch_data = ChapterSearchData {
                                segments: &chapter.segments,
                            };
                            slf.search.prev_match(&ch_data);
                        }
                    }
                };
                // F3: next match, Shift+F3: previous match
                if i.key_pressed(egui::Key::F3) {
                    if i.modifiers.shift {
                        go_back(self);
                    } else {
                        advance(self);
                    }
                }
                // Enter: captured here before TextEdit consumes it
                if i.key_pressed(egui::Key::Enter) {
                    self.search_enter_pressed = true;
                }

            }
            // Ctrl+R: open recent files
            if i.modifiers.ctrl && i.key_pressed(egui::Key::R) {
                self.show_recent = !self.show_recent;
                self.recent_search.clear();
                self.recent_selection = 0;
            }
            // Ctrl+Shift+C: toggle conversation panel
            if i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::C) {
                self.show_conversation = !self.show_conversation;
                self.config.show_conversation = self.show_conversation;
                self.config_dirty = true;
            }
            // Ctrl+O: open file
            if i.modifiers.ctrl && i.key_pressed(egui::Key::O) {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("EPUB", &["epub"])
                    .pick_file()
                {
                    self.start_loading_from_path(&path.to_string_lossy());
                }
            }
            // Recent files navigation
            if self.show_recent {
                if i.key_pressed(egui::Key::ArrowDown) {
                    self.recent_selection = self.recent_selection.saturating_add(1);
                }
                if i.key_pressed(egui::Key::ArrowUp) {
                    self.recent_selection = self.recent_selection.saturating_sub(1);
                }
            }
        });

        // Save reading position if chapter changed via keyboard
        if self.current_chapter != chapter_before_input {
            self.reset_chapter_scroll = true;
            self.save_reading_position();
        }

        // Drag & Drop
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            for file in dropped.iter() {
                let is_epub = file
                    .path
                    .as_ref()
                    .map(|p| {
                        p.extension()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_lowercase()
                            == "epub"
                    })
                    .unwrap_or(false)
                    || file.name.to_lowercase().ends_with(".epub");

                if !is_epub {
                    continue;
                }

                if let Some(path) = &file.path {
                    self.start_loading_from_path(&path.to_string_lossy());
                } else if let Some(bytes) = &file.bytes {
                    self.start_loading_from_bytes(&file.name, bytes.to_vec());
                }
            }
        }

        // ─── Menubar ──────────────────────────────────────────────
        egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open...").clicked() {
                        ui.close_menu();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("EPUB", &["epub"])
                            .pick_file()
                        {
                            self.start_loading_from_path(&path.to_string_lossy());
                        }
                    }
                    if ui.button("Open Directory...").clicked() {
                        ui.close_menu();
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            if let Ok(entries) = std::fs::read_dir(&dir) {
                                for entry in entries.flatten() {
                                    let path = entry.path();
                                    if path.extension().map_or(false, |e| e == "epub") {
                                        self.start_loading_from_path(&path.to_string_lossy());
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    ui.separator();
                    if ui.button("Recent Files").clicked() {
                        ui.close_menu();
                        self.show_recent = true;
                        self.recent_search.clear();
                    }
                    if ui.button("Settings").clicked() {
                        ui.close_menu();
                        self.show_settings = true;
                    }
                    if ui.button("Open settings folder...").clicked() {
                        ui.close_menu();
                        if let Some(settings_dir) = config_path().parent() {
                            let result = std::fs::create_dir_all(settings_dir)
                                .and_then(|_| open_directory(settings_dir));
                            if let Err(error) = result {
                                self.status_message =
                                    Some(format!("Could not open settings folder: {}", error));
                                self.status_message_time = Some(std::time::Instant::now());
                            }
                        }
                    }
                    ui.separator();
                    if ui.button("Restart").clicked() {
                        self.restart_app();
                    }
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::viewport::ViewportCommand::Close);
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui.button("Fullscreen").clicked() {
                        ui.close_menu();
                        self.is_fullscreen = !self.is_fullscreen;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                            self.is_fullscreen,
                        ));
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("Help").clicked() {
                        ui.close_menu();
                        self.show_help = true;
                    }
                });
            });
        });

        // ─── Book info bar ────────────────────────────────────────
        egui::TopBottomPanel::top("book_info").show(ctx, |ui| {
            if let Some(doc) = &self.document {
                ui.add(
                    egui::Label::new(format!("{} -- {}", doc.title, doc.author)).selectable(false),
                );
            } else if self.loading.is_some() {
                ui.add(egui::Label::new("Loading...").selectable(false));
            } else {
                ui.add(egui::Label::new("File > Open...").selectable(false));
            }
        });

        // ─── Status bar ──────────────────────────────────────────
        // Clear status message after 2 seconds
        if let Some(time) = self.status_message_time {
            if time.elapsed() > std::time::Duration::from_secs(2) {
                self.status_message = None;
                self.status_message_time = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            if let Some(msg) = &self.status_message {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(msg.as_str()).selectable(false));
                });
            }
        });

        // ─── Settings window ───────────────────────────────────────
        ui::settings::show(
            ctx,
            &mut self.show_settings,
            &mut self.config,
            &mut self.config_dirty,
        );

        // Save config as soon as any change is made
        if self.config_dirty {
            save_config(&self.config);
            self.config_dirty = false;
        }

        // ─── Help dialog ──────────────────────────────────────────
        ui::help::show(ctx, &mut self.show_help);

        // ─── Recent files dialog ───────────────────────────────
        let mut show_recent = self.show_recent;
        egui::Window::new("Recent Files")
            .open(&mut show_recent)
            .collapsible(false)
            .resizable(true)
            .default_size([500.0, 400.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let _search_response = ui.add(
                    egui::TextEdit::singleline(&mut self.recent_search)
                        .hint_text("Search recent files...")
                        .desired_width(ui.available_width()),
                );
                ui.separator();

                let query = self.recent_search.to_lowercase();
                let filtered: Vec<usize> = self
                    .config
                    .recent_files
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| {
                        if query.is_empty() {
                            return true;
                        }
                        let name = std::path::Path::new(p)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_lowercase())
                            .unwrap_or_default();
                        name.contains(&query)
                    })
                    .map(|(i, _)| i)
                    .collect();

                // Clamp selection to valid range
                if self.recent_selection >= filtered.len() {
                    self.recent_selection = filtered.len().saturating_sub(1);
                }

                // Enter key opens the selected file (works even without search focus)
                let enter_pressed = ctx.input(|i| i.key_pressed(egui::Key::Enter));
                let mut to_remove: Option<usize> = None;
                let mut to_open: Option<String> = None;
                if enter_pressed && !filtered.is_empty() {
                    let idx = filtered[self.recent_selection];
                    to_open = Some(self.config.recent_files[idx].clone());
                }

                egui::ScrollArea::vertical().show(ui, |ui| {
                    if filtered.is_empty() {
                        if self.config.recent_files.is_empty() {
                            ui.label("No recent files.");
                        } else {
                            ui.label("No matches.");
                        }
                    }
                    for (list_i, &idx) in filtered.iter().enumerate() {
                        let path = &self.config.recent_files[idx];
                        let name = std::path::Path::new(path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.clone());
                        let selected = list_i == self.recent_selection;
                        ui.horizontal(|ui| {
                            let response = ui.add_sized(
                                [ui.available_width() - 30.0, 0.0],
                                egui::Button::new(egui::RichText::new(&name))
                                    .fill(if selected {
                                        ui.visuals().selection.bg_fill
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .stroke(if selected {
                                        egui::Stroke::new(
                                            1.0_f32,
                                            ui.visuals().selection.stroke.color,
                                        )
                                    } else {
                                        egui::Stroke::NONE
                                    }),
                            );
                            if response.clicked() {
                                to_open = Some(path.clone());
                            }
                            if selected {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            if ui.small_button("X").clicked() {
                                to_remove = Some(idx);
                            }
                        });
                    }
                });

                if let Some(idx) = to_remove {
                    let path = self.config.recent_files[idx].clone();
                    self.remove_from_recent_files(&path);
                }
                if let Some(path) = to_open {
                    self.show_recent = false;
                    self.recent_search.clear();
                    self.recent_selection = 0;
                    self.start_loading_from_path(&path);
                }
            });
        if !show_recent {
            self.show_recent = false;
        }

        // ─── Main content ─────────────────────────────────────────
        let mut chapter_to_set: Option<usize> = None;
        if let Some(doc) = &self.document {
            if self.show_toc {
                egui::SidePanel::left("toc_panel")
                    .resizable(true)
                    .default_width(220.0)
                    .show(ctx, |ui| {
                        ui.heading("Contents");
                        ui.add_space(4.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.toc_search)
                                .hint_text("Search chapters...")
                                .desired_width(ui.available_width()),
                        );
                        ui.separator();
                        let query = self.toc_search.to_lowercase();
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for (i, chapter) in doc.chapters.iter().enumerate() {
                                if !query.is_empty()
                                    && !chapter.label.to_lowercase().contains(&query)
                                {
                                    continue;
                                }
                                let selected = i == self.current_chapter;
                                let response = ui.add_sized(
                                    [ui.available_width(), 0.0],
                                    egui::Button::new(egui::RichText::new(&chapter.label))
                                        .fill(if selected {
                                            ui.visuals().selection.bg_fill
                                        } else {
                                            Color32::TRANSPARENT
                                        })
                                        .stroke(egui::Stroke::NONE)
                                        .wrap_mode(egui::TextWrapMode::Truncate),
                                );
                                if response.clicked() {
                                    chapter_to_set = Some(i);
                                }
                                response.on_hover_text(&chapter.label);
                            }
                            if query.is_empty() && doc.chapters.is_empty() {
                                ui.label("No chapters found.");
                            } else if !query.is_empty()
                                && !doc
                                    .chapters
                                    .iter()
                                    .any(|chapter| chapter.label.to_lowercase().contains(&query))
                            {
                                ui.label("No matches.");
                            }
                        });
                    });
            }

            // ─── Conversation panel ─────────────────────────────────
            if self.show_conversation {
                egui::SidePanel::right("conversation_panel")
                    .resizable(true)
                    .default_width(300.0)
                    .max_width(500.0)
                    .show(ctx, |ui| {
                        // Header with title and buttons
                        ui.horizontal(|ui| {
                            ui.heading("Conversation");
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Import").clicked() {
                                    if let Some(ref path) = self.current_file_path {
                                        if let Some(import_path) = rfd::FileDialog::new()
                                            .add_filter("JSON", &["json"])
                                            .pick_file()
                                        {
                                            if let Ok(msgs) = import_conversation(path, &import_path.to_string_lossy()) {
                                                self.conversation_messages = msgs;
                                            }
                                        }
                                    }
                                }
                                if ui.small_button("Export").clicked() {
                                    if self.current_file_path.is_some() {
                                        if let Some(export_path) = rfd::FileDialog::new()
                                            .add_filter("JSON", &["json"])
                                            .save_file()
                                        {
                                            let _ = export_conversation(&self.conversation_messages, &export_path.to_string_lossy());
                                        }
                                    }
                                }
                            });
                        });
                        ui.separator();
                        let mut to_delete: Option<String> = None;
                        let mut to_navigate: Option<usize> = None;

                        // Input area stuck at bottom using TopBottomPanel
                        egui::TopBottomPanel::bottom("conversation_input_panel")
                            .show_inside(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let mode_text = if self.conversation_me_mode { "Me" } else { "Author" };
                                    let mode_button_clicked = ui.button(mode_text).clicked();
                                    if mode_button_clicked {
                                        self.conversation_me_mode = !self.conversation_me_mode;
                                    }

                                    let input_response = ui.add(
                                        egui::TextEdit::multiline(&mut self.conversation_input)
                                            .hint_text("Write a note... (Enter to send, Shift+Enter for new line)")
                                            .desired_width(ui.available_width())
                                            .desired_rows(3),
                                    );
                                    if mode_button_clicked {
                                        input_response.request_focus();
                                    }
                                    let enter_send = ctx.input(|i| i.key_pressed(egui::Key::Enter))
                                        && !ctx.input(|i| i.modifiers.shift)
                                        && input_response.has_focus();
                                    if enter_send && !self.conversation_input.trim().is_empty() {
                                        let msg = if self.conversation_me_mode {
                                            ConversationMessage::new_me(
                                                self.conversation_input.trim().to_string(),
                                                self.current_chapter,
                                            )
                                        } else {
                                            ConversationMessage::new_author(
                                                self.conversation_input.trim().to_string(),
                                                self.current_chapter,
                                            )
                                        };
                                        self.conversation_messages.push(msg);
                                        if let Some(ref path) = self.current_file_path {
                                            save_conversation(path, &self.conversation_messages);
                                        }
                                        self.conversation_input.clear();
                                        self.conversation_scroll_to_bottom = true;
                                        ui.ctx().request_repaint();
                                    }
                                });
                            });

                        // Messages area takes remaining space
                        egui::ScrollArea::vertical()
                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                            .show(ui, |ui| {
                                if self.conversation_messages.is_empty() {
                                    ui.label("No messages yet.");
                                }
                                for msg in &self.conversation_messages {
                                    let text = msg.text.clone();
                                    let bubble_color = if msg.is_highlight {
                                        ui.visuals().extreme_bg_color
                                    } else if msg.is_me_message {
                                        ui.visuals().selection.bg_fill
                                    } else {
                                        ui.visuals().extreme_bg_color
                                    };
                                    let text_color = if msg.is_me_message {
                                        Some(egui::Color32::from_rgb(255, 255, 255))
                                    } else {
                                        None
                                    };
                                    // Keep the message actions on the same row as the bubble.
                                    let available = ui.available_width();
                                    let has_go_button = msg.is_highlight && !msg.is_me_message;
                                    let controls_width = if has_go_button { 64.0 } else { 28.0 };
                                    let bubble_width = (available - controls_width - 4.0).max(40.0);
                                    let row_layout = if msg.is_me_message || msg.is_highlight {
                                        egui::Layout::right_to_left(egui::Align::Center)
                                    } else {
                                        egui::Layout::left_to_right(egui::Align::Center)
                                    };
                                    let show_message = |ui: &mut egui::Ui| {
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(bubble_width, 0.0),
                                            egui::Layout::top_down(egui::Align::Min),
                                            |ui| {
                                                let frame = egui::Frame::NONE
                                                    .fill(bubble_color)
                                                    .corner_radius(egui::CornerRadius::same(6))
                                                    .inner_margin(egui::Margin::symmetric(8, 4));
                                                frame.show(ui, |ui| {
                                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                                    if let Some(color) = text_color {
                                                        ui.add(egui::Label::new(egui::RichText::new(&text).color(color)).wrap());
                                                    } else {
                                                        ui.add(egui::Label::new(&text).wrap());
                                                    }
                                                    if let Some(timestamp) = msg.formatted_timestamp() {
                                                        ui.add_space(2.0);
                                                        ui.label(
                                                            egui::RichText::new(timestamp).small().color(
                                                                ui.visuals().widgets.inactive.fg_stroke.color,
                                                            ),
                                                        );
                                                    }
                                                });
                                            },
                                        );
                                    };
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(available, 0.0),
                                        row_layout,
                                        |ui| {
                                            let right_aligned = msg.is_me_message || msg.is_highlight;
                                            if !right_aligned {
                                                show_message(ui);
                                            }
                                            if ui.small_button("X").clicked() {
                                                to_delete = Some(msg.id.clone());
                                            }
                                    // GO button only for actual book highlights.
                                            if has_go_button && ui.button("Go").clicked() {
                                                to_navigate = Some(msg.chapter_idx);
                                            }
                                            if right_aligned {
                                                show_message(ui);
                                            }
                                        },
                                    );
                                    ui.add_space(4.0);
                                }
                                // Scroll to bottom if flag is set
                                if self.conversation_scroll_to_bottom {
                                    ui.scroll_to_cursor(Some(egui::Align::Max));
                                    self.conversation_scroll_to_bottom = false;
                                }
                            });
                        if let Some(id) = to_delete {
                            self.conversation_messages.retain(|m| m.id != id);
                            if let Some(ref path) = self.current_file_path {
                                save_conversation(path, &self.conversation_messages);
                            }
                        }
                        if let Some(chapter_idx) = to_navigate {
                            if self.document.as_ref().map_or(false, |d| chapter_idx < d.chapters.len()) {
                                chapter_to_set = Some(chapter_idx);
                            }
                        }
                    });
            }

            egui::CentralPanel::default().show(ctx, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    // Toolbar
                    ui.horizontal(|ui| {
                        if ui.selectable_label(self.show_toc, "Contents").clicked() {
                            self.show_toc = !self.show_toc;
                            self.config.show_toc = self.show_toc;
                            self.config_dirty = true;
                        }
                        ui.separator();
                        if ui
                            .selectable_label(self.show_conversation, "Conversation")
                            .clicked()
                        {
                            self.show_conversation = !self.show_conversation;
                            self.config.show_conversation = self.show_conversation;
                            self.config_dirty = true;
                        }
                        ui.separator();
                        if ui.button("< Prev").clicked() && self.current_chapter > 0 {
                            chapter_to_set = Some(self.current_chapter - 1);
                        }
                        ui.label(format!(
                            "{}/{}",
                            self.current_chapter + 1,
                            doc.chapters.len()
                        ));
                        if ui.button("Next >").clicked()
                            && self.current_chapter + 1 < doc.chapters.len()
                        {
                            chapter_to_set = Some(self.current_chapter + 1);
                        }
                    });

                    ui.separator();

                    if let Some(chapter) = doc.chapters.get(self.current_chapter) {
                        ui.add(egui::Label::new(&chapter.label).selectable(false));
                        ui.separator();
                    }

                    // ─── Search bar ──────────────────────────────────────────
                    if self.search.show {
                        ui.horizontal(|ui| {
                            let search_response = ui.add(
                                egui::TextEdit::singleline(&mut self.search.query)
                                    .hint_text("Find in chapter...")
                                    .desired_width(200.0),
                            );
                            search_response.request_focus();

                            // Detect search query change
                            if self.search.query != self.search.last_query {
                                self.search.dirty = true;
                                self.search.last_query = self.search.query.clone();
                            }

                            // Handle Enter: flag set in keyboard handler before TextEdit consumed it
                            if search_response.lost_focus()
                                && self.search_enter_pressed
                            {
                                self.search_enter_pressed = false;
                                if let Some(ref doc) = self.document {
                                    if let Some(chapter) =
                                        doc.chapters.get(self.current_chapter)
                                    {
                                        let ch_data = ChapterSearchData {
                                            segments: &chapter.segments,
                                        };
                                        if ui.input(|i| i.modifiers.shift) {
                                            self.search.prev_match(&ch_data);
                                        } else {
                                            self.search.next_match(&ch_data);
                                        }
                                    }
                                }
                            }

                            let match_count = self.search.matches.len();
                            if !self.search.query.is_empty() {
                                if match_count > 0 {
                                    ui.label(format!(
                                        "{}/{}",
                                        self.search.active + 1,
                                        match_count
                                    ));
                                } else {
                                    ui.label("0/0");
                                }
                            }

                            ui.separator();

                            if ui.button("^").on_hover_text("Previous match").clicked() {
                                if let Some(ref doc) = self.document {
                                    if let Some(chapter) =
                                        doc.chapters.get(self.current_chapter)
                                    {
                                        let ch_data = ChapterSearchData {
                                            segments: &chapter.segments,
                                        };
                                        self.search.prev_match(&ch_data);
                                    }
                                }
                            }
                            if ui.button("v").on_hover_text("Next match").clicked() {
                                if let Some(ref doc) = self.document {
                                    if let Some(chapter) =
                                        doc.chapters.get(self.current_chapter)
                                    {
                                        let ch_data = ChapterSearchData {
                                            segments: &chapter.segments,
                                        };
                                        self.search.next_match(&ch_data);
                                    }
                                }
                            }

                            ui.separator();

                            if ui
                                .checkbox(&mut self.search.highlight_all, "Highlight all")
                                .changed()
                            {
                                if let Some(ref doc) = self.document {
                                    if let Some(chapter) =
                                        doc.chapters.get(self.current_chapter)
                                    {
                                        let ch_data = ChapterSearchData {
                                            segments: &chapter.segments,
                                        };
                                        self.search.refresh_highlights(&ch_data);
                                    }
                                }
                            }
                            if ui
                                .checkbox(&mut self.search.match_case, "Match case")
                                .changed()
                            {
                                self.search.dirty = true;
                            }
                            if ui
                                .checkbox(&mut self.search.whole_words, "Whole words")
                                .changed()
                            {
                                self.search.dirty = true;
                            }

                            if ui.button("X").on_hover_text("Close search").clicked() {
                                self.search.close();
                            }
                        });
                        ui.separator();
                    }

                    // Text box with custom colors
                    let font_color = self.config.font_color32();
                    let bg_color = self.config.bg_color32();
                    let ch_width = self.config.text_width_ch;

                    let font_size = self.config.font_size;
                    let approx_char_width = font_size * 0.55;
                    let desired_width = ch_width * approx_char_width;
                    let available = ui.available_width();
                    let panel_width = available.max(0.0);
                    let text_box_width = desired_width.min((panel_width - 32.0).max(0.0));

                    let text_align = self.config.text_align.to_egui();
                    if self.config.show_chapter_progress || self.config.show_reading_time {
                        egui::TopBottomPanel::bottom("reading_status").show_inside(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if self.config.show_reading_time
                                            && self.remaining_reading_minutes > 0
                                        {
                                            ui.add(
                                                egui::Label::new(format!(
                                                    "Book: {}",
                                                    search_mod::format_reading_time(
                                                        self.remaining_reading_minutes
                                                    )
                                                ))
                                                .selectable(false),
                                            );
                                            ui.separator();
                                            ui.add(
                                                egui::Label::new(format!(
                                                    "Ch: {}",
                                                    search_mod::format_reading_time(
                                                        self.remaining_chapter_minutes
                                                    )
                                                ))
                                                .selectable(false),
                                            );
                                        }
                                        if self.config.show_chapter_progress {
                                            if self.config.show_reading_time
                                                && self.remaining_reading_minutes > 0
                                            {
                                                ui.separator();
                                            }
                                            ui.add(
                                                egui::Label::new(format!(
                                                    "Chapter progress: {}%",
                                                    self.chapter_progress_pct
                                                ))
                                                .selectable(false),
                                            );
                                        }
                                    },
                                );
                            });
                        });
                    }

                    let mut chapter_scroll = egui::ScrollArea::vertical()
                        .id_salt("chapter_text")
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                        );

                    if self.reset_chapter_scroll {
                        chapter_scroll = chapter_scroll.scroll_offset(egui::Vec2::ZERO);
                        self.reset_chapter_scroll = false;
                    }

                    {
                        let scale = self.config.scroll_speed / 50.0;
                        let ctx = ui.ctx().clone();
                        ctx.input_mut(|i| {
                            i.smooth_scroll_delta.y *= scale;
                        });
                    }

                    let scroll_output = chapter_scroll.show(ui, |ui| {
                        ui.add_space(8.0);

                        ui.with_layout(egui::Layout::top_down(text_align), |ui| {
                            ui.set_width(panel_width);
                            ui.set_min_width(panel_width);
                            ui.set_max_width(panel_width);
                            let frame_response = ui
                                .allocate_ui_with_layout(
                                    egui::vec2(text_box_width, 0.0),
                                    egui::Layout::top_down(text_align),
                                    |ui| {
                                        egui::Frame::NONE
                                            .fill(bg_color)
                                            .inner_margin(egui::Margin::same(16))
                                            .corner_radius(egui::CornerRadius::same(4))
                                            .show(ui, |ui| {
                                                EpubApp::render_chapter_content(
                                                    ui,
                                                    doc,
                                                    self.current_chapter,
                                                    font_color,
                                                    font_size,
                                                    &mut self.textures,
                                                    self.search.highlighted_segments.as_ref(),
                                                    &mut chapter_to_set,
                                                    &mut self.search.need_scroll,
                                                    self.font_loaded,
                                                );
                                            })
                                    },
                                )
                                .inner;
                            frame_response.response.context_menu(|ui| {
                                if ui.button("Copy all text").clicked() {
                                    let flat = search_mod::build_flat_text(
                                        &doc.chapters[self.current_chapter].segments,
                                    );
                                    ui.ctx().copy_text(flat);
                                    ui.close_menu();
                                }
                            });
                        });

                        ui.add_space(8.0);
                    });

                    let content_height = scroll_output.content_size.y;
                    let viewport_height = scroll_output.inner_rect.height();
                    self.chapter_progress_pct =
                        if content_height <= viewport_height || content_height <= f32::EPSILON {
                            100
                        } else {
                            (((scroll_output.state.offset.y + viewport_height) / content_height)
                                * 100.0)
                                .round()
                                .clamp(0.0, 100.0) as u32
                        };

                    let remaining_words: usize = if self.current_chapter
                        < self.chapter_word_counts.len()
                    {
                        let current_chapter_words = self.chapter_word_counts[self.current_chapter];
                        let words_done_in_chapter = (current_chapter_words as f64
                            * (self.chapter_progress_pct as f64 / 100.0))
                            as usize;
                        let words_left_in_chapter =
                            current_chapter_words.saturating_sub(words_done_in_chapter);
                        let words_in_future_chapters: usize = self
                            .chapter_word_counts
                            .get(self.current_chapter + 1..)
                            .unwrap_or_default()
                            .iter()
                            .sum();
                        self.remaining_chapter_minutes = (words_left_in_chapter as f64
                            / search_mod::WORDS_PER_MINUTE)
                            .ceil() as u64;
                        words_left_in_chapter + words_in_future_chapters
                    } else {
                        self.remaining_chapter_minutes = 0;
                        0
                    };
                    self.remaining_reading_minutes =
                        (remaining_words as f64 / search_mod::WORDS_PER_MINUTE).ceil() as u64;
                });
            });
        } else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(180.0);

                    if let Some(ref loading) = self.loading {
                        ui.heading("Loading EPUB");
                        ui.add_space(10.0);
                        ui.label(&loading.status);
                        ui.add_space(16.0);
                        if ui.button("Cancel").clicked() {
                            self.cancel_loading();
                        }
                    } else {
                        ui.heading("epubthing");
                        ui.add_space(10.0);
                        ui.label("File > Open...");
                    }
                });

                if let Some(err) = &self.error_message {
                    ui.add_space(20.0);
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
            });
        }

        // Apply chapter change from UI interactions
        if let Some(ch) = chapter_to_set {
            self.set_chapter(ch);
        }
    }
}
