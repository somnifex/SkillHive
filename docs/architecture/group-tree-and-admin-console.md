# Group Tree, Admin Console, Client Server Address & Zip Packaging

Status: APPROVED (owner plan approval, 2026-09-07)
Scope: backend + web frontend + Tauri desktop
Branch: `feat/group-tree-admin` (forked from `main` @ `6f4c604`)
Related: `docs/architecture/m2-cloud-sync-plan.md` (storage/sync protocol), `docs/architecture/desktop-enterprise-roadmap.md`

This document is the design of record for the four work packages approved by the
owner. Every design decision below was confirmed or defaulted at plan approval
time; defaults are marked. It complements (and never overrides) the reliability
invariants in `AGENTS.md`.

## 0. Approved decisions at a glance

| ID | Decision | Notes |
| --- | --- | --- |
| D1 | Zip is an **import/export packaging format only**. The canonical server-side storage unit remains the SHA-256 content-addressed blob + immutable manifest (M2.2). | Default adopted at approval. The server never stores zip archives. Incremental sync, dedup, per-file verification and manifest-closure checks are preserved unchanged. |
| D2 | Group tree with **full permission inheritance**: an owner/admin of any ancestor is an effective manager of every descendant. Owner-only operations do **not** expand down the tree. Skill grants are **not** inherited (v1). | Default adopted at approval. |
| D3 | `users.is_global_admin` is the **implicit tree root**: effective owner of every group, plus system-console powers (storage backend selection, user lifecycle, platform parameters). | Default adopted at approval. S3 credentials stay in server env vars only — never in DB or UI. |
| D3.5 | The login screen gains a user-configurable **server address**; desktop persists it outside the WebView and rebuilds `SyncClient` on change; keyring credentials are namespaced per server. | Default adopted at approval. |
| D4 | Delivery: this design doc first, then one milestone per phase, each with focused commits and full local validation. | Owner-approved. |

## 1. Group tree data model

### 1.1 Schema

`groups` gains a self-reference:

- `parent_id String(36) NULL REFERENCES groups(id)` — `NULL` means root-level
  (all existing rows are roots; the migration is purely additive).
- `CHECK (parent_id IS NULL OR parent_id != id)` — no self-parent.
- Index `ix_groups_parent_id` on `parent_id`.
- **Depth limit**: `MAX_GROUP_DEPTH = 16` (levels below the root, so a chain of
  17 groups is invalid). Enforced in the service layer at create/move time.
- **Cycle prevention**: moving a group under one of its own descendants (or
  itself) is rejected with `GROUP_CYCLE` before any write.

Migration `e8f1a2b3c4d5_add_group_tree.py` (down_revision `c4d5e6f7a8b9`) adds
the column, index and CHECK via `batch_alter_table` (same pattern as
`c4d5e6f7a8b9`, works on SQLite and PostgreSQL). Downgrade drops all three.

### 1.2 Integrity rules

- A group may only be created under an **active** parent.
- `dissolve` is refused while **active child groups exist**
  (`GROUP_HAS_CHILDREN`, 409). Because dissolve already requires this, an
  active group's ancestor chain always consists of active groups — inheritance
  queries never need to reason about deleted intermediates.
- Reparenting (`PATCH /groups/{id}` with `parent_group_id`) requires the actor
  to be the **local owner** of the moved group (or a global admin) AND an
  effective manager of the target parent. Setting `parent_group_id: null`
  moves the group to the root.
- Deleting/archiving a parent via the admin status endpoint does not cascade;
  children keep their `parent_id`. Inheritance queries stop at non-active
  ancestors (chain is broken at the first inactive node), so an archived
  parent's subtree becomes independently governed rather than orphaned-blind.

## 2. Permission model

### 2.1 Effective role

Stored membership stays exactly as today (`group_members.role` per group).
The service layer computes one **effective role** per (actor, group):

```
effective_role(group, actor):
  if actor.is_global_admin:            return "owner"     # implicit tree root (D3)
  own = membership(group, actor)       # active row, may be None
  if own.role == "owner":              return "owner"
  if own.role == "admin":              return "admin"
  if any ancestor a of group has membership(actor, a).role in {owner, admin}:
                                       return "admin"     # inherited manager
  if own.role == "member":             return "member"
  return None                                             # no relationship
```

Key consequences (deliberate):

- **Inheritance never produces "owner"** (except the global admin). Therefore
  owner-only operations do not expand down the tree, exactly as approved in D2.
- Ancestor membership is looked up along the parent chain with a single
  recursive CTE (`repositories/groups.py: effective_roles()`), which resolves
  many groups in one query and works on SQLite (tests) and PostgreSQL (prod).
- Nothing is cached; every request resolves roles from the database. This
  respects the AGENTS.md rule that no correctness property may depend on caches.

### 2.2 Permission matrix

| Operation | member | admin (own or inherited) | owner (local) | global admin |
| --- | --- | --- | --- | --- |
| View group / members / enabled skills | ✔ (must have any effective role) | ✔ | ✔ | ✔ |
| Invite (unless `allow_member_invite` opens it to members) | – | ✔ | ✔ | ✔ |
| Review join requests, update group basics (not join policy) | – | ✔ | ✔ | ✔ |
| Remove a plain member | – | ✔ (plain members only) | ✔ | ✔ |
| Enable/adjust/revoke group skill grants | – | ✔ | ✔ | ✔ |
| Create sub-group under this group | – | ✔ | ✔ | ✔ |
| Reparent this group | – | – | ✔ | ✔ |
| Change `join_policy` / `allow_member_invite` | – | – | ✔ | ✔ |
| Set/withdraw admin roles | – | – | ✔ | ✔ |
| Transfer ownership | – | – | ✔ | ✔ |
| Dissolve | – | – | ✔ (no active children) | ✔ |

The existing admin-cannot-remove-admin/owner rule is preserved and now applies
to *effective* admins: an inherited admin may only remove plain `member` rows
of the descendant.

### 2.3 Sync/visibility unchanged (important)

Per the approved plan, `sync_changes.py` / `skill_mutations.py` visibility
continues to key off **direct active membership + active grant**. Tree
inheritance does not widen what appears in a user's change feed, and grants do
not flow down the tree (D2). A future "grant inheritance" flag would be an
explicit follow-up.

## 3. API changes (Phase 1)

- `POST /groups` — `GroupCreate.parent_group_id: str | None = None`. Requires
  effective-manager on the parent (ordinary groups: any logged-in user may
  still create root groups).
- `PATCH /groups/{id}` — `GroupUpdate.parent_group_id: str | None = None` via
  explicit `set_parent` semantics: field present → move (see §1.2); absent →
  unchanged. Errors: `GROUP_CYCLE` (400), `GROUP_DEPTH_EXCEEDED` (400),
  `GROUP_NOT_FOUND` (404), `PERMISSION_DENIED` (403).
- `GET /groups/tree` — returns `list[GroupRead]` (flat, parent-linked; the
  client builds the tree). Visibility: groups where the actor has any effective
  role, plus — for inherited managers — every active descendant of groups they
  manage. Global admins see the full active tree.
- `GET /groups` (paged) — unchanged shape; `current_user_role` now carries the
  **effective** role.
- `GroupRead` adds `parent_id: str | None` and `parent_name: str | None`
  (convenience for breadcrumbs/tree labels).
- All existing endpoints keep their routes and semantics; role checks are
  replaced by the effective-role gate described above. Non-member global admins
  stop receiving 404 on `/groups/*` (D3).

## 4. Admin console & system settings (Phase 2)

- New `system_settings` table: singleton row / key-value JSON, edited only by
  `GlobalAdmin` endpoints (`GET/PATCH /admin/system/settings`).
- Settings: `blob_storage_backend` (`local` | `s3`), `s3_endpoint_url`,
  `s3_bucket`, `s3_prefix`, `s3_region`, `allow_registration`,
  `max_package_bytes` override, plus future platform parameters.
- **S3 credentials (access/secret key) are env-only** (`S3_ACCESS_KEY_ID`,
  `S3_SECRET_ACCESS_KEY`); never stored in the DB, never echoed by the API.
- `S3BlobStorage` implements the existing `BlobStorage` ABC
  (`services/blob_storage.py`) with identical verify-on-write semantics;
  `skill_blob_objects.storage_backend` (column already exists) records where
  each object lives. Reads route by the per-object record, writes go to the
  currently selected backend — a backend switch therefore takes effect for new
  objects immediately without any downtime or data move; an offline
  `migrate-blobs` script can copy old objects afterwards.
- User lifecycle: `POST /admin/users`, `POST /admin/users/{id}/reset-password`,
  `DELETE /admin/users/{id}` (soft delete, refused while the user still holds
  active ownership of groups/skills).
- `GET /admin/groups/tree` for the full tree view.
- Frontend `AdminPage.tsx` gains System Settings + user-management surfaces.

## 5. Client server address (Phase 3)

- Login/register screens gain a "Server address" field with a Test button
  (probes `GET /auth/me` semantics via an unauthenticated ping endpoint).
  Defaults: desktop `http://127.0.0.1:8000`, web same-origin `/api/v1`.
- Web: the address (non-sensitive) lives in `localStorage`; axios `baseURL`
  becomes dynamic; the refresh call follows the same address.
- Desktop: the address is persisted by **Rust** in the app config directory
  (never WebView storage), exposed via `get_server_url` / `set_server_url`
  commands; the managed `SyncClient` is rebuilt on change. Keyring accounts are
  namespaced per server URL so credentials for two servers cannot cross.
- Cross-origin webview calls may need backend CORS with credentials; the
  desktop is expected to authenticate with Bearer tokens from the keyring, so
  the HttpOnly refresh cookie remains a web-only mechanism.

## 6. Zip import/export (Phase 4)

- Desktop-only entry points, always through system file dialogs (the WebView
  never submits arbitrary paths — AGENTS.md §4).
- `import_skill_zip(zip_path)`: Rust extracts to a managed staging dir with
  zip-slip protection (reject `..`/absolute/UNC paths and symlink entries),
  re-uses every `skill_snapshot.rs` bound (file count / per-file / total size,
  portable filenames), then `capture_workspace` → existing commit+outbox flow.
  Zip entry sizes are validated from the central directory before extraction
  to avoid decompression bombs.
- `export_skill_zip(skill_id, dest_path)`: materialize the verified snapshot
  from the local blob store into a temp dir, then zip it.
- No server or sync-protocol changes (D1).

## 7. Testing & validation

- Backend: effective-role matrix (ancestor admin manages descendant; ancestor
  member cannot; inherited admin cannot remove descendant admins; global admin
  full access), cycle/depth rejection, dissolve-with-children, reparent,
  `GET /groups/tree` visibility, migration upgrade from `c4d5e6f7a8b9` and
  fresh-DB parity.
- Frontend: tree rendering, sub-group creation, effective-role display.
- Rust (Phase 4): zip-slip samples, oversize entries, malformed archives.
- Every phase runs the full AGENTS.md §11 gates before commit.

## 8. Risks & rollback

- The migration is additive (nullable column); rollback is the inverse DDL.
- Effective-role computation is the only behavioral change to existing
  endpoints; it is centralized in `GroupService` context helpers so a revert
  is a single-file change.
- Parallel UI-revamp branch (`feat/wanhua-ui-revamp`) touches the same page
  files; merging order must be decided by the owner (rebase this branch after
  the UI revamp lands, resolving page-level conflicts manually).
- Known limitation inherited from the environment: `origin` is reachable only
  via SSH which is unavailable in this workspace, so the branch is forked from
  the local `origin/main` ref (`6f4c604`). Re-run `git fetch` and rebase before
  opening a PR.
