//! MySQL / MariaDB servers: schema browser, SQL editor and a results grid (the shared SQL console).
mod engine;

use engine::{ConnectOptions, Engine, Ssl};
use rusio_plugin_api::egui::{self, Color32, RichText};
use rusio_plugin_api::serde_json;
use rusio_plugin_api::sql::SqlConsole;
use rusio_plugin_api::theme::Theme;
use rusio_plugin_api::{
    setting_i64, Category, ConnectRequest, Connected, Endpoint, Params, Plugin, ProtocolDef, ProtocolIcon,
    SecretField, SettingField, SettingKind, WelcomeCard,
};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_ID: &str = "Mysql";
const BADGE_COLOR: Color32 = Color32::from_rgb(242, 145, 17);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MysqlParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Kept in the OS credential store; blank means "not set".
    pub password: Option<String>,
    /// Blank = connect without a default database.
    pub database: String,
    /// `disable`, `prefer` or `require`.
    pub sslmode: String,
    pub read_only: bool,
    /// Lets the console switch between every database of the server.
    pub all_databases: bool,
}

impl Default for MysqlParams {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3306,
            user: "root".to_string(),
            password: None,
            database: String::new(),
            sslmode: "prefer".to_string(),
            read_only: false,
            all_databases: false,
        }
    }
}

impl MysqlParams {
    fn from_value(v: &Params) -> Result<Self, String> {
        let mut p: MysqlParams =
            serde_json::from_value(v.clone()).map_err(|e| format!("parámetros inválidos: {e}"))?;
        if p.password.as_deref() == Some("") {
            p.password = None;
        }
        Ok(p)
    }

    fn to_value(&self) -> Params {
        serde_json::to_value(self).unwrap_or(Params::Null)
    }
}

fn endpoint_get(params: &Params) -> Option<(String, u16)> {
    MysqlParams::from_value(params).ok().map(|p| (p.host, p.port))
}

fn endpoint_set(params: &Params, host: &str, port: u16) -> Params {
    match MysqlParams::from_value(params) {
        Ok(mut p) => {
            p.host = host.to_string();
            p.port = port;
            p.to_value()
        }
        Err(_) => params.clone(),
    }
}

pub struct MysqlPlugin;

impl Plugin for MysqlPlugin {
    fn protocol(&self) -> ProtocolDef {
        ProtocolDef {
            id: PROTOCOL_ID,
            name: "MySQL / MariaDB",
            badge: "MY",
            color: BADGE_COLOR,
            secrets: vec![SecretField { tag: "mysql-password", pointer: "/password" }],
            quick_connect: None,
            actions: vec![],
            welcome: Some(WelcomeCard {
                title: "MySQL / MariaDB",
                subtitle: "Servidor de base de datos",
            }),
            category: Category::Database,
            icon: ProtocolIcon::Logo(include_bytes!("../assets/mysql.svg")),
            gateway: false,
            endpoint: Some(Endpoint { get: endpoint_get, set: endpoint_set }),
        }
    }

    fn default_params(&self) -> Params {
        MysqlParams::default().to_value()
    }

    fn summary(&self, params: &Params) -> String {
        MysqlParams::from_value(params)
            .map(|p| {
                let db = if p.database.is_empty() { String::new() } else { format!("/{}", p.database) };
                format!("{}@{}:{}{}", p.user, p.host, p.port, db)
            })
            .unwrap_or_default()
    }

    fn edit_ui(&self, ui: &mut egui::Ui, params: &mut Params) {
        let theme = Theme::default();
        let mut p = MysqlParams::from_value(params).unwrap_or_default();

        ui.horizontal(|ui| {
            ui.label("Servidor:");
            ui.text_edit_singleline(&mut p.host);
            ui.label("Puerto:");
            let mut port = p.port.to_string();
            if ui.add(egui::TextEdit::singleline(&mut port).desired_width(60.0)).changed() {
                if let Ok(v) = port.parse::<u16>() {
                    p.port = v;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Usuario:");
            ui.text_edit_singleline(&mut p.user);
            ui.label("Base de datos (opcional):");
            ui.text_edit_singleline(&mut p.database);
        });
        let mut pw = p.password.clone().unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label("Contraseña:");
            ui.add(egui::TextEdit::singleline(&mut pw).password(true));
        });
        p.password = if pw.is_empty() { None } else { Some(pw) };

        ui.horizontal(|ui| {
            ui.label("Cifrado (TLS):");
            let label = |m: &str| match m {
                "disable" => "Desactivado",
                "require" => "Obligatorio",
                _ => "Si el servidor lo ofrece",
            };
            egui::ComboBox::from_id_salt("mysql_sslmode")
                .selected_text(label(&p.sslmode))
                .show_ui(ui, |ui| {
                    for m in ["prefer", "require", "disable"] {
                        ui.selectable_value(&mut p.sslmode, m.to_string(), label(m));
                    }
                });
        });
        ui.checkbox(&mut p.all_databases, "Mostrar todas las bases de datos (selector para cambiar de base)");
        ui.checkbox(&mut p.read_only, "Solo lectura (la sesión rechaza modificaciones)");
        ui.label(
            RichText::new("Con \"Obligatorio\" el cifrado es obligatorio pero el certificado no se verifica.")
                .size(11.0)
                .color(theme.text_muted),
        );

        p.host = p.host.trim().to_string();
        p.user = p.user.trim().to_string();
        p.database = p.database.trim().to_string();
        *params = p.to_value();
    }

    fn settings_schema(&self) -> Vec<SettingField> {
        vec![
            SettingField {
                key: "max_rows",
                label: "Máximo de filas por consulta",
                help: "Un resultado más grande se corta a este número de filas.",
                kind: SettingKind::Int { default: 1000, min: 10, max: 100_000, suffix: " filas" },
            },
            SettingField {
                key: "connect_timeout_secs",
                label: "Tiempo máximo para conectar",
                help: "Cuánto esperar al servidor antes de rendirse.",
                kind: SettingKind::Int { default: 10, min: 1, max: 120, suffix: " s" },
            },
            SettingField {
                key: "statement_timeout_secs",
                label: "Tiempo máximo por consulta",
                help: "El servidor aborta las consultas más lentas. 0 = sin límite.",
                kind: SettingKind::Int { default: 0, min: 0, max: 3600, suffix: " s" },
            },
        ]
    }

    fn connect(&self, req: &ConnectRequest) -> Result<Connected, String> {
        let p = MysqlParams::from_value(req.params)?;
        let opts = ConnectOptions {
            host: p.host.clone(),
            port: p.port,
            user: p.user.clone(),
            password: p.password.clone(),
            database: p.database.clone(),
            ssl: Ssl::parse(&p.sslmode),
            read_only: p.read_only,
            connect_timeout_secs: setting_i64(req.settings, "connect_timeout_secs", 10).clamp(1, 120) as u64,
            statement_timeout_secs: setting_i64(req.settings, "statement_timeout_secs", 0).clamp(0, 3600) as u64,
        };
        let max_rows = setting_i64(req.settings, "max_rows", 1000).clamp(10, 100_000) as usize;

        let conn = engine::connect(&opts)?;
        let ctx = req.ctx.clone();
        let engine = Engine::start(conn, opts, p.all_databases, max_rows, move || ctx.request_repaint());
        let title = if p.database.is_empty() { p.host.clone() } else { format!("{}@{}", p.database, p.host) };
        Ok(Connected {
            title,
            session: Box::new(SqlConsole::new(Box::new(engine), ("MY", BADGE_COLOR))),
        })
    }
}
