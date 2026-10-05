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
#[derive(Default)]
pub struct TocPanelIntents {
    /// Byte offset of a clicked entry: jump the document view there.
    pub jump_to: Option<usize>,
    /// Path of a group whose chevron was clicked: toggle its collapsed
    /// state.
    pub toggled: Option<EntryPath>,
}

/// The sidebar contents: one indented row per visible entry, groups
/// prefixed with an expand/collapse chevron.
pub fn show_panel(
    ui: &mut egui::Ui,
    entries: &[TocEntry],
    collapsed: &HashSet<EntryPath>,
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
            let row_h = ui.text_style_height(&egui::TextStyle::Small);
            for i in shown {
                let entry = &entries[i];
                let group = has_children(entries, i);
                let expanded = group && !collapsed.contains(&entry_path(entries, i));
                ui.horizontal(|ui| {
                    ui.add_space((entry.level as f32 - 1.0) * 14.0);
                    if group {
                        if chevron(ui, expanded, &entry.title).clicked() {
                            intents.toggled = Some(entry_path(entries, i));
                        }
                    } else {
                        ui.add_space(16.0);
                    }
                    let button =
                        egui::Button::new(egui::RichText::new(entry.title.clone()).small())
                            .truncate()
                            .min_size(egui::vec2(ui.available_width(), row_h));
                    if ui.add(button).clicked() {
                        intents.jump_to = Some(entry.byte);
                    }
                });
            }
        });
    intents
}

/// A small expand/collapse glyph button. The visible glyph is minimal, so
/// the accessible label spells the action out ("Collapse Title").
fn chevron(ui: &mut egui::Ui, expanded: bool, title: &str) -> egui::Response {
    let side = ui.text_style_height(&egui::TextStyle::Small);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(side + 2.0, side), egui::Sense::click());
    response.widget_info(|| {
        let label = if expanded {
            format!("Collapse {title}")
        } else {
            format!("Expand {title}")
        };
        egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), &label)
    });
    let color = if response.hovered() {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    // Painted instead of typeset: the default font stack has no ▸/▾
    // glyphs (they render as tofu squares). Same approach as icons.rs.
    let c = rect.center();
    let w = side * 0.42;
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

    #[test]
    fn chevron_paints_a_filled_triangle() {
        use egui_kittest::kittest::Queryable;
        let entries = headings("# Top\n\n## Sub\n\nbody\n");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(300.0, 200.0))
            .build_ui(|ui| {
                show_panel(ui, &entries, &Default::default());
            });
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
