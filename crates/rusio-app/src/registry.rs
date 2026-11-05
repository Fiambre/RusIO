//! The set of protocol plugins compiled into this build (see the `ssh`/`local`/`vnc`
//! cargo features).
use rusio_core::config::SecretSchema;
use rusio_plugin_api::{Plugin, ProtocolDef, QuickLaunch};

pub struct Entry {
    pub plugin: Box<dyn Plugin>,
    pub def: ProtocolDef,
}

pub struct Registry {
    entries: Vec<Entry>,
}

impl Registry {
    pub fn builtin() -> Self {
        #[allow(unused_mut)]
        let mut plugins: Vec<Box<dyn Plugin>> = Vec::new();
        #[cfg(feature = "ssh")]
        plugins.push(Box::new(rusio_plugin_ssh::SshPlugin));
        #[cfg(feature = "local")]
        plugins.push(Box::new(rusio_plugin_local::LocalPlugin));
        #[cfg(feature = "vnc")]
        plugins.push(Box::new(rusio_plugin_vnc::VncPlugin));
        #[cfg(feature = "sqlite")]
        plugins.push(Box::new(rusio_plugin_sqlite::SqlitePlugin));
        #[cfg(feature = "postgres")]
        plugins.push(Box::new(rusio_plugin_postgres::PostgresPlugin));
        #[cfg(feature = "mysql")]
        plugins.push(Box::new(rusio_plugin_mysql::MysqlPlugin));
        #[cfg(feature = "mongodb")]
        plugins.push(Box::new(rusio_plugin_mongodb::MongoPlugin));
        Self::new(plugins)
    }

    pub fn new(plugins: Vec<Box<dyn Plugin>>) -> Self {
        let entries = plugins
            .into_iter()
            .map(|plugin| Entry {
                def: plugin.protocol(),
                plugin,
            })
            .collect();
        Self { entries }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.def.id == id)
    }

    /// Which fields of each protocol are secrets, for the config store.
    pub fn secret_schema(&self) -> SecretSchema {
        self.entries
            .iter()
            .map(|e| (e.def.id.to_string(), e.def.secrets.clone()))
            .collect()
    }

    /// One-click launchers of every plugin, paired with the owning protocol id.
    pub fn quick_launches(&self) -> Vec<(&'static str, QuickLaunch)> {
        self.entries
            .iter()
            .flat_map(|e| e.plugin.quick_launches().into_iter().map(|q| (e.def.id, q)))
            .collect()
    }

    /// Protocols that can be started from the title bar's quick-connect box.
    pub fn quick_connect_entries(&self) -> Vec<&Entry> {
        self.entries.iter().filter(|e| e.def.quick_connect.is_some()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusio_core::config::ConfigStore;
    use rusio_plugin_api::SettingKind;

    #[test]
    fn every_default_connection_is_understood_by_its_plugin() {
        let registry = Registry::builtin();
        for c in &ConfigStore::default().connections {
            let entry = registry
                .get(&c.protocol.kind)
                .unwrap_or_else(|| panic!("no plugin for {}", c.protocol.kind));
            assert!(
                !entry.plugin.summary(&c.protocol.params).is_empty(),
                "{} params were not parsed by its plugin",
                c.protocol.kind
            );
        }
    }

    #[test]
    fn plugin_ids_are_unique_and_secret_tags_are_stable() {
        let registry = Registry::builtin();
        let ids: Vec<_> = registry.entries().iter().map(|e| e.def.id).collect();
        let mut dedup = ids.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len(), "duplicate protocol ids: {ids:?}");

        // These tags name entries already stored in users' credential managers.
        let schema = registry.secret_schema();
        let tags = |kind: &str| -> Vec<&'static str> {
            schema
                .get(kind)
                .map(|f| f.iter().map(|s| s.tag).collect())
                .unwrap_or_default()
        };
        assert_eq!(tags("Ssh"), ["ssh-password", "key-passphrase"]);
        assert_eq!(tags("Vnc"), ["vnc-password"]);
    }

    #[test]
    fn declared_settings_have_defaults_inside_their_range() {
        let registry = Registry::builtin();
        for e in registry.entries() {
            for f in e.plugin.settings_schema() {
                if let SettingKind::Int { default, min, max, .. } = f.kind {
                    assert!(
                        (min..=max).contains(&default),
                        "{}.{} default {default} outside {min}..={max}",
                        e.def.id,
                        f.key
                    );
                }
            }
        }
    }
}
