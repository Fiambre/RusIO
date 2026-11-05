//! SSH terminals and SFTP file browsing (with remote text editing through the host).
mod sftp;
mod sftp_view;
mod ssh_session;
mod tunnel;

use parking_lot::Mutex;
use rusio_plugin_api::egui::{self, RichText};
use rusio_plugin_api::icons::Icon;
use rusio_plugin_api::serde_json;
use rusio_plugin_api::term::{SharedTerminal, TerminalEmulator, TerminalSession};
use rusio_plugin_api::theme::Theme;
use rusio_plugin_api::{
    setting_i64, Category, ConnectRequest, Connected, Endpoint, Params, Plugin, ProfileAction, ProtocolDef,
    ProtocolIcon, QuickConnect, SecretField, SettingField, SettingKind, SettingValues, Tunnel, WelcomeCard,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use sftp::{SftpOptions, SftpSession};
use ssh_session::SshSession;

pub const PROTOCOL_ID: &str = "Ssh";
const ACTION_SFTP: &str = "sftp";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SshAuth {
    Password(String),
    KeyFile { path: String, passphrase: Option<String> },
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
}

impl SshParams {
    fn from_value(v: &Params) -> Result<Self, String> {
        let mut p: SshParams = serde_json::from_value(v.clone()).map_err(|e| format!("parámetros inválidos: {e}"))?;
        // An empty passphrase (blanked by the secret store) means "none".
        if let SshAuth::KeyFile { passphrase, .. } = &mut p.auth {
            if passphrase.as_deref() == Some("") {
                *passphrase = None;
            }
        }
        Ok(p)
    }

    fn to_value(&self) -> Params {
        serde_json::to_value(self).unwrap_or(Params::Null)
    }

    fn defaults() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 22,
            username: "root".to_string(),
            auth: SshAuth::Password(String::new()),
        }
    }
}

fn endpoint_get(params: &Params) -> Option<(String, u16)> {
    SshParams::from_value(params).ok().map(|p| (p.host, p.port))
}

fn endpoint_set(params: &Params, host: &str, port: u16) -> Params {
    match SshParams::from_value(params) {
        Ok(mut p) => {
            p.host = host.to_string();
            p.port = port;
            p.to_value()
        }
        Err(_) => params.clone(),
    }
}

fn connect_timeout(settings: &SettingValues) -> Duration {
    Duration::from_secs(setting_i64(settings, "connect_timeout_secs", 10).clamp(2, 120) as u64)
}

fn sftp_options(settings: &SettingValues) -> SftpOptions {
    SftpOptions {
        connect_timeout: connect_timeout(settings),
        keepalive_secs: setting_i64(settings, "sftp_keepalive_secs", 30).clamp(0, 300) as u32,
        edit_max_bytes: setting_i64(settings, "sftp_edit_max_mb", 2).clamp(1, 50) as u64 * 1024 * 1024,
    }
}

/// `usuario@host:puerto` (user defaults to root, port to 22).
fn parse_quick_connect(raw: &str) -> Option<Params> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (username, rest) = match raw.split_once('@') {
        Some((u, r)) => (u.to_string(), r.to_string()),
        None => ("root".to_string(), raw.to_string()),
    };
    let (host, port) = match rest.split_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(22)),
        None => (rest, 22),
    };
    if host.is_empty() {
        return None;
    }
    Some(
        SshParams {
            host,
            port,
            username,
            auth: SshAuth::Password(String::new()),
        }
        .to_value(),
    )
}

pub struct SshPlugin;

impl Plugin for SshPlugin {
    fn protocol(&self) -> ProtocolDef {
        ProtocolDef {
            id: PROTOCOL_ID,
            name: "SSH (Terminal Remoto)",
            badge: "SSH",
            color: Theme::default().accent_ssh,
            secrets: vec![
                SecretField {
                    tag: "ssh-password",
                    pointer: "/auth/Password",
                },
                SecretField {
                    tag: "key-passphrase",
                    pointer: "/auth/KeyFile/passphrase",
                },
            ],
            quick_connect: Some(QuickConnect {
                label: "SSH",
                hint: "usuario@host:puerto",
                parse: parse_quick_connect,
            }),
            actions: vec![ProfileAction {
                id: ACTION_SFTP,
                label: "Conectar SFTP",
            }],
            welcome: Some(WelcomeCard {
                title: "Servidor SSH",
                subtitle: "Conexión remota segura",
            }),
            category: Category::Remote,
            icon: ProtocolIcon::Glyph(Icon::Server),
            gateway: true,
            endpoint: Some(Endpoint {
                get: endpoint_get,
                set: endpoint_set,
            }),
        }
    }

    fn default_params(&self) -> Params {
        SshParams::defaults().to_value()
    }

    fn summary(&self, params: &Params) -> String {
        SshParams::from_value(params)
            .map(|p| format!("{}@{}:{}", p.username, p.host, p.port))
            .unwrap_or_default()
    }

    fn edit_ui(&self, ui: &mut egui::Ui, params: &mut Params) {
        let theme = Theme::default();
        let mut p = SshParams::from_value(params).unwrap_or_else(|_| SshParams::defaults());

        ui.horizontal(|ui| {
            ui.label("Host / IP:");
            ui.text_edit_singleline(&mut p.host);
            ui.label("Puerto:");
            let mut port_str = p.port.to_string();
            if ui
                .add(egui::TextEdit::singleline(&mut port_str).desired_width(60.0))
                .changed()
            {
                if let Ok(port) = port_str.parse::<u16>() {
                    p.port = port;
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("Usuario:");
            ui.text_edit_singleline(&mut p.username);
        });

        ui.label(RichText::new("Método de Autenticación:").strong().size(11.0));
        let mut kind = match p.auth {
            SshAuth::Password(_) => 0,
            SshAuth::KeyFile { .. } => 1,
            SshAuth::Agent => 2,
        };
        let before = kind;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut kind, 0, "Contraseña");
            ui.selectable_value(&mut kind, 1, "Clave Privada (Keyfile)");
            ui.selectable_value(&mut kind, 2, "SSH Agent");
        });
        if kind != before {
            p.auth = match kind {
                0 => SshAuth::Password(String::new()),
                1 => SshAuth::KeyFile {
                    path: String::new(),
                    passphrase: None,
                },
                _ => SshAuth::Agent,
            };
        }

        match &mut p.auth {
            SshAuth::Password(pw) => {
                ui.horizontal(|ui| {
                    ui.label("Contraseña:");
                    ui.add(egui::TextEdit::singleline(pw).password(true));
                });
            }
            SshAuth::KeyFile { path, passphrase } => {
                ui.horizontal(|ui| {
                    ui.label("Ruta de clave:");
                    ui.text_edit_singleline(path);
                });
                let mut pp = passphrase.clone().unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.label("Passphrase (opcional):");
                    ui.add(egui::TextEdit::singleline(&mut pp).password(true));
                });
                *passphrase = if pp.is_empty() { None } else { Some(pp) };
            }
            SshAuth::Agent => {
                ui.label(
                    RichText::new("Se utilizarán las claves cargadas en el agente SSH de Windows/Pageant.")
                        .size(11.0)
                        .color(theme.text_secondary),
                );
            }
        }

        p.host = p.host.trim().to_string();
        p.username = p.username.trim().to_string();
        *params = p.to_value();
    }

    fn settings_schema(&self) -> Vec<SettingField> {
        vec![
            SettingField {
                key: "connect_timeout_secs",
                label: "Tiempo de espera de conexión",
                help: "Segundos que se espera al abrir la conexión (SSH y SFTP).",
                kind: SettingKind::Int {
                    default: 10,
                    min: 2,
                    max: 120,
                    suffix: " s",
                },
            },
            SettingField {
                key: "sftp_keepalive_secs",
                label: "Keepalive de SFTP y túneles",
                help:
                    "Cada cuántos segundos se envía un latido para que el servidor no cierre la sesión. 0 lo desactiva.",
                kind: SettingKind::Int {
                    default: 30,
                    min: 0,
                    max: 300,
                    suffix: " s",
                },
            },
            SettingField {
                key: "sftp_edit_max_mb",
                label: "Tamaño máximo para editar archivos",
                help: "Los archivos remotos más grandes no se abren en el editor de texto.",
                kind: SettingKind::Int {
                    default: 2,
                    min: 1,
                    max: 50,
                    suffix: " MB",
                },
            },
        ]
    }

    fn connect(&self, req: &ConnectRequest) -> Result<Connected, String> {
        let p = SshParams::from_value(req.params)?;
        let timeout = connect_timeout(req.settings);
        let ctx = req.ctx.clone();
        let repaint = move || ctx.request_repaint();

        match req.action {
            None => {
                let (cols, rows) = (100u16, 30u16);
                let emulator: SharedTerminal = Arc::new(Mutex::new(TerminalEmulator::new(cols, rows)));
                let title = format!("{}@{}:{}", p.username, p.host, p.port);
                let session = SshSession::connect(
                    p.host,
                    p.port,
                    p.username,
                    p.auth,
                    cols,
                    rows,
                    timeout,
                    emulator.clone(),
                    repaint,
                )
                .map_err(|e| format!("Error SSH: {e}"))?;

                let badge = ("SSH", Theme::default().accent_ssh);
                Ok(Connected {
                    title,
                    session: Box::new(TerminalSession::new(
                        Box::new(session),
                        emulator,
                        req.terminal_font_size,
                        badge,
                    )),
                })
            }
            Some(ACTION_SFTP) => {
                let title = format!("SFTP {}@{}", p.username, p.host);
                let opts = sftp_options(req.settings);
                let session = SftpSession::connect(p.host, p.port, p.username, p.auth, opts, repaint)
                    .map_err(|e| format!("Error SFTP: {e}"))?;
                Ok(Connected {
                    title,
                    session: Box::new(session),
                })
            }
            Some(other) => Err(format!("acción desconocida: {other}")),
        }
    }

    fn open_tunnel(
        &self,
        params: &Params,
        settings: &SettingValues,
        target_host: &str,
        target_port: u16,
    ) -> Result<Tunnel, String> {
        tunnel::open_ssh_tunnel(params, &sftp_options(settings), target_host, target_port)
    }
}
