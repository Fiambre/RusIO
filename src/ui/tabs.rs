use eframe::egui::{self, Color32, RichText, Rounding, Stroke, Vec2};

use crate::config::LocalShellType;
use crate::session::{SessionKind, SessionTab};
use crate::theme::Theme;

pub enum TabAction {
    SelectTab(usize),
    CloseTab(usize),
    NewTabLocal(LocalShellType),
    OpenQuickConnect,
}

pub fn render_tabs(
    ui: &mut egui::Ui,
    tabs: &[SessionTab],
    active_idx: Option<usize>,
) -> Option<TabAction> {
    let theme = Theme::default();
    let mut triggered_action = None;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);

        // Render each active session tab
        for (i, tab) in tabs.iter().enumerate() {
            let is_selected = active_idx == Some(i);
            let is_alive = tab.is_alive();

            let (proto_text, proto_color) = match &tab.kind {
                SessionKind::Terminal(t) => match t.backend {
                    crate::terminal::TerminalBackend::Local(_) => ("LOCAL", theme.accent_local),
                    crate::terminal::TerminalBackend::Ssh(_) => ("SSH", theme.accent_ssh),
                },
                SessionKind::Vnc { .. } => ("VNC", theme.accent_vnc),
            };

            let bg_color = if is_selected {
                theme.bg_panel_alt
            } else {
                theme.bg_panel
            };

            let border_color = if is_selected {
                theme.accent_primary
            } else {
                theme.border
            };

            let frame = egui::Frame::none()
                .fill(bg_color)
                .stroke(Stroke::new(if is_selected { 1.5 } else { 1.0 }, border_color))
                .rounding(Rounding {
                    nw: 6.0,
                    ne: 6.0,
                    sw: 0.0,
                    se: 0.0,
                })
                .inner_margin(egui::Margin::symmetric(10.0, 6.0));

            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Protocol badge
                    ui.label(
                        RichText::new(proto_text)
                            .size(9.0)
                            .strong()
                            .color(proto_color),
                    );

                    // Status dot
                    let status_dot = if is_alive { "🟢" } else { "🔴" };
                    ui.label(RichText::new(status_dot).size(8.0));

                    // Title
                    let title_text = if tab.title.len() > 18 {
                        format!("{}...", &tab.title[..16])
                    } else {
                        tab.title.clone()
                    };

                    let title_btn = ui.selectable_label(
                        false,
                        RichText::new(title_text)
                            .size(12.0)
                            .strong()
                            .color(if is_selected {
                                Color32::WHITE
                            } else {
                                theme.text_secondary
                            }),
                    );

                    if title_btn.clicked() {
                        triggered_action = Some(TabAction::SelectTab(i));
                    }

                    // Close button (X)
                    if ui
                        .small_button(RichText::new("✕").size(10.0).color(theme.text_muted))
                        .on_hover_text("Cerrar sesión")
                        .clicked()
                    {
                        triggered_action = Some(TabAction::CloseTab(i));
                    }
                });
            });
        }

        // New Tab '+' button
        ui.menu_button(
            RichText::new(" ➕ ")
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

                if ui
                    .button("⚡ PowerShell (Windows)")
                    .clicked()
                {
                    triggered_action = Some(TabAction::NewTabLocal(LocalShellType::PowerShell));
                    ui.close_menu();
                }

                if ui
                    .button("🐧 WSL (Linux Bash)")
                    .clicked()
                {
                    triggered_action = Some(TabAction::NewTabLocal(LocalShellType::Wsl));
                    ui.close_menu();
                }

                if ui
                    .button("💻 Command Prompt (CMD)")
                    .clicked()
                {
                    triggered_action = Some(TabAction::NewTabLocal(LocalShellType::Cmd));
                    ui.close_menu();
                }

                ui.separator();

                if ui
                    .button("🌐 Conexión Rápida SSH / VNC...")
                    .clicked()
                {
                    triggered_action = Some(TabAction::OpenQuickConnect);
                    ui.close_menu();
                }
            },
        );
    });

    triggered_action
}
