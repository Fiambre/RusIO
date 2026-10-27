use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SshAuth {
    Password(String),
    KeyFile {
        path: String,
        passphrase: Option<String>,
    },
    Agent,
}

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
            LocalShellType::PowerShell => (
                "powershell.exe".to_string(),
                vec!["-NoLogo".to_string()],
            ),
            LocalShellType::Cmd => ("cmd.exe".to_string(), vec![]),
            LocalShellType::Wsl => ("wsl.exe".to_string(), vec![]),
            LocalShellType::GitBash => (
                "bash.exe".to_string(),
                vec!["--login".to_string(), "-i".to_string()],
            ),
            LocalShellType::Custom(cmd) => (cmd.clone(), vec![]),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProtocolConfig {
    Ssh {
        host: String,
        port: u16,
        username: String,
        auth: SshAuth,
    },
    Local {
        shell: LocalShellType,
        working_dir: Option<String>,
    },
    Vnc {
        host: String,
        port: u16,
        password: Option<String>,
    },
}

impl ProtocolConfig {
    pub fn protocol_name(&self) -> &'static str {
        match self {
            ProtocolConfig::Ssh { .. } => "SSH",
            ProtocolConfig::Local { .. } => "LOCAL",
            ProtocolConfig::Vnc { .. } => "VNC",
        }
    }

    pub fn target_summary(&self) -> String {
        match self {
            ProtocolConfig::Ssh { host, port, username, .. } => {
                format!("{}@{}:{}", username, host, port)
            }
            ProtocolConfig::Local { shell, .. } => shell.display_name().to_string(),
            ProtocolConfig::Vnc { host, port, .. } => format!("{}:{}", host, port),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    pub folder_id: Option<Uuid>,
    pub protocol: ProtocolConfig,
    pub color_tag: Option<String>,
    pub description: String,
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

        let connections = vec![
            ConnectionProfile {
                id: Uuid::new_v4(),
                name: "PowerShell Windows".to_string(),
                folder_id: Some(local_folder_id),
                protocol: ProtocolConfig::Local {
                    shell: LocalShellType::PowerShell,
                    working_dir: None,
                },
                color_tag: Some("#38bdf8".to_string()),
                description: "PowerShell nativo con ConPTY".to_string(),
            },
            ConnectionProfile {
                id: Uuid::new_v4(),
                name: "WSL Linux Shell".to_string(),
                folder_id: Some(local_folder_id),
                protocol: ProtocolConfig::Local {
                    shell: LocalShellType::Wsl,
                    working_dir: None,
                },
                color_tag: Some("#a855f7".to_string()),
                description: "Subsistema de Windows para Linux".to_string(),
            },
            ConnectionProfile {
                id: Uuid::new_v4(),
                name: "Command Prompt (CMD)".to_string(),
                folder_id: Some(local_folder_id),
                protocol: ProtocolConfig::Local {
                    shell: LocalShellType::Cmd,
                    working_dir: None,
                },
                color_tag: Some("#94a3b8".to_string()),
                description: "Símbolo del sistema de Windows".to_string(),
            },
            ConnectionProfile {
                id: Uuid::new_v4(),
                name: "Servidor SSH Demo".to_string(),
                folder_id: Some(servers_folder_id),
                protocol: ProtocolConfig::Ssh {
                    host: "test.rebex.net".to_string(),
                    port: 22,
                    username: "demo".to_string(),
                    auth: SshAuth::Password("password".to_string()),
                },
                color_tag: Some("#10b981".to_string()),
                description: "Servidor SSH público de pruebas (Rebex)".to_string(),
            },
            ConnectionProfile {
                id: Uuid::new_v4(),
                name: "VNC Localhost".to_string(),
                folder_id: Some(servers_folder_id),
                protocol: ProtocolConfig::Vnc {
                    host: "127.0.0.1".to_string(),
                    port: 5900,
                    password: None,
                },
                color_tag: Some("#f97316".to_string()),
                description: "Escritorio remoto VNC en puerto 5900".to_string(),
            },
        ];

        Self {
            folders,
            connections,
        }
    }
}

impl ConfigStore {
    fn config_path() -> PathBuf {
        if let Some(mut dir) = dirs::config_dir() {
            dir.push("pulento");
            let _ = fs::create_dir_all(&dir);
            dir.push("connections.json");
            dir
        } else {
            PathBuf::from("connections.json")
        }
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(config) = serde_json::from_str::<ConfigStore>(&data) {
                    return config;
                }
            }
        }
        let default_config = Self::default();
        let _ = default_config.save();
        default_config
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::config_path();
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        fs::write(path, json)
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
            self.connections[pos] = profile;
            let _ = self.save();
        }
    }

    pub fn remove_connection(&mut self, id: Uuid) {
        self.connections.retain(|c| c.id != id);
        let _ = self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_has_folders_and_connections() {
        let config = ConfigStore::default();
        assert!(!config.folders.is_empty(), "Default config should have folders");
        assert!(!config.connections.is_empty(), "Default config should have connections");

        let ssh_conn = config
            .connections
            .iter()
            .find(|c| matches!(c.protocol, ProtocolConfig::Ssh { .. }));
        assert!(ssh_conn.is_some(), "Should include an SSH profile");

        let local_conn = config
            .connections
            .iter()
            .find(|c| matches!(c.protocol, ProtocolConfig::Local { .. }));
        assert!(local_conn.is_some(), "Should include a Local terminal profile");

        let vnc_conn = config
            .connections
            .iter()
            .find(|c| matches!(c.protocol, ProtocolConfig::Vnc { .. }));
        assert!(vnc_conn.is_some(), "Should include a VNC profile");
    }

    #[test]
    fn test_add_remove_folder() {
        let mut config = ConfigStore::default();
        let folder_id = config.add_folder("Laboratorio".to_string());
        assert!(config.folders.iter().any(|f| f.id == folder_id && f.name == "Laboratorio"));

        config.remove_folder(folder_id);
        assert!(!config.folders.iter().any(|f| f.id == folder_id));
    }

    #[test]
    fn test_shell_command_args() {
        let ps = LocalShellType::PowerShell;
        let (cmd, args) = ps.to_command_args();
        assert_eq!(cmd, "powershell.exe");
        assert_eq!(args, vec!["-NoLogo"]);

        let wsl = LocalShellType::Wsl;
        let (cmd, _) = wsl.to_command_args();
        assert_eq!(cmd, "wsl.exe");
    }
}
