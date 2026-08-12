use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use epubthing::{ContentSegment, EpubDocument, StyledSpan};

/// A chapter loaded into memory for rendering.
#[derive(Clone)]
pub struct Chapter {
    pub label: String,
    pub href: String,
    pub segments: Vec<ContentSegment>,
    pub image_data: HashMap<String, Vec<u8>>,
    pub image_errors: HashMap<String, String>,
}

/// A fully loaded EPUB document ready for display.
pub struct LoadedDocument {
    pub title: String,
    pub author: String,
    pub chapters: Vec<Chapter>,
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
    match doc.get_content_segments(href) {
        Ok(segments) => {
            let mut image_data = HashMap::new();
            let mut image_errors = HashMap::new();
            for seg in &segments {
                if let ContentSegment::Image { href: img_href } = seg {
                    match doc.get_image_bytes(img_href) {
                        Ok(bytes) => {
                            if let Err(error) = image::load_from_memory(&bytes) {
                                image_errors.insert(
                                    img_href.clone(),
                                    format!("Image decode failed: {}", error),
                                );
                            } else {
                                image_data.insert(img_href.clone(), bytes);
                            }
                        }
                        Err(error) => {
                            image_errors.insert(img_href.clone(), error.to_string());
                        }
                    }
                }
            }
            Ok(Chapter {
                label,
                href: href.to_string(),
                segments,
                image_data,
                image_errors,
            })
        }
        Err(e) => Ok(Chapter {
            label,
            href: href.to_string(),
            segments: vec![ContentSegment::StyledText(vec![StyledSpan {
                text: format!("[Error: {}]", e),
                bold: false,
                italic: false,
                underline: false,
                heading_level: 0,
                link_url: None,
                color: None,
            }])],
            image_data: HashMap::new(),
            image_errors: HashMap::new(),
        }),
    }
}

/// Loads an EPUB incrementally: parses the first chapter immediately and sends
/// a partial result, then parses the remaining chapters and sends the full document.
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
    let toc_len = doc.toc.len();

    if spine.is_empty() {
        let _ = tx.send(Err("EPUB has no content".into()));
        return;
    }

    // ── Phase 1: parse only the first chapter ──
    if cancel.load(Ordering::SeqCst) {
        let _ = tx.send(Err("Cancelled".into()));
        return;
    }

    let first_label = if toc_len > 0 {
        doc.toc[0].label.clone()
    } else {
        "Chapter 1".into()
    };

    let first_chapter = match build_chapter(&mut doc, &spine[0].0, first_label) {
        Ok(ch) => ch,
        Err(e) => {
            let _ = tx.send(Err(e));
            return;
        }
    };

    let _ = tx.send(Ok(LoadedDocument {
        title: title.clone(),
        author: author.clone(),
        chapters: vec![first_chapter.clone()],
    }));

    // ── Phase 2: parse remaining chapters ──
    let mut chapters: Vec<Chapter> = vec![first_chapter];

    for (i, (href, _id)) in spine.iter().enumerate().skip(1) {
        if cancel.load(Ordering::SeqCst) {
            let _ = tx.send(Err("Cancelled".into()));
            return;
        }

        let label = if i < toc_len {
            doc.toc[i].label.clone()
        } else {
            format!("Chapter {}", i + 1)
        };

        let ch = match build_chapter(&mut doc, href, label) {
            Ok(ch) => ch,
            Err(_e) => Chapter {
                label: format!("Chapter {}", i + 1),
                href: href.clone(),
                segments: vec![ContentSegment::StyledText(vec![StyledSpan {
                    text: "[Error loading chapter]".into(),
                    bold: false,
                    italic: false,
                    underline: false,
                    heading_level: 0,
                    link_url: None,
                    color: None,
                }])],
                image_data: HashMap::new(),
                image_errors: HashMap::new(),
            },
        };

        chapters.push(ch);
    }

    let _ = tx.send(Ok(LoadedDocument {
        title,
        author,
        chapters,
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
