# SkillHive Current Development Status

Updated: 2026-09-04

This file contains the **current dynamic repository state** and supersedes branch/PR metadata captured at the top of `LOCAL_AGENT_HANDOFF.md`.

## Repository state

- Repository: `somnifex/SkillHive`
- PR #3 (`feat: desktop enterprise foundation and M1 local core`) was **merged** into `main` on 2026-09-04.
- PR #3 merge commit was `4cf8fd10c3c3112dd0aa2d6e4216ae622e9602c2`; `main` may contain later handoff-only commits after that merge.
- The historical branch `feat/desktop-foundation` should no longer be used for new development.
- Continuation branch name reserved for local-agent takeover: **`feat/m2-sync`**. It should be kept aligned with the latest `main` before development resumes.
- No GitHub Actions workflow should be added for routine validation unless the owner explicitly changes that policy.

## Active branch for the local agent

Start from the latest `main`, then work on the continuation branch:

```bash
git fetch origin
git checkout main
git pull --ff-only origin main
git checkout feat/m2-sync 2>/dev/null || git checkout -b feat/m2-sync
git rebase origin/main
```

If the remote/local `feat/m2-sync` has no unique development work yet, resetting it to current `main` is acceptable. The important invariant is that M2 work starts from the latest merged handoff state, not from the old `feat/desktop-foundation` branch.

Do not directly develop on `main`.

## Current milestone state

| Milestone | Current state |
| --- | --- |
| M0 Desktop/architecture foundation | COMPLETE |
| M1 Durable local desktop core | CODE COMPLETE / PENDING LOCAL VALIDATION |
| M2 Cloud sync epic (#4) | IN PROGRESS |
| M2.0 Shared Skill mutation path (#5) | CODE COMPLETE / PENDING LOCAL VALIDATION |
| M2.1 Protocol/schema foundation (#6) | IN PROGRESS |
| M2.2 Package/blob storage (#7) | PLANNED |
| M2.3 Device identity/secure credentials (#8) | PLANNED |
| M2.4 Idempotent push (#9) | PLANNED |
| M2.5 Durable pull/change feed (#10) | PLANNED |
| M2.6 Desktop sync orchestrator (#11) | PLANNED |
| M2.7 Conflicts/reliability checkpoint (#12) | PLANNED |
| M3 Enterprise offline authorization | PLANNED |
| M4 Production hardening | PLANNED |

## Exact next task

Continue **M2.1 / Issue #6**.

Before starting M2.2, the local agent should make the current M1/M2.0/M2.1 baseline executable and truthful locally:

1. run backend Ruff/mypy/pytest;
2. run fresh and upgrade-path Alembic validation on SQLite;
3. run the same M2.1 migration path on PostgreSQL;
4. make an explicit MySQL support decision and validate it if retained;
5. run Rust fmt/check/test/clippy;
6. validate desktop SQLite schema v1 -> v2 -> v3;
7. validate per-Skill outbox causal ordering;
8. validate the existing M1 snapshot/deployment/recovery core;
9. fix any compile/runtime/migration failures before adding M2.2 features;
10. update Issue #6 and the handoff status when the results are known.

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

At this status checkpoint, the previous development session did **not** execute:

- `ruff`;
- `mypy`;
- backend `pytest`;
- Alembic upgrade against the new migration;
- PostgreSQL migration validation;
- Cargo compile/test/clippy;
- Tauri runtime;
- M1 filesystem/deployment fault injection.

The merge of PR #3 is therefore an integration event, **not a verification certificate**.

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
