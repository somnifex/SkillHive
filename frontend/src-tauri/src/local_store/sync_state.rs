use rusqlite::{params, TransactionBehavior};

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
        *,
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
}
