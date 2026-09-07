# SkillHive Garbage Collection Design (M2.2, plan §9.7)

Status: **IMPLEMENTED (2026-09-06, commit `a4f28c3`)** — `blob_gc.py` (destructive mark-and-sweep), `sync_trim.py`, the four retention settings and the `SYNC_CURSOR_EXPIRED` contract are live with unit tests. **Production scheduling is still missing**: `run_blob_gc` / `trim_expired_rows` have no caller in `main.py` (only the trash-retention sweep is scheduled), so orphan blobs are reclaimed only by manual runs — tracked in CURRENT_STATUS known issues.

This document is the M2.2 leftover for Issue #7. It specifies the blob
garbage-collection contract for both storage roots (server package store and
desktop cache), records which parts are already implemented, and states the
follow-up work required before the destructive sweep may be switched on.

---

## 1. Requirements (plan §9.7, verbatim)

> Do not use mutable refcount as the sole deletion authority.
>
> Plan mark-and-sweep roots from:
>
> - live SkillVersion package manifests;
> - retained change history required by cursor retention;
> - unexpired mutation receipts if they reference package data;
> - retention/legal policy.
>
> Actual destructive GC can be deferred, but storage design must not block it.

## 2. Why not mutable refcounts

A persisted refcount column is a single point of drift: every reference
creating or dropping path (version create, publish rollback, soft delete,
change-log trim, receipt expiry, an interrupted transaction on any replica)
must increment and decrement correctly forever, including across crashes
between the metadata write and the counter write. A drifted counter either
leaks objects forever or deletes live packages — and the failure is silent.

Mark-and-sweep recomputes liveness from durable facts on every run. A missed
root leaks disk (recoverable by fixing the root query); it can never delete
content that is still referenced, as long as the root set below is respected.
This is the same reasoning the desktop cache manager already applies (§7).

## 3. Storage invariants already in place (M2.2 landed)

The current implementation was built so that a later GC needs **no schema
migration and no backfill**:

| Invariant | Where | GC consequence |
| --- | --- | --- |
| Content addressing (`sha256:<64 hex>`) | `blob_storage.py` (`is_canonical_hash`), desktop `blob_store.rs` | Deletion decisions are per-object; no identity ambiguity |
| Verify-on-write and verify-on-read | `put_verified` / `exists` / desktop `verify` | A swept-then-re-uploaded object is always valid; digest check runs before addressability |
| Idempotent re-upload | `put_verified` accepts an existing verified object as success | Re-creating a wrongly deleted object is a plain re-upload; no tombstone bookkeeping |
| `delete(hash)` on the storage ABC | `BlobStorage.delete` — documented "GC only, not normal mutation path"; `LocalFilesystemBlobStorage.delete` tolerates `FileNotFoundError` | The destructive primitive exists on every backend, including the future S3 contract |
| Blob registry is append-only metadata, **no refcount column** | `SkillBlobObject` (hash PK, size, storage_key, verified_at) | Nothing to backfill; sweep basis is `skill_blob_objects` minus the mark set |
| Package references are plain hash strings | `skills.current_package_hash`, `skill_versions.package_manifest_hash`, `sync_change_log.package_manifest_hash` | Roots are ordinary SQL queries (§4) |
| Crash-safe writes | temp file + atomic rename (server), digest-verified `put_bytes` (desktop) | Stray temp files (server prefix `.skillhive-blob-`) are the only filesystem orphans; sweep may unlink stale ones |

Normal mutation paths never call `delete` today; adoption of a new package
version simply adds objects, and old objects become unreferenced — exactly
the state a sweep is designed to collect.

## 4. Server mark phase — roots

The mark set is computed from durable relational state in one read
transaction (or a repeatable-read snapshot on PostgreSQL). Let
`mark_started = utcnow()`; any object with `created_at >= mark_started` is
conservatively kept regardless of roots (protects uploads racing the sweep).

| Root | Query source | Rationale |
| --- | --- | --- |
| **R1 — live version manifests** | every `skill_versions.package_manifest_hash` (all retained versions), plus `skills.current_package_hash` | Full version history is retained by design; every version row is a root, not just the current one. `skills.current_package_hash` is nominally a duplicate of some version row but is kept as an explicit root so a legacy/normalization gap in `current_version_id` can never orphan the live package |
| **R2 — retained change history** | `sync_change_log.package_manifest_hash` for rows with `created_at >= now − change_log_retention_days` | A client cursor points at a change-log sequence; every not-yet-consumed row must remain pullable with its package (§6) |
| **R3 — unexpired mutation receipts referencing package data** | `sync_mutation_receipts.response_payload["packageManifestHash"]` for receipts within the receipt-retention window | Plan §9.7 lists these roots explicitly. Replay itself is served from the persisted `response_payload` JSON (no blob read needed), so this root is defense-in-depth for clients that re-fetch the manifest after a replay |
| **R4 — in-flight uploads** | any `skill_blob_objects` row with `created_at >= now − blob_gc_orphan_grace_hours` | Upload (PUT) makes an object addressable *before* the mutation that references it commits; an in-flight or retrying client must not have its objects swept |
| **R5 — retention/legal policy** | reserved explicit pin-list (config/admin), empty today | Plan §9.7 "retention/legal policy"; the sweep must accept an injected extra root set so legal holds need no code change later |

**Closure expansion:** each rooted manifest hash expands to its full package
closure — the manifest bytes plus every file blob hash listed in it — using
the same manifest parsing as `validate_snapshot_manifest_bytes`. Marking a
manifest without its closure would corrupt retained versions; marking the
closure without the manifest would leave pull unable to rehydrate.

```
roots ← R1 ∪ R2 ∪ R3 ∪ R5            # hash strings
for manifest in roots: roots ← roots ∪ closure(manifest)
roots ← roots ∪ {objects with created_at ≥ now − orphan_grace}   # R4
```

## 5. Server sweep phase

Sweep basis is the **registry**, not a filesystem walk:

```
for obj in skill_blob_objects where hash ∉ roots:
    storage.delete(obj.hash)     # idempotent; absent object is success
    delete registry row          # metadata follows the bytes
```

Properties:

- **Crash-safe at any point.** Deleting bytes then the row (or being
  interrupted between them) is safe in both directions: a row whose bytes are
  gone makes `exists()` fail verification → negotiation reports the object
  missing → the client re-uploads (idempotent, verified). Orphaned bytes with
  no row are invisible to negotiation and get collected by the next sweep.
- **Idempotent.** The whole job can be re-run after any interruption; deletes
  of already-deleted objects succeed.
- **Never destructive to live data** as long as §4 roots are computed from a
  consistent snapshot and the creation-time cutoff is honored.
- **Filesystem reconciliation (optional second pass):** enumerate
  `sha256/xx/…` like the desktop inventory does, delete bytes with no
  registry row older than the orphan grace, and unlink stale
  `.skillhive-blob-*.tmp` files. This handles pre-registration crash debris;
  it is an optimization, not a correctness requirement.

## 6. Cursor retention and the expired-cursor contract

The pull feed (`sync_changes.list_changes`) serves `sequence > cursor` from
the append-only `sync_change_log`. Trimming that log is the one GC decision
that can silently desync a client, so the design fixes the contract **before**
any trimming is implemented:

- Proposed setting: `change_log_retention_days = 90` (exceeds the longest
  expected offline period for an enterprise device).
- A client whose cursor predates the oldest retained row **must not** receive
  a silently truncated page — that would look like a normal page and hide the
  gap. The server must instead fail with
  `410 SYNC_CURSOR_EXPIRED` ("cursor predates retained history; re-baseline").
- Client behavior on `SYNC_CURSOR_EXPIRED`: clear the durable cursor, mark
  local content stale, and perform a full re-pull (existing
  `remote_only`/`conflict`-safe apply paths make this safe); outbox mutations
  are unaffected (push never depends on the pull cursor).
- **Not implemented yet, and currently impossible to trigger:** no trimming
  exists, so today's feed is unbounded and every cursor is servable. Both the
  trim job and the `SYNC_CURSOR_EXPIRED` check are part of the same future
  work package as the destructive sweep — they must land together.

Receipt retention follows the same window (`receipt_retention_days = 90`
proposed). Receipts older than the window lose replay idempotency for
extremely late retries; that is acceptable only because the desktop already
retires `acked` mutations locally and never replays them after the retention
horizon. The settings landed (`change_log_retention_days` / `receipt_retention_days`, config.py); enforcement now only needs the scheduled run — the
conservative default.

## 7. Desktop cache GC (already implemented and unit-validated)

`cache_manager.rs::enforce_cache_budget` is the desktop's mark-and-sweep and
satisfies §9.7's "no mutable refcount" rule by construction:

- **Marks** are recomputed per run from durable SQLite roots — every
  non-`remote_only` skill's `snapshot_hash`, every deployment's
  `deployed_blob_hash`, and every unacked outbox mutation's `payload_hash`
  (`local_mutations.state <> 'acked'`, i.e. pending/in-flight/conflict/error)
  — then expanded to manifest closures via `read_manifest`.
- The in-run reference tally is an **ephemeral recomputation**, not persisted
  state; nothing can drift between runs. The deletion authority is the mark
  set, not a counter.
- **Sweep order** respects safety: only `synced` + unpinned + undeployed +
  workspace-absent skills are evictable, eviction is claimed with a durable
  compare-and-swap (`claim_skill_for_eviction(skill_id, expected_snapshot_hash)`)
  so a concurrent local edit aborts it, and blobs are removed only when their
  recomputed reference count reaches zero. Unresolved snapshots are skipped,
  never guessed.
- Orphan-blob fallback deletion runs only against hashes absent from the
  recomputed mark set.

No change is required; this section records the compliance argument.

## 8. Future job shape (when destructive GC is scheduled)

```python
def run_blob_gc(session, storage, *, now, orphan_grace, change_retention, receipt_retention, legal_roots=()):
    mark_started = now
    roots = set(legal_roots)                                  # R5
    roots |= all_version_manifest_hashes(session)             # R1
    roots |= skills_current_package_hashes(session)           # R1
    roots |= change_log_manifest_hashes(session, now - change_retention)   # R2
    roots |= receipt_payload_manifest_hashes(session, now - receipt_retention)  # R3
    for manifest in list(roots):
        roots |= manifest_closure(session, storage, manifest)
    # R4 is enforced by the created_at cutoff below.
    for obj in registry_rows_not_in(session, roots):          # anti-join, dialect-safe
        if obj.created_at >= mark_started - orphan_grace:
            continue
        storage.delete(obj.hash)
        session.delete(obj)
    # caller commits; job is safe to re-run
```

Settings (all landed in config.py) (to be added to `app.core.config.Settings` when the job
lands; none exist yet, all defaults conservative):

```python
blob_gc_orphan_grace_hours: int = 24
change_log_retention_days: int = 90
receipt_retention_days: int = 90
blob_gc_batch_size: int = 1000   # bound per invocation; repeated runs converge
```

Operational expectations for the future job: run off-peak; log per-run
(root count, candidates, deletions) without logging blob contents; make the
S3 backend's `delete` equally idempotent (absent-object success), which the
ABC contract already implies.

## 9. Exit-criteria mapping (plan §9.7)

| §9.7 requirement | State |
| --- | --- |
| No mutable refcount as sole deletion authority | Satisfied by design: no refcount column exists server- or desktop-side; deletion authority is recomputed marks (§2, §7) |
| Roots: live SkillVersion package manifests | Specified (R1); queryable against landed schema with no migration |
| Roots: retained change history for cursor retention | Specified (R2) together with the `SYNC_CURSOR_EXPIRED` contract (§6) |
| Roots: unexpired mutation receipts referencing package data | Specified (R3); replay is served from persisted payloads, so this root is conservative |
| Roots: retention/legal policy | Reserved injected root set (R5) |
| Destructive GC may be deferred | Deferred: no trim/sweep job runs today; feed and registry are append-only |
| Storage design must not block it | Satisfied: `delete()` on the ABC, content addressing, idempotent verified re-upload, append-only registry, no backfill required |

## 10. Work package to make GC active (when scheduled)

1. Add the four settings above; implement `run_blob_gc` (§8) plus a bounded
   trim job for `sync_change_log` and `sync_mutation_receipts`.
2. Add the `SYNC_CURSOR_EXPIRED` (410) check to `sync_changes.list_changes`
   and the client-side re-baseline path; land in the same release as trimming.
3. Add pytest coverage: root computation vs. live/pending/deleted content,
   in-flight upload grace, interrupted-sweep re-run, expired-cursor refusal,
   desktop eviction unchanged.
4. Only then enable any scheduler; until then this design remains the
   contract and nothing destructive runs.
