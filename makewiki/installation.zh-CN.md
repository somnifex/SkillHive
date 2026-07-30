# 安装

## 环境要求

使用 Windows 本地流程需要：

- Windows PowerShell 5.1 或 PowerShell 7；
- Python 3.12；
- [uv](https://docs.astral.sh/uv/) 0.11.28；
- Node.js 24；
- pnpm 11.9。

容器流程需要安装支持 Compose 的 Docker。

## 使用 PowerShell 本地安装

在仓库根目录运行：

```powershell
.\scripts\setup.ps1
```

脚本会在需要时把 `.env.example` 复制为 `.env`，根据锁文件安装依赖，执行全部数据库迁移，
并向 SQLite 写入可重复执行的示例数据。

同时启动前后端：

```powershell
.\scripts\dev.ps1
```

也可以在两个终端分别运行：

```powershell
.\scripts\backend.ps1
```

```powershell
.\scripts\frontend.ps1
```

验证地址：

- 界面：<http://127.0.0.1:5173>
- API 健康检查：<http://127.0.0.1:8000/api/v1/health>
- OpenAPI：<http://127.0.0.1:8000/docs>

## 手动安装

```powershell
uv sync --frozen
Copy-Item .env.example .env
Set-Location frontend
pnpm install --frozen-lockfile
Set-Location ..
.\scripts\init-db.ps1
```

## 使用 Docker Compose

默认 Compose 使用 PostgreSQL：

```powershell
Copy-Item .env.example .env
docker compose up --build
```

界面端口为 `5173`，API 端口为 `8000`。后端入口会等待数据库、执行迁移，并在
`SEED_DEMO=true` 时加载示例数据。

停止容器但保留数据卷：

```powershell
docker compose down
```

## 示例账号

| 角色    | 用户名     | 密码          |
| ----- | ------- | ----------- |
| 平台管理员 | `admin` | `Admin123!` |
| 普通用户  | `howie` | `User123!`  |
| 普通用户  | `mei`   | `User123!`  |

这些账号只能用于本地体验。共享部署前必须关闭示例数据、修改或删除全部示例凭据。

接下来可阅读[配置](configuration.zh-CN.md)或[快速入门](getting-started.zh-CN.md)。
