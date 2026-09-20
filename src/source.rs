use std::ops::Range;

use eframe::egui;
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};

use crate::search;

/// A TextBuffer wrapper that exposes text but rejects every mutation,
/// turning egui_code_editor into a read-only viewer.
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

pub fn show(ui: &mut egui::Ui, raw: &str, theme: ColorTheme, highlight: Option<Range<usize>>) {
    let mut buffer = ReadOnlyBuffer(raw);
    let (output, _tokens) = CodeEditor::default()
        .id_source("rumd_source")
        .with_rows(24)
        .with_fontsize(14.0)
        .with_theme(theme)
        .with_numlines(true)
        .with_clickable_links(true)
        .vscroll(true)
        .show(ui, &mut buffer, &markdown_syntax());

    // Highlight the searched match ourselves: egui collapses any selection
    // stored in the TextEdit state back to a bare cursor on load, so a
    // stored selection can never survive a frame, let alone paint. A
    // translucent overlay rect over the match glyphs is the reliable way
    // to show it. Focus keeps egui_code_editor's cursor-follow scroll
    // bringing the match into view.
    if let Some(byte_range) = highlight {
        let start = search::char_index_of_byte(raw, byte_range.start);
        let end = search::char_index_of_byte(raw, byte_range.end);
        let end_cc = egui::text::CCursor::new(egui::text::CharIndex(end));

        // Park the cursor at the match end so the editor scrolls to it.
        let mut state = output.state;
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(end_cc)));
        state.store(ui.ctx(), output.response.response.id);
        output.response.response.request_focus();

        let start_pos = output.galley.pos_from_cursor(egui::text::CCursor::new(
            egui::text::CharIndex(start),
        ));
        let end_pos = output.galley.pos_from_cursor(end_cc);
        let rect = egui::Rect::from_min_max(start_pos.min, end_pos.max)
            .translate(output.galley_pos.to_vec2());
        ui.painter()
            .rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::TextBuffer as _;

    #[test]
    fn read_only_buffer_exposes_text() {
        let text = String::from("# Hello");
        let buf = ReadOnlyBuffer(text.as_str());
        assert_eq!(buf.as_str(), "# Hello");
        assert!(!buf.is_mutable());
    }

    #[test]
    fn read_only_buffer_ignores_edits() {
        let text = String::from("# Hello");
        let mut buf = ReadOnlyBuffer(&text);
        let n = buf.insert_text("junk", egui::text::CharIndex(0));
        assert_eq!(n, 0);
        assert_eq!(buf.as_str(), "# Hello");
        buf.delete_char_range(egui::text::CharIndex(0)..egui::text::CharIndex(1));
        assert_eq!(buf.as_str(), "# Hello");
    }

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
        let raw = "# T\n\nbody";
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| show(ui, raw, theme, None));
        harness.run();
    }

    #[test]
    fn source_selection_handles_multibyte_offsets() {
        use egui_kittest::Harness;
        let raw = "é\nneedle here\n";
        let jump = crate::search::find_matches(raw, "needle")[0].clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| show(ui, raw, theme, Some(jump.clone())));
        harness.run();
        harness.run();
    }

    #[test]
    fn source_jump_focuses_the_editor_for_visible_feedback() {
        use egui_kittest::Harness;
        let raw = "line one\nneedle here\n";
        let jump = crate::search::find_matches(raw, "needle")[0].clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| show(ui, raw, theme, Some(jump.clone())));
        harness.run();
        harness.run();
        assert!(
            harness.ctx.memory(|m| m.focused().is_some()),
            "jump must focus the source editor so the selection paints"
        );
    }

    #[test]
    fn source_jump_paints_a_persistent_highlight() {
        use egui_kittest::Harness;
        let raw = "line one\nneedle here\nline two\nneedle again\n".to_string();
        let jump = crate::search::find_matches(&raw, "needle")[0].clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let raw2 = raw.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| show(ui, &raw2, theme, Some(jump.clone())));
        let expected_fill = egui::Style::default().visuals.selection.bg_fill;
        for frame in 0..3 {
            harness.run();
            let highlight_rects = harness
                .output()
                .shapes
                .iter()
                .filter(|clipped| {
                    matches!(&clipped.shape, egui::Shape::Rect(r) if r.fill == expected_fill)
                })
                .count();
            assert!(
                highlight_rects >= 1,
                "frame {frame}: the match highlight must be painted"
            );
        }
    }
}
