use std::iter::Peekable;
use std::ops::Range;

use crate::{CommonMarkCache, CommonMarkOptions};

use egui::{self, Id, Pos2, TextStyle, Ui};

/// rumd patch: layout for a single table cell's Ui.
///
/// Cells are vertical Uis: a cell's inline segments (plain + bold + …
/// render as separate sequential labels) stack as wrapped lines with
/// correct metrics, so grid row heights match the painted content. In a
/// horizontal cell the first label would consume the cell width and the
/// rest would wrap into an unusable sliver one fragment per line.
///
/// Labels default to `TextWrapMode::Extend` in a vertical Ui when
/// selectable, so wrap mode is forced; cell text then wraps at the widths
/// below instead of sizing columns to full unwrapped cell text.
///
/// All columns get `set_min_width`: a stable wrap width, immune to the
/// grid's first-frame sizing pass (which would otherwise wrap cells in a
/// narrow column and leave permanently inflated row heights). Non-last
/// columns are also capped to their equal share; the last column is
/// uncapped so `num_columns` expands it to the container's right edge.
///
/// Non-last columns get a fixed equal share of the available width (a stable
/// wrap target, immune to the grid's sizing feedback). The last column gets
/// no cap: the grid is created with `num_columns`, which expands it to fill
/// the remaining width up to the container's right edge.
fn rumd_cell_layout(ui: &mut Ui, col_index: usize, column_count: usize, column_width: f32) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    ui.set_min_width(column_width);
    if col_index + 1 != column_count {
        ui.set_max_width(column_width);
    }
}

use crate::List;
use egui_commonmark_backend::elements::*;
use egui_commonmark_backend::misc::*;
use egui_commonmark_backend::pulldown::*;
use pulldown_cmark::{CowStr, HeadingLevel};

/// Newline logic is constructed by the following:
/// All elements try to insert a newline before them (if they are allowed)
/// and end their own line.
struct Newline {
    /// Whether a newline should not be inserted before a widget. This is only for
    /// the first widget.
    should_not_start_newline_forced: bool,
    /// Whether an element should insert a newline before it
    should_start_newline: bool,
    /// Whether an element should end it's own line using a newline
    /// This will have to be set to false in cases such as when blocks are within
    /// a list.
    should_end_newline: bool,
    /// only false when the widget is the last one.
    should_end_newline_forced: bool,
}

impl Default for Newline {
    fn default() -> Self {
        Self {
            should_not_start_newline_forced: true,
            should_start_newline: true,
            should_end_newline: true,
            should_end_newline_forced: true,
        }
    }
}

impl Newline {
    pub fn can_insert_end(&self) -> bool {
        self.should_end_newline && self.should_end_newline_forced
    }

    pub fn can_insert_start(&self) -> bool {
        self.should_start_newline && !self.should_not_start_newline_forced
    }

    pub fn try_insert_start(&self, ui: &mut Ui) {
        // rumd patch: blocks are separated by a small margin row, not a
        // full blank line (see typography::gap_block_start).
        if self.can_insert_start() {
            newline_gap(
                ui,
                egui_commonmark_backend::typography::gap_block_start(ui),
            );
        }
    }

    pub fn try_insert_end(&self, ui: &mut Ui) {
        if self.can_insert_end() {
            newline_gap(
                ui,
                egui_commonmark_backend::typography::gap_block_end(ui),
            );
        }
    }
}

#[derive(Default)]
struct DefinitionList {
    is_first_item: bool,
    is_def_list_def: bool,
}

pub struct CommonMarkViewerInternal {
    curr_table: usize,
    text_style: Style,
    list: List,
    link: Option<Link>,
    image: Option<Image>,
    line: Newline,
    code_block: Option<CodeBlock>,

    /// Only populated if the html_fn option has been set
    html_block: String,
    is_list_item: bool,
    def_list: DefinitionList,
    is_table: bool,
    /// rumd patch: heading level (0 = h1) whose h1/h2 bottom border is
    /// still awaited; set on heading start, painted on heading end.
    pending_heading_border: Option<u8>,
    is_blockquote: bool,
    checkbox_events: Vec<CheckboxClickEvent>,
    deferred_scroll_to_heading: Option<String>,
}

pub(crate) struct CheckboxClickEvent {
    pub(crate) checked: bool,
    pub(crate) span: Range<usize>,
}

impl CommonMarkViewerInternal {
    pub fn new() -> Self {
        Self {
            curr_table: 0,
            text_style: Style::default(),
            list: List::default(),
            link: None,
            image: None,
            line: Newline::default(),
            is_list_item: false,
            def_list: Default::default(),
            code_block: None,
            html_block: String::new(),
            is_table: false,
            pending_heading_border: None,
            is_blockquote: false,
            checkbox_events: Vec::new(),
            deferred_scroll_to_heading: None,
        }
    }
}

fn parser_options_extras(
    is_math_enabled: bool,
    is_scroll_to_heading_enabled: bool,
) -> pulldown_cmark::Options {
    let mut result = parser_options();
    if is_math_enabled {
        result |= pulldown_cmark::Options::ENABLE_MATH;
    }
    if is_scroll_to_heading_enabled {
        result |= pulldown_cmark::Options::ENABLE_HEADING_ATTRIBUTES;
    }
    result
}

impl CommonMarkViewerInternal {
    /// Be aware that this acquires egui::Context internally.
    /// If split Id is provided then split points will be populated
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        text: &str,
        split_points_id: Option<Id>,
    ) -> (egui::InnerResponse<()>, Vec<CheckboxClickEvent>) {
        let max_width = options.max_width(ui);
        let layout = egui::Layout::left_to_right(egui::Align::BOTTOM).with_main_wrap(true);

        let re = ui.allocate_ui_with_layout(egui::vec2(max_width, 0.0), layout, |ui| {
            // rumd patch: horizontal spacing comes from explicit spaces
            // in the text flow; vertical spacing from explicit gap rows
            // (see Newline). Default item spacing would double every gap.
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.spacing_mut().item_spacing.x = 0.0;
            let height = ui.text_style_height(&TextStyle::Body);
            ui.set_row_height(height);

            let mut events = pulldown_cmark::Parser::new_ext(
                text,
                parser_options_extras(options.math_fn.is_some(), options.enable_scroll_to_heading),
            )
            .into_offset_iter()
            .enumerate()
            .peekable();

            while let Some((index, (e, src_span))) = events.next() {
                let start_position = ui.next_widget_position();
                let is_element_end = matches!(e, pulldown_cmark::Event::End(_));
                let should_add_split_point = self.list.is_inside_a_list() && is_element_end;

                if events.peek().is_none() {
                    self.line.should_end_newline_forced = false;
                }

                self.process_event(ui, &mut events, e, src_span, cache, options, max_width);

                if let Some(source_id) = split_points_id
                    && should_add_split_point
                {
                    let scroll_cache = scroll_cache(cache, &source_id);
                    let end_position = ui.next_widget_position();

                    let split_point_exists = scroll_cache
                        .split_points
                        .iter()
                        .any(|(i, _, _)| *i == index);

                    if !split_point_exists {
                        scroll_cache
                            .split_points
                            .push((index, start_position, end_position));
                    }
                }

                if index == 0 {
                    self.line.should_not_start_newline_forced = false;
                }
            }

            // deferral to make it consistent no matter whether the target is before or after the link
            *cache.scroll_to_id_target_mut() = self.deferred_scroll_to_heading.take();

            if let Some(source_id) = split_points_id {
                scroll_cache(cache, &source_id).page_size =
                    Some(ui.next_widget_position().to_vec2());
            }
        });

        (re, std::mem::take(&mut self.checkbox_events))
    }

    pub(crate) fn show_scrollable(
        &mut self,
        source_id: Id,
        ui: &mut egui::Ui,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        text: &str,
    ) {
        let available_size = ui.available_size();
        let scroll_id = source_id.with("_scroll_area");

        let Some(page_size) = scroll_cache(cache, &source_id).page_size else {
            egui::ScrollArea::vertical()
                .id_salt(scroll_id)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    self.show(ui, cache, options, text, Some(source_id));
                });
            // Prevent repopulating points twice at startup
            scroll_cache(cache, &source_id).available_size = available_size;
            return;
        };

        let events = pulldown_cmark::Parser::new_ext(
            text,
            parser_options_extras(options.math_fn.is_some(), options.enable_scroll_to_heading),
        )
        .into_offset_iter()
        .collect::<Vec<_>>();

        let num_rows = events.len();

        egui::ScrollArea::vertical()
            .id_salt(scroll_id)
            // Elements have different widths, so the scroll area cannot try to shrink to the
            // content, as that will mean that the scroll bar will move when loading elements
            // with different widths.
            .auto_shrink([false, true])
            .show_viewport(ui, |ui, viewport| {
                ui.set_height(page_size.y);
                let layout = egui::Layout::left_to_right(egui::Align::BOTTOM).with_main_wrap(true);

                let max_width = options.max_width(ui);
                ui.allocate_ui_with_layout(egui::vec2(max_width, 0.0), layout, |ui| {
                    // rumd patch: see the non-scrollable path.
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let scroll_cache = scroll_cache(cache, &source_id);

                    // finding the first element that's not in the viewport anymore
                    let (first_event_index, _, first_end_position) = scroll_cache
                        .split_points
                        .iter()
                        .filter(|(_, _, end_position)| end_position.y < viewport.min.y)
                        .nth_back(1)
                        .copied()
                        .unwrap_or((0, Pos2::ZERO, Pos2::ZERO));

                    // finding the last element that's just outside the viewport
                    let last_event_index = scroll_cache
                        .split_points
                        .iter()
                        .filter(|(_, start_position, _)| start_position.y > viewport.max.y)
                        .nth(1)
                        .map(|(index, _, _)| *index)
                        .unwrap_or(num_rows);

                    ui.allocate_space(first_end_position.to_vec2());

                    // only rendering the elements that are inside the viewport
                    let mut events = events
                        .into_iter()
                        .enumerate()
                        .skip(first_event_index)
                        .take(last_event_index - first_event_index)
                        .peekable();

                    while let Some((i, (e, src_span))) = events.next() {
                        if events.peek().is_none() {
                            self.line.should_end_newline_forced = false;
                        }

                        self.process_event(ui, &mut events, e, src_span, cache, options, max_width);

                        if i == 0 {
                            self.line.should_not_start_newline_forced = false;
                        }
                    }
                });
            });

        // Forcing full re-render to repopulate split points for the new size
        let scroll_cache = scroll_cache(cache, &source_id);
        if available_size != scroll_cache.available_size {
            scroll_cache.available_size = available_size;
            scroll_cache.page_size = None;
            scroll_cache.split_points.clear();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process_event<'e>(
        &mut self,
        ui: &mut Ui,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        event: pulldown_cmark::Event,
        src_span: Range<usize>,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        self.event(ui, event, src_span, cache, options, max_width);

        self.def_list_def_wrapping(events, max_width, cache, options, ui);
        self.item_list_wrapping(events, max_width, cache, options, ui);
        self.table(events, cache, options, ui, max_width);
        self.blockquote(events, max_width, cache, options, ui);
    }

    fn def_list_def_wrapping<'e>(
        &mut self,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        max_width: f32,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
    ) {
        if self.def_list.is_def_list_def {
            self.def_list.is_def_list_def = false;

            let item_events = delayed_events(events, |tag| {
                matches!(tag, pulldown_cmark::TagEnd::DefinitionListDefinition)
            });

            let mut events_iter = item_events.into_iter().enumerate().peekable();

            self.line.try_insert_start(ui);

            // Proccess a single event separately so that we do not insert spaces where we do not
            // want them
            self.line.should_start_newline = false;
            if let Some((_, (e, src_span))) = events_iter.next() {
                self.process_event(ui, &mut events_iter, e, src_span, cache, options, max_width);
            }

            ui.label(" ".repeat(options.indentation_spaces));
            self.line.should_start_newline = true;
            self.line.should_end_newline = false;
            // Required to ensure that the content is aligned with the identation
            ui.horizontal_wrapped(|ui| {
                while let Some((_, (e, src_span))) = events_iter.next() {
                    self.process_event(
                        ui,
                        &mut events_iter,
                        e,
                        src_span,
                        cache,
                        options,
                        max_width,
                    );
                }
            });
            self.line.should_end_newline = true;

            // Only end the definition items line if it is not the last element in the list
            if !matches!(
                events.peek(),
                Some((
                    _,
                    (
                        pulldown_cmark::Event::End(pulldown_cmark::TagEnd::DefinitionList),
                        _
                    )
                ))
            ) {
                self.line.try_insert_end(ui);
            }
        }
    }

    fn item_list_wrapping<'e>(
        &mut self,
        events: &mut impl Iterator<Item = EventIteratorItem<'e>>,
        max_width: f32,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
    ) {
        if self.is_list_item {
            self.is_list_item = false;

            let item_events = delayed_events_list_item(events);
            let mut events_iter = item_events.into_iter().enumerate().peekable();

            // Required to ensure that the content of the list item is aligned with
            // the * or - when wrapping
            ui.horizontal_wrapped(|ui| {
                while let Some((_, (e, src_span))) = events_iter.next() {
                    self.process_event(
                        ui,
                        &mut events_iter,
                        e,
                        src_span,
                        cache,
                        options,
                        max_width,
                    );
                }
            });
        }
    }

    fn blockquote<'e>(
        &mut self,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        max_width: f32,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
    ) {
        if self.is_blockquote {
            let mut collected_events = delayed_events(events, |tag| {
                matches!(tag, pulldown_cmark::TagEnd::BlockQuote(_))
            });
            self.line.try_insert_start(ui);

            // Currently the blockquotes are made in such a way that they need a newline at the end
            // and the start so when this is the first element in the markdown the newline must be
            // manually enabled
            self.line.should_not_start_newline_forced = false;
            if let Some(alert) = parse_alerts(&options.alerts, &mut collected_events) {
                egui_commonmark_backend::alert_ui(alert, ui, |ui| {
                    for (event, src_span) in collected_events {
                        self.event(ui, event, src_span, cache, options, max_width);
                    }
                })
            } else {
                blockquote(ui, ui.visuals().weak_text_color(), |ui| {
                    self.text_style.quote = true;
                    for (event, src_span) in collected_events {
                        self.event(ui, event, src_span, cache, options, max_width);
                    }
                    self.text_style.quote = false;
                });
            }

            if events.peek().is_none() {
                self.line.should_end_newline_forced = false;
            }

            self.line.try_insert_end(ui);
            self.is_blockquote = false;
        }
    }

    fn table<'e>(
        &mut self,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
        max_width: f32,
    ) {
        if self.is_table {
            self.line.try_insert_start(ui);

            // rumd patch: tables are block-level. Blocks inside lists
            // suppress newline insertion, so a table after inline item
            // text would flow beside it (egui frames do not participate
            // in layout wrapping); break the line like every other block
            // element does. The guard skips the break when nothing has
            // been rendered yet (cursor already at a line start).
            if !self.line.should_not_start_newline_forced {
                newline(ui);
            }

            let id = ui.id().with("_table").with(self.curr_table);
            self.curr_table += 1;

            // rumd patch: GitHub-style table chrome. The header band is
            // painted behind the text (shape slot allocated before the
            // grid, filled after — same trick as rumd's search
            // highlights), and each row ends in a hairline. No zebra
            // stripes; cells get breathing room via grid spacing and
            // per-cell padding.
            let header_band_slot = ui.painter().add(egui::Shape::Noop);
            let mut header_cells: Vec<egui::Rect> = Vec::new();
            let mut row_bottoms: Vec<f32> = Vec::new();
            let mut row_cell_rects: Vec<egui::Rect> = Vec::new();
            let border = egui_commonmark_backend::typography::palette(ui).border;

            egui::Frame::new()
                .stroke(egui::Stroke::new(1.0, border))
                .corner_radius(6.0)
                .show(ui, |ui| {
                // rumd patch: force the frame to the container's width: a
                // table must fill the line it now owns. `available_width`
                // already accounts for indents (lists, blockquotes); the
                // `max_width` bound guards against pathological cases.
                // Cell Uis additionally get an explicit wrap style: they
                // are laid out in non-wrapping horizontal Uis, which would
                // otherwise resolve text to TextWrapMode::Extend and ignore
                // the widths above.
                let stroke = 1.0;
                let inner_width = ui
                    .available_width()
                    .min(max_width - 2.0 * stroke);
                ui.set_min_width(inner_width);

                let Table { header, rows } = parse_table(events);

                let column_count = header.len().max(1);
                // rumd patch: the grid's horizontal spacing is part of
                // the table's width budget; subtract it from the equal
                // column share so the whole grid fits the column.
                let grid_spacing = 12.0;
                let column_width = ((inner_width - grid_spacing * (column_count - 1) as f32)
                    / column_count as f32)
                    .max(ui.spacing().interact_size.x);

                egui::Grid::new(id)
                    .spacing(egui::vec2(grid_spacing, 0.0))
                    .num_columns(column_count)
                    .show(ui, |ui| {
                        self.text_style.table_header = true;
                        for (col_index, col) in header.into_iter().enumerate() {
                            ui.vertical(|ui| {
                                rumd_cell_layout(ui, col_index, column_count, column_width);
                                ui.add_space(5.0);
                                for (e, src_span) in col {
                                    let tmp_start = std::mem::replace(
                                        &mut self.line.should_start_newline,
                                        false,
                                    );
                                    let tmp_end = std::mem::replace(
                                        &mut self.line.should_end_newline,
                                        false,
                                    );
                                    self.event(ui, e, src_span, cache, options, max_width);
                                    self.line.should_start_newline = tmp_start;
                                    self.line.should_end_newline = tmp_end;
                                }
                                ui.add_space(5.0);
                                header_cells.push(ui.min_rect());
                            });
                        }
                        self.text_style.table_header = false;

                        ui.end_row();

                        for row in rows {
                            row_cell_rects.clear();
                            for (col_index, col) in row.into_iter().enumerate() {
                                ui.vertical(|ui| {
                                    rumd_cell_layout(
                                        ui,
                                        col_index,
                                        column_count,
                                        column_width,
                                    );
                                    ui.add_space(5.0);
                                    for (e, src_span) in col {
                                        let tmp_start = std::mem::replace(
                                            &mut self.line.should_start_newline,
                                            false,
                                        );
                                        let tmp_end = std::mem::replace(
                                            &mut self.line.should_end_newline,
                                            false,
                                        );
                                        self.event(ui, e, src_span, cache, options, max_width);
                                        self.line.should_start_newline = tmp_start;
                                        self.line.should_end_newline = tmp_end;
                                    }
                                    ui.add_space(6.0);
                                    row_cell_rects.push(ui.min_rect());
                                });
                            }

                            ui.end_row();
                            row_bottoms.push(
                                row_cell_rects
                                    .iter()
                                    .map(|rect| rect.bottom())
                                    .fold(f32::MIN, f32::max),
                            );
                        }
                    });

                ui.expand_to_include_x(ui.max_rect().right());
            });

            // Header band behind the header text: fill the union of the
            // header cells' rects (bounded by the grid's full width).
            if let (Some(first), Some(last)) = (header_cells.first(), header_cells.last()) {
                let left = ui.min_rect().left().min(first.left());
                let right = ui.min_rect().right().max(last.right());
                let band = egui::Rect::from_min_max(
                    egui::pos2(left, first.top()),
                    egui::pos2(right, last.bottom()),
                );
                ui.painter().set(
                    header_band_slot,
                    egui::Shape::rect_filled(
                        band,
                        0.0,
                        egui_commonmark_backend::typography::palette(ui).code_bg,
                    ),
                );
            }
            // Hairline between each pair of rows (not after the last).
            for (i, bottom) in row_bottoms.iter().enumerate() {
                if i + 1 == row_bottoms.len() {
                    break;
                }
                let rect = ui.min_rect();
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.left() + 6.0, bottom + 3.0),
                        egui::pos2(rect.right() - 6.0, bottom + 3.0),
                    ],
                    egui::Stroke::new(1.0, border),
                );
            }

            self.is_table = false;
            if events.peek().is_none() {
                self.line.should_end_newline_forced = false;
            }

            self.line.try_insert_end(ui);
        }
    }

    fn event(
        &mut self,
        ui: &mut Ui,
        event: pulldown_cmark::Event,
        src_span: Range<usize>,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        match event {
            pulldown_cmark::Event::Start(tag) => self.start_tag(ui, tag, cache, options),
            pulldown_cmark::Event::End(tag) => self.end_tag(ui, tag, cache, options, max_width),
            pulldown_cmark::Event::Text(text) => {
                self.event_text(text, ui);
            }
            pulldown_cmark::Event::Code(text) => {
                self.text_style.code = true;
                self.event_text(text, ui);
                self.text_style.code = false;
            }
            pulldown_cmark::Event::InlineHtml(text) => {
                self.event_text(text, ui);
            }

            pulldown_cmark::Event::Html(text) => {
                if options.html_fn.is_some() {
                    self.html_block.push_str(&text);
                } else {
                    self.event_text(text, ui);
                }
            }
            pulldown_cmark::Event::FootnoteReference(footnote) => {
                footnote_start(ui, &footnote);
            }
            pulldown_cmark::Event::SoftBreak => {
                soft_break(ui);
            }
            pulldown_cmark::Event::HardBreak => {
                // rumd patch: a hard break continues the paragraph, so it
                // gets a full body line, not a block gap.
                newline_gap(
                    ui,
                    egui_commonmark_backend::typography::body_line_height(ui),
                );
            }
            pulldown_cmark::Event::Rule => {
                self.line.try_insert_start(ui);
                rule(ui, self.line.can_insert_end());
            }
            pulldown_cmark::Event::TaskListMarker(mut checkbox) => {
                if options.mutable {
                    if ui
                        .add(egui::Checkbox::without_text(&mut checkbox))
                        .clicked()
                    {
                        self.checkbox_events.push(CheckboxClickEvent {
                            checked: checkbox,
                            span: src_span,
                        });
                    }
                } else {
                    ui.add(ImmutableCheckbox::without_text(&mut checkbox));
                }
            }
            pulldown_cmark::Event::InlineMath(tex) => {
                if let Some(math_fn) = options.math_fn {
                    math_fn(ui, &tex, true);
                }
            }
            pulldown_cmark::Event::DisplayMath(tex) => {
                if let Some(math_fn) = options.math_fn {
                    math_fn(ui, &tex, false);
                }
            }
        }
    }

    fn event_text(&mut self, text: CowStr, ui: &mut Ui) {
        let rich_text = self.text_style.to_richtext(ui, &text);
        if let Some(image) = &mut self.image {
            image.alt_text.push(rich_text);
        } else if let Some(block) = &mut self.code_block {
            block.content.push_str(&text);
        } else if let Some(link) = &mut self.link {
            link.text.push(rich_text);
        } else {
            // rumd patch: explicit line height (Typora-like leading) —
            // egui's font-metric rows are far too tight for prose.
            let line_height = self.text_style.line_height(ui);
            egui_commonmark_backend::typography::text_label(ui, rich_text, line_height);
        }
    }

    fn start_tag(
        &mut self,
        ui: &mut Ui,
        tag: pulldown_cmark::Tag,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
    ) {
        match tag {
            pulldown_cmark::Tag::Paragraph => {
                self.line.try_insert_start(ui);
            }
            pulldown_cmark::Tag::Heading { level, id, .. } => {
                if let Some(scroll_target) = cache.scroll_to_id_target()
                    && let Some(id) = id
                    && id.into_string() == scroll_target
                {
                    ui.scroll_to_cursor(Some(egui::Align::TOP));
                    cache.scroll_to_id_target_mut().take();
                }

                // Headings should always insert a newline even if it is at the start.
                // Whether this is okay in all scenarios is a different question.
                // rumd patch: the break is a small gap row like every
                // other block separator.
                newline_gap(
                    ui,
                    egui_commonmark_backend::typography::gap_block_start(ui),
                );
                let level_index = match level {
                    HeadingLevel::H1 => 0,
                    HeadingLevel::H2 => 1,
                    HeadingLevel::H3 => 2,
                    HeadingLevel::H4 => 3,
                    HeadingLevel::H5 => 4,
                    HeadingLevel::H6 => 5,
                };
                // rumd patch: Typora's GitHub theme underlines h1 and h2.
                if level_index < 2 {
                    self.pending_heading_border = Some(level_index);
                }
                self.text_style.heading = Some(level_index);
            }

            // deliberately not using the built in alerts from pulldown-cmark as
            // the markdown itself cannot be localized :( e.g: [!TIP]
            pulldown_cmark::Tag::BlockQuote(_) => {
                self.is_blockquote = true;
            }
            pulldown_cmark::Tag::CodeBlock(c) => {
                match c {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                        self.code_block = Some(crate::CodeBlock {
                            lang: Some(lang.to_string()),
                            content: "".to_string(),
                        });
                    }
                    pulldown_cmark::CodeBlockKind::Indented => {
                        self.code_block = Some(crate::CodeBlock {
                            lang: None,
                            content: "".to_string(),
                        });
                    }
                }
                self.line.try_insert_start(ui);
            }

            pulldown_cmark::Tag::List(point) => {
                if !self.list.is_inside_a_list() && self.line.can_insert_start() {
                    newline(ui);
                }

                if let Some(number) = point {
                    self.list.start_level_with_number(number);
                } else {
                    self.list.start_level_without_number();
                }
                self.line.should_start_newline = false;
                self.line.should_end_newline = false;
            }

            pulldown_cmark::Tag::Item => {
                self.is_list_item = true;
                self.list.start_item(ui, options);
            }

            pulldown_cmark::Tag::FootnoteDefinition(note) => {
                self.line.try_insert_start(ui);

                self.line.should_start_newline = false;
                self.line.should_end_newline = false;
                footnote(ui, &note);
            }
            pulldown_cmark::Tag::Table(_) => {
                self.is_table = true;
            }
            pulldown_cmark::Tag::TableHead => {}
            pulldown_cmark::Tag::TableRow => {}
            pulldown_cmark::Tag::TableCell => {}
            pulldown_cmark::Tag::Emphasis => {
                self.text_style.emphasis = true;
            }
            pulldown_cmark::Tag::Strong => {
                self.text_style.strong = true;
            }
            pulldown_cmark::Tag::Strikethrough => {
                self.text_style.strikethrough = true;
            }
            pulldown_cmark::Tag::Link { dest_url, .. } => {
                self.link = Some(crate::Link {
                    destination: dest_url.to_string(),
                    text: Vec::new(),
                });
            }
            pulldown_cmark::Tag::Image { dest_url, .. } => {
                self.image = Some(crate::Image::new(&dest_url, options));
            }
            pulldown_cmark::Tag::HtmlBlock => {
                self.line.try_insert_start(ui);
            }
            pulldown_cmark::Tag::MetadataBlock(_) => {}

            pulldown_cmark::Tag::DefinitionList => {
                self.line.try_insert_start(ui);
                self.def_list.is_first_item = true;
            }
            pulldown_cmark::Tag::DefinitionListTitle => {
                // we disable newline as the first title should not insert a newline
                // as we have already done that upon the DefinitionList Tag
                if !self.def_list.is_first_item {
                    self.line.try_insert_start(ui)
                } else {
                    self.def_list.is_first_item = false;
                }
            }
            pulldown_cmark::Tag::DefinitionListDefinition => {
                self.def_list.is_def_list_def = true;
            }
            // Not yet supported
            pulldown_cmark::Tag::Superscript | pulldown_cmark::Tag::Subscript => {}
        }
    }

    fn end_tag(
        &mut self,
        ui: &mut Ui,
        tag: pulldown_cmark::TagEnd,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        match tag {
            pulldown_cmark::TagEnd::Paragraph => {
                self.line.try_insert_end(ui);
            }
            pulldown_cmark::TagEnd::Heading { .. } => {
                // rumd patch: Typora's GitHub theme underlines h1/h2 with
                // a hairline across the reading column.
                if let Some(_level) = self.pending_heading_border.take() {
                    let bottom = ui.next_widget_position().y;
                    let rect = ui.max_rect();
                    ui.painter().line_segment(
                        [
                            egui::pos2(rect.left(), bottom + 3.0),
                            egui::pos2(rect.right(), bottom + 3.0),
                        ],
                        egui::Stroke::new(
                            1.0,
                            egui_commonmark_backend::typography::palette(ui).border,
                        ),
                    );
                }
                self.line.try_insert_end(ui);
                self.text_style.heading = None;
            }
            pulldown_cmark::TagEnd::BlockQuote(_) => {}
            pulldown_cmark::TagEnd::CodeBlock => {
                self.end_code_block(ui, cache, options, max_width);
            }

            pulldown_cmark::TagEnd::List(_) => {
                if self.list.is_last_level() {
                    self.line.should_start_newline = true;
                    self.line.should_end_newline = true;
                }

                self.list.end_level(ui, self.line.can_insert_end());

                if !self.list.is_inside_a_list() {
                    // Reset all the state and make it ready for the next list that occurs
                    self.list = List::default();
                }
            }
            pulldown_cmark::TagEnd::Item => {}
            pulldown_cmark::TagEnd::FootnoteDefinition => {
                self.line.should_start_newline = true;
                self.line.should_end_newline = true;
                self.line.try_insert_end(ui);
            }
            pulldown_cmark::TagEnd::Table => {}
            pulldown_cmark::TagEnd::TableHead => {}
            pulldown_cmark::TagEnd::TableRow => {}
            pulldown_cmark::TagEnd::TableCell => {
                // rumd patch: no manual spacing between cells. The grid's
                // column spacing already separates them, and this label
                // would render as a stray line inside vertical cell Uis
                // (parse_row places the End marker at the start of the
                // next cell's events), misaligning every row.
            }
            pulldown_cmark::TagEnd::Emphasis => {
                self.text_style.emphasis = false;
            }
            pulldown_cmark::TagEnd::Strong => {
                self.text_style.strong = false;
            }
            pulldown_cmark::TagEnd::Strikethrough => {
                self.text_style.strikethrough = false;
            }
            pulldown_cmark::TagEnd::Link => {
                if let Some(link) = self.link.take() {
                    link.end(ui, cache, options, &mut self.deferred_scroll_to_heading);
                }
            }
            pulldown_cmark::TagEnd::Image => {
                if let Some(image) = self.image.take() {
                    image.end(ui, options);
                }
            }
            pulldown_cmark::TagEnd::HtmlBlock => {
                if let Some(html_fn) = options.html_fn {
                    html_fn(ui, &self.html_block);
                    self.html_block.clear();
                }
            }

            pulldown_cmark::TagEnd::MetadataBlock(_) => {}

            pulldown_cmark::TagEnd::DefinitionList => self.line.try_insert_end(ui),
            pulldown_cmark::TagEnd::DefinitionListTitle
            | pulldown_cmark::TagEnd::DefinitionListDefinition => {}
            pulldown_cmark::TagEnd::Superscript | pulldown_cmark::TagEnd::Subscript => {}
        }
    }

    fn end_code_block(
        &mut self,
        ui: &mut Ui,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        if let Some(block) = self.code_block.take() {
            block.end(ui, cache, options, max_width);
            self.line.try_insert_end(ui);
        }
    }
}
