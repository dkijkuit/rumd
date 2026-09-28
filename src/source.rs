use std::ops::Range;

use eframe::egui;
use egui::widgets::text_edit::TextEditOutput;
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};

use crate::search;

/// A TextBuffer wrapper that exposes text but rejects every mutation,
/// turning egui_code_editor into a read-only viewer (lossy documents).
struct ReadOnlyBuffer<'a>(&'a str);

impl egui::TextBuffer for ReadOnlyBuffer<'_> {
    fn is_mutable(&self) -> bool {
        false
    }

    fn as_str(&self) -> &str {
        self.0
    }

    fn insert_text(&mut self, _text: &str, _char_index: egui::text::CharIndex) -> usize {
        0
    }

    fn delete_char_range(&mut self, _char_range: Range<egui::text::CharIndex>) {}

    fn type_id(&self) -> core::any::TypeId {
        // TypeIds are lifetime-erased; 'static satisfies the required bound.
        core::any::TypeId::of::<ReadOnlyBuffer<'static>>()
    }
}

/// Keyword/color rules for highlighting raw markdown text.
pub fn markdown_syntax() -> Syntax {
    Syntax::new("Markdown")
        .with_case_sensitive(true)
        .with_comment_multiline(["<!--", "-->"])
        .with_hyperlinks(["http", "https", "mailto"])
        .with_special([
            "#", "*", "_", "-", "+", ">", "`", "~", "[", "]", "(", ")", "|", ":", "!",
        ])
}

/// Pick the source-view palette matching the app's theme preference.
pub fn code_theme(pref: egui::ThemePreference, ctx: &egui::Context) -> ColorTheme {
    let dark = match pref {
        egui::ThemePreference::System => ctx.theme() == egui::Theme::Dark,
        egui::ThemePreference::Dark => true,
        egui::ThemePreference::Light => false,
    };
    if dark {
        ColorTheme::GITHUB_DARK
    } else {
        ColorTheme::GITHUB_LIGHT
    }
}

/// The configured source editor (identical for the editable and the
/// read-only path).
fn editor(theme: ColorTheme) -> CodeEditor {
    CodeEditor::default()
        .id_source("rumd_source")
        .with_rows(24)
        .with_fontsize(14.0)
        .with_theme(theme)
        .with_numlines(true)
        .with_clickable_links(true)
        .vscroll(true)
}

/// Show the source pane. Returns whether the text changed this frame
/// (the user edited it) and the byte offset of the cursor, so the caller
/// can steer the rendered preview to the edited spot. Lossy (non-UTF-8)
/// documents are read-only: editing them would silently rewrite invalid
/// bytes as replacement characters on save.
pub fn show(
    ui: &mut egui::Ui,
    raw: &mut String,
    lossy: bool,
    theme: ColorTheme,
    query: Option<&str>,
    highlight: Option<Range<usize>>,
) -> (bool, Option<usize>) {
    let (output, _tokens) = if lossy {
        let mut buffer = ReadOnlyBuffer(raw.as_str());
        editor(theme).show(ui, &mut buffer, &markdown_syntax())
    } else {
        editor(theme).show(ui, raw, &markdown_syntax())
    };
    let changed = !lossy && output.response.response.changed();

    // `raw` is the live document text from here on; the editor above may
    // have edited it this frame.
    let raw: &str = raw;

    // Read before the highlight block below: it moves `output.state`.
    let cursor = cursor_byte(raw, &output);

    // Highlight the searched matches ourselves: egui collapses any
    // selection stored in the TextEdit state back to a bare cursor on load,
    // so a stored selection can never survive a frame, let alone paint.
    // The style mirrors the rendered view: a subtle wash on every
    // non-current occurrence and the current match in opaque yellow with
    // its glyphs re-painted in black (the yellow covers the originals, the
    // copy redraws them). Focus keeps egui_code_editor's cursor-follow
    // scroll bringing the match into view.
    if let Some(byte_range) = &highlight {
        let end_cc = egui::text::CCursor::new(egui::text::CharIndex(search::char_index_of_byte(
            raw,
            byte_range.end,
        )));

        // Park the cursor at the match end so the editor scrolls to it.
        let mut state = output.state;
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(end_cc)));
        state.store(ui.ctx(), output.response.response.id);
        output.response.response.request_focus();
    }

    let query = query.map(str::trim).filter(|q| !q.is_empty());
    if let Some(query) = query {
        let mut subtle = Vec::new();
        let mut current: Vec<egui::Rect> = Vec::new();
        for m in search::find_matches(raw, query) {
            let rects =
                search::match_range_rects(&output.galley, output.galley_pos, raw, m.clone());
            if highlight.as_ref().is_some_and(|h| h.start == m.start) {
                current = rects;
            } else {
                subtle.extend(rects);
            }
        }
        for rect in subtle {
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
            // The wash paints over the glyphs and washes them out; redraw
            // the glyphs inside the wash (original colors — every token
            // carries its own color) so the text stays readable, matching
            // the rendered view where washes paint behind the text.
            painter.add(egui::epaint::TextShape::new(
                output.galley_pos,
                output.galley.clone(),
                egui::Color32::TRANSPARENT,
            ));
        }
        for rect in current {
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2.0, egui::Color32::YELLOW);
            painter.add(
                egui::epaint::TextShape::new(
                    output.galley_pos,
                    output.galley.clone(),
                    egui::Color32::BLACK,
                )
                .with_override_text_color(egui::Color32::BLACK),
            );
        }
    }

    (changed, cursor)
}

/// Byte offset of the editor's primary cursor, if it has one.
fn cursor_byte(raw: &str, output: &TextEditOutput) -> Option<usize> {
    let egui::text::CharIndex(chars) = output.state.cursor.char_range()?.primary.index;
    raw.char_indices()
        .enumerate()
        .find(|(char_i, _)| *char_i == chars)
        .map(|(_, (byte, _))| byte)
        .or(Some(raw.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    #[test]
    fn markdown_syntax_has_expected_language() {
        let syntax = markdown_syntax();
        assert_eq!(syntax.language, "Markdown");
        assert!(!syntax.special.is_empty());
    }

    #[test]
    fn code_theme_follows_preference() {
        // Light preference must never pick the dark palette.
        let light = code_theme(egui::ThemePreference::Light, &egui::Context::default());
        assert_eq!(light, ColorTheme::GITHUB_LIGHT);
        let dark = code_theme(egui::ThemePreference::Dark, &egui::Context::default());
        assert_eq!(dark, ColorTheme::GITHUB_DARK);
    }

    #[test]
    fn source_view_runs_headless() {
        use egui_kittest::Harness;
        let raw = "# T\n\nbody".to_string();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                show(ui, &mut raw.clone(), false, theme, None, None);
            });
        harness.run();
    }

    #[test]
    fn lossy_documents_stay_read_only() {
        use egui_kittest::kittest::Queryable;
        use std::cell::RefCell;
        use std::rc::Rc;
        let text = Rc::new(RefCell::new(String::from("# R\n\nbody")));
        let text_for_ui = text.clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                show(ui, &mut text_for_ui.borrow_mut(), true, theme, None, None);
            });
        harness.run();
        harness.get_by_value("# R\n\nbody").focus();
        harness.run();
        harness.get_by_value("# R\n\nbody").type_text("X");
        harness.run();
        assert_eq!(
            text.borrow().as_str(),
            "# R\n\nbody",
            "a lossy document must refuse edits"
        );
    }

    #[test]
    fn source_view_edits_reach_the_text() {
        use egui_kittest::kittest::Queryable;
        use std::cell::RefCell;
        use std::rc::Rc;
        let text = Rc::new(RefCell::new(String::from("# T\n\nbody")));
        let text_for_ui = text.clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                show(ui, &mut text_for_ui.borrow_mut(), false, theme, None, None);
            });
        harness.run();
        harness.get_by_value("# T\n\nbody").focus();
        harness.run();
        harness.get_by_value("# T\n\nbody").type_text("X");
        harness.run();
        assert!(
            text.borrow().contains('X'),
            "typing in the source view must edit the text, got {:?}",
            text.borrow()
        );
    }

    #[test]
    fn source_selection_handles_multibyte_offsets() {
        use egui_kittest::Harness;
        let raw = "é\nneedle here\n".to_string();
        let jump = crate::search::find_matches(&raw, "needle")[0].clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                show(ui, &mut raw.clone(), false, theme, None, Some(jump.clone()));
            });
        harness.run();
        harness.run();
    }

    #[test]
    fn source_jump_focuses_the_editor_for_visible_feedback() {
        use egui_kittest::Harness;
        let raw = "line one\nneedle here\n".to_string();
        let jump = crate::search::find_matches(&raw, "needle")[0].clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                show(ui, &mut raw.clone(), false, theme, None, Some(jump.clone()));
            });
        harness.run();
        harness.run();
        assert!(
            harness.ctx.memory(|m| m.focused().is_some()),
            "jump must focus the source editor so the selection paints"
        );
    }

    fn yellow_rects(harness: &Harness<'_, ()>) -> Vec<egui::Rect> {
        fn walk(shape: &egui::Shape, out: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Vec(children) => {
                    for child in children {
                        walk(child, out);
                    }
                }
                egui::Shape::Rect(r) if r.fill == egui::Color32::YELLOW => {
                    out.push(r.rect);
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in harness.output().shapes.iter() {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    #[test]
    fn source_jump_paints_a_persistent_yellow_highlight() {
        use egui_kittest::Harness;
        let raw = "line one\nneedle here\nline two\nneedle again\n".to_string();
        let jump = crate::search::find_matches(&raw, "needle")[0].clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut raw2 = raw.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                show(
                    ui,
                    &mut raw2,
                    false,
                    theme,
                    Some("needle"),
                    Some(jump.clone()),
                );
            });
        for frame in 0..3 {
            harness.run();
            let yellow = yellow_rects(&harness);
            assert_eq!(
                yellow.len(),
                1,
                "frame {frame}: exactly the current match must be painted yellow"
            );
        }
    }

    #[test]
    fn source_search_shows_other_occurrences_subtly() {
        use egui_kittest::Harness;
        let raw = "line one\nneedle here\nline two\nneedle again\n".to_string();
        let jump = crate::search::find_matches(&raw, "needle")[0].clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut raw2 = raw.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                show(
                    ui,
                    &mut raw2,
                    false,
                    theme,
                    Some("needle"),
                    Some(jump.clone()),
                );
            });
        harness.run();
        harness.run();
        let expected_fill = egui::Style::default().visuals.selection.bg_fill;
        let subtle = harness
            .output()
            .shapes
            .iter()
            .filter(
                |clipped| matches!(&clipped.shape, egui::Shape::Rect(r) if r.fill == expected_fill),
            )
            .count();
        assert_eq!(
            subtle, 1,
            "the non-current occurrence must be painted with the subtle wash"
        );
        assert_eq!(yellow_rects(&harness).len(), 1);
    }

    #[test]
    fn source_search_wash_redraws_the_glyphs_it_covers() {
        // The subtle wash paints over the syntax-colored text and washes it
        // out; the glyphs inside the wash must be redrawn on top so the
        // text keeps its original colors and contrast.
        use egui_kittest::Harness;
        let raw = "line one\nneedle here\nline two\nneedle again\n".to_string();
        let jump = crate::search::find_matches(&raw, "needle")[0].clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut raw2 = raw.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                show(
                    ui,
                    &mut raw2,
                    false,
                    theme,
                    Some("needle"),
                    Some(jump.clone()),
                );
            });
        harness.run();
        harness.run();
        let needle_texts = harness
            .output()
            .shapes
            .iter()
            .filter(|clipped| {
                matches!(&clipped.shape, egui::Shape::Text(t)
                    if t.galley.text().contains("needle"))
            })
            .count();
        assert!(
            needle_texts >= 3,
            "expected the original galley, the black current-match copy and a \
             redraw over the subtle wash, got {needle_texts} text shapes"
        );
    }

    #[test]
    fn source_search_current_match_gets_black_text_over_yellow() {
        use egui_kittest::Harness;
        let raw = "line one\nneedle here\n".to_string();
        let jump = crate::search::find_matches(&raw, "needle")[0].clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut raw2 = raw.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                show(
                    ui,
                    &mut raw2,
                    false,
                    theme,
                    Some("needle"),
                    Some(jump.clone()),
                );
            });
        harness.run();
        harness.run();
        let black_copies = harness
            .output()
            .shapes
            .iter()
            .filter(|clipped| {
                matches!(&clipped.shape, egui::Shape::Text(t)
                    if t.override_text_color == Some(egui::Color32::BLACK))
            })
            .count();
        assert!(
            black_copies >= 1,
            "the current match must re-paint its glyphs in black over the yellow"
        );
    }
}
