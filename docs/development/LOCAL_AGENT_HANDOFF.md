# SkillHive Local Agent Handoff

Status captured: 2026-09-04
Repository: `somnifex/SkillHive`
Development branch: `feat/desktop-foundation`
Integration PR: #3 (Draft)

This document is the engineering handoff for a local development agent. It records the target architecture, what has actually been implemented, what has only been planned, known unverified risks, and the exact order in which development should continue.

Read `AGENTS.md` first. Its constraints are mandatory.

---

## 1. Executive state

SkillHive is being converted from a web-oriented Skill management application into an enterprise desktop application with server-backed identity, authorization and authoritative remote storage.

The architectural direction is:

```text
SkillHive Cloud
  FastAPI modular monolith
  PostgreSQL / relational metadata
  accounts / groups / ACL / devices
  Skill revision history
  sync receipts / change feed
  object storage for complete Skill packages
            |
            | HTTPS sync protocol
            v
SkillHive Desktop
  Tauri 2 + React/Vite
  Rust privileged core
  SQLite durable metadata/outbox
  immutable SHA-256 blob store
  managed workspaces
  Agent discovery/adapters
  transactional deploy/uninstall
            |
            v
Claude / Codex / Gemini / OpenCode / OpenClaw / custom Agent Skill directories
```

The branch is ahead of `main` and was not behind `main` at the handoff checkpoint. `main` has intentionally not been modified directly.

No GitHub Actions workflow is enabled for this development phase.

---

## 2. Milestone status

| Milestone | Status | GitHub | Meaning |
| --- | --- | --- | --- |
| M0 Desktop/architecture foundation | COMPLETE | tracked by #1 | Architecture and desktop shell established |
| M1 Durable local desktop core | CODE COMPLETE / PENDING LOCAL VALIDATION | #2 | Static-reviewed implementation exists; not executed locally yet |
| M2 Cloud sync epic | IN PROGRESS | #4 | Detailed design and issue breakdown exist |
| M2.0 Shared server mutation path | CODE COMPLETE / PENDING LOCAL VALIDATION | #5 | Implemented and statically reviewed; tests not run |
| M2.1 Protocol/schema foundation | IN PROGRESS | #6 | Major schema/protocol pieces implemented; must be locally validated and finished |
| M2.2 Package/blob storage | PLANNED | #7 | Not started |
| M2.3 Device identity/credentials | PLANNED | #8 | Not started beyond schema placeholders/stubs |
| M2.4 Idempotent push | PLANNED | #9 | Not started |
| M2.5 Durable pull/change feed | PLANNED | #10 | Schema baseline exists; online implementation not started |
| M2.6 Desktop sync orchestrator | PLANNED | #11 | `sync.rs` is still a stub |
| M2.7 Conflict/reliability checkpoint | PLANNED | #12 | Design only |
| M3 Enterprise offline authorization | PLANNED | roadmap | Permission leases/revocation policy |
| M4 Production hardening | PLANNED | roadmap | Observability, updates, fault testing, release SLO |

`CODE COMPLETE` must not be relabeled `VERIFIED` until the local validation checklist has actually been run.

---

## 3. Product requirements that drove the architecture

The owner requirements are:

1. SkillHive is a **local desktop program**.
2. It still supports accounts and authoritative server storage.
3. Personal Skills may be stored locally within a bounded cache/workspace model.
4. Once network/auth are available, committed local mutations must eventually upload to the server.
5. Server permissions determine which remote content a user may see/use.
6. Common Agent Skill directories and arbitrary custom Skill directories must be supported.
7. Users must be able to selectively deploy Skills to individual Agent installations.
8. Enterprise stability is a first-class requirement: crash, retry, network loss, process restart, migration and permission change must not silently corrupt state.

The resulting top-level model is:

**cloud-authoritative + durable offline desktop mutations + bounded local cache + explicit sync + transactional deployment**.

This is intentionally not classic peer-to-peer/local-first data ownership.

---

## 4. Architectural invariants

These invariants should be treated as ADR-level requirements.

### 4.1 Local save durability

A user-visible local edit is successful only after:

```text
BEGIN SQLITE TRANSACTION
  write/update local Skill state
  append durable outbox mutation with stable mutation_id
COMMIT
```

If the process dies after the UI is told the edit was saved, the mutation must still be recoverable after restart.

### 4.2 Server authority

Server is authoritative for identity, authorization, device state, remote revisions, remote Skill history and sync acknowledgements.

Offline editing is allowed; offline permission escalation is not.

### 4.3 Idempotency

The logical server idempotency key is:

```text
(user_id, device_id, mutation_id)
```

The same mutation may be delivered one or many times but must have one logical server effect.

### 4.4 Optimistic concurrency

No Last-Write-Wins.

For update/delete:

```text
request.base_revision == skill.sync_revision
```

is required. Otherwise return conflict without modifying the server head.

### 4.5 Revision model

`sync_revision` is a technical positive integer, starting at 1.

It is not semantic version text.

Every server-visible mutation consumes one revision. Metadata-only revisions are allowed, so immutable SkillVersion revisions can contain gaps.

### 4.6 Server mutation atomicity

Future sync mutations must atomically commit:

```text
Skill domain mutation
+ revision
+ SkillVersion if applicable
+ audit row
+ sync_change_log row
+ sync_mutation_receipt row
```

Do not commit any of those separately.

### 4.7 Pull cursor durability

The server cursor comes from durable `sync_change_log.sequence` state.

Desktop writes a new cursor only in the same local transaction that applies the corresponding pulled page.

### 4.8 Complete Skill packages

The desktop model is a directory/package rather than only `skill_markdown`.

`SKILL.md`, scripts, references and assets must round-trip without losing bytes.

### 4.9 Transactional Agent deployment

Never overwrite an Agent directory file-by-file in place.

Deployment and uninstall use durable journals, verified staging/quarantine, atomic renames and recovery.

### 4.10 Security boundary

React/WebView does not own long-lived credentials and cannot request arbitrary local filesystem reads/writes.

Privileged path, storage, credential and deployment logic stays in Rust.

---

# PART A — IMPLEMENTED WORK

## 5. M0 — architecture and desktop foundation

Implemented on `feat/desktop-foundation`:

- Tauri 2 project under `frontend/src-tauri/`.
- Existing React 19/Vite 7 frontend retained rather than rewritten.
- Rust privileged-core module boundaries established.
- Desktop enterprise roadmap added at `docs/architecture/desktop-enterprise-roadmap.md`.
- M2 detailed plan added at `docs/architecture/m2-cloud-sync-plan.md`.
- No routine GitHub Actions workflow remains enabled.

Important: Tauri/Cargo runtime execution has not yet been locally validated.

---

## 6. M1 — durable local desktop core

M1 implementation exists and is statically reviewed. It has not yet received the required local build/runtime/fault validation.

### 6.1 Local SQLite store

Key files:

- `frontend/src-tauri/src/local_store.rs`
- `frontend/src-tauri/src/local_store/migrations.rs`
- `frontend/src-tauri/src/local_store/skills.rs`
- `frontend/src-tauri/src/local_store/mutations.rs`
- `frontend/src-tauri/src/local_store/deployments.rs`
- `frontend/src-tauri/src/local_store/cache.rs`
- `frontend/src-tauri/src/local_store/uninstall.rs`
- `frontend/src-tauri/src/local_store/sync_state.rs`

SQLite configuration includes:

- WAL;
- `synchronous=FULL`;
- foreign keys;
- busy timeout;
- forward-only application-managed schema migrations;
- a single `LocalStore` connection protected by a Rust mutex.

### 6.2 Durable outbox

Local Skill edit and outbox insertion are one SQLite transaction.

Mutation states include:

```text
pending
in_flight
acked
retryable_error
conflict
permission_denied
permanent_error
```

Process restart converts `in_flight` back to a retryable state while keeping the same mutation ID.

M2.1 now also adds per-Skill `local_sequence` ordering and persisted retry metadata.

### 6.3 Immutable blobs and snapshots

Key files:

- `frontend/src-tauri/src/blob_store.rs`
- `frontend/src-tauri/src/skill_snapshot.rs`
- `frontend/src-tauri/src/snapshot_verifier.rs`

Properties:

- SHA-256 content-addressed blobs;
- atomic temp-file write + fsync + rename;
- immutable snapshot manifest;
- no symlink following;
- path traversal rejection;
- portable filename restrictions;
- file count/size/package bounds;
- content identity does not include OS-specific Unix mode bits;
- snapshot capture checks for files changing during read.

### 6.4 Managed workspaces

Key files:

- `frontend/src-tauri/src/workspace.rs`
- `frontend/src-tauri/src/workspace/import.rs`

The frontend does not pass an arbitrary source directory to snapshot code.

Rust allocates a workspace under SkillHive-owned local storage based on the Skill ID.

Existing Agent Skills can be imported through a constrained source derived from a persisted/validated Agent profile plus a single directory name.

### 6.5 Agent adapter model

Key file:

- `frontend/src-tauri/src/agent.rs`

The model distinguishes:

- Agent descriptor/type;
- concrete Agent instance;
- Skill root path;
- built-in vs custom profile.

Known built-in discovery currently covers the intended roots for:

- Claude Code;
- Claude Desktop;
- Codex;
- Gemini;
- OpenCode;
- OpenClaw;
- Grok Build;
- unified `~/.agents/skills`;
- custom profiles.

A built-in profile root is validated against the Rust adapter; a frontend cannot redirect a built-in identity to an arbitrary root.

### 6.6 Transactional deployment

Key files:

- `frontend/src-tauri/src/deployment.rs`
- `frontend/src-tauri/src/uninstall.rs`

Install/update:

```text
Intent journal
-> materialize immutable snapshot to staging
-> verify staging
-> Prepared journal record
-> move current active to backup
-> atomic staging -> active
-> verify active
-> SQLite deployment catalog commit
-> journal ACK / cleanup
```

Recovery is conservative. If new content cannot be proven valid and an old backup exists, restore old state.

Uninstall:

```text
Intent journal
-> active -> quarantine rename
-> SQLite deployment catalog removal
-> permanent quarantine removal
-> journal ACK
```

Crash recovery consults durable journal + SQLite catalog rather than guessing.

### 6.7 Cache manager

Key file:

- `frontend/src-tauri/src/cache_manager.rs`

Protected from eviction:

- dirty/conflict/unresolved work;
- current managed workspace;
- pinned Skill;
- deployed Skill;
- snapshot referenced by any unacknowledged outbox mutation.

Shared blobs are protected through reference analysis.

Eviction first changes local state toward `remote_only` transactionally, then removes recoverable physical cache so crashes favor harmless leaked cache over false metadata.

### 6.8 M1 known unverified risks

The implementation was never executed in the previous development environment.

The local agent must verify at minimum:

- Rust compilation and tests;
- Tauri configuration/build requirements;
- whether a Tauri CLI dependency/installation is missing;
- SQLite migrations v1 -> v2 -> v3;
- deployment atomic rename behavior on target OSes;
- Windows lexical/symlink behavior;
- process-kill recovery at every journal phase;
- filesystem permission failures;
- actual Agent root discovery on supported environments.

`frontend/src-tauri/Cargo.lock` does not currently exist in the branch because Cargo was not run. Do not ignore it; generate and normally commit it after dependency validation.

---

## 7. M2.0 — unified server mutation transaction path

Status: CODE COMPLETE / PENDING LOCAL VALIDATION.

GitHub issue: #5.

### 7.1 Motivation

Originally, `PrivateSkillService`, `GlobalSkillService` and template instantiation directly constructed Skill/SkillVersion rows and called `session.commit()` independently.

That would have created separate correctness paths for browser REST and future desktop sync.

### 7.2 Implemented solution

New shared domain service:

- `backend/app/services/skill_mutations.py`

It centralizes:

- Skill construction;
- immutable SkillVersion construction;
- semantic version uniqueness checks;
- next patch-version derivation;
- metadata/head update;
- publish/status transitions;
- soft-delete transition;
- Skill audit emission;
- technical revision advancement added during M2.1 work.

It **does not commit or rollback the SQLAlchemy session**.

Callers own the transaction.

Existing facades continue handling authorization and preserve REST behavior:

- `backend/app/services/skills.py`
- `backend/app/services/global_skills.py`
- `backend/app/services/templates.py` for Skill instantiation.

Template CRUD itself is not a Skill resource and remains separate; only template instantiation that creates a real Skill is routed through the shared Skill mutation layer.

### 7.3 Contract tests added but not run

- `backend/tests/test_skill_mutations.py`

The tests specify that the caller can rollback all domain rows after a mutation and that durability happens only when the transaction owner commits.

---

## 8. M2.1 — protocol/schema foundation, current in-progress state

GitHub issue: #6.

This is the exact point where local development should resume.

### 8.1 Server ORM models already added

New file:

- `backend/app/models/sync.py`

Models:

#### `Device`

- user ID;
- stable client instance ID;
- display name/platform/app version;
- last seen;
- revoked timestamp;
- unique `(user_id, client_instance_id)`.

#### `SkillBlobObject`

Metadata only:

- SHA-256 hash;
- size;
- storage key;
- backend name;
- creation/verification timestamps.

Actual bytes are not stored in this row.

#### `SyncMutationReceipt`

Includes:

- user;
- device;
- mutation ID;
- operation/resource;
- result code/revision;
- persisted response payload.

Uniqueness:

```text
(user_id, device_id, mutation_id)
```

#### `SyncChangeLog`

Append-only cursor source:

- monotonic `sequence` primary key;
- resource ID/type/revision;
- upsert/delete operation;
- owner scope metadata;
- package manifest hash;
- compact metadata payload;
- timestamp.

SQLite uses an INTEGER variant for autoincrement behavior while server databases retain BIGINT intent.

### 8.2 Existing Skill ORM additions

`backend/app/models/domain.py` now includes:

`Skill`:

- `sync_revision`;
- `current_package_hash`.

`SkillVersion`:

- technical `revision`;
- `package_manifest_hash`;
- `package_size_bytes`;
- `(skill_id, revision)` uniqueness.

The revision model has been updated to start at 1.

### 8.3 Alembic migration already added

File:

- `backend/migrations/versions/b6a31d0f4c9e_add_sync_foundation.py`

The migration currently performs:

1. add Skill/SkillVersion sync fields as nullable transitional columns;
2. deterministic historical revision backfill ordered by `(created_at, id)` per Skill;
3. set each existing Skill baseline revision to at least 1;
4. convert revision fields to required constraints;
5. create device/blob/receipt/change-log tables;
6. create an initial change-feed baseline for existing Skills.

This migration is **handwritten and unexecuted**. Treat migration validation as high priority.

### 8.4 Protocol v1 schema already added

File:

- `backend/app/schemas/sync.py`

Includes:

- protocol version constant;
- device register/read wire models;
- Skill metadata model;
- blob descriptors;
- missing blob negotiation request/response;
- create/update/delete mutation envelope;
- mutation result/conflict/error envelopes;
- change item/change-page response.

Operation-shape validation currently enforces:

Create:

- no remote Skill ID;
- base revision null/0;
- package hash required;
- metadata required.

Update:

- remote Skill ID required;
- positive base revision required;
- package hash required;
- metadata required.

Delete:

- remote Skill ID required;
- positive base revision required;
- package hash forbidden.

### 8.5 Opaque cursor codec already added

File:

- `backend/app/services/sync_cursor.py`

The first protocol uses an opaque `v1.` URL-safe base64 encoding of a fixed-width sequence integer.

Clients must never parse/change cursor semantics themselves.

### 8.6 REST read schema additions

`backend/app/schemas/skill.py` now exposes additive fields:

Skill:

- `sync_revision`;
- `current_package_hash`.

SkillVersion:

- `revision`;
- `package_manifest_hash`;
- `package_size_bytes`.

Existing write request shapes are intentionally unchanged during migration.

### 8.7 Seed compatibility

`backend/app/db/seed.py` was adjusted so seeded Skill/SkillVersion rows satisfy the positive revision model.

Seed/bootstrap is not considered a normal online mutation/change-feed producer. Existing data is covered by migration baseline behavior.

### 8.8 Desktop SQLite schema v3 already added

`frontend/src-tauri/src/local_store/migrations.rs` now targets schema v3.

M2 fields include:

#### `local_mutations`

- stable `local_sequence` per Skill;
- `next_attempt_at`;
- `last_attempt_at`;
- structured server error code/details;
- acknowledged remote revision;
- acknowledged remote ID.

The v3 migration rebuilds the outbox while preserving existing IDs/state and assigning deterministic sequence order.

#### `local_sync_state`

Singleton row for:

- protocol version;
- stable local `client_instance_id`;
- registered device ID;
- server user ID;
- durable server cursor;
- successful push/pull diagnostic timestamps;
- last server error.

### 8.9 Desktop per-Skill causal ordering already added

`frontend/src-tauri/src/local_store/mutations.rs` now assigns monotonically increasing `local_sequence` values.

A mutation is dispatchable only when no earlier mutation for the same Skill remains non-ACKed.

This handles:

```text
local create
-> edit before create has remote ID
-> another edit
-> delete
```

without sending dependent operations out of order.

A conflict, permission denial or permanent failure on an earlier mutation intentionally blocks later same-Skill mutations until resolution.

### 8.10 Local sync state API already added

`frontend/src-tauri/src/local_store/sync_state.rs` provides persisted access/update helpers for the singleton local sync state.

No network sync worker consumes them yet.

### 8.11 Tests already added but not run

- `backend/tests/test_sync_protocol.py`
- updated `backend/tests/test_skill_mutations.py`
- Rust unit tests in local-store modules.

### 8.12 M2.1 work still required before #6 can be called complete

Do these next, in order:

1. Run static/lint/type/build checks locally and fix actual compile/type failures.
2. Verify ORM and Alembic migration schema parity, especially check constraints/index names and SQLite type variants.
3. Run fresh database migration to head.
4. Build a database at prior head `7f4c2b8a91de`, populate representative old Skills/versions/deleted Skills, then upgrade to `b6a31d0f4c9e`.
5. Verify deterministic historical revisions and baseline change events.
6. Verify the migration against PostgreSQL, not only SQLite.
7. If MySQL support is still intended as a supported backend, verify the migration there before calling the migration portable.
8. Validate `Base.metadata.create_all()` test schema matches Alembic head behavior.
9. Run protocol/cursor tests and fix Pydantic strict-mypy issues if any.
10. Run Rust schema v1 -> v2 -> v3 migration tests and per-Skill outbox ordering tests.
11. Update Issue #6 with exact validated scope and any compatibility decisions.

Do not begin the desktop network worker as a shortcut around this work.

---

# PART B — DETAILED FORWARD DEVELOPMENT PLAN

## 9. M2.2 — immutable Skill package storage and blob transport (#7)

Start only after M2.1 schema/protocol foundation is locally sound.

### Goal

Allow a complete M1 Skill snapshot to be uploaded, deduplicated, verified, stored server-side and rehydrated byte-identically.

### 9.1 Define `BlobStorage` abstraction

Server interface should conceptually support:

```text
exists(hash, size)
put_verified(hash, stream)
open(hash)
stat(hash)
delete(hash)   # GC only, not normal mutation path
```

Required backends:

1. local durable filesystem backend for development/single-node self-host mode;
2. S3-compatible backend contract for enterprise/multi-replica deployment.

Multi-replica correctness must not rely on node-local disk.

### 9.2 Validate hash namespace

Canonical wire hash format:

```text
sha256:<64 lowercase hex chars>
```

The storage key must be derived/validated server-side rather than accepting arbitrary client paths.

### 9.3 Missing-object negotiation

Implement a bounded endpoint conceptually equivalent to:

```text
POST /api/v1/sync/blobs/missing
```

Client submits hash/size descriptors for the immutable package closure.

Server returns only missing objects.

Never trust client-declared total size without independently validating each uploaded object and final manifest closure.

### 9.4 Verified upload

Upload must:

- stream with a hard byte bound;
- recompute SHA-256 on server;
- verify exact size;
- write to a temporary/non-addressable key;
- make the object addressable only after verification;
- safely handle concurrent duplicate uploads.

A mismatched object must not create a valid `SkillBlobObject` reference.

### 9.5 Manifest parser

The server must parse the same package semantics implemented by M1 `skill_snapshot.rs`.

Validate:

- file count;
- path normalization;
- no traversal;
- no symlinks;
- per-file size;
- total package size;
- unique paths;
- every referenced blob exists with matching size;
- `SKILL.md` policy where required.

Do not maintain two incompatible snapshot specifications. If a wire-level manifest schema needs to diverge, version it explicitly.

### 9.6 Legacy compatibility

Existing server Skills only have structured JSON and `skill_markdown`.

Provide a deterministic synthesis path for a legacy package containing at minimum `SKILL.md`.

Do not destructively rewrite old rows merely to satisfy desktop pull.

### 9.7 Garbage collection

Do not use mutable refcount as the sole deletion authority.

Plan mark-and-sweep roots from:

- live SkillVersion package manifests;
- retained change history required by cursor retention;
- unexpired mutation receipts if they reference package data;
- retention/legal policy.

Actual destructive GC can be deferred, but storage design must not block it.

### Exit criteria

- complete M1 snapshot round-trips byte-identically;
- duplicates dedupe by hash;
- corruption/mismatch cannot become referenced;
- production storage abstraction supports shared object storage.

---

## 10. M2.3 — device identity and secure credentials (#8)

Can partly proceed in parallel with late M2.2 after M2.1 is stable.

### Goal

Give each desktop installation a durable server device identity and an authenticated Rust network boundary without exposing long-lived credentials to React.

### 10.1 Stable local installation identity

Generate one `client_instance_id` and persist it in `local_sync_state`.

It survives app restart/reinstall only according to explicit product policy; do not regenerate on every startup.

### 10.2 Device registration

Server registration is idempotent by:

```text
(user_id, client_instance_id)
```

Return/update:

- server device UUID;
- display name;
- platform;
- app version;
- last seen;
- revoked state.

### 10.3 Device validation

All sync calls requiring device context must validate:

- authenticated user owns the device;
- device is not revoked.

Revocation must cause future sync authorization to fail without deleting the local outbox.

### 10.4 Credential storage

Long-lived refresh/session credential must be stored through OS facilities behind Rust.

Intended platform facilities:

- macOS Keychain;
- Windows Credential Manager;
- Linux Secret Service/keyring.

Choose a maintained Rust integration after checking current platform support locally.

Never store the long-lived refresh token in:

- WebView localStorage;
- IndexedDB;
- plaintext SQLite;
- config JSON;
- log files.

### 10.5 Rust HTTP client boundary

React should request high-level authenticated operations from Rust rather than reading the refresh credential.

Token refresh failure is not a mutation failure; preserve pending mutation ID/state.

### Exit criteria

- durable client instance identity;
- idempotent registered server device;
- Rust can authenticate without exposing refresh secret to WebView;
- revoked device cannot sync;
- pending local mutations survive auth/device failure.

---

## 11. M2.4 — idempotent mutation push (#9)

Depends on M2.0, M2.1, M2.2 and M2.3.

### 11.1 Endpoint

Initial protocol should use single-item semantics for correctness/observability:

```text
POST /api/v1/sync/mutations
```

Do not add batching until single mutation semantics are stable.

### 11.2 Processing order

Within one server transaction:

```text
authenticate user
-> validate active device
-> lookup existing receipt
-> if receipt exists, return stored result
-> resolve/lock target Skill where applicable
-> authorize current operation
-> validate base_revision
-> validate package closure
-> call shared SkillMutationService
-> append sync_change_log
-> append/retain audit from domain mutation
-> insert SyncMutationReceipt with full replayable response
-> commit once
-> return result
```

### 11.3 Row locking

Update/delete must serialize against concurrent server writers appropriately on PostgreSQL.

Do not emulate correctness with process-local mutexes.

### 11.4 Create mapping

Desktop has a local Skill ID before the server has a remote Skill ID.

On create ACK, one SQLite transaction must:

- mark mutation ACKed;
- save returned remote Skill ID;
- save returned server revision;
- update Skill state only if no later pending mutation exists.

Later same-Skill mutations then use the acknowledged remote ID/revision.

### 11.5 Lost response

If server commits but response is lost:

- client remains `in_flight` or later recovers to retryable;
- same mutation ID is resent;
- server finds receipt;
- same result is returned;
- no duplicate SkillVersion is created.

### Exit criteria

Test at minimum:

- duplicate request;
- timeout after server commit;
- restart after server ACK before local ACK;
- create followed by multiple offline edits;
- permission revoked before pending mutation upload;
- stale base revision.

---

## 12. M2.5 — durable pull and tombstones (#10)

### Goal

Make all server-visible Skill changes, including browser-originated mutations, observable through one durable ordered feed.

### 12.1 Event emission

M2.0 centralizes Skill mutation mechanics but does not yet emit live change-log rows.

Before pull is enabled, every browser REST Skill mutation that changes desktop-visible state must emit a `SyncChangeLog` entry in the same transaction.

This includes relevant:

- create;
- metadata update;
- new version;
- publish/status change where visible projection changes;
- delete tombstone;
- template instantiation that creates a Skill.

### 12.2 Pull endpoint

Concept:

```text
GET /api/v1/sync/changes?cursor=<opaque>&limit=<bounded>
```

Ordering by `sequence` only.

Return:

- bounded ordered changes;
- opaque next cursor;
- `has_more`;
- server time only for diagnostics.

### 12.3 Authorization projection

M2 online pull remains server-authoritative.

Initial recommended rollout:

1. private user-owned Skills read/write;
2. currently visible group/global Skills as read-only pull projection;
3. add shared writes only through explicit existing permission capability.

Do not send a private Skill body merely because the caller is a global administrator unless the product ACL explicitly allows that content access.

### 12.4 Pull apply transaction

For each page, desktop must transactionally:

- upsert/tombstone metadata;
- update remote revision/package identity;
- record local conflict state if remote changed under local dirty work;
- save the page cursor only after all page changes are durable.

If content is not cached, local state may be `remote_only`.

### 12.5 Tombstones

Deletion is explicit.

Never infer deletion because a row is missing from a query result.

Keep soft-deleted server rows/change history until cursor retention/GC policy is designed.

### Exit criteria

- browser-originated changes appear in desktop pull;
- interrupted pull resumes from last committed cursor;
- delete is deterministic;
- cursor never advances past uncommitted local application.

---

## 13. M2.6 — serialized desktop sync orchestrator (#11)

Only compose the worker after push/pull primitives are individually correct.

First implementation should use one conservative worker/coordinator.

Cycle:

```text
1. obtain/refresh authenticated Rust session
2. ensure active registered device
3. claim oldest eligible per-Skill mutation
4. negotiate/upload required package objects
5. submit mutation with stable mutation ID
6. transactionally persist ACK/conflict/error
7. continue bounded push work
8. pull change pages from durable cursor
9. apply page transactionally
10. reconcile cache/deployments after DB state is durable
```

### Retry

Use persisted exponential backoff + jitter.

`next_attempt_at` must survive restart.

No busy loop.

### Trigger sources

Reasonable triggers:

- app startup;
- login/session availability;
- network recovery signal;
- local mutation commit;
- explicit user sync request;
- bounded periodic wake if needed.

Correctness remains in SQLite/server receipts, not in worker memory.

### Shutdown

Cancellation at any step must preserve enough durable state for restart.

Do not delete/replace a claimed mutation ID during cancellation.

---

## 14. M2.7 — conflicts, error classification and reliability checkpoint (#12)

### Error classes

Retryable:

- network unavailable;
- timeout/unknown outcome;
- selected 408/425/429 cases;
- 5xx;
- temporary object-storage failure.

Conflict:

- base revision mismatch.

Permission denied:

- authenticated but operation no longer allowed.

Permanent validation:

- invalid metadata;
- malformed/unsupported package manifest;
- unsupported protocol version;
- unrecoverable client payload error.

Authentication/device failure must remain distinguishable from domain permission failure.

### Conflict state

Preserve:

- local immutable snapshot;
- local pending mutation;
- remote head revision;
- remote package hash;
- remote metadata needed for user choice.

Initial resolution modes:

1. keep local by creating a new update against latest remote revision after explicit confirmation;
2. keep remote and resolve/drop local pending chain explicitly;
3. create a new personal copy.

Automatic three-way package merge is not required for M2.

### Required reliability scenarios

Before M2 is VERIFIED, run locally:

- response lost after server commit;
- same mutation sent repeatedly;
- kill after HTTP ACK before SQLite ACK;
- create + multiple offline edits before first upload;
- two devices update same base revision;
- browser edit while desktop offline;
- stale device delete/update;
- interrupted/corrupt blob upload;
- 500/timeout/429 persisted backoff;
- permission removed before pending upload;
- pull interrupted between pages;
- restart with pending outbox/cursor state.

No scenario may silently lose an acknowledged local edit or overwrite a newer server revision.

---

## 15. M3 — enterprise permission/offline authorization plan

M3 is intentionally separate from M2.

M2 establishes device identity and online server authorization. M3 adds deterministic offline entitlement behavior.

### 15.1 Permission/entitlement lease

For managed/shared content, server may issue signed lease data conceptually containing:

```text
skill/resource ID
permission level
issued_at
expires_at
offline policy
policy version
```

Possible enterprise policy examples:

```text
Personal: offline unlimited
Team: offline 7 days
Confidential: offline 8 hours
Restricted: offline disabled
```

Do not implement this by simply caching an ACL forever.

### 15.2 Revocation reconciliation

Once the client observes revocation:

```text
mark inaccessible
-> remove Agent deployment
-> stop managed editing
-> purge decrypted workspace according to policy
-> evict restricted cache
-> audit locally/server-side as appropriate
```

If local dirty work exists, do not silently destroy it. Policy decides whether the user may discard, request access, or create/export a personal copy.

### 15.3 Security limitation

On an uncontrolled endpoint, server revocation cannot prove that a human has not copied already-visible plaintext elsewhere.

Do not promise impossible DRM guarantees.

For higher-sensitivity managed content, keep an architectural path toward encrypted local blobs and key destruction/crypto-shredding.

---

## 16. M4 — production hardening plan

### Observability

Add structured logging/metrics for:

- mutation lifecycle;
- retry class;
- sync latency;
- conflict count;
- pull lag/cursor age;
- package upload/download failures;
- deployment recovery;
- local DB migration/recovery;
- device/auth failures.

Do not log secrets or full sensitive Skill bodies by default.

### Update/migration safety

Desktop release needs:

- signed installers;
- signed/verified application updates;
- explicit DB schema version;
- migration checkpoint/backup policy;
- failed-migration safe startup mode;
- rollback/recovery documentation.

### Release SLO/correctness gates

Examples:

- zero acknowledged local edits lost;
- duplicate logical server mutations = 0;
- unauthorized committed server mutations = 0;
- transactional Agent deployment per Skill;
- restart recovers every documented incomplete state;
- API/sync availability targets measured separately from correctness invariants.

---

# PART C — DATA AND MODULE MAP

## 17. Server data model direction

Existing core tables remain.

M2 adds/extends:

```text
users
skills
  sync_revision
  current_package_hash
skill_versions
  revision
  package_manifest_hash
  package_size_bytes

devices
skill_blob_objects
sync_mutation_receipts
sync_change_log

audit_logs
groups
group_members
group_skill_grants
...
```

Large package bytes belong in the BlobStorage backend, not arbitrary JSON/BLOB columns in the relational DB.

---

## 18. Desktop local data model direction

Current local DB includes:

```text
schema_migrations
local_skills
local_mutations
agent_profiles
skill_deployments
local_cache_policy
local_sync_state
```

Runtime filesystem conceptually includes:

```text
<app-local-data>/
  skillhive.sqlite3
  blobs/
  workspaces/
  deployment-journal/
  uninstall-journal/
```

These runtime files must never be committed to Git.

---

## 19. Important module boundaries

### Backend

```text
backend/app/api/            HTTP boundary
backend/app/permissions/    authorization dependencies/policy
backend/app/services/       application/domain services
backend/app/repositories/   DB read/query helpers
backend/app/models/         ORM
backend/app/schemas/        request/response/wire models
backend/migrations/         schema history
```

`SkillMutationService` is not an authorization service and not a transaction owner.

### Desktop Rust

```text
agent.rs                    Agent descriptors/instances/root validation
blob_store.rs               immutable local content blobs
skill_snapshot.rs           capture/manifest
snapshot_verifier.rs        byte/hash verification
workspace.rs                managed editable workspaces
cache_manager.rs            bounded recoverable cache
deployment.rs               install/update transaction
uninstall.rs                uninstall transaction
local_store/*               durable SQLite state
credentials.rs              currently stub; M2.3
sync.rs                     currently stub; M2.6
lib.rs                      Tauri command/composition boundary
```

Keep cloud authorization out of Agent adapters.

---

# PART D — KNOWN RISKS AND DO-NOT-ASSUME LIST

## 20. Current implementation is not runtime-verified

Do not assume any of the following pass merely because files exist:

- Python import/type/lint correctness;
- Alembic upgrade/downgrade correctness;
- PostgreSQL migration portability;
- MySQL migration portability;
- Rust compile/lifetime/type correctness;
- Tauri command generation/config correctness;
- Windows/macOS/Linux deployment behavior;
- frontend build compatibility with desktop mode;
- crash-recovery tests.

The previous development session deliberately used static review only at the owner's request to avoid GitHub Actions quota.

---

## 21. M2 migration risk areas

Pay special attention to:

1. ORM `CheckConstraint`/index names matching migration metadata.
2. SQLite autoincrement semantics for `sync_change_log.sequence`.
3. JSON parameter insertion during historical change-feed backfill across dialects.
4. adding nullable columns, backfilling, then enforcing NOT NULL without table corruption.
5. deterministic ordering for historical versions with same timestamp.
6. Skills with zero versions.
7. deleted Skills in baseline change feed.
8. fresh schema created via ORM versus upgraded schema via Alembic.
9. downgrade only as a development aid; production safety should favor forward migrations.

---

## 22. Package compatibility risk

The current legacy server `SkillVersion.content` still exists and browser APIs use it.

Do not remove it during early M2.

M2.2 should introduce package storage incrementally and provide compatibility projection/synthesis.

Desktop package identity should eventually be canonical for desktop synchronization, while browser compatibility can continue reading projected `skill_markdown` during migration.

---

## 23. Tauri tooling risk

`frontend/package.json` currently has normal Vite scripts but no confirmed locally-tested Tauri CLI workflow.

`frontend/src-tauri/Cargo.toml` exists, but Cargo has not been run in this branch.

During local validation determine and document the supported developer command, for example either a locally installed Cargo Tauri CLI or a pinned project dependency.

Prefer a reproducible project-pinned developer toolchain over requiring undocumented global tools.

Do not add a cloud CI workflow merely to discover this.

---

## 24. Credentials implementation is not complete

`frontend/src-tauri/src/credentials.rs` is still a placeholder boundary.

Do not accidentally store tokens in SQLite/WebView while implementing networking before M2.3.

If a temporary developer auth method is required, keep it explicit, local-only and uncommitted; do not turn it into production architecture.

---

## 25. Sync worker implementation is not complete

`frontend/src-tauri/src/sync.rs` is still a placeholder.

Do not start by writing a loop that calls existing CRUD endpoints.

Existing CRUD endpoints do not provide idempotent receipt semantics, full package transfer or durable cursor pull.

Build M2.2–M2.5 primitives first.

---

# PART E — DEVELOPMENT WORKFLOW

## 26. Branch/commit policy

Continue on:

```text
feat/desktop-foundation
```

unless the owner explicitly requests a new branch.

Do not directly push development rewrites to `main`.

Prefer commit boundaries such as:

```text
fix(migrations): make sync revision backfill portable
fix(desktop): preserve causal outbox ordering across v3 migration
feat(sync): add verified filesystem blob storage
feat(sync): add device registration service
feat(sync): add idempotent mutation endpoint
```

Keep correctness changes reviewable.

---

## 27. Documentation/status policy

When a work package changes state:

- update its GitHub Issue;
- update this handoff milestone/status section;
- update the local validation checklist if a new failure mode was introduced;
- update architecture docs only when the architecture itself changes.

Do not spend development time rewriting legacy `makewiki/` or README text during active M2 unless specifically requested.

---

## 28. Local-only validation policy

The owner explicitly requested **no routine GitHub Actions validation** due to limited Actions quota.

Do not add `.github/workflows` unless explicitly authorized later.

All current validation should run on the local developer machine.

See `docs/development/LOCAL_VALIDATION_CHECKLIST.md` for exact commands/scenarios.

---

## 29. Ignore/generated/local files policy

`.gitignore` should cover all ordinary local artifacts.

Never commit:

```text
.env / environment secrets
local SQLite/DB files and WAL/SHM/journals
OS keyring exports / credentials / private keys
SkillHive runtime app-local data
Rust target/
node_modules/
frontend dist/coverage/.vite
Python virtualenv/cache/coverage
logs
editor/OS temp files
temporary package upload/download files
```

Do not ignore `Cargo.lock` for this desktop application. Once generated and locally validated, commit it for reproducibility.

Do not ignore source migrations, snapshot test fixtures, or intentionally versioned protocol fixtures simply because they contain the word `data` or `blob`.

---

# PART F — HANDOFF START PROCEDURE

## 30. First local-agent session

The new local agent should execute this sequence before making broad changes:

1. `git checkout feat/desktop-foundation`
2. confirm working tree is clean or identify owner-local uncommitted work;
3. read `AGENTS.md`;
4. read this file;
5. read `docs/development/LOCAL_VALIDATION_CHECKLIST.md`;
6. read `docs/architecture/m2-cloud-sync-plan.md`;
7. inspect Issue #6 requirements;
8. install/sync Python/frontend/Rust dependencies locally;
9. run the M2.1 validation subset;
10. fix compile/migration/test issues before introducing M2.2 features;
11. update #6 and this handoff when M2.1 reaches a truthful state.

Recommended immediate coding target:

> Make M2.1 schema/protocol foundation locally green on SQLite + PostgreSQL and make the desktop local schema v3/outbox tests green, then mark #6 verified/code-complete according to actual results.

Only after that start #7.

---

## 31. Handoff completion definition

This handoff is sufficient when a local agent can answer, from repository files alone:

- what SkillHive is becoming;
- which branch to use;
- what is implemented;
- what has never been run;
- which invariants cannot be violated;
- which files are generated/ignored;
- which issue is next;
- how M2.2–M4 should proceed;
- which local validation gates are required.

If any of those becomes stale, update this file as part of the development change that made it stale.
