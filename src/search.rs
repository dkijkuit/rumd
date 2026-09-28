//! Search support: match finding, document section splitting, and locating
//! matches inside laid-out text.

use std::ops::Range;

use eframe::egui;

/// Rects of one match (byte range into `text`) inside a laid-out galley,
/// one per touched text row, translated by `galley_pos`.
///
/// A single `pos_from_cursor` pair mis-locates a match that starts at a row
/// wrap: the wrap-boundary character maps to the END of the previous row
/// (`layout_from_cursor` uses an inclusive row end), which yields an
/// inverted rect that renders nothing. Clamping the match to each row's
/// char range and addressing rows directly avoids that.
pub fn match_range_rects(
    galley: &egui::epaint::Galley,
    galley_pos: egui::Pos2,
    text: &str,
    byte_range: Range<usize>,
) -> Vec<egui::Rect> {
    let start = char_index_of_byte(text, byte_range.start);
    let end = char_index_of_byte(text, byte_range.end);
    let mut rects = Vec::new();
    let mut running = 0usize;
    for (row_nr, row) in galley.rows.iter().enumerate() {
        let count = row.char_count_excluding_newline().0;
        let row_lo = running;
        let row_hi = row_lo + count;
        running += row.char_count_including_newline().0;
        if start >= row_hi || end <= row_lo {
            continue;
        }
        let ca = (start - row_lo).min(count);
        let cb = (end - row_lo).min(count);
        if ca >= cb {
            continue;
        }
        let cursor = |column: usize| egui::epaint::text::cursor::LayoutCursor {
            row: row_nr,
            column: egui::text::CharIndex(column),
        };
        let from = galley.pos_from_layout_cursor(&cursor(ca));
        let to = galley.pos_from_layout_cursor(&cursor(cb));
        let rect = egui::Rect::from_min_max(from.min, to.max).translate(galley_pos.to_vec2());
        if rect.is_positive() {
            rects.push(rect);
        }
    }
    rects
}

/// Rects of every match of `query` in a laid-out galley.
pub fn match_rects_in_galley(
    galley: &egui::epaint::Galley,
    galley_pos: egui::Pos2,
    text: &str,
    query: &str,
) -> Vec<egui::Rect> {
    let query = query.trim();
    let mut rects = Vec::new();
    if query.is_empty() {
        return rects;
    }
    for m in find_matches(text, query) {
        rects.extend(match_range_rects(galley, galley_pos, text, m));
    }
    rects
}

/// Find all case-insensitive occurrences of `query` in `text`.
/// Returns byte-offset ranges into the original `text`.
/// An empty query yields no matches.
pub fn find_matches(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let fold = |c: char| -> Vec<char> { c.to_lowercase().collect() };
    let query_folded: Vec<Vec<char>> = query.chars().map(fold).collect();
    let text_folded: Vec<(usize, usize, Vec<char>)> = text
        .char_indices()
        .map(|(byte, c)| (byte, c.len_utf8(), fold(c)))
        .collect();
    let mut matches = Vec::new();
    if text_folded.len() < query_folded.len() {
        return matches;
    }
    'window: for start in 0..=(text_folded.len() - query_folded.len()) {
        for (step, q_chars) in query_folded.iter().enumerate() {
            if text_folded[start + step].2 != *q_chars {
                continue 'window;
            }
        }
        let last = start + query_folded.len() - 1;
        let end = text_folded[last].0 + text_folded[last].1;
        matches.push(text_folded[start].0..end);
    }
    matches
}

/// Convert a byte offset into a char index (egui cursors are char-based).
/// Offsets that are not char boundaries are floored to the nearest one,
/// so stale offsets from earlier document revisions cannot panic.
pub fn char_index_of_byte(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    text[..byte].chars().count()
}

/// Split `text` into sections for rendered-view jumps.
///
/// Boundaries are never placed inside fenced code blocks (``` or ~~~).
/// If any ATX heading (`#`…) line exists outside a fence, sections start
/// at heading lines; otherwise at blank lines that precede content.
/// A document with no boundaries is a single section.
pub fn split_sections(text: &str) -> Vec<Range<usize>> {
    let lines: Vec<(usize, &str)> = {
        let mut v = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            v.push((offset, line));
            offset += line.len();
        }
        v
    };

    let mut fence: Option<char> = None;
    let mut headings = Vec::new();
    let mut blanks = Vec::new();
    for (i, &(offset, line)) in lines.iter().enumerate() {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            fence = Some('`');
            continue;
        }
        if trimmed.starts_with("~~~") {
            fence = Some('~');
            continue;
        }
        if trimmed.starts_with('#') {
            headings.push(offset);
        }
        let next_is_content = lines.get(i + 1).is_some_and(|&(_, l)| !l.trim().is_empty());
        if content.trim().is_empty() && next_is_content {
            blanks.push(offset);
        }
    }

    let boundaries: &[usize] = if headings.is_empty() {
        &blanks
    } else {
        &headings
    };
    let mut sections = Vec::new();
    let mut start = 0;
    for &b in boundaries {
        if b > start {
            sections.push(start..b);
            start = b;
        }
    }
    if start < text.len() {
        sections.push(start..text.len());
    }
    sections
}

/// Index of the section containing byte offset `at`.
pub fn section_containing(sections: &[Range<usize>], at: usize) -> Option<usize> {
    sections.iter().position(|r| r.start <= at && at < r.end)
}

/// Section index for a cursor position: like [`section_containing`], but a
/// cursor sitting at the very end of the text (past the last section's
/// exclusive end) belongs to the last section.
pub fn section_for_cursor(sections: &[Range<usize>], at: usize) -> Option<usize> {
    if let Some(i) = section_containing(sections, at) {
        return Some(i);
    }
    let last = sections.len().checked_sub(1)?;
    sections
        .last()
        .is_some_and(|r| r.start <= at)
        .then_some(last)
}

/// Live find-bar state.
#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    /// Byte-offset ranges into the current document.
    pub matches: Vec<Range<usize>>,
    /// Index of the current match into `matches`.
    pub current: usize,
}

impl SearchState {
    pub fn current_match(&self) -> Option<&Range<usize>> {
        self.matches.get(self.current)
    }

    /// Advance `delta` matches, wrapping at both ends.
    pub fn step(&mut self, delta: isize) {
        if self.matches.is_empty() {
            self.current = 0;
            return;
        }
        let n = self.matches.len() as isize;
        self.current = (self.current as isize + delta).rem_euclid(n) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_case_insensitive_matches() {
        assert_eq!(find_matches("AbC abc ABC", "abc"), vec![0..3, 4..7, 8..11]);
    }

    #[test]
    fn empty_query_yields_no_matches() {
        assert!(find_matches("hello", "").is_empty());
        assert!(find_matches("", "x").is_empty());
    }

    #[test]
    fn offsets_are_byte_offsets_into_original() {
        // 'é' is two bytes; the match range must still slice the original text.
        let text = "éx abc";
        let m = find_matches(text, "abc");
        assert_eq!(m, vec![4..7]);
        assert_eq!(&text[m[0].clone()], "abc");
    }

    #[test]
    fn overlapping_occurrences_count_separately() {
        assert_eq!(find_matches("aaa", "aa"), vec![0..2, 1..3]);
    }

    #[test]
    fn char_index_of_byte_handles_multibyte() {
        let text = "éx abc";
        assert_eq!(char_index_of_byte(text, 3), 2);
        assert_eq!(char_index_of_byte(text, 100), text.chars().count());
    }

    #[test]
    fn char_index_of_byte_floors_to_char_boundary() {
        // Byte 1 is inside 'é'; a stale offset must not panic.
        assert_eq!(char_index_of_byte("éx", 1), 0);
        assert_eq!(char_index_of_byte("éx", 2), 1);
    }

    #[test]
    fn splits_at_headings_outside_fences() {
        let md = "intro\n\n# One\ntext\n\n## Two\nmore";
        let s = split_sections(md);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], 0..7);
        assert_eq!(&md[s[1].clone()], "# One\ntext\n\n");
        assert_eq!(&md[s[2].clone()], "## Two\nmore");
    }

    #[test]
    fn heading_inside_fence_is_not_a_boundary() {
        let md = "a\n\n```rust\n# not a heading\n```\n\nb";
        let s = split_sections(md);
        // No headings outside fences → blank-line fallback applies. The
        // blank before the fence is outside it, so: "a" | fence block | "b".
        assert_eq!(s.len(), 3);
        assert_eq!(&md[s[1].clone()], "\n```rust\n# not a heading\n```\n");
    }

    #[test]
    fn tilde_fences_are_respected() {
        let md = "a\n\n~~~\n## not a heading\n~~~\n\nb";
        let s = split_sections(md);
        assert_eq!(s.len(), 3);
        assert_eq!(&md[s[1].clone()], "\n~~~\n## not a heading\n~~~\n");
    }

    #[test]
    fn unclosed_fence_never_splits() {
        let md = "a\n\n```\n# heading inside\nmore\n\nstill fenced";
        let s = split_sections(md);
        // Only the blank *before* the unclosed fence is a boundary; the
        // interior blanks never split.
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], 0..2);
        assert_eq!(
            &md[s[1].clone()],
            "\n```\n# heading inside\nmore\n\nstill fenced"
        );
    }

    #[test]
    fn blank_line_fallback_when_no_headings() {
        let md = "para one\n\npara two\n\npara three";
        let s = split_sections(md);
        assert_eq!(s.len(), 3);
        assert_eq!(s[2], 19..md.len());
    }

    #[test]
    fn empty_text_has_no_sections() {
        assert!(split_sections("").is_empty());
    }

    #[test]
    fn section_containing_finds_and_rejects() {
        let sections = vec![0..5, 5..10];
        assert_eq!(section_containing(&sections, 3), Some(0));
        assert_eq!(section_containing(&sections, 7), Some(1));
        assert_eq!(section_containing(&sections, 10), None);
    }

    #[test]
    fn section_for_cursor_handles_inside_and_boundaries() {
        let sections = vec![0..5, 5..10];
        assert_eq!(section_for_cursor(&sections, 3), Some(0));
        // Boundary bytes belong to the section that starts there.
        assert_eq!(section_for_cursor(&sections, 5), Some(1));
        // A cursor parked at the end of the text belongs to the last
        // section, where section_containing finds nothing.
        assert_eq!(section_for_cursor(&sections, 10), Some(1));
    }

    #[test]
    fn section_for_cursor_empty_sections_is_none() {
        assert_eq!(section_for_cursor(&[], 0), None);
    }

    #[test]
    fn search_state_steps_with_wraparound() {
        let mut s = SearchState {
            open: true,
            query: "a".into(),
            matches: vec![0..1, 2..3, 4..5],
            current: 0,
        };
        s.step(1);
        assert_eq!(s.current, 1);
        s.step(-1);
        assert_eq!(s.current, 0);
        s.step(-1);
        assert_eq!(s.current, 2);
        s.step(1);
        assert_eq!(s.current, 0);
    }

    #[test]
    fn search_state_step_on_empty_is_noop() {
        let mut s = SearchState::default();
        s.step(1);
        s.step(-1);
        assert_eq!(s.current, 0);
        assert!(s.current_match().is_none());
    }
}
