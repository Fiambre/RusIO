use rusio_plugin_api::egui::{self, FontId, Margin, Modifiers, RichText, Vec2};

use crate::editor::EditorSession;
use rusio_plugin_api::icons::{self, Icon};
use rusio_plugin_api::theme::Theme;

const CTRL_H: f32 = 26.0;

pub fn render_editor(ui: &mut egui::Ui, e: &mut EditorSession) {
    let theme = Theme::default();
    e.poll();

    let ctrl_s = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, egui::Key::S));
    if ctrl_s {
        e.save();
    }

    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = CTRL_H;
        ui.spacing_mut().button_padding = Vec2::new(10.0, 4.0);

        let connected = e.is_connected();
        icons::status_dot(ui, if connected { theme.status_online } else { theme.status_offline });
        ui.label(
            RichText::new(&e.path)
                .size(11.0)
                .monospace()
                .color(theme.text_secondary),
        );
        if e.modified {
            ui.label(RichText::new("modificado").size(10.0).color(theme.accent_vnc));
        }
        ui.separator();

        ui.add_enabled_ui(e.is_loaded() && !e.is_saving(), |ui| {
            if icons::icon_button_sized(ui, Icon::Save, Some(theme.accent_primary), "Guardar (Ctrl+S)", CTRL_H) {
                e.save();
            }
        });
        if icons::icon_button_sized(
            ui,
            Icon::Refresh,
            None,
            "Recargar (descarta los cambios)",
            CTRL_H,
        ) {
            e.reload();
        }

        ui.separator();
        ui.label(RichText::new("Fuente:").size(11.0));
        if ui.small_button("-").clicked() && e.font_size > 9.0 {
            e.font_size -= 1.0;
        }
        ui.label(format!("{:.0}", e.font_size));
        if ui.small_button("+").clicked() && e.font_size < 28.0 {
            e.font_size += 1.0;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !e.status.is_empty() {
                ui.label(
                    RichText::new(&e.status)
                        .size(11.0)
                        .color(if e.is_error { theme.status_offline } else { theme.text_secondary }),
                );
            }
        });
    });
    ui.separator();

    if !e.is_loaded() {
        if !e.is_error {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Cargando...").color(theme.text_muted));
            });
        }
        return;
    }

    let font = FontId::monospace(e.font_size);
    let lines = e.text.split('\n').count().max(1);
    let gutter: String = (1..=lines).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");

    egui::ScrollArea::both()
        .id_salt("editor_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.add(
                    egui::Label::new(
                        RichText::new(gutter)
                            .font(font.clone())
                            .color(theme.text_muted),
                    )
                    .selectable(false),
                );

                let resp = ui.add(
                    egui::TextEdit::multiline(&mut e.text)
                        .font(font)
                        .code_editor()
                        .frame(egui::Frame::NONE)
                        .margin(Margin::ZERO)
                        .desired_width(f32::INFINITY)
                        .desired_rows(lines.max(30)),
                );
                if resp.changed() {
                    e.modified = true;
                }
            });
        });
}
