use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::{validate_non_empty, LocalStore, LocalStoreError};

/// The `deployment_prefs.skill_id` sentinel for the global default row.
const GLOBAL_PREF_KEY: &str = "";

const MAX_TARGETS: usize = 64;
const MAX_PROFILE_ID_LEN: usize = 128;

impl LocalStore {
    /// Global default deployment targets. Empty when the user has not
    /// configured any, in which case the deploy dialog falls back to
    /// per-call selection.
    pub fn default_deployment_targets(&self) -> Result<Vec<String>, LocalStoreError> {
        let connection = self.lock_connection()?;
        let stored: Option<String> = connection
            .query_row(
                "SELECT profile_ids FROM deployment_prefs WHERE skill_id = ?1",
                [GLOBAL_PREF_KEY],
                |row| row.get(0),
            )
            .optional()?;
        stored
            .map(|raw| parse_profile_ids(&raw))
            .transpose()
            .map(|value| value.unwrap_or_default())
    }

    pub fn set_default_deployment_targets(
        &self,
        profile_ids: &[String],
    ) -> Result<Vec<String>, LocalStoreError> {
        self.write_pref(GLOBAL_PREF_KEY, profile_ids)
    }

    /// Per-skill override, `None` when the skill has no override and should
    /// resolve to the global default.
    pub fn skill_deployment_targets(
        &self,
        skill_id: &str,
    ) -> Result<Option<Vec<String>>, LocalStoreError> {
        validate_skill_pref_key(skill_id)?;
        let connection = self.lock_connection()?;
        let stored: Option<String> = connection
            .query_row(
                "SELECT profile_ids FROM deployment_prefs WHERE skill_id = ?1",
                [skill_id],
                |row| row.get(0),
            )
            .optional()?;
        stored.map(|raw| parse_profile_ids(&raw)).transpose()
    }

    pub fn set_skill_deployment_targets(
        &self,
        skill_id: &str,
        profile_ids: &[String],
    ) -> Result<Vec<String>, LocalStoreError> {
        validate_skill_pref_key(skill_id)?;
        self.write_pref(skill_id, profile_ids)
    }

    pub fn clear_skill_deployment_targets(&self, skill_id: &str) -> Result<(), LocalStoreError> {
        validate_skill_pref_key(skill_id)?;
        let connection = self.lock_connection()?;
        connection.execute(
            "DELETE FROM deployment_prefs WHERE skill_id = ?1",
            [skill_id],
        )?;
        Ok(())
    }

    fn write_pref(
        &self,
        pref_key: &str,
        profile_ids: &[String],
    ) -> Result<Vec<String>, LocalStoreError> {
        let normalized = normalize_profile_ids(profile_ids)?;
        let encoded = serde_json::to_string(&normalized)
            .map_err(|error| LocalStoreError::InvalidInput(error.to_string()))?;

        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            r#"
            INSERT INTO deployment_prefs(skill_id, profile_ids, updated_at)
            VALUES (?1, ?2, CURRENT_TIMESTAMP)
            ON CONFLICT(skill_id) DO UPDATE SET
                profile_ids = excluded.profile_ids,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![pref_key, encoded],
        )?;
        transaction.commit()?;
        Ok(normalized)
    }
}

fn validate_skill_pref_key(skill_id: &str) -> Result<(), LocalStoreError> {
    validate_non_empty("skill_id", skill_id)
}

/// Deduplicates while preserving order and bounds the entry count so a
/// malformed WebView payload cannot balloon the row.
fn normalize_profile_ids(profile_ids: &[String]) -> Result<Vec<String>, LocalStoreError> {
    if profile_ids.len() > MAX_TARGETS {
        return Err(LocalStoreError::InvalidInput(format!(
            "deployment targets exceed the maximum of {MAX_TARGETS}"
        )));
    }
    let mut normalized: Vec<String> = Vec::new();
    for profile_id in profile_ids {
        validate_non_empty("agent profile id", profile_id)?;
        if profile_id.len() > MAX_PROFILE_ID_LEN {
            return Err(LocalStoreError::InvalidInput(format!(
                "agent profile id exceeds {MAX_PROFILE_ID_LEN} characters"
            )));
        }
        let trimmed = profile_id.trim();
        if !normalized.iter().any(|existing| existing == trimmed) {
            normalized.push(trimmed.to_owned());
        }
    }
    Ok(normalized)
}

fn parse_profile_ids(raw: &str) -> Result<Vec<String>, LocalStoreError> {
    let parsed: Vec<String> = serde_json::from_str(raw)
        .map_err(|error| LocalStoreError::InvalidPersistedState(error.to_string()))?;
    let mut normalized: Vec<String> = Vec::new();
    for profile_id in parsed {
        validate_non_empty("agent profile id", &profile_id)?;
        if profile_id.len() > MAX_PROFILE_ID_LEN {
            return Err(LocalStoreError::InvalidInput(format!(
                "agent profile id exceeds {MAX_PROFILE_ID_LEN} characters"
            )));
        }
        if !normalized.contains(&profile_id) {
            normalized.push(profile_id);
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_store() -> (tempfile::TempDir, LocalStore) {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(temp.path().join("prefs.db")).expect("open store");
        (temp, store)
    }

    #[test]
    fn default_targets_roundtrip_and_dedupe() {
        let (_temp, store) = open_store();

        assert!(store.default_deployment_targets().expect("read").is_empty());

        let saved = store
            .set_default_deployment_targets(&[
                "claude-code:default".to_owned(),
                "zcode:default".to_owned(),
                "claude-code:default".to_owned(),
                "  custom:my-dir:configured  ".to_owned(),
            ])
            .expect("set");
        assert_eq!(
            saved,
            vec![
                "claude-code:default".to_owned(),
                "zcode:default".to_owned(),
                "custom:my-dir:configured".to_owned(),
            ]
        );

        assert_eq!(
            store.default_deployment_targets().expect("read"),
            saved,
            "targets must survive a reopen"
        );
    }

    #[test]
    fn skill_override_resolves_independently_of_global() {
        let (_temp, store) = open_store();

        assert!(store
            .skill_deployment_targets("s1")
            .expect("read")
            .is_none());

        store
            .set_default_deployment_targets(&["gemini:default".to_owned()])
            .expect("global");
        store
            .set_skill_deployment_targets("s1", &["cursor:default".to_owned()])
            .expect("override");

        assert_eq!(
            store.skill_deployment_targets("s1").expect("read"),
            Some(vec!["cursor:default".to_owned()]),
        );
        assert_eq!(
            store.default_deployment_targets().expect("read"),
            vec!["gemini:default".to_owned()],
            "per-skill override must not touch the global row"
        );

        store.clear_skill_deployment_targets("s1").expect("clear");
        assert!(store
            .skill_deployment_targets("s1")
            .expect("read")
            .is_none());
    }

    #[test]
    fn blank_profile_ids_are_rejected() {
        let (_temp, store) = open_store();
        assert!(store
            .set_default_deployment_targets(&["  ".to_owned()])
            .is_err());
        assert!(store
            .set_skill_deployment_targets("s1", &[String::new()])
            .is_err());
    }

    #[test]
    fn skill_pref_requires_a_skill_id() {
        let (_temp, store) = open_store();
        assert!(store
            .set_skill_deployment_targets(" ", &["claude-code:default".to_owned()])
            .is_err());
        assert!(store.skill_deployment_targets(" ").is_err());
    }
}
