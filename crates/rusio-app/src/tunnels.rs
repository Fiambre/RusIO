//! Reaching a connection through gateways (bastions / jump hosts).
//!
//! A profile may name another profile in `via`; that one may name yet another, and so on.
//! `build_chain` turns that into an ordered list of hops (outermost first) and `resolve`
//! opens one tunnel per hop, each through the previous one, finally pointing the target's
//! params at the last tunnel's local port.
use rusio_core::config::ConnectionProfile;
use rusio_plugin_api::{Params, SettingValues};
use std::any::Any;
use std::collections::HashSet;

use crate::registry::Registry;

pub const MAX_HOPS: usize = 5;

/// One gateway on the way to the target, with everything needed to use it off the UI thread.
#[derive(Debug)]
pub struct Hop {
    pub plugin_id: String,
    pub name: String,
    pub params: Params,
    pub settings: SettingValues,
}

/// Target params already aimed at the last tunnel, plus the tunnels that must stay open
/// for as long as the session lives.
pub struct Resolved {
    pub params: Params,
    pub guards: Vec<Box<dyn Any + Send>>,
}

/// The gateways `profile` goes through, outermost first (empty if it connects directly).
pub fn build_chain(
    profile: &ConnectionProfile,
    connections: &[ConnectionProfile],
    registry: &Registry,
    settings_for: &dyn Fn(&str) -> SettingValues,
) -> Result<Vec<Hop>, String> {
    let Some(first_via) = profile.via else {
        return Ok(Vec::new());
    };

    let target = registry
        .get(&profile.protocol.kind)
        .ok_or_else(|| format!("no hay plugin para el protocolo \"{}\"", profile.protocol.kind))?;
    if target.def.endpoint.is_none() {
        return Err(format!(
            "{} no admite conexiones a través de una puerta de enlace",
            target.def.name
        ));
    }

    let mut seen: HashSet<_> = [profile.id].into();
    let mut hops = Vec::new();
    let mut next = Some(first_via);
    while let Some(id) = next {
        if !seen.insert(id) {
            return Err("las puertas de enlace forman un ciclo".to_string());
        }
        if hops.len() >= MAX_HOPS {
            return Err(format!("demasiadas puertas de enlace encadenadas (máximo {MAX_HOPS})"));
        }
        let gw = connections
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| "la puerta de enlace de esta conexión ya no existe".to_string())?;
        let entry = registry
            .get(&gw.protocol.kind)
            .ok_or_else(|| format!("no hay plugin para la puerta de enlace \"{}\"", gw.name))?;
        if !entry.def.gateway {
            return Err(format!("\"{}\" no puede actuar como puerta de enlace", gw.name));
        }
        if gw.via.is_some() && entry.def.endpoint.is_none() {
            return Err(format!("\"{}\" no puede conectarse a través de otra puerta", gw.name));
        }
        hops.push(Hop {
            plugin_id: gw.protocol.kind.clone(),
            name: gw.name.clone(),
            params: gw.protocol.params.clone(),
            settings: settings_for(&gw.protocol.kind),
        });
        next = gw.via;
    }
    hops.reverse();
    Ok(hops)
}

/// Opens the tunnels (blocking: run it on a worker thread).
pub fn resolve(
    registry: &Registry,
    hops: &[Hop],
    target_kind: &str,
    target_params: &Params,
) -> Result<Resolved, String> {
    let mut guards: Vec<Box<dyn Any + Send>> = Vec::new();
    let Some(first) = hops.first() else {
        return Ok(Resolved { params: target_params.clone(), guards });
    };

    // Params to reach hop `i`: the first goes direct, the rest through the previous tunnel.
    let mut gateway_params = first.params.clone();
    let mut final_params = target_params.clone();

    for (i, hop) in hops.iter().enumerate() {
        let gateway = registry
            .get(&hop.plugin_id)
            .ok_or_else(|| format!("no hay plugin para la puerta de enlace \"{}\"", hop.name))?;

        let (next_kind, next_params) = match hops.get(i + 1) {
            Some(next) => (next.plugin_id.as_str(), &next.params),
            None => (target_kind, target_params),
        };
        let next_entry = registry
            .get(next_kind)
            .ok_or_else(|| format!("no hay plugin para el protocolo \"{next_kind}\""))?;
        let endpoint = next_entry
            .def
            .endpoint
            .as_ref()
            .ok_or_else(|| format!("{} no admite puertas de enlace", next_entry.def.name))?;
        let (host, port) = (endpoint.get)(next_params)
            .ok_or_else(|| "el destino no tiene host y puerto válidos".to_string())?;

        let tunnel = gateway
            .plugin
            .open_tunnel(&gateway_params, &hop.settings, &host, port)
            .map_err(|e| format!("túnel por \"{}\": {e}", hop.name))?;
        guards.push(tunnel.guard);

        let rewritten = (endpoint.set)(next_params, "127.0.0.1", tunnel.local_port);
        if i + 1 < hops.len() {
            gateway_params = rewritten;
        } else {
            final_params = rewritten;
        }
    }

    // Close the innermost tunnel first.
    guards.reverse();
    Ok(Resolved { params: final_params, guards })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use rusio_core::config::ProtocolData;
    use rusio_plugin_api::egui::{self, Color32};
    use rusio_plugin_api::serde_json::json;
    use rusio_plugin_api::{
        ConnectRequest, Connected, Endpoint, Plugin, ProtocolDef, Tunnel,
    };
    use std::sync::Arc;
    use uuid::Uuid;

    fn ep_get(p: &Params) -> Option<(String, u16)> {
        Some((p["host"].as_str()?.to_string(), p["port"].as_u64()? as u16))
    }

    fn ep_set(p: &Params, host: &str, port: u16) -> Params {
        let mut p = p.clone();
        p["host"] = json!(host);
        p["port"] = json!(port);
        p
    }

    /// What each `open_tunnel` call was asked: (gateway params host:port, target host:port).
    type Calls = Arc<Mutex<Vec<(String, String)>>>;

    struct Fake {
        id: &'static str,
        gateway: bool,
        endpoint: bool,
        calls: Calls,
        next_port: Mutex<u16>,
    }

    struct Guard(&'static str, Arc<Mutex<Vec<&'static str>>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.1.lock().push(self.0);
        }
    }


    impl Plugin for Fake {
        fn protocol(&self) -> ProtocolDef {
            ProtocolDef {
                id: self.id,
                name: self.id,
                badge: "T",
                color: Color32::WHITE,
                category: rusio_plugin_api::Category::Remote,
                icon: rusio_plugin_api::ProtocolIcon::Glyph(rusio_plugin_api::icons::Icon::Server),
                secrets: vec![],
                quick_connect: None,
                actions: vec![],
                welcome: None,
                gateway: self.gateway,
                endpoint: self.endpoint.then_some(Endpoint { get: ep_get, set: ep_set }),
            }
        }
        fn default_params(&self) -> Params {
            json!({})
        }
        fn summary(&self, _: &Params) -> String {
            String::new()
        }
        fn edit_ui(&self, _: &mut egui::Ui, _: &mut Params) {}
        fn connect(&self, _: &ConnectRequest) -> Result<Connected, String> {
            Err("n/a".into())
        }
        fn open_tunnel(
            &self,
            params: &Params,
            _: &SettingValues,
            host: &str,
            port: u16,
        ) -> Result<Tunnel, String> {
            let gw = ep_get(params).map(|(h, p)| format!("{h}:{p}")).unwrap();
            self.calls.lock().push((gw, format!("{host}:{port}")));
            let mut next = self.next_port.lock();
            *next += 1;
            Ok(Tunnel { local_port: *next, guard: Box::new(()) })
        }
    }

    fn fake(id: &'static str, gateway: bool, endpoint: bool, calls: &Calls, first_port: u16) -> Box<dyn Plugin> {
        Box::new(Fake { id, gateway, endpoint, calls: calls.clone(), next_port: Mutex::new(first_port) })
    }

    fn profile(name: &str, kind: &str, host: &str, port: u16, via: Option<Uuid>) -> ConnectionProfile {
        ConnectionProfile {
            id: Uuid::new_v4(),
            name: name.into(),
            folder_id: None,
            protocol: ProtocolData::new(kind, json!({ "host": host, "port": port })),
            color_tag: None,
            description: String::new(),
            via,
            icon: None,
        }
    }

    fn setup() -> (Registry, Calls) {
        let calls: Calls = Arc::default();
        let reg = Registry::new(vec![
            fake("Gw", true, true, &calls, 40000),
            fake("Db", false, true, &calls, 0),
            fake("NoEp", false, false, &calls, 0),
        ]);
        (reg, calls)
    }

    fn none(_: &str) -> SettingValues {
        SettingValues::new()
    }

    #[test]
    fn direct_connection_needs_no_hops() {
        let (reg, _) = setup();
        let p = profile("db", "Db", "10.0.0.5", 5432, None);
        assert!(build_chain(&p, &[p.clone()], &reg, &none).unwrap().is_empty());
        let r = resolve(&reg, &[], "Db", &p.protocol.params).unwrap();
        assert_eq!(r.params, p.protocol.params);
        assert!(r.guards.is_empty());
    }

    #[test]
    fn one_hop_aims_the_target_at_the_local_port() {
        let (reg, calls) = setup();
        let bastion = profile("bastion", "Gw", "bastion.example", 22, None);
        let db = profile("db", "Db", "10.0.0.5", 5432, Some(bastion.id));
        let all = vec![bastion.clone(), db.clone()];

        let hops = build_chain(&db, &all, &reg, &none).unwrap();
        assert_eq!(hops.len(), 1);
        assert_eq!(hops[0].name, "bastion");

        let r = resolve(&reg, &hops, "Db", &db.protocol.params).unwrap();
        // The gateway was reached directly and asked to forward to the private target...
        assert_eq!(calls.lock()[0], ("bastion.example:22".to_string(), "10.0.0.5:5432".to_string()));
        // ...and the session now connects to the tunnel's local port.
        assert_eq!(r.params["host"], "127.0.0.1");
        assert_eq!(r.params["port"], 40001);
        assert_eq!(r.guards.len(), 1);
    }

    #[test]
    fn chained_hops_go_through_each_other_outermost_first() {
        let (reg, calls) = setup();
        let outer = profile("outer", "Gw", "outer.example", 22, None);
        let inner = profile("inner", "Gw", "192.168.1.10", 22, Some(outer.id));
        let db = profile("db", "Db", "10.0.0.5", 5432, Some(inner.id));
        let all = vec![outer.clone(), inner.clone(), db.clone()];

        let hops = build_chain(&db, &all, &reg, &none).unwrap();
        assert_eq!(hops.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["outer", "inner"]);

        let r = resolve(&reg, &hops, "Db", &db.protocol.params).unwrap();
        let calls = calls.lock();
        // outer is reached directly and forwards to the inner gateway's address;
        assert_eq!(calls[0], ("outer.example:22".to_string(), "192.168.1.10:22".to_string()));
        // inner is reached through outer's tunnel and forwards to the database.
        assert_eq!(calls[1], ("127.0.0.1:40001".to_string(), "10.0.0.5:5432".to_string()));
        assert_eq!(r.params["port"], 40002);
        assert_eq!(r.guards.len(), 2);
    }

    #[test]
    fn guards_drop_innermost_first() {
        let order: Arc<Mutex<Vec<&'static str>>> = Arc::default();
        struct Ordered(Arc<Mutex<Vec<&'static str>>>, Mutex<usize>);
        impl Plugin for Ordered {
            fn protocol(&self) -> ProtocolDef {
                Fake { id: "Gw", gateway: true, endpoint: true, calls: Arc::default(), next_port: Mutex::new(0) }.protocol()
            }
            fn default_params(&self) -> Params { json!({}) }
            fn summary(&self, _: &Params) -> String { String::new() }
            fn edit_ui(&self, _: &mut egui::Ui, _: &mut Params) {}
            fn connect(&self, _: &ConnectRequest) -> Result<Connected, String> { Err("n/a".into()) }
            fn open_tunnel(&self, _: &Params, _: &SettingValues, _: &str, _: u16) -> Result<Tunnel, String> {
                let mut n = self.1.lock();
                *n += 1;
                let name = if *n == 1 { "outer" } else { "inner" };
                Ok(Tunnel { local_port: 1000 + *n as u16, guard: Box::new(Guard(name, self.0.clone())) })
            }
        }
        let reg = Registry::new(vec![Box::new(Ordered(order.clone(), Mutex::new(0)))]);
        let outer = profile("outer", "Gw", "o", 22, None);
        let inner = profile("inner", "Gw", "i", 22, Some(outer.id));
        let target = profile("t", "Gw", "t", 22, Some(inner.id));
        let hops = build_chain(&target, &[outer, inner, target.clone()], &reg, &none).unwrap();
        let r = resolve(&reg, &hops, "Gw", &target.protocol.params).unwrap();
        drop(r);
        assert_eq!(*order.lock(), ["inner", "outer"]);
    }

    #[test]
    fn cycles_missing_and_invalid_gateways_are_rejected() {
        let (reg, _) = setup();

        let mut a = profile("a", "Gw", "a", 22, None);
        let b = profile("b", "Gw", "b", 22, Some(a.id));
        a.via = Some(b.id);
        assert!(build_chain(&a, &[a.clone(), b.clone()], &reg, &none).unwrap_err().contains("ciclo"));

        let orphan = profile("x", "Db", "x", 1, Some(Uuid::new_v4()));
        assert!(build_chain(&orphan, &[orphan.clone()], &reg, &none).unwrap_err().contains("ya no existe"));

        let not_gw = profile("notgw", "Db", "n", 1, None);
        let t = profile("t", "Db", "t", 1, Some(not_gw.id));
        assert!(build_chain(&t, &[not_gw, t.clone()], &reg, &none).unwrap_err().contains("no puede actuar"));

        let gw = profile("gw", "Gw", "g", 22, None);
        let no_ep = profile("t", "NoEp", "t", 1, Some(gw.id));
        assert!(build_chain(&no_ep, &[gw, no_ep.clone()], &reg, &none).unwrap_err().contains("no admite"));
    }

    #[test]
    fn chain_length_is_capped() {
        let (reg, _) = setup();
        let mut all: Vec<ConnectionProfile> = vec![profile("g0", "Gw", "g", 22, None)];
        for i in 1..=MAX_HOPS {
            let prev = all[i - 1].id;
            all.push(profile(&format!("g{i}"), "Gw", "g", 22, Some(prev)));
        }
        let target = profile("t", "Db", "t", 1, Some(all.last().unwrap().id));
        assert!(build_chain(&target, &all, &reg, &none).unwrap_err().contains("demasiadas"));
    }
}
