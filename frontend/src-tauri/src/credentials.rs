//! OS-backed credential storage boundary (M2.3).
//!
//! Refresh/session secrets live in the platform credential facility —
//! Windows Credential Manager via the `keyring` crate — and never in
//! SQLite or WebView localStorage (plan §16). The store is namespaced per
//! service name so multiple SkillHive profiles on one machine cannot
//! overwrite each other.
//!
//! Error mapping is deliberate: a missing credential is a typed variant so
//! the sync worker can distinguish "never logged in" from "keychain
//! unavailable" without string matching.

use keyring::Entry;
use sha2::{Digest, Sha256};

/// Default service identifier under which credentials are stored.
pub const DEFAULT_SERVICE: &str = "app.skillhive.desktop";

/// The account (user) a stored secret belongs to.
pub const ACCOUNT_ACCESS_TOKEN: &str = "access-token";
pub const ACCOUNT_REFRESH_TOKEN: &str = "refresh-token";

/// Credential service name scoped to one server address.
///
/// Tokens must never be replayed against a different server than the one
/// that issued them, so each configured server address gets its own
/// namespace. The address itself never lands in the OS credential store
/// (privacy); only a short SHA-256 prefix does.
pub fn service_for_server(base_url: &str) -> String {
    let normalized = base_url.trim().trim_end_matches('/').to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    let prefix: String = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{DEFAULT_SERVICE}:{prefix}")
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialStoreError {
    #[error("credential not found for account {account:?}")]
    NotFound { account: String },
    #[error("OS credential store unavailable: {0}")]
    Backend(String),
    #[error("credential value is not valid UTF-8")]
    InvalidUtf8,
}

impl CredentialStoreError {
    fn from_keyring(account: &str, error: &keyring::Error) -> Self {
        match error {
            keyring::Error::NoEntry => Self::NotFound {
                account: account.to_owned(),
            },
            _ => Self::Backend(error_string(error)),
        }
    }
}

fn error_string(error: &keyring::Error) -> String {
    match error {
        keyring::Error::NoEntry => "credential entry missing".to_owned(),
        keyring::Error::Ambiguous(_) => "ambiguous credential entry".to_owned(),
        keyring::Error::PlatformFailure(_) => "platform credential store failure".to_owned(),
        keyring::Error::NoStorageAccess(_) => "credential store access denied".to_owned(),
        keyring::Error::Invalid(_, message) => format!("invalid credential input: {message}"),
        _ => format!("credential store error: {error}"),
    }
}

/// Facade over the OS credential facility.
///
/// Every operation is namespaced by `service` so tests (and future
/// multi-account support) can isolate entries without leaking between
/// profiles.
pub struct CredentialStore {
    service: String,
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self::new(DEFAULT_SERVICE)
    }
}

impl CredentialStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, account: &str) -> Result<Entry, CredentialStoreError> {
        Entry::new(&self.service, account).map_err(|error| {
            CredentialStoreError::Backend(format!("credential entry unavailable: {error}"))
        })
    }

    /// Persist or overwrite a secret. An existing value is replaced; the OS
    /// store is the single source of truth, not an append log.
    pub fn set_secret(&self, account: &str, secret: &str) -> Result<(), CredentialStoreError> {
        let entry = self.entry(account)?;
        entry
            .set_password(secret)
            .map_err(|error| CredentialStoreError::from_keyring(account, &error))
    }

    /// Read a secret. Returns [`CredentialStoreError::NotFound`] when absent.
    pub fn get_secret(&self, account: &str) -> Result<String, CredentialStoreError> {
        let entry = self.entry(account)?;
        match entry.get_password() {
            Ok(secret) => Ok(secret),
            Err(error) => Err(CredentialStoreError::from_keyring(account, &error)),
        }
    }

    /// Remove a secret. Deleting an already-absent credential succeeds so
    /// logout paths are idempotent.
    pub fn delete_secret(&self, account: &str) -> Result<(), CredentialStoreError> {
        let entry = self.entry(account)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(CredentialStoreError::from_keyring(account, &error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_service() -> String {
        format!("app.skillhive.test.{}", uuid::Uuid::new_v4())
    }

    #[test]
    fn server_namespaces_are_stable_and_distinct() {
        let first = service_for_server("http://127.0.0.1:8000");
        let same = service_for_server("http://127.0.0.1:8000/");
        let other = service_for_server("https://skillhive.example.com");
        assert_eq!(first, same, "trailing slash must not change the namespace");
        assert_ne!(first, other);
        assert!(first.starts_with(DEFAULT_SERVICE));
        assert!(!first.contains("127.0.0.1"), "raw address must not leak");
    }

    #[test]
    fn set_get_round_trip() {
        let store = CredentialStore::new(test_service());
        store
            .set_secret(ACCOUNT_REFRESH_TOKEN, "refresh-secret-1")
            .expect("set");
        assert_eq!(
            store.get_secret(ACCOUNT_REFRESH_TOKEN).expect("get"),
            "refresh-secret-1"
        );
        store.delete_secret(ACCOUNT_REFRESH_TOKEN).expect("delete");
    }

    #[test]
    fn overwrite_replaces_existing_value() {
        let store = CredentialStore::new(test_service());
        store.set_secret("token", "first").expect("first");
        store.set_secret("token", "second").expect("second");
        assert_eq!(store.get_secret("token").expect("get"), "second");
        store.delete_secret("token").expect("cleanup");
    }

    #[test]
    fn missing_credential_is_typed_not_found() {
        let store = CredentialStore::new(test_service());
        let error = store.get_secret("absent").expect_err("must be absent");
        assert!(matches!(error, CredentialStoreError::NotFound { .. }));
    }

    #[test]
    fn delete_is_idempotent() {
        let store = CredentialStore::new(test_service());
        store.delete_secret("never-stored").expect("absent delete");
        store.set_secret("transient", "value").expect("set");
        store.delete_secret("transient").expect("first delete");
        store.delete_secret("transient").expect("second delete");
        assert!(matches!(
            store.get_secret("transient"),
            Err(CredentialStoreError::NotFound { .. })
        ));
    }
}
