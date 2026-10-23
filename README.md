# TunnelHub

自托管内网服务统一访问与转发管理平台。

TunnelHub 计划通过公网服务端与内网 Agent 建立主动连接，让多个内网 HTTP 服务共享统一的公网访问入口。项目不负责安装、配置、启动、停止或升级 Caddy、FRP 等外部基础设施。

## 当前状态

当前版本实现了服务配置管理 API、SQLite 持久化、基础 Web 管理界面，以及 Agent 的 WebSocket 握手、共享令牌校验、心跳和断线重连。**HTTP 请求经隧道转发、按域名路由、管理员登录和服务访问认证尚未实现。** 管理 API 目前没有管理员认证，请勿将当前版本直接暴露到公网或用于生产环境。

## 已实现功能

- 服务配置新增、查询、编辑、删除和启停。
- 校验访问主机名、目标 URL 和重复域名。
- SQLite 持久化，默认数据目录为 `$XDG_DATA_HOME/tunnelhub/`，否则为 `$HOME/.local/share/tunnelhub/`。
- React + TypeScript 管理界面，包含搜索、状态切换和配置编辑。

## 技术栈

- 后端：Rust、Axum、Tokio、SQLx
- 数据库：SQLite
- 前端：React、TypeScript、Vite
- Agent：Rust、WebSocket（握手与心跳已实现，HTTP 转发开发中）

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

### Agent 通信验证（开发阶段）

服务端和 Agent 必须设置相同的共享令牌，且至少 24 个字符：

```bash
export TUNNELHUB_AGENT_TOKEN='请替换为长度至少24位的随机令牌'
# 终端一：启动 Server
make dev-server
# 终端二：启动 Agent
TUNNELHUB_SERVER_URL=http://127.0.0.1:8080 TUNNELHUB_AGENT_ID=home-node make dev-agent
```

Agent 通过 `/api/agent/connect` 建立 WebSocket 连接，每 15 秒发送心跳；连接断开后采用指数退避重连。当前是开发阶段的共享令牌认证，不具备逐节点凭证管理，也尚未传输实际 HTTP 请求。生产部署前必须使用 HTTPS/WSS，并补齐管理员认证、逐节点授权和目标地址访问限制。

## 架构边界

- TunnelHub 管理自身的服务配置、节点和请求路由。
- Caddy、FRP 等底层网络组件由部署者自行管理，平台不会自动更改其配置或生命周期。
- 内网 Agent 应主动连接公网服务端；不要求家庭网络拥有公网 IP。

详细设计见 [架构说明](docs/architecture.md)。
