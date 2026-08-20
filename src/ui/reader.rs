//! Markdown reader: renders a chapter's converted Markdown with egui-commonmark.
//!
//! Chapters are converted `html -> markdown` once (cached per chapter index) and
//! rendered inside a scrollable frame. Internal links navigate between chapters
//! via `CommonMarkCache` link hooks; images are served by an `epub://` bytes
//! loader backed by the loaded archive.

use eframe::egui;
use eframe::egui::Context;
use eframe::egui::load::{
    BytesLoadResult, BytesLoader, BytesPoll, LoadError, SizeHint, SizedTexture, TextureLoadResult,
    TextureLoader, TexturePoll,
};
use eframe::egui::TextureOptions;
use eframe::epaint::ColorImage;
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
///
/// Implements both `BytesLoader` (raw bytes) and `TextureLoader` (decoded
/// pixels). The texture path decodes with the `image` crate and routes SVG
/// content to `egui_extras`, falling back to a neutral placeholder instead of
/// an error so the reader never shows the red "?" placeholder for an image.
struct EpubImageLoader;

/// Decodes EPUB asset bytes into an image, or a 1x1 transparent placeholder.
fn decode_bytes(uri: &str, bytes: &[u8]) -> ColorImage {
    let is_svg = uri.to_ascii_lowercase().ends_with(".svg")
        || guess_mime(uri).as_deref() == Some("image/svg+xml");
    let result = if is_svg {
        egui_extras::image::load_svg_bytes(bytes).map_err(|e| e.to_string())
    } else {
        egui_extras::image::load_image_bytes(bytes).map_err(|e| e.to_string())
    };
    match result {
        Ok(img) => img,
        Err(e) => {
            eprintln!("epubthing: could not decode image {uri}: {e}");
            ColorImage::new([1, 1], egui::Color32::TRANSPARENT)
        }
    }
}

impl TextureLoader for EpubImageLoader {
    fn id(&self) -> &str {
        egui::generate_loader_id!(EpubImageLoader)
    }

    fn load(
        &self,
        ctx: &Context,
        uri: &str,
        texture_options: TextureOptions,
        _size_hint: SizeHint,
    ) -> TextureLoadResult {
        if !uri.starts_with("epub://") {
            return Err(LoadError::NotSupported);
        }
        let bytes = self.load_asset(uri)?;
        let image = decode_bytes(uri, &bytes);
        let handle = ctx.load_texture(uri, image, texture_options);
        Ok(TexturePoll::Ready {
            texture: SizedTexture::from_handle(&handle),
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

impl EpubImageLoader {
    /// Returns cached or freshly-read asset bytes for an `epub://` uri.
    fn load_asset(&self, uri: &str) -> eframe::egui::load::Result<Arc<[u8]>> {
        if let Some(bytes) = LOADER_CACHE.lock().unwrap().get(uri) {
            return Ok(bytes.clone());
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
        Ok(bytes)
    }
}

impl BytesLoader for EpubImageLoader {
    fn id(&self) -> &str {
        egui::generate_loader_id!(EpubImageLoader)
    }

    fn load(&self, ctx: &Context, uri: &str) -> BytesLoadResult {
        if !uri.starts_with("epub://") {
            return Err(LoadError::NotSupported);
        }
        let bytes = self.load_asset(uri)?;
        ctx.request_repaint();
        Ok(BytesPoll::Ready {
            size: None,
            bytes: egui::load::Bytes::Shared(bytes),
            mime: guess_mime(uri),
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
    ctx.add_texture_loader(Arc::new(EpubImageLoader));
}

/// State for rendering chapters as markdown.
pub(crate) struct MarkdownReader {
    cache: CommonMarkCache,
    md_cache: HashMap<usize, String>,
    link_map: HashMap<usize, Vec<(String, usize)>>,
    /// Target scroll offset requested by a minimap drag, applied next frame.
    minimap_pending_scroll: Option<f32>,
    /// Chapter index the minimap state belongs to (so drags never leak across
    /// chapter changes).
    minimap_chapter: Option<usize>,
}

impl MarkdownReader {
    pub(crate) fn new() -> Self {
        Self {
            cache: CommonMarkCache::default(),
            md_cache: HashMap::new(),
            link_map: HashMap::new(),
            minimap_pending_scroll: None,
            minimap_chapter: None,
        }
    }

    /// Drops all cached markdown/cookies. Call when a new book is loaded.
    pub(crate) fn reset(&mut self) {
        self.cache = CommonMarkCache::default();
        self.md_cache.clear();
        self.link_map.clear();
        self.minimap_pending_scroll = None;
        self.minimap_chapter = None;
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
        let (md, targets) = epubthing::rewrite_links_and_images(&md, base, &chapter_hrefs, index);
        let md = epubthing::trim_leading_blank_lines(&md);

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
        show_minimap: bool,
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
        if self.minimap_chapter != Some(index) {
            self.minimap_pending_scroll = None;
            self.minimap_chapter = Some(index);
        }
        if let Some(target) = self.minimap_pending_scroll.take() {
            area = area.vertical_scroll_offset(target);
        }
        let output = area.show(ui, |ui| {
            let available = ui.available_width();
            let width = column_width.min(available.max(0.0));
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::top_down(align), |ui| {
                let frame = egui::Frame::NONE
                    .fill(bg)
                    .inner_margin(egui::Margin::same(16))
                    .corner_radius(egui::CornerRadius::same(4));
                frame.show(ui, |ui| {
                        ui.set_max_width((width - 32.0).max(0.0));
                        ui.style_mut().visuals.override_text_color = Some(fg);
                        let _ = CommonMarkViewer::new()
                            .alerts(egui_commonmark::AlertBundle::empty())
                            .show(ui, &mut self.cache, &markdown);
                    });
            });
        });

        if show_minimap {
            self.draw_minimap(ui, &output, &markdown);
        }

        for (target, target_index) in &targets {
            if self.cache.get_link_hook(target).unwrap_or(false) {
                requested = Some(*target_index);
                break;
            }
        }
        requested
    }

    /// Draws a VSCode-style minimap strip along the right edge of the reader.
    /// Each markdown line gets a small tick. The thick thumb shows the
    /// viewport position. Clicking or dragging the strip scrolls the chapter:
    /// the target offset is applied on the next frame via `minimap_pending_scroll`.
    fn draw_minimap(&mut self, ui: &mut egui::Ui, output: &egui::scroll_area::ScrollAreaOutput<()>, markdown: &str) {
        const STRIP_W: f32 = 14.0;
        const STRIP_PAD: f32 = 2.0;

        let inner = output.inner_rect;
        // Leave the native scrollbar (rightmost ~bar_width px) untouched.
        let scrollbar_w = ui.spacing().scroll.bar_width;
        let strip = egui::Rect::from_min_max(
            egui::pos2(inner.right() - scrollbar_w - STRIP_W, inner.top()),
            egui::pos2(inner.right() - scrollbar_w, inner.bottom()),
        );
        let strip_h = strip.height().max(1.0);
        let viewport_h = inner.height().max(1.0);
        let content_h = output.content_size.y.max(1.0);
        let max_scroll = (content_h - viewport_h).max(0.0);
        let scroll_frac = if max_scroll > 0.0 {
            output.state.offset.y / max_scroll
        } else {
            0.0
        };

        let visuals = ui.visuals();
        let painter = ui.painter();

        // Background strip.
        let bg = visuals.extreme_bg_color.gamma_multiply(0.55);
        painter.rect_filled(strip, 0.0, bg);

        // Line ticks.
        let n = markdown.lines().count().max(1) as f32;
        let line_h = (strip_h / n).clamp(1.0, 2.5);
        let tick = visuals.text_color().gamma_multiply(0.35);
        for (i, line) in markdown.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let y = strip.top() + strip_h * (i as f32 / n);
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(strip.left() + STRIP_PAD, y),
                    egui::pos2(strip.right() - STRIP_PAD, (y + line_h).min(strip.bottom())),
                ),
                0.0,
                tick,
            );
        }

        // Viewport thumb.
        let thumb_h = (strip_h * (viewport_h / content_h)).clamp(16.0, strip_h);
        let max_thumb_y = strip_h - thumb_h;
        let thumb_y = strip.top() + scroll_frac * max_thumb_y;
        let thumb_color = visuals.selection.bg_fill.gamma_multiply(0.85);
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(strip.left(), thumb_y),
                egui::pos2(strip.right(), (thumb_y + thumb_h).min(strip.bottom())),
            ),
            0.0,
            thumb_color,
        );

        // Click / drag to scroll. Drag proceeds from the grabbed point: clamp
        // so the thumb follows the pointer on the first frame.
        let ctx = ui.ctx();
        if let Some(pointer_pos) = ctx.pointer_interact_pos() {
            if ctx.input(|i| i.pointer.primary_down()) && strip.expand(2.0).contains(pointer_pos) {
                let frac = ((pointer_pos.y - strip.top()) / strip_h).clamp(0.0, 1.0);
                self.minimap_pending_scroll = Some(frac * max_scroll);
                ctx.request_repaint();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_jpeg_bytes() {
        let mut buf = Vec::new();
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(2, 2));
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Jpeg)
        .unwrap();
        let rgba = egui_extras::image::load_image_bytes(&buf).unwrap();
        assert_eq!(rgba.size, [2, 2]);
    }

    #[test]
    fn decodes_png_bytes() {
        let mut buf = Vec::new();
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(2, 2));
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
        let rgba = egui_extras::image::load_image_bytes(&buf).unwrap();
        assert_eq!(rgba.size, [2, 2]);
    }

    #[test]
    fn decodes_webp_bytes() {
        let mut buf = Vec::new();
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(2, 2));
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::WebP)
        .unwrap();
        let rgba = egui_extras::image::load_image_bytes(&buf).unwrap();
        assert_eq!(rgba.size, [2, 2]);
    }
}