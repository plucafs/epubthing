//! Markdown conversion pipeline used by the reader.
//!
//! An EPUB chapter's HTML is converted to Markdown with `mdka`, then passed
//! through three post-processing steps before being rendered with
//! `egui_commonmark`:
//!
//! 1. `strip_raw_html` removes `<...>` fragments that `mdka` leaves in place
//!    (for example id-only anchors); pulldown-cmark would otherwise render
//!    those literally as plain text.
//! 2. `heal_markdown` repairs per-line artifacts typical of EPUB text
//!    (spaced characters, `/` definition separators) while preserving
//!    Markdown block syntax.
//! 3. `rewrite_links_and_images` rewrites `![alt](src)` to `epub://`
//!    resolved paths and resolves internal links against the chapter spine.

/// Strip raw `<...>` html that mdka leaves in the markdown (e.g. id-only anchors).
/// pulldown-cmark would otherwise render those literally as plain text.
pub fn strip_raw_html(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut chars = markdown.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            let mut is_tag = false;
            match chars.peek() {
                Some(n) if n.is_ascii_alphabetic() || *n == '/' || *n == '!' => {
                    is_tag = true;
                    let mut in_quote = false;
                    // scan until matching '>'
                    for ch in chars.by_ref() {
                        if ch == '"' || ch == '\'' {
                            in_quote = !in_quote;
                        }
                        if ch == '>' && !in_quote {
                            break;
                        }
                        if ch != '\n' {
                            tag.push(ch);
                        }
                    }
                }
                _ => {}
            }
            if !is_tag {
                out.push('<');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Rewrites `![alt](src)` image targets to `epub://` resolved paths and returns
/// internal (chapter) link targets ready for link hooks.
///
/// Returns `(markdown, targets)` where each target is `(link, chapter_index)`
/// with `chapter_index` pointing into `chapter_hrefs`.
pub fn rewrite_links_and_images(
    markdown: &str,
    base: &str,
    chapter_hrefs: &[String],
) -> (String, Vec<(String, usize)>) {
    let mut out = String::with_capacity(markdown.len());
    let mut targets = Vec::new();

    let chars: Vec<char> = markdown.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let is_image = chars[i] == '!' && chars.get(i + 1) == Some(&'[');
        let is_link = !is_image && chars[i] == '[';
        if (is_image || is_link) && i + 1 < chars.len() {
            // find closing `](`
            let mut j = i + 1;
            let mut in_quote = false;
            while j < chars.len() {
                if chars[j] == '"' || chars[j] == '\'' {
                    in_quote = !in_quote;
                }
                if !in_quote && chars[j] == ']' && chars.get(j + 1) == Some(&'(') {
                    break;
                }
                j += 1;
            }
            if j < chars.len() {
                // find closing ')'
                let mut k = j + 2;
                let mut depth = 1;
                while k < chars.len() && depth > 0 {
                    if chars[k] == '(' {
                        depth += 1;
                    } else if chars[k] == ')' {
                        depth -= 1;
                    }
                    k += 1;
                }
                let target_raw: String = chars[j + 2..k - 1].iter().collect();
                let target = target_raw.trim();

                let rewritten = if is_image
                    && !target.starts_with("http")
                    && !target.starts_with("data:")
                    && !target.starts_with("epub:")
                {
                    Some(format!("epub://{}", crate::resolve_path(base, target)))
                } else if !is_image
                    && !target.starts_with("http")
                    && !target.starts_with("mailto:")
                    && !target.starts_with("data:")
                {
                    let path = target.split('#').next().unwrap_or(target);
                    let resolved = crate::resolve_path(base, path);
                    if let Some(idx) = chapter_hrefs.iter().position(|h| *h == resolved) {
                        targets.push((target.to_string(), idx));
                    }
                    None
                } else {
                    None
                };

                // emit prefix `![` / `[`
                out.extend(chars[i..=j].iter());
                out.push('(');
                match rewritten {
                    Some(new_target) => out.push_str(&new_target),
                    None => out.push_str(target),
                }
                out.push(')');
                i = k;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, targets)
}

/// Per-line text-healing, preserving Markdown block syntax.
pub fn heal_markdown(markdown: &str) -> String {
    let mut out = String::new();
    for line in markdown.lines() {
        if looks_like_prose(line) {
            out.push_str(&fix_spaced_chars(&remove_definition_separators(line)));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

fn looks_like_prose(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    let c = trimmed.chars().next().unwrap();
    !(c.is_ascii_digit() || " #>*+-`|~[{<".contains(c))
}

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

#[cfg(test)]
mod tests {
    use super::{heal_markdown, rewrite_links_and_images, strip_raw_html};
    use crate::EpubDocument;

    #[test]
    fn strips_id_only_anchors() {
        let md = "<a id=\"titlepage\"></a>\n\n# Title\n\n![logo](../images/logo.png)\n";
        let stripped = strip_raw_html(md);
        assert!(!stripped.contains('<'));
        assert!(stripped.contains("# Title"));
        assert!(stripped.contains("![logo](../images/logo.png)"));
    }

    #[test]
    fn rewrites_images_to_epub_scheme_and_resolves() {
        let md = "# T\n\n![log](../images/logo.png)\n\n[Home](chapter-1.xhtml#top)\n\n[web](https://example.com/)\n";
        let chapter_hrefs = vec!["text/chapter-1.xhtml".to_string()];
        let (out, targets) = rewrite_links_and_images(md, "text", &chapter_hrefs);
        assert!(out.contains("epub://images/logo.png"), "{out}");
        assert!(out.contains("https://example.com/"), "{out}");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, "chapter-1.xhtml#top");
        assert_eq!(targets[0].1, 0);
    }

    #[test]
    fn healing_keeps_md_blocks_intact() {
        let md = "# H\n\n> quote / end\n\n- item / x\n\ntext **b** / separator\n";
        let healed = heal_markdown(md);
        assert!(healed.contains("# H"));
        assert!(healed.contains("> quote / end"));
        assert!(healed.contains("- item / x"));
        assert!(healed.contains("text **b**"));
        assert!(!healed.contains("/ separator"));
    }

    #[test]
    fn endnote_style_markdown_survives_pipeline() {
        let md = strip_raw_html(
            "<a id=\"endnotes\"></a>\n\n## Endnotes\n\n1. <a id=\"note-1\"></a>\n\nText. [↩](chapter-24.xhtml#noteref-1)\n",
        );
        let (out, targets) =
            rewrite_links_and_images(&md, "text", &["text/chapter-24.xhtml".to_string()]);
        assert!(!out.contains("<a "));
        assert!(out.contains("1."), "{out}");
        assert!(targets.iter().any(|(t, i)| t == "chapter-24.xhtml#noteref-1" && *i == 0));
    }

    #[test]
    fn advanced_epub_toc_uses_nav_labels() {
        let doc = EpubDocument::open("test-epubs/herman-melville_moby-dick_advanced.epub")
            .expect("open epub");
        assert!(
            !doc.toc.is_empty(),
            "toc should be populated from the EPUB3 nav document"
        );
        assert_eq!(doc.toc[0].label, "Titlepage");
        assert_eq!(doc.toc[5].label, "Moby Dick");
        assert_eq!(doc.toc[5].href, "text/halftitlepage.xhtml");
        // nested chapter entries are attached as children
        assert_eq!(doc.toc[5].children[0].label, "I: Loomings");
        assert_eq!(doc.toc[5].children[0].href, "text/chapter-1.xhtml");
        assert_eq!(crate::flatten_toc(&doc.toc).len(), 145);
    }

    #[test]
    fn all_chapters_survive_pipeline_without_raw_html() {
        let mut doc = EpubDocument::open("test-epubs/herman-melville_moby-dick_advanced.epub")
            .expect("open epub");
        let chapter_hrefs: Vec<String> = doc
            .spine
            .iter()
            .map(|s| crate::resolve_path("", &s.href))
            .collect();
        let spine = doc.spine.clone();

        for ch in &spine {
            let html = doc.get_content(&ch.href).unwrap_or_default();
            let base = ch.href.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            let md = mdka::html_to_markdown(&html);
            let md = strip_raw_html(&md);
            let md = heal_markdown(&md);
            let (out, targets) = rewrite_links_and_images(&md, base, &chapter_hrefs);

            assert!(!out.contains('<'), "raw html survived in {:?}", ch.href);

            for (target, _ti) in &targets {
                if target.starts_with("epub://") {
                    let path = target.trim_start_matches("epub://");
                    assert!(
                        doc.get_asset(path).is_ok(),
                        "image {:?} missing in {:?}",
                        target,
                        ch.href
                    );
                } else {
                    let resolved = crate::resolve_path(
                        base,
                        target.split('#').next().unwrap_or(target),
                    );
                    assert!(
                        doc.get_content(&resolved).is_ok()
                            || crate::flatten_toc(&doc.toc)
                                .iter()
                                .any(|t| t.href == resolved),
                        "link {:?} from {:?} does not resolve",
                        target,
                        ch.href
                    );
                }
            }
        }
    }
}