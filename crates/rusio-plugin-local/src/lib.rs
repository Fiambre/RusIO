//! Local terminals (PowerShell, CMD, WSL, Git Bash) through a pseudo-terminal.
mod local_session;

use parking_lot::Mutex;
use rusio_plugin_api::egui;
use rusio_plugin_api::icons::Icon;
use rusio_plugin_api::serde_json::{self, json};
use rusio_plugin_api::term::{SharedTerminal, TerminalEmulator, TerminalSession};
use rusio_plugin_api::theme::Theme;
use rusio_plugin_api::{Category, ConnectRequest, Connected, Params, Plugin, ProtocolDef, ProtocolIcon, QuickLaunch};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use local_session::LocalSession;

pub const PROTOCOL_ID: &str = "Local";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LocalShellType {
    PowerShell,
    Cmd,
    Wsl,
    GitBash,
    Custom(String),
}

impl LocalShellType {
    pub fn display_name(&self) -> &'static str {
        match self {
            LocalShellType::PowerShell => "PowerShell",
            LocalShellType::Cmd => "CMD (Command Prompt)",
            LocalShellType::Wsl => "WSL (Linux)",
            LocalShellType::GitBash => "Git Bash",
            LocalShellType::Custom(_) => "Custom Shell",
        }
    }

    pub fn to_command_args(&self) -> (String, Vec<String>) {
        match self {
            LocalShellType::PowerShell => ("powershell.exe".to_string(), vec!["-NoLogo".to_string()]),
            LocalShellType::Cmd => ("cmd.exe".to_string(), vec![]),
            LocalShellType::Wsl => ("wsl.exe".to_string(), vec![]),
            LocalShellType::GitBash => ("bash.exe".to_string(), vec!["--login".to_string(), "-i".to_string()]),
            LocalShellType::Custom(cmd) => (cmd.clone(), vec![]),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalParams {
    pub shell: LocalShellType,
    pub working_dir: Option<String>,
}

impl LocalParams {
    fn from_value(v: &Params) -> Result<Self, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("parámetros inválidos: {e}"))
    }

    fn to_value(&self) -> Params {
        serde_json::to_value(self).unwrap_or(Params::Null)
    }
}

pub struct LocalPlugin;

impl Plugin for LocalPlugin {
    fn protocol(&self) -> ProtocolDef {
        ProtocolDef {
            id: PROTOCOL_ID,
            name: "Local (PowerShell/WSL/CMD)",
            badge: "LOCAL",
            color: Theme::default().accent_local,
            category: Category::Local,
            icon: ProtocolIcon::Glyph(Icon::Terminal),
            secrets: vec![],
            quick_connect: None,
            actions: vec![],
            welcome: None,
            gateway: false,
            endpoint: None,
        }
    }

    fn default_params(&self) -> Params {
        LocalParams {
            shell: LocalShellType::PowerShell,
            working_dir: None,
        }
        .to_value()
    }

    fn summary(&self, params: &Params) -> String {
        LocalParams::from_value(params)
            .map(|p| p.shell.display_name().to_string())
            .unwrap_or_default()
    }

    fn edit_ui(&self, ui: &mut egui::Ui, params: &mut Params) {
        let mut p = LocalParams::from_value(params).unwrap_or(LocalParams {
            shell: LocalShellType::PowerShell,
            working_dir: None,
        });

        ui.horizontal(|ui| {
            ui.label("Shell:");
            egui::ComboBox::from_id_salt("local_shell_select")
                .selected_text(p.shell.display_name())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut p.shell, LocalShellType::PowerShell, "PowerShell");
                    ui.selectable_value(&mut p.shell, LocalShellType::Wsl, "WSL (Linux Bash)");
                    ui.selectable_value(&mut p.shell, LocalShellType::Cmd, "Command Prompt (CMD)");
                    ui.selectable_value(&mut p.shell, LocalShellType::GitBash, "Git Bash");
                });
        });

        let mut cwd = p.working_dir.clone().unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label("Directorio de inicio (opcional):");
            ui.text_edit_singleline(&mut cwd);
        });
        let cwd = cwd.trim();
        p.working_dir = if cwd.is_empty() { None } else { Some(cwd.to_string()) };

        *params = p.to_value();
    }

    fn quick_launches(&self) -> Vec<QuickLaunch> {
        let theme = Theme::default();
        let mk = |shell: &str, label: &str, subtitle: &str, color, featured| QuickLaunch {
            label: label.to_string(),
            subtitle: subtitle.to_string(),
            color,
            params: json!({ "shell": shell, "working_dir": null }),
            featured,
        };
        vec![
            mk(
                "PowerShell",
                "PowerShell",
                "Terminal local ConPTY",
                theme.accent_primary,
                true,
            ),
            mk("Wsl", "WSL", "Ubuntu / Debian Bash", theme.accent_local, true),
            mk("Cmd", "CMD", "Símbolo del sistema", theme.text_secondary, false),
        ]
    }

    fn connect(&self, req: &ConnectRequest) -> Result<Connected, String> {
        let p = LocalParams::from_value(req.params)?;
        let (cols, rows) = (100u16, 30u16);
        let emulator: SharedTerminal = Arc::new(Mutex::new(TerminalEmulator::new(cols, rows)));

        let ctx = req.ctx.clone();
        let session = LocalSession::spawn(
            p.shell.clone(),
            p.working_dir.clone(),
            cols,
            rows,
            emulator.clone(),
            move || ctx.request_repaint(),
        )
        .map_err(|e| format!("No se pudo iniciar la terminal local: {e}"))?;

        let badge = ("LOCAL", Theme::default().accent_local);
        Ok(Connected {
            title: p.shell.display_name().to_string(),
            session: Box::new(TerminalSession::new(
                Box::new(session),
                emulator,
                req.terminal_font_size,
                badge,
            )),
        })
    }
}
