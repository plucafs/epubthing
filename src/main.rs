mod app;
mod config;
mod conversation;
mod ui;

use eframe::egui;
use eframe::egui::Color32;

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use app::fonts;
use app::loading::{self as loading_mod, LoadedDocument, LoadingTask, TocEntry};
use config::{config_path, load_config, save_config, AppConfig, ReadingPosition};
use conversation::{
    export_conversation, import_conversation, load_conversation, save_conversation,
    ConversationMessage,
};
use ui::reader::{self, MarkdownReader};

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

// ─── TOC tree (sidebar) ────────────────────────────────────────────────────

/// Whether an entry (or any descendant) matches the sidebar search query.
fn toc_matches(entry: &TocEntry, query: &str) -> bool {
    if entry.label.to_lowercase().contains(query) {
        return true;
    }
    entry.children.iter().any(|child| toc_matches(child, query))
}

/// Collects the spine indexes covered by a TOC tree. Entries pointing at the
/// same spine item share one index by construction.
fn collect_toc_chapters(entry: &TocEntry, out: &mut HashSet<usize>) {
    if let Some(index) = entry.chapter {
        out.insert(index);
    }
    for child in &entry.children {
        collect_toc_chapters(child, out);
    }
}

/// Flattens the TOC tree into `(label, chapter_index)` pairs in depth-first
/// order, for the Ctrl+G TOC window.
fn collect_toc_flat(entries: &[TocEntry], out: &mut Vec<(String, usize)>) {
    for entry in entries {
        if let Some(index) = entry.chapter {
            out.push((entry.label.clone(), index));
        }
        collect_toc_flat(&entry.children, out);
    }
}

/// Renders one TOC entry: a clickable leaf or a collapsible container.
fn show_toc_entry(
    ui: &mut egui::Ui,
    entry: &TocEntry,
    query: &str,
    current_chapter: usize,
    chapter_to_set: &mut Option<usize>,
) {
    if entry.children.is_empty() {
        if !query.is_empty() && !entry.label.to_lowercase().contains(query) {
            return;
        }
        show_toc_leaf(ui, entry, current_chapter, chapter_to_set);
        return;
    }

    if !query.is_empty() && !toc_matches(entry, query) {
        return;
    }

    let selected = entry.chapter == Some(current_chapter);
    let selection_color = ui.visuals().selection.stroke.color;
    let text = egui::RichText::new(&entry.label)
        .strong()
        .color(if selected {
            selection_color
        } else {
            ui.visuals().text_color()
        });

    let id = ui.make_persistent_id(entry.href.as_str());
    let state = egui::containers::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        false,
    );

    let header = state.show_header(ui, |ui| {
        let width = ui.available_width().max(0.0);
        // Match the row height to a standard interactive row so the native
        // collapse arrow centers vertically against the label button.
        let row_height = ui.spacing().interact_size.y;
        let response = ui.add_sized(
            [width, row_height],
            egui::Button::new(text)
                .fill(if selected {
                    ui.visuals().selection.bg_fill
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(egui::Stroke::NONE)
                .wrap_mode(egui::TextWrapMode::Truncate),
        );
        // Navigation only from the label button; the native toggle arrow
        // (rendered by `show_header`) only expands/collapses.
        if response.clicked() {
            if let Some(index) = entry.chapter {
                *chapter_to_set = Some(index);
            }
        }
        response.on_hover_text(&entry.label);
    });

    if !query.is_empty() {
        // Search mode: force the tree open without persisting the state.
        ui.indent(id, |ui| {
            for child in &entry.children {
                show_toc_entry(ui, child, query, current_chapter, chapter_to_set);
            }
        });
    } else {
        header.body(|ui| {
            for child in &entry.children {
                show_toc_entry(ui, child, query, current_chapter, chapter_to_set);
            }
        });
    }
}

/// Renders a TOC leaf as a full-width button, highlighting the current chapter.
fn show_toc_leaf(
    ui: &mut egui::Ui,
    entry: &TocEntry,
    current_chapter: usize,
    chapter_to_set: &mut Option<usize>,
) {
    let selected = entry.chapter == Some(current_chapter);
    let text = if selected {
        egui::RichText::new(&entry.label).color(ui.visuals().selection.stroke.color)
    } else {
        egui::RichText::new(&entry.label)
    };
    let response = ui.add_sized(
        [ui.available_width(), 0.0],
        egui::Button::new(text)
            .fill(if selected {
                ui.visuals().selection.bg_fill
            } else {
                Color32::TRANSPARENT
            })
            .stroke(egui::Stroke::NONE)
            .wrap_mode(egui::TextWrapMode::Truncate),
    );
    if response.clicked() {
        if let Some(index) = entry.chapter {
            *chapter_to_set = Some(index);
        }
    }
    response.on_hover_text(&entry.label);
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
    /// Focus the recent-files search field on the next frame
    recent_needs_focus: bool,
    /// Scroll the recent-files list to the selected row on the next frame
    recent_scroll_to_sel: bool,
    /// Whether the TOC window (Ctrl+G) is open
    show_toc_window: bool,
    /// Search filter for the TOC window
    toc_window_search: String,
    /// Current selection index in the filtered TOC window list
    toc_window_selection: usize,
    /// Focus the TOC window search field on the next frame
    toc_window_needs_focus: bool,
    /// Scroll the TOC window list to the selected row on the next frame
    toc_window_scroll_to_sel: bool,
    // --- Conversation ---
    conversation_messages: Vec<ConversationMessage>,
    conversation_input: String,
    /// false = Author mode (left), true = Me mode (right)
    conversation_me_mode: bool,
    /// Flag to scroll conversation to bottom after sending
    conversation_scroll_to_bottom: bool,
    reset_chapter_scroll: bool,
    /// Background loading task
    loading: Option<LoadingTask>,
    /// Markdown rendering state for the current book
    markdown_reader: MarkdownReader,
    /// Whether the epub:// image loader was registered
    loader_registered: bool,
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
            loaded_font: String::new(),
            font_loaded: false,
            pending_startup_load: None,
            toc_search: String::new(),
            is_fullscreen: false,
            show_recent: false,
            recent_search: String::new(),
            recent_selection: 0,
            recent_needs_focus: false,
            recent_scroll_to_sel: false,
            show_toc_window: false,
            toc_window_search: String::new(),
            toc_window_selection: 0,
            toc_window_needs_focus: false,
            toc_window_scroll_to_sel: false,
            conversation_messages: Vec::new(),
            conversation_input: String::new(),
            conversation_me_mode: true, // Default to Me mode
            conversation_scroll_to_bottom: false,
            reset_chapter_scroll: false,
            loading: None,
            markdown_reader: MarkdownReader::new(),
            loader_registered: false,
        }
    }

    /// Starts background loading of an EPUB from a file path.
    fn start_loading_from_path(&mut self, path: &str) {
        self.cancel_loading();
        self.document = None;
        self.error_message = None;
        self.loading = None;
        reader::set_image_source(None);

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
        self.loading = None;
        reader::set_image_source(None);
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

    /// Opens the recent files dialog and focuses the search field.
    fn open_recent_files(&mut self) {
        self.show_recent = true;
        self.recent_search.clear();
        self.recent_selection = 0;
        self.recent_needs_focus = true;
        self.recent_scroll_to_sel = false;
    }

    /// Opens the TOC window and focuses its search field.
    fn open_toc_window(&mut self) {
        self.show_toc_window = true;
        self.toc_window_search.clear();
        self.toc_window_selection = 0;
        self.toc_window_needs_focus = true;
        self.toc_window_scroll_to_sel = false;
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
                    reader::set_image_source(Some(self.document.as_ref().unwrap().raw.clone()));
                    self.markdown_reader.reset();
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

        // --- Register the epub:// image loader once ---
        if !self.loader_registered {
            reader::register_image_loader(ctx);
            self.loader_registered = true;
        }

        // --- Poll background loading ---
        self.poll_loading();

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
                self.show_recent = false;
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
            // Ctrl+R: open recent files
            if i.modifiers.ctrl && i.key_pressed(egui::Key::R) {
                if self.show_recent {
                    self.show_recent = false;
                } else {
                    self.open_recent_files();
                }
            }
            // Ctrl+G: open the TOC window
            if i.modifiers.ctrl && i.key_pressed(egui::Key::G) {
                if self.show_toc_window {
                    self.show_toc_window = false;
                } else {
                    self.open_toc_window();
                }
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
                    self.recent_scroll_to_sel = true;
                }
                if i.key_pressed(egui::Key::ArrowUp) {
                    self.recent_selection = self.recent_selection.saturating_sub(1);
                    self.recent_scroll_to_sel = true;
                }
            }
            // TOC window navigation
            if self.show_toc_window {
                if i.key_pressed(egui::Key::ArrowDown) {
                    self.toc_window_selection = self.toc_window_selection.saturating_add(1);
                    self.toc_window_scroll_to_sel = true;
                }
                if i.key_pressed(egui::Key::ArrowUp) {
                    self.toc_window_selection = self.toc_window_selection.saturating_sub(1);
                    self.toc_window_scroll_to_sel = true;
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
                    ui.separator();
                    if ui.button("Recent Files").clicked() {
                        ui.close_menu();
                        self.open_recent_files();
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
                    if ui.button("Show Book Folder...").clicked() {
                        ui.close_menu();
                        match self.current_file_path.as_ref() {
                            Some(path) => {
                                let dir = std::path::Path::new(path)
                                    .parent()
                                    .filter(|p| !p.as_os_str().is_empty())
                                    .unwrap_or_else(|| std::path::Path::new("."));
                                if let Err(error) = open_directory(dir) {
                                    self.status_message = Some(format!(
                                        "Could not open book folder: {}",
                                        error
                                    ));
                                    self.status_message_time = Some(std::time::Instant::now());
                                }
                            }
                            None => {
                                self.status_message = Some("No book loaded yet.".into());
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
                    if ui.button("Table of Contents").clicked() {
                        ui.close_menu();
                        if self.show_toc_window {
                            self.show_toc_window = false;
                        } else {
                            self.open_toc_window();
                        }
                    }
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
            .resizable([true, false])
            .default_size([520.0, 520.0])
            .min_size([280.0, 520.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let search_response = ui.add(
                    egui::TextEdit::singleline(&mut self.recent_search)
                        .hint_text("Search recent files...")
                        .desired_width(ui.available_width()),
                );
                if self.recent_needs_focus {
                    search_response.request_focus();
                    self.recent_needs_focus = false;
                }
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
                let clamped = filtered.len().saturating_sub(1);
                if self.recent_selection != clamped && self.recent_selection >= filtered.len() {
                    self.recent_selection = clamped;
                    self.recent_scroll_to_sel = true;
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
                            if selected && self.recent_scroll_to_sel {
                                response.scroll_to_me(Some(egui::Align::Center));
                                self.recent_scroll_to_sel = false;
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

        // ─── Table of contents window (Ctrl+G) ──────────────────────
        // Build the flat entry list while the document is borrowed, then run
        // the window outside the borrow so navigation can mutate app state.
        let toc_window_data: Option<Vec<(String, usize)>> = if let Some(doc) = &self.document {
            let mut flat: Vec<(String, usize)> = Vec::new();
            collect_toc_flat(&doc.toc, &mut flat);
            let mut in_tree = HashSet::new();
            for entry in &doc.toc {
                collect_toc_chapters(entry, &mut in_tree);
            }
            for (i, chapter) in doc.chapters.iter().enumerate() {
                if !in_tree.contains(&i) {
                    flat.push((chapter.label.clone(), i));
                }
            }
            Some(flat)
        } else {
            None
        };
        if let Some(flat) = toc_window_data {
            let mut show_toc_window = self.show_toc_window;
            egui::Window::new("Table of Contents")
                .open(&mut show_toc_window)
                .collapsible(false)
                .resizable([true, false])
                .default_size([520.0, 520.0])
                .min_size([280.0, 520.0])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let search_response = ui.add(
                        egui::TextEdit::singleline(&mut self.toc_window_search)
                            .hint_text("Search chapters...")
                            .desired_width(ui.available_width()),
                    );
                    if self.toc_window_needs_focus {
                        search_response.request_focus();
                        self.toc_window_needs_focus = false;
                    }
                    ui.separator();

                    let query = self.toc_window_search.to_lowercase();
                    let filtered: Vec<(String, usize)> = flat
                        .iter()
                        .filter(|(label, _)| {
                            query.is_empty() || label.to_lowercase().contains(&query)
                        })
                        .cloned()
                        .collect();

                    let clamped = filtered.len().saturating_sub(1);
                    if self.toc_window_selection != clamped
                        && self.toc_window_selection >= filtered.len()
                    {
                        self.toc_window_selection = clamped;
                        self.toc_window_scroll_to_sel = true;
                    }

                    // Enter opens the selected chapter
                    let enter_pressed = ctx.input(|i| i.key_pressed(egui::Key::Enter));
                    let mut to_navigate: Option<usize> = None;
                    if enter_pressed && !filtered.is_empty() {
                        to_navigate = Some(filtered[self.toc_window_selection].1);
                    }

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if filtered.is_empty() {
                            if flat.is_empty() {
                                ui.label("No chapters found.");
                            } else {
                                ui.label("No matches.");
                            }
                        }
                        let current = self.current_chapter;
                        for (list_i, (label, chapter_idx)) in filtered.iter().enumerate() {
                            let selected = list_i == self.toc_window_selection;
                            let highlighted = selected
                                || (query.is_empty() && *chapter_idx == current);
                            ui.horizontal(|ui| {
                                let response = ui.add_sized(
                                    [ui.available_width() - 4.0, 0.0],
                                    egui::Button::new(egui::RichText::new(label))
                                        .fill(if highlighted {
                                            ui.visuals().selection.bg_fill
                                        } else {
                                            Color32::TRANSPARENT
                                        })
                                        .stroke(if highlighted {
                                            egui::Stroke::new(
                                                1.0_f32,
                                                ui.visuals().selection.stroke.color,
                                            )
                                        } else {
                                            egui::Stroke::NONE
                                        })
                                        .wrap_mode(egui::TextWrapMode::Truncate),
                                );
                                if response.clicked() {
                                    to_navigate = Some(*chapter_idx);
                                }
                                if selected && self.toc_window_scroll_to_sel {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                    self.toc_window_scroll_to_sel = false;
                                }
                            });
                        }
                    });

                    if let Some(idx) = to_navigate {
                        self.set_chapter(idx);
                        self.show_toc_window = false;
                        self.toc_window_search.clear();
                        self.toc_window_selection = 0;
                    }
                });
            if !show_toc_window {
                self.show_toc_window = false;
            }
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
                            for entry in &doc.toc {
                                show_toc_entry(
                                    ui,
                                    entry,
                                    &query,
                                    self.current_chapter,
                                    &mut chapter_to_set,
                                );
                            }

                            // Spine chapters that are not listed in the TOC tree.
                            let mut in_tree = HashSet::new();
                            for entry in &doc.toc {
                                collect_toc_chapters(entry, &mut in_tree);
                            }
                            let tail: Vec<_> = doc
                                .chapters
                                .iter()
                                .enumerate()
                                .filter(|(i, _)| !in_tree.contains(i))
                                .collect();
                            if !tail.is_empty() {
                                ui.add_space(4.0);
                                ui.separator();
                            }
                            for (i, chapter) in tail {
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

                    // Markdown reader
                    let font_color = self.config.font_color32();
                    let bg_color = self.config.bg_color32();
                    let font_size = self.config.font_size;
                    let column_width = self.config.text_width_ch * font_size * 0.55;
                    let align = self.config.text_align.to_egui();

                    {
                        let scale = self.config.scroll_speed / 50.0;
                        let ctx = ui.ctx().clone();
                        ctx.input_mut(|i| {
                            i.smooth_scroll_delta.y *= scale;
                        });
                    }

                    if let Some(requested) = self.markdown_reader.render(
                        ui,
                        doc,
                        self.current_chapter,
                        font_size,
                        font_color,
                        bg_color,
                        column_width,
                        align,
                        self.font_loaded,
                        &mut self.reset_chapter_scroll,
                        self.config.show_minimap,
                    ) {
                        chapter_to_set = Some(requested);
                    }
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
