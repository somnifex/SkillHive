# SkillHive Agent Development Contract

Last updated: 2026-09-04
Scope: entire repository
Primary development branch: `feat/desktop-foundation`
Draft PR: #3

This file is the mandatory entry point for any local coding agent taking over SkillHive development.

## 1. Read order and source-of-truth priority

Before changing code, read in this order:

1. `AGENTS.md` — non-negotiable engineering constraints.
2. `docs/development/LOCAL_AGENT_HANDOFF.md` — current implementation status and next work.
3. `docs/development/LOCAL_VALIDATION_CHECKLIST.md` — local verification procedure.
4. `docs/architecture/m2-cloud-sync-plan.md` — M2 protocol/architecture design.
5. `docs/architecture/desktop-enterprise-roadmap.md` — long-term architecture roadmap.
6. Relevant source code, migrations and tests.

If sources disagree, use this precedence:

**current source code + migrations > LOCAL_AGENT_HANDOFF > architecture plans > historical README/makewiki/docs**.

Do not infer implementation state from old documentation. The repository was intentionally analyzed and refactored from code, not from README claims.

## 2. Product architecture

SkillHive is being migrated to an enterprise desktop application with server-backed accounts and authoritative cloud storage.

Target architecture:

- Desktop: Tauri 2 + existing React/Vite frontend.
- Desktop privileged core: Rust.
- Local durable metadata/outbox: SQLite.
- Local immutable content: SHA-256 content-addressed blobs and Skill snapshots.
- Cloud API: existing FastAPI application, evolved as a modular monolith.
- Server metadata/identity/ACL/sync state: relational DB, primarily PostgreSQL.
- Large Skill package bytes: object-storage abstraction; local filesystem only for single-node development/self-host mode.
- Agent integration: adapter/instance model with per-instance Skill root.

Do not prematurely split the server into microservices.

## 3. Reliability invariants

These are correctness requirements, not optional implementation preferences.

### Local edits

A user-visible local save must commit the local Skill state and durable outbox mutation in the same SQLite transaction.

Never report a save as successful before that transaction commits.

Do not replace the durable mutation log with a simple `dirty=true` flag.

### Mutation identity

A mutation ID is generated once when the local mutation is committed and must survive retry/restart.

Never generate a new mutation ID merely because an HTTP request timed out or the process restarted.

### Cloud authority

The server is authoritative for:

- user/account identity;
- device authorization;
- ACL/grants;
- server revision;
- remote Skill metadata/version history;
- sync receipts/change history.

The desktop may work offline, but it cannot grant itself server permissions.

### Optimistic concurrency

No silent Last-Write-Wins.

`base_revision != server current revision` must become an explicit conflict. Preserve the local immutable snapshot until the conflict is resolved.

### Server transaction boundary

For a future sync mutation, the following must commit atomically in one DB transaction:

- domain Skill mutation;
- technical revision update;
- immutable SkillVersion creation when required;
- audit entry;
- sync change event;
- mutation receipt.

A server HTTP response lost after commit must be safe to retry by the same mutation ID.

### Technical revision

`sync_revision` / SkillVersion `revision` are server concurrency identifiers and are independent of semantic version strings such as `1.2.3`.

Server Skill revisions start at **1** and are monotonically increasing per Skill.

Metadata-only mutations may consume a revision even when no new SkillVersion row is created. Revision gaps between SkillVersion rows are valid.

### Change cursor

Desktop pull correctness must use a durable server change-log cursor, never wall-clock timestamps.

A local pull cursor advances only after the corresponding page has been durably applied locally.

### Redis/cache

Redis or process-local caches may improve performance, but no correctness property may depend on them.

Losing Redis must make the system slower, not incorrect.

## 4. Desktop security boundaries

The WebView is untrusted relative to privileged local filesystem/credential operations.

Do not:

- persist long-lived refresh credentials in `localStorage`, IndexedDB or ordinary frontend state;
- allow the WebView to submit arbitrary filesystem paths for snapshot/import/deployment;
- let the frontend forge built-in Agent profiles or built-in Agent Skill roots;
- let an Agent adapter decide cloud authorization.

Long-lived credentials belong behind the Rust credential boundary using operating-system secure credential facilities in M2.3.

Skill workspaces must be allocated under SkillHive-managed local directories by Rust.

## 5. Skill content model

A Skill is a directory/package, not only a Markdown string.

The canonical desktop package can contain at least:

- `SKILL.md`;
- `scripts/`;
- `references/`;
- `assets/`;
- other validated portable files.

Use immutable snapshot manifests referencing SHA-256 file blobs.

Do not deploy directly from a mutable workspace.

Snapshot rules include:

- no path traversal;
- no symlink following;
- bounded file count, file size and package size;
- portable filename rules across Windows/macOS/Linux;
- stable content identity independent of Unix permission bits.

## 6. Agent deployment invariants

Agent deployment is transactional.

Install/update pattern:

`intent journal -> staging -> verify -> prepared -> backup old -> atomic activate -> verify active -> SQLite catalog -> journal ACK`

Uninstall pattern:

`journal -> active rename to quarantine -> SQLite catalog delete -> permanent delete -> journal ACK`

Recovery policy is conservative:

- roll forward only when the new snapshot is verified;
- if new state is uncertain and an old verified backup exists, restore the old version;
- never follow symlinks during deployment/recovery/uninstall.

Built-in Agent profiles must be derived by Rust adapters. Custom profiles may use user-selected directories but must pass Rust validation.

## 7. Local cache invariants

Never evict content required by:

- dirty/unresolved local work;
- active managed workspace;
- pinned Skill;
- installed deployment;
- any unacknowledged mutation snapshot.

Evict only recoverable clean content. Prefer claiming local state as `remote_only` transactionally before deleting physical cached blobs, so a crash can at worst leak disk space rather than falsely claim content exists.

## 8. Current milestone policy

Milestone states use these exact meanings:

- `VERIFIED`: executed locally and validation passed.
- `CODE COMPLETE / PENDING LOCAL VALIDATION`: implementation is believed complete by static review but has not been executed.
- `IN PROGRESS`: known implementation work remains.
- `PLANNED`: design exists but implementation has not started.

Never promote `CODE COMPLETE` to `VERIFIED` without running the local validation checklist.

At handoff:

- M0: complete.
- M1: **CODE COMPLETE / PENDING LOCAL VALIDATION**.
- M2.0 (#5): **CODE COMPLETE / PENDING LOCAL VALIDATION**.
- M2.1 (#6): **IN PROGRESS**.
- M2.2–M2.7 (#7–#12): planned.
- M3/M4: planned.

See `docs/development/LOCAL_AGENT_HANDOFF.md` for exact details.

## 9. Immediate next task

Continue **M2.1 / Issue #6** before starting M2.2.

Do not jump directly to a sync worker.

Finish and locally validate:

- ORM/Alembic schema parity;
- historical migration/backfill behavior;
- protocol v1 model constraints;
- opaque cursor codec;
- desktop SQLite v3 migration;
- per-Skill outbox ordering;
- fresh DB and upgrade-from-old-schema behavior.

Only then proceed to M2.2 (`BlobStorage`) and M2.3 (device identity/secure credentials).

## 10. Validation and CI policy

The project owner explicitly requested that development **not use GitHub Actions for routine validation** because Actions quota is limited.

Therefore:

- do not add or enable `.github/workflows/*` unless the owner explicitly requests it;
- do not treat absence of CI as permission to skip validation;
- run validation locally according to `docs/development/LOCAL_VALIDATION_CHECKLIST.md`;
- record failures and fixes in local commits/PR notes.

At handoff, backend tests, mypy, Ruff, Cargo, Tauri runtime and fault-injection scenarios added during this refactor have **not yet been executed**.

## 11. Required local quality gates

Before marking a milestone verified, run the applicable local checks.

Backend baseline:

```bash
uv sync --dev
uv run ruff check backend
uv run mypy backend/app backend/tests
uv run pytest
```

Alembic baseline:

```bash
uv run alembic upgrade head
uv run alembic current
```

Also test migration from a database at the previous Alembic head; do not validate only a fresh database.

Frontend baseline:

```bash
cd frontend
pnpm install
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

Rust/Tauri baseline:

```bash
cd frontend/src-tauri
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Then run the desktop application and fault scenarios listed in the local validation checklist.

If repository/toolchain details make a command invalid, fix the documented command together with the code; do not silently skip the check.

## 12. Git/branch rules

- Continue work on `feat/desktop-foundation` unless the owner explicitly asks for another branch.
- Do not directly rewrite `main`.
- Keep PR #3 as the integration PR until the owner decides otherwise.
- Prefer focused commits with one correctness concern per commit.
- Do not squash away useful migration/history checkpoints before local validation.

## 13. Files/directories not to commit

Follow `.gitignore`.

Never commit:

- `.env` or environment-specific secret files;
- local database files/WAL/SHM/journals;
- local credential/key material;
- SkillHive runtime cache/workspace/blob/deployment-journal directories;
- `node_modules`, Vite output/coverage/cache;
- Rust `target/` build output;
- Python virtualenv/cache/test/coverage output;
- editor/OS temporary files;
- logs and temporary working directories.

Do **not** add `Cargo.lock` to ignore rules. SkillHive Desktop is an application; once Cargo generates a lockfile and the dependency set is locally validated, the lockfile should normally be committed for reproducible desktop builds.

Do not edit `makewiki/` or legacy README files merely to make them match implementation during active refactoring. Update user-facing docs only when explicitly scheduled; code and the handoff documents are the current engineering source of truth.

## 14. Files that require special care

Treat the following as high-risk correctness files:

- `backend/app/services/skill_mutations.py`
- `backend/app/models/domain.py`
- `backend/app/models/sync.py`
- `backend/migrations/versions/*sync*`
- `backend/app/schemas/sync.py`
- `frontend/src-tauri/src/local_store/migrations.rs`
- `frontend/src-tauri/src/local_store/mutations.rs`
- `frontend/src-tauri/src/deployment.rs`
- `frontend/src-tauri/src/uninstall.rs`
- `frontend/src-tauri/src/blob_store.rs`
- `frontend/src-tauri/src/skill_snapshot.rs`

Changes to these files must preserve crash/retry/migration semantics and should include or update tests.

## 15. Architectural non-goals for the current phase

Do not introduce these prematurely:

- microservice decomposition;
- Redis-dependent locking/correctness;
- automatic merge of conflicting Skill packages;
- online-only local saves;
- silent LWW conflict resolution;
- arbitrary filesystem access from the WebView;
- symlink-based deployment as the default;
- M3 entitlement/offline-permission lease logic inside M2 protocol code;
- broad UI redesign before sync/storage correctness is stable.

## 16. Handoff documentation maintenance

When completing a work package:

1. update the relevant GitHub issue status;
2. update `docs/development/LOCAL_AGENT_HANDOFF.md` milestone table and known risks;
3. update `docs/development/LOCAL_VALIDATION_CHECKLIST.md` if new failure modes exist;
4. keep `AGENTS.md` stable unless a project-level invariant changes.
