# 部署

仓库内的 Compose 是可复现的评估基线，不是一套完整生产平台。正式运维仍需补充域名与
TLS、秘密管理、监控、备份和升级策略。

## 选择数据库

- **SQLite：**适合单进程本机体验。请备份 `data/skillhive.db`，不要放在不可靠的共享

  文件系统上。
- **PostgreSQL：**默认 Compose 选择，也是共享实例的建议起点。
- **MySQL：**通过 PyMySQL 支持；字符集应使用 `utf8mb4`。

## Compose 基线

```powershell
Copy-Item .env.example .env
docker compose up --build -d
docker compose ps
```

前端代理会把 `/api/` 转发到后端；后端在启动 Uvicorn 前执行 Alembic 迁移。接入流量前，
应检查 `/api/v1/health` 并完成一次登录。

`mysql` profile 提供了可选 MySQL 服务，但仍需同步修改后端数据库地址：

```powershell
docker compose --profile mysql up mysql
```

## 生产环境加固

向不可信用户开放前，至少完成：

1. 生成唯一的高熵 `JWT_SECRET_KEY`。
2. 设置 `SEED_DEMO=false`，删除或修改全部示例凭据。
3. 只通过 HTTPS 服务，并设置 `COOKIE_SECURE=true`。
4. 把 `CORS_ORIGINS` 限制到精确的 HTTPS 来源。
5. 使用低权限数据库账号、加密连接、持久化存储和经过验证的备份。
6. 保持 `DEBUG=false`，限制 API 和数据库网络入口。
7. 在反向代理限制请求大小、超时和访问频率。
8. 集中采集日志，但不记录密码、Token、Cookie 和私有 Skill 正文。
9. 监控健康状态、登录失败、管理操作、存储容量和备份结果。
10. 阅读[安全策略](../SECURITY.md)和[当前限制](faq.zh-CN.md)。

如果运行多个后端副本，应使用共享锁定状态或网关策略替代当前进程内登录锁定，之后才能
把它作为可靠的安全控制。

## 升级流程

1. 阅读 [CHANGELOG.md](../CHANGELOG.md) 并备份数据库。
2. 构建或拉取新版本镜像。
3. 对目标数据库执行一次 `alembic upgrade head`。
4. 启动新版前后端。
5. 验证健康检查、登录刷新、私有 Skill 读取和一个有权限的群组流程。
6. 确认检查和日志正常后再结束备份保留期。

迁移以向前升级为目标。应先在生产数据副本上演练恢复与升级，不要修改已经执行过的迁移。

## 备份

数据库与部署秘密应分开备份。数据库备份包含账号、私有 Skill、模板、会话和审计记录，
必须加密并严格限制访问。应定期实际恢复，而不是只确认“备份任务成功”。
