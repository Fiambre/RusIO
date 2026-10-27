use eframe::egui::{self, Color32, Rounding, Stroke, Vec2};

pub struct Theme {
    pub bg_dark: Color32,
    pub bg_panel: Color32,
    pub bg_panel_alt: Color32,
    pub bg_active: Color32,
    pub bg_hover: Color32,
    pub border: Color32,
    pub border_focus: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub accent_primary: Color32,   // Indigo/Cyan
    pub accent_ssh: Color32,       // Emerald
    pub accent_local: Color32,     // Purple/Blue
    pub accent_vnc: Color32,       // Orange/Amber
    pub status_online: Color32,
    pub status_connecting: Color32,
    pub status_offline: Color32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg_dark: Color32::from_rgb(13, 16, 23),       // Deep obsidian
            bg_panel: Color32::from_rgb(20, 24, 34),      // Slate panel
            bg_panel_alt: Color32::from_rgb(26, 31, 44),  // Card background
            bg_active: Color32::from_rgb(37, 44, 62),     // Active item
            bg_hover: Color32::from_rgb(30, 36, 51),      // Hover item
            border: Color32::from_rgb(38, 45, 63),        // Border
            border_focus: Color32::from_rgb(99, 102, 241),// Focus indigo
            text_primary: Color32::from_rgb(243, 244, 246),
            text_secondary: Color32::from_rgb(156, 163, 175),
            text_muted: Color32::from_rgb(107, 114, 128),
            accent_primary: Color32::from_rgb(99, 102, 241), // Indigo #6366f1
            accent_ssh: Color32::from_rgb(16, 185, 129),     // Emerald #10b981
            accent_local: Color32::from_rgb(168, 85, 247),   // Purple #a855f7
            accent_vnc: Color32::from_rgb(249, 115, 22),     // Orange #f97316
            status_online: Color32::from_rgb(34, 197, 94),
            status_connecting: Color32::from_rgb(234, 179, 8),
            status_offline: Color32::from_rgb(239, 68, 68),
        }
    }
}

pub fn apply_theme(ctx: &egui::Context) {
    let theme = Theme::default();
    let mut visuals = egui::Visuals::dark();

    visuals.override_text_color = Some(theme.text_primary);
    visuals.panel_fill = theme.bg_panel;
    visuals.window_fill = theme.bg_panel;
    visuals.faint_bg_color = theme.bg_panel_alt;
    visuals.extreme_bg_color = theme.bg_dark;

    visuals.widgets.noninteractive.bg_fill = theme.bg_panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, theme.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, theme.text_secondary);
    visuals.widgets.noninteractive.rounding = Rounding::same(6.0);

    visuals.widgets.inactive.bg_fill = theme.bg_panel_alt;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, theme.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, theme.text_primary);
    visuals.widgets.inactive.rounding = Rounding::same(6.0);

    visuals.widgets.hovered.bg_fill = theme.bg_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, theme.accent_primary);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.hovered.rounding = Rounding::same(6.0);

    visuals.widgets.active.bg_fill = theme.bg_active;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, theme.accent_primary);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.rounding = Rounding::same(6.0);

    visuals.widgets.open.bg_fill = theme.bg_active;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, theme.border_focus);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.open.rounding = Rounding::same(6.0);

    visuals.selection.bg_fill = Color32::from_rgba_premultiplied(99, 102, 241, 60);
    visuals.selection.stroke = Stroke::new(1.0, theme.accent_primary);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(12.0);
    style.spacing.button_padding = Vec2::new(10.0, 6.0);

    ctx.set_style(style);
}
