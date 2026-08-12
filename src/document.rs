use anyhow::{Context, Result};
use std::io::Read;
use zip::read::ZipArchive;

use crate::html::{parse_html_segments, resolve_path, strip_html};
use crate::ncx::{extract_toc_from_spine, find_ncx_href, parse_ncx};
use crate::opf::{parse_container, parse_opf};
use crate::types::{ContentSegment, Metadata, SpineItem, TocItem};

/// An EPUB document loaded in memory
pub struct EpubDocument {
    archive: ZipArchive<std::io::Cursor<Vec<u8>>>,
    pub metadata: Metadata,
    pub spine: Vec<SpineItem>,
    pub toc: Vec<TocItem>,
    opf_dir: String,
}

impl EpubDocument {
    /// Opens an EPUB file from disk
    pub fn open(path: &str) -> Result<Self> {
        let data = std::fs::read(path).context("Could not read EPUB file")?;
        Self::from_bytes(data)
    }

    /// Opens an EPUB from bytes (e.g. from drag-and-drop)
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let cursor = std::io::Cursor::new(data);
        let mut archive =
            ZipArchive::new(cursor).context("Could not open ZIP archive (EPUB)")?;

        let container = {
            let mut f = archive
                .by_name("META-INF/container.xml")
                .context("container.xml not found in EPUB")?;
            let mut buf = String::new();
            f.read_to_string(&mut buf)?;
            buf
        };

        let opf_path = parse_container(&container)?;

        let opf_dir = std::path::Path::new(&opf_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let opf_content = {
            let mut f = archive
                .by_name(&opf_path)
                .with_context(|| format!("OPF file not found: {}", opf_path))?;
            let mut buf = String::new();
            f.read_to_string(&mut buf)?;
            buf
        };

        let (metadata, spine) = parse_opf(&opf_content)?;

        let ncx_href = find_ncx_href(&opf_content);
        let toc = if let Some(ncx_path) = ncx_href {
            let full_ncx = resolve_path(&opf_dir, &ncx_path);
            if let Ok(mut f) = archive.by_name(&full_ncx) {
                let mut ncx_xml = String::new();
                if f.read_to_string(&mut ncx_xml).is_ok() {
                    parse_ncx(&ncx_xml)
                } else {
                    extract_toc_from_spine(&spine)
                }
            } else {
                extract_toc_from_spine(&spine)
            }
        } else {
            extract_toc_from_spine(&spine)
        };

        Ok(EpubDocument {
            archive,
            metadata,
            spine,
            toc,
            opf_dir,
        })
    }

    /// Returns the text content of a spine item
    pub fn get_content(&mut self, href: &str) -> Result<String> {
        let full_path = resolve_path(&self.opf_dir, href);
        let mut f = self
            .archive
            .by_name(&full_path)
            .with_context(|| format!("File not found: {}", full_path))?;
        let mut buf = String::new();
        f.read_to_string(&mut buf)?;
        Ok(buf)
    }

    /// Returns the cleaned text content (no HTML) of a chapter
    pub fn get_text_content(&mut self, href: &str) -> Result<String> {
        let html = self.get_content(href)?;
        Ok(strip_html(&html))
    }

    /// Returns parsed content segments (text and images) of a spine item
    pub fn get_content_segments(&mut self, href: &str) -> Result<Vec<ContentSegment>> {
        let html = self.get_content(href)?;
        let base_dir = std::path::Path::new(href)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(parse_html_segments(&html, &base_dir))
    }

    /// Returns raw bytes for an image path (relative to the EPUB root)
    pub fn get_image_bytes(&mut self, path: &str) -> Result<Vec<u8>> {
        self.get_asset(path)
    }

    /// Returns the bytes of an image or other binary asset
    pub fn get_asset(&mut self, href: &str) -> Result<Vec<u8>> {
        let full_path = resolve_path(&self.opf_dir, href);
        let mut f = self
            .archive
            .by_name(&full_path)
            .with_context(|| format!("Asset not found: {}", full_path))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Ok(buf)
    }
}
