# SkillHive Current Development Status

Updated: 2026-09-07

This file contains the **current dynamic repository state** and supersedes branch/PR metadata captured at the top of `LOCAL_AGENT_HANDOFF.md`.

## Repository state

- Repository: `somnifex/SkillHive`
- PR #3 (`feat: desktop enterprise foundation and M1 local core`) was **merged** into `main` on 2026-09-04.
- PR #3 merge commit was `4cf8fd10c3c3112dd0aa2d6e4216ae622e9602c2`; `main` may contain later handoff-only commits after that merge.
- The historical branch `feat/desktop-foundation` should no longer be used for new development.
- Continuation branch name reserved for local-agent takeover: **`feat/m2-sync`**. It should be kept aligned with the latest `main` before development resumes.
- No GitHub Actions workflow should be added for routine validation unless the owner explicitly changes that policy.

## Local agent branch reality (2026-09-05)

The active local development branch is **`feat/m2-continue`** (5 commits ahead of `main`, local only):

```text
206205d chore(frontend): pin Tauri CLI and record reproducible registry
ac94d7a chore: ignore tauri-build generated schemas (gen/)
298ece4 fix(desktop): repair Rust baseline so check/test/clippy pass on Windows
67e0fd0 fix(migrations): bind change-feed baseline timestamps dialect-neutrally
dcedf9a fix(sync): make M2.1 baseline pass local backend validation
```

It was forked from `feat/m2-sync` after that branch was aligned with latest `main` (merge b9c7257). Treat `feat/m2-continue` as the current continuation branch.

## Current milestone state

| Milestone | Current state |
| --- | --- |
| M0 Desktop/architecture foundation | COMPLETE |
| M1 Durable local desktop core | CODE COMPLETE / PENDING LOCAL VALIDATION |
| M2 Cloud sync epic (#4) | IN PROGRESS |
| M2.0 Shared Skill mutation path (#5) | CODE COMPLETE — backend validated locally (see validation truth) |
| M2.1 Protocol/schema foundation (#6) | IN PROGRESS — SQLite-validated; PostgreSQL/MySQL bypassed by owner instruction |
| M2.2 Package/blob storage (#7) | CODE COMPLETE — backend storage/transport validated on SQLite; GC design doc landed (`docs/development/GC_DESIGN.md`, destructive sweep deliberately deferred) |
| M2.3 Device identity/secure credentials (#8) | CODE COMPLETE — server endpoints + desktop identity/credential/HTTP boundary; local cargo tests pass |
| M2.4 Idempotent push (#9) | CODE COMPLETE (desktop) — push endpoint validated; desktop durable ACK transaction, blob negotiation/upload, push client landed |
| M2.5 Durable pull/change feed (#10) | CODE COMPLETE (desktop) — page apply + cursor commit + HTTP pull client + verified blob download landed; workspace hydration deferred until a consumer needs it |
| M2.6 Desktop sync orchestrator (#11) | CODE COMPLETE (desktop) — `SyncEngine::run_cycle` composes session→device→push→pull with durable state; WebView commands (`desktop_login`, `desktop_logout`, `sync_now`, `sync_state`) wired; background triggers/periodic wake landed (`sync_worker.rs`) and validated live |
| M2.7 Conflicts/reliability checkpoint (#12) | CODE COMPLETE (desktop) — `list_conflicts` + keep-local/keep-remote resolution ops, 4xx→permanent-error classifier wired into dispatch; **live server-side AND live client-process scenarios validated 2026-09-06 (see validation truth)** |
| M3 Enterprise offline authorization | CODE COMPLETE + LIVE-VALIDATED — grant offline policy (migration `c4d5e6f7a8b9`), signed JWT entitlement leases shipped in pull metadata, desktop schema-v4 entitlement store, pull-apply + startup + post-pull reconciliation landed (`b2165a7`, `b5be325`, `97fe35e`, `bef5977`); **live CDP scenarios validated 2026-09-07 (see validation truth)** |
| M4 Production hardening | PLANNED |

## Exact next task

Continue on branch `feat/m2-continue`. M3 is now code complete AND
live-validated (see validation truth below). The next milestone is **M4
(production hardening)**: structured logs/correlation IDs, metrics,
migration backup/safe startup, fault-injection tests, signed release
process. Remaining M2 gaps stay record-only:

1. **Workspaces/hydration polish (M2.5 leftover)** — pulled `remote_only`
   records carry metadata + manifest only; workspace hydration (materialize
   files from the blob closure) is deferred until a consumer needs it. This
   also blocks deploying a pulled managed skill (manifest format mismatch —
   recorded in the M3 validation run).
2. **M2.2 destructive GC sweep** — design doc landed; implementation
   deliberately deferred.
3. **UI-side local-skill surface** — the WebView pages still use the legacy
   REST/axios path; Tauri commands (`desktop_login`, `sync_now`, local
   commit/deploy) are exposed but not yet consumed by React pages.
4. **PostgreSQL/MySQL migration re-validation** when a server becomes
   available (owner bypassed SQL-server flows, 2026-09-04).

## M3 state (2026-09-07)

Server side (commit `b2165a7` + `b5be325`):

- `group_skill_grants` carries `offline_policy` (`unlimited`/`ttl`/
  `disabled`) + `offline_ttl_hours` with DB-level CHECK constraints
  (migration `c4d5e6f7a8b9`; legacy grants backfill `unlimited`).
- Grant create/update APIs accept and persist the policy with pairing
  validation; admin grant-revoke flow unchanged.
- `app/services/entitlements.py` signs leases with the existing JWT secret
  under a dedicated `type: "skill_lease"` claim (access tokens can never be
  replayed as leases and vice versa). `unlimited` gets a 7-day refresh
  bound instead of infinite expiry; `disabled` leases are dead on arrival
  (exp == issued); `ttl` uses the grant's hours. `policy_version` derives
  from the grant's `updated_at`.
- The pull projection (`sync_changes.py`) attaches `metadata.entitlement`
  (lease token, permission level, policy, ttl, signed `issued_at`/
  `expires_at`) for grant-entitled managed skills.

Desktop side (commits `97fe35e` + `bef5977`):

- Schema v4 adds `local_entitlements` (lease stored verbatim) with
  expiry indexing.
- `apply_changes_page` extracts `metadata.entitlement` inside the same
  page transaction: fresh lease keeps the skill usable; expired-at-apply
  flips it `access_revoked`; fresh lease re-entitles an expired-lease
  revocation; a malformed lease fails the whole page (cursor never
  advances past an uninterpretable lease — fail closed).
- `expire_due_entitlements` runs at startup (before cache/agent
  reconciliation) and after each sync pull; it also marks active
  deployments `revoked`. Reconciled skill IDs surface in
  `DesktopStartupStatus.expiredEntitlements`.
- Trust model (handoff §15.3): the desktop enforces the expiry contract as
  a policy clock from the authenticated transport; it does not verify the
  lease signature locally and makes no DRM claims.

Validation truth: backend `uv run python -m pytest backend/tests` →
106 passed (includes 8 entitlement-lease tests); `uv run mypy backend/app`
strict → clean; `cargo test --lib` → 91 passed (6 entitlement + 3
pull-apply entitlement tests); `cargo clippy -D warnings` and
`cargo fmt --check` → clean.

### Validated 2026-09-07 (live desktop M3 entitlement leases, CDP harness)

Toolchain: fresh E2E server DB (Alembic → `c4d5e6f7a8b9`, seeded dev data,
`tmp/e2e-server/`), Vite dev server, debug `skillhive-desktop.exe` with
WebView2 CDP (schema v4 created on the running desktop). Admin granted the
seeded global skill 需求澄清助手 to a group; member `howie` logged in via the
real client process. `sync_now` pulled the change feed and the desktop:

- schema upgraded to v4 (`local_entitlements` created); the pulled upsert
  landed with `metadata.entitlement` carrying the signed JWT lease, policy
  `ttl`/8h, and server-signed `issued_at`/`expires_at` — OK;
- **ttl lease honoured**: desktop stored the lease verbatim; skill usable
  (`remote_only` → deploy attempt correctly refused by state gate, not by
  the lease) — OK;
- **grant revoke → lease re-issue cycle**: with all grants revoked, the
  pull no longer ships a lease; backdating the stored lease (simulated
  expiry) + `sync_now` → post-pull sweep flipped the skill
  `access_revoked` — OK;
- **expired lease gates deployment**: deploy attempt in `AccessRevoked`
  state → rejected (`not deployable in state AccessRevoked`) — OK;
- **deployment revocation reconciliation**: seeded an `installed`
  deployment, expired the lease, `sync_now` → skill `access_revoked` AND
  deployment → `revoked` with `last_error` "entitlement expired;
  deployment revoked by offline policy" — OK;
- **re-entitlement**: fresh grants + new feed event → fresh lease cleared
  the expired-lease revocation (skill back to `synced`, new expiry stamped)
  — OK (validated twice, including after the disabled-policy cycle);
- **disabled policy (restricted, zero offline window)**: grants set to
  `disabled` → pulled lease expires at issuance (exp == issued) and the
  post-pull sweep immediately revoked the skill — OK;
- **server-side write gate (exit criterion)**: a non-owner update mutation
  against the managed global skill → protocol `permission_denied`
  (`SKILL_NOT_FOUND`) with a durable receipt — OK;
- **startup surface**: `desktop_startup_status` exposes
  `expiredEntitlements`; `sync_state` stayed clean (`last_server_error`
  null) through all cycles — OK.

Not covered live: deploying a pulled managed skill end-to-end fails on the
pre-existing M2.5 workspace-hydration gap (server-synthesized legacy
manifest lacks desktop `blob_hash` fields) — recorded as the known M2.5
gap, not an M3 defect; `unlimited` policy lease was observed shipping
(expiry = issued + 7d) before the grants were switched to `ttl`, but the
full 7-day expiry was not awaited (policy math is unit-tested).

Legacy package synthesis (plan §17 / handoff §9.6) is **landed** (commit
`b0914e5`) and **live-validated through the desktop CDP harness**
(2026-09-06, see validation truth): browser-created Skills carry a
synthesized SKILL.md package on the pull feed, and the desktop applies
it as a digest-verified manifest blob on a `remote_only` row.

## M2.1 already implemented but unverified

The merged code already contains:

- shared `SkillMutationService` transaction path;
- server technical Skill revisions;
- `Device`, `SkillBlobObject`, `SyncMutationReceipt`, `SyncChangeLog` ORM models;
- Alembic migration `b6a31d0f4c9e_add_sync_foundation.py`;
- protocol v1 Pydantic models;
- opaque sync cursor codec;
- additive REST read fields for technical revision/package identity;
- desktop SQLite schema v3;
- `local_sync_state`;
- per-Skill `local_sequence` ordering and persisted retry metadata;
- protocol/mutation tests that were written but not executed in the previous environment.

Do not interpret these files as verified merely because PR #3 was merged.

## Validation truth

### Validated 2026-09-06 (live desktop client-process E2E, real Tauri process against live server)

Toolchain: backend `uvicorn` 127.0.0.1:8000 on SQLite (`tmp/e2e-server/`), Vite dev server, debug `skillhive-desktop.exe` with WebView2 CDP debugging (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`). The WebView was driven through the Chrome DevTools Protocol (`Runtime.evaluate` → `window['__TAURI__']['core'].invoke(...)`) so every scenario runs the app's real Rust command path and real runtime state (`%LOCALAPPDATA%/app.skillhive.desktop`), not a test double.

**Six real bugs found and fixed by this run:**

- `54e341f` — `state not managed` on every command: setup managed `Arc<LocalStore>` while commands expect `LocalStore` (Tauri resolves by exact TypeId, no auto-deref). Fixed by managing plain values and giving the sync worker its own handles to the same SQLite/blob paths (safe: WAL + busy_timeout, content-addressed idempotent blob writes).
- `54e341f` — pull upsert crashed with `UNIQUE constraint failed: local_skills.remote_id`: a pushed create stores the server ID only in `remote_id` under the client-generated local key, so the feed echo must resolve by `id OR remote_id` and merge (`apply_upsert`/`apply_tombstone`), inserting under the local key when absent. Unit tests added.
- `506a8a2` — every follow-up update got 422 `VALIDATION_ERROR` ("update mutation requires remoteSkillId"): `submit_mutation` read the remote ID only from the mutation row's `acknowledged_remote_id` (NULL for updates); now falls back to the skill's `remote_id`.
- `4c66ec3` — conflict never converged: `apply_definitive_error` never persisted the server's `conflict_head_revision` onto the skill row, so keep-local re-queued against a stale base and re-conflicted forever. Now `remote_revision = COALESCE(?3, remote_revision)`; validated live through full convergence.
- `f63e918` (server) — sync-created skills carried only `content.skill_markdown`, so the existing web UI's editor/preview saw an empty body (plan §17 requires sync writes to populate enough legacy content); sync writes now mirror the entrypoint into `instructions` as well, with a UI-side `skill_markdown` fallback.
- `02cdd8e` (desktop) — an offline outbox chain's own change-feed echo, pulled mid-chain, labeled the Skill `conflict`; no transition ever cleared that label, wedging the row after every mutation acked. `apply_acked` now clears a stale `conflict` label only when no unacked mutation remains.

Scenario results (all through the real client process):

- login → device registration → refresh token lands in **Windows Credential Manager** (`refresh-token.app.skillhive.desktop`); SQLite bytes and WebView storage contain no tokens/passwords — OK;
- `sync_now` push/pull cycle; create → server revision 1, updates with base revisions → acked revisions — OK;
- mutation replay → receipt idempotency, single server effect — OK;
- **hard kill of the desktop process with a pending outbox mutation → restart recovery re-queued the same mutation ID, server replayed the receipt, converged (cursor advanced, no duplicate effect)** — OK;
- live REVISION_CONFLICT (server-side edit between local commits) → skill enters `conflict` with remote head recorded → `resolve_conflict(keep_local)` re-queues against the true head → acked, both sides converge at the same revision — OK;
- REST delete → pulled tombstone removed the local row — OK;
- multi-file skill create → package closure upload: manifest + file blobs stored server-side, `current_package_hash` matches — OK;
- **blob-download pull path**: local manifest blob deleted + cursor rewound → full re-pull reapplied the 7-event feed (6 upserts + 1 tombstone) and re-downloaded the manifest, digest-verified before storage — OK;
- 422 `VALIDATION_ERROR` on a malformed update → mutation `permanent_error`, no retry storm — OK;
- **server offline at session refresh** → cycle stops with a transport error, mutation stays `pending` with `retry_count 0` (no state corruption, no backoff miscalibration); server back up → next cycle pushes and converges — OK (validated twice);
- **server-side device revocation** → `sync_now` stops with `DEVICE_REVOKED` (403), pending mutation state untouched, no server error recorded; un-revoke → next cycle pushes normally — OK;
- **wrong-password login** → typed `authentication failed (401)`, no credential written — OK; `desktop_logout` → `sync_now` stops quietly with `not signed in` — OK;
- **hard kill again on the final build**: commit → immediate `Stop-Process` (worker may not have run yet, mutation still `pending`) → relaunch → **startup cycle alone converged** mutation → `acked` rev 1 and server package stored, no manual `sync_now` — OK;
- **cross-UI compatibility (M2 exit criterion 5)**: desktop-created skill readable via the existing web UI's REST endpoints with both `content.skill_markdown` and `content.instructions` populated (found and fixed `f63e918`: sync writes mirrored the entrypoint only into `skill_markdown`, so the web editor saw an empty body); then a browser REST edit → desktop pull applied the update (revision 2, `synced`) — full desktop→browser→desktop round trip — OK;
- **offline outbox chain (M2 exit criterion 3)**: server stopped → create + two updates committed offline (chain of 3) → server restored → cycles dispatched create→update→update in per-Skill order, each acking exactly once (rev 1→2→3). Found and fixed `02cdd8e`: the Skill's own change-feed echo pulled mid-chain while updates were pending labeled the Skill `conflict`, and no later transition cleared it, wedging the row after the chain fully acked — `apply_acked` now clears a stale `conflict` label when no unacked mutation remains (a genuine conflict always keeps a conflicted mutation row, which holds the gate). Re-validated on a fresh offline chain: converges to `synced` at the server head — OK.

Notes: pull downloads only the package **manifest** per feed row (closure file blobs hydrate lazily by design); an empty feed page still counts as one applied page (`pagesApplied: 1` with zero upserts is the converged-cursor shape, not a failure).

### Validated 2026-09-06 (live desktop, plan §17 legacy package synthesis — commit `b0914e5`)

Same CDP harness as above; fresh server DB (Alembic → `b6a31d0f4c9e`, `tmp/e2e-server/`), fresh desktop login as a new user (device `4a545614…`):

- browser REST create with `instructions` only → server synthesized the SKILL.md package **synchronously at create time**: change feed row carries `packageManifestHash` (`sha256:967caac2…`) — OK;
- manifest downloaded via `/sync/blobs/{hash}` → `{"format_version":1,"files":[{"path":"SKILL.md",…}]}`, entrypoint SKILL.md carries YAML front-matter (`name`/`description`) + the legacy body — OK;
- desktop `desktop_login` → `sync_now` → pulled the upsert; `local_skills` gained a `remote_only` row for the browser skill with `current_blob_hash` = the feed's manifest hash; the manifest bytes landed digest-verified in the desktop blob store (`%LOCALAPPDATA%/app.skillhive.desktop/blobs/sha256/96/967caac2…`) and parse to the exact same JSON — OK;
- cursor rewind (`server_cursor=0`) + second cycle → converged shape (empty page, `pagesApplied: 1`, `blobsDownloaded: 0` on the repeat; the manifest was already content-addressed locally) — OK.

Backend pytest covers the deterministic/lazy-synthesis/empty-body cases (`test_sync_changes_api.py`, 88 passed); this run closes the desktop side.

Not covered live: per-mutation mid-dispatch transport backoff (`retryable_error` + `next_attempt_at` timing — unit-tested in `outcomes.rs`, but the kill/race between session refresh and closure upload was not reproducibly injectable in the live process); `permission_denied` outcome (needs a second-user grant-revoke scenario); workspace hydration of pulled content.

### Validated 2026-09-06 (live end-to-end sync protocol run, Windows 11 Pro 10.0.26200)

Toolchain: Python 3.12 (uv venv) backend on `uvicorn` 127.0.0.1:8000, SQLite `data/skillhive.db` migrated fresh → `b6a31d0f4c9e`.

Scenario results (HTTP exercised with curl/urllib exactly as the desktop client would call):

- login → access token + HttpOnly `skillhive_refresh` cookie (path `/api/v1/auth`) — OK;
- refresh rotates the cookie and revokes the prior refresh session (second use → 401) — OK;
- device registration idempotent per `(user, clientInstanceId)` — OK;
- blob negotiation → PUT both objects (204) → create mutation → `acked` with remote ID/revision — OK;
- **same mutation replayed → identical receipt response, single server effect — OK**;
- two devices updating the same base revision → first `acked`, second `conflict` carrying the remote head — OK;
- stale `delete` against an old base revision → `conflict` (server head preserved) — OK;
- revoked device mutation → rejected (401-class device failure, outbox semantics separate) — OK;
- re-registration of a revoked client instance refused — OK;
- pull feed: pagination stable/ordered/disjoint across pages, tombstone after REST delete — OK;
- malformed cursor → 400 `SYNC_CURSOR_INVALID` — OK;
- private content filtered from an unrelated user's pull (empty feed) and direct read → 404 — OK;
- corrupt blob upload (digest mismatch) → 400; size-mismatch upload → 400; correct upload + roundtrip download byte-identical; missing blob → 404 — OK;
- final change feed: 5 events (browser create, desktop create, update, tombstone, second-device update) — OK.

Not covered live (unit-level coverage exists in `cargo test`/`pytest`): superseded by the client-process run above, which drove `apply_changes_page`, `apply_mutation_outcome`, and conflict resolution from the actual Tauri process against this same server contract.

### Validated 2026-09-05 (local agent session, Windows 11 Pro 10.0.26200)

Toolchain: Python 3.12.0 (uv-managed venv) · Node v24.14.1 · pnpm 11.9.0 · rustc/cargo 1.94.1

Commands passed on branch `feat/m2-continue` (HEAD 206205d):

- `uv run ruff check backend` — all checks passed
- `uv run mypy backend/app backend/tests` — 60 files clean (strict)
- `uv run pytest backend/tests` — 32 passed
- `cargo fmt --check` — clean
- `cargo check` / `cargo test` — 31 passed, 0 failed
- `cargo clippy --all-targets --all-features -- -D warnings` — clean
- Alembic on SQLite: fresh → head; upgrade path 7f4c2b8a91de → head with representative legacy data; ORM↔migration parity diff 0; idempotent re-run; downgrade both directions
- Desktop SQLite: fresh install reaches schema v3 via the running app; `local_sync_state` singleton present; WAL active
- Frontend: `pnpm lint`, `pnpm typecheck`, `pnpm test` (3 passed), `pnpm build` — all green
- Desktop build: `pnpm exec tauri build --debug --no-bundle` produces `skillhive-desktop.exe`; app launches and creates runtime state (smoke-tested, then state removed)
- M1 unit-level validation passes via cargo test: snapshot round-trip, blob tamper detection, workspace boundary/ID escape, deployment journal recovery (prepared-new-install rollback, verified catalog ACK), uninstall rollback, agent profile forgery rejection, per-Skill outbox gating (create→update→delete, in-flight claim, restart recovery)

### Known fixes landed to make the baseline truthful

- `dcedf9a` — backend baseline (Literal protocol version, autoflush-safe mutation test, ruff import order)
- `67e0fd0` — change-feed baseline timestamps bound dialect-neutrally (SQLite returns str from raw SELECT)
- `298ece4` — desktop baseline (Rust syntax error, `File::by_ref` ambiguity, unix-only `File` imports, clippy fixes, `icons/icon.ico`, Cargo.lock committed)
- `206205d` — project-pinned `@tauri-apps/cli`, `frontend/.npmrc` registry pin

### Explicitly skipped (owner instruction, do NOT count as verified)

- **PostgreSQL migration validation** — bypassed. Owner instruction (2026-09-04): “暂时绕过所有sql流程” (temporarily bypass all SQL-server flows). WSL/Docker unavailable (owner cancelled WSL install), no local PostgreSQL/MySQL server. Re-validate when a server becomes available.
- **MySQL support decision** — deferred together with PostgreSQL above.
- **Interactive Tauri runtime UI testing** — desktop exe launches and initializes its SQLite/blob/journal state (smoke test), but full window interaction was not manually exercised.

The merge of PR #3 remains an integration event, **not** a full verification certificate, but the local executable baseline is now real. M1/M2.0 status above reflects backend + Rust unit-level validation only; fault-injection coverage beyond what `cargo test` covers (PART H scenarios) remains manual/TODO.

Use `docs/development/LOCAL_VALIDATION_CHECKLIST.md` as the verification contract.

## Document priority

For dynamic development state, use:

1. `AGENTS.md`
2. this file (`CURRENT_STATUS.md`)
3. current source code/migrations and Git history
4. `LOCAL_AGENT_HANDOFF.md` for detailed implementation history and forward plan
5. architecture plans
6. historical README/makewiki documentation

If the old handoff mentions PR #3 as Draft or `feat/desktop-foundation` as the active branch, treat those statements as historical snapshot metadata superseded by this file.
