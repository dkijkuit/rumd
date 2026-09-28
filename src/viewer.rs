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
    /// Last laid-out height of each section. Offscreen sections are
    /// replaced by placeholder rects of this height so their text is never
    /// shaped: egui's galley cache only survives a single frame, so
    /// without skipping, every frame (and every Rendered<->Split toggle,
    /// which re-shapes at a new wrap width) re-layouts the whole document.
    section_heights: Vec<Option<f32>>,
    /// Jump awaiting its exact scroll: set when a jump is consumed, kept
    /// until the current match has been painted (the downward paint margin
    /// makes that happen within a frame or two) and scrolled to exactly.
    /// The first, estimate-based scroll can be off by a lot when content
    /// density varies, so this may take a few frames.
    scroll_target: Option<(String, usize, usize)>,
}

/// What to highlight in the rendered view while the find bar is open.
pub struct SearchHighlight {
    pub query: String,
    /// Index of the section containing the current match.
    pub section: usize,
    /// Byte offset of the current match in the raw markdown.
    pub match_byte: usize,
}

impl RenderedView {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop per-document layout measurements (call when the document
    /// changes, so placeholder heights never come from the old text).
    pub fn clear(&mut self) {
        self.section_heights.clear();
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        markdown: &str,
        sections: &[Range<usize>],
        pending_jump: &mut Option<usize>,
        search: Option<&SearchHighlight>,
    ) {
        // CentralPanel clips overflow, so long documents need a ScrollArea.
        // (egui_commonmark's show_scrollable assumes static markdown and
        // requires manual cache clearing on every change.)
        // Paint content well below the viewport: matches that are off
        // screen are normally culled, which would leave the jump's estimate
        // as the best available position. With the extra paint margin the
        // current match's exact rect becomes known and the scroll can be
        // corrected on the next frame. (Downward only: content above the
        // viewport would paint over the panels.)
        let paint_margin = egui::vec2(0.0, ui.available_height() * 3.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Allocate the highlight shape before any content: filling
                // it in after the scan paints the rects behind all of the
                // view's text (same layer, earlier paint index).
                // Two highlight slots, allocated before any content so both
                // paint behind the view's text (filled in after the scan):
                // one for the subtle occurrence rects, one for the yellow
                // current-match highlight, whose clip is narrowed to the
                // match rect so the black glyph copy cannot spill over.
                let (subtle_slot, current_slot) = if search.is_some() {
                    (
                        Some(ui.painter().add(egui::Shape::Noop)),
                        Some(ui.painter().add(egui::Shape::Noop)),
                    )
                } else {
                    (None, None)
                };
                let available = ui.available_width();
                let width = column_width(available);
                let full = ui.max_rect();
                let left = full.left() + (available - width) / 2.0;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(left, full.top()),
                    egui::vec2(width, full.height()),
                );
                let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                // Rendered rect of every section (document order), used to
                // attribute painted occurrence rects to sections.
                let mut section_rects: Vec<egui::Rect> = Vec::new();
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
                // New document or section count changed: no measurements.
                if self.section_heights.len() != sections.len() {
                    self.section_heights = vec![None; sections.len()];
                }
                for (i, range) in sections.iter().enumerate() {
                    // Skip sections the viewport cannot reach: placeholders
                    // of the remembered height keep the scroll extent (and
                    // thus the scrollbar) honest without shaping any text.
                    // Search re-steers across frames, so estimates are fine
                    // for sections it has not reached yet.
                    let known_height = self.section_heights[i];
                    // The clip rect stays screen-fixed while scrolling
                    // (unlike max_rect, which travels with the content).
                    let clip = inner.clip_rect();
                    let margin = clip.height() * 3.0;
                    let vis_top = clip.top() - margin;
                    let vis_bottom = clip.bottom() + margin;
                    let top = inner.next_widget_position().y;
                    let needed = known_height.is_none()
                        || *pending_jump == Some(i)
                        || search.is_some_and(|s| s.section == i)
                        || (top <= vis_bottom && known_height.is_none_or(|h| top + h >= vis_top));
                    if !needed {
                        let height = known_height.unwrap_or_default();
                        let (_, response) = inner.allocate_exact_size(
                            egui::vec2(inner.available_width(), height),
                            egui::Sense::hover(),
                        );
                        section_rects.push(response.rect);
                        continue;
                    }
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
                    // Widen the paint window downward (see paint_margin).
                    let mut section_clip = section_ui.clip_rect();
                    section_clip.max.y += paint_margin.y;
                    section_ui.set_clip_rect(section_clip);
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
                    self.section_heights[i] = Some(section_ui.min_rect().height());
                    section_rects.push(section_ui.min_rect());
                    if *pending_jump == Some(i) {
                        *pending_jump = None;
                        inner.scroll_to_rect(response.response.rect, None);
                        if let Some(search) = search {
                            self.scroll_target =
                                Some((search.query.clone(), search.section, search.match_byte));
                        }
                    }
                }
                // Tell the ScrollArea how tall the content is: nothing else
                // expands this ui, so the measured content would stay zero.
                ui.expand_to_include_rect(inner.min_rect());

                // Highlight search matches. Rendered text carries no source
                // byte offsets, so the current match is identified by
                // aligning the painted occurrences of its section (document
                // order) with the section's source matches (byte order) —
                // exact when the whole section is on screen, and anchored
                // by the source-match positions when it is not. Only what
                // is on screen is highlighted (offscreen content is
                // culled); the rects track scrolling and layout changes
                // automatically on the next frame.
                if let (Some(subtle_idx), Some(current_idx), Some(search)) =
                    (subtle_slot, current_slot, search)
                {
                    let hits = match_rects(ui.ctx(), ui.layer_id(), &search.query);
                    let rects: Vec<egui::Rect> = hits.iter().map(|(rect, _)| *rect).collect();
                    let current = current_occurrence(
                        markdown,
                        sections,
                        search,
                        section_rects.get(search.section).copied(),
                        &rects,
                    );
                    // Subtle highlight for every non-current occurrence.
                    let mut shapes = Vec::with_capacity(rects.len());
                    for (i, rect) in rects.iter().enumerate() {
                        if current != Some(i) {
                            shapes.push(egui::Shape::rect_filled(
                                *rect,
                                2.0,
                                ui.visuals().selection.bg_fill,
                            ));
                        }
                    }
                    ui.painter().set(subtle_idx, egui::Shape::Vec(shapes));
                    // Current match: opaque yellow with the matched glyphs
                    // re-painted in black, clipped to the match rect.
                    if let Some(i) = current {
                        let (rect, text_shape) = &hits[i];
                        let black_copy = egui::Shape::Text(
                            text_shape
                                .clone()
                                .with_override_text_color(egui::Color32::BLACK),
                        );
                        let highlight = egui::Shape::Vec(vec![
                            egui::Shape::rect_filled(*rect, 2.0, egui::Color32::YELLOW),
                            black_copy,
                        ]);
                        ui.painter_at(*rect).set(current_idx, highlight);
                    }
                    // A jump must bring the match itself into view, not
                    // just the top of its section. Once the match is painted
                    // we have its exact rect and are done; until then, keep
                    // the target alive across frames and steer by the
                    // estimate, centered in the viewport so estimation error
                    // in either direction has the most room.
                    let target_active =
                        self.scroll_target
                            .as_ref()
                            .is_some_and(|(query, section, match_byte)| {
                                query == &search.query
                                    && *section == search.section
                                    && *match_byte == search.match_byte
                            });
                    if target_active {
                        match current.map(|i| hits[i].0) {
                            Some(rect) => {
                                inner.scroll_to_rect(rect, None);
                                self.scroll_target = None;
                            }
                            None => {
                                if let Some(section_rect) = section_rects.get(search.section) {
                                    let est_top =
                                        match_y(markdown, sections, search, *section_rect);
                                    let row = ui.text_style_height(&egui::TextStyle::Body);
                                    inner.scroll_to_rect(
                                        egui::Rect::from_center_size(
                                            egui::pos2(section_rect.left(), est_top),
                                            egui::vec2(section_rect.width(), row),
                                        ),
                                        None,
                                    );
                                }
                            }
                        }
                    }
                }
            });
    }
}

/// Absolute byte offsets of every match of `query` within `section`, in
/// document order.
fn section_match_starts(markdown: &str, section: Range<usize>, query: &str) -> Vec<usize> {
    crate::search::find_matches(&markdown[section.clone()], query)
        .into_iter()
        .map(|m| section.start + m.start)
        .collect()
}

/// Estimated document y of the current match inside its section's rect.
fn match_y(
    markdown: &str,
    sections: &[Range<usize>],
    search: &SearchHighlight,
    section_rect: egui::Rect,
) -> f32 {
    let section = &sections[search.section];
    let starts = section_match_starts(markdown, section.clone(), &search.query);
    let match_start = starts
        .iter()
        .copied()
        .find(|start| *start >= search.match_byte)
        .unwrap_or(search.match_byte);
    let len = (section.len()).max(1) as f32;
    section_rect.top() + (match_start - section.start) as f32 / len * section_rect.height()
}

/// Index (into `rects`) of the painted occurrence that is the current
/// match, by aligning the section's painted rects with its source matches.
/// Both are in document order; the best offset absorbs partially visible
/// sections, and same-row occurrences pair up in order.
fn current_occurrence(
    markdown: &str,
    sections: &[Range<usize>],
    search: &SearchHighlight,
    section_rect: Option<egui::Rect>,
    rects: &[egui::Rect],
) -> Option<usize> {
    let section_rect = section_rect?;
    if search.section >= sections.len() {
        return None;
    }
    let section = sections[search.section].clone();
    let starts = section_match_starts(markdown, section.clone(), &search.query);
    if starts.is_empty() {
        return None;
    }
    // Painted rects belonging to this section, in document order.
    let mut in_section: Vec<usize> = rects
        .iter()
        .enumerate()
        .filter(|(_, rect)| section_rect.y_range().contains(rect.top()))
        .map(|(i, _)| i)
        .collect();
    in_section.sort_by(|&a, &b| {
        rects[a]
            .top()
            .total_cmp(&rects[b].top())
            .then(rects[a].left().total_cmp(&rects[b].left()))
    });
    if in_section.is_empty() || in_section.len() > starts.len() {
        return None;
    }
    let len = (section.len()).max(1) as f32;
    let est_y = |start: usize| {
        section_rect.top() + (start - section.start) as f32 / len * section_rect.height()
    };
    // Try every alignment of painted rects against source matches and keep
    // the one with the least total position error.
    let offsets = starts.len() - in_section.len();
    let best_offset = (0..=offsets).min_by(|&a, &b| {
        let err = |d: usize| -> f32 {
            in_section
                .iter()
                .enumerate()
                .map(|(j, &i)| (rects[i].top() - est_y(starts[j + d])).abs())
                .sum()
        };
        err(a).total_cmp(&err(b))
    })?;
    let d = best_offset;
    let ordinal = starts
        .iter()
        .take_while(|start| **start < search.match_byte)
        .count();
    let j = ordinal.checked_sub(d)?;
    if j < in_section.len() {
        Some(in_section[j])
    } else {
        None
    }
}

/// Every visible occurrence of `query` in the text painted so far this
/// frame in `layer_id`, as (rect, source text shape) pairs. Locates matches
/// with galley cursors (the source view's technique); the text shape lets
/// the current match re-paint its glyphs in black over the yellow.
fn match_rects(
    ctx: &egui::Context,
    layer_id: egui::LayerId,
    query: &str,
) -> Vec<(egui::Rect, egui::epaint::TextShape)> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    ctx.graphics(|graphics| {
        let Some(list) = graphics.get(layer_id) else {
            return;
        };
        for clipped in list.all_entries() {
            let egui::Shape::Text(text_shape) = &clipped.shape else {
                continue;
            };
            let text_shape_pos = text_shape.pos;
            for rect in crate::search::match_rects_in_galley(
                &text_shape.galley,
                text_shape_pos,
                text_shape.galley.text(),
                query,
            ) {
                hits.push((rect, text_shape.clone()));
            }
        }
    });
    hits
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
        view.show(ui, markdown, &sections, &mut None, None);
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

    fn long_doc(paragraphs: usize) -> String {
        let mut md = String::new();
        for i in 0..paragraphs {
            md.push_str(&format!("Paragraph {i} text.\n"));
            // Filler rows make each section tall enough that the far end
            // sits beyond the viewport plus its 3x paint margin.
            for _ in 0..8 {
                md.push_str("filler filler filler filler filler filler filler\n");
            }
            md.push('\n');
        }
        md
    }

    /// Sections far beyond the viewport (and its paint margin) must not be
    /// laid out: shaping the whole document is what makes frames (and mode
    /// switches, which re-shape at a new wrap width) slow on long docs.
    #[test]
    fn far_offscreen_sections_are_not_laid_out() {
        let md = long_doc(80);
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| show_whole(&mut view, ui, &md));
        harness.run();
        harness.run();
        assert!(harness.query_by_label("Paragraph 0 text.").is_some());
        assert!(
            harness.query_by_label("Paragraph 70 text.").is_none(),
            "far offscreen content must not be laid out"
        );
    }

    /// Content skipped as offscreen must appear once scrolled near: the
    /// placeholder rects keep the scrollbar honest and real layout takes
    /// over for anything the viewport reaches.
    #[test]
    fn scrolling_to_far_content_lays_it_out() {
        let md = long_doc(80);
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| show_whole(&mut view, ui, &md));
        harness.run();
        harness.run();
        // Wheel the document to the bottom (negative y reveals content
        // further down); clamped at max scroll.
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(egui::pos2(300.0, 200.0)));
        harness.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -4000.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        // Smooth scrolling animates for several frames after the event.
        harness.run_steps(30);
        harness.run();
        assert!(
            harness.query_by_label("Paragraph 70 text.").is_some(),
            "scrolled-to content must be laid out and visible"
        );
        assert!(
            harness.query_by_label("Paragraph 0 text.").is_none(),
            "content scrolled far above must be dropped again"
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
                view.show(ui, &md, &sections, &mut jump_for_ui.borrow_mut(), None);
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
                view.show(ui, &md, &sections, &mut jump_for_ui.borrow_mut(), None);
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
                view.show(ui, &md, &secs, &mut None, None);
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
                view.show(ui, &md, &sections, &mut None, None);
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
        assert!(min_x < f32::MAX && max_x > f32::MIN, "no text painted");
        let center = (min_x + max_x) / 2.0;
        assert!(
            (center - 500.0).abs() <= 10.0,
            "content center {center} must be within 10px of the window center 500 (bounds {min_x}..{max_x})"
        );
    }

    #[test]
    fn temp_probe_overflow() {
        let md = "| Unit | Cost | Limit | Effect |\n|------|------|-------|--------|\n| **Frigate** | 2 Alloy (Sorvath: 1) | 12 (Concord: 11) | Your muscle. Fleet and battle power for all of your battles, long enough to wrap and fill the last column fully. |\n".to_string();
        let sections = crate::search::split_sections(&md);
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut None, None);
            });
        harness.run();
        harness.run();
        let mut galleys: Vec<(f32, f32, String)> = Vec::new();
        for clipped in harness.output().shapes.iter() {
            if let egui::Shape::Text(t) = &clipped.shape {
                let l = t.galley.rect.left() + t.pos.x;
                let r = t.galley.rect.right() + t.pos.x;
                let text: String = t.galley.text().chars().take(40).collect();
                galleys.push((l, r, text));
            }
        }
        galleys.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (l, r, t) in galleys.iter().take(6) {
            println!("galley {l}..{r}: {t:?}");
        }
    }

    #[test]
    fn temp_probe_hit21() {
        let md = include_str!("../20-rules-v1.md").to_string();
        let sections = crate::search::split_sections(&md);
        let matches = crate::search::find_matches(&md, "card");
        assert_eq!(matches.len(), 22);
        let m = matches[20].clone(); // hit #21
        let section = crate::search::section_containing(&sections, m.start).unwrap();
        println!(
            "hit21 byte {} section {} of {}",
            m.start,
            section,
            sections.len()
        );
        let search = SearchHighlight {
            query: "card".into(),
            section,
            match_byte: m.start,
        };
        let mut view = RenderedView::new();
        let owned = md.clone();
        let search_for_ui = search;
        let jump = std::rc::Rc::new(std::cell::RefCell::new(Some(section)));
        let jump_for_ui = jump.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui(move |ui| {
                view.show(
                    ui,
                    &owned,
                    &sections,
                    &mut jump_for_ui.borrow_mut(),
                    Some(&search_for_ui),
                );
            });
        for frame in 1..=10 {
            harness.run();
            let yellow = yellow_rects(&harness);
            let (subtle, _, _) = highlight_shape_counts(&harness);
            let consumed = jump.borrow().is_none();
            println!(
                "frame {frame}: yellow={} subtle={} consumed={}",
                yellow.len(),
                subtle,
                consumed
            );
            for y in &yellow {
                println!("   yellow at {:?}", y);
            }
        }
    }

    #[test]
    fn table_inside_list_item_renders_below_text() {
        // A GFM table indented into a list item must be a block below the
        // item text, not flow beside it (regression: narrow tables fit the
        // remaining line and floated next to the text).
        let md = "2. Score points when Earth falls:\n   | Source | Points |\n   |--------|--------|\n   | Each Colony Dome | 3 |\n   | Each 3 claimed sectors | 1 |\n".to_string();
        let sections = crate::search::split_sections(&md);
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut None, None);
            });
        harness.run();
        harness.run();
        let rect_of = |needle: &str| -> Option<egui::Rect> {
            for clipped in harness.output().shapes.iter() {
                if let egui::Shape::Text(t) = &clipped.shape
                    && t.galley.text().contains(needle)
                {
                    let mut rect = t.galley.rect;
                    rect = rect.translate(t.pos.to_vec2());
                    return Some(rect);
                }
            }
            None
        };
        let item_text = rect_of("Score points").expect("item text painted");
        let cell = rect_of("Each Colony Dome").expect("table cell painted");
        assert!(
            cell.top() >= item_text.bottom() - 2.0,
            "table must render below the list item text: text bottom {} vs cell top {}",
            item_text.bottom(),
            cell.top()
        );
    }

    #[test]
    fn rulebook_scoring_table_renders_below_item_text() {
        // Exact content of the rulebook's section 3 item 2: a long lead
        // followed by an indented GFM table.
        let md = "2. **Highest score when Earth falls.** When the doom track ends, score points:\n   | Source | Points |\n   |--------|--------|\n   | Each Colony Dome | 3 |\n   | Each 3 claimed sectors | 1 |\n   | Most claimed sectors (if strictly alone in the lead) | 2 |\n   | Each bold dilemma choice you made | 1 |\n".to_string();
        let sections = crate::search::split_sections(&md);
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut None, None);
            });
        harness.run();
        harness.run();
        let rect_of = |needle: &str| -> Option<egui::Rect> {
            for clipped in harness.output().shapes.iter() {
                if let egui::Shape::Text(t) = &clipped.shape
                    && t.galley.text().contains(needle)
                {
                    let mut rect = t.galley.rect;
                    rect = rect.translate(t.pos.to_vec2());
                    return Some(rect);
                }
            }
            None
        };
        let item_text = rect_of("Highest score").expect("item text painted");
        let header = rect_of("Source").expect("table header painted");
        let cell = rect_of("Each Colony Dome").expect("table cell painted");
        // The table's top row must not overlap the item text's line.
        assert!(
            header.top() >= item_text.bottom() - 2.0,
            "table must render below the list item text: text bottom {} vs table top {}",
            item_text.bottom(),
            header.top()
        );
        assert!(
            cell.top() >= item_text.bottom() - 2.0,
            "table must render below the list item text: text bottom {} vs cell top {}",
            item_text.bottom(),
            cell.top()
        );
        // On its own line the table starts at the line's left edge, i.e.
        // at the same x as the item text — not somewhere to its right.
        assert!(
            (header.left() - item_text.left()).abs() < 40.0,
            "table must start at the line start (x {}), not beside the text (x {})",
            item_text.left(),
            header.left()
        );
    }

    #[test]
    fn table_last_column_fills_to_column_edge() {
        // The last column must stretch to the right edge of the reading
        // column instead of leaving dead space after a squeezed column.
        let md = "| Unit | Cost | Limit | Effect |\n|------|------|-------|--------|\n| **Frigate** | 2 Alloy (Sorvath: 1) | 12 (Concord: 11) | Your muscle. Fleet and battle power for all of your battles, long enough to wrap and fill the last column fully. |\n".to_string();
        let sections = crate::search::split_sections(&md);
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut None, None);
            });
        harness.run();
        harness.run();
        let column = column_width(900.0);
        let column_left = (900.0 - column) / 2.0;
        let column_right = column_left + column;
        let mut max_x = f32::MIN;
        for clipped in harness.output().shapes.iter() {
            if let egui::Shape::Text(t) = &clipped.shape {
                max_x = max_x.max(t.galley.rect.right() + t.pos.x);
            }
        }
        assert!(
            max_x >= column_right - 60.0,
            "table does not reach the column's right edge ({column_right}): max right {max_x}"
        );
        assert!(
            max_x <= column_right + 8.0,
            "text overflows the centered column ({column_left}..{column_right}): max right {max_x}"
        );
    }

    #[test]
    fn table_cell_inline_segments_do_not_explode_row_height() {
        // The rulebook's BUILD table has cells mixing plain and bold
        // segments ("Needs a colony-capable sector … **+2 morale** …").
        // Cell segments render as separate sequential labels; with a
        // non-wrapping cell the first label eats the whole cell width and
        // the rest wrap one fragment per line, stretching the row over a
        // thousand pixels.
        let md = "| Unit | Cost | Limit | Effect |\n|------|------|-------|--------|\n| **Frigate** | 2 Alloy (Sorvath: 1) | 12 (Concord: 11) | Your muscle. Fleet and battle power. |\n| **Colony Dome** | 4 Alloy (Ashkari: 5) | 5 | Needs a colony-capable sector you control (Terrestrial — Ellarien can also colonize Barren/Hostile at Tier 1). **+2 morale** (+3 Ellarien off-home). 3 points each at the end! |\n| **Outpost Beacon** | 1 Alloy | 6 | +2 strength when **defending** its sector. |\n".to_string();
        let sections = crate::search::split_sections(&md);
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut None, None);
            });
        harness.run();
        harness.run();
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for clipped in harness.output().shapes.iter() {
            if let egui::Shape::Text(t) = &clipped.shape {
                min_y = min_y.min(t.galley.rect.top() + t.pos.y);
                max_y = max_y.max(t.galley.rect.bottom() + t.pos.y);
            }
        }
        assert!(min_y < f32::MAX, "no text painted");
        // The whole table fits in a handful of wrapped lines per cell;
        // a sliver-wrapped cell would stretch it past any sane bound.
        assert!(
            max_y - min_y <= 400.0,
            "table grew to {} px tall — a cell is wrapping in slivers",
            max_y - min_y
        );
    }

    fn highlight_shape_counts(harness: &Harness<'_, ()>) -> (usize, usize, Option<usize>) {
        // (subtle bg_fill rects, current-accent rects, first highlight
        // index in the paint list; None if no highlights were painted)
        let subtle = egui::Style::default().visuals.selection.bg_fill;
        fn walk(
            shape: &egui::Shape,
            subtle: egui::Color32,
            index: usize,
            subtle_count: &mut usize,
            first_index: &mut Option<usize>,
        ) {
            match shape {
                egui::Shape::Vec(children) => {
                    for child in children {
                        walk(child, subtle, index, subtle_count, first_index);
                    }
                }
                egui::Shape::Rect(r) if r.fill == subtle => {
                    *subtle_count += 1;
                    if first_index.is_none() {
                        *first_index = Some(index);
                    }
                }
                _ => {}
            }
        }
        let mut subtle_count = 0;
        let mut first_index = None;
        for (i, clipped) in harness.output().shapes.iter().enumerate() {
            walk(
                &clipped.shape,
                subtle,
                i,
                &mut subtle_count,
                &mut first_index,
            );
        }
        let current_count = yellow_rects(harness).len();
        (subtle_count, current_count, first_index)
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

    fn black_text_copies(harness: &Harness<'_, ()>) -> usize {
        fn walk(shape: &egui::Shape, out: &mut usize) {
            match shape {
                egui::Shape::Vec(children) => {
                    for child in children {
                        walk(child, out);
                    }
                }
                egui::Shape::Text(t) if t.override_text_color == Some(egui::Color32::BLACK) => {
                    *out += 1;
                }
                _ => {}
            }
        }
        let mut out = 0;
        for clipped in harness.output().shapes.iter() {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    fn first_text_index(harness: &Harness<'_, ()>, needle: &str) -> Option<usize> {
        harness.output().shapes.iter().position(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(t)
                    if t.galley.text().contains(needle))
        })
    }

    #[test]
    fn rendered_search_highlights_visible_occurrences_behind_the_text() {
        let md = "alpha and alpha\n\n# Heading\n\nbeta text\n".to_string();
        let sections = crate::search::split_sections(&md);
        let search = SearchHighlight {
            query: "alpha".into(),
            section: 0,
            match_byte: 0,
        };
        let mut view = RenderedView::new();
        let owned = md.clone();
        let search_for_ui = search;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &owned, &sections, &mut None, Some(&search_for_ui));
            });
        harness.run();
        harness.run();
        let (subtle, current, first_highlight) = highlight_shape_counts(&harness);
        assert!(
            subtle + current >= 2,
            "both occurrences must be highlighted (subtle {subtle}, current {current})"
        );
        // Highlights must paint BEHIND the text, or they wash it out.
        let text_index = first_text_index(&harness, "alpha").expect("alpha text painted");
        let first_highlight = first_highlight.expect("highlights painted");
        assert!(
            first_highlight < text_index,
            "highlights (index {first_highlight}) must precede the text (index {text_index}) in the paint list"
        );
    }

    #[test]
    fn rendered_search_emphasizes_the_current_match() {
        // Two occurrences in different sections; the current match is the
        // one in the last section (byte 17). Only that occurrence may get
        // the yellow/current treatment.
        let md = "alpha\n\nmid text\n\nalpha end\n".to_string();
        let sections = crate::search::split_sections(&md);
        assert_eq!(sections.len(), 3);
        let search = SearchHighlight {
            query: "alpha".into(),
            section: 2,
            match_byte: 17,
        };
        let mut view = RenderedView::new();
        let owned = md.clone();
        let search_for_ui = search;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &owned, &sections, &mut None, Some(&search_for_ui));
            });
        harness.run();
        harness.run();
        let (subtle, current, _) = highlight_shape_counts(&harness);
        assert_eq!(
            (subtle, current),
            (1, 1),
            "the current match must be yellow-emphasized, the other stay subtle"
        );
    }

    #[test]
    fn rendered_search_emphasizes_second_match_on_the_same_row() {
        // Both occurrences share one visual row; the current match is the
        // second. The emphasis must move to it, not stay on the first.
        let md = "alpha alpha\n".to_string();
        let sections = crate::search::split_sections(&md);
        assert_eq!(sections.len(), 1);
        let search = SearchHighlight {
            query: "alpha".into(),
            section: 0,
            match_byte: 6,
        };
        let mut view = RenderedView::new();
        let owned = md.clone();
        let search_for_ui = search;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &owned, &sections, &mut None, Some(&search_for_ui));
            });
        harness.run();
        harness.run();
        // Where is the second occurrence?
        let second_rect = harness
            .output()
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(t) if t.galley.text().contains("alpha") => {
                    let start = crate::search::char_index_of_byte(t.galley.text(), 6);
                    let end = crate::search::char_index_of_byte(t.galley.text(), 11);
                    let from = t
                        .galley
                        .pos_from_cursor(egui::text::CCursor::new(egui::text::CharIndex(start)));
                    let to = t
                        .galley
                        .pos_from_cursor(egui::text::CCursor::new(egui::text::CharIndex(end)));
                    Some(egui::Rect::from_min_max(from.min, to.max).translate(t.pos.to_vec2()))
                }
                _ => None,
            })
            .expect("paragraph painted");
        // The yellow rect must sit on the second occurrence, not the first.
        let yellow = yellow_rects(&harness);
        assert_eq!(yellow.len(), 1, "exactly one current-match highlight");
        assert!(
            (yellow[0].left() - second_rect.left()).abs() < 2.0
                && (yellow[0].top() - second_rect.top()).abs() < 2.0,
            "yellow rect {:?} must cover the second occurrence {second_rect:?}",
            yellow[0]
        );
    }

    #[test]
    fn rendered_search_highlights_match_wrapping_inside_a_table_cell() {
        // Hit #21 of the rulebook ("card" in the Choir row of the Houses
        // table): the cell text wraps, and the match starts on a wrapped
        // row. pos_from_cursor maps the wrap-boundary character to the end
        // of the previous row, which used to produce an inverted rect that
        // was silently dropped — no highlight at all.
        let md = "| House | Soul | Weakness |\n|-------|------|----------|\n| **Choir** | +1 Data/round; Tier 1: +1 card/round; Tier 2: reroll a bad die once/round | Units cost +1 Data |\n".to_string();
        let sections = crate::search::split_sections(&md);
        let m = &crate::search::find_matches(&md, "card")[0];
        let search = SearchHighlight {
            query: "card".into(),
            section: 0,
            match_byte: m.start,
        };
        let mut view = RenderedView::new();
        let owned = md.clone();
        let search_for_ui = search;
        let jump = std::rc::Rc::new(std::cell::RefCell::new(Some(0usize)));
        let jump_for_ui = jump.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui(move |ui| {
                view.show(
                    ui,
                    &owned,
                    &sections,
                    &mut jump_for_ui.borrow_mut(),
                    Some(&search_for_ui),
                );
            });
        for _ in 0..6 {
            harness.run();
        }
        let yellow = yellow_rects(&harness);
        assert_eq!(
            yellow.len(),
            1,
            "the match wrapping inside a table cell must be highlighted"
        );
    }

    #[test]
    fn rendered_search_current_match_gets_black_text_over_yellow() {
        let md = "alpha alpha\n".to_string();
        let sections = crate::search::split_sections(&md);
        let search = SearchHighlight {
            query: "alpha".into(),
            section: 0,
            match_byte: 6,
        };
        let mut view = RenderedView::new();
        let owned = md.clone();
        let search_for_ui = search;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &owned, &sections, &mut None, Some(&search_for_ui));
            });
        harness.run();
        harness.run();
        let black_copies = black_text_copies(&harness);
        assert!(
            black_copies >= 1,
            "the current match must re-paint its glyphs in black over the yellow"
        );
    }

    #[test]
    fn rendered_search_skips_emphasis_when_current_section_not_visible() {
        // An out-of-range section index means the current match cannot be
        // attributed to any painted occurrence: no emphasis.
        let md = "alpha\n\nmid text\n\nalpha end\n".to_string();
        let sections = crate::search::split_sections(&md);
        let search = SearchHighlight {
            query: "alpha".into(),
            section: 99,
            match_byte: 17,
        };
        let mut view = RenderedView::new();
        let owned = md.clone();
        let search_for_ui = search;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &owned, &sections, &mut None, Some(&search_for_ui));
            });
        harness.run();
        harness.run();
        let (subtle, current, _) = highlight_shape_counts(&harness);
        assert_eq!(current, 0, "unknown section must not emphasize any match");
        assert!(subtle >= 2, "all occurrences still highlighted subtly");
    }

    #[test]
    fn rendered_search_jump_scrolls_the_current_match_into_view() {
        // A single tall section whose early content (a big table) makes the
        // byte-fraction estimate undershoot the match's real position by
        // more than half a viewport. The jump must still converge on the
        // match — a painted yellow rect inside the viewport proves it.
        // The code block is height-heavy but byte-light; the paragraphs
        // AFTER the match dilute its byte fraction, so the estimate undershoots
        // by roughly a viewport and the match lands below the fold.
        let mut md = String::from("# Heading\n\n```\n");
        for i in 0..200 {
            md.push_str(&format!("x{i}\n"));
        }
        md.push_str("```\n\n");
        let match_byte = md.len();
        md.push_str("needle right here\n\n");
        md.push_str("alpha bravo charlie delta echo foxtrot golf hotel india juliet\n");
        md.push_str("kilo lima mike november oscar papa quebec romeo sierra tango\n");
        md.push_str("uniform one two three four five six seven eight nine ten\n");
        md.push_str("eleven twelve thirteen fourteen fifteen sixteen seventeen\n");
        let sections = crate::search::split_sections(&md);
        assert_eq!(sections.len(), 1);
        let search = SearchHighlight {
            query: "needle".into(),
            section: 0,
            match_byte,
        };
        let mut view = RenderedView::new();
        let owned = md.clone();
        let search_for_ui = search;
        let jump = std::rc::Rc::new(std::cell::RefCell::new(Some(0usize)));
        let jump_for_ui = jump.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(
                    ui,
                    &owned,
                    &sections,
                    &mut jump_for_ui.borrow_mut(),
                    Some(&search_for_ui),
                );
            });
        for _ in 0..8 {
            harness.run();
        }
        assert!(jump.borrow().is_none(), "jump must be consumed");
        let yellow = yellow_rects(&harness);
        assert_eq!(
            yellow.len(),
            1,
            "the jumped-to match must have been scrolled into view and highlighted"
        );
        // The highlight must sit inside the visible area, not just be
        // painted somewhere below the fold.
        let in_view = harness
            .output()
            .shapes
            .iter()
            .any(|clipped| match &clipped.shape {
                egui::Shape::Vec(children) => children.iter().any(|child| {
                    matches!(child, egui::Shape::Rect(r)
                        if r.fill == egui::Color32::YELLOW
                            && clipped.clip_rect.contains_rect(r.rect))
                }),
                _ => false,
            });
        assert!(in_view, "the yellow highlight must be inside the viewport");
    }

    #[test]
    fn rendered_search_without_matches_paints_no_highlights() {
        let md = "alpha and alpha\n\n# Heading\n\nbeta text\n".to_string();
        let sections = crate::search::split_sections(&md);
        let search = SearchHighlight {
            query: "zzz".into(),
            section: 0,
            match_byte: 0,
        };
        let mut view = RenderedView::new();
        let owned = md.clone();
        let search_for_ui = search;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &owned, &sections, &mut None, Some(&search_for_ui));
            });
        harness.run();
        harness.run();
        let (subtle, current, first_highlight) = highlight_shape_counts(&harness);
        assert_eq!(
            (subtle, current, first_highlight),
            (0, 0, None),
            "no matches must paint no highlight rects"
        );
    }

    #[test]
    fn wide_table_stays_inside_centered_column() {
        // egui_commonmark renders table cells without wrapping; a long cell
        // made the grid overflow the centered column out to the window edge
        // (seen with table-heavy documents like a game rulebook).
        let md = "| Piece | Count | Notes |\n|-------|-------|-------|\n| Hex sector tiles | 28 | Terrestrial, Barren, Hostile, Asteroid, Gas Giant, Ruins, Deep Space; some with wormhole sockets |\n| Frigate tokens | 6 × 12 | Your fleets |\n\nA plain paragraph so the left edge is anchored by body text too.\n".to_string();
        let sections = crate::search::split_sections(&md);
        let mut view = RenderedView::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut None, None);
            });
        harness.run();
        harness.run();
        let column = column_width(900.0);
        let column_left = (900.0 - column) / 2.0;
        let column_right = column_left + column;
        let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
        for clipped in harness.output().shapes.iter() {
            if let egui::Shape::Text(t) = &clipped.shape {
                min_x = min_x.min(t.galley.rect.left() + t.pos.x);
                max_x = max_x.max(t.galley.rect.right() + t.pos.x);
            }
        }
        assert!(min_x < f32::MAX, "no text painted");
        // Small tolerance for rounding and the table frame's inner margin.
        assert!(
            max_x <= column_right + 8.0,
            "text overflows the centered column ({column_left}..{column_right}): max right {max_x}"
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
                view.show(ui, &md, &sections, &mut None, None);
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
