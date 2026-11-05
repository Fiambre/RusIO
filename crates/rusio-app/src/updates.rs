//! Looks for a newer release on GitHub. It only reads the public "latest release" of the repository;
//! nothing is downloaded or installed, and no data about the user is sent.
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Public half of the key that signs the installers published by the release workflow.
/// An installer is only run if its `.sig` verifies against this key.
const UPDATE_PUBLIC_KEY: [u8; 32] = [
    0xaf, 0x24, 0x3d, 0x5b, 0x60, 0x1c, 0x42, 0x18, 0x59, 0x83, 0x8d, 0xbc, 0x9a, 0xc9, 0x66, 0x1b, 0xea, 0x91, 0xe6,
    0x57, 0xa9, 0xd9, 0xd9, 0xc4, 0xf9, 0xe6, 0x60, 0x48, 0x7c, 0x91, 0xdc, 0x76,
];
const DOWNLOAD_PREFIX: &str = "https://github.com/Fiambre/RusIO/releases/download/";
/// An installer larger than this is refused.
const MAX_INSTALLER_BYTES: u64 = 64 * 1024 * 1024;

const LATEST_URL: &str = "https://api.github.com/repos/Fiambre/RusIO/releases/latest";

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateInfo {
    /// Version without the leading `v`, e.g. `0.2.0`.
    pub version: String,
    /// Page of the release, where the installer can be downloaded.
    pub url: String,
    /// The installer and its detached signature, when the release has both.
    pub installer: Option<Installer>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Installer {
    pub url: String,
    pub sig_url: String,
}

/// Asks GitHub for the latest release. `Ok(None)` means the app is up to date.
pub fn check(current: &str) -> Result<Option<UpdateInfo>, String> {
    let body = ureq::get(LATEST_URL)
        .set("User-Agent", &format!("RusIO/{current}"))
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|e| format!("no se pudo consultar GitHub: {e}"))?
        .into_string()
        .map_err(|e| format!("respuesta ilegible: {e}"))?;
    parse_latest(&body, current)
}

fn parse_latest(body: &str, current: &str) -> Result<Option<UpdateInfo>, String> {
    let json: Value = serde_json::from_str(body).map_err(|e| format!("respuesta no válida: {e}"))?;
    let tag = json
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or("la respuesta no trae la versión")?;
    let version = tag.trim_start_matches('v').to_string();
    if !is_newer(&version, current) {
        return Ok(None);
    }
    let url = json
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|u| u.starts_with("https://github.com/"))
        .unwrap_or("https://github.com/Fiambre/RusIO/releases/latest")
        .to_string();
    let installer = find_installer(&json);
    Ok(Some(UpdateInfo {
        version,
        url,
        installer,
    }))
}

/// The `RusIO-Setup-*.exe` asset and its `.sig`, if both are hosted by this repository's releases.
fn find_installer(release: &Value) -> Option<Installer> {
    let assets = release.get("assets")?.as_array()?;
    let url_of = |name: &str| {
        assets
            .iter()
            .find(|a| a.get("name").and_then(Value::as_str) == Some(name))
            .and_then(|a| a.get("browser_download_url")?.as_str())
            .filter(|u| u.starts_with(DOWNLOAD_PREFIX))
            .map(str::to_string)
    };
    let exe = assets
        .iter()
        .filter_map(|a| a.get("name")?.as_str())
        .find(|n| n.starts_with("RusIO-Setup-") && n.ends_with(".exe"))?;
    Some(Installer {
        url: url_of(exe)?,
        sig_url: url_of(&format!("{exe}.sig"))?,
    })
}

/// Bytes downloaded so far and the expected total, shared with the UI to draw a progress bar.
#[derive(Default, Clone)]
pub struct Progress {
    pub done: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            (self.done.load(Ordering::Relaxed) as f32 / total as f32).min(1.0)
        }
    }
}

/// Downloads the installer, checks its signature and saves it in a temporary folder.
/// Returns the path only if the signature is valid.
pub fn download_installer(installer: &Installer, progress: &Progress) -> Result<PathBuf, String> {
    let get = |url: &str| {
        ureq::get(url)
            .set("User-Agent", concat!("RusIO/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(120))
            .call()
            .map_err(|e| format!("descarga fallida: {e}"))
    };
    let sig = read_limited(get(&installer.sig_url)?.into_reader(), 128, None)?;

    let resp = get(&installer.url)?;
    let total = resp
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    if total > MAX_INSTALLER_BYTES {
        return Err("el instalador es demasiado grande".to_string());
    }
    progress.total.store(total, Ordering::Relaxed);
    let data = read_limited(resp.into_reader(), MAX_INSTALLER_BYTES, Some(&progress.done))?;

    verify(&data, &sig, &UPDATE_PUBLIC_KEY)?;

    let dir = std::env::temp_dir().join("rusio-update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("no se pudo crear la carpeta temporal: {e}"))?;
    let path = dir.join("RusIO-Setup.exe");
    std::fs::write(&path, &data).map_err(|e| format!("no se pudo guardar el instalador: {e}"))?;
    Ok(path)
}

fn read_limited(mut r: impl Read, max: u64, progress: Option<&AtomicU64>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(|e| format!("descarga interrumpida: {e}"))?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() as u64 > max {
            return Err("la descarga supera el tamaño permitido".to_string());
        }
        if let Some(p) = progress {
            p.store(out.len() as u64, Ordering::Relaxed);
        }
    }
}

/// Checks an Ed25519 signature (raw 64 bytes) of `data` against `public_key`.
fn verify(data: &[u8], sig: &[u8], public_key: &[u8; 32]) -> Result<(), String> {
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| "clave de actualización inválida".to_string())?;
    let sig: [u8; 64] = sig.try_into().map_err(|_| "firma con formato inválido".to_string())?;
    key.verify(data, &Signature::from_bytes(&sig))
        .map_err(|_| "la firma del instalador NO es válida; no se instalará".to_string())
}

/// Starts the installer silently; it waits for RusIO to exit, replaces the files and reopens the app.
pub fn launch_installer(path: &std::path::Path) -> Result<(), String> {
    std::process::Command::new(path)
        .arg("/S")
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("no se pudo iniciar el instalador: {e}"))
}

/// True when `candidate` is a higher `major.minor.patch` than `current` (pre-release suffixes are ignored).
fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> Option<[u64; 3]> {
        let core = v.trim_start_matches('v').split(['-', '+']).next()?;
        let mut it = core.split('.').map(|p| p.parse::<u64>());
        let out = [
            it.next()?.ok()?,
            it.next().unwrap_or(Ok(0)).ok()?,
            it.next().unwrap_or(Ok(0)).ok()?,
        ];
        Some(out)
    }
    match (parts(candidate), parts(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_compared_numerically() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.5"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("garbage", "0.1.0"));
        assert!(is_newer("0.2.0-beta", "0.1.0"));
    }

    #[test]
    fn latest_release_is_parsed() {
        let body = r#"{"tag_name":"v0.2.0","html_url":"https://github.com/Fiambre/RusIO/releases/tag/v0.2.0"}"#;
        let up = parse_latest(body, "0.1.0").unwrap().unwrap();
        assert_eq!(up.version, "0.2.0");
        assert!(up.url.ends_with("v0.2.0"));
        assert_eq!(parse_latest(body, "0.2.0").unwrap(), None);
    }

    #[test]
    fn foreign_links_and_bad_bodies_are_not_trusted() {
        let body = r#"{"tag_name":"v0.2.0","html_url":"http://evil.example/x"}"#;
        let up = parse_latest(body, "0.1.0").unwrap().unwrap();
        assert!(up.url.starts_with("https://github.com/Fiambre/RusIO"));
        assert_eq!(up.installer, None);
        assert!(parse_latest("{}", "0.1.0").is_err());
        assert!(parse_latest("not json", "0.1.0").is_err());
    }

    #[test]
    fn installer_assets_must_come_from_this_repository() {
        let release = |host: &str| {
            let base = format!("{host}/Fiambre/RusIO/releases/download/v0.2.0/RusIO-Setup-0.2.0.exe");
            serde_json::json!({"assets": [
                {"name": "RusIO-Setup-0.2.0.exe", "browser_download_url": base},
                {"name": "RusIO-Setup-0.2.0.exe.sig", "browser_download_url": format!("{base}.sig")},
            ]})
        };
        let inst = find_installer(&release("https://github.com")).unwrap();
        assert!(inst.url.ends_with(".exe") && inst.sig_url.ends_with(".exe.sig"));
        assert_eq!(find_installer(&release("https://evil.example")), None);
        // An installer without its signature is not offered.
        let no_sig = serde_json::json!({"assets": [{"name": "RusIO-Setup-0.2.0.exe",
            "browser_download_url": "https://github.com/Fiambre/RusIO/releases/download/v0.2.0/RusIO-Setup-0.2.0.exe"}]});
        assert_eq!(find_installer(&no_sig), None);
    }

    #[test]
    fn signatures_are_checked() {
        use ed25519_dalek::{Signer, SigningKey};
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        let data = b"installer bytes";
        let sig = sk.sign(data).to_bytes();
        assert!(verify(data, &sig, &pk).is_ok());
        assert!(verify(b"tampered", &sig, &pk).is_err());
        assert!(verify(data, &sig[..10], &pk).is_err());
        let other = SigningKey::from_bytes(&[8u8; 32]).verifying_key().to_bytes();
        assert!(verify(data, &sig, &other).is_err());
    }

    #[test]
    fn downloads_are_capped() {
        let big = std::io::repeat(1).take(1000);
        assert!(read_limited(big, 100, None).is_err());
        assert_eq!(read_limited(&b"abc"[..], 100, None).unwrap(), b"abc");
    }

    /// Checks a file signed with `openssl pkeyutl -sign -rawin` against the embedded public key.
    /// `RUSIO_SIG_TEST="<file>|<sig>"` enables it.
    #[test]
    fn embedded_key_verifies_openssl_signatures() {
        let Ok(spec) = std::env::var("RUSIO_SIG_TEST") else {
            return;
        };
        let (file, sig) = spec.split_once('|').unwrap();
        let data = std::fs::read(file).unwrap();
        let mut sig_bytes = std::fs::read(sig).unwrap();
        verify(&data, &sig_bytes, &UPDATE_PUBLIC_KEY).unwrap();
        sig_bytes[0] ^= 1;
        assert!(verify(&data, &sig_bytes, &UPDATE_PUBLIC_KEY).is_err());
    }

    /// Talks to GitHub for real; set `RUSIO_NET_TEST=1` to run it.
    #[test]
    fn live_check_against_github() {
        if std::env::var("RUSIO_NET_TEST").is_err() {
            return;
        }
        let up = check("0.0.1")
            .unwrap()
            .expect("0.0.1 must be older than the latest release");
        assert!(is_newer(&up.version, "0.0.1"));
        assert!(up.url.starts_with("https://github.com/Fiambre/RusIO"));
        assert_eq!(check("999.0.0").unwrap(), None);
        // A release that ships a signed installer must download and verify.
        if let Some(installer) = &up.installer {
            let path = download_installer(installer, &Progress::default()).unwrap();
            assert!(std::fs::metadata(path).unwrap().len() > 1_000_000);
        }
    }
}
