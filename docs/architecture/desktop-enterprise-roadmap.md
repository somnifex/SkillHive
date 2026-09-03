# SkillHive Desktop Enterprise Roadmap

## Product direction

SkillHive is moving from a browser-centric application to a desktop-first enterprise application with cloud-authoritative identity, authorization, storage, and synchronization.

The desktop client owns local durability, bounded cache, workspace editing, agent discovery, and transactional deployment. The server remains authoritative for accounts, organizations, permissions, skill metadata, immutable skill versions, synchronization receipts, and audit history.

## Architecture invariants

1. No acknowledged local edit may be lost after process crash, OS restart, temporary network loss, or application upgrade.
2. Every locally committed mutation must eventually synchronize when valid authentication, authorization, and network connectivity are available.
3. Sync operations must be idempotent and safe to retry.
4. Server authorization is authoritative; clients may not infer or extend access.
5. Offline access to managed skills must be bounded by an explicit permission lease policy.
6. Skill versions are immutable and content-addressed.
7. Agent deployment must be transactional: stage, validate, atomic swap, record.
8. Local cache eviction may never remove dirty or actively edited content.
9. Server correctness must not depend on volatile caches such as Redis.
10. Schema and application upgrades must be recoverable and migration-safe.

## Target system

### Desktop

- Tauri 2 shell around the existing React/Vite frontend.
- Rust desktop core as the privileged boundary for filesystem access, credentials, local database access, networking, and agent deployment.
- SQLite in WAL mode for durable local state.
- Immutable local blob store plus mutable workspaces.
- Transactional outbox for offline mutations.
- Sync state machine and conflict handling.
- Agent adapter registry with built-in and custom agent profiles.
- OS secure credential storage for refresh/session secrets.

### Server

Retain FastAPI as a modular monolith initially.

Modules to evolve toward:

- auth
- devices
- users / organizations / groups
- skills
- immutable skill_versions
- authorization / grants / entitlement leases
- sync
- audit

PostgreSQL remains the system of record. Skill package payloads should move toward object storage with hashes referenced from PostgreSQL.

## Development milestones

### M0 - Architecture and safety baseline

Goal: establish boundaries and prevent the desktop migration from becoming an uncontrolled rewrite.

Deliverables:

- Architecture Decision Records for cloud authority, local durability, immutable versions, sync, permissions, agent adapters, deployment transactions, and device identity.
- Desktop branch and build skeleton.
- CI baseline for frontend/backend and later Rust.
- Existing server behavior covered by regression tests before schema changes.

Exit criteria:

- Existing server tests remain green.
- Desktop shell launches the current frontend without changing business semantics.
- No production server schema migration yet.

### M1 - Desktop local core

Goal: make SkillHive useful locally without requiring cloud availability for every operation.

Deliverables:

- SQLite schema and migration runner.
- Local repository abstraction.
- Immutable blob store.
- Workspace manager.
- Local mutation outbox.
- Agent adapter interface.
- Built-in discovery for common agent skill directories.
- Custom agent directory profile.
- Transactional install/update/uninstall engine.

Initial adapters:

- Claude Code
- Codex
- generic Agent Skills compatible directory
- custom directory

Exit criteria:

- A local skill can be imported, edited, committed locally, and deployed to at least two agent profiles.
- Forced termination during deployment cannot leave a partially installed skill as the active deployment.
- Dirty local content survives restart.

### M2 - Cloud sync protocol

Goal: connect the durable local model to the existing account/server platform.

Deliverables:

- Device registration and device identity.
- Idempotent mutation API.
- Base revision and optimistic concurrency checks.
- Sync cursor for incremental pulls.
- Tombstones for deletion.
- Retryable/permanent sync error classification.
- Conflict preservation; no silent last-write-wins.

Exit criteria:

- Replaying the same mutation produces one server-side effect.
- Offline edits survive reconnect and upload exactly once logically.
- Concurrent remote/local edits produce an explicit conflict state.

### M3 - Enterprise authorization and offline policy

Goal: make local caching compatible with server-side permission revocation.

Deliverables:

- Permission/entitlement reconciliation before mutation upload.
- Signed or server-verifiable entitlement lease model.
- Configurable offline TTL policy for managed skills.
- Revocation reconciliation that removes active deployments and inaccessible cache according to policy.
- Preserve unauthorized local changes without silently uploading them; allow policy-controlled personal-copy handling.
- Device revoke flow.

Exit criteria:

- A revoked user cannot upload new managed-skill mutations after revocation is observed.
- Expired offline entitlement makes restricted content unavailable according to policy.
- Revocation reconciles all deployments deterministically.

### M4 - Reliability, observability, and release hardening

Deliverables:

- Structured logs and correlation IDs across desktop sync and server requests.
- Local recovery diagnostics and integrity checks.
- Metrics for sync latency, failures, conflicts, deployment failures, and authorization denials.
- Signed desktop release/update process.
- Database migration backup/recovery procedure.
- Fault-injection tests for network loss, crash, duplicate requests, server 5xx, authorization changes, and disk-full conditions.

Exit criteria:

- Release candidate meets defined correctness invariants and SLOs.
- Upgrade/downgrade recovery path is documented and tested.

## Reliability test matrix

Every major milestone must cover at least these failure modes where applicable:

- process killed after local commit but before upload
- process killed during agent deployment
- network disconnect during upload response
- duplicate mutation submission
- local disk full
- local SQLite lock contention
- remote authorization revoked while client is offline
- remote skill edited while local workspace is dirty
- corrupted cached blob
- application upgrade with pending mutations
- server 500/timeout/retry storm

## Coding boundaries

The React UI must not directly access filesystem paths, SQLite, refresh tokens, or agent directories.

Desktop privileged operations flow through the Rust core. Authorization decisions remain server-defined and are represented locally only as verifiable/reconcilable state.

Agent adapters handle capability and filesystem conventions only; they do not own authorization logic.

## Immediate development scope

The first implementation branch intentionally limits itself to M0 and the beginning of M1:

1. Introduce the Tauri desktop shell without deleting the web build.
2. Create the Rust module boundaries for local persistence, sync, credentials, and agent deployment.
3. Add the initial agent descriptor/profile model.
4. Preserve the existing FastAPI server and API behavior unchanged.
5. Add CI/build checks before deeper migration work.

This sequencing keeps the current server usable while the desktop architecture becomes testable incrementally.
