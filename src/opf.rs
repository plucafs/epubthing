use anyhow::{Context, Result};
use quick_xml::de::from_str;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Deserialize;

use crate::types::{Metadata, SpineItem};

// ─── Parsing container.xml ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename = "container")]
struct Container {
    #[serde(rename = "rootfiles")]
    rootfiles: Rootfiles,
}

#[derive(Debug, Deserialize)]
struct Rootfiles {
    #[serde(rename = "rootfile", default)]
    rootfile: Vec<Rootfile>,
}

#[derive(Debug, Deserialize)]
struct Rootfile {
    #[serde(rename = "@full-path")]
    full_path: String,
    #[serde(rename = "@media-type")]
    media_type: String,
}

pub(crate) fn parse_container(xml: &str) -> Result<String> {
    let container: Container = from_str(xml).context("Error parsing container.xml")?;
    for rf in &container.rootfiles.rootfile {
        if rf.media_type == "application/oebps-package+xml" {
            return Ok(rf.full_path.clone());
        }
    }
    container
        .rootfiles
        .rootfile
        .first()
        .map(|r| r.full_path.clone())
        .context("No rootfile found in container.xml")
}

// ─── Parsing OPF ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename = "package")]
pub(crate) struct OpfPackage {
    #[serde(rename = "spine", default)]
    pub(crate) spine: Option<OpfSpine>,
    #[serde(rename = "manifest", default)]
    pub(crate) manifest: Option<OpfManifest>,
}

/// Extracts the first value of each wanted metadata element that is a direct
/// child of `<metadata>`. Uses a manual reader scan because `quick_xml::de`
/// rejects non-consecutive repeated element names, and OPFs interleave
/// `<meta>`/`<link>` between repeated elements such as `<dc:title>`.
fn parse_metadata(xml: &str) -> Result<Metadata> {
    let mut reader = Reader::from_str(xml);
    let mut out = Metadata::default();

    let mut stack: Vec<String> = Vec::new();
    let capture = |out: &mut Metadata, name: &str, text: &str| {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match name {
            "title" if out.title.is_empty() => out.title.push_str(text),
            "creator" if out.creator.is_none() => out.creator = Some(text.to_owned()),
            "language" if out.language.is_none() => out.language = Some(text.to_owned()),
            "publisher" if out.publisher.is_none() => out.publisher = Some(text.to_owned()),
            "date" if out.date.is_none() => out.date = Some(text.to_owned()),
            "description" if out.description.is_none() => {
                out.description = Some(text.to_owned())
            }
            _ => {}
        }
    };

    loop {
        match reader.read_event() {
            Err(e) => return Err(e).context("Error parsing OPF metadata"),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                stack.push(String::from_utf8_lossy(e.local_name().as_ref()).into_owned());
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "metadata" {
                    continue;
                }
            }
            Ok(Event::Text(t)) => {
                let is_metadata_child = stack
                    .last()
                    .filter(|_| stack.len() >= 2 && stack[stack.len() - 2] == "metadata")
                    .map(String::as_str);
                if let Some(name) = is_metadata_child {
                    if let Ok(text) = t.unescape() {
                        capture(&mut out, name, &text);
                    }
                }
            }
            Ok(Event::End(_)) => {
                stack.pop();
            }
            Ok(_) => {}
        }
    }
    Ok(out)
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct OpfSpine {
    #[serde(rename = "itemref", default)]
    pub(crate) itemref: Vec<OpfItemref>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpfItemref {
    #[serde(rename = "@idref")]
    pub(crate) idref: String,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct OpfManifest {
    #[serde(rename = "item", default)]
    pub(crate) item: Vec<OpfManifestItem>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct OpfManifestItem {
    #[serde(rename = "@id")]
    id: String,
    #[serde(rename = "@href")]
    pub(crate) href: String,
    #[serde(rename = "@media-type")]
    pub(crate) media_type: Option<String>,
    #[serde(rename = "@properties")]
    pub(crate) properties: Option<String>,
}

pub(crate) fn parse_opf(xml: &str) -> Result<(Metadata, Vec<SpineItem>)> {
    let package: OpfPackage = from_str(xml).context("Error parsing OPF file")?;

    let metadata = parse_metadata(xml).context("Error parsing OPF metadata")?;

    let manifest_map: std::collections::HashMap<String, OpfManifestItem> = package
        .manifest
        .unwrap_or_default()
        .item
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect();

    let spine: Vec<SpineItem> = package
        .spine
        .unwrap_or_default()
        .itemref
        .iter()
        .filter_map(|ir| {
            manifest_map.get(&ir.idref).map(|item| SpineItem {
                id: ir.idref.clone(),
                href: item.href.clone(),
                media_type: item.media_type.clone(),
            })
        })
        .collect();

    Ok((metadata, spine))
}

#[cfg(test)]
mod tests {
    use crate::EpubDocument;

    #[test]
    fn parses_metadata_by_local_name() {
        let doc = EpubDocument::open("test-epubs/herman-melville_moby-dick.epub")
            .expect("open test epub");
        assert_eq!(doc.metadata.title, "Moby Dick");
        assert_eq!(doc.metadata.creator.as_deref(), Some("Herman Melville"));
        assert_eq!(doc.metadata.language.as_deref(), Some("en-US"));
        assert_eq!(
            doc.metadata.publisher.as_deref(),
            Some("Standard Ebooks")
        );
        assert_eq!(
            doc.metadata.date.as_deref(),
            Some("2018-03-27T22:02:30Z")
        );
    }
}
