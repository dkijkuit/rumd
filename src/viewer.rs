use std::ops::Range;

use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

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
                // Sections are rendered separately so a pending search jump
                // can scroll to the exact section containing the match. The
                // cache is documented to support multiple source ids.
                for (i, range) in sections.iter().enumerate() {
                    let response = CommonMarkViewer::new()
                        .show(ui, &mut self.cache, &markdown[range.clone()]);
                    if *pending_jump == Some(i) {
                        *pending_jump = None;
                        ui.scroll_to_rect(response.response.rect, None);
                    }
                }
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
}
