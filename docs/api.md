# API reference

SkillHive exposes a JSON REST API under `/api/v1`. The generated OpenAPI document from the running
application is authoritative for request/response schemas; this page records authentication,
resource groups, and integration behavior that clients must understand.

## Base URLs and errors

Local base URL: `http://127.0.0.1:8000/api/v1`.

Successful responses use standard HTTP status codes. Validation and application failures return a
consistent JSON error body with a machine-readable code/message where the relevant endpoint
defines one. Clients must not depend on human-readable text alone.

`GET /health` is unauthenticated and can be used for basic liveness checks.

## Authentication model

`POST /auth/login` returns a short-lived bearer access token and sets a rotating refresh token in
an HttpOnly cookie. Send the access token as:

```http
Authorization: Bearer <access-token>
```

When it expires, call `POST /auth/refresh` with credentials/cookies enabled. The server rotates the
refresh session. `POST /auth/logout` revokes it; `POST /auth/change-password` revokes existing
refresh sessions.

Browser clients must use an origin listed in `CORS_ORIGINS`. The project frontend holds the access
token in memory and relies on the refresh cookie rather than local storage.

The forgot-password endpoint returns `202` without revealing whether an account exists. It is
currently a placeholder: it does not deliver email or modify credentials.

## Endpoint groups

### System and accounts

| Method | Path                    | Purpose                                   |
| ------ | ----------------------- | ----------------------------------------- |
| GET    | `/health`               | Health check                              |
| POST   | `/auth/register`        | Register user and create starter template |
| POST   | `/auth/login`           | Authenticate and create refresh session   |
| POST   | `/auth/refresh`         | Rotate session and issue access token     |
| POST   | `/auth/logout`          | Revoke current refresh session            |
| GET    | `/auth/me`              | Current account                           |
| POST   | `/auth/change-password` | Change password and revoke sessions       |
| POST   | `/auth/forgot-password` | Privacy-preserving placeholder response   |

### Private skills

| Method             | Path                          | Purpose                                     |
| ------------------ | ----------------------------- | ------------------------------------------- |
| GET, POST          | `/skills`                     | Search/list or create owned skills          |
| GET, PATCH, DELETE | `/skills/{skill_id}`          | Read, update, or soft-delete an owned skill |
| POST               | `/skills/{skill_id}/copy`     | Copy into another owned skill               |
| GET, POST          | `/skills/{skill_id}/versions` | List or create immutable versions           |

Every private-skill operation applies an owner predicate. Platform administrator status does not
bypass it.

### Templates

| Method             | Path                                   | Purpose                                              |
| ------------------ | -------------------------------------- | ---------------------------------------------------- |
| GET, POST          | `/templates`                           | List visible templates or create an authorized scope |
| GET, PATCH, DELETE | `/templates/{template_id}`             | Read or manage an authorized template                |
| POST               | `/templates/{template_id}/instantiate` | Generate a new private skill                         |

Personal templates are owner-only, group templates require group membership to use and group
management to change, and global templates are managed by platform administrators.

### Groups and membership

| Method             | Path                                            | Purpose                                       |
| ------------------ | ----------------------------------------------- | --------------------------------------------- |
| GET, POST          | `/groups`                                       | List joined/discoverable groups or create one |
| GET, PATCH, DELETE | `/groups/{group_id}`                            | Read, update, or dissolve a group             |
| GET                | `/groups/{group_id}/members`                    | List group membership                         |
| POST               | `/groups/{group_id}/members/invite`             | Create a user invitation                      |
| PATCH, DELETE      | `/groups/{group_id}/members/{user_id}`          | Change role or remove member                  |
| GET                | `/groups/invitations`                           | List current user's invitations               |
| POST               | `/groups/invitations/{invitation_id}/accept`    | Accept invitation                             |
| POST               | `/groups/invitations/{invitation_id}/decline`   | Decline invitation                            |
| GET, POST          | `/groups/{group_id}/join-requests`              | List management queue or request joining      |
| PATCH              | `/groups/{group_id}/join-requests/{request_id}` | Approve or reject request                     |
| POST               | `/groups/{group_id}/leave`                      | Leave as a non-owner                          |
| POST               | `/groups/{group_id}/transfer-ownership`         | Transfer owner role                           |

The `invite_link` policy value is reserved but no complete invite-link endpoint/workflow exists.

### Group skills

| Method | Path                                                    | Purpose                              |
| ------ | ------------------------------------------------------- | ------------------------------------ |
| GET    | `/groups/{group_id}/skills`                             | List skills enabled for members      |
| GET    | `/groups/{group_id}/skills/catalog`                     | List grant-eligible published skills |
| GET    | `/groups/{group_id}/skills/catalog/{skill_id}/versions` | List eligible versions               |
| POST   | `/groups/{group_id}/skills/{skill_id}`                  | Enable with latest or locked policy  |
| PATCH  | `/groups/{group_id}/skills/{skill_id}`                  | Change version policy                |
| DELETE | `/groups/{group_id}/skills/{skill_id}`                  | Disable for the group                |

### Platform administration

All `/admin/*` routes require platform-administrator status.

- `/admin/users` and `/admin/users/{user_id}/status`
- `/admin/groups`, group status, membership, and member-role endpoints
- `/admin/skills`, skill versions, publish, disable, and archive actions
- `/admin/skills/{skill_id}/grants` and group-grant deletion
- `/admin/audit-logs`

Global skill creation/versioning uses both `/admin/skills` and
`/admin/skills/{skill_id}/versions`. Grant creation is performed through the applicable global
skill administration operation exposed by OpenAPI; clients should generate against the running
schema rather than assuming an undocumented payload.

## Pagination and filtering

Collection endpoints that support pagination expose their exact query parameters and response
envelope in OpenAPI. Treat page numbering, totals, and filters as endpoint contracts; do not infer
them from UI state.

## Compatibility

The `/api/v1` prefix is the compatibility boundary for the initial release. Additive fields may be
introduced within v1. Clients should ignore unknown response fields and must not depend on database
identifiers belonging to another resource type or installation.
