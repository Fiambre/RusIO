use eframe::egui::{self, Color32, RichText, CornerRadius, Stroke, Vec2};

use rusio_plugin_api::icons::{self, Icon};
use rusio_plugin_api::theme::Theme;
use rusio_plugin_api::{Params, QuickLaunch};

use crate::session::SessionTab;

pub enum TabAction {
    SelectTab(usize),
    CloseTab(usize),
    /// Open a one-click session of the given protocol.
    Quick { plugin_id: &'static str, params: Params },
    OpenQuickConnect,
}

pub fn render_tabs(
    ui: &mut egui::Ui,
    tabs: &[SessionTab],
    active_idx: Option<usize>,
    quick: &[(&'static str, QuickLaunch)],
) -> Option<TabAction> {
    let theme = Theme::default();
    let mut triggered_action = None;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);

        // Render each active session tab
        for (i, tab) in tabs.iter().enumerate() {
            let is_selected = active_idx == Some(i);
            let is_alive = tab.session.is_alive();
            let (proto_text, proto_color) = tab.session.badge();
            let full_title = tab.display_title();

            let bg_color = if is_selected { theme.bg_panel_alt } else { Color32::TRANSPARENT };

            let frame = egui::Frame::NONE
                .fill(bg_color)
                .corner_radius(CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 })
                .inner_margin(egui::Margin::symmetric(10, 6));

            let out = frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        dot.center(),
                        3.0,
                        if is_alive { theme.status_online } else { theme.status_offline },
                    );

                    let proto_resp = ui.add(
                        egui::Label::new(RichText::new(proto_text).size(9.0).strong().color(proto_color))
                            .selectable(false)
                            .sense(egui::Sense::click()),
                    );
                    if proto_resp.clicked() {
                        triggered_action = Some(TabAction::SelectTab(i));
                    }

                    let title_text: String = if full_title.chars().count() > 18 {
                        format!("{}…", full_title.chars().take(16).collect::<String>())
                    } else {
                        full_title.clone()
                    };

                    let title_resp = ui.add(
                        egui::Label::new(
                            RichText::new(title_text)
                                .size(12.0)
                                .color(if is_selected { Color32::WHITE } else { theme.text_secondary }),
                        )
                        .selectable(false)
                        .sense(egui::Sense::click()),
                    );
                    if title_resp.clicked() {
                        triggered_action = Some(TabAction::SelectTab(i));
                    }

                    if icons::icon_button(ui, Icon::Close, None, "Cerrar sesión") {
                        triggered_action = Some(TabAction::CloseTab(i));
                    }
                });
            });

            if is_selected {
                let r = out.response.rect;
                ui.painter().line_segment(
                    [r.left_bottom(), r.right_bottom()],
                    Stroke::new(2.0_f32, theme.accent_primary),
                );
            }
        }

        // New Tab '+' button
        ui.menu_button(
            RichText::new(" + ")
                .size(12.0)
                .strong()
                .color(theme.accent_primary),
            |ui| {
                ui.set_min_width(180.0);
                ui.label(
                    RichText::new("NUEVA SESIÓN")
                        .size(10.0)
                        .strong()
                        .color(theme.text_muted),
                );
                ui.separator();

                for (plugin_id, q) in quick {
                    if ui.button(&q.label).clicked() {
                        triggered_action = Some(TabAction::Quick {
                            plugin_id,
                            params: q.params.clone(),
                        });
                        ui.close();
                    }
                }

                ui.separator();

                if ui
                    .button("Nueva conexión...")
                    .clicked()
                {
                    triggered_action = Some(TabAction::OpenQuickConnect);
                    ui.close();
                }
            },
        );
    });

    triggered_action
}
