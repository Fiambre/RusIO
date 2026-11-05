//! The contract between the RusIO host app and its protocol plugins.
//!
//! A plugin is a crate that implements [`Plugin`]. The host knows nothing about SSH, VNC,
//! databases... it only talks to this trait: it asks the plugin for its connection form,
//! its settings, its quick launchers and, when the user connects, for a [`Session`] that
//! draws its own tab with egui.
//!
//! Connection parameters travel as `serde_json::Value`; each plugin (de)serializes them
//! into its own typed struct. Secrets are declared in [`ProtocolDef::secrets`] and are kept
//! out of `connections.json` by the host.

pub mod data;
pub mod icons;
pub mod sql;
pub mod sql_text;
pub mod term;
pub mod theme;

use crossbeam_channel::Sender;
use egui::Color32;
use serde_json::{Map, Value};
use std::sync::Arc;

pub use egui;
pub use rusio_core::config::SecretField;
pub use serde_json;

pub type Params = Value;
pub type SettingValues = Map<String, Value>;

// ---------------------------------------------------------------- protocol description

/// Static description of the protocol a plugin provides.
/// Where a protocol is grouped in the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Local,
    Remote,
    Database,
}

impl Category {
    /// Display order in the sidebar.
    pub const ALL: [Category; 3] = [Category::Local, Category::Remote, Category::Database];

    pub fn label(self) -> &'static str {
        match self {
            Category::Local => "Local",
            Category::Remote => "Remoto",
            Category::Database => "Bases de datos",
        }
    }
}

/// The picture shown next to a protocol's connections.
#[derive(Clone, Copy)]
pub enum ProtocolIcon {
    /// An SVG logo (the file's bytes, usually via `include_bytes!`).
    Logo(&'static [u8]),
    /// One of the painted icons, tinted with the protocol's color.
    Glyph(icons::Icon),
}

pub struct ProtocolDef {
    /// Key under which the protocol is stored in `connections.json` (e.g. `"Ssh"`).
    pub id: &'static str,
    /// Name shown in the "new connection" form.
    pub name: &'static str,
    /// Short label of the badge next to connections and tabs (e.g. `"SSH"`).
    pub badge: &'static str,
    pub color: Color32,
    pub category: Category,
    pub icon: ProtocolIcon,
    /// Fields of `params` that must go to the OS credential store.
    pub secrets: Vec<SecretField>,
    /// Lets the title bar's quick-connect box create a connection of this protocol.
    pub quick_connect: Option<QuickConnect>,
    /// Extra entries for a connection's context menu (e.g. "Conectar SFTP").
    pub actions: Vec<ProfileAction>,
    /// Card on the welcome screen that opens the "new connection" form for this protocol.
    pub welcome: Option<WelcomeCard>,
    /// True if connections of this protocol can serve as a gateway (bastion/jump host) for
    /// others: the plugin implements [`Plugin::open_tunnel`].
    pub gateway: bool,
    /// Present if connections of this protocol can be reached *through* a gateway: lets the
    /// host read and rewrite the network endpoint inside `params`.
    pub endpoint: Option<Endpoint>,
}

/// How the host reads and redirects the network target of a connection's params.
pub struct Endpoint {
    pub get: fn(&Params) -> Option<(String, u16)>,
    /// Same params, pointed at `host:port` (used to aim a connection at a local tunnel port).
    pub set: fn(&Params, &str, u16) -> Params,
}

/// A forwarded port. Dropping `guard` closes the tunnel.
pub struct Tunnel {
    /// `127.0.0.1` port that reaches the target through the gateway.
    pub local_port: u16,
    pub guard: Box<dyn std::any::Any + Send>,
}

pub struct QuickConnect {
    pub label: &'static str,
    pub hint: &'static str,
    /// Turns what the user typed into connection params, or `None` if it is not valid.
    pub parse: fn(&str) -> Option<Params>,
}

pub struct ProfileAction {
    pub id: &'static str,
    pub label: &'static str,
}

pub struct WelcomeCard {
    pub title: &'static str,
    pub subtitle: &'static str,
}

/// A one-click session with fixed params (e.g. "PowerShell").
pub struct QuickLaunch {
    pub label: String,
    pub subtitle: String,
    pub color: Color32,
    pub params: Params,
    /// Show it as a card on the welcome screen.
    pub featured: bool,
}

// ---------------------------------------------------------------- settings

/// A setting the plugin wants in the app's configuration panel. Values are stored by the
/// host under the plugin's protocol id and handed back in [`ConnectRequest::settings`].
pub struct SettingField {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: SettingKind,
}

pub enum SettingKind {
    Bool {
        default: bool,
    },
    Int {
        default: i64,
        min: i64,
        max: i64,
        suffix: &'static str,
    },
    Text {
        default: &'static str,
    },
    Choice {
        default: &'static str,
        options: &'static [(&'static str, &'static str)],
    },
}

impl SettingKind {
    pub fn default_value(&self) -> Value {
        match self {
            SettingKind::Bool { default } => Value::Bool(*default),
            SettingKind::Int { default, .. } => Value::from(*default),
            SettingKind::Text { default } | SettingKind::Choice { default, .. } => {
                Value::String((*default).to_string())
            }
        }
    }
}

pub fn setting_bool(values: &SettingValues, key: &str, default: bool) -> bool {
    values.get(key).and_then(Value::as_bool).unwrap_or(default)
}

pub fn setting_i64(values: &SettingValues, key: &str, default: i64) -> i64 {
    values.get(key).and_then(Value::as_i64).unwrap_or(default)
}

pub fn setting_str<'a>(values: &'a SettingValues, key: &str, default: &'a str) -> &'a str {
    values.get(key).and_then(Value::as_str).unwrap_or(default)
}

// ---------------------------------------------------------------- plugin

pub struct ConnectRequest<'a> {
    pub params: &'a Params,
    /// One of [`ProtocolDef::actions`]; `None` means the plain connection.
    pub action: Option<&'a str>,
    /// The plugin's current setting values (see [`Plugin::settings_schema`]).
    pub settings: &'a SettingValues,
    pub terminal_font_size: f32,
    /// Plugins call `ctx.request_repaint()` from their worker threads.
    pub ctx: &'a egui::Context,
}

pub struct Connected {
    /// Tab title.
    pub title: String,
    pub session: Box<dyn Session>,
}

pub trait Plugin: Send + Sync {
    fn protocol(&self) -> ProtocolDef;

    /// Params of a brand-new connection (pre-fills the form).
    fn default_params(&self) -> Params;

    /// One-line description of the target, shown under the connection name.
    fn summary(&self, params: &Params) -> String;

    /// Draws the protocol-specific part of the connection form and edits `params` in place.
    fn edit_ui(&self, ui: &mut egui::Ui, params: &mut Params);

    fn quick_launches(&self) -> Vec<QuickLaunch> {
        Vec::new()
    }

    /// Settings shown in the configuration panel under this plugin's name.
    fn settings_schema(&self) -> Vec<SettingField> {
        Vec::new()
    }

    /// Opens the session. May return before the connection is established: the session
    /// itself reports progress in its own tab.
    fn connect(&self, req: &ConnectRequest) -> Result<Connected, String>;

    /// Gateway role (see [`ProtocolDef::gateway`]): connects with `params` and forwards a
    /// local port to `target_host:target_port` as seen from the gateway. Blocks while
    /// connecting, so the host calls it from a worker thread.
    fn open_tunnel(
        &self,
        _params: &Params,
        _settings: &SettingValues,
        _target_host: &str,
        _target_port: u16,
    ) -> Result<Tunnel, String> {
        Err("este protocolo no puede actuar como puerta de enlace".to_string())
    }
}

// ---------------------------------------------------------------- sessions

/// What a session can ask of the host while it draws.
pub trait Host {
    /// Opens `file` in a text editor tab.
    fn open_text_editor(&mut self, file: Arc<dyn TextFile>);
    fn set_status(&mut self, msg: String);
}

/// A tab's content.
pub trait Session {
    /// Badge shown on the tab.
    fn badge(&self) -> (&'static str, Color32);
    fn is_alive(&self) -> bool;
    fn ui(&mut self, ui: &mut egui::Ui, host: &mut dyn Host);
    fn close(&mut self) {}
    /// Overrides the tab title (e.g. to mark unsaved changes).
    fn title(&self) -> Option<String> {
        None
    }
    /// True if closing the tab would lose work; the host asks for confirmation.
    fn is_dirty(&self) -> bool {
        false
    }
}

/// A remote text file a session lets the host edit (read/write run on the session's worker).
pub trait TextFile: Send + Sync {
    fn path(&self) -> String;
    fn is_alive(&self) -> bool;
    fn read(&self, reply: Sender<Result<Vec<u8>, String>>);
    fn write(&self, data: Vec<u8>, reply: Sender<Result<(), String>>);
}
