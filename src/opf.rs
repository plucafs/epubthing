use anyhow::{Context, Result};
use quick_xml::de::from_str;
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
    #[serde(rename = "metadata", default)]
    pub(crate) metadata: Option<OpfMetadata>,
    #[serde(rename = "spine", default)]
    pub(crate) spine: Option<OpfSpine>,
    #[serde(rename = "manifest", default)]
    pub(crate) manifest: Option<OpfManifest>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct OpfMetadata {
    #[serde(rename = "title", default)]
    title: Vec<String>,
    #[serde(rename = "creator", default)]
    creator: Vec<String>,
    #[serde(rename = "language", default)]
    language: Vec<String>,
    #[serde(rename = "publisher", default)]
    publisher: Vec<String>,
    #[serde(rename = "date", default)]
    date: Vec<String>,
    #[serde(rename = "description", default)]
    description: Vec<String>,
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
}

pub(crate) fn parse_opf(xml: &str) -> Result<(Metadata, Vec<SpineItem>)> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut metadata = Metadata::default();
    let mut spine_idrefs: Vec<String> = Vec::new();
    let mut manifest_items: std::collections::HashMap<String, OpfManifestItem> = std::collections::HashMap::new();

    let mut in_metadata = false;
    let mut in_spine = false;
    let mut in_manifest = false;

    let mut current_title = String::new();
    let mut current_creator = String::new();
    let mut current_language = String::new();
    let mut current_publisher = String::new();
    let mut current_date = String::new();
    let mut current_description = String::new();

    let mut in_dc_title = false;
    let mut in_dc_creator = false;
    let mut in_dc_language = false;
    let mut in_dc_publisher = false;
    let mut in_dc_date = false;
    let mut in_dc_description = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "metadata" => in_metadata = true,
                    "spine" => in_spine = true,
                    "manifest" => in_manifest = true,
                    "dc:title" if in_metadata => {
                        in_dc_title = true;
                        current_title.clear();
                    }
                    "dc:creator" if in_metadata => {
                        in_dc_creator = true;
                        current_creator.clear();
                    }
                    "dc:language" if in_metadata => {
                        in_dc_language = true;
                        current_language.clear();
                    }
                    "dc:publisher" if in_metadata => {
                        in_dc_publisher = true;
                        current_publisher.clear();
                    }
                    "dc:date" if in_metadata => {
                        in_dc_date = true;
                        current_date.clear();
                    }
                    "dc:description" if in_metadata => {
                        in_dc_description = true;
                        current_description.clear();
                    }
                    "itemref" if in_spine => {
                        let mut idref = String::new();
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"idref" {
                                idref = String::from_utf8_lossy(&attr.value).to_string();
                            }
                        }
                        if !idref.is_empty() {
                            spine_idrefs.push(idref);
                        }
                    }
                    "item" if in_manifest => {
                        let mut id = String::new();
                        let mut href = String::new();
                        let mut media_type = None;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"id" => id = String::from_utf8_lossy(&attr.value).to_string(),
                                b"href" => href = String::from_utf8_lossy(&attr.value).to_string(),
                                b"media-type" => {
                                    media_type = Some(String::from_utf8_lossy(&attr.value).to_string())
                                }
                                _ => {}
                            }
                        }
                        if !id.is_empty() {
                            manifest_items.insert(
                                id.clone(),
                                OpfManifestItem {
                                    id,
                                    href,
                                    media_type,
                                },
                            );
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "itemref" if in_spine => {
                        let mut idref = String::new();
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"idref" {
                                idref = String::from_utf8_lossy(&attr.value).to_string();
                            }
                        }
                        if !idref.is_empty() {
                            spine_idrefs.push(idref);
                        }
                    }
                    "item" if in_manifest => {
                        let mut id = String::new();
                        let mut href = String::new();
                        let mut media_type = None;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"id" => id = String::from_utf8_lossy(&attr.value).to_string(),
                                b"href" => href = String::from_utf8_lossy(&attr.value).to_string(),
                                b"media-type" => {
                                    media_type = Some(String::from_utf8_lossy(&attr.value).to_string())
                                }
                                _ => {}
                            }
                        }
                        if !id.is_empty() {
                            manifest_items.insert(
                                id.clone(),
                                OpfManifestItem {
                                    id,
                                    href,
                                    media_type,
                                },
                            );
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                if in_dc_title {
                    current_title = text;
                } else if in_dc_creator {
                    current_creator = text;
                } else if in_dc_language {
                    current_language = text;
                } else if in_dc_publisher {
                    current_publisher = text;
                } else if in_dc_date {
                    current_date = text;
                } else if in_dc_description {
                    current_description = text;
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "metadata" => in_metadata = false,
                    "spine" => in_spine = false,
                    "manifest" => in_manifest = false,
                    "dc:title" if in_metadata => {
                        in_dc_title = false;
                        if metadata.title.is_empty() {
                            metadata.title = current_title.clone();
                        }
                    }
                    "dc:creator" if in_metadata => {
                        in_dc_creator = false;
                        if metadata.creator.is_none() {
                            metadata.creator = Some(current_creator.clone());
                        }
                    }
                    "dc:language" if in_metadata => {
                        in_dc_language = false;
                        if metadata.language.is_none() {
                            metadata.language = Some(current_language.clone());
                        }
                    }
                    "dc:publisher" if in_metadata => {
                        in_dc_publisher = false;
                        if metadata.publisher.is_none() {
                            metadata.publisher = Some(current_publisher.clone());
                        }
                    }
                    "dc:date" if in_metadata => {
                        in_dc_date = false;
                        if metadata.date.is_none() {
                            metadata.date = Some(current_date.clone());
                        }
                    }
                    "dc:description" if in_metadata => {
                        in_dc_description = false;
                        if metadata.description.is_none() {
                            metadata.description = Some(current_description.clone());
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("Error parsing OPF file: {}", e)),
            _ => {}
        }
    }

    let spine: Vec<SpineItem> = spine_idrefs
        .iter()
        .filter_map(|idref| {
            manifest_items.get(idref).map(|item| SpineItem {
                id: idref.clone(),
                href: item.href.clone(),
                media_type: item.media_type.clone(),
            })
        })
        .collect();

    Ok((metadata, spine))
}
