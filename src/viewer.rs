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

    pub fn show(&mut self, ui: &mut egui::Ui, markdown: &str) {
        // CentralPanel clips overflow, so long documents need a ScrollArea.
        // (egui_commonmark's show_scrollable assumes static markdown and
        // requires manual cache clearing on every change.)
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                CommonMarkViewer::new().show(ui, &mut self.cache, markdown)
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;
    use egui_kittest::{Harness, kittest::Queryable};

    const GOLDEN: &str = "# Sample Heading\n\nSome paragraph text.\n\n- done item\n- [ ] todo item\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nThis is ~~struck text~~.\n\n```rust\nlet x = 1;\n```\n";

    #[test]
    fn golden_doc_renders_gfm_features() {
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| view.show(ui, GOLDEN));
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
                .build_ui(move |ui| view.show(ui, &owned));
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
            .build_ui(move |ui| view.show(ui, &md));
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
}
