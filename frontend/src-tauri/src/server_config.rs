//! Desktop server address configuration.
//!
//! The configured backend address is persisted by Rust in the app data
//! directory (`server.json`) — never in WebView storage — and falls back to
//! the `SKILLHIVE_SERVER_URL` environment variable, then to the conventional
//! local development server. The value is validated (http/https, no
//! userinfo, no query/fragment) before it is ever stored or used to build
//! an HTTP client.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8000";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerConfigFile {
    base_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ServerConfigError {
    #[error("server address must be an http(s) URL: {0}")]
    InvalidUrl(String),
    #[error("server address filesystem error: {0}")]
    Io(String),
}

/// Normalizes and validates a user-provided server address.
pub fn normalize_server_url(raw: &str) -> Result<String, ServerConfigError> {
    let trimmed = raw.trim();
    let without_trailing = trimmed.trim_end_matches('/');
    if !without_trailing.starts_with("http://") && !without_trailing.starts_with("https://") {
        return Err(ServerConfigError::InvalidUrl(trimmed.to_owned()));
    }
    let scheme_end = without_trailing
        .find("://")
        .map(|index| index + 3)
        .unwrap_or(0);
    let rest = &without_trailing[scheme_end..];
    if rest.is_empty() || rest.contains('@') || rest.contains('?') || rest.contains('#') {
        return Err(ServerConfigError::InvalidUrl(trimmed.to_owned()));
    }
    Ok(without_trailing.to_owned())
}

/// Resolution order: explicit stored value, environment variable, default.
/// Each layer is validated; invalid layers are skipped rather than bricking
/// startup.
pub fn resolve(stored: Option<&str>, env_value: Option<&str>) -> String {
    if let Some(value) = stored.and_then(|value| normalize_server_url(value).ok()) {
        return value;
    }
    if let Some(value) = env_value.and_then(|value| normalize_server_url(value).ok()) {
        return value;
    }
    DEFAULT_SERVER_URL.to_owned()
}

pub fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("server.json")
}

fn stored_url(data_dir: &Path) -> Option<String> {
    let contents = fs::read_to_string(config_path(data_dir)).ok()?;
    let parsed = serde_json::from_str::<ServerConfigFile>(&contents).ok()?;
    Some(parsed.base_url)
}

/// Loads the configured server address for this installation.
pub fn load(data_dir: &Path) -> String {
    resolve(
        stored_url(data_dir).as_deref(),
        Some(&std::env::var("SKILLHIVE_SERVER_URL").unwrap_or_default()),
    )
}

/// Persists the server address atomically (temp file + rename).
pub fn store(data_dir: &Path, base_url: &str) -> Result<String, ServerConfigError> {
    let normalized = normalize_server_url(base_url)?;
    let path = config_path(data_dir);
    let contents = serde_json::to_string(&ServerConfigFile {
        base_url: normalized.clone(),
    })
    .map_err(|error| ServerConfigError::Io(error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, contents).map_err(|error| ServerConfigError::Io(error.to_string()))?;
    fs::rename(&temporary, &path).map_err(|error| ServerConfigError::Io(error.to_string()))?;
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("skillhive-cfg-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn normalizes_and_rejects_addresses() {
        assert_eq!(
            normalize_server_url(" http://localhost:8000/ ").unwrap(),
            "http://localhost:8000"
        );
        assert_eq!(
            normalize_server_url("https://skillhive.example.com").unwrap(),
            "https://skillhive.example.com"
        );
        assert!(normalize_server_url("localhost:8000").is_err());
        assert!(normalize_server_url("ftp://x").is_err());
        assert!(normalize_server_url("http://user@host").is_err());
        assert!(normalize_server_url("http://host/path?q=1").is_err());
        assert!(normalize_server_url("").is_err());
    }

    #[test]
    fn resolution_order_is_stored_then_env_then_default() {
        assert_eq!(
            resolve(Some("http://a:1"), Some("http://b:2")),
            "http://a:1"
        );
        assert_eq!(resolve(None, Some("http://b:2")), "http://b:2");
        assert_eq!(resolve(Some("not-a-url"), Some("http://b:2")), "http://b:2");
        assert_eq!(resolve(Some("http://a:1"), Some("also-bad")), "http://a:1");
        assert_eq!(resolve(None, None), DEFAULT_SERVER_URL);
    }

    #[test]
    fn store_and_load_round_trip() {
        let dir = temp_dir();
        let stored = store(&dir, "http://127.0.0.1:9000/").unwrap();
        assert_eq!(stored, "http://127.0.0.1:9000");
        assert_eq!(resolve(stored_url(&dir).as_deref(), None), stored);
    }

    #[test]
    fn invalid_stored_file_is_skipped() {
        let dir = temp_dir();
        fs::write(config_path(&dir), "{ not json").unwrap();
        assert_eq!(stored_url(&dir), None);
        fs::write(config_path(&dir), r#"{"base_url": "not-a-url"}"#).unwrap();
        assert_eq!(stored_url(&dir).as_deref(), Some("not-a-url"));
        // An invalid stored value falls through to the default at resolve
        // time instead of bricking startup.
        assert_eq!(
            resolve(stored_url(&dir).as_deref(), None),
            DEFAULT_SERVER_URL
        );
    }
}
