# SkillHive Local Validation Checklist

Last updated: 2026-09-04
Validation mode: local developer machine only
CI policy: do not use GitHub Actions unless the owner explicitly authorizes it

This checklist defines what must be executed before a milestone can move from `CODE COMPLETE / PENDING LOCAL VALIDATION` to `VERIFIED`.

Read `AGENTS.md` and `LOCAL_AGENT_HANDOFF.md` first.

---

## 1. General rules

- Work on `feat/desktop-foundation` unless instructed otherwise.
- Start from a clean Git working tree or record any owner-local changes before testing.
- Never point migration/fault tests at production or irreplaceable data.
- Use disposable SQLite/PostgreSQL/MySQL databases.
- Do not commit `.env`, local DB files, credentials, build output, runtime cache/workspaces or test logs containing secrets.
- Record exact toolchain versions and the first failing command before changing code.
- Fix the implementation rather than weakening tests/constraints unless the design itself is proven wrong.
- A passing fresh install does not validate an upgrade migration. Test both.

---

# PART A — TOOLCHAIN BASELINE

## 2. Record local toolchain

Capture locally:

```bash
git status --short
git branch --show-current
git rev-parse HEAD
python --version
uv --version
node --version
pnpm --version
rustc --version
cargo --version
```

Expected Python project range is Python 3.12.x.

`frontend/package.json` currently declares pnpm 11.9.0.

`frontend/src-tauri/Cargo.toml` currently declares Rust minimum 1.77.2, but use a current compatible stable toolchain unless the repository later pins one more strictly.

---

# PART B — BACKEND STATIC/UNIT VALIDATION

## 3. Install/sync backend dependencies

From repository root:

```bash
uv sync --dev
```

Do not create/commit an environment-specific `.env` from test secrets.

## 4. Ruff

```bash
uv run ruff check backend
```

Expected: zero errors.

Do not blanket-ignore new warnings to make the command green.

## 5. mypy

```bash
uv run mypy backend/app backend/tests
```

The project uses strict mypy configuration.

Pay particular attention to:

- SQLAlchemy mapped types;
- Pydantic v2 validators/`Self` return types;
- sync model optional/required fields;
- migration-adjacent model imports;
- service transaction return types.

## 6. pytest

```bash
uv run pytest
```

At minimum verify the following suites include the new behavior:

- auth;
- private Skill CRUD;
- global Skill CRUD;
- template instantiation;
- M2.0 transaction-boundary tests;
- M2.1 protocol/cursor/revision tests.

If an unrelated legacy test breaks because an additive read field appeared, preserve backwards-compatible response behavior unless the API change was intentional.

---

# PART C — M2.0 TRANSACTION CONTRACT

## 7. Shared Skill mutation service

Explicitly verify these properties, not only endpoint response codes:

### Rollback contract

Call `SkillMutationService` inside a Session, do not commit, then rollback.

Expected after rollback:

- Skill absent;
- SkillVersion absent;
- audit row absent.

### Commit-owner contract

Call `SkillMutationService`, append another same-transaction row if useful, then commit from caller.

Expected:

- all rows durable together.

### No hidden commits

Search relevant new domain helpers for direct transaction completion:

```bash
rg "\.commit\(|\.rollback\(" backend/app/services/skill_mutations.py
```

Expected: no domain-layer commit/rollback.

Facade services may still own one request-level commit until the sync endpoint takes over its larger transaction.

### Online business write paths

Confirm these real Skill-producing/changing paths use the shared domain mutation layer:

- private create/update/create-version/delete;
- global create/update/create-version/publish/status;
- template instantiate.

Seed/bootstrap is not an online user mutation path.

---

# PART D — M2.1 ALEMBIC / SERVER SCHEMA

## 8. Fresh SQLite migration

Use a disposable DB path, for example:

```bash
rm -f tmp/m2-fresh.db tmp/m2-fresh.db-*
mkdir -p tmp
DATABASE_URL=sqlite:///./tmp/m2-fresh.db uv run alembic upgrade head
DATABASE_URL=sqlite:///./tmp/m2-fresh.db uv run alembic current
```

Expected Alembic head includes:

```text
b6a31d0f4c9e
```

Inspect tables/constraints with a local SQLite client or a short Python/SQLAlchemy script.

Expected new server objects:

```text
devices
skill_blob_objects
sync_mutation_receipts
sync_change_log
```

Expected Skill fields:

```text
skills.sync_revision
skills.current_package_hash
skill_versions.revision
skill_versions.package_manifest_hash
skill_versions.package_size_bytes
```

## 9. Fresh ORM schema parity

Tests currently use `Base.metadata.create_all()`.

Create a disposable ORM schema and compare behavior/constraints with Alembic-head schema.

Check especially:

- `(skill_id, revision)` uniqueness;
- nonnegative package size;
- nonnegative receipt revision;
- positive Skill revision convention;
- change-log sequence primary-key/autoincrement behavior;
- device uniqueness;
- receipt idempotency uniqueness.

If `alembic check` is usable with the local environment/database, use it as an additional signal, but still inspect semantic differences manually.

## 10. Upgrade-from-previous-head SQLite test

This is mandatory.

Create DB at the previous head:

```bash
rm -f tmp/m2-upgrade.db tmp/m2-upgrade.db-*
DATABASE_URL=sqlite:///./tmp/m2-upgrade.db uv run alembic upgrade 7f4c2b8a91de
```

Populate representative legacy data before running head migration. Include at least:

1. one private Skill with one version;
2. one Skill with several versions whose semantic versions do not correspond to technical revisions;
3. one global published Skill;
4. one soft-deleted Skill;
5. if the old schema permits it, a Skill with unusual timestamp ordering edge cases;
6. a Skill whose current version is not necessarily the latest row by semantic version.

Then:

```bash
DATABASE_URL=sqlite:///./tmp/m2-upgrade.db uv run alembic upgrade head
```

Verify:

- every Skill has `sync_revision >= 1`;
- each version has a positive deterministic `revision`;
- version revisions are unique per Skill;
- Skill baseline revision is at least its historical version count per the implemented migration logic;
- deleted Skill baseline emits delete change operation;
- non-deleted Skills emit upsert baseline changes;
- migration preserved old content/manifest/version IDs;
- no data is silently dropped.

Re-run `alembic upgrade head` to verify idempotent migration state handling.

## 11. PostgreSQL migration test

PostgreSQL is mandatory for enterprise server validation.

Use disposable local Docker/PostgreSQL credentials. The repository's `.env.example`/Compose defaults are development-only and must not be reused in shared/production environments.

Start local Postgres as appropriate, then set `DATABASE_URL` explicitly, for example conceptually:

```bash
DATABASE_URL=postgresql+psycopg://<local-user>:<local-password>@127.0.0.1:<port>/<test-db> \
  uv run alembic upgrade head
```

Run both:

- fresh migration;
- previous-head -> populated legacy data -> head migration.

Check:

- BIGINT sequence behavior;
- JSON backfill binding;
- constraint creation;
- index creation;
- nullable -> backfill -> NOT NULL transition;
- foreign keys and delete behavior.

Do not call M2.1 migration portable based only on SQLite success.

## 12. MySQL compatibility decision

The repository currently has optional MySQL dependencies/profile history.

The local agent must make an explicit decision:

- if MySQL remains supported, run the same migration path on MySQL 8.x and fix portability;
- if it is intentionally dropped from the enterprise support matrix, document that decision in architecture/configuration rather than silently letting migrations fail.

Do not leave accidental pseudo-support.

## 13. Migration downgrade

Downgrade is lower priority than forward safety but should be exercised on disposable data:

```bash
DATABASE_URL=<disposable-url> uv run alembic downgrade 7f4c2b8a91de
```

Expected: schema objects added by M2.1 are removed cleanly.

Do not use downgrade as a production rollback strategy without a data-retention analysis.

---

# PART E — M2.1 PROTOCOL CONTRACT

## 14. Pydantic protocol v1 validation

Run `backend/tests/test_sync_protocol.py` and add missing cases if needed.

Required cases:

### Create

Accept:

- protocol version 1;
- valid device/mutation UUIDs;
- no remote ID;
- base revision null or zero;
- valid package SHA-256;
- valid metadata.

Reject:

- non-null remote ID;
- positive/nonzero incompatible create base revision;
- missing package hash;
- missing metadata;
- malformed hash;
- unsupported protocol version;
- extra unknown fields.

### Update

Require:

- remote ID;
- `baseRevision >= 1`;
- package hash;
- metadata.

Reject revision zero.

### Delete

Require:

- remote ID;
- `baseRevision >= 1`.

Reject package hash on delete.

### Blob negotiation

Verify:

- duplicate hash rejection;
- object count bound;
- per-object size bound;
- total declared package bound;
- malformed hash rejection.

## 15. Cursor codec

Required properties:

- `None`/empty starting cursor maps to sequence zero where intended;
- encode/decode round-trip;
- stable `v1.` prefix;
- malformed base64 rejected;
- wrong byte length rejected;
- oversized input rejected;
- unsupported prefix/version rejected;
- max valid integer behavior tested.

The cursor is opaque at API boundary even if implementation currently encodes one sequence integer.

---

# PART F — DESKTOP RUST / SQLITE VALIDATION

## 16. Format/check/test/clippy

From `frontend/src-tauri`:

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

The first Cargo run should generate a `Cargo.lock` for this application. Review it and normally commit it after dependency/toolchain validation.

Do not add `Cargo.lock` to `.gitignore`.

## 17. Local SQLite migration chain

Test a new DB reaches schema v3.

Also create representative schema-v1 and schema-v2 databases and reopen through current `LocalStore`.

Verify:

- migration records advance in order;
- existing Skill rows survive;
- existing mutation IDs/states survive v3 rebuild;
- `local_sequence` is deterministic and positive;
- `local_sync_state` singleton exists;
- no duplicate sequences per Skill;
- foreign keys remain valid;
- WAL configuration succeeds after migration.

## 18. Per-Skill causal outbox

Required cases:

### Create + update

```text
create seq=1
update seq=2
```

Before ACK of seq 1, only seq 1 is dispatchable.

After seq 1 ACK, seq 2 becomes dispatchable.

### Create + update + delete

Later operations remain blocked until every earlier operation is ACKed/resolved.

### Conflict blocking

If seq 1 becomes `conflict`, seq 2 must not become dispatchable automatically.

### Permission/permanent failure blocking

Same behavior as conflict until explicit resolution/policy changes the dependency chain.

### Restart in-flight

Claim a mutation, simulate restart, call recovery.

Expected:

- same mutation ID;
- retryable state;
- no duplicate row;
- later same-Skill mutation still blocked.

## 19. Local sync state persistence

Set and reopen:

- client instance ID;
- device ID;
- server user ID;
- cursor;
- last successful push/pull timestamps;
- last server error.

Expected values survive process reopen exactly.

No credentials belong in this table.

---

# PART G — M1 SNAPSHOT/WORKSPACE/BLOB VALIDATION

## 20. Snapshot round-trip

Build a test Skill workspace containing:

```text
SKILL.md
scripts/example.py
references/info.md
assets/small-binary-or-text-fixture
```

Capture snapshot, materialize into a new directory, verify.

Expected byte-identical files and stable manifest identity.

## 21. Snapshot limits/security

Verify rejection of:

- path traversal;
- symlink file;
- symlink directory;
- broken symlink;
- too many files;
- file over configured maximum;
- package over configured maximum;
- Windows-reserved path names;
- trailing dot/space;
- forbidden separators/characters;
- duplicate logical path if constructible;
- file modified concurrently during capture.

## 22. BlobStore tamper tests

Verify:

- same bytes -> same hash;
- corrupted physical blob is detected when verified;
- storage path symlink is rejected;
- temporary write interruption does not expose a partial blob as valid;
- existing valid object is not corrupted by duplicate write.

## 23. Managed workspace boundary

Verify frontend-facing API cannot request snapshot of `/etc`, home directory, another project directory, etc.

Workspace paths must derive from SkillHive-managed root + Skill identity.

---

# PART H — M1 AGENT / DEPLOYMENT FAULT TESTS

## 24. Agent root discovery

On each available platform/Agent installation, verify detected roots match actual product behavior.

At minimum verify configured behavior for:

- Claude Code;
- Codex / unified Agent Skills root;
- custom profile.

Record unsupported/nonexistent Agent cases as clean discovery results, not fatal desktop startup failures.

## 25. Built-in profile forgery

Attempt to persist a built-in descriptor/profile ID with an arbitrary root.

Expected: Rust/local persistence rejects mismatch.

Custom profile should remain possible if path passes validation.

## 26. Install happy path

Deploy an immutable snapshot to a disposable Agent root.

Verify:

- staging verified before activation;
- target content matches snapshot;
- SQLite catalog records exact deployed hash/path;
- journal removed only after catalog ACK;
- repeated update to a new snapshot leaves no mixed files.

## 27. Kill/fault injection phases — install/update

Simulate termination/error after each conceptual phase:

```text
intent written
partial staging copy
staging complete before Prepared
Prepared
old active moved to backup
new active moved into place
active verified before SQLite catalog
SQLite catalog committed before journal ACK
```

Restart and run recovery.

Expected invariant:

- never accept unverified new content;
- restore old known-good content when new state is uncertain and backup exists;
- if filesystem is valid but SQLite commit was lost, replay catalog safely;
- no permanent half-mixed Agent directory.

## 28. Uninstall happy path and faults

Test failure/termination after:

```text
uninstall intent
active -> quarantine
SQLite catalog removal
before quarantine deletion
before journal ACK
```

On restart, catalog authority determines rollback/finalize behavior.

## 29. Broken symlink / permission behavior

Exercise if platform allows:

- broken target symlink;
- root symlink;
- read-only target root;
- file locked by another process;
- antivirus/indexer-style rename interference on Windows.

Expected: explicit error/recoverable journal state, not silent traversal or partial overwrite.

---

# PART I — CACHE VALIDATION

## 30. Eviction protection

Confirm cache does not evict snapshots required by:

- dirty Skill;
- conflict Skill;
- active workspace;
- pin;
- deployment;
- older unacknowledged mutation payload even if it is not the current Skill snapshot.

## 31. Shared blob references

Two snapshots sharing a blob must not cause that blob to be physically deleted while either protected/referenced snapshot still requires it.

## 32. Crash during eviction

Inject failure after SQLite state claim but before/during physical deletion.

Acceptable outcome: extra local bytes remain.

Unacceptable outcome: DB says complete cached content exists after required physical content was deleted without a recoverable path.

---

# PART J — FRONTEND VALIDATION

## 33. Frontend baseline

From `frontend`:

```bash
pnpm install
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

Record the lockfile behavior. Preserve the existing package manager choice.

## 34. Desktop dev command

The branch has not yet established a locally verified Tauri developer command.

Determine and document a reproducible path.

Preferred outcome is a project-pinned Tauri CLI/developer dependency rather than undocumented global installation, if compatible with the project.

Do not solve this by adding GitHub Actions.

## 35. Web compatibility

Existing Vite web development/build should remain usable while desktop integration evolves unless the product owner explicitly retires web mode.

Verify Tauri-specific Vite configuration does not break normal frontend tests/build.

---

# PART K — M2.2+ FUTURE VALIDATION GATES

These sections become active as the corresponding implementation lands.

## 36. Blob transport

Required future tests:

- missing-object negotiation;
- duplicate hash request;
- concurrent upload same hash;
- declared size smaller/larger than stream;
- digest mismatch;
- interrupted upload;
- object exists metadata but bytes missing;
- filesystem backend restart;
- S3 backend failure mapping;
- package closure with one missing child blob.

## 37. Device/credential

Required future tests:

- registration repeated with same client instance;
- another user cannot use device ID;
- revoked device rejected;
- refresh secret absent from WebView/localStorage/SQLite;
- token refresh failure preserves mutation queue;
- OS keyring unavailable/locked error is surfaced without plaintext fallback.

## 38. Idempotent push

Required future tests:

- same mutation request repeated many times -> one server effect;
- response lost after DB commit;
- unique receipt race;
- two devices same base revision -> one commits, one conflicts;
- permission checked at execution time;
- invalid package closure -> no Skill mutation/change/receipt success;
- transaction rollback leaves no partial change-feed/receipt.

## 39. Durable pull

Required future tests:

- pagination stable by sequence;
- page apply fails -> cursor unchanged;
- restart resumes exact cursor;
- browser REST update emits event;
- tombstone survives filtering/pagination;
- unauthorized private content not projected;
- remote update while local dirty -> explicit conflict/reconciliation state.

## 40. Sync worker

Required future tests:

- offline startup;
- online recovery;
- app shutdown during upload;
- app shutdown during push;
- app shutdown after server ACK before local ACK;
- app shutdown during pull apply;
- persisted backoff survives restart;
- no tight retry loop;
- unrelated Skills can eventually progress without violating same-Skill causality.

---

# PART L — RELEASE STATE RECORD

## 41. How to record validation

When a checkpoint passes, add a short dated section to the relevant issue and update `LOCAL_AGENT_HANDOFF.md` with:

```text
Validated date:
OS/platform:
Python/Node/Rust versions:
DB dialects tested:
Commands passed:
Fault scenarios passed:
Known skipped scenarios and why:
```

Do not write `verified` if mandatory scenarios were skipped.

## 42. Current handoff state

At the time this file was created:

- no GitHub Actions validation is enabled;
- M1 is code-complete but not locally verified;
- M2.0 is code-complete but not locally verified;
- M2.1 is in progress and unverified;
- M2.2+ have not been implemented.

The local agent should begin by making M2.1 and the prerequisite M1/M2.0 build/test baseline truthful, then continue the issue dependency chain.
