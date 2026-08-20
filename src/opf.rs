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
    #[serde(rename = "dc:title", default)]
    title: Vec<String>,
    #[serde(rename = "dc:creator", default)]
    creator: Vec<String>,
    #[serde(rename = "dc:language", default)]
    language: Vec<String>,
    #[serde(rename = "dc:publisher", default)]
    publisher: Vec<String>,
    #[serde(rename = "dc:date", default)]
    date: Vec<OpfDate>,
    #[serde(rename = "dc:description", default)]
    description: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OpfDate {
    #[serde(rename = "$text")]
    value: String,
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

    let meta = package.metadata.unwrap_or_default();
    let metadata = Metadata {
        title: meta.title.first().cloned().unwrap_or_default(),
        creator: meta.creator.first().cloned(),
        language: meta.language.first().cloned(),
        publisher: meta.publisher.first().cloned(),
        date: meta.date.first().map(|d| d.value.clone()),
        description: meta.description.first().cloned(),
    };

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
