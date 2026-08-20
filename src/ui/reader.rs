//! Markdown reader: renders a chapter's converted Markdown with egui-commonmark.
//!
//! Chapters are converted `html -> markdown` once (cached per chapter index) and
//! rendered inside a scrollable frame. Internal links navigate between chapters
//! via `CommonMarkCache` link hooks; images are served by an `epub://` bytes
//! loader backed by the loaded archive.

use eframe::egui;
use eframe::egui::Context;
use eframe::egui::load::{BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use epubthing::EpubDocument;

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;
use std::sync::{Arc, Mutex};

use crate::app::fonts;
use crate::app::loading::LoadedDocument;

/// Image bytes shared between the loader and the app (cleared per book).
static LOADER_CACHE: LazyLock<Mutex<HashMap<String, Arc<[u8]>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The archive backing `epub://` asset lookups. Set on each book load.
static IMAGE_SOURCE: Mutex<Option<Arc<Mutex<EpubDocument>>>> = Mutex::new(None);

/// Points the image loader at a newly opened archive.
pub(crate) fn set_image_source(doc: Option<Arc<Mutex<EpubDocument>>>) {
    *IMAGE_SOURCE.lock().unwrap() = doc;
    LOADER_CACHE.lock().unwrap().clear();
}

fn guess_mime(path: &str) -> Option<String> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    Some(
        match ext.as_str() {
            "png" => "image/png".to_owned(),
            "jpg" | "jpeg" => "image/jpeg".to_owned(),
            "gif" => "image/gif".to_owned(),
            "webp" => "image/webp".to_owned(),
            "svg" => "image/svg+xml".to_owned(),
            "bmp" => "image/bmp".to_owned(),
            other => format!("image/{other}"),
        },
    )
}

/// Loader for `epub://` image uris. Reads assets from the open archive.
struct EpubImageLoader;

impl BytesLoader for EpubImageLoader {
    fn id(&self) -> &str {
        egui::generate_loader_id!(EpubImageLoader)
    }

    fn load(&self, ctx: &Context, uri: &str) -> BytesLoadResult {
        if !uri.starts_with("epub://") {
            return Err(LoadError::NotSupported);
        }
        if let Some(bytes) = LOADER_CACHE.lock().unwrap().get(uri) {
            return Ok(BytesPoll::Ready {
                size: None,
                bytes: egui::load::Bytes::Shared(bytes.clone()),
                mime: guess_mime(uri),
            });
        }
        let path = uri.trim_start_matches("epub://");
        let source = IMAGE_SOURCE
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| LoadError::Loading("no document loaded".into()))?;
        let bytes: Arc<[u8]> = source
            .lock()
            .unwrap()
            .get_asset(path)
            .map_err(|e| LoadError::Loading(e.to_string()))?
            .into();
        LOADER_CACHE
            .lock()
            .unwrap()
            .insert(uri.to_owned(), bytes.clone());
        ctx.request_repaint();
        Ok(BytesPoll::Ready {
            size: None,
            bytes: egui::load::Bytes::Shared(bytes),
            mime: guess_mime(path),
        })
    }

    fn forget(&self, uri: &str) {
        LOADER_CACHE.lock().unwrap().remove(uri);
    }

    fn forget_all(&self) {
        LOADER_CACHE.lock().unwrap().clear();
    }

    fn byte_size(&self) -> usize {
        LOADER_CACHE
            .lock()
            .unwrap()
            .values()
            .map(|bytes| bytes.len())
            .sum()
    }
}

/// Registers the `epub://` image loader. Call once at app startup.
pub(crate) fn register_image_loader(ctx: &Context) {
    ctx.add_bytes_loader(Arc::new(EpubImageLoader));
}

/// State for rendering chapters as markdown.
pub(crate) struct MarkdownReader {
    cache: CommonMarkCache,
    md_cache: HashMap<usize, String>,
    link_map: HashMap<usize, Vec<(String, usize)>>,
}

impl MarkdownReader {
    pub(crate) fn new() -> Self {
        Self {
            cache: CommonMarkCache::default(),
            md_cache: HashMap::new(),
            link_map: HashMap::new(),
        }
    }

    /// Drops all cached markdown/cookies. Call when a new book is loaded.
    pub(crate) fn reset(&mut self) {
        self.cache = CommonMarkCache::default();
        self.md_cache.clear();
        self.link_map.clear();
    }

    /// Converts a chapter's HTML to markdown once and caches the result.
    fn ensure_chapter(&mut self, doc: &LoadedDocument, index: usize) {
        if self.md_cache.contains_key(&index) {
            return;
        }
        let Some(chapter) = doc.chapters.get(index) else {
            return;
        };
        let base = chapter.href.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let chapter_hrefs: Vec<String> = doc
            .chapters
            .iter()
            .map(|c| epubthing::resolve_path("", &c.href))
            .collect();

        let mut md = mdka::html_to_markdown(&chapter.html);
        md = epubthing::strip_raw_html(&md);
        md = epubthing::heal_markdown(&md);
        let (md, targets) = epubthing::rewrite_links_and_images(&md, base, &chapter_hrefs);

        self.md_cache.insert(index, md);
        self.link_map.insert(index, targets);
    }

    /// Renders the chapter at `index`. Returns the chapter to navigate to if an
    /// internal link was clicked. `column_width` constrains the text column;
    /// `align` positions it. When `reset_scroll` is `true`, scrolls the chapter
    /// area back to the top (and clears the flag).
    pub(crate) fn render(
        &mut self,
        ui: &mut egui::Ui,
        doc: &LoadedDocument,
        index: usize,
        font_size: f32,
        fg: egui::Color32,
        bg: egui::Color32,
        column_width: f32,
        align: egui::Align,
        font_loaded: bool,
        reset_scroll: &mut bool,
    ) -> Option<usize> {
        self.ensure_chapter(doc, index);
        let Some(markdown) = self.md_cache.get(&index).cloned() else {
            return None;
        };
        let targets = self.link_map.get(&index).cloned().unwrap_or_default();

        let family: egui::FontFamily = if font_loaded {
            fonts::reader_font_family()
        } else {
            egui::FontFamily::Proportional
        };
        ui.style_mut().text_styles = BTreeMap::from([
            (egui::TextStyle::Body, egui::FontId::new(font_size, family.clone())),
            (
                egui::TextStyle::Small,
                egui::FontId::new(font_size * 0.8, family.clone()),
            ),
            (
                egui::TextStyle::Button,
                egui::FontId::new(font_size, family.clone()),
            ),
            (
                egui::TextStyle::Heading,
                egui::FontId::new(font_size * 1.5, family.clone()),
            ),
            (
                egui::TextStyle::Monospace,
                egui::FontId::new(font_size * 0.9, family),
            ),
        ]);

        self.cache.link_hooks_clear();
        for (target, _) in &targets {
            self.cache.add_link_hook(target.clone());
        }

        let mut requested: Option<usize> = None;
        let mut area = egui::ScrollArea::vertical()
            .id_salt("chapter_text")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
        if *reset_scroll {
            area = area.scroll_offset(egui::Vec2::ZERO);
            *reset_scroll = false;
        }
        area.show(ui, |ui| {
            let available = ui.available_width();
            let width = column_width.min(available.max(0.0));
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::top_down(align), |ui| {
                let frame = egui::Frame::NONE
                    .fill(bg)
                    .inner_margin(egui::Margin::same(16))
                    .corner_radius(egui::CornerRadius::same(4));
                let response = frame
                    .show(ui, |ui| {
                        ui.set_max_width((width - 32.0).max(0.0));
                        ui.style_mut().visuals.override_text_color = Some(fg);
                        let _ = CommonMarkViewer::new()
                            .show(ui, &mut self.cache, &markdown);
                    })
                    .response;
                response.context_menu(|ui| {
                    if ui.button("Copy all text").clicked() {
                        ui.ctx().copy_text(md_to_plain(&markdown));
                        ui.close_menu();
                    }
                });
            });
        });

        for (target, target_index) in &targets {
            if self.cache.get_link_hook(target).unwrap_or(false) {
                requested = Some(*target_index);
                break;
            }
        }
        requested
    }
}

/// Rough markdown-to-plain-text for "Copy all text".
fn md_to_plain(markdown: &str) -> String {
    let mut out = String::new();
    for line in markdown.lines() {
        let text = line.trim_start_matches(['#', '>', '*', '-', '`', ' ', '\t']);
        out.push_str(text);
        out.push('\n');
    }
    out
}