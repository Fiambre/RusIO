use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::config::ConfigStore;

/// App preferences. Stored in a fixed place (next to the default connections file)
/// because it has to be found before we know where the connections live.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `None` = use the default location.
    pub connections_path: Option<PathBuf>,
    pub terminal_font_size: f32,
    pub editor_font_size: f32,
    /// Per-plugin settings, keyed by protocol id then by the plugin's own setting key.
    pub plugins: BTreeMap<String, Map<String, Value>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            connections_path: None,
            terminal_font_size: 14.0,
            editor_font_size: 13.0,
            plugins: BTreeMap::new(),
        }
    }
}

impl Settings {
    fn path() -> PathBuf {
        ConfigStore::default_path().with_file_name("settings.json")
    }

    pub fn load() -> Self {
        fs::read_to_string(Self::path())
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(Self::path(), json);
        }
    }

    pub fn connections_path_or_default(&self) -> PathBuf {
        self.connections_path.clone().unwrap_or_else(ConfigStore::default_path)
    }

    /// Current values of a plugin's settings (empty if it never saved any).
    pub fn plugin_values(&self, plugin_id: &str) -> Map<String, Value> {
        self.plugins.get(plugin_id).cloned().unwrap_or_default()
    }

    pub fn set_plugin_value(&mut self, plugin_id: &str, key: &str, value: Value) {
        self.plugins
            .entry(plugin_id.to_string())
            .or_default()
            .insert(key.to_string(), value);
    }
}
