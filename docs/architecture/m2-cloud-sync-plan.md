# M2 Cloud Sync Protocol — Detailed Engineering Plan

Status: planned after M1 code-complete checkpoint

## 1. Objective

M2 connects the M1 durable desktop core to the existing FastAPI account/server platform without weakening M1 correctness guarantees.

The target model is:

- server-authoritative identity, authorization, metadata, versions and sync history;
- durable offline desktop edits represented by immutable snapshots plus an outbox;
- logically exactly-once server effects through idempotent mutation receipts;
- optimistic concurrency with explicit conflicts, never silent last-write-wins;
- incremental pull through a durable server cursor;
- complete Agent Skill package synchronization, including `SKILL.md`, scripts, references and assets;
- existing browser REST APIs remain usable throughout the migration.

M2 does **not** implement offline entitlement leases or enterprise revocation TTL. Those remain M3, but M2 must preserve the authorization boundaries needed by M3.

## 2. Current code constraints

The plan is based on the current backend implementation rather than documentation assumptions.

### 2.1 Existing server model

`Skill` currently owns metadata, `current_version_id`, ownership, status and soft-delete state.

`SkillVersion` currently stores:

- a human-readable `version` string;
- `content` JSON;
- `manifest` JSON;
- dependency config;
- change log;
- status and creator.

The current `SkillContent` schema contains structured prompt fields plus `skill_markdown`. It cannot represent a general Agent Skill directory containing scripts, references, assets or arbitrary additional files.

### 2.2 Existing mutation path

`PrivateSkillService` performs create/update/delete/version operations directly and commits inside service methods. Existing browser REST endpoints call this service.

M2 must not create a second independent mutation implementation. Browser REST mutations and desktop sync mutations must converge on one transactional domain mutation layer so that revisions, change feed entries, audit records and immutable package references cannot diverge depending on which API was used.

### 2.3 Existing authorization

Private Skill writes are already owner-scoped through `PrivateSkillService`/repository lookups. Group and global skill access exists through the existing group/global service layers and grants.

M2 will preserve these server-side authorization checks. M3 later adds offline entitlement leases and deterministic local revocation behavior.

### 2.4 Existing authentication

`TokenSession` represents refresh sessions but there is currently no first-class device entity. M2 adds device identity while retaining the existing user authentication model.

## 3. M2 correctness invariants

The following are release-blocking invariants.

1. A successfully committed local mutation remains retryable until the desktop durably records a server acknowledgement.
2. Replaying the same `(user_id, device_id, mutation_id)` produces one logical server-side effect.
3. A lost HTTP response cannot cause a duplicate Skill version or duplicate delete.
4. Server authorization is evaluated when each mutation is processed; a client cannot extend its permissions.
5. `base_revision` mismatch never overwrites the server head. It produces an explicit conflict result.
6. Server revisions are technical monotonic concurrency identifiers and are independent of human semantic-version labels.
7. A server mutation acknowledgement is returned only after the domain change, change-feed row, audit row and mutation receipt are committed atomically.
8. A change cursor advances only over durable committed change-feed entries.
9. A Skill version references only blobs whose hashes and sizes were verified by the server.
10. Deletion is represented by a tombstone/change event; absence is not used as a deletion signal.
11. Existing REST-originated changes are visible to desktop pull just like sync-originated changes.
12. Redis or any volatile cache may improve performance but may not be required for correctness.

## 4. Protocol identity model

M2 uses four distinct identities.

### User identity

Existing server `users.id`. Authentication and authorization remain user-scoped.

### Device identity

Server-issued UUID bound to one user and one desktop installation.

Desktop stores a stable local installation UUID before registration. Registration returns the server device UUID.

### Mutation identity

Desktop-generated UUID created when the M1 outbox transaction is committed. It never changes across retry/restart.

Idempotency key:

`(user_id, device_id, mutation_id)`

### Resource revision

Monotonic integer maintained by the server for each mutable Skill resource.

`revision` is not the existing semantic `version` string.

## 5. Proposed server schema

Schema names are provisional but responsibilities are fixed.

### 5.1 `devices`

Fields:

- `id` UUID PK
- `user_id` FK users
- `client_instance_id` UUID, unique per user
- `display_name`
- `platform`
- `app_version`
- `created_at`
- `last_seen_at`
- `revoked_at`

Unique constraint:

`(user_id, client_instance_id)`

M3 may extend this table with device public keys and entitlement-signing metadata.

### 5.2 Skill revision fields

Add to `skills`:

- `sync_revision BIGINT NOT NULL DEFAULT 0`
- optional `current_package_hash`

`sync_revision` changes for every server-authoritative mutation that changes the desktop-visible representation.

### 5.3 Skill version package fields

Add to `skill_versions`:

- `revision BIGINT NOT NULL`
- `package_manifest_hash VARCHAR(...) NULL`
- `package_size_bytes BIGINT NULL`

Unique constraints:

- `(skill_id, revision)`
- retain `(skill_id, version)` for current browser compatibility

Existing `content` JSON remains during migration. A sync-created version may populate legacy `skill_markdown` from `SKILL.md` for compatibility, but the package manifest becomes the canonical representation for desktop synchronization.

### 5.4 `skill_blob_objects`

Metadata only; actual bytes are stored through a storage backend abstraction.

Fields:

- `hash` SHA-256 PK
- `size_bytes`
- `storage_key`
- `storage_backend`
- `created_at`
- optional integrity verification timestamp

Do not use mutable reference counts as the sole deletion authority. Blob garbage collection should be mark-and-sweep from live manifests and retained mutation/change history.

### 5.5 `sync_mutation_receipts`

Fields:

- `id`
- `user_id`
- `device_id`
- `mutation_id`
- `operation`
- `resource_type`
- `resource_id`
- `result_code`
- `result_revision`
- `response_payload` JSON
- `created_at`

Unique:

`(user_id, device_id, mutation_id)`

A retry first reads this table. If a receipt already exists, the server returns the previously committed logical result without re-running the mutation.

### 5.6 `sync_change_log`

Append-only committed change stream.

Fields:

- `sequence BIGINT` monotonic PK/cursor source
- `resource_type`
- `resource_id`
- `resource_revision`
- `operation` (`upsert`, `delete`, later grant-related events)
- `owner_user_id` / scope metadata needed for pull filtering
- `package_manifest_hash` where applicable
- compact metadata payload
- `created_at`

Every browser or sync mutation that changes a Skill must insert its change-log row in the same DB transaction.

### 5.7 Tombstones

Continue using `skills.deleted_at`/deleted status rather than hard deleting a Skill during M2.

A delete creates a `sync_change_log(operation='delete')` entry with the final revision.

Hard deletion is prohibited until retention/cursor policy is defined. M3/M4 can add safe garbage collection.

## 6. Server package storage

M1 snapshots are content-addressed manifests referencing SHA-256 file blobs. M2 should preserve that exact model across the network.

Introduce a server `BlobStorage` interface with operations conceptually equivalent to:

- `exists(hash, size)`
- `put_verified(hash, bytes)`
- `open(hash)`
- `delete(hash)` for later GC only

Backends:

- development/self-host single-node backend: local filesystem under a dedicated data directory;
- production/enterprise backend: S3-compatible object storage.

PostgreSQL/MySQL/SQLite store blob metadata and package references, not large package bytes.

Local-filesystem storage must be documented/configured as single-node unless backed by shared durable storage. Multi-replica production must use a shared storage backend.

## 7. Snapshot transport protocol

The client must not repeatedly upload content that the server already owns.

### Step A — manifest closure preparation

Desktop reads the immutable M1 snapshot manifest and computes the closure:

- manifest hash;
- each file blob hash;
- each size.

### Step B — missing-object negotiation

Endpoint concept:

`POST /api/v1/sync/blobs/missing`

Request contains bounded hash/size entries.

Response lists only missing objects.

Limits must bound:

- hashes per request;
- individual blob size;
- total declared package size;
- manifest file count.

### Step C — upload missing blobs

For the initial implementation, authenticated bounded streaming upload endpoints are acceptable.

The storage layer recomputes SHA-256 while streaming and rejects digest/size mismatch before the object becomes addressable.

S3 deployments may later use presigned uploads, but mutation correctness cannot depend on trusting a client-reported upload result.

### Step D — commit mutation

Only after required objects are present does the desktop submit the mutation referencing `package_manifest_hash`.

The server parses and validates the manifest and verifies that the complete blob closure exists before committing the Skill version.

## 8. Mutation API

Endpoint concept:

`POST /api/v1/sync/mutations`

Start with single-mutation semantics for correctness and observability. Add bounded batching only after the single-item protocol is stable.

Request envelope:

```json
{
  "protocolVersion": 1,
  "deviceId": "...",
  "mutationId": "...",
  "operation": "create|update|delete",
  "clientSkillId": "...",
  "remoteSkillId": "... or null",
  "baseRevision": 12,
  "packageManifestHash": "sha256:...",
  "metadata": {
    "name": "...",
    "slug": "...",
    "description": "...",
    "category": "...",
    "tags": []
  }
}
```

Delete does not require package upload but still carries `baseRevision`.

### Create

- authenticate user;
- validate active device;
- check existing receipt;
- validate metadata/slug;
- validate package closure;
- create server Skill + immutable SkillVersion;
- assign revision 1;
- insert change event;
- write audit;
- insert receipt containing `remoteSkillId` and revision;
- commit;
- return receipt result.

### Update

- authenticate and validate device;
- existing receipt short-circuit;
- lock target Skill row;
- authorize write using current server rules;
- require `baseRevision == skill.sync_revision`;
- validate package closure;
- create immutable SkillVersion with next revision;
- update Skill metadata/current version/revision;
- insert change event + audit + receipt;
- commit.

### Delete

- same receipt/device/auth flow;
- lock resource;
- authorize;
- require matching `baseRevision`;
- increment revision;
- mark deleted/tombstoned;
- insert delete change event + audit + receipt;
- commit.

## 9. Idempotency transaction pattern

For every mutation:

1. authenticate user and device;
2. begin DB transaction;
3. query receipt by `(user, device, mutation)`;
4. if receipt exists, return its persisted response;
5. lock/check resource as required;
6. evaluate authorization;
7. evaluate base revision;
8. apply domain mutation;
9. append change event;
10. append audit event;
11. append mutation receipt;
12. commit transaction;
13. return result.

A timeout after step 12 is safe: the desktop retries the same mutation ID and receives the stored receipt.

## 10. Optimistic concurrency and conflicts

No Last Write Wins.

If desktop sends `baseRevision = 12` while server head is 13:

- server makes no content change;
- result is `conflict`;
- response includes current remote revision, current manifest hash and current metadata necessary for resolution;
- desktop marks the outbox row `conflict` and local Skill `conflict`;
- local snapshot remains pinned/protected from cache eviction.

Conflict resolution is a new mutation based on the latest remote revision. Initial resolution modes can remain simple:

- keep local as a new update after explicit user confirmation;
- keep remote and discard local pending mutation;
- create a new personal copy.

Automatic content merge is not an M2 requirement.

## 11. Pull protocol

Endpoint concept:

`GET /api/v1/sync/changes?cursor=<opaque>&limit=<bounded>`

Response:

- ordered change entries;
- `nextCursor`;
- `hasMore`;
- server timestamp for diagnostics only.

The cursor is an opaque encoding of durable `sync_change_log.sequence` state. Client wall clock is never used as a correctness cursor.

Pull ordering is stable by sequence.

For an upsert entry the client receives enough metadata to decide whether it needs the referenced package manifest/blob closure.

For a delete entry it receives a tombstone with resource ID and final revision.

### Existing REST changes

Before enabling desktop pull, all current browser create/update/delete/version paths must emit the same revision/change-feed events. Otherwise desktop state can silently miss browser edits.

This requires refactoring the current service mutation code into a shared domain mutation layer before the sync endpoint is considered complete.

## 12. Visibility and permissions in M2

M2 does not implement offline permission leases, but online visibility still remains server-authoritative.

Recommended sequence:

1. full read/write sync for user-owned private Skills;
2. read-only pull projection for group/global Skills the user can currently access;
3. write mutations for shared resources only where the existing server permission service explicitly grants write capability;
4. M3 adds lease issuance, expiry and deterministic offline revocation/purge.

The pull API must not send private Skill bodies merely because the requesting user is a global administrator. It must reuse the existing ownership/grant access semantics.

## 13. Desktop schema evolution (local schema v3+)

M2 needs additional local durable state.

### `local_sync_state`

Singleton fields:

- protocol version;
- server cursor;
- last successful push/pull timestamps for diagnostics;
- last server error;
- registered device ID.

### `local_mutations` additions

Add:

- per-Skill monotonic `local_sequence`;
- `next_attempt_at`;
- `last_attempt_at`;
- structured `server_error_code`;
- optional acknowledged `remote_revision`;
- optional acknowledged `remote_id`.

Dispatch rule changes from “oldest rows globally” to:

- only the oldest unacknowledged mutation per Skill is dispatchable;
- later mutations for the same Skill wait until the previous mutation is ACKed or explicitly resolved;
- unrelated Skills may sync concurrently later, but M2 should begin with one serialized network worker.

This is required because a locally created Skill may accumulate edits before its create mutation has returned a server resource ID.

### Local Skill acknowledgement

ACK processing must be a SQLite transaction that:

- marks mutation ACKed;
- records server resource ID if this was create;
- records server revision;
- updates local Skill sync state only if no later pending mutation exists;
- persists new pull/sync metadata as appropriate.

The HTTP worker must not consider a mutation complete before this local ACK transaction commits.

## 14. Desktop sync worker state machine

One serialized M2 worker is preferred initially.

Cycle:

1. confirm network reachability opportunistically;
2. load/refresh authenticated session through credential boundary;
3. ensure registered active device;
4. process earliest dispatchable mutation;
5. negotiate/upload missing blobs when required;
6. submit mutation using stable mutation ID;
7. durably apply ACK/conflict/error locally;
8. repeat push until bounded budget exhausted;
9. pull changes from durable cursor;
10. apply each pull page transactionally;
11. reconcile local cache/deployments only after local database state is durable.

No busy loop. Retry is bounded exponential backoff with jitter and persisted `next_attempt_at`.

## 15. Error classification

### Retryable

- network unavailable;
- timeout / response lost;
- HTTP 408/425/429 where appropriate;
- HTTP 5xx;
- temporary object-storage failure.

Retry keeps the same mutation ID.

### Conflict

- base revision mismatch;
- server returns current head details;
- requires explicit resolution.

### Permission denied

- authenticated but no longer authorized;
- mutation becomes `permission_denied`;
- local work is preserved;
- M3 determines managed-content revocation/export policy.

### Permanent validation error

- invalid metadata;
- package manifest invalid;
- unsupported protocol version;
- irrecoverably malformed request.

Mutation becomes `permanent_error`; no automatic retry storm.

### Authentication/device error

Handled separately from mutation failure so refresh/device re-registration can occur without changing the mutation ID.

## 16. Credential and device storage

M2 must replace the current desktop credential stub with OS-backed secret storage before persistent refresh credentials are used.

Target:

- macOS Keychain;
- Windows Credential Manager;
- Linux Secret Service/libsecret-compatible keyring.

React/WebView receives only short-lived operation results, never the long-lived refresh credential.

Device registration metadata may live in SQLite; secrets remain in the OS credential store.

## 17. Backward compatibility strategy

The existing web frontend and REST API remain functional during M2.

Required compatibility steps:

- new DB fields have safe migration defaults;
- old SkillVersion rows may have null package hashes;
- first desktop pull of a legacy Skill synthesizes a package containing at minimum `SKILL.md` from existing `skill_markdown`/content;
- browser writes invoke the shared domain mutation service and therefore receive server revisions/change events;
- sync writes populate enough legacy content for the current web UI to continue displaying the Skill.

No flag day migration.

## 18. Implementation work packages

### M2.0 — Domain mutation refactor

Refactor current `PrivateSkillService` mutation internals into a shared transactional domain service. Preserve existing REST behavior while creating one place that owns revision, version creation, audit and change-feed emission.

### M2.1 — Protocol/schema foundation

- server Alembic migration for devices, revisions, receipts, change log and blob metadata;
- Pydantic sync protocol v1 schemas;
- desktop SQLite v3 migration for device/cursor/outbox sequencing;
- protocol compatibility/version negotiation.

### M2.2 — Package storage and transport

- storage backend abstraction;
- local dev storage backend;
- S3-compatible production backend contract;
- missing-hash negotiation;
- verified streaming upload/download;
- server manifest validation identical in constraints to desktop snapshot validation.

### M2.3 — Device identity and secure credentials

- device registration/revocation-aware server endpoints;
- desktop stable client installation ID;
- OS credential store implementation;
- authenticated Rust HTTP client boundary.

### M2.4 — Idempotent push

- mutation receipt transaction;
- create/update/delete handlers;
- base revision enforcement;
- REST and sync mutation paths share domain mutation code;
- local ACK transaction and per-Skill mutation sequencing.

### M2.5 — Incremental pull

- append-only change feed;
- opaque cursor;
- paginated ordered pull;
- tombstone application;
- package download/cache hydration;
- legacy Skill package synthesis.

### M2.6 — Desktop sync orchestrator

- serialized worker;
- persisted retry schedule;
- startup/resume/network-recovery triggers;
- push-before-pull ordering;
- cache/deployment reconciliation only after durable local apply.

### M2.7 — Conflict/error handling

- conflict state persistence;
- remote-head metadata download;
- permission-denied preservation;
- retryable/permanent error classifier;
- minimal conflict resolution operations.

### M2.8 — Reliability validation checkpoint

No GitHub Actions are required during development. Validation is performed locally when the branch is pulled.

Required local scenarios before M2 is accepted:

- response lost after server commit, same mutation retried;
- same mutation submitted repeatedly;
- process killed after HTTP ACK but before local ACK transaction;
- process killed after local ACK;
- offline create followed by multiple offline edits before first upload;
- two devices edit the same server revision;
- browser modifies a Skill while desktop is offline;
- delete while another device is stale;
- server 500/timeout/429 backoff;
- blob upload interrupted/resumed/retried;
- corrupted uploaded blob rejected;
- permission removed before pending mutation upload;
- pull interrupted between pages;
- application restart with pending cursor/outbox state.

## 19. M2 exit criteria

M2 is code-complete only when all of the following are implemented:

1. Offline-created personal Skill packages upload to the server including arbitrary supported Skill files.
2. Replaying a committed mutation causes one logical server effect.
3. Multiple offline edits queued before create ACK synchronize in deterministic per-Skill order.
4. Browser-originated changes are visible to desktop incremental pull.
5. Desktop-originated changes remain readable by the existing browser UI.
6. Concurrent server/local edits produce explicit conflict state and preserve local content.
7. Deletes synchronize through tombstones rather than inference from missing rows.
8. Sync resumes after restart using the same mutation IDs and durable cursor.
9. Shared/group/global visibility is filtered by current server authorization; M3 may still limit offline use with leases.
10. Long-lived credentials never enter WebView/localStorage.
11. No correctness property depends on Redis or process-local state.
12. Local manual reliability validation is completed before M2 is marked verified.

## 20. Recommended implementation order

The order is intentionally dependency-driven:

`domain mutation refactor`
→ `server/local schemas`
→ `blob/package storage`
→ `device + credential boundary`
→ `single mutation push`
→ `local ACK sequencing`
→ `change feed + pull`
→ `conflict/error handling`
→ `shared visibility projection`
→ `local reliability validation`

Do not begin with a background sync loop. The worker is the final composition layer; protocol idempotency, durable server receipts and local ACK semantics must exist first.
