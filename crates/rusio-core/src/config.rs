use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::secrets;

/// A protocol-specific secret inside a connection's `params`.
/// `pointer` is a JSON pointer (RFC 6901) to a string (or null) value; `tag` names the
/// credential-store entry. Secret fields must be `String` or `Option<String>` where an
/// empty string means "not set".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretField {
    pub tag: &'static str,
    pub pointer: &'static str,
}

/// Secret fields per protocol id, supplied by the plugins at load time.
pub type SecretSchema = HashMap<String, Vec<SecretField>>;

/// Protocol id plus its plugin-defined parameters.
///
/// Serialized as a single-key map `{ "<id>": <params> }`, which is exactly how the old
/// `ProtocolConfig` enum was written, so existing connection files keep loading.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolData {
    pub kind: String,
    pub params: Value,
}

impl ProtocolData {
    pub fn new(kind: impl Into<String>, params: Value) -> Self {
        Self {
            kind: kind.into(),
            params,
        }
    }
}

impl Serialize for ProtocolData {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(Some(1))?;
        m.serialize_entry(&self.kind, &self.params)?;
        m.end()
    }
}

impl<'de> Deserialize<'de> for ProtocolData {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            Value::Object(map) if map.len() == 1 => {
                let (kind, params) = map.into_iter().next().unwrap();
                Ok(ProtocolData { kind, params })
            }
            _ => Err(de::Error::custom("protocol must be a single-key object")),
        }
    }
}

fn nonempty_str(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

impl ProtocolData {
    /// True if `params` carries a non-empty secret in memory.
    fn has_secrets(&self, fields: &[SecretField]) -> bool {
        fields.iter().any(|f| nonempty_str(self.params.pointer(f.pointer)))
    }

    /// Fills in secrets that are not in the file from the OS credential store.
    fn load_secrets(&mut self, id: Uuid, fields: &[SecretField]) {
        for f in fields {
            if nonempty_str(self.params.pointer(f.pointer)) {
                continue;
            }
            if let Some(slot) = self.params.pointer_mut(f.pointer) {
                if let Some(v) = secrets::get(id, f.tag) {
                    *slot = Value::String(v);
                }
            }
        }
    }

    /// Moves secrets into the OS store and blanks them in this (to-be-serialized) copy.
    /// Returns false if the store failed; the secret is then left in place.
    fn stash_secrets(&mut self, id: Uuid, fields: &[SecretField]) -> bool {
        let mut all_ok = true;
        for f in fields {
            let Some(slot) = self.params.pointer_mut(f.pointer) else {
                continue;
            };
            let Some(s) = slot.as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            match secrets::set(id, f.tag, s) {
                Ok(()) => *slot = Value::String(String::new()),
                Err(_) => all_ok = false,
            }
        }
        all_ok
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    pub folder_id: Option<Uuid>,
    pub protocol: ProtocolData,
    pub color_tag: Option<String>,
    pub description: String,
    /// Another connection (a gateway such as an SSH bastion) this one is reached through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<Uuid>,
    /// Key of a user-picked icon (see the plugin API's icon choices); `None` = the protocol's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: Uuid,
    pub name: String,
    pub expanded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigStore {
    pub folders: Vec<Folder>,
    pub connections: Vec<ConnectionProfile>,
    /// Where this store is persisted. Empty = never written (tests, fresh default).
    #[serde(skip)]
    pub path: PathBuf,
    /// True when the last save could not use the OS credential store, so secrets were
    /// written to the file instead.
    #[serde(skip)]
    pub secrets_unavailable: bool,
    /// Which fields of each protocol are secrets (provided by the plugins).
    #[serde(skip)]
    pub schema: SecretSchema,
}

impl Default for ConfigStore {
    fn default() -> Self {
        let local_folder_id = Uuid::new_v4();
        let servers_folder_id = Uuid::new_v4();

        let folders = vec![
            Folder {
                id: local_folder_id,
                name: "Terminales Locales".to_string(),
                expanded: true,
            },
            Folder {
                id: servers_folder_id,
                name: "Servidores Remotos".to_string(),
                expanded: true,
            },
        ];

        let local = |name: &str, shell: &str, color: &str, desc: &str| ConnectionProfile {
            id: Uuid::new_v4(),
            name: name.to_string(),
            folder_id: Some(local_folder_id),
            protocol: ProtocolData::new("Local", json!({ "shell": shell, "working_dir": null })),
            color_tag: Some(color.to_string()),
            description: desc.to_string(),
            via: None,
            icon: None,
        };

        let connections = vec![
            local(
                "PowerShell Windows",
                "PowerShell",
                "#38bdf8",
                "PowerShell nativo con ConPTY",
            ),
            local("WSL Linux Shell", "Wsl", "#a855f7", "Subsistema de Windows para Linux"),
            local(
                "Command Prompt (CMD)",
                "Cmd",
                "#94a3b8",
                "Símbolo del sistema de Windows",
            ),
            ConnectionProfile {
                id: Uuid::new_v4(),
                name: "Servidor SSH Demo".to_string(),
                folder_id: Some(servers_folder_id),
                protocol: ProtocolData::new(
                    "Ssh",
                    json!({
                        "host": "test.rebex.net",
                        "port": 22,
                        "username": "demo",
                        "auth": { "Password": "password" }
                    }),
                ),
                color_tag: Some("#10b981".to_string()),
                description: "Servidor SSH público de pruebas (Rebex)".to_string(),
                via: None,
                icon: None,
            },
            ConnectionProfile {
                id: Uuid::new_v4(),
                name: "VNC Localhost".to_string(),
                folder_id: Some(servers_folder_id),
                protocol: ProtocolData::new("Vnc", json!({ "host": "127.0.0.1", "port": 5900, "password": null })),
                color_tag: Some("#f97316".to_string()),
                description: "Escritorio remoto VNC en puerto 5900".to_string(),
                via: None,
                icon: None,
            },
        ];

        Self {
            folders,
            connections,
            path: PathBuf::new(),
            secrets_unavailable: false,
            schema: SecretSchema::new(),
        }
    }
}

impl ConfigStore {
    pub fn default_path() -> PathBuf {
        if let Some(base) = dirs::config_dir() {
            let dir = base.join("rusio");
            let legacy = base.join("pulento");
            // The project used to be called Pulento: carry its data over once.
            if !dir.exists() && legacy.exists() && fs::rename(&legacy, &dir).is_err() {
                let _ = fs::create_dir_all(&dir);
                if let Ok(entries) = fs::read_dir(&legacy) {
                    for e in entries.flatten() {
                        let _ = fs::copy(e.path(), dir.join(e.file_name()));
                    }
                }
            }
            let _ = fs::create_dir_all(&dir);
            dir.join("connections.json")
        } else {
            PathBuf::from("connections.json")
        }
    }

    pub fn load(path: PathBuf, schema: SecretSchema) -> Self {
        match Self::load_from(&path, &schema) {
            Ok(cfg) => cfg,
            Err(_) => {
                // Keep a copy of an unreadable file instead of silently overwriting it.
                if path.exists() {
                    let _ = fs::copy(&path, path.with_extension("json.bak"));
                }
                let mut cfg = Self::default();
                cfg.path = path;
                cfg.schema = schema;
                let _ = cfg.save();
                cfg
            }
        }
    }

    pub fn load_from(path: &Path, schema: &SecretSchema) -> Result<Self, String> {
        let data = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut cfg: ConfigStore = serde_json::from_str(&data).map_err(|e| format!("archivo inválido: {}", e))?;
        cfg.path = path.to_path_buf();
        cfg.schema = schema.clone();

        // Passwords found in the file are legacy plaintext: after loading, saving moves
        // them into the OS store and rewrites the file without them.
        let mut had_plaintext = false;
        for c in &mut cfg.connections {
            if let Some(fields) = schema.get(&c.protocol.kind) {
                had_plaintext |= c.protocol.has_secrets(fields);
                c.protocol.load_secrets(c.id, fields);
            }
        }
        if had_plaintext {
            let _ = cfg.save();
        }
        Ok(cfg)
    }

    pub fn save(&mut self) -> Result<(), std::io::Error> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        // Serialize a copy with secrets moved to the OS store. If the store is unavailable
        // (e.g. Linux without a Secret Service) they stay in the file rather than being lost.
        let mut out = self.clone();
        let mut all_ok = true;
        for c in &mut out.connections {
            if let Some(fields) = self.schema.get(&c.protocol.kind) {
                all_ok &= c.protocol.stash_secrets(c.id, fields);
            }
        }
        self.secrets_unavailable = !all_ok;

        let json = serde_json::to_string_pretty(&out).map_err(|e| std::io::Error::other(e))?;
        fs::write(&self.path, json)
    }

    /// Moves the store to `new_path`. If a valid file already exists there it is loaded
    /// (returns `true`); otherwise the current data is written there (returns `false`).
    /// The old file is left untouched.
    pub fn switch_to(&mut self, new_path: PathBuf) -> Result<bool, String> {
        if new_path == self.path {
            return Ok(false);
        }
        if new_path.exists() {
            let cfg = Self::load_from(&new_path, &self.schema)?;
            *self = cfg;
            Ok(true)
        } else {
            let old = std::mem::replace(&mut self.path, new_path);
            if let Err(e) = self.save() {
                self.path = old;
                return Err(e.to_string());
            }
            Ok(false)
        }
    }

    fn tags_for(&self, kind: &str) -> Vec<&'static str> {
        self.schema
            .get(kind)
            .map(|fs| fs.iter().map(|f| f.tag).collect())
            .unwrap_or_default()
    }

    pub fn add_folder(&mut self, name: String) -> Uuid {
        let id = Uuid::new_v4();
        self.folders.push(Folder {
            id,
            name,
            expanded: true,
        });
        let _ = self.save();
        id
    }

    pub fn remove_folder(&mut self, id: Uuid) {
        self.folders.retain(|f| f.id != id);
        // Move connections in that folder to root
        for c in &mut self.connections {
            if c.folder_id == Some(id) {
                c.folder_id = None;
            }
        }
        let _ = self.save();
    }

    pub fn add_connection(&mut self, profile: ConnectionProfile) {
        self.connections.push(profile);
        let _ = self.save();
    }

    pub fn update_connection(&mut self, profile: ConnectionProfile) {
        if let Some(pos) = self.connections.iter().position(|c| c.id == profile.id) {
            // Drop stored secrets first so clearing a password in the editor really clears it;
            // save() re-stores whatever the new profile still has. Both the old and the new
            // protocol may have stored something.
            let mut tags = self.tags_for(&self.connections[pos].protocol.kind);
            tags.extend(self.tags_for(&profile.protocol.kind));
            secrets::delete_all(profile.id, &tags);
            self.connections[pos] = profile;
            let _ = self.save();
        }
    }

    pub fn remove_connection(&mut self, id: Uuid) {
        if let Some(c) = self.connections.iter().find(|c| c.id == id) {
            let tags = self.tags_for(&c.protocol.kind);
            secrets::delete_all(id, &tags);
        }
        self.connections.retain(|c| c.id != id);
        // Connections that went through the deleted one now connect directly.
        for c in &mut self.connections {
            if c.via == Some(id) {
                c.via = None;
            }
        }
        let _ = self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> SecretSchema {
        let mut s = SecretSchema::new();
        s.insert(
            "Ssh".into(),
            vec![
                SecretField {
                    tag: "ssh-password",
                    pointer: "/auth/Password",
                },
                SecretField {
                    tag: "key-passphrase",
                    pointer: "/auth/KeyFile/passphrase",
                },
            ],
        );
        s
    }

    #[test]
    fn default_config_has_each_protocol() {
        let config = ConfigStore::default();
        assert!(!config.folders.is_empty());
        for kind in ["Ssh", "Local", "Vnc"] {
            assert!(
                config.connections.iter().any(|c| c.protocol.kind == kind),
                "missing {kind}"
            );
        }
    }

    #[test]
    fn add_remove_folder() {
        let mut config = ConfigStore::default();
        let folder_id = config.add_folder("Laboratorio".to_string());
        assert!(config
            .folders
            .iter()
            .any(|f| f.id == folder_id && f.name == "Laboratorio"));

        config.remove_folder(folder_id);
        assert!(!config.folders.iter().any(|f| f.id == folder_id));
    }

    #[test]
    fn via_is_optional_and_omitted_when_unset() {
        // Files written before tunnels existed have no "via" key.
        let legacy = r#"{"id":"11111111-1111-1111-1111-111111111111","name":"x","folder_id":null,
            "protocol":{"Vnc":{"host":"h","port":1,"password":null}},"color_tag":null,"description":""}"#;
        let mut p: ConnectionProfile = serde_json::from_str(legacy).unwrap();
        assert_eq!(p.via, None);
        assert!(!serde_json::to_string(&p).unwrap().contains("via"));

        let gw = Uuid::new_v4();
        p.via = Some(gw);
        let back: ConnectionProfile = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.via, Some(gw));
    }

    #[test]
    fn deleting_a_gateway_unlinks_its_dependents() {
        let mut config = ConfigStore::default();
        let gw = config.connections[3].id;
        let dep = config.connections[4].id;
        config.connections[4].via = Some(gw);

        config.remove_connection(gw);
        let dep = config.connections.iter().find(|c| c.id == dep).unwrap();
        assert_eq!(dep.via, None);
    }

    #[test]
    fn protocol_data_matches_legacy_enum_format() {
        // The previous `ProtocolConfig` enum was written externally tagged.
        let legacy = r#"{"Ssh":{"host":"h","port":22,"username":"u","auth":{"Password":"x"}}}"#;
        let p: ProtocolData = serde_json::from_str(legacy).unwrap();
        assert_eq!(p.kind, "Ssh");
        assert_eq!(p.params["host"], "h");
        assert_eq!(serde_json::to_string(&p).unwrap(), legacy);

        assert!(serde_json::from_str::<ProtocolData>(r#"{"A":1,"B":2}"#).is_err());
    }

    #[test]
    fn secret_pointers_find_and_blank_only_present_fields() {
        let fields = schema().remove("Ssh").unwrap();
        let mut p = ProtocolData::new(
            "Ssh",
            json!({ "auth": { "KeyFile": { "path": "k", "passphrase": null } } }),
        );
        // Password variant absent and passphrase null: nothing to move.
        assert!(!p.has_secrets(&fields));

        p.params["auth"]["KeyFile"]["passphrase"] = json!("pp");
        assert!(p.has_secrets(&fields));
    }
}
