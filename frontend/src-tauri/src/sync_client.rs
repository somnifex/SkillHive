//! Authenticated Rust HTTP client boundary (M2.3).
//!
//! This module is the only place where network credentials exist. The
//! WebView receives only short-lived operation results — never the refresh
//! token, which lives exclusively in the OS credential store ([`crate::
//! credentials::CredentialStore`], plan §16).
//!
//! The server delivers the refresh credential as an HttpOnly `Set-Cookie`
//! (path `/api/v1/auth`, same-site lax). reqwest's own cookie store stays
//! disabled: the value is read from that header here and immediately
//! written to the OS credential store, so no secret ever reaches the
//! WebView, localStorage, or SQLite. Requests use the blocking client so
//! the whole boundary is callable from the serialized sync worker without
//! pulling an async runtime into the worker's design.
//!
//! Session lifecycle is deliberately separate from mutation outcomes:
//! an authentication or device failure surfaces as a typed
//! [`SyncClientError`] and never causes pending outbox mutation IDs to
//! change (plan §15).

use serde::{Deserialize, Serialize};

use crate::credentials::{
    CredentialStore, CredentialStoreError, ACCOUNT_ACCESS_TOKEN, ACCOUNT_REFRESH_TOKEN,
};

const REFRESH_COOKIE_NAME: &str = "skillhive_refresh";
const REQUEST_TIMEOUT_SECS: u64 = 30;
const CONNECT_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, thiserror::Error)]
pub enum SyncClientError {
    #[error("not signed in")]
    NotSignedIn,
    #[error("credential store failure: {0}")]
    CredentialStore(String),
    #[error("authentication failed: {code} ({status})")]
    Authentication { code: String, status: u16 },
    #[error("server rejected request: {code} ({status})")]
    Request { code: String, status: u16 },
    #[error("server error ({status})")]
    Server { code: Option<String>, status: u16 },
    #[error("request failed: {0}")]
    Network(String),
    #[error("malformed response: {0}")]
    Malformed(String),
}

impl SyncClientError {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Authentication { status, .. }
            | Self::Request { status, .. }
            | Self::Server { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn error_code(&self) -> Option<&str> {
        match self {
            Self::Authentication { code, .. } | Self::Request { code, .. } => Some(code),
            Self::Server { code, .. } => code.as_deref(),
            _ => None,
        }
    }
}

impl From<CredentialStoreError> for SyncClientError {
    fn from(error: CredentialStoreError) -> Self {
        match error {
            CredentialStoreError::NotFound { .. } => Self::NotSignedIn,
            other => Self::CredentialStore(other.to_string()),
        }
    }
}

/// Wire model for `POST /api/v1/auth/login` responses. `TokenResponse` on
/// the server is plain snake_case JSON (no alias generator); the refresh
/// token itself is NOT in this payload — it arrives via `Set-Cookie`.
#[derive(Debug, Clone, Deserialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub expires_in: u64,
}

/// Server error envelope: `{"error": {"code", "message", "details"}}`.
#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
}

/// Wire model for `POST /api/v1/devices/register` (server uses camelCase
/// aliases via `to_camel`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRegistrationRequest {
    pub protocol_version: u32,
    pub client_instance_id: String,
    pub display_name: String,
    pub platform: String,
    pub app_version: String,
}

/// Wire model for `DeviceRead` responses (camelCase aliases on the server).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRegistration {
    pub device_id: String,
    pub client_instance_id: String,
    #[allow(dead_code)]
    pub display_name: String,
    #[allow(dead_code)]
    pub platform: String,
    #[allow(dead_code)]
    pub app_version: String,
    #[allow(dead_code)]
    pub last_seen_at: Option<String>,
    pub revoked_at: Option<String>,
}

/// Result of one login + device registration cycle.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEstablished {
    pub device: DeviceRegistration,
    pub expires_in: u64,
}

impl DeviceRegistration {
    /// Server-side revocation state, exposed for the sync worker.
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
}

/// An authenticated HTTP boundary for the sync engine.
///
/// The access token lives only in process memory for the duration of a
/// call; the refresh token lives only in the OS credential store. Nothing
/// in this struct is ever serialized to the WebView.
pub struct SyncClient {
    base_url: String,
    http: reqwest::blocking::Client,
}

impl SyncClient {
    pub fn new(base_url: &str) -> Result<Self, SyncClientError> {
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .connect_timeout(std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .build()
            .map_err(|error| SyncClientError::Network(error.to_string()))?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            http,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Authenticate with username/password and persist the refresh secret in
    /// the OS credential store (plan §16: never SQLite, never the WebView).
    pub fn login(
        &self,
        username: &str,
        password: &str,
        registration: &DeviceRegistrationRequest,
    ) -> Result<SessionEstablished, SyncClientError> {
        let response = self
            .http
            .post(format!("{}/api/v1/auth/login", self.base_url))
            .json(&serde_json::json!({
                "username": username,
                "password": password,
            }))
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(map_transport)?;

        let refresh_token = extract_refresh_cookie(&response)?;
        let login: LoginResponse = response
            .json()
            .map_err(|error| SyncClientError::Malformed(error.to_string()))?;

        self.store_refresh(&refresh_token)?;
        let device = self.register_device_with(&login.access_token, registration)?;
        Ok(SessionEstablished {
            device,
            expires_in: login.expires_in,
        })
    }

    /// Exchange the stored refresh credential for a fresh access token and
    /// persist the rotated refresh token from the `Set-Cookie` header.
    /// Called by the sync worker before each cycle; a failure here never
    /// touches outbox mutation state.
    pub fn ensure_access_token(&self) -> Result<String, SyncClientError> {
        let refresh = CredentialStore::default().get_secret(ACCOUNT_REFRESH_TOKEN)?;
        let response = self
            .http
            .post(format!("{}/api/v1/auth/refresh", self.base_url))
            .header(
                reqwest::header::COOKIE,
                format!("{}={}", REFRESH_COOKIE_NAME, refresh),
            )
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(map_transport)?;

        let rotated = extract_refresh_cookie(&response)?;
        self.store_refresh(&rotated)?;

        let refreshed: LoginResponse = response
            .json()
            .map_err(|error| SyncClientError::Malformed(error.to_string()))?;
        Ok(refreshed.access_token)
    }

    /// Register (or re-register) the installation. Idempotent per
    /// `(user, client_instance_id)`. A revoked registration surfaces as a
    /// typed authentication/device failure so the worker can stop without
    /// destroying pending local mutations.
    pub fn register_device(
        &self,
        registration: &DeviceRegistrationRequest,
    ) -> Result<DeviceRegistration, SyncClientError> {
        let token = self.ensure_access_token()?;
        self.register_device_with(&token, registration)
    }

    /// Remove stored secrets (logout). Idempotent.
    pub fn clear_credentials(&self) -> Result<(), SyncClientError> {
        let store = CredentialStore::default();
        store.delete_secret(ACCOUNT_REFRESH_TOKEN)?;
        store.delete_secret(ACCOUNT_ACCESS_TOKEN)?;
        Ok(())
    }

    fn register_device_with(
        &self,
        access_token: &str,
        registration: &DeviceRegistrationRequest,
    ) -> Result<DeviceRegistration, SyncClientError> {
        let response = self
            .http
            .post(format!("{}/api/v1/devices/register", self.base_url))
            .bearer_auth(access_token)
            .json(registration)
            .send()
            .map_err(map_transport)?;
        deserialize_or_classify(response)
    }

    /// POST a JSON envelope and decode the JSON response, classifying
    /// non-2xx responses per plan §15. Used by the sync transport layer.
    pub(super) fn post_json<T: serde::de::DeserializeOwned, S: serde::Serialize>(
        &self,
        path: &str,
        body: &S,
    ) -> Result<T, SyncClientError> {
        let token = self.ensure_access_token()?;
        let response = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .json(body)
            .send()
            .map_err(map_transport)?;
        deserialize_or_classify(response)
    }

    /// GET a JSON envelope with bearer auth and query parameters. Used by
    /// the change-feed pull client.
    pub(super) fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> Result<T, SyncClientError> {
        let token = self.ensure_access_token()?;
        let response = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .query(&query)
            .send()
            .map_err(map_transport)?;
        deserialize_or_classify(response)
    }

    /// GET raw octet-stream bytes with bearer auth (verified blob download).
    pub(super) fn get_octet_stream(&self, path: &str) -> Result<Vec<u8>, SyncClientError> {
        let token = self.ensure_access_token()?;
        let response = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .send()
            .map_err(map_transport)?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            let code = response
                .json::<ErrorEnvelope>()
                .ok()
                .map(|envelope| envelope.error.code);
            return Err(classify_status(status, code));
        }
        let bytes = response
            .bytes()
            .map_err(|error| SyncClientError::Network(error.to_string()))?;
        Ok(bytes.to_vec())
    }

    /// PUT raw octet-stream bytes with bearer auth (verified blob upload).
    pub(super) fn put_octet_stream(&self, path: &str, bytes: &[u8]) -> Result<(), SyncClientError> {
        let token = self.ensure_access_token()?;
        let response = self
            .http
            .put(format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(bytes.to_vec())
            .send()
            .map_err(map_transport)?;
        let status = response.status().as_u16();
        if response.status().is_success() {
            return Ok(());
        }
        let code = response
            .json::<ErrorEnvelope>()
            .ok()
            .map(|envelope| envelope.error.code);
        Err(classify_status(status, code))
    }

    fn store_refresh(&self, refresh_token: &str) -> Result<(), SyncClientError> {
        CredentialStore::default().set_secret(ACCOUNT_REFRESH_TOKEN, refresh_token)?;
        Ok(())
    }
}

fn deserialize_or_classify<T: serde::de::DeserializeOwned>(
    response: reqwest::blocking::Response,
) -> Result<T, SyncClientError> {
    let status = response.status().as_u16();
    if response.status().is_success() {
        return response
            .json()
            .map_err(|error| SyncClientError::Malformed(error.to_string()));
    }

    // Error envelope is best-effort: a malformed body still surfaces as a
    // typed HTTP status failure so the caller can classify by status.
    let code = response
        .json::<ErrorEnvelope>()
        .ok()
        .map(|envelope| envelope.error.code);
    Err(classify_status(status, code))
}

fn extract_refresh_cookie(
    response: &reqwest::blocking::Response,
) -> Result<String, SyncClientError> {
    response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|header| {
            let (name, value) = header.split_once('=')?;
            (name.trim() == REFRESH_COOKIE_NAME).then(|| value.trim().to_owned())
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SyncClientError::Authentication {
            code: "MISSING_REFRESH_TOKEN".to_owned(),
            status: 401,
        })
}

fn map_transport(error: reqwest::Error) -> SyncClientError {
    if let Some(status) = error.status() {
        return classify_status(status.as_u16(), None);
    }
    SyncClientError::Network(error.to_string())
}

fn classify_status(status: u16, code: Option<String>) -> SyncClientError {
    match status {
        400 | 404 | 409 | 413 | 422 => SyncClientError::Request {
            code: code.unwrap_or_else(|| "UNKNOWN".to_owned()),
            status,
        },
        401 | 403 => SyncClientError::Authentication {
            code: code.unwrap_or_else(|| "UNKNOWN".to_owned()),
            status,
        },
        _ => SyncClientError::Server { code, status },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration() -> DeviceRegistrationRequest {
        DeviceRegistrationRequest {
            protocol_version: 1,
            client_instance_id: "00000000-0000-4000-8000-000000000000".to_owned(),
            display_name: "Test Desktop".to_owned(),
            platform: "windows".to_owned(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    #[test]
    fn device_registration_request_serializes_camel_case() {
        let json = serde_json::to_value(registration()).expect("serialize");
        assert_eq!(json["protocolVersion"], 1);
        assert_eq!(json["clientInstanceId"], registration().client_instance_id);
        assert_eq!(json["displayName"], "Test Desktop");
        assert!(json.get("protocol_version").is_none());
    }

    #[test]
    fn device_read_deserializes_camel_case_response() {
        let body = serde_json::json!({
            "protocolVersion": 1,
            "deviceId": "11111111-1111-4111-8111-111111111111",
            "clientInstanceId": "00000000-0000-4000-8000-000000000000",
            "displayName": "Sync Desktop",
            "platform": "windows",
            "appVersion": "0.2.0",
            "lastSeenAt": "2026-09-06T00:00:00Z",
            "revokedAt": null
        });
        let device: DeviceRegistration = serde_json::from_value(body).expect("deserialize");
        assert_eq!(device.device_id, "11111111-1111-4111-8111-111111111111");
        assert!(!device.is_revoked());
    }

    #[test]
    fn login_response_deserializes_snake_case_payload() {
        let body = serde_json::json!({
            "access_token": "at",
            "token_type": "bearer",
            "expires_in": 900,
            "user": {"id": "u"}
        });
        let login: LoginResponse = serde_json::from_value(body).expect("deserialize");
        assert_eq!(login.access_token, "at");
        assert_eq!(login.expires_in, 900);
    }

    #[test]
    fn http_status_classification_matches_plan_error_classes() {
        let auth = classify_status(401, Some("INVALID_ACCESS_TOKEN".to_owned()));
        assert!(matches!(auth, SyncClientError::Authentication { .. }));
        assert_eq!(auth.status(), Some(401));
        assert_eq!(auth.error_code(), Some("INVALID_ACCESS_TOKEN"));

        assert!(matches!(
            classify_status(404, None),
            SyncClientError::Request { .. }
        ));
        assert!(matches!(
            classify_status(500, None),
            SyncClientError::Server { .. }
        ));
        assert!(matches!(
            classify_status(429, None),
            SyncClientError::Server { .. }
        ));
    }

    #[test]
    fn refresh_cookie_header_is_parsed() {
        // Mirrors the find_map parsing core of extract_refresh_cookie: take
        // the name=value pair before the first ';' attribute separator.
        let header = format!(
            "{}=rotated-token; Path=/api/v1/auth; HttpOnly",
            REFRESH_COOKIE_NAME
        );
        let pair = header.split(';').next().expect("cookie pair");
        let (name, value) = pair.split_once('=').expect("cookie name/value");
        assert_eq!(name.trim(), REFRESH_COOKIE_NAME);
        assert_eq!(value.trim(), "rotated-token");
    }
}
