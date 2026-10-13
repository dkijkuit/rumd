//! Table of contents: heading extraction and the collapsible sidebar.

use std::collections::HashSet;

use eframe::egui;

/// Identity of an outline entry: the (level, title) chain from the top of
/// the outline down to the entry itself. Byte offsets shift while editing,
/// but these paths survive, so collapse state keyed by path persists
/// across live edits.
pub type EntryPath = Vec<(u8, String)>;

/// One markdown heading: nesting level (1-6), display title, and the byte
/// offset of its line in the raw markdown (matching `search::split_sections`
/// boundaries, so the offset maps to a rendered section start).
#[derive(Clone)]
pub struct TocEntry {
    pub level: u8,
    pub title: String,
    pub byte: usize,
}

/// Every ATX heading (`#`-`######` + space) outside a ``` / ~~~ fence, in
/// document order. Line-leading only (up to the same leniency as
/// `search::split_sections`); setext underlines are not headings here.
pub fn headings(text: &str) -> Vec<TocEntry> {
    let mut out = Vec::new();
    let mut fence: Option<char> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
        } else if trimmed.starts_with("```") {
            fence = Some('`');
        } else if trimmed.starts_with("~~~") {
            fence = Some('~');
        } else if let Some((level, title)) = atx_heading(trimmed) {
            out.push(TocEntry {
                level,
                title: title.to_string(),
                byte: offset,
            });
        }
        offset += line.len();
    }
    out
}

/// Parse an ATX heading line body: 1-6 leading `#`s, at least one space,
/// closing `#`s (separated by a space) stripped, title trimmed.
fn atx_heading(trimmed: &str) -> Option<(u8, &str)> {
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 || !trimmed[hashes..].starts_with(' ') {
        return None;
    }
    let body = trimmed[hashes..].trim();
    // A run of trailing hashes only closes the heading when a space
    // separates it from the title ("# Title #", not "# Title#").
    let bare = body.trim_end_matches('#');
    let title = if bare.len() < body.len() && (bare.is_empty() || bare.ends_with(' ')) {
        bare.trim_end()
    } else {
        body
    };
    Some((hashes as u8, title))
}

/// Path from the outline root down to entry `i`: every preceding entry
/// that nests it (walking back through ever shallower levels), plus `i`.
pub fn entry_path(entries: &[TocEntry], i: usize) -> EntryPath {
    let mut chain: Vec<(u8, String)> = Vec::new();
    let mut level = entries[i].level;
    for entry in entries[..i].iter().rev() {
        if entry.level < level {
            chain.push((entry.level, entry.title.clone()));
            level = entry.level;
        }
    }
    chain.reverse();
    chain.push((entries[i].level, entries[i].title.clone()));
    chain
}

/// True when entry `i` is immediately followed by deeper entries, i.e. it
/// can collapse a group.
pub fn has_children(entries: &[TocEntry], i: usize) -> bool {
    entries
        .get(i + 1)
        .is_some_and(|next| next.level > entries[i].level)
}

/// Indices of the entries that are shown: each entry whose path is in
/// `collapsed` stays visible itself, but hides all following entries
/// deeper than its level (until one at its level or shallower appears).
pub fn visible(entries: &[TocEntry], collapsed: &HashSet<EntryPath>) -> Vec<usize> {
    let mut shown = Vec::new();
    let mut hiding_below: Option<u8> = None;
    for (i, entry) in entries.iter().enumerate() {
        if hiding_below.is_some_and(|level| entry.level > level) {
            continue;
        }
        hiding_below = None;
        shown.push(i);
        if collapsed.contains(&entry_path(entries, i)) {
            hiding_below = Some(entry.level);
        }
    }
    shown
}

/// What the user did inside the panel this frame.
#[derive(Default, Clone)]
pub struct TocPanelIntents {
    /// Byte offset of a clicked entry: jump the document view there.
    pub jump_to: Option<usize>,
    /// Path of a group whose chevron was clicked: toggle its collapsed
    /// state.
    pub toggled: Option<EntryPath>,
}

/// The sidebar contents: one flat row per visible entry, groups prefixed
/// with an expand/collapse chevron in a left gutter. Clicking a row jumps
/// to its section; clicking a group's chevron toggles the group. The
/// `active` entry (the section currently in view) paints with the
/// selection style.
pub fn show_panel(
    ui: &mut egui::Ui,
    entries: &[TocEntry],
    collapsed: &HashSet<EntryPath>,
    active: Option<usize>,
) -> TocPanelIntents {
    let mut intents = TocPanelIntents::default();
    if entries.is_empty() {
        ui.label(
            egui::RichText::new("No headings")
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        return intents;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let shown = visible(entries, collapsed);
            let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
            for i in shown {
                let entry = &entries[i];
                let group = has_children(entries, i);
                let expanded = group && !collapsed.contains(&entry_path(entries, i));
                let is_active = active == Some(i);
                let (row_rect, row) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), row_h),
                    egui::Sense::click(),
                );
                let title = entry.title.clone();
                row.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, row.enabled(), &title)
                });
                let indent = (entry.level as f32 - 1.0) * INDENT;
                let chev_rect = egui::Rect::from_center_size(
                    egui::pos2(row_rect.left() + indent + GUTTER / 2.0, row_rect.center().y),
                    egui::vec2(GUTTER, row_h),
                );
                // The whole row is one hover target: the fill reacts to the
                // pointer anywhere on it, chevron included.
                let visuals = ui.visuals();
                let hovered = row.hovered();
                let fill = if is_active {
                    visuals.selection.bg_fill
                } else if hovered {
                    visuals.widgets.hovered.bg_fill
                } else {
                    egui::Color32::TRANSPARENT
                };
                if fill != egui::Color32::TRANSPARENT {
                    ui.painter().rect_filled(row_rect, ROW_ROUNDING, fill);
                }
                let color = if is_active {
                    visuals.selection.stroke.color
                } else {
                    match entry.level {
                        1 => visuals.strong_text_color(),
                        2 => visuals.text_color(),
                        _ => visuals.weak_text_color(),
                    }
                };
                paint_title(
                    ui,
                    row_rect.left() + indent + GUTTER,
                    row_rect,
                    &title,
                    color,
                );
                if group {
                    let chevron = chevron(ui, chev_rect, expanded, &title);
                    if chevron.clicked() {
                        intents.toggled = Some(entry_path(entries, i));
                    } else if row.clicked() {
                        intents.jump_to = Some(entry.byte);
                    }
                } else if row.clicked() {
                    intents.jump_to = Some(entry.byte);
                }
            }
        });
    intents
}

/// Width of the chevron gutter every row reserves, so titles align
/// between group and leaf entries. The glyph is sized to fit inside it,
/// keeping `CHEVRON_TEXT_GAP` of clear space before the title.
const GUTTER: f32 = 20.0;
/// Indentation per heading level.
const INDENT: f32 = 14.0;
/// Corner rounding of the row hover/selection fill.
const ROW_ROUNDING: f32 = 4.0;
/// Breathing room kept between the chevron glyph and the row title.
const CHEVRON_TEXT_GAP: f32 = 4.0;

/// Draw a row's title, left-aligned and vertically centered in the row,
/// elided with an ellipsis when it does not fit.
fn paint_title(
    ui: &egui::Ui,
    text_x: f32,
    row_rect: egui::Rect,
    title: &str,
    color: egui::Color32,
) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let max_w = row_rect.right() - text_x;
    let text = elided(ui, title, &font, max_w);
    ui.painter().text(
        egui::pos2(text_x, row_rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        font,
        color,
    );
}

/// Cut `text` back until it (plus an ellipsis) fits `max_w`.
fn elided(ui: &egui::Ui, text: &str, font: &egui::FontId, max_w: f32) -> String {
    let measure = |s: &str| {
        ui.painter()
            .layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE)
            .size()
            .x
    };
    if measure(text) <= max_w {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if measure(&candidate) <= max_w {
            return candidate;
        }
    }
    "…".to_owned()
}

/// A small expand/collapse glyph button in the row's gutter. The visible
/// glyph is minimal, so the accessible label spells the action out
/// ("Collapse Title").
fn chevron(ui: &mut egui::Ui, rect: egui::Rect, expanded: bool, title: &str) -> egui::Response {
    let response = ui.allocate_rect(rect, egui::Sense::click());
    let label = if expanded {
        format!("Collapse {title}")
    } else {
        format!("Expand {title}")
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), &label)
    });
    let color = if response.hovered() {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    // Painted instead of typeset: the default font stack has no ▸/▾
    // glyphs (they render as tofu squares). Same approach as icons.rs.
    // The glyph scales with the row height but is capped so it stays
    // inside the gutter with CHEVRON_TEXT_GAP to spare before the
    // title; otherwise it creeps onto (or past) the text.
    let c = rect.center();
    let w = (rect.height() * 0.42).min(GUTTER / 2.0 - CHEVRON_TEXT_GAP);
    let points = if expanded {
        vec![
            egui::pos2(c.x - w, c.y - w * 0.6),
            egui::pos2(c.x + w, c.y - w * 0.6),
            egui::pos2(c.x, c.y + w * 0.7),
        ]
    } else {
        vec![
            egui::pos2(c.x - w * 0.6, c.y - w),
            egui::pos2(c.x - w * 0.6, c.y + w),
            egui::pos2(c.x + w * 0.7, c.y),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn extracts_levels_titles_and_byte_offsets() {
        let md = "# Top\n\ntext\n\n## Sub\n\nmore\n\n### Deep\n";
        let hs = headings(md);
        assert_eq!(hs.len(), 3);
        assert_eq!((hs[0].level, hs[0].title.as_str()), (1, "Top"));
        assert_eq!((hs[1].level, hs[1].title.as_str()), (2, "Sub"));
        assert_eq!((hs[2].level, hs[2].title.as_str()), (3, "Deep"));
        assert_eq!(hs[0].byte, 0);
        assert_eq!(hs[1].byte, md.find("## Sub").unwrap());
        assert_eq!(hs[2].byte, md.find("### Deep").unwrap());
    }

    #[test]
    fn fenced_headings_are_ignored() {
        let md = "# Real\n\n```rust\n# not a heading\nlet x = 1;\n```\n\n~~~\n## also not\n~~~\n\n## Also real\n";
        let hs = headings(md);
        let titles: Vec<&str> = hs.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles, vec!["Real", "Also real"]);
    }

    #[test]
    fn hash_without_space_is_not_a_heading() {
        let md = "#tag\n\n##hashtag\n\n# Real\n";
        let hs = headings(md);
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].title, "Real");
    }

    #[test]
    fn more_than_six_hashes_is_not_a_heading() {
        let md = "####### seven\n\n# Real\n";
        let hs = headings(md);
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].title, "Real");
    }

    #[test]
    fn closing_hashes_are_trimmed_from_titles() {
        let md = "# Closed #\n\n## Deeper ###\n";
        let hs = headings(md);
        assert_eq!(hs[0].title, "Closed");
        assert_eq!(hs[1].title, "Deeper");
    }

    #[test]
    fn titles_keep_inline_markdown_verbatim() {
        let md = "# Some *bold* `code` title\n";
        let hs = headings(md);
        assert_eq!(hs[0].title, "Some *bold* `code` title");
    }

    #[test]
    fn titles_are_trimmed() {
        let md = "#   Spaced   \n";
        let hs = headings(md);
        assert_eq!(hs[0].title, "Spaced");
    }

    #[test]
    fn empty_text_has_no_headings() {
        assert!(headings("").is_empty());
        assert!(headings("just text\n\nmore text\n").is_empty());
    }

    #[test]
    fn unclosed_fence_swallows_the_rest() {
        let md = "# Real\n\n```\n# never\n## never either\n";
        let hs = headings(md);
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].title, "Real");
    }

    fn sample() -> Vec<TocEntry> {
        // # A / ## B / ### C / ## D / # E
        headings("# A\n\n## B\n\n### C\n\n## D\n\n# E\n")
    }

    fn md_offsets(md: &str) -> Vec<usize> {
        vec![
            md.find("# A").unwrap(),
            md.find("## B").unwrap(),
            md.find("### C").unwrap(),
            md.find("## D").unwrap(),
            md.find("# E").unwrap(),
        ]
    }

    fn render(
        entries: &[TocEntry],
        active: Option<usize>,
    ) -> egui_kittest::Harness<'static, TocPanelIntents> {
        let entries = entries.to_vec();
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(300.0, 400.0))
            .build_ui_state(
                // Intents are per-frame (the app merges them into its own
                // state); accumulate here so a repaint frame after the
                // click cannot wipe them.
                move |ui, state| {
                    let intents = show_panel(ui, &entries, &Default::default(), active);
                    if intents.jump_to.is_some() {
                        state.jump_to = intents.jump_to;
                    }
                    if intents.toggled.is_some() {
                        state.toggled = intents.toggled;
                    }
                },
                TocPanelIntents::default(),
            )
    }

    fn filled_rects<State>(harness: &egui_kittest::Harness<'_, State>) -> Vec<egui::Rect> {
        harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill != egui::Color32::TRANSPARENT => {
                    Some(rect.rect)
                }
                _ => None,
            })
            .collect()
    }

    fn text_color_in<State>(
        harness: &egui_kittest::Harness<'_, State>,
        row: egui::Rect,
    ) -> Option<egui::Color32> {
        harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if row.contains_rect(text.visual_bounding_rect()) => {
                    Some(text.fallback_color)
                }
                _ => None,
            })
            .next()
    }

    #[test]
    fn clicking_a_leaf_row_jumps_to_its_section() {
        let entries = sample();
        let md = "# A\n\n## B\n\n### C\n\n## D\n\n# E\n";
        let mut harness = render(&entries, None);
        harness.get_by_label("C").click();
        harness.run();
        let intents = harness.state().clone();
        assert_eq!(intents.jump_to, Some(md_offsets(md)[2]));
        assert!(intents.toggled.is_none());
    }

    #[test]
    fn clicking_a_group_row_jumps_too() {
        let entries = sample();
        let md = "# A\n\n## B\n\n### C\n\n## D\n\n# E\n";
        let mut harness = render(&entries, None);
        harness.get_by_label("B").click();
        harness.run();
        let intents = harness.state().clone();
        assert_eq!(intents.jump_to, Some(md_offsets(md)[1]));
    }

    #[test]
    fn clicking_a_chevron_toggles_without_jumping() {
        let entries = sample();
        let mut harness = render(&entries, None);
        harness.get_by_label("Collapse B").click();
        harness.run();
        let intents = harness.state().clone();
        assert_eq!(intents.toggled, Some(entry_path(&entries, 1)));
        assert!(intents.jump_to.is_none(), "chevron must not jump");
    }

    #[test]
    fn rows_paint_no_fill_until_hovered_or_active() {
        let entries = sample();
        let harness = render(&entries, None);
        let rows: Vec<egui::Rect> = ["A", "B", "C", "D", "E"]
            .iter()
            .map(|title| harness.get_by_label(title).rect())
            .collect();
        let fills: Vec<egui::Rect> = filled_rects(&harness)
            .into_iter()
            .filter(|fill| {
                rows.iter()
                    .any(|row| fill.height() <= row.height() + 4.0 && fill.intersects(*row))
            })
            .collect();
        assert!(
            fills.is_empty(),
            "idle rows must be flat, found fills at {fills:?}"
        );
    }

    #[test]
    fn title_colors_step_down_by_level() {
        let entries = sample();
        let harness = render(&entries, None);
        let ctx = egui::Context::default();
        let style = ctx.style_of(egui::Theme::Dark);
        let visuals = &style.visuals;
        let a = harness.get_by_label("A").rect();
        let b = harness.get_by_label("B").rect();
        let c = harness.get_by_label("C").rect();
        assert_eq!(
            text_color_in(&harness, a),
            Some(visuals.strong_text_color())
        );
        assert_eq!(text_color_in(&harness, b), Some(visuals.text_color()));
        assert_eq!(text_color_in(&harness, c), Some(visuals.weak_text_color()));
    }

    #[test]
    fn the_active_row_paints_selection_style() {
        let entries = sample();
        let harness = render(&entries, Some(2));
        let ctx = egui::Context::default();
        let style = ctx.style_of(egui::Theme::Dark);
        let visuals = &style.visuals;
        let c = harness.get_by_label("C").rect();
        let d = harness.get_by_label("D").rect();
        let covering = |row: egui::Rect| {
            filled_rects(&harness).into_iter().any(|fill| {
                fill.height() <= row.height() + 4.0 && fill.contains_rect(row.shrink(1.0))
            })
        };
        assert!(covering(c), "the active row must paint a selection fill");
        assert!(
            !covering(d),
            "inactive rows must not paint a selection fill"
        );
        assert_eq!(
            text_color_in(&harness, c),
            Some(visuals.selection.stroke.color),
            "the active row's title must use the accent color"
        );
    }

    #[test]
    fn chevron_keeps_a_gap_from_the_title() {
        let entries = headings("# Top\n\n## Sub\n\nbody\n");
        let harness = render(&entries, None);
        let row = harness.get_by_label("Top").rect();
        let chev = harness.get_by_label("Collapse Top").rect();
        let mut triangle_right = f32::MIN;
        let mut title_left = f32::MAX;
        for clipped in &harness.output().shapes {
            match &clipped.shape {
                egui::Shape::Path(poly)
                    if poly.points.len() == 3
                        && poly.fill != egui::Color32::TRANSPARENT
                        && poly.points.iter().all(|p| chev.expand(2.0).contains(*p)) =>
                {
                    triangle_right = triangle_right.max(
                        poly.points.iter().map(|p| p.x).fold(f32::MIN, f32::max),
                    );
                }
                egui::Shape::Text(text) if row.contains_rect(text.visual_bounding_rect()) => {
                    title_left = title_left.min(text.visual_bounding_rect().left());
                }
                _ => {}
            }
        }
        let gap = title_left - triangle_right;
        assert!(
            gap >= CHEVRON_TEXT_GAP - 0.05,
            "chevron-to-title gap {gap:.2} must be at least {CHEVRON_TEXT_GAP}"
        );
    }

    #[test]
    fn chevron_paints_a_filled_triangle() {
        let entries = headings("# Top\n\n## Sub\n\nbody\n");
        let mut harness = render(&entries, None);
        harness.run();
        let rect = harness.get_by_label("Collapse Top").rect();
        let triangles = harness
            .output()
            .shapes
            .iter()
            .filter(|clipped| match &clipped.shape {
                egui::Shape::Path(poly) => {
                    poly.points.len() == 3
                        && poly.fill != egui::Color32::TRANSPARENT
                        && poly.points.iter().all(|p| rect.expand(2.0).contains(*p))
                }
                _ => false,
            })
            .count();
        assert!(
            triangles >= 1,
            "the chevron must paint a filled triangle, not typeset a glyph"
        );
    }

    #[test]
    fn nothing_collapsed_shows_every_entry() {
        let entries = sample();
        assert_eq!(visible(&entries, &Default::default()), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn collapsed_group_hides_only_deeper_entries() {
        let entries = sample();
        let mut collapsed = std::collections::HashSet::new();
        collapsed.insert(entry_path(&entries, 1)); // collapse B
        assert_eq!(visible(&entries, &collapsed), vec![0, 1, 3, 4]);
    }

    #[test]
    fn hiding_ends_at_the_collapsed_level() {
        let entries = sample();
        let mut collapsed = std::collections::HashSet::new();
        collapsed.insert(entry_path(&entries, 1)); // collapse B: hides C, D stays
        assert!(
            visible(&entries, &collapsed).contains(&3),
            "D must survive B's collapse"
        );
    }

    #[test]
    fn paths_distinguish_same_titles_in_different_parents() {
        let md = "# A\n\n## Same\n\n# B\n\n## Same\n";
        let entries = headings(md);
        assert_ne!(entry_path(&entries, 1), entry_path(&entries, 3));
    }

    #[test]
    fn only_entries_followed_by_deeper_ones_are_groups() {
        let entries = sample();
        let groups: Vec<bool> = (0..entries.len())
            .map(|i| has_children(&entries, i))
            .collect();
        assert_eq!(groups, vec![true, true, false, false, false]);
    }
}
