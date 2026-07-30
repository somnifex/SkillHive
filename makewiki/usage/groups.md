# Groups

Groups define a collaboration boundary for members, templates, and enabled global skills.

## Create a group

Any active user can create a group and becomes its owner. Choose its name, slug, description, and
joining policy:

- **Public:** users can join without approval.
- **Approval required:** requests wait for an owner or administrator.
- **Invite only:** users join through invitations sent by group management.

Invite-link joining is reserved in the data model but is not implemented as a complete user
workflow.

## Invite or approve members

Owners and administrators can invite a user. Members can invite only when the group's
`allow_member_invite` setting permits it. The recipient accepts or declines from their invitation
list.

For approval-required groups, management reviews pending requests and approves or rejects them.
An invite-only group rejects unsolicited join requests.

## Manage roles

- Owners can appoint or remove group administrators and manage all members.
- Administrators can manage ordinary members but cannot take owner-only actions.
- Members can view shared resources and leave.

The owner must transfer ownership or dissolve the group instead of leaving it directly. Ownership
transfer should be confirmed carefully because it changes who controls administrators and
dissolution.

## Group lifecycle

Management can update group settings. The owner alone can transfer ownership or dissolve the
group. Platform administrators can also change a group's platform status through the
administration area.
