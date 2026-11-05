use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};

use rusio_plugin_api::theme::Theme;

pub const TITLEBAR_HEIGHT: f32 = 38.0;

/// Draws the RusIO mark: a white "R" whose leg is a cyan terminal-cursor accent, on an
/// indigo tile. Same geometry as assets/icon.ico.
pub fn paint_logo(ui: &mut egui::Ui, size: f32) {
    let theme = Theme::default();
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let p = ui.painter();
    let at = |x: f32, y: f32| Pos2::new(rect.min.x + size * x, rect.min.y + size * y);
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));
    let accent = Color32::from_rgb(34, 211, 238);

    p.rect_filled(
        rect,
        CornerRadius::same((size * 0.225).round() as u8),
        theme.accent_primary,
    );
    // stem
    p.rect_filled(r(0.27, 0.20, 0.40, 0.76), CornerRadius::ZERO, Color32::WHITE);
    // bowl: square on the left, round on the right
    p.rect_filled(
        r(0.27, 0.20, 0.71, 0.56),
        CornerRadius {
            nw: 0,
            sw: 0,
            ne: (size * 0.17).round() as u8,
            se: (size * 0.17).round() as u8,
        },
        Color32::WHITE,
    );
    // counter
    p.rect_filled(
        r(0.40, 0.32, 0.55, 0.44),
        CornerRadius::same((size * 0.05).round() as u8),
        theme.accent_primary,
    );
    // leg
    p.add(egui::Shape::convex_polygon(
        vec![at(0.43, 0.52), at(0.58, 0.52), at(0.75, 0.76), at(0.60, 0.76)],
        accent,
        Stroke::NONE,
    ));
}

pub enum WindowButton {
    Minimize,
    Maximize,
    Close,
}

fn window_button(ui: &mut egui::Ui, kind: WindowButton, maximized: bool) -> egui::Response {
    let theme = Theme::default();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(44.0, TITLEBAR_HEIGHT), Sense::click());

    if resp.hovered() {
        let fill = match kind {
            WindowButton::Close => Color32::from_rgb(220, 53, 69),
            _ => theme.bg_active,
        };
        ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    }

    let color = if resp.hovered() {
        Color32::WHITE
    } else {
        theme.text_secondary
    };
    let stroke = Stroke::new(1.2_f32, color);
    let c = rect.center();
    let p = ui.painter();
    match kind {
        WindowButton::Minimize => {
            p.line_segment([c + Vec2::new(-5.0, 0.0), c + Vec2::new(5.0, 0.0)], stroke);
        }
        WindowButton::Maximize => {
            if maximized {
                p.rect_stroke(
                    Rect::from_center_size(c + Vec2::new(-1.0, 1.0), Vec2::splat(7.0)),
                    CornerRadius::ZERO,
                    stroke,
                    egui::StrokeKind::Middle,
                );
                p.line_segment([c + Vec2::new(-2.0, -5.0), c + Vec2::new(5.0, -5.0)], stroke);
                p.line_segment([c + Vec2::new(5.0, -5.0), c + Vec2::new(5.0, 2.0)], stroke);
            } else {
                p.rect_stroke(
                    Rect::from_center_size(c, Vec2::splat(9.0)),
                    CornerRadius::ZERO,
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
        }
        WindowButton::Close => {
            p.line_segment([c + Vec2::new(-5.0, -5.0), c + Vec2::new(5.0, 5.0)], stroke);
            p.line_segment([c + Vec2::new(-5.0, 5.0), c + Vec2::new(5.0, -5.0)], stroke);
        }
    }
    resp
}

/// Window controls (right side). Call inside a right_to_left layout.
pub fn window_controls(ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    ui.spacing_mut().item_spacing = Vec2::ZERO;

    if window_button(ui, WindowButton::Close, maximized).clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    if window_button(ui, WindowButton::Maximize, maximized).clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    if window_button(ui, WindowButton::Minimize, maximized).clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
}

/// Makes the whole bar draggable and double-click toggles maximize.
/// Must be called with the bar's full rect, BEFORE adding widgets on top.
pub fn drag_area(ui: &mut egui::Ui, rect: Rect) {
    let resp = ui.interact(rect, ui.id().with("titlebar_drag"), Sense::click_and_drag());
    let ctx = ui.ctx();
    if resp.double_clicked() {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if resp.drag_started() {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

/// The window has no native frame, so its edges are handled here: a cursor hint and a native
/// resize when the press starts within a few pixels of the border.
pub fn resize_edges(ctx: &egui::Context) {
    use egui::{CursorIcon as C, ResizeDirection as D};
    const EDGE: f32 = 6.0;
    const CORNER: f32 = 12.0;
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let r = ctx.content_rect();
    let left = pos.x - r.left() < EDGE;
    let right = r.right() - pos.x < EDGE;
    let top = pos.y - r.top() < EDGE;
    let bottom = r.bottom() - pos.y < EDGE;
    let near_x = pos.x - r.left() < CORNER || r.right() - pos.x < CORNER;
    let near_y = pos.y - r.top() < CORNER || r.bottom() - pos.y < CORNER;
    let dir = match (left, right, top, bottom) {
        (true, _, _, _) if near_y && pos.y < r.center().y => Some((D::NorthWest, C::ResizeNorthWest)),
        (true, _, _, _) if near_y => Some((D::SouthWest, C::ResizeSouthWest)),
        (_, true, _, _) if near_y && pos.y < r.center().y => Some((D::NorthEast, C::ResizeNorthEast)),
        (_, true, _, _) if near_y => Some((D::SouthEast, C::ResizeSouthEast)),
        (_, _, true, _) if near_x && pos.x < r.center().x => Some((D::NorthWest, C::ResizeNorthWest)),
        (_, _, true, _) if near_x => Some((D::NorthEast, C::ResizeNorthEast)),
        (_, _, _, true) if near_x && pos.x < r.center().x => Some((D::SouthWest, C::ResizeSouthWest)),
        (_, _, _, true) if near_x => Some((D::SouthEast, C::ResizeSouthEast)),
        (true, _, _, _) => Some((D::West, C::ResizeHorizontal)),
        (_, true, _, _) => Some((D::East, C::ResizeHorizontal)),
        (_, _, true, _) => Some((D::North, C::ResizeVertical)),
        (_, _, _, true) => Some((D::South, C::ResizeVertical)),
        _ => None,
    };
    if let Some((direction, cursor)) = dir {
        ctx.set_cursor_icon(cursor);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}
