//! Looks for a newer release on GitHub. It only reads the public "latest release" of the repository;
//! nothing is downloaded or installed, and no data about the user is sent.
use serde_json::Value;
use std::time::Duration;

const LATEST_URL: &str = "https://api.github.com/repos/Fiambre/RusIO/releases/latest";

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateInfo {
    /// Version without the leading `v`, e.g. `0.2.0`.
    pub version: String,
    /// Page of the release, where the installer can be downloaded.
    pub url: String,
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
    Ok(Some(UpdateInfo { version, url }))
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
        assert!(parse_latest("{}", "0.1.0").is_err());
        assert!(parse_latest("not json", "0.1.0").is_err());
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
    }
}
