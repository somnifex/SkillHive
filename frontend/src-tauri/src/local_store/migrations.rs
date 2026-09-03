use rusqlite::{Connection, TransactionBehavior};

use super::LocalStoreError;

pub(super) const LATEST_SCHEMA_VERSION: i64 = 1;

const MIGRATIONS: &[(i64, &str)] = &[(
    1,
    r#"
    CREATE TABLE local_skills (
        id TEXT PRIMARY KEY NOT NULL,
        remote_id TEXT UNIQUE,
        name TEXT NOT NULL,
        slug TEXT NOT NULL,
        workspace_path TEXT NOT NULL,
        current_blob_hash TEXT NOT NULL,
        remote_revision INTEGER CHECK (remote_revision IS NULL OR remote_revision >= 0),
        sync_state TEXT NOT NULL CHECK (
            sync_state IN (
                'remote_only', 'synced', 'dirty', 'uploading', 'conflict',
                'sync_error', 'access_revoked', 'corrupted'
            )
        ),
        pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    );

    CREATE INDEX idx_local_skills_sync_state
        ON local_skills(sync_state);
    CREATE INDEX idx_local_skills_updated_at
        ON local_skills(updated_at);

    CREATE TABLE local_mutations (
        id TEXT PRIMARY KEY NOT NULL,
        skill_id TEXT NOT NULL,
        operation TEXT NOT NULL CHECK (operation IN ('create', 'update', 'delete')),
        base_revision INTEGER CHECK (base_revision IS NULL OR base_revision >= 0),
        payload_hash TEXT NOT NULL,
        state TEXT NOT NULL CHECK (
            state IN (
                'pending', 'in_flight', 'acked', 'retryable_error', 'conflict',
                'permission_denied', 'permanent_error'
            )
        ),
        retry_count INTEGER NOT NULL DEFAULT 0 CHECK (retry_count >= 0),
        last_error TEXT,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        FOREIGN KEY(skill_id) REFERENCES local_skills(id) ON DELETE CASCADE
    );

    CREATE INDEX idx_local_mutations_dispatch
        ON local_mutations(state, created_at);
    CREATE INDEX idx_local_mutations_skill
        ON local_mutations(skill_id, created_at);

    CREATE TABLE agent_profiles (
        id TEXT PRIMARY KEY NOT NULL,
        descriptor_id TEXT NOT NULL,
        display_name TEXT NOT NULL,
        skill_root TEXT NOT NULL UNIQUE,
        enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
        is_custom INTEGER NOT NULL DEFAULT 0 CHECK (is_custom IN (0, 1)),
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    );

    CREATE INDEX idx_agent_profiles_descriptor
        ON agent_profiles(descriptor_id, enabled);

    CREATE TABLE skill_deployments (
        skill_id TEXT NOT NULL,
        agent_profile_id TEXT NOT NULL,
        deployed_blob_hash TEXT NOT NULL,
        target_path TEXT NOT NULL,
        state TEXT NOT NULL CHECK (
            state IN (
                'installing', 'installed', 'updating', 'removing', 'modified',
                'missing', 'failed', 'revoked'
            )
        ),
        last_error TEXT,
        last_verified_at TEXT,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        PRIMARY KEY(skill_id, agent_profile_id),
        FOREIGN KEY(skill_id) REFERENCES local_skills(id) ON DELETE CASCADE,
        FOREIGN KEY(agent_profile_id) REFERENCES agent_profiles(id) ON DELETE CASCADE
    );

    CREATE INDEX idx_skill_deployments_state
        ON skill_deployments(state);
    "#,
)];

pub(super) fn migrate(connection: &mut Connection) -> Result<(), LocalStoreError> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )?;

    let current_version: i64 = connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;

    if current_version > LATEST_SCHEMA_VERSION {
        return Err(LocalStoreError::UnsupportedSchema {
            found: current_version,
            supported: LATEST_SCHEMA_VERSION,
        });
    }

    for (version, sql) in MIGRATIONS {
        if *version <= current_version {
            continue;
        }

        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations(version) VALUES (?1)",
            [version],
        )?;
        transaction.commit()?;
    }

    Ok(())
}
