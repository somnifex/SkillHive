/// Durable cloud synchronization boundary.
///
/// The implementation will consume the local mutation outbox and use
/// idempotent server mutation identifiers. This module must not directly
/// mutate agent deployment directories.
pub struct SyncEngine;
