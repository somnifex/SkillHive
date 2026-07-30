# Frequently asked questions

## Is SkillHive an execution engine?

No. SkillHive stores, versions, scopes, and governs skill instructions. It does not execute an AI
agent or call a model on behalf of the user.

## Who can read my private skill?

Only its owner through the normal application API. Platform administration does not include an
endpoint for reading another user's private skill body. Database operators and anyone with backup
access still control the underlying data and must protect it accordingly.

## Does a group template create a shared skill?

No. Instantiating any template creates a private skill owned by the current user. Group scope
controls who can use the template, not who owns the result.

## What is the difference between a template and a global skill?

A template generates a new private skill that the user can customize. A global skill is centrally
versioned and published; group management enables it for members using latest or locked behavior.

## Can I use SQLite in production?

SQLite is best for a single local process or small controlled evaluation. A shared installation
usually benefits from PostgreSQL, connection management, external backups, and operational
monitoring. Choose based on concurrency and recovery requirements rather than project size alone.

## Why do I see demo users?

Seed data was enabled. It is idempotent and intended for development. Set `SEED_DEMO=false` in
container deployments and remove or rotate those credentials before sharing the instance.

## What is not implemented yet?

- Forgot-password requests do not send email or reset credentials.
- Invite-link joining is reserved but has no complete workflow.
- There is no one-click export of a complete skill package.
- Built-in multi-replica/shared-state login lockout is not available.
- The supplied deployment does not provision domains, TLS, monitoring, or managed backups.

## Where is the API reference?

Run the backend and open `/docs` for interactive OpenAPI documentation, or read the
[maintained endpoint guide](../docs/api.md).

## How is the project licensed?

Source and documentation are licensed under the [MIT License](../LICENSE).
