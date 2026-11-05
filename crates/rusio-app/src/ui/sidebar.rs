use eframe::egui::{self, CornerRadius, Rect, RichText, Sense, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

use rusio_core::config::{ConfigStore, ConnectionProfile};
use rusio_plugin_api::icons::{self, Icon};
use rusio_plugin_api::theme::Theme;
use rusio_plugin_api::ProtocolIcon;

use crate::registry::Registry;

pub enum SidebarAction {
    /// Connect a saved profile, optionally through one of its protocol's extra actions.
    ConnectProfile(ConnectionProfile, Option<&'static str>),
    /// Open a one-click session of the given protocol.
    CreateNewConnection(Option<Uuid>),
    CreateNewFolder,
    EditConnection(ConnectionProfile),
    DeleteConnection(Uuid),
    DeleteFolder(Uuid),
}

#[derive(Default)]
pub struct SidebarState {
    pub search_query: String,
}

fn summary_of(registry: &Registry, profile: &ConnectionProfile) -> String {
    registry
        .get(&profile.protocol.kind)
        .map(|e| e.plugin.summary(&profile.protocol.params))
        .unwrap_or_default()
}

/// What the user did in the list this frame (applied after drawing, never while iterating).
#[derive(Default)]
struct Out {
    action: Option<SidebarAction>,
    folder_to_delete: Option<Uuid>,
    conn_to_edit: Option<ConnectionProfile>,
    conn_to_delete: Option<Uuid>,
}

fn matches_query(registry: &Registry, c: &ConnectionProfile, query: &str) -> bool {
    query.is_empty() || c.name.to_lowercase().contains(query) || summary_of(registry, c).to_lowercase().contains(query)
}

pub fn render_sidebar(
    ui: &mut egui::Ui,
    config: &mut ConfigStore,
    state: &mut SidebarState,
    registry: &Registry,
) -> Option<SidebarAction> {
    let theme = Theme::default();
    let mut out = Out::default();

    ui.vertical(|ui| {
        // Bottom Fast Launcher Toolbar
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("CONEXIONES").size(11.0).strong().color(theme.text_muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::icon_button(ui, Icon::FolderPlus, None, "Nueva carpeta") {
                    out.action = Some(SidebarAction::CreateNewFolder);
                }
                if icons::icon_button(ui, Icon::Plus, Some(theme.accent_primary), "Nueva conexión") {
                    out.action = Some(SidebarAction::CreateNewConnection(None));
                }
            });
        });

        ui.add_space(6.0);
        ui.add(
            egui::TextEdit::singleline(&mut state.search_query)
                .hint_text("Buscar conexiones...")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .id_salt("sidebar_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let query = state.search_query.to_lowercase();
                let names: HashMap<Uuid, String> = config.connections.iter().map(|c| (c.id, c.name.clone())).collect();

                for folder_idx in 0..config.folders.len() {
                    let folder_id = config.folders[folder_idx].id;
                    let in_folder: Vec<ConnectionProfile> = config
                        .connections
                        .iter()
                        .filter(|c| c.folder_id == Some(folder_id))
                        .filter(|c| matches_query(registry, c, &query))
                        .cloned()
                        .collect();
                    if !query.is_empty() && in_folder.is_empty() {
                        continue;
                    }
                    render_folder(ui, config, folder_idx, &in_folder, &theme, registry, &names, &mut out);
                }

                // Root / ungrouped connections
                let root_conns: Vec<ConnectionProfile> = config
                    .connections
                    .iter()
                    .filter(|c| c.folder_id.is_none())
                    .filter(|c| matches_query(registry, c, &query))
                    .cloned()
                    .collect();

                if !root_conns.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("SIN CARPETA")
                            .size(11.0)
                            .strong()
                            .color(theme.text_secondary),
                    );
                    for profile in &root_conns {
                        if let Some(a) = render_connection_item(
                            ui,
                            profile,
                            &theme,
                            registry,
                            &names,
                            &mut out.conn_to_edit,
                            &mut out.conn_to_delete,
                        ) {
                            out.action = Some(a);
                        }
                    }
                }

                if let Some(id) = out.folder_to_delete.take() {
                    out.action = Some(SidebarAction::DeleteFolder(id));
                }
                if let Some(p) = out.conn_to_edit.take() {
                    out.action = Some(SidebarAction::EditConnection(p));
                }
                if let Some(id) = out.conn_to_delete.take() {
                    out.action = Some(SidebarAction::DeleteConnection(id));
                }
            });
    });

    out.action
}

#[allow(clippy::too_many_arguments)]
fn render_folder(
    ui: &mut egui::Ui,
    config: &mut ConfigStore,
    folder_idx: usize,
    conns: &[ConnectionProfile],
    theme: &Theme,
    registry: &Registry,
    names: &HashMap<Uuid, String>,
    out: &mut Out,
) {
    let folder_id = config.folders[folder_idx].id;
    let folder_name = config.folders[folder_idx].name.clone();
    let mut is_expanded = config.folders[folder_idx].expanded;

    ui.vertical(|ui| {
        let row = ui.horizontal(|ui| {
            ui.add_space(8.0);
            icons::chevron(ui, is_expanded);
            ui.label(
                RichText::new(&folder_name)
                    .strong()
                    .size(12.5)
                    .color(theme.text_primary),
            );
            ui.label(
                RichText::new(conns.len().to_string())
                    .size(10.0)
                    .color(theme.text_muted),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::icon_button(ui, Icon::Close, None, "Eliminar carpeta") {
                    out.folder_to_delete = Some(folder_id);
                }
                if icons::icon_button(ui, Icon::Plus, None, "Añadir conexión a esta carpeta") {
                    out.action = Some(SidebarAction::CreateNewConnection(Some(folder_id)));
                }
            });
        });
        if row.response.interact(Sense::click()).clicked() {
            is_expanded = !is_expanded;
        }
        config.folders[folder_idx].expanded = is_expanded;

        if is_expanded {
            ui.indent(folder_id, |ui| {
                if conns.is_empty() {
                    ui.label(RichText::new("(Carpeta vacía)").size(11.0).color(theme.text_muted));
                }
                for profile in conns {
                    if let Some(a) = render_connection_item(
                        ui,
                        profile,
                        theme,
                        registry,
                        names,
                        &mut out.conn_to_edit,
                        &mut out.conn_to_delete,
                    ) {
                        out.action = Some(a);
                    }
                }
            });
        }
    });
    ui.add_space(4.0);
}

fn render_connection_item(
    ui: &mut egui::Ui,
    profile: &ConnectionProfile,
    theme: &Theme,
    registry: &Registry,
    names: &HashMap<Uuid, String>,
    conn_to_edit: &mut Option<ConnectionProfile>,
    conn_to_delete: &mut Option<Uuid>,
) -> Option<SidebarAction> {
    let mut action = None;

    let entry = registry.get(&profile.protocol.kind);
    let (badge_text, badge_color) = entry
        .map(|e| (e.def.badge, e.def.color))
        .unwrap_or(("?", theme.text_muted));
    let mut summary = summary_of(registry, profile);
    if let Some(gw) = profile.via.and_then(|id| names.get(&id)) {
        summary = format!("{summary} · vía {gw}");
    }

    const ROW_H: f32 = 38.0;
    const ACTIONS_W: f32 = icons::ICON_BTN + 24.0;

    let response = ui.scope(|ui| {
        ui.set_max_width(ui.available_width());
        // Selectable labels would swallow right/double clicks meant for the row.
        ui.style_mut().interaction.selectable_labels = false;
        let r = ui.available_rect_before_wrap();
        let row_rect = Rect::from_min_size(r.min, Vec2::new(r.width(), ROW_H));
        let hover = ui.rect_contains_pointer(row_rect);
        if hover {
            ui.painter()
                .rect_filled(row_rect, CornerRadius::same(6), theme.bg_hover);
        }
        ui.horizontal(|ui| {
            ui.set_height(ROW_H);
            ui.add_space(4.0);
            ui.allocate_ui(Vec2::new(30.0, ROW_H), |ui| {
                ui.centered_and_justified(|ui| {
                    let custom = profile.icon.as_deref().and_then(icons::choice_by_key);
                    if let Some(g) = custom {
                        icons::protocol_icon(ui, "custom", ProtocolIcon::Glyph(g), badge_color, 18.0);
                    } else if let Some(e) = entry {
                        icons::protocol_icon(ui, e.def.id, e.def.icon, badge_color, 18.0);
                    }
                })
                .response
                .on_hover_text(badge_text);
            });

            // Name column always leaves room for the hover actions.
            let text_w = (ui.available_width() - ACTIONS_W).max(40.0);
            ui.allocate_ui(Vec2::new(text_w, ROW_H), |ui| {
                ui.set_width(text_w);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(
                        egui::Label::new(
                            RichText::new(&profile.name)
                                .size(12.0)
                                .strong()
                                .color(theme.text_primary),
                        )
                        .truncate(),
                    );
                    ui.add(egui::Label::new(RichText::new(&summary).size(10.0).color(theme.text_secondary)).truncate());
                });
            });

            // Space is always reserved so hovering never changes the row width.
            if hover {
                if icons::icon_button(ui, Icon::Play, Some(theme.status_online), "Conectar ahora") {
                    action = Some(SidebarAction::ConnectProfile(profile.clone(), None));
                }
            } else {
                ui.allocate_space(Vec2::splat(icons::ICON_BTN));
            }
        });
    });

    let item = response.response.interact(Sense::click());

    // Right click: edit / delete live here
    item.context_menu(|ui| {
        if ui.button("Conectar").clicked() {
            action = Some(SidebarAction::ConnectProfile(profile.clone(), None));
            ui.close();
        }
        if let Some(entry) = entry {
            for a in &entry.def.actions {
                if ui.button(a.label).clicked() {
                    action = Some(SidebarAction::ConnectProfile(profile.clone(), Some(a.id)));
                    ui.close();
                }
            }
        }
        if ui.button("Editar...").clicked() {
            *conn_to_edit = Some(profile.clone());
            ui.close();
        }
        ui.separator();
        if ui
            .button(RichText::new("Eliminar...").color(theme.status_offline))
            .clicked()
        {
            *conn_to_delete = Some(profile.id);
            ui.close();
        }
    });

    // Double click to connect
    if item.double_clicked() {
        action = Some(SidebarAction::ConnectProfile(profile.clone(), None));
    }

    ui.add_space(2.0);
    action
}
