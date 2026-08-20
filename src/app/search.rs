#![allow(dead_code)] // Reimplemented on markdown; kept for the upcoming rewrite.
use std::collections::HashMap;

use epubthing::{ContentSegment, StyledSpan};

/// Data about a single chapter needed by search operations.
pub struct ChapterSearchData<'a> {
    pub segments: &'a [ContentSegment],
}

/// In-chapter search state.
pub struct SearchState {
    pub show: bool,
    pub query: String,
    pub highlight_all: bool,
    pub match_case: bool,
    pub whole_words: bool,
    pub matches: Vec<(usize, usize)>,
    pub active: usize,
    pub flat_text: String,
    pub highlighted_segments: Option<Vec<ContentSegment>>,
    pub dirty: bool,
    pub last_query: String,
    pub need_scroll: bool,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            show: false,
            query: String::new(),
            highlight_all: true,
            match_case: false,
            whole_words: false,
            matches: Vec::new(),
            active: 0,
            flat_text: String::new(),
            highlighted_segments: None,
            dirty: false,
            last_query: String::new(),
            need_scroll: false,
        }
    }
}

pub const ACTIVE_MATCH_COLOR: [u8; 4] = [255, 140, 20, 255];

impl SearchState {
    pub fn clear(&mut self) {
        self.query.clear();
        self.last_query.clear();
        self.matches.clear();
        self.highlighted_segments = None;
        self.dirty = false;
        self.need_scroll = false;
    }

    pub fn close(&mut self) {
        self.show = false;
        self.clear();
    }

    /// Move to the next match. Returns true if the active match changed.
    pub fn next_match(&mut self, chapter: &ChapterSearchData<'_>) -> bool {
        if self.dirty {
            recompute(self, chapter);
        }
        if self.matches.is_empty() {
            return false;
        }
        if self.active + 1 < self.matches.len() {
            self.active += 1;
        } else {
            self.active = 0;
        }
        let active_only = !self.highlight_all;
        self.highlighted_segments = Some(build_highlighted_segments(
            chapter,
            &self.matches,
            self.active,
            active_only,
        ));
        self.need_scroll = true;
        true
    }

    /// Move to the previous match. Returns true if the active match changed.
    pub fn prev_match(&mut self, chapter: &ChapterSearchData<'_>) -> bool {
        if self.dirty {
            recompute(self, chapter);
        }
        if self.matches.is_empty() {
            return false;
        }
        self.active = if self.active > 0 {
            self.active - 1
        } else {
            self.matches.len() - 1
        };
        let active_only = !self.highlight_all;
        self.highlighted_segments = Some(build_highlighted_segments(
            chapter,
            &self.matches,
            self.active,
            active_only,
        ));
        self.need_scroll = true;
        true
    }

    /// Rebuild highlighted segments after an option change (highlight_all toggle, etc.)
    pub fn refresh_highlights(&mut self, chapter: &ChapterSearchData<'_>) {
        let active_only = !self.highlight_all;
        self.highlighted_segments = if self.matches.is_empty() {
            None
        } else {
            Some(build_highlighted_segments(
                chapter,
                &self.matches,
                self.active,
                active_only,
            ))
        };
    }
}

/// Builds a flat text string from a chapter's text segments.
pub fn build_flat_text(segments: &[ContentSegment]) -> String {
    let mut text = String::new();
    for seg in segments {
        if let ContentSegment::StyledText(spans) = seg {
            for span in spans {
                text.push_str(&span.text);
            }
        }
    }
    text
}

pub fn word_count(segments: &[ContentSegment]) -> usize {
    let text = build_flat_text(segments);
    text.split_whitespace().count()
}

pub fn format_reading_time(minutes: u64) -> String {
    match minutes {
        0 => "less than 1m".into(),
        m if m < 60 => format!("{}m", m),
        m => {
            let h = m / 60;
            let rem = m % 60;
            if rem == 0 {
                format!("{}h", h)
            } else {
                format!("{}h {}m", h, rem)
            }
        }
    }
}

pub const WORDS_PER_MINUTE: f64 = 200.0;

/// Finds all match ranges in `text` for `query`, respecting options.
pub fn find_matches(
    text: &str,
    query: &str,
    match_case: bool,
    whole_words: bool,
) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let text_cmp = if match_case {
        text.to_string()
    } else {
        text.to_lowercase()
    };
    let query_cmp = if match_case {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let chars: Vec<char> = text.chars().collect();
    let q_chars: Vec<char> = query_cmp.chars().collect();
    let q_len = q_chars.len();
    let mut matches = Vec::new();
    let mut pos = 0usize;
    while let Some(found) = text_cmp[pos..].find(&query_cmp) {
        let start = pos + found;
        let end = start + q_len;
        if whole_words {
            let prev_char = if start > 0 {
                chars.get(start - 1)
            } else {
                None
            };
            let next_char = chars.get(end);
            let is_word_start =
                start == 0 || prev_char.map_or(true, |c| !c.is_alphanumeric());
            let is_word_end = end >= chars.len()
                || next_char.map_or(true, |c| !c.is_alphanumeric());
            if !is_word_start || !is_word_end {
                pos = start + 1;
                continue;
            }
        }
        let char_start = text[..start].chars().count();
        let char_end = text[..end].chars().count();
        matches.push((char_start, char_end));
        pos = start + 1;
    }
    matches
}

/// Recomputes search matches for the current chapter and query.
pub fn recompute(state: &mut SearchState, chapter: &ChapterSearchData<'_>) {
    state.matches.clear();
    state.active = 0;
    state.highlighted_segments = None;
    state.flat_text = build_flat_text(chapter.segments);
    state.matches = find_matches(
        &state.flat_text,
        &state.query,
        state.match_case,
        state.whole_words,
    );
    if !state.matches.is_empty() {
        let active_only = !state.highlight_all;
        state.highlighted_segments = Some(build_highlighted_segments(
            chapter,
            &state.matches,
            state.active,
            active_only,
        ));
    }
    state.dirty = false;
}

/// Builds highlighted segments from a chapter and match positions.
fn build_highlighted_segments(
    chapter: &ChapterSearchData<'_>,
    matches: &[(usize, usize)],
    active: usize,
    active_only: bool,
) -> Vec<ContentSegment> {
    let highlight_color: [u8; 4] = [255, 200, 60, 255];
    let active_color: [u8; 4] = [255, 140, 20, 255];
    let mut result: Vec<ContentSegment> = Vec::new();
    let mut char_pos = 0usize;

    for seg in chapter.segments {
        match seg {
            ContentSegment::Image { href } => {
                result.push(ContentSegment::Image { href: href.clone() });
            }
            ContentSegment::StyledText(spans) => {
                let mut new_spans: Vec<StyledSpan> = Vec::new();
                for span in spans {
                    let span_len = span.text.chars().count();
                    let span_end = char_pos + span_len;
                    let relevant: Vec<(usize, usize, bool)> = if active_only {
                        matches
                            .get(active)
                            .map(|&(ms, me)| vec![(ms, me, true)])
                            .unwrap_or_default()
                    } else {
                        matches
                            .iter()
                            .enumerate()
                            .map(|(i, &(ms, me))| (ms, me, i == active))
                            .collect()
                    };
                    let mut cuts: Vec<usize> = vec![char_pos, span_end];
                    let mut cut_is_active: HashMap<usize, bool> = HashMap::new();
                    for (ms, me, is_active) in &relevant {
                        let os = (*ms).max(char_pos);
                        let oe = (*me).min(span_end);
                        if os < oe {
                            cuts.push(os);
                            cuts.push(oe);
                            cut_is_active.insert(os, *is_active);
                        }
                    }
                    cuts.sort();
                    cuts.dedup();
                    for w in cuts.windows(2) {
                        let s = w[0];
                        let e = w[1];
                        if s >= e {
                            continue;
                        }
                        let seg_text: String =
                            span.text.chars().skip(s - char_pos).take(e - s).collect();
                        let is_highlight =
                            cut_is_active.get(&s).copied().unwrap_or(false);
                        let span_color = if cut_is_active.contains_key(&s) {
                            if is_highlight {
                                Some(active_color)
                            } else {
                                Some(highlight_color)
                            }
                        } else {
                            None
                        };
                        new_spans.push(StyledSpan {
                            text: seg_text,
                            bold: span.bold,
                            italic: span.italic,
                            underline: span.underline,
                            heading_level: span.heading_level,
                            link_url: span.link_url.clone(),
                            color: span_color,
                        });
                    }
                    char_pos = span_end;
                }
                if !new_spans.is_empty() {
                    result.push(ContentSegment::StyledText(new_spans));
                }
            }
        }
    }
    result
}
