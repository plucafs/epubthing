use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use epubthing::TocItem;
use epubthing::{resolve_path, EpubDocument};

/// A chapter loaded into memory for rendering.
#[derive(Clone)]
pub struct Chapter {
    pub label: String,
    pub href: String,
    /// Raw chapter HTML, converted to Markdown for rendering.
    pub html: String,
}

/// One entry of the TOC sidebar tree.
#[derive(Clone)]
pub struct TocEntry {
    pub label: String,
    pub href: String,
    /// Index into `LoadedDocument::chapters`, when this entry maps to a spine item.
    pub chapter: Option<usize>,
    pub children: Vec<TocEntry>,
}

/// A fully loaded EPUB document ready for display.
pub struct LoadedDocument {
    pub title: String,
    pub author: String,
    /// The underlying archive, kept alive for runtime asset access.
    pub raw: Arc<Mutex<EpubDocument>>,
    pub chapters: Vec<Chapter>,
    /// Nested table of contents for the sidebar tree.
    pub toc: Vec<TocEntry>,
}

/// State for a background loading task.
pub struct LoadingTask {
    pub status: String,
    pub cancel: Arc<AtomicBool>,
    pub receiver: mpsc::Receiver<Result<LoadedDocument, String>>,
}

impl LoadingTask {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// Builds a single chapter's content from an EpubDocument.
fn build_chapter(
    doc: &mut EpubDocument,
    href: &str,
    label: String,
) -> Result<Chapter, String> {
    let html = doc
        .get_content(href)
        .map_err(|e| format!("Error loading {}: {}", href, e))?;
    Ok(Chapter {
        label,
        href: href.to_string(),
        html,
    })
}

/// Builds the sidebar tree by resolving each TOC href to a spine chapter index.
fn map_toc_entries(items: &[TocItem], chapter_by_href: &HashMap<String, usize>) -> Vec<TocEntry> {
    items
        .iter()
        .map(|item| TocEntry {
            label: item.label.clone(),
            href: item.href.clone(),
            chapter: chapter_by_href.get(&resolve_path("", &item.href)).copied(),
            children: map_toc_entries(&item.children, chapter_by_href),
        })
        .collect()
}

/// Collects the first label found for each resolved TOC href.
fn toc_label_map(items: &[TocItem], out: &mut HashMap<String, String>) {
    for item in items {
        out.entry(resolve_path("", &item.href))
            .or_insert_with(|| item.label.clone());
        toc_label_map(&item.children, out);
    }
}

/// Loads an EPUB, then sends the full document once all chapters are parsed.
fn load_incremental(
    mut doc: EpubDocument,
    cancel: &AtomicBool,
    tx: &mpsc::Sender<Result<LoadedDocument, String>>,
) {
    let title = doc.metadata.title.clone();
    let author = doc
        .metadata
        .creator
        .clone()
        .unwrap_or_else(|| "Unknown".into());

    let spine: Vec<_> = doc
        .spine
        .iter()
        .map(|s| (s.href.clone(), s.id.clone()))
        .collect();

    if spine.is_empty() {
        let _ = tx.send(Err("EPUB has no content".into()));
        return;
    }

    let chapter_by_href: HashMap<String, usize> = spine
        .iter()
        .enumerate()
        .map(|(i, (href, _))| (resolve_path("", href), i))
        .collect();
    let chapter_resolved: Vec<String> =
        spine.iter().map(|(href, _)| resolve_path("", href)).collect();
    let mut labels = HashMap::new();
    toc_label_map(&doc.toc, &mut labels);

    let mut chapters = Vec::with_capacity(spine.len());

    for (i, (href, _id)) in spine.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            let _ = tx.send(Err("Cancelled".into()));
            return;
        }

        let label = labels
            .get(&chapter_resolved[i])
            .cloned()
            .unwrap_or_else(|| format!("Chapter {}", i + 1));

        let ch = match build_chapter(&mut doc, href, label) {
            Ok(ch) => {
                // Spine chapters the book's own TOC never references keep a
                // placeholder label; give them a real title from their HTML.
                if ch.label.starts_with("Chapter ") {
                    if let Some(title) = epubthing::first_heading_text(&ch.html) {
                        Chapter { label: title, ..ch }
                    } else {
                        ch
                    }
                } else {
                    ch
                }
            }
            Err(_e) => Chapter {
                label: format!("Chapter {}", i + 1),
                href: href.clone(),
                html: "[Error loading chapter]".into(),
            },
        };

        chapters.push(ch);
    }

    let toc = map_toc_entries(&doc.toc, &chapter_by_href);

    let _ = tx.send(Ok(LoadedDocument {
        title,
        author,
        raw: Arc::new(Mutex::new(doc)),
        chapters,
        toc,
    }));
}

/// Spawns a background thread to load an EPUB from a file path.
/// The first chapter is parsed and sent immediately; remaining chapters
/// are parsed afterward and sent as a full replacement document.
pub fn start_loading_from_path(path: &str, cancel: Arc<AtomicBool>) -> LoadingTask {
    let path_owned = path.to_string();
    let filename = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());

    let status = format!("Loading {}...", filename);
    let (tx, rx) = mpsc::channel();
    let thread_cancel = cancel.clone();

    std::thread::spawn(move || {
        let doc = match EpubDocument::open(&path_owned).map_err(|e| format!("{}", e)) {
            Ok(d) => d,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        load_incremental(doc, &thread_cancel, &tx);
    });

    LoadingTask {
        status,
        cancel,
        receiver: rx,
    }
}

/// Spawns a background thread to load an EPUB from bytes.
/// Same incremental strategy as start_loading_from_path.
pub fn start_loading_from_bytes(name: &str, bytes: Vec<u8>, cancel: Arc<AtomicBool>) -> LoadingTask {
    let status = format!("Loading {} ({} bytes)...", name, bytes.len());
    let (tx, rx) = mpsc::channel();
    let thread_cancel = cancel.clone();

    std::thread::spawn(move || {
        let doc = match EpubDocument::from_bytes(bytes).map_err(|e| format!("{}", e)) {
            Ok(d) => d,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        load_incremental(doc, &thread_cancel, &tx);
    });

    LoadingTask {
        status,
        cancel,
        receiver: rx,
    }
}
#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use epubthing::TocItem;
    use epubthing::resolve_path;

    use super::{map_toc_entries, toc_label_map};

    fn item(label: &str, href: &str, children: Vec<TocItem>) -> TocItem {
        TocItem {
            label: label.to_string(),
            href: href.to_string(),
            children,
        }
    }

    #[test]
    fn label_map_prefers_first_label_per_href() {
        let toc = vec![item(
            "Moby Dick",
            "text/halftitlepage.xhtml",
            vec![item("I: Loomings", "text/chapter-1.xhtml", vec![])],
        )];
        let mut map = HashMap::new();
        toc_label_map(&toc, &mut map);
        assert_eq!(
            map.get("text/halftitlepage.xhtml").map(|s| s.as_str()),
            Some("Moby Dick")
        );
        assert_eq!(
            map.get("text/chapter-1.xhtml").map(|s| s.as_str()),
            Some("I: Loomings")
        );
    }

    #[test]
    fn maps_toc_hrefs_to_chapter_indexes() {
        let toc = vec![item(
            "Moby Dick",
            "text/halftitlepage.xhtml",
            vec![item("I: Loomings", "text/chapter-1.xhtml", vec![])],
        )];
        let mut by_href: HashMap<String, usize> = HashMap::new();
        by_href.insert(resolve_path("", "text/halftitlepage.xhtml"), 2);
        by_href.insert(resolve_path("", "text/chapter-1.xhtml"), 3);

        let entries = map_toc_entries(&toc, &by_href);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "Moby Dick");
        assert_eq!(entries[0].chapter, Some(2));
        assert_eq!(entries[0].children[0].label, "I: Loomings");
        assert_eq!(entries[0].children[0].chapter, Some(3));
    }
}
