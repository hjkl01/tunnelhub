# TunnelHub

自托管内网服务统一访问与转发管理平台。

TunnelHub 通过公网控制端与内网 Agent 建立主动连接，让多个内网 HTTP 服务共享统一的公网访问入口。项目不负责安装、配置、启动、停止或升级 Caddy、FRP 等外部基础设施。

## 项目状态

项目处于早期开发阶段，当前优先实现服务端骨架、配置存储、管理 API 与内网 Agent 通信协议。生产环境部署前请勿将尚未完成安全审计的服务直接暴露到公网。

## 设计目标

- 使用 Web 后台管理内网服务和节点。
- 内网 Agent 主动连接公网服务，不要求家庭网络拥有公网 IP。
- 多个 HTTP 服务共用一个公网 HTTPS 入口。
- 支持动态路由、WebSocket、SSE 和流式传输。
- 使用 SQLite 保存平台配置与节点元数据。
- 不管理 Caddy、FRP 或其他外部代理组件。

## 技术栈

- 后端：Rust、Axum、Tokio
- 数据库：SQLite、SQLx
- 前端：React、TypeScript、Vite
- Agent：Rust

## 开发计划

1. 定义服务端、Agent 和隧道协议的项目结构。
2. 实现 SQLite 初始化、服务配置 CRUD 和健康检查 API。
3. 实现 Agent 注册、身份认证、心跳和断线重连。
4. 实现统一 HTTP 入口与基于节点隧道的动态转发。
5. 增加访问控制、审计日志和部署文档。

## 开发环境

需要 Rust stable、Node.js LTS、npm 和 SQLite 开发依赖。具体构建命令会随着首个可运行版本落地补充。
