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
                        server_cursor: row.get(4)?,
                        last_successful_push_at: row.get(5)?,
                        last_successful_pull_at: row.get(6)?,
                        last_server_error: row.get(7)?,
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
    fn blank_input_is_rejected() {
        let (_temp, store) = open_temp_store();
        assert!(store.set_client_instance_id("   ").is_err());
        assert!(store.update_sync_cursor("").is_err());
        assert!(store.record_sync_error("").is_err());
        assert!(store
            .record_device_registration(" ", "device", "user")
            .is_err());
    }
}
