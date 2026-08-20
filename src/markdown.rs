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
/// with `chapter_index` pointing into `chapter_hrefs`. Internal links that do
/// not resolve to a spine chapter are still hooked (to `current_index`) so the
/// reader never falls back to an external-hyperlink render for relative URLs.
/// Links whose visible text exceeds `MAX_LINK_TEXT` characters are flattened to
/// plain text so a long paragraph cannot become one giant clickable block.
pub const MAX_LINK_TEXT: usize = 200;

pub fn rewrite_links_and_images(
    markdown: &str,
    base: &str,
    chapter_hrefs: &[String],
    current_index: usize,
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

                let label: String = chars[i + 1..j].iter().collect();

                // Bug 6: a long link text would render as one huge clickable
                // block (typical of anchors wrapping whole paragraphs).
                // Flatten it to plain text and skip the link entirely.
                if is_link && label.chars().count() > MAX_LINK_TEXT {
                    out.push_str(&label);
                    i = k;
                    continue;
                }

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
                    let idx = chapter_hrefs.iter().position(|h| *h == resolved);
                    // Hook every internal link: spine chapters navigate, and
                    // anything else (same-file `#anchor`, out-of-spine paths) is
                    // still swallowed so no dead external-hyperlink is rendered.
                    targets.push((target.to_string(), idx.unwrap_or(current_index)));
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
        if let Some(cleaned) = strip_alert_token(line) {
            out.push_str(&cleaned);
        } else if is_heading_line(line) {
            out.push_str(&trim_heading_asterisks(line));
        } else if looks_like_prose(line) {
            out.push_str(&fix_spaced_chars(&remove_definition_separators(line)));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// GitHub-flavoured admonition tokens (`> [!WARNING]`) that some EPUBs carry
/// into blockquotes. The reader renders quotes as plain quotes, so the token
/// itself is dropped to avoid leaking "[!WARNING]" into the text.
fn strip_alert_token(line: &str) -> Option<String> {
    let rest = line.split_once('>')?.1.trim_start();
    if !rest.starts_with("[!") {
        return None;
    }
    let ident_end = rest.find(']')?;
    let ident = rest[2..ident_end].to_ascii_uppercase();
    const ALERT_IDENTS: [&str; 5] = ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"];
    if !ALERT_IDENTS.contains(&ident.as_str()) {
        return None;
    }
    Some(format!("> {}", rest[ident_end + 1..].trim_start()))
}

fn is_heading_line(line: &str) -> bool {
    let mut count = 0;
    for c in line.chars() {
        if c == '#' {
            count += 1;
            if count > 6 {
                return false;
            }
        } else {
            return count > 0 && c == ' ';
        }
    }
    count > 0
}

/// Removes any leading blank, whitespace-only or `\u{a0}` lines that `mdka`
/// can emit before the first real content of a chapter (left-overs from
/// stripped `<a id>` anchors and block-level wrappers).
pub fn trim_leading_blank_lines(markdown: &str) -> String {
    let mut start = 0;
    for (idx, c) in markdown.char_indices() {
        if c == '\n' {
            // Every char seen so far was blank, so drop the whole line.
            start = idx + 1;
        } else if c.is_whitespace() || c == '\u{a0}' {
            // Still inside a (possibly) blank prefix line.
        } else {
            break;
        }
    }
    if start == 0 {
        markdown.to_owned()
    } else {
        markdown[start..].to_owned()
    }
}

/// Removes runs of four or more asterisks at the start and end of a heading's
/// content. Such runs are invalid Markdown emphasis (valid emphasis is 2-3
/// asterisks) and are produced when `mdka` nests empty `<strong>`/`<em>`
/// elements inside a heading, rendering as a spurious `******` prefix.
fn trim_heading_asterisks(line: &str) -> String {
    let (hashes, content) = line
        .find(' ')
        .map(|i| (&line[..i], line[i + 1..].trim_start()))
        .unwrap_or((line, ""));

    let mut content = content.to_owned();
    loop {
        let leading = content.chars().take_while(|c| *c == '*').count();
        if leading >= 4 {
            content = content[leading..].trim_start().to_owned();
        } else {
            break;
        }
    }
    loop {
        let trailing = content.chars().rev().take_while(|c| *c == '*').count();
        if trailing >= 4 {
            let keep = content.chars().count() - trailing;
            content = content.chars().take(keep).collect::<String>();
        } else {
            break;
        }
    }

    let mut out = String::with_capacity(line.len());
    out.push_str(hashes);
    if !content.is_empty() {
        out.push(' ');
        out.push_str(content.trim_end());
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
    use super::{heal_markdown, rewrite_links_and_images, strip_raw_html, trim_leading_blank_lines};
    use crate::EpubDocument;

    #[test]
    fn trims_leading_blank_lines() {
        assert_eq!(trim_leading_blank_lines("\n\n## Title\n\nBody\n"), "## Title\n\nBody\n");
    }

    #[test]
    fn trims_whitespace_only_and_nbsp_prefix_lines() {
        let md = " \t\n\u{a0}\u{a0}\n# Heading\n\nText\n";
        assert_eq!(trim_leading_blank_lines(md), "# Heading\n\nText\n");
    }

    #[test]
    fn keeps_leading_content_and_trailing_structure() {
        let md = "starts with text\n\nBody\n";
        assert_eq!(trim_leading_blank_lines(md), md);
        let md = "\n# Title\n";
        assert_eq!(trim_leading_blank_lines(md), "# Title\n");
    }

    #[test]
    fn strips_github_alert_tokens_from_quotes() {
        let md = "> [!WARNING]\n> Attenzione: something bad.\n\n> [!NOTE] A normal note.\n\n> plain quote\n";
        let healed = heal_markdown(md);
        assert!(healed.contains("> Attenzione: something bad."), "{healed}");
        assert!(healed.contains("> A normal note."), "{healed}");
        assert!(!healed.contains("[!"), "{healed}");
        assert!(healed.contains("> plain quote"), "{healed}");
    }

    #[test]
    fn keeps_non_alert_quotes_and_unknown_tokens() {
        let md = "> [!THING] not an alert\n";
        let healed = heal_markdown(md);
        assert_eq!(healed, md);
    }

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
        let (out, targets) = rewrite_links_and_images(md, "text", &chapter_hrefs, 0);
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
            rewrite_links_and_images(&md, "text", &["text/chapter-24.xhtml".to_string()], 0);
        assert!(!out.contains("<a "));
        assert!(out.contains("1."), "{out}");
        assert!(targets.iter().any(|(t, i)| t == "chapter-24.xhtml#noteref-1" && *i == 0));
    }

    #[test]
    fn out_of_spine_link_is_hooked_to_current_chapter() {
        let md = "See [note 1](#note-1) and [again](missing.xhtml).\n";
        let (out, targets) = rewrite_links_and_images(md, "text", &[], 7);
        // no dead external hyperlinks are left in place
        assert_eq!(out, md);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0], ("#note-1".to_string(), 7));
        assert_eq!(targets[1], ("missing.xhtml".to_string(), 7));
    }

    #[test]
    fn long_link_text_is_flattened_to_plain_text() {
        let long = "x".repeat(300);
        let md = format!("Some intro [{}](missing.xhtml) more.\n", long);
        let (out, targets) = rewrite_links_and_images(&md, "text", &[], 0);
        assert!(!out.contains("["), "{out}");
        assert!(!out.contains("missing.xhtml"), "{out}");
        assert!(targets.is_empty());
    }

    #[test]
    fn heading_asterisk_runs_are_trimmed() {
        let md = "## ******Moby Dick\n\n### Some **** title **\n\n# **Valid bold**\n";
        let healed = heal_markdown(md);
        assert_eq!(
            healed,
            "## Moby Dick\n\n### Some **** title **\n\n# **Valid bold**\n"
        );
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
            let (out, targets) = rewrite_links_and_images(&md, base, &chapter_hrefs, 0);

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