use eframe::egui::{self, Color32, RichText, Sense};
use uuid::Uuid;

use crate::config::{ConfigStore, ConnectionProfile, LocalShellType, ProtocolConfig};
use crate::theme::Theme;

pub enum SidebarAction {
    ConnectProfile(ConnectionProfile),
    LaunchLocal(LocalShellType),
    CreateNewConnection(Option<Uuid>),
    CreateNewFolder,
    EditConnection(ConnectionProfile),
    DeleteConnection(Uuid),
    DeleteFolder(Uuid),
}

pub struct SidebarState {
    pub search_query: String,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            search_query: String::new(),
        }
    }
}

pub fn render_sidebar(
    ui: &mut egui::Ui,
    config: &mut ConfigStore,
    state: &mut SidebarState,
) -> Option<SidebarAction> {
    let theme = Theme::default();
    let mut triggered_action = None;

    ui.vertical(|ui| {
        // App brand header
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("⚡ PULENTO")
                    .strong()
                    .size(17.0)
                    .color(theme.accent_primary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(RichText::new("+ Carpeta").size(11.0))
                    .on_hover_text("Crear nueva carpeta para organizar conexiones")
                    .clicked()
                {
                    triggered_action = Some(SidebarAction::CreateNewFolder);
                }
                if ui
                    .button(
                        RichText::new("+ Conexión")
                            .size(11.0)
                            .color(Color32::WHITE),
                    )
                    .on_hover_text("Crear nuevo perfil de conexión")
                    .clicked()
                {
                    triggered_action = Some(SidebarAction::CreateNewConnection(None));
                }
            });
        });

        ui.add_space(6.0);

        // Search input
        ui.horizontal(|ui| {
            ui.label(RichText::new("🔍").size(11.0));
            ui.add(
                egui::TextEdit::singleline(&mut state.search_query)
                    .hint_text("Buscar conexiones...")
                    .desired_width(ui.available_width() - 10.0),
            );
        });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);

        // Scrollable connections list
        egui::ScrollArea::vertical()
            .id_salt("sidebar_scroll")
            .max_height(ui.available_height() - 95.0)
            .show(ui, |ui| {
                let query = state.search_query.to_lowercase();

                // Render folders
                let mut folder_to_delete = None;
                let mut conn_to_delete = None;
                let mut conn_to_edit = None;

                for folder_idx in 0..config.folders.len() {
                    let folder = &config.folders[folder_idx];
                    let folder_id = folder.id;
                    let folder_name = folder.name.clone();
                    let mut is_expanded = folder.expanded;

                    // Filter connections in this folder
                    let matching_conns: Vec<_> = config
                        .connections
                        .iter()
                        .filter(|c| c.folder_id == Some(folder_id))
                        .filter(|c| {
                            if query.is_empty() {
                                true
                            } else {
                                c.name.to_lowercase().contains(&query)
                                    || c.protocol.target_summary().to_lowercase().contains(&query)
                            }
                        })
                        .cloned()
                        .collect();

                    if !query.is_empty() && matching_conns.is_empty() {
                        continue;
                    }

                    // Folder Row
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            let icon = if is_expanded { "📂 ▼" } else { "📁 ▶" };
                            if ui
                                .button(RichText::new(icon).size(12.0))
                                .clicked()
                            {
                                is_expanded = !is_expanded;
                            }

                            ui.label(
                                RichText::new(format!("{} ({})", folder_name, matching_conns.len()))
                                    .strong()
                                    .size(13.0)
                                    .color(theme.text_primary),
                            );

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .small_button(RichText::new("🗑").size(10.0))
                                    .on_hover_text("Eliminar carpeta")
                                    .clicked()
                                {
                                    folder_to_delete = Some(folder_id);
                                }
                                if ui
                                    .small_button(RichText::new("+").size(11.0))
                                    .on_hover_text("Añadir conexión a esta carpeta")
                                    .clicked()
                                {
                                    triggered_action =
                                        Some(SidebarAction::CreateNewConnection(Some(folder_id)));
                                }
                            });
                        });

                        config.folders[folder_idx].expanded = is_expanded;

                        // Folder items
                        if is_expanded {
                            ui.indent(folder_id, |ui| {
                                if matching_conns.is_empty() {
                                    ui.label(
                                        RichText::new("(Carpeta vacía)")
                                            .size(11.0)
                                            .color(theme.text_muted),
                                    );
                                } else {
                                    for profile in matching_conns {
                                        if let Some(action) = render_connection_item(
                                            ui,
                                            &profile,
                                            &theme,
                                            &mut conn_to_edit,
                                            &mut conn_to_delete,
                                        ) {
                                            triggered_action = Some(action);
                                        }
                                    }
                                }
                            });
                        }
                    });
                    ui.add_space(2.0);
                }

                // Root / ungrouped connections
                let root_conns: Vec<_> = config
                    .connections
                    .iter()
                    .filter(|c| c.folder_id.is_none())
                    .filter(|c| {
                        if query.is_empty() {
                            true
                        } else {
                            c.name.to_lowercase().contains(&query)
                                || c.protocol.target_summary().to_lowercase().contains(&query)
                        }
                    })
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

                    for profile in root_conns {
                        if let Some(action) = render_connection_item(
                            ui,
                            &profile,
                            &theme,
                            &mut conn_to_edit,
                            &mut conn_to_delete,
                        ) {
                            triggered_action = Some(action);
                        }
                    }
                }

                if let Some(id) = folder_to_delete {
                    triggered_action = Some(SidebarAction::DeleteFolder(id));
                }
                if let Some(p) = conn_to_edit {
                    triggered_action = Some(SidebarAction::EditConnection(p));
                }
                if let Some(id) = conn_to_delete {
                    triggered_action = Some(SidebarAction::DeleteConnection(id));
                }
            });

        // Bottom Fast Launcher Toolbar
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label(
                RichText::new("INICIO RÁPIDO LOCAL")
                    .size(10.0)
                    .strong()
                    .color(theme.text_muted),
            );

            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(
                        RichText::new("⚡ PowerShell")
                            .size(11.0)
                            .color(theme.accent_primary),
                    )
                    .clicked()
                {
                    triggered_action = Some(SidebarAction::LaunchLocal(LocalShellType::PowerShell));
                }
                if ui
                    .button(
                        RichText::new("🐧 WSL")
                            .size(11.0)
                            .color(theme.accent_local),
                    )
                    .clicked()
                {
                    triggered_action = Some(SidebarAction::LaunchLocal(LocalShellType::Wsl));
                }
                if ui
                    .button(
                        RichText::new("💻 CMD")
                            .size(11.0)
                            .color(theme.text_secondary),
                    )
                    .clicked()
                {
                    triggered_action = Some(SidebarAction::LaunchLocal(LocalShellType::Cmd));
                }
            });
        });
    });

    triggered_action
}

fn render_connection_item(
    ui: &mut egui::Ui,
    profile: &ConnectionProfile,
    theme: &Theme,
    conn_to_edit: &mut Option<ConnectionProfile>,
    conn_to_delete: &mut Option<Uuid>,
) -> Option<SidebarAction> {
    let mut action = None;

    let (badge_text, badge_color) = match &profile.protocol {
        ProtocolConfig::Ssh { .. } => ("SSH", theme.accent_ssh),
        ProtocolConfig::Local { .. } => ("LOCAL", theme.accent_local),
        ProtocolConfig::Vnc { .. } => ("VNC", theme.accent_vnc),
    };

    let response = ui.scope(|ui| {
        ui.horizontal(|ui| {
            // Protocol badge
            ui.label(
                RichText::new(badge_text)
                    .size(9.0)
                    .strong()
                    .color(badge_color),
            );

            // Connection Name & Summary
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(&profile.name)
                        .size(12.0)
                        .strong()
                        .color(theme.text_primary),
                );
                ui.label(
                    RichText::new(profile.protocol.target_summary())
                        .size(10.0)
                        .color(theme.text_secondary),
                );
            });

            // Action buttons on right
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(RichText::new("🗑").size(10.0))
                    .on_hover_text("Eliminar conexión")
                    .clicked()
                {
                    *conn_to_delete = Some(profile.id);
                }
                if ui
                    .small_button(RichText::new("✏").size(10.0))
                    .on_hover_text("Editar conexión")
                    .clicked()
                {
                    *conn_to_edit = Some(profile.clone());
                }
                if ui
                    .small_button(
                        RichText::new("▶")
                            .size(11.0)
                            .strong()
                            .color(Color32::from_rgb(34, 197, 94)),
                    )
                    .on_hover_text("Conectar ahora")
                    .clicked()
                {
                    action = Some(SidebarAction::ConnectProfile(profile.clone()));
                }
            });
        });
    });

    // Double click to connect
    if response.response.interact(Sense::click()).double_clicked() {
        action = Some(SidebarAction::ConnectProfile(profile.clone()));
    }

    ui.add_space(2.0);
    action
}
