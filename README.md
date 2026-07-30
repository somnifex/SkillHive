<p align="center">
  <img src="frontend/public/brand/skillhive-logo.png" width="144" alt="SkillHive logo" />
</p>

<h1 align="center">SkillHive</h1>

<p align="center">
  A self-hosted workspace for creating, versioning, and sharing AI skills with clear permissions.
</p>

<p align="center">
  <a href="https://github.com/somnifex/SkillHive/actions/workflows/ci.yml"><img alt="CI status" src="https://github.com/somnifex/SkillHive/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-2f81f7.svg"></a>
  <a href="makewiki/README.zh-CN.md">简体中文</a>
</p>

SkillHive gives individuals and teams one place to maintain private skills, reusable templates,
group access, published organization-wide skills, and an audit trail. It ships with a browser UI,
a versioned REST API, and SQLite as the zero-configuration default.

## Why SkillHive?

- **Start quickly.** Every account receives an OpenAI-style starter template for creating a
  structured `SKILL.md`.
- **Keep ownership clear.** Private skills and personal templates remain visible only to their
  owner.
- **Collaborate safely.** Group owners and administrators control membership, roles, templates,
  and enabled skills.
- **Publish deliberately.** Platform administrators can version, publish, disable, archive, and
  grant global skills.
- **Run it your way.** Use SQLite locally or connect PostgreSQL/MySQL for a shared deployment.
- **Trace important changes.** Administrative and permission-sensitive actions are recorded in
  the audit log.

## Quick start

Prerequisites: Python 3.12, [uv 0.11.28](https://docs.astral.sh/uv/), Node.js 24, pnpm 11.9,
and PowerShell.

```powershell
.\scripts\setup.ps1
.\scripts\dev.ps1
```

Open <http://127.0.0.1:5173>. The setup script creates `.env`, installs locked dependencies,
migrates the default SQLite database, and loads development seed data.

> Seed accounts and the example JWT secret are for local evaluation only. Replace them before
> exposing SkillHive to a network.

For separate processes, Docker, other databases, and production preparation, follow the
[installation guide](makewiki/installation.md) and [deployment guide](makewiki/deployment.md).

## Documentation

| Need | English | 简体中文 |
|---|---|---|
| Start here | [User guide](makewiki/README.md) | [用户手册](makewiki/README.zh-CN.md) |
| Install | [Installation](makewiki/installation.md) | [安装](makewiki/installation.zh-CN.md) |
| Configure | [Configuration](makewiki/configuration.md) | [配置](makewiki/configuration.zh-CN.md) |
| Deploy | [Deployment](makewiki/deployment.md) | [部署](makewiki/deployment.zh-CN.md) |
| Solve a problem | [Troubleshooting](makewiki/troubleshooting.md) | [故障排查](makewiki/troubleshooting.zh-CN.md) |
| Integrate or contribute | [Developer reference](docs/index.md) | [开发者参考](docs/index.md) |

Interactive API documentation is available at `/docs` while the backend is running.

## Project status

SkillHive is at `0.1.0`: suitable for local evaluation and community development. Before a
production deployment, review the [security policy](SECURITY.md) and the hardening checklist in
the [deployment guide](makewiki/deployment.md). Password-reset email delivery and invite-link
joining are not yet implemented.

## Contributing

Issues, focused pull requests, documentation improvements, and test coverage are welcome. Read
[CONTRIBUTING.md](CONTRIBUTING.md) and our [Code of Conduct](CODE_OF_CONDUCT.md) before taking
part. Security reports must follow [SECURITY.md](SECURITY.md), not a public issue.

Use [GitHub Issues](https://github.com/somnifex/SkillHive/issues) for reproducible bugs and focused
feature requests.

## License

SkillHive is available under the [MIT License](LICENSE).
