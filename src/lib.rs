mod document;
mod html;
mod markdown;
mod ncx;
mod opf;
mod types;

pub use document::EpubDocument;
pub use html::{first_heading_text, resolve_path};
pub use markdown::{heal_markdown, rewrite_links_and_images, strip_raw_html, trim_leading_blank_lines};
pub use ncx::flatten_toc;
pub use types::{ContentSegment, Metadata, SpineItem, StyledSpan, TocItem};
