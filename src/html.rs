use crate::types::{ContentSegment, StyledSpan};

pub fn resolve_path(base: &str, href: &str) -> String {
    let href = href.split(['?', '#']).next().unwrap_or_default();
    let mut parts = Vec::new();

    if !href.starts_with('/') {
        for component in base.split('/') {
            append_path_component(&mut parts, component);
        }
    }

    for component in href.split('/') {
        append_path_component(&mut parts, component);
    }

    parts.join("/")
}

fn append_path_component(parts: &mut Vec<String>, component: &str) {
    match component {
        "" | "." => {}
        ".." => {
            parts.pop();
        }
        component => parts.push(percent_decode(component)),
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[index + 1]), hex_value(bytes[index + 2])) {
                decoded.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// Split XHTML content into styled text segments and images
pub(crate) fn parse_html_segments(
    html: &str,
    base_dir: &str,
) -> Vec<ContentSegment> {
    let mut segments = Vec::new();
    let mut current_spans: Vec<StyledSpan> = Vec::new();
    let mut bold = false;
    let mut italic = false;
    let mut underline = false;
    let mut heading = 0u8;
    let mut link_url: Option<String> = None;

    let mut remaining = html;

    loop {
        let tag_start = remaining.find('<');
        let img_start = remaining.to_lowercase().find("<img");

        if let (Some(pos), Some(tag_pos)) = (img_start, tag_start) {
            if pos == tag_pos {
                if !current_spans.is_empty() {
                    segments.push(ContentSegment::StyledText(std::mem::take(
                        &mut current_spans,
                    )));
                }

                let after_img = &remaining[pos..];
                let img_end = after_img
                    .find('>')
                    .map(|p| p + 1)
                    .unwrap_or(after_img.len());
                let img_tag = &after_img[..img_end];

                if let Some(src) = extract_attr(img_tag, "src") {
                    let full_src = if src.starts_with('/') {
                        resolve_path("", &src)
                    } else {
                        resolve_path(base_dir, &src)
                    };
                    segments.push(ContentSegment::Image { href: full_src });
                }

                remaining = &after_img[img_end..];
                continue;
            }
        }

        match tag_start {
            None => {
                push_text(
                    remaining,
                    bold,
                    italic,
                    underline,
                    heading,
                    &link_url,
                    &mut current_spans,
                );
                if !current_spans.is_empty() {
                    segments.push(ContentSegment::StyledText(std::mem::take(
                        &mut current_spans,
                    )));
                }
                break;
            }
            Some(pos) => {
                if pos > 0 {
                    push_text(
                        &remaining[..pos],
                        bold,
                        italic,
                        underline,
                        heading,
                        &link_url,
                        &mut current_spans,
                    );
                }

                let after = &remaining[pos..];
                let tag_end = after.find('>').map(|p| p + 1).unwrap_or(after.len());
                let tag = &after[..tag_end];
                let lower = tag.to_lowercase();

                if lower.starts_with("<br") || lower.starts_with("<hr") {
                    current_spans.push(StyledSpan::new(
                        "\n".into(),
                        bold,
                        italic,
                        underline,
                        heading,
                    ));
                    remaining = &after[tag_end..];
                    continue;
                }

                if is_block_boundary(&lower)
                    && current_spans
                        .last()
                        .map_or(false, |span| !span.text.ends_with('\n'))
                {
                    current_spans.push(StyledSpan::new("\n".into(), false, false, false, 0));
                }

                if lower.starts_with("<b>") || lower.starts_with("<strong>") {
                    bold = true;
                } else if lower.starts_with("</b>") || lower.starts_with("</strong>") {
                    bold = false;
                } else if lower.starts_with("<i>") || lower.starts_with("<em>") {
                    italic = true;
                } else if lower.starts_with("</i>") || lower.starts_with("</em>") {
                    italic = false;
                } else if lower.starts_with("<u>") {
                    underline = true;
                } else if lower.starts_with("</u>") {
                    underline = false;
                } else if lower.starts_with("<h1") {
                    heading = 1;
                } else if lower.starts_with("</h1") {
                    heading = 0;
                } else if lower.starts_with("<h2") {
                    heading = 2;
                } else if lower.starts_with("</h2") {
                    heading = 0;
                } else if lower.starts_with("<h3") {
                    heading = 3;
                } else if lower.starts_with("</h3") {
                    heading = 0;
                } else if lower.starts_with("<h4") {
                    heading = 4;
                } else if lower.starts_with("</h4") {
                    heading = 0;
                } else if lower.starts_with("<a ") {
                    link_url = extract_attr(tag, "href");
                } else if lower.starts_with("</a>") {
                    link_url = None;
                }

                remaining = &after[tag_end..];
            }
        }
    }

    segments
}

fn push_text(
    html: &str,
    bold: bool,
    italic: bool,
    underline: bool,
    heading: u8,
    link_url: &Option<String>,
    spans: &mut Vec<StyledSpan>,
) {
    let text = decode_entities(html);
    if text.trim().is_empty() {
        if !text.contains('\n') && !text.contains('\r') && !spans.is_empty() {
            if spans.last().map_or(false, |span| !span.text.ends_with('\n')) {
                spans.push(StyledSpan::new(
                    " ".into(),
                    bold,
                    italic,
                    underline,
                    heading,
                ));
            }
        }
        return;
    }

    let fixed_text = remove_definition_separators(&fix_spaced_chars(&text));
    let trimmed = fixed_text.trim();
    if trimmed.is_empty() {
        return;
    }

    let has_leading_space = text.chars().next().map_or(false, char::is_whitespace)
        && spans
            .last()
            .map_or(false, |span| !span.text.ends_with('\n'));
    let has_trailing_space = text.chars().last().map_or(false, char::is_whitespace)
        && !text.contains('\n')
        && !text.contains('\r');
    let mut normalized = String::new();
    if has_leading_space {
        normalized.push(' ');
    }
    normalized.push_str(trimmed);
    if has_trailing_space {
        normalized.push(' ');
    }

    spans.push(StyledSpan {
        text: normalized,
        bold,
        italic,
        underline,
        heading_level: heading,
        link_url: link_url.clone(),
        color: None,
    });
}

fn is_block_boundary(tag: &str) -> bool {
    tag.starts_with("<p")
        || tag.starts_with("</p")
        || tag.starts_with("<div")
        || tag.starts_with("</div")
        || tag.starts_with("<h1")
        || tag.starts_with("<h2")
        || tag.starts_with("<h3")
        || tag.starts_with("<h4")
        || tag.starts_with("<h5")
        || tag.starts_with("<h6")
        || tag.starts_with("<li")
        || tag.starts_with("</li")
        || tag.starts_with("<br")
        || tag.starts_with("<hr")
}

fn decode_entities(html: &str) -> String {
    html.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

/// Fix strangely spaced characters (e.g., "I C AN B E T HERE" -> "I CAN BE THERE")
/// This handles EPUBs where characters are incorrectly separated by spaces.
fn fix_spaced_chars(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut fixed_words = Vec::with_capacity(words.len());
    let mut i = 0;

    while i < words.len() {
        if is_uppercase_single(words[i]) {
            let mut j = i;
            while j < words.len() && is_uppercase_single(words[j]) {
                j += 1;
            }

            if j < words.len() && is_uppercase_word(words[j]) {
                if j - i > 1 {
                    fixed_words.push(words[i..j - 1].concat());
                }
                fixed_words.push(format!("{}{}", words[j - 1], words[j]));
                i = j + 1;
                continue;
            }

            if j - i >= 3 {
                fixed_words.push(words[i..j].concat());
                i = j;
                continue;
            }
        }

        fixed_words.push(words[i].to_string());
        i += 1;
    }

    fixed_words.join(" ")
}

fn is_uppercase_single(word: &str) -> bool {
    word.len() == 1 && word.as_bytes()[0].is_ascii_uppercase()
}

fn is_uppercase_word(word: &str) -> bool {
    word.len() > 1 && word.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn remove_definition_separators(text: &str) -> String {
    text.split_whitespace()
        .filter_map(|word| {
            if word == "/" {
                None
            } else {
                Some(word.trim_matches('/'))
            }
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let attr_lower = attr.to_ascii_lowercase();
    let mut start = 0;

    let value_start = loop {
        let relative_start = lower[start..].find(&attr_lower)?;
        let attribute_start = start + relative_start;
        let has_boundary = attribute_start == 0
            || !lower.as_bytes()[attribute_start - 1].is_ascii_alphanumeric();
        if !has_boundary {
            start = attribute_start + attr_lower.len();
            continue;
        }

        let mut value_start = attribute_start + attr_lower.len();
        while lower.as_bytes().get(value_start).is_some_and(u8::is_ascii_whitespace) {
            value_start += 1;
        }
        if lower.as_bytes().get(value_start) != Some(&b'=') {
            start = attribute_start + attr_lower.len();
            continue;
        }
        value_start += 1;
        while lower.as_bytes().get(value_start).is_some_and(u8::is_ascii_whitespace) {
            value_start += 1;
        }
        break value_start;
    };

    let after_eq = &tag[value_start..];
    let delim = after_eq.chars().next()?;
    if delim == '"' || delim == '\'' {
        let inner = &after_eq[1..];
        let end = inner.find(delim)?;
        Some(inner[..end].to_string())
    } else {
        let end = after_eq.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
        Some(after_eq[..end].to_string())
    }
}

/// Remove HTML tags and return plain text
pub(crate) fn strip_html(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut in_style_or_script = false;
    let mut last_was_newline = false;

    let chars: Vec<char> = html.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        if !in_tag && chars[i] == '<' {
            in_tag = true;

            let remaining: String = chars[i..].iter().collect();
            let tag_lower = remaining.to_lowercase();

            if tag_lower.starts_with("</style") || tag_lower.starts_with("</script") {
                in_style_or_script = false;
            } else if tag_lower.starts_with("<style") || tag_lower.starts_with("<script") {
                in_style_or_script = true;
            }

            if !last_was_newline {
                if tag_lower.starts_with("<br")
                    || tag_lower.starts_with("<p")
                    || tag_lower.starts_with("</p")
                    || tag_lower.starts_with("<div")
                    || tag_lower.starts_with("</div")
                    || tag_lower.starts_with("<h")
                    || tag_lower.starts_with("</h")
                    || tag_lower.starts_with("<li")
                    || tag_lower.starts_with("</li")
                    || tag_lower.starts_with("<tr")
                    || tag_lower.starts_with("</tr")
                    || tag_lower.starts_with("<hr")
                {
                    result.push('\n');
                    last_was_newline = true;
                }
            }

            i += 1;
            continue;
        }

        if in_tag {
            if chars[i] == '>' {
                in_tag = false;
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }

        if in_style_or_script {
            i += 1;
            continue;
        }

        if chars[i] == '&' {
            let remaining: String = chars[i..].iter().collect();
            if remaining.starts_with("&amp;") {
                result.push('&');
                i += 5;
                continue;
            } else if remaining.starts_with("&lt;") {
                result.push('<');
                i += 4;
                continue;
            } else if remaining.starts_with("&gt;") {
                result.push('>');
                i += 4;
                continue;
            } else if remaining.starts_with("&quot;") {
                result.push('"');
                i += 6;
                continue;
            } else if remaining.starts_with("&apos;") {
                result.push('\'');
                i += 6;
                continue;
            } else if remaining.starts_with("&nbsp;") {
                result.push(' ');
                i += 6;
                continue;
            }
        }

        if last_was_newline
            && (chars[i] == ' ' || chars[i] == '\t' || chars[i] == '\n' || chars[i] == '\r')
        {
            i += 1;
            continue;
        }

        result.push(chars[i]);
        if chars[i] != ' ' && chars[i] != '\n' && chars[i] != '\r' && chars[i] != '\t' {
            last_was_newline = false;
        }
        i += 1;
    }

    let mut cleaned = String::new();
    let mut prev_was_newline = false;
    for ch in result.chars() {
        if ch == '\n' {
            if !prev_was_newline {
                cleaned.push(ch);
                prev_was_newline = true;
            }
        } else {
            cleaned.push(ch);
            prev_was_newline = false;
        }
    }

    cleaned.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{parse_html_segments, resolve_path};
    use crate::types::ContentSegment;

    fn flattened_text(html: &str) -> String {
        parse_html_segments(html, "")
            .into_iter()
            .filter_map(|segment| match segment {
                ContentSegment::StyledText(spans) => {
                    Some(spans.into_iter().map(|span| span.text).collect::<String>())
                }
                ContentSegment::Image { .. } => None,
            })
            .collect()
    }

    #[test]
    fn resolves_relative_paths_and_parent_components() {
        assert_eq!(
            resolve_path("OEBPS/text", "../images/cover.jpg"),
            "OEBPS/images/cover.jpg"
        );
        assert_eq!(
            resolve_path("OEBPS/text", "./chapter.xhtml"),
            "OEBPS/text/chapter.xhtml"
        );
    }

    #[test]
    fn strips_fragment_and_query_from_archive_paths() {
        assert_eq!(
            resolve_path("OEBPS", "text/chapter.xhtml#note-1?ignored=true"),
            "OEBPS/text/chapter.xhtml"
        );
        assert_eq!(
            resolve_path("OEBPS", "images/cover.jpg?cache=1#cover"),
            "OEBPS/images/cover.jpg"
        );
    }

    #[test]
    fn decodes_encoded_path_components() {
        assert_eq!(
            resolve_path("OEBPS", "images/My%20Cover%20%28front%29.jpg"),
            "OEBPS/images/My Cover (front).jpg"
        );
    }

    #[test]
    fn root_relative_paths_ignore_the_base() {
        assert_eq!(
            resolve_path("OEBPS/text", "/Images/cover.jpg"),
            "Images/cover.jpg"
        );
    }

    #[test]
    fn preserves_spaces_between_inline_elements() {
        assert_eq!(
            flattened_text("<p>Hello <em>world</em>!</p>"),
            "Hello world!\n"
        );
    }

    #[test]
    fn repairs_uppercase_spaced_characters_without_joining_normal_words() {
        assert_eq!(
            flattened_text("I C AN B E T HERE FOR M Y C HILDREN"),
            "I CAN BE THERE FOR MY CHILDREN"
        );
        assert_eq!(flattened_text("a b c"), "a b c");
    }

    #[test]
    fn removes_definition_separator_slashes_and_keeps_blocks_separate() {
        let text = flattened_text(
            "<p>CHAPTER I</p><p>STREET EPISTEMOLOGY</p><p>t/</p><p>Noun: A public thoroughfare.</p><p>/</p><p>Noun: The study of knowledge.</p>",
        );
        assert!(!text.contains('/'));
        assert!(text.contains("Noun: A public thoroughfare."));
        assert!(text.contains("Noun: The study of knowledge."));
    }

    #[test]
    fn accepts_whitespace_around_image_attributes() {
        let segments = parse_html_segments(
            "<p>Before</p><img alt='cover' src = \"images/cover%20art.jpg\" />",
            "OEBPS/text",
        );
        assert!(segments.iter().any(|segment| {
            matches!(segment, ContentSegment::Image { href } if href == "OEBPS/text/images/cover art.jpg")
        }));
    }
}
