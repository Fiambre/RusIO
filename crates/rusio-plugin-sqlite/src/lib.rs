//! SQLite databases: schema browser, SQL editor and a results grid.
mod engine;

use engine::{Engine, OpenOptions};
use rusio_plugin_api::egui::{self, Color32, RichText};
use rusio_plugin_api::serde_json;
use rusio_plugin_api::sql::SqlConsole;
use rusio_plugin_api::theme::Theme;
use rusio_plugin_api::{
    setting_i64, Category, ConnectRequest, Connected, Params, Plugin, ProtocolDef, ProtocolIcon, SettingField,
    SettingKind, WelcomeCard,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const PROTOCOL_ID: &str = "Sqlite";
const BADGE_COLOR: Color32 = Color32::from_rgb(45, 212, 191);

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SqliteParams {
    pub path: String,
    pub read_only: bool,
    pub create: bool,
}

impl SqliteParams {
    fn from_value(v: &Params) -> Result<Self, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("parámetros inválidos: {e}"))
    }

    fn to_value(&self) -> Params {
        serde_json::to_value(self).unwrap_or(Params::Null)
    }
}

pub struct SqlitePlugin;

impl Plugin for SqlitePlugin {
    fn protocol(&self) -> ProtocolDef {
        ProtocolDef {
            id: PROTOCOL_ID,
            name: "SQLite (archivo)",
            badge: "SQL",
            color: BADGE_COLOR,
            secrets: vec![],
            quick_connect: None,
            actions: vec![],
            welcome: Some(WelcomeCard {
                title: "Base de datos SQLite",
                subtitle: "Archivo .db / .sqlite",
            }),
            category: Category::Database,
            icon: ProtocolIcon::Logo(include_bytes!("../assets/sqlite.svg")),
            gateway: false,
            endpoint: None,
        }
    }

    fn default_params(&self) -> Params {
        SqliteParams::default().to_value()
    }

    fn summary(&self, params: &Params) -> String {
        SqliteParams::from_value(params).map(|p| p.path).unwrap_or_default()
    }

    fn edit_ui(&self, ui: &mut egui::Ui, params: &mut Params) {
        let theme = Theme::default();
        let mut p = SqliteParams::from_value(params).unwrap_or_default();

        ui.horizontal(|ui| {
            ui.label("Archivo:");
            ui.add(egui::TextEdit::singleline(&mut p.path).desired_width(300.0));
            if ui.button("Examinar...").clicked() {
                if let Some(f) = rfd::FileDialog::new()
                    .set_title("Base de datos SQLite")
                    .add_filter("SQLite", &["db", "sqlite", "sqlite3", "db3"])
                    .add_filter("Todos los archivos", &["*"])
                    .pick_file()
                {
                    p.path = f.to_string_lossy().into_owned();
                }
            }
        });
        ui.checkbox(&mut p.read_only, "Solo lectura (impide modificar los datos)");
        ui.add_enabled(
            !p.read_only,
            egui::Checkbox::new(&mut p.create, "Crear el archivo si no existe"),
        );
        ui.label(
            RichText::new("Abre el archivo directamente: no hay servidor ni contraseña.")
                .size(11.0)
                .color(theme.text_muted),
        );

        p.path = p.path.trim().to_string();
        *params = p.to_value();
    }

    fn settings_schema(&self) -> Vec<SettingField> {
        vec![
            SettingField {
                key: "max_rows",
                label: "Máximo de filas por consulta",
                help: "Un resultado más grande se corta a este número de filas.",
                kind: SettingKind::Int {
                    default: 1000,
                    min: 10,
                    max: 100_000,
                    suffix: " filas",
                },
            },
            SettingField {
                key: "busy_timeout_ms",
                label: "Espera si la base está bloqueada",
                help: "Cuánto esperar a que otro proceso libere el archivo antes de fallar.",
                kind: SettingKind::Int {
                    default: 5000,
                    min: 0,
                    max: 60_000,
                    suffix: " ms",
                },
            },
        ]
    }

    fn connect(&self, req: &ConnectRequest) -> Result<Connected, String> {
        let p = SqliteParams::from_value(req.params)?;
        let max_rows = setting_i64(req.settings, "max_rows", 1000).clamp(10, 100_000) as usize;
        let busy_timeout_ms = setting_i64(req.settings, "busy_timeout_ms", 5000).clamp(0, 60_000) as u64;

        let path = Path::new(&p.path);
        let conn = engine::open(
            path,
            &OpenOptions {
                read_only: p.read_only,
                create: p.create && !p.read_only,
                busy_timeout_ms,
            },
        )?;

        let ctx = req.ctx.clone();
        let engine = Engine::start(conn, max_rows, move || ctx.request_repaint());
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.path.clone());

        Ok(Connected {
            title,
            session: Box::new(SqlConsole::new(Box::new(engine), ("SQL", BADGE_COLOR))),
        })
    }
}
