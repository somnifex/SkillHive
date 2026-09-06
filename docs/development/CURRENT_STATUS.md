# SkillHive Current Development Status

Updated: 2026-09-06

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
| M2.2 Package/blob storage (#7) | CODE COMPLETE — backend storage/transport validated on SQLite; GC design doc outstanding |
| M2.3 Device identity/secure credentials (#8) | CODE COMPLETE — server endpoints + desktop identity/credential/HTTP boundary; local cargo tests pass |
| M2.4 Idempotent push (#9) | CODE COMPLETE (desktop) — push endpoint validated; desktop durable ACK transaction, blob negotiation/upload, push client landed |
| M2.5 Durable pull/change feed (#10) | CODE COMPLETE (desktop) — page apply + cursor commit + HTTP pull client + verified blob download landed; workspace hydration deferred until a consumer needs it |
| M2.6 Desktop sync orchestrator (#11) | CODE COMPLETE (core) — `SyncEngine::run_cycle` composes session→device→push→pull with durable state; WebView commands (`desktop_login`, `desktop_logout`, `sync_now`, `sync_state`) wired; background triggers/periodic wake outstanding |
| M2.7 Conflicts/reliability checkpoint (#12) | CODE COMPLETE (core) — `list_conflicts` + keep-local/keep-remote resolution ops, 4xx→permanent-error classifier wired into dispatch; the 12 reliability scenarios of Issue #12 still require a live end-to-end run |
| M3 Enterprise offline authorization | PLANNED |
| M4 Production hardening | PLANNED |

## Exact next task

Continue on branch `feat/m2-continue`. All M2 desktop code work packages
(M2.2–M2.7 core) are code complete; what remains before M2 can be called
VERIFIED:

1. **End-to-end reliability run (Issue #12)** — start the local backend
   (`SKILLHIVE_SERVER_URL=http://127.0.0.1:8000`) and the desktop exe, then
   work through the 12 scenarios in `LOCAL_AGENT_HANDOFF.md` §14 (response
   lost after commit, repeated mutation, kill after HTTP ACK, offline edit
   chains, two-device conflict, tombstones, backoff persistence, pull
   interruption, restart with pending outbox/cursor). Record results per
   `LOCAL_VALIDATION_CHECKLIST.md` §41.
2. **Background triggers (M2.6 remainder)** — app-startup sync, network-
   recovery trigger, and a bounded periodic wake; `sync_now` (explicit
   user request) already works.
3. **M2.2 leftover (Issue #7)** — mark-and-sweep GC design doc.

Outstanding leftovers: M2.2 mark-and-sweep GC design doc (Issue #7);
PostgreSQL/MySQL migration re-validation when a server becomes available
(owner bypassed SQL-server flows, 2026-09-04).

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

Not covered live (unit-level coverage exists in `cargo test`/`pytest`): client-side SQLite apply paths (`apply_changes_page`, `apply_mutation_outcome`, conflict resolution) — those are covered by 75 desktop unit tests but not yet against this live server from the actual Tauri process.

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
