# TunnelHub

自托管内网服务统一访问与转发管理平台。

TunnelHub 计划通过公网服务端与内网 Agent 建立主动连接，让多个内网 HTTP 服务共享统一的公网访问入口。项目不负责安装、配置、启动、停止或升级 Caddy、FRP 等外部基础设施。

## 当前状态

当前版本实现了服务配置管理 API、SQLite 持久化和基础 Web 管理界面。**Agent 隧道、访问域名的实际请求转发、管理员登录和服务访问认证尚未实现。** 请勿将当前版本的管理 API 直接暴露到公网或用于生产环境。

## 已实现功能

- 服务配置新增、查询、编辑、删除和启停。
- 校验访问主机名、目标 URL 和重复域名。
- SQLite 持久化，默认数据目录为 `$XDG_DATA_HOME/tunnelhub/`，否则为 `$HOME/.local/share/tunnelhub/`。
- React + TypeScript 管理界面，包含搜索、状态切换和配置编辑。

## 技术栈

- 后端：Rust、Axum、Tokio、SQLx
- 数据库：SQLite
- 前端：React、TypeScript、Vite
- Agent：Rust（通信功能开发中）

## 开发环境

需要 Rust stable、Node.js LTS、npm。

```bash
# 同时启动后端和前端开发服务器
make dev

# 或分别启动
make dev-server
make dev-frontend

# 检查 Rust workspace 并构建前端
make check

# 构建 release 二进制和前端静态资源
make build
```

默认地址：

- 后端 API：`http://127.0.0.1:8080`
- 前端开发服务器：Vite 启动时打印的地址，通常为 `http://127.0.0.1:5173`
- 健康检查：`GET /api/health`
- 服务列表：`GET /api/services/`

可以通过 `TUNNELHUB_LISTEN` 设置后端监听地址，通过 `TUNNELHUB_DATABASE_URL` 指定 SQLite 文件。

## 架构边界

- TunnelHub 管理自身的服务配置、节点和请求路由。
- Caddy、FRP 等底层网络组件由部署者自行管理，平台不会自动更改其配置或生命周期。
- 内网 Agent 应主动连接公网服务端；不要求家庭网络拥有公网 IP。

详细设计见 [架构说明](docs/architecture.md)。
