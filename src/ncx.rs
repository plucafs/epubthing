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

/// Find the EPUB3 nav document (the `nav` property item) from the OPF manifest.
pub(crate) fn find_nav_href(opf: &str) -> Option<String> {
    let package: OpfPackage = from_str(opf).ok()?;
    let manifest = package.manifest?;
    for item in &manifest.item {
        let has_nav_property = item
            .properties
            .as_deref()
            .map(|props| props.split_whitespace().any(|p| p == "nav"))
            .unwrap_or(false);
        if has_nav_property {
            return Some(item.href.clone());
        }
    }
    None
}

/// Parse the TOC `<nav>` of an EPUB3 navigation document into a tree.
/// Only the nav labelled `toc` is considered. Entries keep the nesting of the
/// source `<ol>/<li>` structure.
pub(crate) fn parse_nav_html(html: &str) -> Vec<TocItem> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(html);
    reader.config_mut().trim_text(true);

    let mut items = Vec::new();
    let mut stack: Vec<TocItem> = Vec::new();
    let mut in_toc_nav = false;
    let mut in_link = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "nav" => {
                        let is_toc = e
                            .attributes()
                            .flatten()
                            .any(|attr| {
                                let key = attr.key.as_ref();
                                let value = String::from_utf8_lossy(&attr.value).to_string();
                                (key == b"epub:type" || key == b"type")
                                    && value
                                        .split_whitespace()
                                        .any(|t| t == "toc")
                            });
                        in_toc_nav = is_toc;
                    }
                    "li" if in_toc_nav => {
                        stack.push(TocItem {
                            label: String::new(),
                            href: String::new(),
                            children: vec![],
                        });
                    }
                    "a" if in_toc_nav => {
                        if let Some(item) = stack.last_mut() {
                            in_link = true;
                            item.href = e
                                .attributes()
                                .flatten()
                                .find_map(|attr| {
                                    if attr.key.as_ref() == b"href" {
                                        Some(String::from_utf8_lossy(&attr.value).to_string())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_default();
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                if in_link {
                    if let Some(item) = stack.last_mut() {
                        append_label_part(&mut item.label, &e.unescape().unwrap_or_default());
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "a" if in_link => in_link = false,
                    "li" if in_toc_nav => {
                        if let Some(mut item) = stack.pop() {
                            item.label = item.label.trim().to_string();
                            let has_entry = !item.label.is_empty() && !item.href.is_empty();
                            // Pure container `<li>` (no link) but with children:
                            // lift its children onto the parent instead of dropping them.
                            if has_entry {
                                attach_toc_item(&mut stack, &mut items, item);
                            } else if !item.children.is_empty() {
                                for child in item.children {
                                    attach_toc_item(&mut stack, &mut items, child);
                                }
                            }
                        }
                    }
                    "nav" => in_toc_nav = false,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    items
}

/// Appends a text chunk to a label, inserting a single space only where two
/// word characters meet. This keeps markup spanning in labels readable
/// (e.g. `<span>I</span>: Loomings` -> "I: Loomings").
fn append_label_part(label: &mut String, part: &str) {
    if part.is_empty() {
        return;
    }
    if !label.is_empty() {
        let first = part.chars().next().unwrap();
        let last = label.chars().next_back().unwrap();
        if last.is_alphanumeric() && first.is_alphanumeric() {
            label.push(' ');
        }
    }
    label.push_str(part);
}

/// Attaches a finished item to the top of the stack, or to the roots when the
/// stack is empty.
fn attach_toc_item(stack: &mut Vec<TocItem>, items: &mut Vec<TocItem>, item: TocItem) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(item);
    } else {
        items.push(item);
    }
}

/// Depth-first flattening of a TOC tree, preserving document order.
pub fn flatten_toc(toc: &[TocItem]) -> Vec<&TocItem> {
    let mut flat = Vec::new();
    flatten_toc_into(toc, &mut flat);
    flat
}

fn flatten_toc_into<'a>(toc: &'a [TocItem], out: &mut Vec<&'a TocItem>) {
    for item in toc {
        out.push(item);
        flatten_toc_into(&item.children, out);
    }
}

/// Parse NCX file to get proper chapter titles as a tree following `navPoint`
/// nesting.
pub(crate) fn parse_ncx(xml: &str) -> Vec<TocItem> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut items = Vec::new();
    let mut stack: Vec<TocItem> = Vec::new();
    let mut in_nav_label = false;
    let mut in_text = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "navPoint" => {
                        stack.push(TocItem {
                            label: String::new(),
                            href: String::new(),
                            children: vec![],
                        });
                    }
                    "navLabel" => in_nav_label = true,
                    "text" => in_text = true,
                    "content" => set_ncx_src(&mut stack, e),
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                if in_text && in_nav_label {
                    if let Some(item) = stack.last_mut() {
                        if item.label.is_empty() {
                            item.label = e.unescape().unwrap_or_default().to_string();
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "navLabel" => in_nav_label = false,
                    "text" => in_text = false,
                    "navPoint" => {
                        if let Some(item) = stack.pop() {
                            if !item.label.is_empty() || !item.children.is_empty() {
                                attach_toc_item(&mut stack, &mut items, item);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                if String::from_utf8_lossy(e.name().as_ref()) == "content" {
                    set_ncx_src(&mut stack, e);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    items
}

/// Captures the `src` attribute of an NCX `<content>` element onto the
/// innermost open `navPoint`, keeping the first value seen.
fn set_ncx_src(stack: &mut Vec<TocItem>, e: &quick_xml::events::BytesStart) {
    if let Some(item) = stack.last_mut() {
        if item.href.is_empty() {
            for attr in e.attributes().flatten() {
                if attr.key.as_ref() == b"src" {
                    item.href = String::from_utf8_lossy(&attr.value).to_string();
                }
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::parse_nav_html;

    const SAMPLE_NAV: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc"><h1 id="toc-title">Table of Contents</h1>
      <ol>
        <li><a href="text/titlepage.xhtml">Titlepage</a></li>
        <li><a href="text/chapter-1.xhtml">I: Loomings</a></li>
        <li><a href="text/chapter-2.xhtml">II: The Carpetbag</a>
          <ol><li><a href="text/chapter-2.xhtml#a">Sub A</a></li></ol>
        </li>
      </ol>
    </nav>
    <nav epub:type="landmarks">
      <ol><li><a href="text/titlepage.xhtml">Begin Reading</a></li></ol>
    </nav>
  </body>
</html>
"#;

    #[test]
    fn parses_toc_nav_tree() {
        let items = parse_nav_html(SAMPLE_NAV);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].label, "Titlepage");
        assert_eq!(items[0].href, "text/titlepage.xhtml");
        assert_eq!(items[1].label, "I: Loomings");
        assert_eq!(items[2].label, "II: The Carpetbag");
        assert!(items[2].children.len() == 1, "nested entries should be children");
        assert_eq!(items[2].children[0].label, "Sub A");
        assert_eq!(items[2].children[0].href, "text/chapter-2.xhtml#a");
        assert_eq!(super::flatten_toc(&items).len(), 4);
    }

    #[test]
    fn ignores_landmarks_nav() {
        let items = parse_nav_html(SAMPLE_NAV);
        assert!(items.iter().all(|i| i.label != "Begin Reading"));
    }

    const SAMPLE_NAV_SPAN: &str = r#"
<nav epub:type="toc"><ol>
  <li><a href="text/chapter-1.xhtml"><span epub:type="z3998:roman">I</span>: Loomings</a></li>
  <li><a href="text/chapter-2.xhtml">An <em>Emphasised</em> Title</a></li>
</ol></nav>
"#;

    #[test]
    fn nav_labels_collect_inline_markup() {
        let items = parse_nav_html(SAMPLE_NAV_SPAN);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "I: Loomings");
        assert_eq!(items[1].label, "An Emphasised Title");
    }

    const SAMPLE_NCX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <navMap>
    <navPoint id="np-1" playOrder="1">
      <navLabel><text>Titlepage</text></navLabel>
      <content src="text/titlepage.xhtml"/>
    </navPoint>
    <navPoint id="np-2" playOrder="2">
      <navLabel><text>Moby Dick</text></navLabel>
      <content src="text/halftitlepage.xhtml"/>
      <navPoint id="np-3" playOrder="3">
        <navLabel><text>I: Loomings</text></navLabel>
        <content src="text/chapter-1.xhtml"/>
      </navPoint>
      <navPoint id="np-4" playOrder="4">
        <navLabel><text>II: The Carpetbag</text></navLabel>
        <content src="text/chapter-2.xhtml"/>
      </navPoint>
    </navPoint>
  </navMap>
</ncx>
"#;

    #[test]
    fn parses_ncx_tree() {
        let items = super::parse_ncx(SAMPLE_NCX);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "Titlepage");
        assert_eq!(items[0].href, "text/titlepage.xhtml");
        assert_eq!(items[1].label, "Moby Dick");
        assert_eq!(items[1].href, "text/halftitlepage.xhtml");
        assert_eq!(items[1].children.len(), 2);
        assert_eq!(items[1].children[0].label, "I: Loomings");
        assert_eq!(items[1].children[0].href, "text/chapter-1.xhtml");
        assert_eq!(super::flatten_toc(&items).len(), 4);
    }

    #[test]
    fn flatten_toc_depth_first() {
        let items = parse_nav_html(SAMPLE_NAV);
        let flat = super::flatten_toc(&items);
        let labels: Vec<&str> = flat.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Titlepage",
                "I: Loomings",
                "II: The Carpetbag",
                "Sub A"
            ]
        );
    }
}
