use std::process::ExitCode;

use epubthing::EpubDocument;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: spike_convert <path-to-epub> [chapter-index]");
        return ExitCode::from(2);
    }
    let path = &args[1];
    let chapter_idx: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    let mut doc = match EpubDocument::open(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to open epub: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let href = match doc.spine.get(chapter_idx) {
        Some(item) => item.href.clone(),
        None => {
            eprintln!("Chapter index {} out of range (spine has {} items)", chapter_idx, doc.spine.len());
            return ExitCode::FAILURE;
        }
    };

    let html = match doc.get_content(&href) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to read {}: {}", href, e);
            return ExitCode::FAILURE;
        }
    };

    let mut markdown = mdka::html_to_markdown(&html);
    if args.iter().any(|a| a == "--heal") {
        markdown = heal_markdown(&markdown);
    }

    eprintln!("=== {} chapter[{}] href={} ===", path, chapter_idx, href);
    eprintln!("HTML {} bytes -> Markdown {} bytes", html.len(), markdown.len());

    if args.iter().any(|a| a == "--out") {
        let out = format!("spike-out-ch{}.md", chapter_idx);
        if let Err(e) = std::fs::write(&out, &markdown) {
            eprintln!("Failed to write {}: {}", out, e);
            return ExitCode::FAILURE;
        }
        eprintln!("Wrote {}", out);
        return ExitCode::SUCCESS;
    }

    print!("{}", markdown);
    ExitCode::SUCCESS
}

/// Per-line version of the text-healing hacks used by src/html.rs:
/// fix_spaced_chars + remove_definition_separators. Applied only to lines that
/// look like prose (not markdown block syntax) so md structure survives.
fn heal_markdown(markdown: &str) -> String {
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
    !(c.is_ascii_digit()
        || " #>*+-`|~[{<".contains(c))
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
    use super::{fix_spaced_chars, heal_markdown, remove_definition_separators};

    #[test]
    fn repairs_spaced_uppercase_words() {
        assert_eq!(
            fix_spaced_chars("I C AN B E T HERE FOR M Y C HILDREN"),
            "I CAN BE THERE FOR MY CHILDREN"
        );
        assert_eq!(fix_spaced_chars("a b c"), "a b c");
    }

    #[test]
    fn drops_definition_separator_slashes() {
        assert_eq!(
            remove_definition_separators("t/ Noun: A public thoroughfare. / Noun: Study."),
            "t Noun: A public thoroughfare. Noun: Study."
        );
    }

    #[test]
    fn healing_preserves_markdown_structure() {
        let md = "# Heading\n\n**Bold** *italic*\n\n- item A / item B\n\n- item C\n\n> quote / end\n";
        let healed = heal_markdown(md);
        assert!(healed.contains("# Heading"));
        assert!(healed.contains("**Bold** *italic*"));
        assert!(healed.contains("- item A / item B"));
        assert!(healed.contains("> quote / end"));
    }

    #[test]
    fn healing_drops_slashes_in_plain_paragraph_lines() {
        let md = "Noun: A public thoroughfare. /\n\nNoun: The study of knowledge.";
        let healed = heal_markdown(md);
        assert!(!healed.contains('/'));
        assert!(healed.contains("Noun: A public thoroughfare."));
    }
}