//! Search support: match finding and document section splitting.

use std::ops::Range;

/// Find all case-insensitive occurrences of `query` in `text`.
/// Returns byte-offset ranges into the original `text`.
/// An empty query yields no matches.
// Wired into the app in later tasks; a binary crate flags unused `pub`
// items, so silence until then (removed when the callers land).
#[allow(dead_code)]
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
#[allow(dead_code)]
pub fn char_index_of_byte(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
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
}
