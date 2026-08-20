use eframe::egui;
use eframe::egui::Context;
use eframe::egui::load::{BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use epubthing::EpubDocument;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

static DOC: OnceLock<Arc<Mutex<EpubDocument>>> = OnceLock::new();

/// Loader for `epub://` image uris. Reads assets from the opened archive.
struct EpubImageLoader {
    cache: Mutex<HashMap<String, Arc<[u8]>>>,
}

impl EpubImageLoader {
    fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }
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

impl BytesLoader for EpubImageLoader {
    fn id(&self) -> &str {
        egui::generate_loader_id!(EpubImageLoader)
    }

    fn load(&self, ctx: &Context, uri: &str) -> BytesLoadResult {
        if !uri.starts_with("epub://") {
            return Err(LoadError::NotSupported);
        }
        if let Some(bytes) = self.cache.lock().unwrap().get(uri) {
            return Ok(BytesPoll::Ready {
                size: None,
                bytes: egui::load::Bytes::Shared(bytes.clone()),
                mime: guess_mime(uri),
            });
        }
        let path = uri.trim_start_matches("epub://");
        let doc = DOC
            .get()
            .ok_or_else(|| LoadError::Loading("no document loaded".into()))?;
        let bytes: Arc<[u8]> = doc
            .lock()
            .unwrap()
            .get_asset(path)
            .map_err(|e| LoadError::Loading(e.to_string()))?
            .into();
        self.cache.lock().unwrap().insert(uri.to_owned(), bytes.clone());
        ctx.request_repaint();
        Ok(BytesPoll::Ready {
            size: None,
            bytes: egui::load::Bytes::Shared(bytes),
            mime: guess_mime(path),
        })
    }

    fn forget(&self, uri: &str) {
        self.cache.lock().unwrap().remove(uri);
    }

    fn forget_all(&self) {
        self.cache.lock().unwrap().clear();
    }

    fn byte_size(&self) -> usize {
        self.cache.lock().unwrap().values().map(|b| b.len()).sum()
    }
}

struct Chapter {
    href: String,
    label: String,
}

struct SpikeApp {
    doc: Arc<Mutex<EpubDocument>>,
    chapters: Vec<Chapter>,
    current: usize,
    markdown: String,
    cache: CommonMarkCache,
    font_size: f32,
    bg: egui::Color32,
    fg: egui::Color32,
    /// (markdown link target, chapter index)
    link_targets: Vec<(String, usize)>,
    requested: Option<usize>,
    rebuild_next_frame: bool,
    show_raw_markdown: bool,
    mark_inject: bool,
}

impl SpikeApp {
    fn new(path: &str, ctx: &Context) -> Self {
        let doc = Arc::new(Mutex::new(
            EpubDocument::open(path).expect("open epub"),
        ));
        DOC.set(doc.clone()).ok();

        ctx.add_bytes_loader(Arc::new(EpubImageLoader::new()));

        let chapters = {
            let d = doc.lock().unwrap();
            let label_by_href: std::collections::HashMap<String, String> =
                epubthing::flatten_toc(&d.toc)
                    .into_iter()
                    .filter_map(|item| {
                        let href = epubthing::resolve_path("", &item.href);
                        if href.is_empty() {
                            None
                        } else {
                            Some((href, item.label.clone()))
                        }
                    })
                    .collect();
            d.spine
                .iter()
                .enumerate()
                .map(|(i, s)| Chapter {
                    href: s.href.clone(),
                    label: label_by_href
                        .get(&epubthing::resolve_path("", &s.href))
                        .cloned()
                        .unwrap_or_else(|| format!("Chapter {}", i + 1)),
                })
                .collect()
        };

        let mut app = Self {
            doc,
            chapters,
            current: 0,
            markdown: String::new(),
            cache: CommonMarkCache::default(),
            font_size: 18.0,
            bg: egui::Color32::from_rgb(252, 248, 240),
            fg: egui::Color32::from_rgb(30, 30, 30),
            link_targets: Vec::new(),
            requested: None,
            rebuild_next_frame: true,
            show_raw_markdown: false,
            mark_inject: false,
        };
        app.rebuild();
        app
    }

    fn rebuild(&mut self) {
        let ch = &self.chapters[self.current];
        let html = {
            let mut d = self.doc.lock().unwrap();
            d.get_content(&ch.href).unwrap_or_default()
        };

        let mut md = mdka::html_to_markdown(&html);
        md = strip_raw_html(&md);
        md = heal_markdown(&md);
        if self.mark_inject {
            md = md.replace("the", "<mark>the</mark>");
        }

        let base = ch.href.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let chapter_hrefs: Vec<String> = self
            .chapters
            .iter()
            .map(|c| epubthing::resolve_path("", &c.href))
            .collect();

        let (md, targets) = rewrite_links_and_images(&md, base, &chapter_hrefs);
        self.markdown = md;
        self.link_targets = targets;

        self.cache.link_hooks_clear();
        for (target, _) in &self.link_targets {
            self.cache.add_link_hook(target.clone());
        }
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("< Prev").clicked() && self.current > 0 {
                self.requested = Some(self.current - 1);
            }
            ui.label(format!(
                "{}/{}  {}",
                self.current + 1,
                self.chapters.len(),
                self.chapters[self.current].label
            ));
            if ui.button("Next >").clicked() && self.current + 1 < self.chapters.len() {
                self.requested = Some(self.current + 1);
            }
            ui.separator();
            ui.add(egui::Slider::new(&mut self.font_size, 10.0..=40.0).text("font"));
            ui.separator();
            if ui.checkbox(&mut self.show_raw_markdown, "raw markdown").changed() {
                self.rebuild_next_frame = true;
            }
            if ui.checkbox(&mut self.mark_inject, "inject <mark>").changed() {
                self.rebuild_now();
            }
        });

        ui.separator();

        let available = ui.available_width();
        let width = available.min(760.0);

        if self.show_raw_markdown {
            egui::ScrollArea::vertical()
                .id_salt("raw_md")
                .show(ui, |ui| {
                    ui.set_max_width(width);
                    ui.add(
                        egui::text_edit::TextEdit::multiline(&mut self.markdown)
                            .desired_width(width)
                            .font(egui::TextStyle::Monospace),
                    );
                });
            return;
        }

        // Font size: apply per frame via the ui's text styles. The viewer reads
        // these when resolving its RichText styles.
        let size = self.font_size;
        ui.style_mut().text_styles = std::collections::BTreeMap::from([
            (egui::TextStyle::Body, egui::FontId::proportional(size)),
            (egui::TextStyle::Small, egui::FontId::proportional(size * 0.8)),
            (egui::TextStyle::Button, egui::FontId::proportional(size)),
            (
                egui::TextStyle::Heading,
                egui::FontId::proportional(size * 1.4),
            ),
            (
                egui::TextStyle::Monospace,
                egui::FontId::monospace(size * 0.9),
            ),
        ]);

        // Work on a clone so we can mutate self while egui borrows the string.
        let markdown = self.markdown.clone();

        egui::ScrollArea::vertical()
            .id_salt("chapter_text")
            .show(ui, |ui| {
                ui.set_max_width(width);
                let frame = egui::Frame::NONE
                    .fill(self.bg)
                    .inner_margin(egui::Margin::same(20))
                    .corner_radius(egui::CornerRadius::same(4));
                frame.show(ui, |ui| {
                    ui.style_mut().visuals.override_text_color = Some(self.fg);
                    let _ = CommonMarkViewer::new().show(ui, &mut self.cache, &markdown);
                })
                .response
                .context_menu(|ui| {
                    if ui.button("Copy all text").clicked() {
                        ui.ctx().copy_text(
                            mdka::html_to_markdown(&markdown).replace('#', "").trim().to_string(),
                        );
                        ui.close_menu();
                    }
                });
            });

        // Per-chapter internal-link navigation
        let mut requested: Option<usize> = None;
        for (target, index) in &self.link_targets {
            if self.cache.get_link_hook(target).unwrap_or(false) {
                requested = Some(*index);
            }
        }
        if let Some(ch) = requested {
            self.requested = Some(ch);
        }
    }

    fn rebuild_now(&mut self) {
        self.rebuild();
    }
}

/// Strip raw `<...>` html that mdka leaves in the markdown (e.g. id-only anchors).
/// pulldown-cmark would otherwise render those literally as plain text.
fn strip_raw_html(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut chars = markdown.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            let mut is_tag = false;
            match chars.peek() {
                Some(n) if n.is_ascii_alphabetic() || *n == '/' || *n == '!' => {
                    is_tag = true;
                    let mut in_quote = false;
                    // scan until matching '>'
                    for ch in chars.by_ref() {
                        if ch == '"' || ch == '\'' {
                            in_quote = !in_quote;
                        }
                        if ch == '>' && !in_quote {
                            break;
                        }
                        if ch != '\n' {
                            tag.push(ch);
                        }
                    }
                }
                _ => {}
            }
            if is_tag {
                let _ = tag;
            } else {
                out.push('<');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Rewrites `![alt](src)` image targets to `epub://` resolved paths and returns
/// internal (chapter) link targets ready for link hooks.
fn rewrite_links_and_images(
    markdown: &str,
    base: &str,
    chapter_hrefs: &[String],
) -> (String, Vec<(String, usize)>) {
    let mut out = String::with_capacity(markdown.len());
    let mut targets = Vec::new();

    let chars: Vec<char> = markdown.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let is_image = chars[i] == '!' && chars.get(i + 1) == Some(&'[');
        let is_link = !is_image && chars[i] == '[';
        if (is_image || is_link) && i + 1 < chars.len() {
            // find closing `](`
            let mut j = i + 1;
            let mut in_quote = false;
            while j < chars.len() {
                if chars[j] == '"' || chars[j] == '\'' {
                    in_quote = !in_quote;
                }
                if !in_quote && chars[j] == ']' && chars.get(j + 1) == Some(&'(') {
                    break;
                }
                j += 1;
            }
            if j < chars.len() {
                // find closing ')'
                let mut k = j + 2;
                let mut depth = 1;
                while k < chars.len() && depth > 0 {
                    if chars[k] == '(' {
                        depth += 1;
                    } else if chars[k] == ')' {
                        depth -= 1;
                    }
                    k += 1;
                }
                let target_raw: String = chars[j + 2..k - 1].iter().collect();
                let target = target_raw.trim();

                let rewritten = if is_image
                    && !target.starts_with("http")
                    && !target.starts_with("data:")
                    && !target.starts_with("epub:")
                {
                    Some(format!("epub://{}", epubthing::resolve_path(base, target)))
                } else if !is_image
                    && !target.starts_with("http")
                    && !target.starts_with("mailto:")
                    && !target.starts_with("data:")
                {
                    let path = target.split('#').next().unwrap_or(target);
                    let resolved = epubthing::resolve_path(base, path);
                    if let Some(idx) = chapter_hrefs.iter().position(|h| *h == resolved) {
                        targets.push((target.to_string(), idx));
                    }
                    None
                } else {
                    None
                };

                // emit prefix `![` / `[`
                out.extend(chars[i..=j].iter());
                out.push('(');
                match rewritten {
                    Some(new_target) => out.push_str(&new_target),
                    None => out.push_str(target),
                }
                out.push(')');
                i = k;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, targets)
}

/// Per-line text-healing (ported from src/html.rs), preserving md block syntax.
fn heal_markdown(markdown: &str) -> String {
    let mut out = String::new();
    for line in markdown.lines() {
        if looks_like_prose(line) {
            out.push_str(&fix_spaced_chars(&remove_definition_separators(line)));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

fn looks_like_prose(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    let c = trimmed.chars().next().unwrap();
    !(c.is_ascii_digit() || " #>*+-`|~[{<".contains(c))
}

fn fix_spaced_chars(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut fixed_words = Vec::with_capacity(words.len());
    let mut i = 0;

    while i < words.len() {
        if is_uppercase_single(words[i]) {
            let mut j = i;
            while j < words.len() && is_uppercase_single(words[j]) {
                j += 1;
            }

            if j < words.len() && is_uppercase_word(words[j]) {
                if j - i > 1 {
                    fixed_words.push(words[i..j - 1].concat());
                }
                fixed_words.push(format!("{}{}", words[j - 1], words[j]));
                i = j + 1;
                continue;
            }

            if j - i >= 3 {
                fixed_words.push(words[i..j].concat());
                i = j;
                continue;
            }
        }

        fixed_words.push(words[i].to_string());
        i += 1;
    }

    fixed_words.join(" ")
}

fn is_uppercase_single(word: &str) -> bool {
    word.len() == 1 && word.as_bytes()[0].is_ascii_uppercase()
}

fn is_uppercase_word(word: &str) -> bool {
    word.len() > 1 && word.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn remove_definition_separators(text: &str) -> String {
    text.split_whitespace()
        .filter_map(|word| {
            if word == "/" {
                None
            } else {
                Some(word.trim_matches('/'))
            }
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl eframe::App for SpikeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(ch) = self.requested.take() {
            if ch != self.current {
                self.current = ch;
                self.rebuild();
            }
        }
        if self.rebuild_next_frame {
            self.rebuild();
            self.rebuild_next_frame = false;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let rect = ui.max_rect();
            ui.painter().rect_filled(rect, 0.0, self.bg);
            self.render(ui);
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "test-epubs/herman-melville_moby-dick_advanced.epub".to_owned());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_title("epubthing md spike"),
        ..Default::default()
    };

    eframe::run_native(
        "epubthing-md-spike",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(SpikeApp::new(&path, &cc.egui_ctx)))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{heal_markdown, rewrite_links_and_images, strip_raw_html};
    use epubthing::EpubDocument;

    #[test]
    fn strips_id_only_anchors() {
        let md = "<a id=\"titlepage\"></a>\n\n# Title\n\n![logo](../images/logo.png)\n";
        let stripped = strip_raw_html(md);
        assert!(!stripped.contains('<'));
        assert!(stripped.contains("# Title"));
        assert!(stripped.contains("![logo](../images/logo.png)"));
    }

    #[test]
    fn rewrites_images_to_epub_scheme_and_resolves() {
        let md = "# T\n\n![log](../images/logo.png)\n\n[Home](chapter-1.xhtml#top)\n\n[web](https://example.com/)\n";
        let chapter_hrefs = vec!["text/chapter-1.xhtml".to_string()];
        let (out, targets) = rewrite_links_and_images(md, "text", &chapter_hrefs);
        assert!(out.contains("epub://images/logo.png"), "{out}");
        assert!(out.contains("https://example.com/"), "{out}");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, "chapter-1.xhtml#top");
        assert_eq!(targets[0].1, 0);
    }

    #[test]
    fn healing_keeps_md_blocks_intact() {
        let md = "# H\n\n> quote / end\n\n- item / x\n\ntext **b** / separator\n";
        let healed = heal_markdown(md);
        assert!(healed.contains("# H"));
        assert!(healed.contains("> quote / end"));
        assert!(healed.contains("- item / x"));
        assert!(healed.contains("text **b**"));
        assert!(!healed.contains("/ separator"));
    }

    #[test]
    fn endnote_style_markdown_survives_pipeline() {
        let md = strip_raw_html(
            "<a id=\"endnotes\"></a>\n\n## Endnotes\n\n1. <a id=\"note-1\"></a>\n\nText. [↩](chapter-24.xhtml#noteref-1)\n",
        );
        let (out, targets) = rewrite_links_and_images(&md, "text", &["text/chapter-24.xhtml".to_string()]);
        assert!(!out.contains("<a "));
        assert!(out.contains("1."), "{out}");
        assert!(targets.iter().any(|(t, i)| t == "chapter-24.xhtml#noteref-1" && *i == 0));
    }

    #[test]
    fn all_chapters_survive_pipeline_without_raw_html() {
        let mut doc = EpubDocument::open("test-epubs/herman-melville_moby-dick_advanced.epub")
            .expect("open epub");
        let chapter_hrefs: Vec<String> = doc
            .spine
            .iter()
            .map(|s| epubthing::resolve_path("", &s.href))
            .collect();
        let spine = doc.spine.clone();

        for ch in &spine {
            let html = doc.get_content(&ch.href).unwrap_or_default();
            let base = ch.href.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            let md = mdka::html_to_markdown(&html);
            let md = strip_raw_html(&md);
            let md = heal_markdown(&md);
            let (out, targets) = rewrite_links_and_images(&md, base, &chapter_hrefs);

            assert!(!out.contains('<'), "raw html survived in {:?}", ch.href);

            for (target, _ti) in &targets {
                if target.starts_with("epub://") {
                    let path = target.trim_start_matches("epub://");
                    assert!(
                        doc.get_asset(path).is_ok(),
                        "image {:?} missing in {:?}",
                        target,
                        ch.href
                    );
                } else {
                    let resolved = epubthing::resolve_path(base, &target.split('#').next().unwrap_or(target));
                    assert!(
                        doc.get_content(&resolved).is_ok()
                            || epubthing::flatten_toc(&doc.toc)
                                .iter()
                                .any(|t| t.href == resolved),
                        "link {:?} from {:?} does not resolve",
                        target,
                        ch.href
                    );
                }
            }
        }
    }
}