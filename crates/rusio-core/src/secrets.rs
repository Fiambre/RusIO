//! Passwords live in the OS credential store (Windows Credential Manager, macOS Keychain,
//! Linux Secret Service) keyed by connection id and a per-field tag, instead of in
//! connections.json.
use uuid::Uuid;

const SERVICE: &str = "rusio";
/// Service name used before the project was renamed from Pulento.
const LEGACY_SERVICE: &str = "pulento";

/// Only platforms with a real keyring backend compiled in. Elsewhere `keyring` falls back
/// to an in-memory mock, which would silently lose the secret.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn entry_in(service: &str, id: Uuid, tag: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(service, &format!("{}:{}", id, tag)).map_err(|e| e.to_string())
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn entry(id: Uuid, tag: &str) -> Result<keyring::Entry, String> {
    entry_in(SERVICE, id, tag)
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub fn get(id: Uuid, tag: &str) -> Option<String> {
    if let Some(v) = entry(id, tag).ok().and_then(|e| e.get_password().ok()) {
        return Some(v);
    }
    // Stored under the pre-rename service name: move it across on first read.
    let old = entry_in(LEGACY_SERVICE, id, tag).ok()?;
    let v = old.get_password().ok()?;
    if set(id, tag, &v).is_ok() {
        let _ = old.delete_credential();
    }
    Some(v)
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub fn set(id: Uuid, tag: &str, value: &str) -> Result<(), String> {
    entry(id, tag)?.set_password(value).map_err(|e| e.to_string())
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn delete(id: Uuid, tag: &str) {
    for service in [SERVICE, LEGACY_SERVICE] {
        if let Ok(e) = entry_in(service, id, tag) {
            let _ = e.delete_credential();
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn get(_id: Uuid, _tag: &str) -> Option<String> {
    None
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn set(_id: Uuid, _tag: &str, _value: &str) -> Result<(), String> {
    Err("no hay almacén de credenciales en esta plataforma".to_string())
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn delete(_id: Uuid, _tag: &str) {}

/// Removes the given stored secrets of a connection.
pub fn delete_all(id: Uuid, tags: &[&str]) {
    for tag in tags {
        delete(id, tag);
    }
}

#[cfg(all(test, any(windows, target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;

    const A: &str = "test-a";
    const B: &str = "test-b";

    #[test]
    fn roundtrip_set_get_delete() {
        let id = Uuid::new_v4();
        assert!(get(id, A).is_none());

        set(id, A, "s3cr3t-ñ").expect("keyring unavailable");
        assert_eq!(get(id, A).as_deref(), Some("s3cr3t-ñ"));
        assert!(get(id, B).is_none(), "tags must not collide");

        delete_all(id, &[A, B]);
        assert!(get(id, A).is_none());
    }

    #[test]
    fn migrates_legacy_service_on_read() {
        let id = Uuid::new_v4();
        entry_in(LEGACY_SERVICE, id, A)
            .unwrap()
            .set_password("old-secret")
            .expect("keyring unavailable");

        assert_eq!(get(id, A).as_deref(), Some("old-secret"));
        assert!(entry_in(LEGACY_SERVICE, id, A).unwrap().get_password().is_err());
        assert_eq!(entry(id, A).unwrap().get_password().unwrap(), "old-secret");

        delete_all(id, &[A]);
    }
}
