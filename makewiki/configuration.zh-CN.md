# 配置

SkillHive 从环境变量和根目录 `.env` 读取设置。可以复制 `.env.example` 作为起点，但不要
把生成的 `.env` 提交到版本库。

## 应用与数据库

| 变量              | 默认值                             | 作用               |
| --------------- | ------------------------------- | ---------------- |
| `APP_NAME`      | `SkillHive`                     | API 中显示的服务名称     |
| `ENVIRONMENT`   | `development`                   | 环境标识             |
| `DEBUG`         | 代码默认 `false`                    | 调试行为；生产环境保持关闭    |
| `API_V1_PREFIX` | `/api/v1`                       | v1 接口前缀          |
| `DATABASE_URL`  | `sqlite:///./data/skillhive.db` | SQLAlchemy 数据库地址 |
| `SEED_DEMO`     | Compose 中为 `true`               | 容器启动时载入开发账号      |

PostgreSQL 示例：

```env
DATABASE_URL=postgresql+psycopg://skillhive:password@localhost:5432/skillhive
```

MySQL 示例：

```env
DATABASE_URL=mysql+pymysql://skillhive:password@localhost:3306/skillhive
```

修改数据库地址后，应先执行迁移：

```powershell
.venv\Scripts\uv.exe run alembic upgrade head
```

## 登录与 Cookie

| 变量                            | 默认值     | 生产建议              |
| ----------------------------- | ------- | ----------------- |
| `JWT_SECRET_KEY`              | 开发示例值   | 换成高熵随机密钥          |
| `JWT_ALGORITHM`               | `HS256` | 除非同步迁移令牌，否则不要修改   |
| `ACCESS_TOKEN_EXPIRE_MINUTES` | `15`    | 保持较短              |
| `REFRESH_TOKEN_EXPIRE_DAYS`   | `7`     | 与组织会话策略一致         |
| `COOKIE_SECURE`               | `false` | HTTPS 环境设为 `true` |
| `LOGIN_MAX_ATTEMPTS`          | `5`     | 临时锁定前的失败次数        |
| `LOGIN_LOCKOUT_MINUTES`       | `15`    | 锁定时间              |

Access Token 只保存在浏览器内存，Refresh Token 使用 HttpOnly Cookie，并对应数据库中的
可撤销会话。登录锁定计数保存在单个后端进程内，重启后会清空。

## 浏览器来源

`CORS_ORIGINS` 使用英文逗号分隔精确可信的前端来源：

```env
CORS_ORIGINS=http://localhost:5173,http://127.0.0.1:5173
```

部署后应替换成实际 HTTPS 来源。使用凭据 Cookie 时，不要配置宽泛的通配来源。

## 让配置生效

环境变量改变后重启后端；影响前端构建的设置需要重新构建前端。先检查
`/api/v1/health`，再从实际浏览器来源验证登录和令牌刷新。
