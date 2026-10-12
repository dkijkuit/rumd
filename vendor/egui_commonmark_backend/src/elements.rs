use egui::{self, NumExt, RichText, Sense, TextBuffer, TextStyle, Ui, Vec2, epaint};

#[inline]
pub fn rule(ui: &mut Ui, end_line: bool) {
    // rumd patch: a quiet 1px rule in GitHub's border color, with a
    // little breathing room, instead of egui's chunky interactive
    // separator.
    let color = crate::typography::palette(ui).border;
    let height = 1.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height + 12.0), Sense::hover());
    let line_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - height / 2.0),
        egui::vec2(rect.width(), height),
    );
    ui.painter().rect_filled(line_rect, 0.0, color);
    // This does not add a new line, but instead ends the separator
    if end_line {
        newline(ui);
    }
}

#[inline]
pub fn soft_break(ui: &mut Ui) {
    ui.label(" ");
}

#[inline]
pub fn newline(ui: &mut Ui) {
    ui.label("\n");
}

/// rumd patch: a line break whose row is only `gap` points tall.
///
/// Upstream separates every block with a full empty body line; with
/// explicit line heights everywhere that row would be a whole leading
/// tall (24pt at 16px body), which reads as scattered. Block gaps are
/// supposed to be margins, not blank lines, so they get their own
/// (small) height here. The label still contains "\n" because an empty
/// label is what forces the wrapping layout to break the row.
pub fn newline_gap(ui: &mut Ui, gap: f32) {
    let mut job = egui::text::LayoutJob::simple(
        "\n".to_owned(),
        TextStyle::Body.resolve(ui.style()),
        egui::Color32::TRANSPARENT,
        f32::INFINITY,
    );
    crate::typography::apply_line_height(&mut job, gap);
    ui.add(egui::Label::new(job));
}

pub fn bullet_point(ui: &mut Ui) {
    // rumd patch: marker slot sized to the body (see
    // typography::list_marker_width) instead of four space widths.
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(crate::typography::list_marker_width(ui), height_body(ui)),
        Sense::hover(),
    );
    // rumd patch: a GitHub-sized dot (upstream's height/6 is a pinprick
    // next to 16px Inter).
    ui.painter().circle_filled(
        rect.center(),
        (rect.height() / 4.5).at_least(2.0),
        ui.visuals().strong_text_color(),
    );
}

pub fn bullet_point_hollow(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(crate::typography::list_marker_width(ui), height_body(ui)),
        Sense::hover(),
    );
    ui.painter().circle(
        rect.center(),
        (rect.height() / 4.5).at_least(2.0),
        egui::Color32::TRANSPARENT,
        egui::Stroke::new(1.2, ui.visuals().strong_text_color()),
    );
}

pub fn number_point(ui: &mut Ui, number: &str) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(crate::typography::list_marker_width(ui), height_body(ui)),
        Sense::hover(),
    );
    ui.painter().text(
        rect.right_center(),
        egui::Align2::RIGHT_CENTER,
        format!("{number}."),
        TextStyle::Body.resolve(ui.style()),
        ui.visuals().strong_text_color(),
    );
}

#[inline]
pub fn footnote_start(ui: &mut Ui, note: &str) {
    ui.label(RichText::new(note).raised().strong().small());
}

pub fn footnote(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width_body_space(ui) * 4.0, height_body(ui)),
        Sense::hover(),
    );
    ui.painter().text(
        rect.right_top(),
        egui::Align2::RIGHT_TOP,
        format!("{text}."),
        TextStyle::Small.resolve(ui.style()),
        ui.visuals().strong_text_color(),
    );
}

fn height_body(ui: &Ui) -> f32 {
    ui.text_style_height(&TextStyle::Body)
}

fn width_body_space(ui: &Ui) -> f32 {
    let id = TextStyle::Body.resolve(ui.style());
    ui.fonts_mut(|f| f.glyph_width(&id, ' '))
}

/// Enhanced/specialized version of egui's code blocks. This one features copy button and borders
pub fn code_block<'t>(
    ui: &mut Ui,
    max_width: f32,
    text: &str,
    layouter: &'t mut dyn FnMut(&Ui, &dyn TextBuffer, f32) -> std::sync::Arc<egui::Galley>,
) {
    let mut text = text.strip_suffix('\n').unwrap_or(text);
    let palette = crate::typography::palette(ui);

    // rumd patch: GitHub-style chrome — a flat tinted panel with a hairline
    // border and generous padding, instead of egui's sunken widget frame.
    // The TextEdit itself is frameless so it doesn't paint its own
    // background over the panel.
    egui::Frame::new()
        .fill(palette.code_bg)
        .stroke(egui::Stroke::new(1.0, palette.code_border))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            let output = egui::TextEdit::multiline(&mut text)
                .layouter(layouter)
                .desired_width(max_width)
                // prevent trailing lines
                .desired_rows(1)
                // rumd patch: chrome comes from the surrounding frame;
                // the TextEdit itself paints nothing.
                .frame(egui::Frame::new())
                .margin(egui::Margin::symmetric(0, 2))
                .show(ui);
            code_block_copy_button(ui, &output, text);
        });
}

/// rumd patch: the copy affordance of a code block, factored out of
/// `code_block`.
fn code_block_copy_button(
    ui: &mut Ui,
    output: &egui::widgets::text_edit::TextEditOutput,
    text: &str,
) {
    let frame_rect = output.response.rect;

    // Copy icon
    let spacing = &ui.style().spacing;
    let position = egui::pos2(
        frame_rect.right_top().x - spacing.icon_width * 0.5 - spacing.button_padding.x,
        frame_rect.right_top().y + spacing.button_padding.y * 2.0,
    );

    // Check if we should show ✔ instead of 🗐 if the text was copied and the mouse is hovered
    let persistent_id = ui.make_persistent_id(output.response.id);
    let copied_icon = ui.memory_mut(|m| *m.data.get_temp_mut_or_default::<bool>(persistent_id));

    let copy_button = ui
        .put(
            egui::Rect {
                min: position,
                max: position,
            },
            egui::Button::new(if copied_icon { "✔" } else { "🗐" })
                .small()
                .frame(false)
                .fill(egui::Color32::TRANSPARENT),
        )
        // workaround for a regression after egui 0.27 where the edit cursor was shown even when
        // hovering over the button. We try interact_cursor first to allow the cursor to be
        // overriden
        .on_hover_cursor(
            ui.visuals()
                .interact_cursor
                .unwrap_or(egui::CursorIcon::Default),
        );

    // Update icon state in persistent memory
    if copied_icon && !copy_button.hovered() {
        ui.memory_mut(|m| *m.data.get_temp_mut_or_default(persistent_id) = false);
    }
    if !copied_icon && copy_button.clicked() {
        ui.memory_mut(|m| *m.data.get_temp_mut_or_default(persistent_id) = true);
    }

    if copy_button.clicked() {
        use egui::TextBuffer as _;
        let copy_text = if let Some(cursor) = output.cursor_range.clone() {
            let selected_chars = cursor.as_sorted_char_range();
            let selected_text = text.char_range(selected_chars);
            if selected_text.is_empty() {
                text.to_owned()
            } else {
                selected_text.to_owned()
            }
        } else {
            text.to_owned()
        };
        ui.copy_text(copy_text);
    }
}

// Stripped down version of egui's Checkbox. The only difference is that this
// creates a noninteractive checkbox. ui.add_enabled could have been used instead,
// but it makes the checkbox too grey.
pub struct ImmutableCheckbox<'a> {
    checked: &'a mut bool,
}

impl<'a> ImmutableCheckbox<'a> {
    pub fn without_text(checked: &'a mut bool) -> Self {
        ImmutableCheckbox { checked }
    }
}

impl egui::Widget for ImmutableCheckbox<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let ImmutableCheckbox { checked } = self;

        let spacing = &ui.spacing();
        let icon_width = spacing.icon_width;

        let mut desired_size = egui::vec2(icon_width, 0.0);
        desired_size = desired_size.at_least(Vec2::splat(spacing.interact_size.y));
        desired_size.y = desired_size.y.max(icon_width);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());

        if ui.is_rect_visible(rect) {
            let visuals = ui.style().visuals.noninteractive();
            let (small_icon_rect, big_icon_rect) = ui.spacing().icon_rectangles(rect);
            ui.painter().add(epaint::RectShape::new(
                big_icon_rect.expand(visuals.expansion),
                visuals.corner_radius,
                visuals.bg_fill,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            ));

            if *checked {
                // Check mark:
                ui.painter().add(egui::Shape::line(
                    vec![
                        egui::pos2(small_icon_rect.left(), small_icon_rect.center().y),
                        egui::pos2(small_icon_rect.center().x, small_icon_rect.bottom()),
                        egui::pos2(small_icon_rect.right(), small_icon_rect.top()),
                    ],
                    visuals.fg_stroke,
                ));
            }
        }

        response
    }
}

pub fn blockquote(ui: &mut Ui, _accent: egui::Color32, add_contents: impl FnOnce(&mut Ui)) {
    // rumd patch: GitHub-style quote — 4px quiet bar, muted text (the
    // caller weakens the text color), comfortable left padding.
    let start = ui.painter().add(egui::Shape::Noop);
    let response = egui::Frame::new()
        // Bar width plus breathing room before the text.
        .outer_margin(egui::Margin {
            left: 16,
            ..Default::default()
        })
        .show(ui, add_contents)
        .response;

    let color = crate::typography::palette(ui).border;
    ui.painter().set(
        start,
        egui::epaint::Shape::line_segment(
            [
                egui::pos2(response.rect.left_top().x, response.rect.left_top().y + 2.0),
                egui::pos2(
                    response.rect.left_bottom().x,
                    response.rect.left_bottom().y - 2.0,
                ),
            ],
            egui::Stroke::new(4.0, color),
        ),
    );
}
