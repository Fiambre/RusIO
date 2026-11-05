//! MongoDB servers: collection browser, a mongo-shell style query editor and a results grid
//! (the shared console, with documents flattened into columns).
mod engine;
mod shell;

use engine::{ConnectOptions, Engine};
use rusio_plugin_api::egui::{self, Color32, RichText};
use rusio_plugin_api::serde_json;
use rusio_plugin_api::sql::SqlConsole;
use rusio_plugin_api::theme::Theme;
use rusio_plugin_api::{
    setting_i64, Category, ConnectRequest, Connected, Endpoint, Params, Plugin, ProtocolDef, ProtocolIcon, SecretField,
    SettingField, SettingKind, WelcomeCard,
};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_ID: &str = "Mongodb";
const BADGE_COLOR: Color32 = Color32::from_rgb(77, 179, 97);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MongoParams {
    pub host: String,
    pub port: u16,
    /// Blank = no authentication.
    pub user: String,
    /// Kept in the OS credential store; blank means "not set".
    pub password: Option<String>,
    pub auth_source: String,
    /// Blank = pick one from the selector.
    pub database: String,
    pub tls: bool,
    pub read_only: bool,
    /// `mongodb://...` connection string; replaces host, port and credentials. It may carry a
    /// password, so it lives in the credential store too. Blank means "not set".
    pub uri: Option<String>,
}

impl Default for MongoParams {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 27017,
            user: String::new(),
            password: None,
            auth_source: "admin".to_string(),
            database: String::new(),
            tls: false,
            read_only: false,
            uri: None,
        }
    }
}

impl MongoParams {
    fn from_value(v: &Params) -> Result<Self, String> {
        let mut p: MongoParams = serde_json::from_value(v.clone()).map_err(|e| format!("parámetros inválidos: {e}"))?;
        if p.password.as_deref() == Some("") {
            p.password = None;
        }
        if p.uri.as_deref().is_some_and(|u| u.trim().is_empty()) {
            p.uri = None;
        }
        Ok(p)
    }

    fn to_value(&self) -> Params {
        serde_json::to_value(self).unwrap_or(Params::Null)
    }
}

/// Only plain host/port connections can be pointed at a tunnel.
fn endpoint_get(params: &Params) -> Option<(String, u16)> {
    MongoParams::from_value(params)
        .ok()
        .filter(|p| p.uri.is_none())
        .map(|p| (p.host, p.port))
}

fn endpoint_set(params: &Params, host: &str, port: u16) -> Params {
    match MongoParams::from_value(params) {
        Ok(mut p) => {
            p.host = host.to_string();
            p.port = port;
            p.to_value()
        }
        Err(_) => params.clone(),
    }
}

pub struct MongoPlugin;

impl Plugin for MongoPlugin {
    fn protocol(&self) -> ProtocolDef {
        ProtocolDef {
            id: PROTOCOL_ID,
            name: "MongoDB",
            badge: "MG",
            color: BADGE_COLOR,
            secrets: vec![
                SecretField {
                    tag: "mongo-password",
                    pointer: "/password",
                },
                SecretField {
                    tag: "mongo-uri",
                    pointer: "/uri",
                },
            ],
            quick_connect: None,
            actions: vec![],
            welcome: Some(WelcomeCard {
                title: "MongoDB",
                subtitle: "Base de datos de documentos",
            }),
            category: Category::Database,
            icon: ProtocolIcon::Logo(include_bytes!("../assets/mongodb.svg")),
            gateway: false,
            endpoint: Some(Endpoint {
                get: endpoint_get,
                set: endpoint_set,
            }),
        }
    }

    fn default_params(&self) -> Params {
        MongoParams::default().to_value()
    }

    fn summary(&self, params: &Params) -> String {
        MongoParams::from_value(params)
            .map(|p| {
                if p.uri.is_some() {
                    return "cadena de conexión".to_string();
                }
                let user = if p.user.is_empty() {
                    String::new()
                } else {
                    format!("{}@", p.user)
                };
                let db = if p.database.is_empty() {
                    String::new()
                } else {
                    format!("/{}", p.database)
                };
                format!("{user}{}:{}{db}", p.host, p.port)
            })
            .unwrap_or_default()
    }

    fn edit_ui(&self, ui: &mut egui::Ui, params: &mut Params) {
        let theme = Theme::default();
        let mut p = MongoParams::from_value(params).unwrap_or_default();

        let mut uri = p.uri.clone().unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label("Cadena de conexión (opcional):");
            ui.add(egui::TextEdit::singleline(&mut uri).password(true).desired_width(260.0));
        });
        p.uri = if uri.trim().is_empty() {
            None
        } else {
            Some(uri.trim().to_string())
        };
        let using_uri = p.uri.is_some();
        ui.label(
            RichText::new(
                "Si la rellenas, reemplaza servidor, puerto, usuario, contraseña y TLS (y no admite túnel SSH).",
            )
            .size(11.0)
            .color(theme.text_muted),
        );
        ui.add_space(4.0);

        ui.add_enabled_ui(!using_uri, |ui| {
            ui.horizontal(|ui| {
                ui.label("Servidor:");
                ui.text_edit_singleline(&mut p.host);
                ui.label("Puerto:");
                let mut port = p.port.to_string();
                if ui
                    .add(egui::TextEdit::singleline(&mut port).desired_width(60.0))
                    .changed()
                {
                    if let Ok(v) = port.parse::<u16>() {
                        p.port = v;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Usuario (vacío = sin autenticación):");
                ui.text_edit_singleline(&mut p.user);
            });
            let mut pw = p.password.clone().unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label("Contraseña:");
                ui.add(egui::TextEdit::singleline(&mut pw).password(true));
                ui.label("Base de autenticación:");
                ui.add(egui::TextEdit::singleline(&mut p.auth_source).desired_width(80.0));
            });
            p.password = if pw.is_empty() { None } else { Some(pw) };
            ui.checkbox(&mut p.tls, "Cifrado TLS (el certificado no se verifica)");
        });
        ui.horizontal(|ui| {
            ui.label("Base de datos (opcional):");
            ui.text_edit_singleline(&mut p.database);
        });
        ui.checkbox(&mut p.read_only, "Solo lectura (rechaza escrituras y comandos)");

        p.host = p.host.trim().to_string();
        p.user = p.user.trim().to_string();
        p.database = p.database.trim().to_string();
        p.auth_source = p.auth_source.trim().to_string();
        *params = p.to_value();
    }

    fn settings_schema(&self) -> Vec<SettingField> {
        vec![
            SettingField {
                key: "max_rows",
                label: "Máximo de documentos por consulta",
                help: "Un resultado más grande se corta a este número de documentos.",
                kind: SettingKind::Int { default: 1000, min: 10, max: 100_000, suffix: " docs" },
            },
            SettingField {
                key: "connect_timeout_secs",
                label: "Tiempo máximo para conectar",
                help: "Cuánto esperar al servidor antes de rendirse.",
                kind: SettingKind::Int { default: 10, min: 1, max: 120, suffix: " s" },
            },
            SettingField {
                key: "query_timeout_secs",
                label: "Tiempo máximo por consulta",
                help: "El servidor aborta las consultas más lentas (find, aggregate y distinct). 0 = sin límite. MongoDB no permite cancelar a mano.",
                kind: SettingKind::Int { default: 0, min: 0, max: 3600, suffix: " s" },
            },
        ]
    }

    fn connect(&self, req: &ConnectRequest) -> Result<Connected, String> {
        let p = MongoParams::from_value(req.params)?;
        let opts = ConnectOptions {
            uri: p.uri.clone(),
            host: p.host.clone(),
            port: p.port,
            user: p.user.clone(),
            password: p.password.clone(),
            auth_source: p.auth_source.clone(),
            database: p.database.clone(),
            tls: p.tls,
            read_only: p.read_only,
            connect_timeout_secs: setting_i64(req.settings, "connect_timeout_secs", 10).clamp(1, 120) as u64,
            query_timeout_secs: setting_i64(req.settings, "query_timeout_secs", 0).clamp(0, 3600) as u64,
        };
        let max_rows = setting_i64(req.settings, "max_rows", 1000).clamp(10, 100_000) as usize;

        let client = engine::connect(&opts)?;
        let ctx = req.ctx.clone();
        let engine = Engine::start(client, opts, max_rows, move || ctx.request_repaint());
        let title = if p.uri.is_some() {
            "MongoDB".to_string()
        } else if p.database.is_empty() {
            p.host.clone()
        } else {
            format!("{}@{}", p.database, p.host)
        };
        Ok(Connected {
            title,
            session: Box::new(SqlConsole::new(Box::new(engine), ("MG", BADGE_COLOR))),
        })
    }
}
