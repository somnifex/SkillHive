# Platform administration

The **Administration** area is available only to platform administrators. It governs shared
platform state; it does not provide a back door into private skill or personal-template bodies.

## Users and groups

Administrators can list users, change account status, list all groups, inspect membership, adjust
member roles, and change group status. Use suspension for access control; preserve records needed
for audit and investigation.

Group ownership and role changes can remove someone's ability to manage shared resources. Confirm
the target user and group before applying them.

## Global skills

1. Create the global skill as a draft.
2. Add an immutable version.
3. Review content and metadata.
4. Publish the intended version.
5. Grant the skill to selected groups.

A published skill can later be disabled or archived. These state changes may affect every group
using it, especially those following the latest-version policy.

## Global templates

Platform administrators create and maintain templates available to every signed-in user. A skill
created from one remains private to its creator.

## Audit logs

Audit records cover authentication, account, group, skill, template, and grant actions. Use
filters and pagination to investigate changes. Treat audit exports or screenshots as sensitive:
they can contain identifiers and operational metadata.

## Operational discipline

Use a separate normal account for day-to-day private work. Reserve the administrator account for
governance, review high-impact state changes, and rotate its password away from the demo default
before allowing any shared access.
