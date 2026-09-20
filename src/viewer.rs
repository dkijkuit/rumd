use std::ops::Range;

use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

/// Target width of the centered rendered-content column (~70 characters).
pub const COLUMN_WIDTH: f32 = 660.0;
const COLUMN_MARGIN: f32 = 40.0;
const COLUMN_MIN: f32 = 320.0;
const SYNTAX_LIGHT: &str = "InspiredGitHub";
const SYNTAX_DARK: &str = "base16-ocean.dark";

/// Column width for a given available width: at most [`COLUMN_WIDTH`],
/// shrinking with the window, never exceeding the available width.
pub fn column_width(available: f32) -> f32 {
    COLUMN_WIDTH
        .min(available - 2.0 * COLUMN_MARGIN)
        .max(COLUMN_MIN.min(available))
}

/// Renders markdown into egui widgets. Keeps a parse cache between frames.
#[derive(Default)]
pub struct RenderedView {
    cache: CommonMarkCache,
}

impl RenderedView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        markdown: &str,
        sections: &[Range<usize>],
        pending_jump: &mut Option<usize>,
    ) {
        // CentralPanel clips overflow, so long documents need a ScrollArea.
        // (egui_commonmark's show_scrollable assumes static markdown and
        // requires manual cache clearing on every change.)
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let available = ui.available_width();
                let width = column_width(available);
                let full = ui.max_rect();
                let left = full.left() + (available - width) / 2.0;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(left, full.top()),
                    egui::vec2(width, full.height()),
                );
                let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                // Sections are rendered separately so a pending search jump
                // can scroll to the exact section containing the match. The
                // cache is documented to support multiple source ids.
                //
                // Each section gets its OWN child Ui with a distinct id:
                // egui_commonmark derives widget ids (tables, code-block
                // widgets, checkboxes) from `ui.id()` plus a counter that
                // resets on every `show()` call, so sections sharing one Ui
                // would collide ids (egui's red id-clash overlay, shared
                // widget state).
                for (i, range) in sections.iter().enumerate() {
                    let top = inner.cursor().top();
                    let sec_max = egui::Rect::from_min_max(
                        egui::pos2(inner.max_rect().left(), top),
                        egui::pos2(inner.max_rect().right(), f32::INFINITY),
                    );
                    let mut section_ui = inner.new_child(
                        egui::UiBuilder::new()
                            .id_salt(format!("rumd_section_{i}"))
                            .max_rect(sec_max),
                    );
                    // Wrap at the actual column width, never at a wider
                    // constant, or lines overflow and overlap neighbours.
                    let wrap_width = section_ui.available_width();
                    let viewer = CommonMarkViewer::new()
                        .default_width(Some(wrap_width as usize))
                        .syntax_theme_light(SYNTAX_LIGHT)
                        .syntax_theme_dark(SYNTAX_DARK);
                    let response =
                        viewer.show(&mut section_ui, &mut self.cache, &markdown[range.clone()]);
                    // `new_child` does not advance the parent cursor; claim
                    // the section's rect so the next section stacks below it.
                    inner.allocate_rect(section_ui.min_rect(), egui::Sense::hover());
                    if *pending_jump == Some(i) {
                        *pending_jump = None;
                        inner.scroll_to_rect(response.response.rect, None);
                    }
                }
                // Tell the ScrollArea how tall the content is: nothing else
                // expands this ui, so the measured content would stay zero.
                ui.expand_to_include_rect(inner.min_rect());
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use eframe::egui;
    use egui_kittest::{Harness, kittest::Queryable};

    fn show_whole(view: &mut RenderedView, ui: &mut egui::Ui, markdown: &str) {
        let sections = crate::search::split_sections(markdown);
        view.show(ui, markdown, &sections, &mut None);
    }

    const GOLDEN: &str = "# Sample Heading\n\nSome paragraph text.\n\n- done item\n- [ ] todo item\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nThis is ~~struck text~~.\n\n```rust\nlet x = 1;\n```\n";

    #[test]
    fn golden_doc_renders_gfm_features() {
        let mut view = RenderedView::new();
        let owned = GOLDEN.to_string();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| show_whole(&mut view, ui, &owned));
        harness.run();
        harness.run();
        assert!(harness.query_by_label("Sample Heading").is_some());
        assert!(harness.query_by_label("done item").is_some());
        assert!(harness.query_by_label("Some paragraph text.").is_some());
        assert!(harness.query_by_label("struck text").is_some());
    }

    #[test]
    fn empty_and_odd_input_do_not_panic() {
        for md in ["", "unclosed **bold", "![broken image](missing.png)"] {
            let mut view = RenderedView::new();
            let owned = md.to_string();
            let mut harness = Harness::builder()
                .with_size(egui::vec2(600.0, 400.0))
                .build_ui(move |ui| show_whole(&mut view, ui, &owned));
            harness.run();
        }
    }

    #[test]
    fn long_document_is_scrollable() {
        let mut md = String::new();
        for i in 0..80 {
            md.push_str(&format!("Paragraph {i} text.\n\n"));
        }
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| show_whole(&mut view, ui, &md));
        harness.run();
        harness.run();
        let bars = harness
            .query_all_by_role(egui::accesskit::Role::ScrollBar)
            .count();
        let views = harness
            .query_all_by_role(egui::accesskit::Role::ScrollView)
            .count();
        assert!(
            bars + views > 0,
            "long document produced no scrollable area (bars {bars}, views {views})"
        );
    }

    #[test]
    fn pending_jump_to_section_consumes_and_does_not_panic() {
        let md = "# A\n\nword\n\n# B\n\nword\n".to_string();
        let sections = crate::search::split_sections(&md);
        assert_eq!(sections.len(), 2);
        let mut view = RenderedView::new();
        let jump = Rc::new(RefCell::new(Some(1usize)));
        let jump_for_ui = jump.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut jump_for_ui.borrow_mut());
            });
        harness.run();
        harness.run();
        assert!(jump.borrow().is_none(), "pending jump must be consumed");
    }

    #[test]
    fn single_fence_document_renders_as_one_section() {
        let md = "```\n# not heading\n\nstill fenced\n".to_string();
        let sections = crate::search::split_sections(&md);
        assert_eq!(sections.len(), 1);
        let mut view = RenderedView::new();
        let jump = Rc::new(RefCell::new(Some(0usize)));
        let jump_for_ui = jump.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut jump_for_ui.borrow_mut());
            });
        harness.run();
        harness.run();
        assert!(jump.borrow().is_none());
    }

    #[test]
    fn column_width_clamps() {
        assert_eq!(super::column_width(2000.0), 660.0);
        assert_eq!(super::column_width(600.0), 520.0); // available - 2*margin
        assert_eq!(super::column_width(300.0), 300.0); // never exceeds available
        assert_eq!(super::column_width(100.0), 100.0);
    }

    #[test]
    fn no_widget_id_clashes_across_sections() {
        // Two sections that EACH contain a table and an identical code
        // block: per-section widget ids must never collide within a frame.
        // egui_commonmark derives ids from `ui.id()` + a counter that resets
        // per `show()` call, so sections must get distinct Ui ids.
        let md = "# A\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nlet x = 1;\n```\n\n# B\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nlet x = 1;\n```\n".to_string();
        let sections = crate::search::split_sections(&md);
        assert_eq!(sections.len(), 2);
        let mut view = RenderedView::new();
        let secs = sections.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &secs, &mut None);
            });
        harness.run();
        harness.run();
        // egui paints "🔥 <error>" text (debug id-clash overlay) when two
        // widgets share an id at different rects in the same frame.
        let clash = harness.output().shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(t)
                if t.galley.text().contains('🔥'))
        });
        assert!(!clash, "egui id-clash debug overlay was painted");
    }

    #[test]
    fn content_is_centered_in_window() {
        // A column-filling paragraph (wraps at the column edge) must land
        // in the horizontal center of the window.
        let md = "a very long paragraph ".repeat(60);
        let sections = crate::search::split_sections(&md);
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 600.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut None);
            });
        harness.run();
        harness.run();
        let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
        for clipped in harness.output().shapes.iter() {
            if let egui::Shape::Text(t) = &clipped.shape {
                min_x = min_x.min(t.galley.rect.left() + t.pos.x);
                max_x = max_x.max(t.galley.rect.right() + t.pos.x);
            }
        }
        assert!(
            min_x < f32::MAX && max_x > f32::MIN,
            "no text painted"
        );
        let center = (min_x + max_x) / 2.0;
        assert!(
            (center - 500.0).abs() <= 10.0,
            "content center {center} must be within 10px of the window center 500 (bounds {min_x}..{max_x})"
        );
    }

    #[test]
    fn text_wraps_at_column_width_not_constant() {
        // In a narrow window the column is narrower than COLUMN_WIDTH; the
        // viewer must wrap text at the actual column width, not the
        // constant, or lines overflow the column and overlap neighbours.
        let md = "a very long paragraph ".repeat(40);
        let sections = crate::search::split_sections(&md);
        assert_eq!(sections.len(), 1);
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut None);
            });
        harness.run();
        harness.run();
        // Column width at this window size is 552; allow a small tolerance
        // for rounding and scrollbar insets.
        let limit = super::column_width(600.0) + 16.0;
        let overflowing: Vec<_> = harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(t) if t.galley.rect.width() > limit => {
                    Some(t.galley.rect.width())
                }
                _ => None,
            })
            .collect();
        assert!(
            overflowing.is_empty(),
            "text galleys overflow the column (limit {limit}): {overflowing:?}"
        );
    }
}
