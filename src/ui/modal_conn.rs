use eframe::egui::{self, Color32, RichText, Vec2};
use uuid::Uuid;

use crate::config::{
    ConnectionProfile, Folder, LocalShellType, ProtocolConfig, SshAuth,
};
use crate::theme::Theme;

#[derive(Clone)]
pub struct ConnEditorState {
    pub id: Option<Uuid>,
    pub name: String,
    pub folder_id: Option<Uuid>,
    pub protocol_kind: usize, // 0 = SSH, 1 = Local, 2 = VNC
    // SSH fields
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    pub ssh_auth_type: usize, // 0 = Password, 1 = Keyfile, 2 = Agent
    pub ssh_password: String,
    pub ssh_key_path: String,
    pub ssh_passphrase: String,
    // Local fields
    pub local_shell: LocalShellType,
    pub local_cwd: String,
    // VNC fields
    pub vnc_host: String,
    pub vnc_port: u16,
    pub vnc_password: String,
    // Style
    pub color_tag: String,
    pub description: String,
}

impl Default for ConnEditorState {
    fn default() -> Self {
        Self {
            id: None,
            name: "Nueva Conexión".to_string(),
            folder_id: None,
            protocol_kind: 0,
            ssh_host: "127.0.0.1".to_string(),
            ssh_port: 22,
            ssh_user: "root".to_string(),
            ssh_auth_type: 0,
            ssh_password: String::new(),
            ssh_key_path: String::new(),
            ssh_passphrase: String::new(),
            local_shell: LocalShellType::PowerShell,
            local_cwd: String::new(),
            vnc_host: "127.0.0.1".to_string(),
            vnc_port: 5900,
            vnc_password: String::new(),
            color_tag: "#10b981".to_string(),
            description: String::new(),
        }
    }
}

impl ConnEditorState {
    pub fn from_profile(p: &ConnectionProfile) -> Self {
        let mut state = Self::default();
        state.id = Some(p.id);
        state.name = p.name.clone();
        state.folder_id = p.folder_id;
        state.description = p.description.clone();
        if let Some(ref c) = p.color_tag {
            state.color_tag = c.clone();
        }

        match &p.protocol {
            ProtocolConfig::Ssh {
                host,
                port,
                username,
                auth,
            } => {
                state.protocol_kind = 0;
                state.ssh_host = host.clone();
                state.ssh_port = *port;
                state.ssh_user = username.clone();
                match auth {
                    SshAuth::Password(pw) => {
                        state.ssh_auth_type = 0;
                        state.ssh_password = pw.clone();
                    }
                    SshAuth::KeyFile { path, passphrase } => {
                        state.ssh_auth_type = 1;
                        state.ssh_key_path = path.clone();
                        state.ssh_passphrase = passphrase.clone().unwrap_or_default();
                    }
                    SshAuth::Agent => {
                        state.ssh_auth_type = 2;
                    }
                }
            }
            ProtocolConfig::Local { shell, working_dir } => {
                state.protocol_kind = 1;
                state.local_shell = shell.clone();
                state.local_cwd = working_dir.clone().unwrap_or_default();
            }
            ProtocolConfig::Vnc {
                host,
                port,
                password,
            } => {
                state.protocol_kind = 2;
                state.vnc_host = host.clone();
                state.vnc_port = *port;
                state.vnc_password = password.clone().unwrap_or_default();
            }
        }

        state
    }

    pub fn to_profile(&self) -> ConnectionProfile {
        let protocol = match self.protocol_kind {
            0 => {
                let auth = match self.ssh_auth_type {
                    0 => SshAuth::Password(self.ssh_password.clone()),
                    1 => SshAuth::KeyFile {
                        path: self.ssh_key_path.clone(),
                        passphrase: if self.ssh_passphrase.is_empty() {
                            None
                        } else {
                            Some(self.ssh_passphrase.clone())
                        },
                    },
                    _ => SshAuth::Agent,
                };
                ProtocolConfig::Ssh {
                    host: self.ssh_host.trim().to_string(),
                    port: self.ssh_port,
                    username: self.ssh_user.trim().to_string(),
                    auth,
                }
            }
            1 => ProtocolConfig::Local {
                shell: self.local_shell.clone(),
                working_dir: if self.local_cwd.trim().is_empty() {
                    None
                } else {
                    Some(self.local_cwd.trim().to_string())
                },
            },
            _ => ProtocolConfig::Vnc {
                host: self.vnc_host.trim().to_string(),
                port: self.vnc_port,
                password: if self.vnc_password.is_empty() {
                    None
                } else {
                    Some(self.vnc_password.clone())
                },
            },
        };

        ConnectionProfile {
            id: self.id.unwrap_or_else(Uuid::new_v4),
            name: self.name.clone(),
            folder_id: self.folder_id,
            protocol,
            color_tag: Some(self.color_tag.clone()),
            description: self.description.clone(),
        }
    }
}

pub enum ModalResult {
    Save(ConnectionProfile),
    SaveAndConnect(ConnectionProfile),
    Cancel,
}

pub fn render_conn_modal(
    ctx: &egui::Context,
    state: &mut ConnEditorState,
    folders: &[Folder],
) -> Option<ModalResult> {
    let mut result = None;
    let theme = Theme::default();

    egui::Window::new(if state.id.is_some() {
        "✏ Editar Conexión"
    } else {
        "➕ Nueva Conexión"
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

        // Protocol selector
        ui.label(RichText::new("Tipo de Protocolo:").strong().size(12.0));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut state.protocol_kind, 0, "🌐 SSH (Terminal Remoto)");
            ui.selectable_value(&mut state.protocol_kind, 1, "⚡ Local (PowerShell/WSL/CMD)");
            ui.selectable_value(&mut state.protocol_kind, 2, "🖥 VNC (Escritorio Remoto)");
        });

        ui.separator();

        // Protocol specific config
        match state.protocol_kind {
            0 => {
                // SSH Configuration
                ui.horizontal(|ui| {
                    ui.label("Host / IP:");
                    ui.text_edit_singleline(&mut state.ssh_host);
                    ui.label("Puerto:");
                    let mut port_str = state.ssh_port.to_string();
                    if ui.add(egui::TextEdit::singleline(&mut port_str).desired_width(60.0)).changed() {
                        if let Ok(p) = port_str.parse::<u16>() {
                            state.ssh_port = p;
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Usuario:");
                    ui.text_edit_singleline(&mut state.ssh_user);
                });

                ui.label(RichText::new("Método de Autenticación:").strong().size(11.0));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut state.ssh_auth_type, 0, "Contraseña");
                    ui.selectable_value(&mut state.ssh_auth_type, 1, "Clave Privada (Keyfile)");
                    ui.selectable_value(&mut state.ssh_auth_type, 2, "SSH Agent");
                });

                match state.ssh_auth_type {
                    0 => {
                        ui.horizontal(|ui| {
                            ui.label("Contraseña:");
                            ui.add(egui::TextEdit::singleline(&mut state.ssh_password).password(true));
                        });
                    }
                    1 => {
                        ui.horizontal(|ui| {
                            ui.label("Ruta de clave:");
                            ui.text_edit_singleline(&mut state.ssh_key_path);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Passphrase (opcional):");
                            ui.add(egui::TextEdit::singleline(&mut state.ssh_passphrase).password(true));
                        });
                    }
                    _ => {
                        ui.label(
                            RichText::new("Se utilizarán las claves cargadas en el agente SSH de Windows/Pageant.")
                                .size(11.0)
                                .color(theme.text_secondary),
                        );
                    }
                }
            }
            1 => {
                // Local Terminal Configuration
                ui.horizontal(|ui| {
                    ui.label("Shell:");
                    egui::ComboBox::from_id_salt("local_shell_select")
                        .selected_text(state.local_shell.display_name())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut state.local_shell,
                                LocalShellType::PowerShell,
                                "PowerShell",
                            );
                            ui.selectable_value(
                                &mut state.local_shell,
                                LocalShellType::Wsl,
                                "WSL (Linux Bash)",
                            );
                            ui.selectable_value(
                                &mut state.local_shell,
                                LocalShellType::Cmd,
                                "Command Prompt (CMD)",
                            );
                            ui.selectable_value(
                                &mut state.local_shell,
                                LocalShellType::GitBash,
                                "Git Bash",
                            );
                        });
                });

                ui.horizontal(|ui| {
                    ui.label("Directorio de inicio (opcional):");
                    ui.text_edit_singleline(&mut state.local_cwd);
                });
            }
            _ => {
                // VNC Configuration
                ui.horizontal(|ui| {
                    ui.label("Host / IP:");
                    ui.text_edit_singleline(&mut state.vnc_host);
                    ui.label("Puerto:");
                    let mut port_str = state.vnc_port.to_string();
                    if ui.add(egui::TextEdit::singleline(&mut port_str).desired_width(60.0)).changed() {
                        if let Ok(p) = port_str.parse::<u16>() {
                            state.vnc_port = p;
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Contraseña VNC (opcional):");
                    ui.add(egui::TextEdit::singleline(&mut state.vnc_password).password(true));
                });
            }
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // Action Buttons
        ui.horizontal(|ui| {
            if ui
                .button(
                    RichText::new("🚀 Guardar y Conectar")
                        .strong()
                        .color(Color32::WHITE),
                )
                .clicked()
            {
                result = Some(ModalResult::SaveAndConnect(state.to_profile()));
            }

            if ui.button(RichText::new("💾 Guardar")).clicked() {
                result = Some(ModalResult::Save(state.to_profile()));
            }

            if ui.button(RichText::new("Cancelar")).clicked() {
                result = Some(ModalResult::Cancel);
            }
        });
    });

    result
}

pub fn render_new_folder_modal(
    ctx: &egui::Context,
    name: &mut String,
) -> Option<bool> {
    let mut result = None;

    egui::Window::new("📁 Nueva Carpeta")
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
                    .button(
                        RichText::new("Crear")
                            .strong()
                            .color(Color32::WHITE),
                    )
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
