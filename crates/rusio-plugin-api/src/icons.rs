use egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use crate::theme::Theme;

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Icon {
    Close,
    Plus,
    Edit,
    Play,
    ChevronDown,
    ChevronRight,
    ArrowUp,
    Refresh,
    Upload,
    UploadFolder,
    FolderPlus,
    FilePlus,
    Save,
    Gear,
    Terminal,
    Monitor,
    Database,
    Server,
    Cloud,
    Globe,
    Key,
    Shield,
    Format,
}

pub const ICON_BTN: f32 = 22.0;

/// Paints an icon centered in `rect` using only painter primitives, so it never
/// depends on glyph coverage of the loaded fonts.
pub fn paint_icon(ui: &egui::Ui, rect: Rect, icon: Icon, color: Color32) {
    let p = ui.painter();
    let c = rect.center();
    let st = Stroke::new(1.4_f32, color);
    let v = |x: f32, y: f32| c + Vec2::new(x, y);
    match icon {
        Icon::Close => {
            p.line_segment([v(-3.5, -3.5), v(3.5, 3.5)], st);
            p.line_segment([v(-3.5, 3.5), v(3.5, -3.5)], st);
        }
        Icon::Plus => {
            p.line_segment([v(-4.0, 0.0), v(4.0, 0.0)], st);
            p.line_segment([v(0.0, -4.0), v(0.0, 4.0)], st);
        }
        Icon::Edit => {
            p.line_segment([v(-3.5, 3.5), v(3.5, -3.5)], Stroke::new(2.2_f32, color));
            p.line_segment([v(-4.5, 4.5), v(-2.0, 4.5)], st);
        }
        Icon::Play => {
            p.add(Shape::convex_polygon(
                vec![v(-2.5, -4.0), v(-2.5, 4.0), v(4.0, 0.0)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::ChevronDown => {
            p.line_segment([v(-3.5, -1.5), v(0.0, 2.0)], st);
            p.line_segment([v(0.0, 2.0), v(3.5, -1.5)], st);
        }
        Icon::ChevronRight => {
            p.line_segment([v(-1.5, -3.5), v(2.0, 0.0)], st);
            p.line_segment([v(2.0, 0.0), v(-1.5, 3.5)], st);
        }
        Icon::ArrowUp => {
            p.line_segment([v(0.0, 5.5), v(0.0, -5.5)], st);
            p.line_segment([v(-4.0, -1.5), v(0.0, -5.5)], st);
            p.line_segment([v(4.0, -1.5), v(0.0, -5.5)], st);
        }
        Icon::Refresh => {
            let r = 5.5_f32;
            let (a0, a1) = (0.5_f32, 5.2_f32);
            let n = 16;
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = a0 + (a1 - a0) * i as f32 / n as f32;
                    v(r * a.cos(), r * a.sin())
                })
                .collect();
            p.add(Shape::line(pts.clone(), st));
            let end = *pts.last().unwrap();
            let tangent = Vec2::new(-a1.sin(), a1.cos());
            let normal = Vec2::new(a1.cos(), a1.sin());
            p.add(Shape::convex_polygon(
                vec![end + tangent * 4.0, end + normal * 3.2, end - normal * 3.2],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Upload => {
            p.line_segment([v(0.0, 3.0), v(0.0, -5.5)], st);
            p.line_segment([v(-3.5, -2.0), v(0.0, -5.5)], st);
            p.line_segment([v(3.5, -2.0), v(0.0, -5.5)], st);
            p.line_segment([v(-5.5, 6.0), v(5.5, 6.0)], st);
        }
        Icon::UploadFolder => {
            p.rect_stroke(
                Rect::from_min_max(v(-6.5, -3.5), v(6.5, 6.0)),
                CornerRadius::same(2),
                st,
                egui::StrokeKind::Middle,
            );
            p.line_segment([v(-6.5, -3.5), v(-6.5, -5.5)], st);
            p.line_segment([v(-6.5, -5.5), v(-2.5, -5.5)], st);
            p.line_segment([v(-2.5, -5.5), v(-1.0, -3.5)], st);
            p.line_segment([v(0.0, 4.0), v(0.0, -1.0)], st);
            p.line_segment([v(-2.5, 1.5), v(0.0, -1.0)], st);
            p.line_segment([v(2.5, 1.5), v(0.0, -1.0)], st);
        }
        Icon::FolderPlus => {
            p.rect_stroke(
                Rect::from_min_max(v(-6.5, -3.5), v(6.5, 6.0)),
                CornerRadius::same(2),
                st,
                egui::StrokeKind::Middle,
            );
            p.line_segment([v(-6.5, -3.5), v(-6.5, -5.5)], st);
            p.line_segment([v(-6.5, -5.5), v(-2.5, -5.5)], st);
            p.line_segment([v(-2.5, -5.5), v(-1.0, -3.5)], st);
            p.line_segment([v(-2.5, 1.5), v(2.5, 1.5)], st);
            p.line_segment([v(0.0, -1.0), v(0.0, 4.0)], st);
        }
        Icon::FilePlus => {
            p.rect_stroke(
                Rect::from_min_max(v(-4.5, -6.0), v(4.5, 6.0)),
                CornerRadius::same(2),
                st,
                egui::StrokeKind::Middle,
            );
            p.line_segment([v(-2.0, 0.5), v(2.0, 0.5)], st);
            p.line_segment([v(0.0, -1.5), v(0.0, 2.5)], st);
        }
        Icon::Gear => {
            p.circle_stroke(c, 4.4, st);
            p.circle_stroke(c, 1.8, st);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let dir = Vec2::new(a.cos(), a.sin());
                p.line_segment([c + dir * 5.0, c + dir * 7.2], Stroke::new(2.4_f32, color));
            }
        }
        Icon::Terminal => {
            p.rect_stroke(
                Rect::from_min_max(v(-7.0, -5.5), v(7.0, 5.5)),
                CornerRadius::same(2),
                st,
                egui::StrokeKind::Middle,
            );
            p.line_segment([v(-4.0, -2.0), v(-1.5, 0.0)], st);
            p.line_segment([v(-1.5, 0.0), v(-4.0, 2.0)], st);
            p.line_segment([v(0.5, 2.5), v(4.0, 2.5)], st);
        }
        Icon::Monitor => {
            p.rect_stroke(
                Rect::from_min_max(v(-7.0, -6.0), v(7.0, 3.5)),
                CornerRadius::same(2),
                st,
                egui::StrokeKind::Middle,
            );
            p.line_segment([v(0.0, 3.5), v(0.0, 6.5)], st);
            p.line_segment([v(-3.5, 6.5), v(3.5, 6.5)], st);
        }
        Icon::Server => {
            for y in [-6.0_f32, 0.5] {
                p.rect_stroke(
                    Rect::from_min_max(v(-7.0, y), v(7.0, y + 5.5)),
                    CornerRadius::same(2),
                    st,
                    egui::StrokeKind::Middle,
                );
                p.circle_filled(v(-4.0, y + 2.75), 0.9, color);
            }
        }
        Icon::Database => {
            let (rx, ry) = (6.0_f32, 2.4_f32);
            let arc = |cy: f32, a0: f32, a1: f32| -> Vec<Pos2> {
                (0..=12)
                    .map(|i| {
                        let a = a0 + (a1 - a0) * i as f32 / 12.0;
                        v(rx * a.cos(), cy + ry * a.sin())
                    })
                    .collect()
            };
            let pi = std::f32::consts::PI;
            // top cap (full ellipse), then the lower curves of each band
            p.add(Shape::closed_line(arc(-4.5, 0.0, 2.0 * pi), st));
            p.add(Shape::line(arc(0.0, 0.0, pi), st));
            p.add(Shape::line(arc(4.5, 0.0, pi), st));
            p.line_segment([v(-rx, -4.5), v(-rx, 4.5)], st);
            p.line_segment([v(rx, -4.5), v(rx, 4.5)], st);
        }
        Icon::Cloud => {
            p.line_segment([v(-5.0, 4.0), v(5.5, 4.0)], st);
            p.circle_stroke(v(-4.0, 1.5), 2.5, st);
            p.circle_stroke(v(-0.5, -1.0), 3.5, st);
            p.circle_stroke(v(4.0, 1.5), 2.5, st);
        }
        Icon::Globe => {
            p.circle_stroke(c, 6.5, st);
            p.line_segment([v(-6.5, 0.0), v(6.5, 0.0)], st);
            p.line_segment([v(0.0, -6.5), v(0.0, 6.5)], st);
            p.add(Shape::closed_line(
                (0..=16)
                    .map(|i| {
                        let a = std::f32::consts::TAU * i as f32 / 16.0;
                        v(3.0 * a.cos(), 6.5 * a.sin())
                    })
                    .collect(),
                st,
            ));
        }
        Icon::Key => {
            p.circle_stroke(v(-3.5, 0.0), 3.0, st);
            p.line_segment([v(-0.5, 0.0), v(6.5, 0.0)], st);
            p.line_segment([v(4.0, 0.0), v(4.0, 3.0)], st);
            p.line_segment([v(6.5, 0.0), v(6.5, 2.5)], st);
        }
        Icon::Shield => {
            p.add(Shape::closed_line(
                vec![
                    v(0.0, -6.5),
                    v(5.5, -4.0),
                    v(5.0, 1.5),
                    v(0.0, 6.5),
                    v(-5.0, 1.5),
                    v(-5.5, -4.0),
                ],
                st,
            ));
            p.line_segment([v(-2.0, 0.0), v(-0.5, 2.0)], st);
            p.line_segment([v(-0.5, 2.0), v(2.5, -2.0)], st);
        }
        Icon::Format => {
            for (y, x0, x1) in [
                (-5.0_f32, -6.0, 6.0),
                (-1.7, -3.0, 6.0),
                (1.7, -3.0, 3.0),
                (5.0, -6.0, 6.0),
            ] {
                p.line_segment([v(x0, y), v(x1, y)], st);
            }
        }
        Icon::Save => {
            p.rect_stroke(
                Rect::from_min_max(v(-5.5, -5.5), v(5.5, 5.5)),
                CornerRadius::same(2),
                st,
                egui::StrokeKind::Middle,
            );
            p.rect_stroke(
                Rect::from_min_max(v(-3.0, -5.5), v(3.0, -2.0)),
                CornerRadius::ZERO,
                st,
                egui::StrokeKind::Middle,
            );
            p.rect_stroke(
                Rect::from_min_max(v(-3.0, 1.0), v(3.0, 5.5)),
                CornerRadius::ZERO,
                st,
                egui::StrokeKind::Middle,
            );
        }
    }
}

/// Fixed-size square icon button. Returns true when clicked.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, color: Option<Color32>, tip: &str) -> bool {
    icon_button_sized(ui, icon, color, tip, ICON_BTN)
}

/// Same as `icon_button` with an explicit square size (toolbars use 26).
pub fn icon_button_sized(ui: &mut egui::Ui, icon: Icon, color: Option<Color32>, tip: &str, size: f32) -> bool {
    let theme = Theme::default();
    let enabled = ui.is_enabled();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), theme.bg_active);
    }
    let col = if !enabled {
        theme.text_muted.gamma_multiply(0.5)
    } else {
        color.unwrap_or(if resp.hovered() {
            Color32::WHITE
        } else {
            theme.text_secondary
        })
    };
    paint_icon(ui, rect, icon, col);
    resp.on_hover_text(tip).clicked()
}

/// Small non-interactive chevron used as a folder indicator.
pub fn chevron(ui: &mut egui::Ui, open: bool) {
    let theme = Theme::default();
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    let icon = if open { Icon::ChevronDown } else { Icon::ChevronRight };
    paint_icon(ui, rect, icon, theme.text_muted);
    let _ = Pos2::ZERO;
}

/// Painted status dot (the "●" glyph is missing from the loaded fonts).
pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.0, color);
}

/// Paints a protocol's picture into `rect`: its SVG logo, or the tinted glyph.
pub fn paint_protocol_icon(ui: &egui::Ui, id: &str, icon: crate::ProtocolIcon, color: Color32, rect: Rect) {
    match icon {
        crate::ProtocolIcon::Logo(bytes) => {
            // The `.svg` suffix is how the image loader recognises the format.
            let uri = format!("bytes://logo-{id}.svg");
            egui::Image::from_bytes(uri, bytes)
                .fit_to_exact_size(rect.size())
                .paint_at(ui, rect);
        }
        crate::ProtocolIcon::Glyph(g) => paint_icon(ui, rect, g, color),
    }
}

/// Same, allocating a `size` square in the layout.
pub fn protocol_icon(ui: &mut egui::Ui, id: &str, icon: crate::ProtocolIcon, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_protocol_icon(ui, id, icon, color, rect);
}

/// Icons a user can pick for a connection: `(key stored in connections.json, glyph, label)`.
pub const CHOICES: &[(&str, Icon, &str)] = &[
    ("terminal", Icon::Terminal, "Terminal"),
    ("server", Icon::Server, "Servidor"),
    ("monitor", Icon::Monitor, "Escritorio"),
    ("database", Icon::Database, "Base de datos"),
    ("cloud", Icon::Cloud, "Nube"),
    ("globe", Icon::Globe, "Web"),
    ("key", Icon::Key, "Acceso"),
    ("shield", Icon::Shield, "Seguro"),
];

pub fn choice_by_key(key: &str) -> Option<Icon> {
    CHOICES.iter().find(|(k, _, _)| *k == key).map(|(_, i, _)| *i)
}
