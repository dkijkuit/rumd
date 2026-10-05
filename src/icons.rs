//! Hand-painted vector toolbar icons.
//!
//! Icons are drawn with epaint shapes instead of an SVG/font dependency so
//! they stay crisp at any zoom factor and follow the theme stroke colors.

use eframe::egui;
use eframe::epaint::{PathShape, Pos2, Shape, Stroke, vec2};

/// The icons the toolbar needs. All are drawn into a square `size` box
/// centered on `center`, in `color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Folder,
    Refresh,
    Save,
    Sun,
    Moon,
    Search,
    Toc,
}

impl Icon {
    pub fn paint(&self, painter: &egui::Painter, center: Pos2, size: f32, color: egui::Color32) {
        let h = size / 2.0;
        match self {
            Icon::Folder => paint_folder(painter, center, h, color),
            Icon::Refresh => paint_refresh(painter, center, h, color),
            Icon::Save => paint_save(painter, center, h, color),
            Icon::Sun => paint_sun(painter, center, h, color),
            Icon::Moon => paint_moon(painter, center, h, color),
            Icon::Search => paint_search(painter, center, h, color),
            Icon::Toc => paint_toc(painter, center, h, color),
        }
    }
}

fn stroke_for(h: f32) -> f32 {
    (h / 8.0).max(1.2)
}

/// Folder outline: tab step across the top, closed down the sides.
fn paint_folder(painter: &egui::Painter, c: Pos2, h: f32, color: egui::Color32) {
    let stroke = Stroke::new(stroke_for(h), color);
    let x_left = c.x - h;
    let x_right = c.x + h;
    let x_step = c.x - h * 0.15;
    let y_tab = c.y - h * 0.58;
    let y_body = c.y - h * 0.22;
    let y_bottom = c.y + h * 0.72;
    let r = h * 0.18;
    let pts = vec![
        Pos2::new(x_left + r, y_tab),
        Pos2::new(x_step - r, y_tab),
        // rounded inner step
        Pos2::new(x_step, y_tab + r * 0.7),
        Pos2::new(x_step, y_body + r * 0.3),
        Pos2::new(x_step + r, y_body),
        Pos2::new(x_right - r, y_body),
        Pos2::new(x_right, y_body + r),
        Pos2::new(x_right, y_bottom - r),
        Pos2::new(x_right - r, y_bottom),
        Pos2::new(x_left + r, y_bottom),
        Pos2::new(x_left, y_bottom - r),
        Pos2::new(x_left, y_tab + r),
    ];
    painter.add(Shape::Path(PathShape::closed_line(pts, stroke)));
}

/// Circular arrow: a nearly full stroked arc plus a triangular arrowhead.
fn paint_refresh(painter: &egui::Painter, c: Pos2, h: f32, color: egui::Color32) {
    let radius = h * 0.72;
    let stroke = Stroke::new(stroke_for(h), color);
    // Start at 40° (top-right), sweep clockwise all the way around to 20°,
    // leaving a small gap in the top-right quadrant. Screen y is down, so
    // `c.y - r*sin(t)` keeps t in the mathematical (y-up) convention.
    let start = 50f32.to_radians();
    let end = -30f32.to_radians();
    let sweep = end - start + std::f32::consts::TAU;
    let points: Vec<Pos2> = (0..=48)
        .map(|i| {
            let t = start + sweep * (i as f32 / 48.0);
            Pos2::new(c.x + radius * t.cos(), c.y - radius * t.sin())
        })
        .collect();
    painter.add(Shape::Path(PathShape::line(points, stroke)));
    // Arrowhead at the arc end, aligned with the direction of travel so it
    // flows out of the line instead of crossing it.
    let t_end = start + sweep;
    let tip0 = Pos2::new(c.x + radius * t_end.cos(), c.y - radius * t_end.sin());
    // d/dt of the arc parametrization (screen coordinates).
    let dir = vec2(-t_end.sin(), -t_end.cos());
    let perp = vec2(-dir.y, dir.x);
    let head = 0.5 * h;
    let apex = tip0 + dir * head * 1.1;
    let b1 = tip0 + perp * head * 0.42;
    let b2 = tip0 - perp * head * 0.42;
    painter.add(Shape::convex_polygon(
        vec![apex, b1, b2],
        color,
        Stroke::NONE,
    ));
}

/// Save: a downward arrow dropping into an open tray.
fn paint_save(painter: &egui::Painter, c: Pos2, h: f32, color: egui::Color32) {
    let stroke = Stroke::new(stroke_for(h), color);
    // Tray: a U-shaped container in the lower half, open toward the top.
    let y_top = c.y + h * 0.15;
    let y_bottom = c.y + h * 0.75;
    let x_left = c.x - h * 0.7;
    let x_right = c.x + h * 0.7;
    let r = h * 0.15;
    let tray = vec![
        Pos2::new(x_left, y_top + r),
        Pos2::new(x_left, y_bottom - r),
        Pos2::new(x_left + r, y_bottom),
        Pos2::new(x_right - r, y_bottom),
        Pos2::new(x_right, y_bottom - r),
        Pos2::new(x_right, y_top + r),
    ];
    painter.add(Shape::Path(PathShape::line(tray, stroke)));
    // Arrow: shaft down the middle with a triangular head.
    let shaft_top = c.y - h * 0.8;
    let shaft_bottom = c.y + h * 0.1;
    painter.add(Shape::line_segment(
        [Pos2::new(c.x, shaft_top), Pos2::new(c.x, shaft_bottom)],
        stroke,
    ));
    let head = h * 0.32;
    painter.add(Shape::convex_polygon(
        vec![
            Pos2::new(c.x, shaft_bottom + head * 0.9),
            Pos2::new(c.x - head, shaft_bottom),
            Pos2::new(c.x + head, shaft_bottom),
        ],
        color,
        Stroke::NONE,
    ));
}

/// Sun: filled core with eight rays.
fn paint_sun(painter: &egui::Painter, c: Pos2, h: f32, color: egui::Color32) {
    painter.add(Shape::circle_filled(c, h * 0.55, color));
    let stroke = Stroke::new(stroke_for(h), color);
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        let dir = vec2(a.cos(), a.sin());
        let inner = c + dir * (h * 0.78);
        let outer = c + dir * (h * 1.05);
        painter.add(Shape::line_segment([inner, outer], stroke));
    }
}

/// Crescent moon: a filled ribbon between the outer arc of one circle and
/// the inner arc of an offset circle. Filled paths in epaint must stay
/// convex, so the ribbon is tessellated into a triangle strip by hand.
fn paint_moon(painter: &egui::Painter, c: Pos2, h: f32, color: egui::Color32) {
    let radius = h * 0.85;
    // Offset circle shifted toward the upper-left carves the crescent.
    let inner_center = c + vec2(-h * 0.5, -h * 0.28);
    let inner_radius = h * 0.78;
    // Standard two-circle intersection.
    let d_vec = inner_center - c;
    let d = d_vec.length();
    if d <= 1e-4 {
        return;
    }
    let d_norm = d_vec / d;
    let a = (d * d + radius * radius - inner_radius * inner_radius) / (2.0 * d);
    let half_sq = radius * radius - a * a;
    if half_sq <= 0.0 {
        return;
    }
    let half = half_sq.sqrt();
    let base = c + d_norm * a;
    let perp = vec2(-d_norm.y, d_norm.x);
    let p1 = base + perp * half;
    let p2 = base - perp * half;
    // Outer arc through the far side of the big circle (away from the bite).
    let outer = arc_between(c, radius, p1, p2, c - d_norm * radius);
    // Inner arc through the carving-circle edge nearest the far side.
    let inner = arc_between(
        inner_center,
        inner_radius,
        p1,
        p2,
        inner_center - d_norm * inner_radius,
    );
    let n = outer.len();
    let mut mesh = egui::Mesh::default();
    for p in outer.iter().chain(inner.iter()) {
        mesh.colored_vertex(*p, color);
    }
    for i in 0..n - 1 {
        let o0 = i as u32;
        let o1 = (i + 1) as u32;
        let i0 = (n + i) as u32;
        let i1 = (n + i + 1) as u32;
        mesh.add_triangle(o0, o1, i0);
        mesh.add_triangle(o1, i1, i0);
    }
    painter.add(Shape::mesh(mesh));
}

/// Sample the arc of `center`/`radius` from `from` to `to`, choosing the
/// sweep direction whose midpoint passes nearest `via`.
fn arc_between(center: Pos2, radius: f32, from: Pos2, to: Pos2, via: Pos2) -> Vec<Pos2> {
    let a_from = (from - center).angle();
    let a_to = (to - center).angle();
    let direct = a_to - a_from;
    let candidates = [
        direct,
        direct + std::f32::consts::TAU,
        direct - std::f32::consts::TAU,
    ];
    let mid_dist = |sweep: f32| -> f32 {
        let t = a_from + sweep / 2.0;
        let mid = Pos2::new(center.x + radius * t.cos(), center.y + radius * t.sin());
        (mid - via).length_sq()
    };
    let sweep = candidates
        .into_iter()
        .min_by(|s1, s2| mid_dist(*s1).total_cmp(&mid_dist(*s2)))
        .unwrap_or(direct);
    arc_points(center, radius, a_from, a_from + sweep, 40)
}

fn arc_points(center: Pos2, radius: f32, from: f32, to: f32, steps: usize) -> Vec<Pos2> {
    (0..=steps)
        .map(|i| {
            let t = from + (to - from) * (i as f32 / steps as f32);
            Pos2::new(center.x + radius * t.cos(), center.y + radius * t.sin())
        })
        .collect()
}

/// Magnifier: stroked lens with a round-capped handle.
fn paint_search(painter: &egui::Painter, c: Pos2, h: f32, color: egui::Color32) {
    let stroke = Stroke::new(stroke_for(h), color);
    let lens_center = c + vec2(-h * 0.18, -h * 0.18);
    let lens_radius = h * 0.6;
    painter.circle(lens_center, lens_radius, egui::Color32::TRANSPARENT, stroke);
    let dir = vec2(
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
    );
    let from = lens_center + dir * lens_radius;
    let to = c + dir * h * 0.95;
    painter.add(Shape::line_segment([from, to], stroke));
}

/// Table of contents: heading list — indented bars with leading dots.
fn paint_toc(painter: &egui::Painter, c: Pos2, h: f32, color: egui::Color32) {
    let stroke = Stroke::new(stroke_for(h), color);
    let ys = [-0.55, 0.0, 0.55];
    for (row, &dy) in ys.iter().enumerate() {
        let y = c.y + h * dy;
        // Leading dot, one indent step further in on every row.
        let dot_x = c.x - h * 0.75 + row as f32 * h * 0.42;
        painter.add(Shape::circle_filled(Pos2::new(dot_x, y), h * 0.14, color));
        // Text bar after the dot; the last row is shorter (a deeper title).
        let from = Pos2::new(dot_x + h * 0.38, y);
        let to = Pos2::new(c.x + h * (if row == 2 { 0.35 } else { 0.8 }), y);
        painter.add(Shape::line_segment([from, to], stroke));
    }
}

/// A compact square button that shows only an icon.
///
/// The accessible label (`label`) doubles as the kittest query handle.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str, enabled: bool) -> egui::Response {
    const ICON_SIZE: f32 = 17.0;
    let side = ICON_SIZE + 12.0;
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(side, side), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
        });
        let visuals = ui.style().interact(&response);
        if response.enabled() {
            let bg = if response.hovered() || response.is_pointer_button_down_on() {
                visuals.weak_bg_fill
            } else {
                egui::Color32::TRANSPARENT
            };
            if bg != egui::Color32::TRANSPARENT {
                ui.painter()
                    .rect_filled(rect.expand(2.0), visuals.corner_radius, bg);
            }
            let fg = visuals.fg_stroke.color;
            icon.paint(
                &ui.painter().with_clip_rect(rect),
                rect.center(),
                ICON_SIZE,
                fg,
            );
        } else {
            icon.paint(
                &ui.painter().with_clip_rect(rect),
                rect.center(),
                ICON_SIZE,
                ui.visuals().weak_text_color(),
            );
        }
        response
    })
    .inner
}
