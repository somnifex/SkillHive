# Security policy

## Supported versions

SkillHive is in its initial development phase. Security fixes are applied to the latest `0.1.x`
release and the current `main` branch.

| Version                   | Supported |
| ------------------------- | --------- |
| 0.1.x                     | Yes       |
| Older or unreleased forks | No        |

## Reporting a vulnerability

Do not disclose an unpatched vulnerability in a public issue, discussion, pull request, or chat.
Use GitHub's
[private vulnerability reporting form](https://github.com/somnifex/SkillHive/security/advisories/new).
If private vulnerability reporting has not been enabled, contact a repository maintainer through
a private channel listed on the repository profile.

Include:

- the affected version or commit;
- prerequisites and minimal reproduction steps;
- expected and observed behavior;
- the potential confidentiality, integrity, or availability impact;
- known mitigations, if any.

Do not include live credentials, access or refresh tokens, private skill content, personal data,
or a production database. Use synthetic examples.

Maintainers will acknowledge a complete report when it is reviewed, coordinate validation and a
fix, and credit reporters who want attribution. Timelines depend on severity and maintainer
availability. Please allow a reasonable remediation window before public disclosure.

## Deployment responsibility

The repository defaults are designed for local evaluation. Operators are responsible for at
least the following before exposing SkillHive:

- replace the example `JWT_SECRET_KEY` with a high-entropy secret;
- remove or rotate all seed-account passwords and set `SEED_DEMO=false`;
- terminate TLS and set `COOKIE_SECURE=true`;
- restrict `CORS_ORIGINS` to exact trusted origins;
- use a dedicated least-privilege database account and encrypted connections;
- protect backups, logs, and environment secrets;
- monitor audit events and keep dependencies and the base images updated.

See the full [deployment hardening checklist](makewiki/deployment.md).

## Current security limitations

Login lockout state is held in process memory and resets when the backend restarts. The
forgot-password endpoint does not yet deliver email or change a password. These behaviors should
be considered when planning an internet-facing deployment.
