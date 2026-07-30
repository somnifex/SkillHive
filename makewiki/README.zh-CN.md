# SkillHive 用户手册

SkillHive 是一个可自行部署的 AI Skill 工作台：个人可以沉淀私有 Skill，团队可以通过群组
协作，平台管理员可以治理全局内容，同时不能借助管理接口读取其他用户的私有 Skill 正文。

[English](README.md)

## 按任务查找文档

| 我想要……           | 请阅读                                                     |
| --------------- | ------------------------------------------------------- |
| 在本机快速体验         | [快速入门](getting-started.zh-CN.md)                        |
| 使用 SQLite 或容器安装 | [安装](installation.zh-CN.md)                             |
| 创建并维护自己的 Skill  | [私有 Skill](usage/private-skills.zh-CN.md)               |
| 从可复用模板开始创建      | [模板](usage/templates.zh-CN.md)                          |
| 创建或加入团队         | [群组](usage/groups.zh-CN.md)                             |
| 为群组启用已发布 Skill  | [群组 Skill](usage/group-skills.zh-CN.md)                 |
| 管理整个平台          | [平台管理](usage/administration.zh-CN.md)                   |
| 调整安全、数据库或浏览器配置  | [配置](configuration.zh-CN.md)                            |
| 准备共享或生产环境       | [部署](deployment.zh-CN.md)                               |
| 解决使用问题          | [故障排查](troubleshooting.zh-CN.md) 或 [常见问题](faq.zh-CN.md) |

## 角色速览

- **普通用户：**管理自己的 Skill 和个人模板，加入群组，使用群组已启用的 Skill。
- **群组成员：**查看群组成员、群组模板以及可用 Skill。
- **群组管理员：**还可以管理普通成员、群组设置、群组模板和已启用 Skill。
- **群组所有者：**负责管理员任免、转移所有权和解散群组。
- **平台管理员：**管理账号/群组状态、全局模板、全局 Skill、授权和审计记录。

权限由后端强制执行。平台管理员也不能通过管理 API 读取其他用户的私有 Skill 或个人模板
正文。

## 当前版本

`0.1.0` 适合本地评估和社区开发。部署到公网前，请先查看
[当前限制](faq.zh-CN.md)和
[生产加固清单](deployment.zh-CN.md)。
