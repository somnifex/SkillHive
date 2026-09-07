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

## Group tree / admin console feature branch (2026-09-07)

The owner approved a new feature package (plan-approval round, 2026-09-07):
tree-nested groups with permission inheritance, a system-admin console
(storage backend selection local/S3, user lifecycle, platform parameters),
a user-configurable server address on the login screen, and client-side zip
import/export. Design of record:
`docs/architecture/group-tree-and-admin-console.md`.

- Branch: **`feat/group-tree-admin`**, forked from `main` (`6f4c604`) via a
  separate worktree (`../SkillHive-group-tree`) because
  `feat/wanhua-ui-revamp` carries uncommitted UI work that must not mix in.
- Known limitation: `origin` is SSH-only and unreachable from the current
  environment, so the fork point is the local `origin/main` ref; re-fetch and
  rebase before opening a PR.
- Phases: 0 design doc → 1 group tree (backend+frontend) → 2 admin console +
  S3 storage → 3 login server address → 4 zip import/export. Update this
  section as each phase lands.

### Merged into main (2026-09-07, local)

`feat/group-tree-admin` (8 commits, fast-forward) and
`feat/wanhua-ui-revamp` (the workspace UI revamp, committed as `a120f13`)
are both merged into `main` via merge commit `b2303c7`. The five overlapping
page files (GroupsPage / GroupDetailPage / AuthPages / SkillsPage /
AdminPage) were resolved by taking the wanhua visual revamp as the base and
re-applying the feature deltas (group tree table, sub-group creation,
breadcrumbs, server-address field, zip import/export, admin console
surfaces). Post-merge validation on `main`: backend ruff/mypy clean +
**pytest 130 passed**, frontend lint/typecheck/test/build green,
`cargo test --lib` **107 passed** + clippy/fmt clean, Alembic fresh →
`f7a8b9c0d1e2`. `origin` is still unreachable (SSH) — push when access is
restored.

### Landed 2026-09-07 (all phases, commits `027088b` → `dc94566`)

- **Group tree** (`41dd0ad` + `af768df`): `groups.parent_id` self-reference
  (migration `e8f1a2b3c4d5`), effective-role resolution via recursive CTE
  (ancestor owner/admin grants admin on descendants; owner-only operations
  stay local; global admins act as implicit tree root), depth limit 16,
  cycle rejection, `GROUP_HAS_CHILDREN` dissolve guard, `POST /groups`
  with `parent_group_id`, `PATCH /groups/{id}` reparenting, `GET /groups/tree`;
  frontend tree table with parent column, sub-group creation, breadcrumbs.
  Skill grants deliberately do NOT inherit down the tree (design §2.3).
- **Admin console + storage** (`bdc47c9` + `b758c78`): `system_settings`
  table (migration `f7a8b9c0d1e2`), `S3BlobStorage` behind a
  `RoutedBlobStorage` facade (writes to the active backend, reads fall back
  across configured backends; a switch takes effect on the next operation
  without a restart), `/admin/system/settings` GET/PATCH with env-only S3
  credentials, registration toggle, user lifecycle (create / password reset
  with session revocation / guarded soft delete), `GET /admin/groups/tree`,
  and the AdminPage surfaces (users, group tree, system settings).
- **Server address** (`12d9be3`): login/register pages gain a server
  address field with a live test; web persists in localStorage with a
  dynamic axios base URL; desktop persists in Rust (`server.json`, atomic
  write, validated, env fallback) via `get/set_server_url` commands,
  `SyncClient` managed through a runtime-replaceable `SyncClientHandle`
  shared by commands and the sync worker; keyring credentials are
  namespaced per server (SHA-256 prefix) — existing desktop installs
  re-login once.
- **Zip packaging** (`dc94566`): desktop-only import/export through native
  dialogs opened in Rust (WebView never submits paths); import validates
  entry names/sizes, reuses the full snapshot capture policy and queues the
  create mutation through the standard commit path; export materializes the
  verified snapshot from the blob store. Server and sync protocol untouched
  (D1).

Validation truth (2026-09-07, worktree `../SkillHive-group-tree`):

- backend: `uv run ruff check backend` clean; `uv run mypy backend/app
  backend/tests` clean (strict, 86 files); `uv run pytest backend/tests` →
  **130 passed**; Alembic fresh → head (`f7a8b9c0d1e2`) and staged upgrades
  `c4d5e6f7a8b9 → e8f1a2b3c4d5 → f7a8b9c0d1e2` with legacy data intact,
  `ck_groups_parent_not_self` enforced.
- frontend: `pnpm lint/typecheck/test/build` all green.
- desktop: `cargo test --lib` → **107 passed**; `cargo clippy -D warnings`
  and `cargo fmt --check` clean.
- Known flake (pre-existing, also on clean tree): see the
  `telemetry::tests::events_append_json_lines` note in
  `LOCAL_VALIDATION_CHECKLIST.md` §1.
- PostgreSQL/MySQL migration re-validation still bypassed per owner
  instruction (no server available) — unchanged from the standing record.

### Landed 2026-09-07 (desktop productization, branch `feat/desktop-productization`)

Owner-requested product package closing three feedback items (no visible
upload path, "why is this a web page", management gaps):

- **NSIS packaging** (`tauri.conf.json` `bundle.active=true`, zh/en NSIS,
  `icons/icon.ico`): `pnpm tauri build` now emits a Windows installer; the
  desktop app is the primary product form while the web build keeps working.
- **Agent deployment surfaced in UI**: built-in adapters extended to 12
  (+ Cursor `~/.cursor/skills`, Windsurf `~/.windsurf/skills`, Trae
  `~/.trae/skills`, ZCode `~/.zcode/skills`; conventional paths, verify
  against vendor docs if a target disagrees). New `/agents` page: discovery
  cards, enable toggles, custom directories, global default deploy targets,
  deployments table with uninstall. SkillsPage rows gain deploy (modal with
  per-skill override persisted in new `deployment_prefs` local-store v5
  table), hydrate ("下载到本地"), and batch deploy/delete; pagination bug
  fixed (server-side page param wired).
- **Workspace hydration (closes M2.5 leftover)**: new `hydrate_skill`
  engine + `hydrate_skill_workspace` command materialize a pulled
  `remote_only` skill's snapshot closure (verified blob downloads) and its
  managed workspace; deploy commands auto-hydrate first. Pulled skills are
  now deployable end to end.
- **Sync/conflict surfaces**: top-bar SyncStatus chip (last push/pull,
  server-error badge, 立即同步), desktop-only conflict center in Settings
  (`list_conflicts` + keep_local/keep_remote).
- **Trash lifecycle (server)**: soft delete = recycle bin;
  `GET /skills/trash`, `POST /skills/{id}/restore` (returns as draft),
  `DELETE /skills/{id}/purge` (versions cascade; blobs reclaimed by GC);
  admin setting `trash_retention_days` (default 30, `0`=manual only) with a
  daily background sweep (savepoint-per-row). `slug_exists` guard fixed to
  match the (owner, slug) unique constraint (409 instead of 500 on
  trash-reserved slugs).
- **Version management**: `skill_versions.tags` JSON column (migration
  `a9b0c1d2e3f4`), `PUT .../versions/{version}/tags` (unique per skill),
  `POST /skills/{id}/rollback` (mints a NEW patch version from an old
  content — additive, never a history rewrite), `GET .../versions/{version}/export`
  portable zip; version drawer UI with tag editor / rollback / download.

Validation truth (2026-09-07, local): backend ruff/mypy clean + **pytest 134
passed**; Alembic fresh → `a9b0c1d2e3f4`, downgrade/upgrade round-trip
verified; frontend lint/typecheck/test/build green; `cargo fmt --check`,
`cargo clippy -D warnings` clean, `cargo test --lib` **116 passed**
(includes hydration + deployment-prefs suites). PostgreSQL/MySQL re-validation
remains bypassed per owner instruction.

### Live end-to-end validation (2026-09-08, local server + release client)

Ran the full product loop against `uvicorn` (SQLite, port 8000) and the
built `skillhive-desktop.exe`, driven through the GUI:

- **Fixed during testing (3 real bugs):** desktop WebView CORS preflight
  rejected (`http://tauri.localhost` missing from `cors_origins`); sync
  device never registered because `desktop_login` had no UI caller (login
  page now registers the device + triggers an immediate cycle; logout clears
  keyring); server camelCase manifests (`blobHash`/`sizeBytes`) rejected by
  the desktop snapshot reader so pulled skills could never hydrate/deploy
  (manifest deserializes both spellings now, unit-tested); sync chip showed
  "8 hours ago" on UTC+8 (SQLite CURRENT_TIMESTAMP parsed as local time).
- **Verified live:** register + login (web session + device registration);
  skill creation from the UI; pull landing the skill locally (`remote_only`);
  hydration ("下载到本地" → `remote_only`→`synced`, workspace + SKILL.md
  materialized); **one-click deploy to both `~/.zcode/skills/literature-review`
  and `~/.codex/skills/literature-review` with correct SKILL.md content**;
  Agent 部署 page (12 adapters discovered per directory presence, enable
  toggles, deployment table showing both targets 已安装); trash lifecycle
  (delete → trash tab with delete-time → restore as draft → strong-confirm
  purge → 404); version tags (stable on 0.1.0, 409 on tag clash), rollback
  (mints 0.1.1 "回滚自 0.1.0"), per-version zip export (SKILL.md inside).
- Sync worker cycles healthy throughout (ok, 72-98ms per cycle).
- Remaining known cosmetic: session loss on WebView reload (access-token
  expiry drops the web session while keyring credentials persist) — user
  re-logs in; recording as known issue.

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
| M2.5 Durable pull/change feed (#10) | CODE COMPLETE (desktop) — page apply + cursor commit + HTTP pull client + verified blob download landed; **workspace hydration landed 2026-09-07 on `feat/desktop-productization` (`hydrate_skill` + `hydrate_skill_workspace`, pulled skills deploy end to end)** |
| M2.6 Desktop sync orchestrator (#11) | CODE COMPLETE (desktop) — `SyncEngine::run_cycle` composes session→device→push→pull with durable state; WebView commands (`desktop_login`, `desktop_logout`, `sync_now`, `sync_state`) wired; background triggers/periodic wake landed (`sync_worker.rs`) and validated live |
| M2.7 Conflicts/reliability checkpoint (#12) | CODE COMPLETE (desktop) — `list_conflicts` + keep-local/keep-remote resolution ops, 4xx→permanent-error classifier wired into dispatch; **live server-side AND live client-process scenarios validated 2026-09-06 (see validation truth)** |
| M3 Enterprise offline authorization | CODE COMPLETE + LIVE-VALIDATED — grant offline policy (migration `c4d5e6f7a8b9`), signed JWT entitlement leases shipped in pull metadata, desktop schema-v4 entitlement store, pull-apply + startup + post-pull reconciliation landed (`b2165a7`, `b5be325`, `97fe35e`, `bef5977`); **live CDP scenarios validated 2026-09-07 (see validation truth)** |
| M4 Production hardening | IN PROGRESS — observability + migration safety landed (commits `0d0fba4`, `1b0b2c5`, `210af26`, `bf64a8f`; see M4 state below). Remaining: fault-injection test package, signed release/update process (design-only), release SLO gates doc |

## Exact next task

Continue on branch `feat/m2-continue`. M4 is in progress: the
observability + migration-safety packages landed (see M4 state below).
The remaining M4 work is the **fault-injection test package** (network
loss / crash / duplicate / 5xx / auth-change / disk-full — mostly
already covered live; needs pytest+cargo home in one recorded
checklist), the **signed release/update process** (design-only given the
local-only constraint), and the **release SLO/correctness gates** doc.
Remaining M2 gaps stay record-only:

1. **M2.2 destructive GC sweep** — design doc landed; implementation
   deliberately deferred.
2. **PostgreSQL/MySQL migration re-validation** when a server becomes
   available (owner bypassed SQL-server flows, 2026-09-04).
3. **UI-side local-skill surface (mostly closed 2026-09-07)** — SkillsPage
   and the new Agent 部署 page now consume deploy/uninstall/discover/prefs/
   hydrate/sync/conflict commands; local *editing* (create/commit via
   `commit_local_skill_workspace`) still uses the REST path — acceptable
   while the server remains the creation authority for typed content.
4. Trash restore on the desktop currently goes through the REST endpoints
   (online-only); offline restore would need a new sync mutation type and is
   deliberately deferred (protocol v1 unchanged).

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

## M4 state (2026-09-07)

Landed (commits `0d0fba4`, `1b0b2c5`, `210af26`, `bf64a8f`):

**Backend observability (`0d0fba4`)**
- `app/core/observability.py`: pure-ASGI `X-Request-ID` middleware —
  echoes the caller-supplied ID (desktop sends its per-cycle ID; capped
  at 128 chars) or generates one, appends it to every response
  *including handler-generated error envelopes*, stamps a context var
  read by a logging filter, and emits one structured access-log line per
  request (method/path/status/duration only — never tokens, query
  strings, or bodies). AppError/HTTPException handlers log their error
  codes for metrics.
- `app/services/sync_trim.py`: bounded change-log/receipt trim jobs
  completing the GC work package (GC_DESIGN.md §6 + §10 item 1). The
  410 `SYNC_CURSOR_EXPIRED` check was already live and keys off the
  trimmed oldest sequence. Bounded per invocation; repeated runs
  converge.
- Real bug found and fixed: `migrations/env.py` called `fileConfig` with
  the default `disable_existing_loggers=True`, which flipped every
  pre-existing `app.*` logger to `disabled=True` for the rest of the
  process after any in-process alembic run (silently swallowing all
  later logs; found by the new observability tests in full-suite runs).
  Now `disable_existing_loggers=False`.
- Real bug found and fixed: `test_entitlements.py` anchored lease tests
  at a fixed 2026-09-06 timestamp; the ttl-8h token expired ~2h of real
  clock later, making the JWT layer reject the token as expired instead
  of testing policy math. Now anchored at the real wall clock.

**Desktop telemetry + correlation (`1b0b2c5`, `bf64a8f`)**
- `telemetry.rs`: dependency-free JSON-line event log at
  `<app data>/logs/skillhive.log` (5 MB rollover to `.old`). Failure-
  tolerant by contract — log writes never break sync paths — and
  redaction-safe: callers pass explicit field slices (identifiers and
  counts only).
- `SyncEngine::run_cycle` generates one correlation ID per cycle, sends
  it as `X-Request-ID` through all four authenticated request methods in
  `sync_client.rs`, and emits `sync_cycle_begin`/`sync_cycle_end` (with
  outcome counters and duration). Mutation-dispatch failures and
  entitlement expirations emit their own events (IDs only). Startup
  emits `startup_begin`/`startup_complete` with recovery counts, and
  deployment-journal recovery emits `deployment_recovery`.

**Desktop migration safety (`210af26`)**
- `LocalStore::open` snapshots the DB to `<db>.sqlite3.pre-migration`
  (`VACUUM INTO` — consistent, compacted, WAL-safe) whenever a migration
  is pending. Steps remain transactional; the backup covers
  non-transactional failure modes (disk-full during checkpointing, torn
  pages, kill between steps). `migration_start`/`migration_done`/
  `migration_backup` telemetry events included.

Validation truth: backend `uv run python -m pytest backend/tests` →
120 passed (8 observability + 5 trim tests included); `uv run mypy
backend/app backend/tests` strict → clean; `uv run ruff check backend`
→ clean; `cargo test --lib` → 95 passed; `cargo clippy -D warnings` and
`cargo fmt --check` → clean.

M4 remaining (explicit): fault-injection test package (much is already
live-validated 2026-09-06; needs one recorded pytest+cargo checklist
mapped to the roadmap reliability matrix), signed release/update
process (design-only given local-only constraint), release SLO gates
doc.

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
