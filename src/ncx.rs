use quick_xml::de::from_str;

use crate::opf::OpfPackage;
use crate::types::{SpineItem, TocItem};

/// Find NCX file path from OPF manifest
pub(crate) fn find_ncx_href(opf: &str) -> Option<String> {
    let package: OpfPackage = from_str(opf).ok()?;
    let manifest = package.manifest?;
    for item in &manifest.item {
        if item.media_type.as_deref() == Some("application/x-dtbncx+xml") {
            return Some(item.href.clone());
        }
    }
    None
}

/// Parse NCX file to get proper chapter titles
pub(crate) fn parse_ncx(xml: &str) -> Vec<TocItem> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut items = Vec::new();
    let mut current_label = String::new();
    let mut current_src = String::new();
    let mut in_nav_label = false;
    let mut in_text = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "navPoint" => {
                        current_label.clear();
                        current_src.clear();
                    }
                    "navLabel" => in_nav_label = true,
                    "text" => in_text = true,
                    _ => {}
                }
                if name == "content" {
                    for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"src" {
                                current_src = String::from_utf8_lossy(&attr.value).to_string();
                            }
                    }
                }
            }
            Ok(Event::Text(ref e)) => {
                if in_text && in_nav_label {
                    current_label = e.unescape().unwrap_or_default().to_string();
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "navLabel" => in_nav_label = false,
                    "text" => in_text = false,
                    "navPoint" => {
                        if !current_label.is_empty() {
                            items.push(TocItem {
                                label: current_label.clone(),
                                href: current_src.clone(),
                                children: vec![],
                            });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "content" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"src" {
                            current_src = String::from_utf8_lossy(&attr.value).to_string();
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    items
}

/// Fallback: build TOC from spine ids
pub(crate) fn extract_toc_from_spine(spine: &[SpineItem]) -> Vec<TocItem> {
    spine
        .iter()
        .map(|item| TocItem {
            label: item.id.clone(),
            href: item.href.clone(),
            children: vec![],
        })
        .collect()
}
