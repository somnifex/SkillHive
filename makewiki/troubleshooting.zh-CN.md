# 故障排查

## 界面无法连接 API

1. 打开 <http://127.0.0.1:8000/api/v1/health>。
2. 确认后端进程正在监听 `8000`。
3. 检查浏览器来源是否精确出现在 `CORS_ORIGINS`。
4. 使用容器时查看 `docker compose ps` 和后端日志。
5. 修改构建期路由或代理设置后重新构建前端。

如果 Cookie 或 CORS 只配置了其中一个，请不要混用 `localhost` 和 `127.0.0.1`。

## 登录成功后立即失效

检查系统时间、各后端进程的 `JWT_SECRET_KEY` 是否一致、Cookie 域名/路径，以及浏览器是否
接受 Refresh Cookie。HTTPS 环境设置 `COOKIE_SECURE=true`；普通本地 HTTP 保持 `false`。

## 账号被临时锁定

等待 `LOGIN_LOCKOUT_MINUTES`（默认 15 分钟），并核对凭据。重启后端会清空当前内存计数，
但这只能用于开发诊断，不应作为生产恢复流程。

## 数据库迁移失败

确认 `DATABASE_URL` 指向可连接数据库，账号有权创建/修改应用 Schema，然后运行：

```powershell
.venv\Scripts\uv.exe run alembic current
.venv\Scripts\uv.exe run alembic upgrade head
```

不要通过删除迁移记录或修改已执行迁移来强行通过。先备份数据库，再调查第一个失败版本。

## SQLite 提示数据库锁定

停止重复后端进程和占用文件的工具。SQLite 不适合大量并发写入；共享实例应迁移到
PostgreSQL，而不是不断延长超时。

## 找不到模板或群组 Skill

模板问题先检查范围和当前群组成员/角色。群组 Skill 先确认平台管理员已经发布并授权，再
确认群组管理者已经启用；停用或归档的全局 Skill 不可用。

## PowerShell 阻止脚本运行

先审查脚本，再在组织允许执行本地脚本的 PowerShell 会话中运行。优先采用获得许可的
进程级策略，不要直接放宽整台机器的策略。

## 本地依赖与 CI 不一致

使用锁定命令：`uv sync --frozen` 和 `pnpm install --frozen-lockfile`。确认 Python 3.12、
Node.js 24、pnpm 11.9、uv 0.11.28。

## 仍需协助

运行[质量检查](../docs/testing.md)，记录准确的失败命令和版本，移除秘密与私有内容后，
使用仓库提供的 Bug 模板提交问题。
