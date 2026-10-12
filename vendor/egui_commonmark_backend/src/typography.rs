//! rumd patch: Typora/GitHub-flavored typography for rendered markdown.
//!
//! Everything here is a deliberate deviation from the upstream crate,
//! tuned to make rendered documents read like Typora's default (GitHub)
//! theme:
//!
//! - text labels are laid out with an explicit `TextFormat::line_height`
//!   (egui otherwise derives it from the font's own vertical metrics —
//!   ~1.15em for egui's bundled Ubuntu-Light, which read as a wall of
//!   text);
//! - bold/italic runs use real font families (`rumd-bold`, …) registered
//!   by rumd's `src/fonts.rs` — egui has no weight synthesis, so
//!   upstream's `RichText::strong()` only brightened the color;
//! - block chrome (quotes, code, rules, tables) uses GitHub's palette
//!   instead of raw egui visuals.

use egui::{Color32, Ui};

/// Font families registered by rumd's `src/fonts.rs`; literals are
/// duplicated there and must stay in sync.
pub const FAMILY_HEADING: &str = "rumd-heading";
pub const FAMILY_BOLD: &str = "rumd-bold";
pub const FAMILY_ITALIC: &str = "rumd-italic";
pub const FAMILY_BOLD_ITALIC: &str = "rumd-bold-italic";

/// GitHub's palette, keyed by mode. Light values are github.com's light
/// theme, dark values its dark theme (which Typora's default theme
/// mirrors).
pub struct Palette {
    /// Normal body text.
    pub text: Color32,
    /// Quote text and h6.
    pub muted: Color32,
    /// Blockquote bar and rules/borders.
    pub border: Color32,
    /// Fenced/indented code block background.
    pub code_bg: Color32,
    /// Code block border.
    pub code_border: Color32,
    /// Inline code background (translucent).
    pub inline_code_bg: Color32,
}

const LIGHT: Palette = Palette {
    text: Color32::from_rgb(0x1f, 0x23, 0x28),
    muted: Color32::from_rgb(0x59, 0x63, 0x6a),
    border: Color32::from_rgb(0xd1, 0xd9, 0xe0),
    code_bg: Color32::from_rgb(0xf6, 0xf8, 0xfa),
    code_border: Color32::from_rgb(0xd1, 0xd9, 0xe0),
    inline_code_bg: Color32::from_rgb(0xeb, 0xef, 0xf3),
};

const DARK: Palette = Palette {
    text: Color32::from_rgb(0xe6, 0xed, 0xf3),
    muted: Color32::from_rgb(0x91, 0x9d, 0xaa),
    border: Color32::from_rgb(0x3d, 0x44, 0x4d),
    code_bg: Color32::from_rgb(0x15, 0x1b, 0x23),
    code_border: Color32::from_rgb(0x3d, 0x44, 0x4d),
    inline_code_bg: Color32::from_rgb(0x2c, 0x33, 0x3d),
};

/// The GitHub palette for the ui's current mode.
pub fn palette(ui: &Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

/// Line-height factors relative to the run's font size.
pub const LINE_HEIGHT_BODY: f32 = 1.5;
pub const LINE_HEIGHT_HEADING: f32 = 1.25;
pub const LINE_HEIGHT_CODE: f32 = 1.45;

/// rumd patch: vertical gaps between blocks, in points at a 16px body.
/// Paragraph ends get [`GAP_BLOCK_END`]; every block (including
/// paragraphs and headings) starts with [`GAP_BLOCK_START`] so the two
/// stack to a Typora-like ~24pt between paragraphs and headings —
/// browsers would collapse these margins; the markdown flow cannot, so
/// the split is chosen to stack well.
pub const GAP_BLOCK_START: f32 = 8.0;
pub const GAP_BLOCK_END: f32 = 16.0;
/// Gap between consecutive list items (GitHub uses ~4px).
pub const GAP_LIST_ITEM: f32 = 4.0;

/// rumd patch: the body line height for the ui's current text styles.
pub fn body_line_height(ui: &Ui) -> f32 {
    ui.style()
        .text_styles
        .get(&egui::TextStyle::Body)
        .map_or(24.0, |d| d.size)
        * LINE_HEIGHT_BODY
}

/// rumd patch: the block-start gap, scaled to the body size (authored
/// against 16px).
pub fn gap_block_start(ui: &Ui) -> f32 {
    GAP_BLOCK_START * body_scale(ui)
}

/// rumd patch: the block-end gap, scaled to the body size.
pub fn gap_block_end(ui: &Ui) -> f32 {
    GAP_BLOCK_END * body_scale(ui)
}

/// rumd patch: the list-item gap, scaled to the body size.
pub fn gap_list_item(ui: &Ui) -> f32 {
    GAP_LIST_ITEM * body_scale(ui)
}

/// rumd patch: per-level list indentation in points (GitHub ≈ 32px at
/// 16px body).
pub fn list_indent(ui: &Ui) -> f32 {
    32.0 * body_scale(ui)
}

/// rumd patch: width of the bullet/number marker slot (GitHub's marker
/// gutter ≈ 26px at 16px body).
pub fn list_marker_width(ui: &Ui) -> f32 {
    26.0 * body_scale(ui)
}

fn body_scale(ui: &Ui) -> f32 {
    ui.style()
        .text_styles
        .get(&egui::TextStyle::Body)
        .map_or(1.0, |d| d.size / 16.0)
}

/// Heading font sizes for h1..=h6, Typora's default theme at a 16px body
/// (2em, 1.5em, 1.25em, 1em bold, 0.875em, 0.85em).
pub const HEADING_SIZES: [f32; 6] = [32.0, 24.0, 20.0, 16.0, 14.0, 13.5];

/// Set an explicit line height on every section of a layout job. This is
/// what gives markdown text its Typora-like leading: egui otherwise uses
/// the font's own (tight) vertical metrics.
pub fn apply_line_height(job: &mut egui::text::LayoutJob, line_height: f32) {
    for section in &mut job.sections {
        section.format.line_height = Some(line_height);
    }
}

/// Build a wrapping, selectable text label with an explicit line height,
/// preserving the RichText's families, sizes and colors.
pub fn text_label(ui: &mut Ui, rich_text: egui::RichText, line_height: f32) -> egui::Response {
    let mut job = egui::text::LayoutJob::default();
    rich_text.append_to(
        &mut job,
        ui.style(),
        egui::FontSelection::Default,
        egui::Align::LEFT,
    );
    apply_line_height(&mut job, line_height);
    ui.add(egui::Label::new(job))
}
