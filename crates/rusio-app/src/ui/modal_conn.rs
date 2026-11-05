use eframe::egui::{self, Color32, Rect, RichText, CornerRadius, Sense, Stroke, Vec2};
use rusio_core::config::{ConnectionProfile, Folder, ProtocolData};
use rusio_plugin_api::icons;
use rusio_plugin_api::theme::Theme;
use rusio_plugin_api::{Category, Params};
use uuid::Uuid;

use crate::registry::Registry;

#[derive(Clone)]
pub struct ConnEditorState {
    pub id: Option<Uuid>,
    pub name: String,
    pub folder_id: Option<Uuid>,
    /// Protocol id (see `ProtocolDef::id`) and its plugin-defined params.
    pub protocol_id: String,
    pub params: Params,
    pub color_tag: String,
    pub description: String,
    /// Gateway connection this one is reached through (see `ConnectionProfile::via`).
    pub via: Option<Uuid>,
    /// Picked icon key (`None` = the protocol's own).
    pub icon: Option<String>,
}

impl ConnEditorState {
    /// A blank connection; `protocol_id` picks the protocol (first registered if `None`).
    pub fn new(registry: &Registry, protocol_id: Option<&str>) -> Self {
        let entry = protocol_id
            .and_then(|id| registry.get(id))
            .or_else(|| registry.entries().first());
        Self {
            id: None,
            name: "Nueva Conexión".to_string(),
            folder_id: None,
            protocol_id: entry.map(|e| e.def.id.to_string()).unwrap_or_default(),
            params: entry
                .map(|e| e.plugin.default_params())
                .unwrap_or(Params::Null),
            color_tag: "#10b981".to_string(),
            description: String::new(),
            via: None,
            icon: None,
        }
    }

    pub fn from_profile(p: &ConnectionProfile) -> Self {
        Self {
            id: Some(p.id),
            name: p.name.clone(),
            folder_id: p.folder_id,
            protocol_id: p.protocol.kind.clone(),
            params: p.protocol.params.clone(),
            color_tag: p.color_tag.clone().unwrap_or_else(|| "#10b981".to_string()),
            description: p.description.clone(),
            via: p.via,
            icon: p.icon.clone(),
        }
    }

    pub fn to_profile(&self) -> ConnectionProfile {
        ConnectionProfile {
            id: self.id.unwrap_or_else(Uuid::new_v4),
            name: self.name.clone(),
            folder_id: self.folder_id,
            protocol: ProtocolData::new(self.protocol_id.clone(), self.params.clone()),
            color_tag: Some(self.color_tag.clone()),
            description: self.description.clone(),
            via: self.via,
            icon: self.icon.clone(),
        }
    }
}

pub enum ModalResult {
    Save(ConnectionProfile),
    SaveAndConnect(ConnectionProfile),
    Cancel,
}

/// True if following `via` links from `start` reaches `target` (would create a cycle).
fn reaches(connections: &[ConnectionProfile], start: Option<Uuid>, target: Option<Uuid>) -> bool {
    let Some(target) = target else { return false };
    let mut cur = start;
    for _ in 0..32 {
        match cur {
            Some(id) if id == target => return true,
            Some(id) => cur = connections.iter().find(|c| c.id == id).and_then(|c| c.via),
            None => return false,
        }
    }
    true
}

/// A selectable square showing an icon; returns true when clicked.
fn icon_choice(
    ui: &mut egui::Ui,
    selected: bool,
    tip: &str,
    paint: impl FnOnce(&egui::Ui, Rect),
) -> bool {
    let theme = Theme::default();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::same(6), theme.bg_active);
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0_f32, theme.accent_primary), egui::StrokeKind::Middle);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), theme.bg_hover);
    }
    paint(ui, Rect::from_center_size(rect.center(), Vec2::splat(18.0)));
    resp.on_hover_text(tip).clicked()
}

pub enum TypePick {
    Chosen(&'static str),
    Cancel,
}

/// First step of "new connection": pick the kind, grouped by category.
pub fn render_type_picker(ctx: &egui::Context, registry: &Registry) -> Option<TypePick> {
    let theme = Theme::default();
    let mut result = None;
    egui::Window::new("Nueva conexión: elige el tipo")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_width(460.0);
            for cat in Category::ALL {
                let entries: Vec<_> = registry.entries().iter().filter(|e| e.def.category == cat).collect();
                if entries.is_empty() {
                    continue;
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(cat.label().to_uppercase())
                        .size(11.0)
                        .strong()
                        .color(theme.text_muted),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
                    for e in entries {
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(140.0, 76.0), Sense::click());
                        let fill = if resp.hovered() { theme.bg_hover } else { theme.bg_panel_alt };
                        ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
                        ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0_f32, theme.border), egui::StrokeKind::Middle);
                        let icon_rect = Rect::from_center_size(
                            egui::pos2(rect.center().x, rect.min.y + 24.0),
                            Vec2::splat(26.0),
                        );
                        icons::paint_protocol_icon(ui, e.def.id, e.def.icon, e.def.color, icon_rect);
                        ui.painter().text(
                            egui::pos2(rect.center().x, rect.max.y - 18.0),
                            egui::Align2::CENTER_CENTER,
                            e.def.name,
                            egui::FontId::proportional(11.5),
                            theme.text_primary,
                        );
                        if resp.clicked() {
                            result = Some(TypePick::Chosen(e.def.id));
                        }
                    }
                });
            }
            ui.add_space(8.0);
            ui.separator();
            if ui.button("Cancelar").clicked() {
                result = Some(TypePick::Cancel);
            }
        });
    result
}

pub fn render_conn_modal(
    ctx: &egui::Context,
    state: &mut ConnEditorState,
    folders: &[Folder],
    connections: &[ConnectionProfile],
    registry: &Registry,
) -> Option<ModalResult> {
    let mut result = None;
    let theme = Theme::default();

    egui::Window::new(if state.id.is_some() {
        "Editar Conexión"
    } else {
        "Nueva Conexión"
    })
    .collapsible(false)
    .resizable(false)
    .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, 0.0))
    .fixed_size(Vec2::new(520.0, 480.0))
    .show(ctx, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);

        // General settings
        ui.horizontal(|ui| {
            ui.label(RichText::new("Nombre:").strong().size(12.0));
            ui.text_edit_singleline(&mut state.name);
        });

        // Folder selection
        ui.horizontal(|ui| {
            ui.label(RichText::new("Carpeta:").strong().size(12.0));
            let selected_folder_name = if let Some(fid) = state.folder_id {
                folders
                    .iter()
                    .find(|f| f.id == fid)
                    .map(|f| f.name.as_str())
                    .unwrap_or("Desconocida")
            } else {
                "(Ninguna / Raíz)"
            };

            egui::ComboBox::from_id_salt("folder_select")
                .selected_text(selected_folder_name)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(state.folder_id.is_none(), "(Ninguna / Raíz)")
                        .clicked()
                    {
                        state.folder_id = None;
                    }
                    for f in folders {
                        if ui
                            .selectable_label(state.folder_id == Some(f.id), &f.name)
                            .clicked()
                        {
                            state.folder_id = Some(f.id);
                        }
                    }
                });
        });

        ui.separator();

        // The type was chosen before this form opened and is fixed from here on.
        if let Some(e) = registry.get(&state.protocol_id) {
            ui.horizontal(|ui| {
                icons::protocol_icon(ui, e.def.id, e.def.icon, e.def.color, 18.0);
                ui.label(RichText::new(e.def.name).strong().size(13.0));
            });
        }

        // Icon: the protocol's own, or one from the shared set
        ui.horizontal(|ui| {
            ui.label(RichText::new("Icono:").strong().size(12.0));
            if let Some(e) = registry.get(&state.protocol_id) {
                if icon_choice(ui, state.icon.is_none(), "Predeterminado", |ui, rect| {
                    icons::paint_protocol_icon(ui, e.def.id, e.def.icon, e.def.color, rect)
                }) {
                    state.icon = None;
                }
            }
            for (key, glyph, label) in icons::CHOICES {
                let selected = state.icon.as_deref() == Some(*key);
                if icon_choice(ui, selected, label, |ui, rect| {
                    icons::paint_icon(ui, rect, *glyph, theme.text_primary)
                }) {
                    state.icon = Some((*key).to_string());
                }
            }
        });

        ui.separator();

        // Protocol-specific form, drawn by the plugin
        match registry.get(&state.protocol_id) {
            Some(entry) => entry.plugin.edit_ui(ui, &mut state.params),
            None => {
                ui.label(
                    RichText::new(format!(
                        "No hay ningún plugin instalado para el protocolo \"{}\".",
                        state.protocol_id
                    ))
                    .color(theme.status_offline),
                );
            }
        }

        // Gateway: only for protocols that can be reached through one
        let tunnelable = registry
            .get(&state.protocol_id)
            .map_or(false, |e| e.def.endpoint.is_some());
        if tunnelable {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Conectar a través de:").strong().size(12.0));
                let candidates: Vec<&ConnectionProfile> = connections
                    .iter()
                    .filter(|c| Some(c.id) != state.id)
                    .filter(|c| registry.get(&c.protocol.kind).map_or(false, |e| e.def.gateway))
                    .filter(|c| !reaches(connections, c.via, state.id))
                    .collect();
                let current = state
                    .via
                    .and_then(|id| connections.iter().find(|c| c.id == id))
                    .map_or("(Conexión directa)", |c| c.name.as_str());
                egui::ComboBox::from_id_salt("via_select")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(state.via.is_none(), "(Conexión directa)").clicked() {
                            state.via = None;
                        }
                        for c in candidates {
                            if ui.selectable_label(state.via == Some(c.id), &c.name).clicked() {
                                state.via = Some(c.id);
                            }
                        }
                    });
            });
            ui.label(
                RichText::new("Un túnel SSH hacia un bastión: el destino se alcanza desde esa máquina.")
                    .size(11.0)
                    .color(theme.text_muted),
            );
        } else {
            state.via = None;
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // Action buttons
        ui.horizontal(|ui| {
            if ui
                .button(RichText::new("Guardar y Conectar").strong().color(Color32::WHITE))
                .clicked()
            {
                result = Some(ModalResult::SaveAndConnect(state.to_profile()));
            }

            if ui.button("Guardar").clicked() {
                result = Some(ModalResult::Save(state.to_profile()));
            }

            if ui.button("Cancelar").clicked() {
                result = Some(ModalResult::Cancel);
            }
        });
    });

    result
}

pub fn render_new_folder_modal(ctx: &egui::Context, name: &mut String) -> Option<bool> {
    let mut result = None;

    egui::Window::new("Nueva Carpeta")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, 0.0))
        .fixed_size(Vec2::new(320.0, 140.0))
        .show(ctx, |ui| {
            ui.label("Nombre de la carpeta:");
            ui.text_edit_singleline(name);
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Crear").strong().color(Color32::WHITE))
                    .clicked()
                {
                    result = Some(true);
                }
                if ui.button("Cancelar").clicked() {
                    result = Some(false);
                }
            });
        });

    result
}
