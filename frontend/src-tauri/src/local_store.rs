/// Local persistence boundary.
///
/// M1 will implement SQLite/WAL storage, schema migrations, immutable blobs,
/// workspaces, and the durable mutation outbox behind this module.
pub struct LocalStore;
