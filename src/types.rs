/// A styled text span
#[derive(Debug, Clone)]
pub struct StyledSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub heading_level: u8,
    pub link_url: Option<String>,
    pub color: Option<[u8; 4]>,
}

impl StyledSpan {
    pub(crate) fn new(
        text: String,
        bold: bool,
        italic: bool,
        underline: bool,
        heading: u8,
    ) -> Self {
        Self {
            text,
            bold,
            italic,
            underline,
            heading_level: heading,
            link_url: None,
            color: None,
        }
    }
}

/// Parsed content segment: styled text or an image reference
#[derive(Debug, Clone)]
pub enum ContentSegment {
    StyledText(Vec<StyledSpan>),
    Image { href: String },
}

#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub title: String,
    pub creator: Option<String>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub date: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SpineItem {
    pub id: String,
    pub href: String,
    pub media_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TocItem {
    pub label: String,
    pub href: String,
    pub children: Vec<TocItem>,
}
