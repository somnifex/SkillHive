# SkillHive user guide

SkillHive is a self-hosted workspace for turning reusable instructions into versioned AI skills.
Individuals keep private work under their own account, teams collaborate through groups, and
platform administrators govern organization-wide content without gaining access to private skill
bodies.

[简体中文](README.zh-CN.md)

## Choose a task

| I want to…                                     | Read                                                   |
| ---------------------------------------------- | ------------------------------------------------------ |
| Try SkillHive locally                          | [Getting started](getting-started.md)                  |
| Install with SQLite or containers              | [Installation](installation.md)                        |
| Create and version my own skill                | [Private skills](usage/private-skills.md)              |
| Start from a reusable template                 | [Templates](usage/templates.md)                        |
| Create or join a team                          | [Groups](usage/groups.md)                              |
| Make published skills available to a group     | [Group skills](usage/group-skills.md)                  |
| Administer the whole installation              | [Administration](usage/administration.md)              |
| Change security, database, or browser settings | [Configuration](configuration.md)                      |
| Prepare a shared or production instance        | [Deployment](deployment.md)                            |
| Fix a problem                                  | [Troubleshooting](troubleshooting.md) or [FAQ](faq.md) |

## Roles at a glance

- **User:** manages their own skills and personal templates, joins groups, and uses skills enabled

  for those groups.
- **Group member:** views the group's members, templates, and available skills.
- **Group administrator:** additionally manages ordinary members, settings, group templates, and

  enabled skills.
- **Group owner:** controls administrators, ownership transfer, and group dissolution.
- **Platform administrator:** manages account/group status, global templates and skills, grants,

  and audit records.

Permissions are enforced by the backend. Platform administrators cannot read another user's
private skill or personal-template content through the administrative API.

## Current release

Version `0.1.0` is intended for evaluation and community development. Review the
[known limitations](faq.md) and
[production checklist](deployment.md) before operating it on a public
network.
