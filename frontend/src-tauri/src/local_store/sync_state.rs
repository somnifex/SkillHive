use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

use super::{validate_non_empty, LocalStore, LocalStoreError, LocalSyncState};

impl LocalStore {
    pub fn sync_state(&self) -> Result<LocalSyncState, LocalStoreError> {
        let connection = self.lock_connection()?;
        connection
            .query_row(
                r#"
                SELECT protocol_version, client_instance_id, device_id, server_user_id,
                       server_url, server_login_identity,
                       server_cursor, last_successful_push_at, last_successful_pull_at,
                       last_server_error
                FROM local_sync_state
                WHERE id = 1
                "#,
                [],
                |row| {
                    Ok(LocalSyncState {
                        protocol_version: row.get(0)?,
                        client_instance_id: row.get(1)?,
                        device_id: row.get(2)?,
                        server_user_id: row.get(3)?,
                        server_url: row.get(4)?,
                        server_login_identity: row.get(5)?,
                        server_cursor: row.get(6)?,
                        last_successful_push_at: row.get(7)?,
                        last_successful_pull_at: row.get(8)?,
                        last_server_error: row.get(9)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    /// Persists the stable local installation identity before server registration.
    /// Secrets are intentionally not stored in SQLite.
    pub fn set_client_instance_id(&self, client_instance_id: &str) -> Result<(), LocalStoreError> {
        validate_non_empty("client_instance_id", client_instance_id)?;
        let connection = self.lock_connection()?;
        connection.execute(
            r#"
            UPDATE local_sync_state
            SET client_instance_id = ?1, updated_at = CURRENT_TIMESTAMP
            WHERE id = 1
            "#,
            [client_instance_id],
        )?;
        Ok(())
    }

    pub fn record_device_registration(
        &self,
        client_instance_id: &str,
        device_id: &str,
        server_user_id: &str,
    ) -> Result<(), LocalStoreError> {
        for (field, value) in [
            ("client_instance_id", client_instance_id),
            ("device_id", device_id),
            ("server_user_id", server_user_id),
        ] {
            validate_non_empty(field, value)?;
        }

        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            r#"
            UPDATE local_sync_state
            SET client_instance_id = ?1,
                device_id = ?2,
                server_user_id = ?3,
                last_server_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = 1
            "#,
            params![client_instance_id, device_id, server_user_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Checks and reserves the server/account scope before credentials are
    /// changed by a desktop login.  Local rows are intentionally not
    /// re-keyed in place: if a different server or account is requested while
    /// any local state exists, the login is rejected instead of allowing old
    /// outbox rows/cursors to cross the trust boundary.
    pub fn prepare_login_scope(
        &self,
        server_url: &str,
        login_identity: &str,
    ) -> Result<(), LocalStoreError> {
        validate_non_empty("server_url", server_url)?;
        validate_non_empty("login_identity", login_identity)?;
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (bound_url, bound_identity, bound_user): (
            Option<String>,
            Option<String>,
            Option<String>,
        ) = transaction.query_row(
            "SELECT server_url, server_login_identity, server_user_id
             FROM local_sync_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let local_rows: i64 = transaction.query_row(
            "SELECT (SELECT COUNT(*) FROM local_skills) +
                    (SELECT COUNT(*) FROM local_mutations)",
            [],
            |row| row.get(0),
        )?;
        let url_matches = bound_url
            .as_deref()
            .map_or(true, |value| value == server_url);
        let identity_matches = bound_identity.as_deref().map_or(true, |value| {
            value.eq_ignore_ascii_case(login_identity.trim())
        });
        let scope_complete =
            bound_url.is_some() && bound_identity.is_some() && bound_user.is_some();
        let established_server = bound_url.is_some() && bound_user.is_some();
        if local_rows > 0 && established_server && !url_matches {
            return Err(LocalStoreError::InvalidInput(
                "local data belongs to another SkillHive server/account; export or clear it before switching"
                    .to_owned(),
            ));
        }

        // Do not persist the submitted username before authentication: it is
        // only a login attempt, not proof of account identity. An incomplete
        // legacy/pending scope is marked by the server URL alone so the
        // worker fails closed while authentication is in flight. The real
        // user ID is adopted only after the server accepts the credentials.
        if scope_complete && url_matches && identity_matches {
            transaction.commit()?;
            return Ok(());
        }
        let preserve_pending_user = url_matches && established_server;
        transaction.execute(
            r#"
            UPDATE local_sync_state
            SET server_url = ?1,
                server_login_identity = NULL,
                server_user_id = CASE WHEN ?2 THEN server_user_id ELSE NULL END,
                device_id = NULL,
                server_cursor = NULL,
                last_server_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = 1
            "#,
            params![server_url, preserve_pending_user],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Rebinds the authenticated device after login.  The server user ID is
    /// taken from the authenticated response, never inferred from the local
    /// client-instance ID.
    pub fn record_authenticated_device(
        &self,
        server_url: &str,
        login_identity: &str,
        client_instance_id: &str,
        device_id: &str,
        server_user_id: &str,
    ) -> Result<(), LocalStoreError> {
        for (field, value) in [
            ("server_url", server_url),
            ("login_identity", login_identity),
            ("client_instance_id", client_instance_id),
            ("device_id", device_id),
            ("server_user_id", server_user_id),
        ] {
            validate_non_empty(field, value)?;
        }
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (bound_url, bound_identity, bound_user): (
            Option<String>,
            Option<String>,
            Option<String>,
        ) = transaction.query_row(
            "SELECT server_url, server_login_identity, server_user_id
             FROM local_sync_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let local_rows: i64 = transaction.query_row(
            "SELECT (SELECT COUNT(*) FROM local_skills) +
                    (SELECT COUNT(*) FROM local_mutations)",
            [],
            |row| row.get(0),
        )?;
        let scope_complete =
            bound_url.is_some() && bound_identity.is_some() && bound_user.is_some();
        let expected_user = bound_url
            .as_ref()
            .and(bound_user.as_ref())
            .map(String::as_str);
        if bound_url
            .as_deref()
            .is_some_and(|value| value != server_url)
            || (local_rows > 0 && expected_user.is_some_and(|user| user != server_user_id))
        {
            return Err(LocalStoreError::InvalidInput(
                "authenticated server/account does not match the durable local scope".to_owned(),
            ));
        }
        let adopting_scope = !scope_complete;
        transaction.execute(
            r#"
            UPDATE local_sync_state
            SET server_url = ?1,
                server_login_identity = ?2,
                client_instance_id = ?3,
                device_id = ?4,
                server_user_id = ?5,
                server_cursor = CASE WHEN ?6 THEN NULL ELSE server_cursor END,
                last_successful_push_at = CASE WHEN ?6 THEN NULL ELSE last_successful_push_at END,
                last_successful_pull_at = CASE WHEN ?6 THEN NULL ELSE last_successful_pull_at END,
                last_server_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = 1
            "#,
            params![
                server_url,
                login_identity.trim(),
                client_instance_id,
                device_id,
                server_user_id,
                adopting_scope
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Prevents the worker from dispatching an old cursor/outbox against a
    /// replacement HTTP client.  A bound store with local rows must have a
    /// complete server/account identity.
    pub fn assert_server_scope(&self, server_url: &str) -> Result<(), LocalStoreError> {
        validate_non_empty("server_url", server_url)?;
        let state = self.sync_state()?;
        let connection = self.lock_connection()?;
        let local_rows: i64 = connection.query_row(
            "SELECT (SELECT COUNT(*) FROM local_skills) +
                    (SELECT COUNT(*) FROM local_mutations)",
            [],
            |row| row.get(0),
        )?;
        let scope_started = state.server_url.is_some()
            || state.server_login_identity.is_some()
            || state.server_user_id.is_some()
            || state.server_cursor.is_some();
        if state
            .server_url
            .as_deref()
            .is_some_and(|value| value != server_url)
            || (scope_started
                && (state.server_url.is_none()
                    || state.server_login_identity.is_none()
                    || state.server_user_id.is_none()))
            || (local_rows > 0 && (state.server_url.is_none() || state.server_user_id.is_none()))
        {
            return Err(LocalStoreError::InvalidInput(
                "local sync state is bound to a different or incomplete server/account scope"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    /// Changes the configured server only when no local state could be sent
    /// to the old server.  This is intentionally conservative and is used by
    /// the Tauri server-address command before replacing the HTTP client.
    pub fn prepare_server_switch(&self, server_url: &str) -> Result<(), LocalStoreError> {
        validate_non_empty("server_url", server_url)?;
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (bound_url, bound_identity, bound_user): (
            Option<String>,
            Option<String>,
            Option<String>,
        ) = transaction.query_row(
            "SELECT server_url, server_login_identity, server_user_id
             FROM local_sync_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let local_rows: i64 = transaction.query_row(
            "SELECT (SELECT COUNT(*) FROM local_skills) +
                    (SELECT COUNT(*) FROM local_mutations)",
            [],
            |row| row.get(0),
        )?;
        let scope_complete =
            bound_url.is_some() && bound_identity.is_some() && bound_user.is_some();
        let established_server = bound_url.is_some() && bound_user.is_some();
        if local_rows > 0
            && established_server
            && bound_url
                .as_deref()
                .is_some_and(|value| value != server_url)
        {
            return Err(LocalStoreError::InvalidInput(
                "local data belongs to another server; synchronize or clear it before switching"
                    .to_owned(),
            ));
        }
        if bound_url.as_deref() != Some(server_url) || !scope_complete {
            transaction.execute(
                r#"
                UPDATE local_sync_state
                SET server_url = ?1, server_login_identity = NULL,
                    server_user_id = NULL, device_id = NULL, server_cursor = NULL,
                    last_server_error = NULL, updated_at = CURRENT_TIMESTAMP
                WHERE id = 1
                "#,
                params![server_url],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Stops background synchronization immediately when the user logs out.
    /// The server URL is retained for the next explicit login, but the
    /// incomplete scope makes every worker cycle fail closed until a real
    /// authenticated user ID is adopted again.
    pub fn invalidate_authenticated_scope(&self) -> Result<(), LocalStoreError> {
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            r#"
            UPDATE local_sync_state
            SET server_login_identity = NULL,
                server_user_id = NULL,
                device_id = NULL,
                server_cursor = NULL,
                last_successful_push_at = NULL,
                last_successful_pull_at = NULL,
                last_server_error = 'desktop logout pending',
                updated_at = CURRENT_TIMESTAMP
            WHERE id = 1
            "#,
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn update_sync_cursor(&self, cursor: &str) -> Result<(), LocalStoreError> {
        validate_non_empty("server_cursor", cursor)?;
        let connection = self.lock_connection()?;
        connection.execute(
            r#"
            UPDATE local_sync_state
            SET server_cursor = ?1,
                last_successful_pull_at = CURRENT_TIMESTAMP,
                last_server_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = 1
            "#,
            [cursor],
        )?;
        Ok(())
    }

    pub fn record_sync_error(&self, error: &str) -> Result<(), LocalStoreError> {
        validate_non_empty("last_server_error", error)?;
        let connection = self.lock_connection()?;
        connection.execute(
            r#"
            UPDATE local_sync_state
            SET last_server_error = ?1, updated_at = CURRENT_TIMESTAMP
            WHERE id = 1
            "#,
            [error],
        )?;
        Ok(())
    }

    /// Marks one successful push cycle for diagnostics. Used by the sync
    /// orchestrator after its dispatch step made progress.
    pub fn record_push_success(&self) -> Result<(), LocalStoreError> {
        let connection = self.lock_connection()?;
        connection.execute(
            r#"
            UPDATE local_sync_state
            SET last_successful_push_at = CURRENT_TIMESTAMP,
                last_server_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = 1
            "#,
            [],
        )?;
        Ok(())
    }

    /// Returns the stable installation identity, creating it exactly once if
    /// absent. The UUID is committed inside the same immediate transaction
    /// that first observes the missing identity, so two racing callers
    /// cannot produce different identities — the conditional UPDATE means a
    /// lost race keeps the previously persisted value.
    pub fn ensure_client_instance_id(&self) -> Result<String, LocalStoreError> {
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT client_instance_id FROM local_sync_state WHERE id = 1",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        if let Some(existing) = existing {
            return Ok(existing); // read-only path: transaction drops without commit
        }

        let client_instance_id = Uuid::new_v4().to_string();
        transaction.execute(
            r#"
            UPDATE local_sync_state
            SET client_instance_id = ?1, updated_at = CURRENT_TIMESTAMP
            WHERE id = 1 AND client_instance_id IS NULL
            "#,
            [&client_instance_id],
        )?;
        transaction.commit()?;
        Ok(client_instance_id)
    }
}

/// Transaction-scoped cursor advance used by the pull apply transaction:
/// the page metadata writes and the cursor commit land in ONE SQLite
/// transaction, so an interrupted apply rolls back both.
pub(super) fn record_pull_cursor_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    cursor: &str,
) -> Result<(), LocalStoreError> {
    validate_non_empty("server_cursor", cursor)?;
    transaction.execute(
        r#"
        UPDATE local_sync_state
        SET server_cursor = ?1,
            last_successful_pull_at = CURRENT_TIMESTAMP,
            last_server_error = NULL,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = 1
        "#,
        params![cursor],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp_store() -> (tempfile::TempDir, LocalStore) {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(temp.path().join("skillhive.db")).expect("open store");
        (temp, store)
    }

    #[test]
    fn client_instance_id_is_stable_across_calls_and_restarts() {
        let (_temp, store) = open_temp_store();
        let first = store.ensure_client_instance_id().expect("ensure");
        let second = store.ensure_client_instance_id().expect("again");
        assert_eq!(first, second);

        let path = store.db_path().to_path_buf();
        drop(store);
        let reopened = LocalStore::open(path).expect("reopen store");
        assert_eq!(
            reopened.ensure_client_instance_id().expect("after restart"),
            first
        );
    }

    #[test]
    fn set_client_instance_id_overrides_generated_identity() {
        let (_temp, store) = open_temp_store();
        store
            .set_client_instance_id("fixed-instance-id")
            .expect("set");
        assert_eq!(
            store.ensure_client_instance_id().expect("ensure"),
            "fixed-instance-id"
        );
    }

    #[test]
    fn record_device_registration_updates_identity_and_error_state() {
        let (_temp, store) = open_temp_store();
        store
            .record_sync_error("previous failure")
            .expect("record error");
        store
            .record_device_registration("instance-1", "device-1", "user-1")
            .expect("register");

        let state = store.sync_state().expect("state");
        assert_eq!(state.client_instance_id.as_deref(), Some("instance-1"));
        assert_eq!(state.device_id.as_deref(), Some("device-1"));
        assert_eq!(state.server_user_id.as_deref(), Some("user-1"));
        assert_eq!(state.last_server_error, None);
    }

    #[test]
    fn update_sync_cursor_persists_and_clears_error() {
        let (_temp, store) = open_temp_store();
        store.record_sync_error("stale error").expect("error");
        store.update_sync_cursor("v1.AAAAAQ").expect("cursor");

        let state = store.sync_state().expect("state");
        assert_eq!(state.server_cursor.as_deref(), Some("v1.AAAAAQ"));
        assert_eq!(state.last_server_error, None);
    }

    #[test]
    fn record_push_success_stamps_timestamp_and_clears_error() {
        let (_temp, store) = open_temp_store();
        store.record_sync_error("stale error").expect("error");
        store.record_push_success().expect("push success");

        let state = store.sync_state().expect("state");
        assert!(state.last_successful_push_at.is_some());
        assert_eq!(state.last_server_error, None);
    }

    #[test]
    fn blank_input_is_rejected() {
        let (_temp, store) = open_temp_store();
        assert!(store.set_client_instance_id("   ").is_err());
        assert!(store.update_sync_cursor("").is_err());
        assert!(store.record_sync_error("").is_err());
        assert!(store
            .record_device_registration(" ", "device", "user")
            .is_err());
    }

    #[test]
    fn login_scope_is_bound_and_rejects_another_account_with_local_rows() {
        let (_temp, store) = open_temp_store();
        store
            .prepare_login_scope("https://one.example", "alice")
            .expect("reserve scope");
        store
            .record_authenticated_device(
                "https://one.example",
                "alice",
                "instance-1",
                "device-1",
                "user-1",
            )
            .expect("bind identity");
        store
            .commit_skill_edit(super::super::CommitSkillEdit {
                skill_id: "local-1".to_owned(),
                remote_id: None,
                name: "Local".to_owned(),
                slug: "local".to_owned(),
                workspace_path: std::env::temp_dir().join("skillhive-scope"),
                blob_hash: "sha256:local".to_owned(),
                base_revision: None,
                operation: super::super::MutationOperation::Create,
            })
            .expect("local mutation");

        store
            .prepare_login_scope("https://one.example", "bob")
            .expect("same-server account switch is checked after auth");
        assert!(store
            .record_authenticated_device(
                "https://one.example",
                "bob",
                "instance-1",
                "device-2",
                "user-2",
            )
            .is_err());
        assert!(store.prepare_server_switch("https://two.example").is_err());
        let state = store.sync_state().expect("state");
        assert_eq!(state.server_user_id.as_deref(), Some("user-1"));
        assert_eq!(state.server_url.as_deref(), Some("https://one.example"));
    }

    #[test]
    fn server_scope_switch_clears_cursor_when_store_is_empty() {
        let (_temp, store) = open_temp_store();
        store
            .prepare_login_scope("https://one.example", "alice")
            .expect("reserve");
        store
            .record_authenticated_device(
                "https://one.example",
                "alice",
                "instance-1",
                "device-1",
                "user-1",
            )
            .expect("bind");
        store.update_sync_cursor("v1.AAAAAQ").expect("cursor");
        store
            .prepare_server_switch("https://two.example")
            .expect("switch empty scope");
        let state = store.sync_state().expect("state");
        assert_eq!(state.server_url.as_deref(), Some("https://two.example"));
        assert_eq!(state.server_user_id, None);
        assert_eq!(state.device_id, None);
        assert_eq!(state.server_cursor, None);
    }

    #[test]
    fn logout_invalidation_blocks_worker_scope_before_network_result() {
        let (_temp, store) = open_temp_store();
        store
            .prepare_login_scope("https://one.example", "alice")
            .expect("reserve");
        store
            .record_authenticated_device(
                "https://one.example",
                "alice",
                "instance-1",
                "device-1",
                "user-1",
            )
            .expect("bind");
        store.update_sync_cursor("v1.AAAAAQ").expect("cursor");

        // This is committed before the logout HTTP request.  A network/5xx
        // result therefore cannot leave a worker with a valid durable scope.
        store.invalidate_authenticated_scope().expect("invalidate");
        let state = store.sync_state().expect("state");
        assert_eq!(state.server_url.as_deref(), Some("https://one.example"));
        assert_eq!(state.server_user_id, None);
        assert_eq!(state.server_cursor, None);
        assert!(store.assert_server_scope("https://one.example").is_err());
    }
}
