# TunnelHub

基于 FRP 的内网服务统一访问入口。FRP 负责公网与内网之间的隧道，TunnelHub 负责按 Host 域名把 HTTP 请求反向代理到宿主机或局域网内的目标服务。

TunnelHub 不实现隧道协议，也不安装、配置、启停或升级 FRP、Caddy。

## 请求链路

```text
公网用户
  -> 你自行维护的 FRP 入口
  -> TunnelHub 前端容器端口（默认宿主机 8088）
  -> 按 Host 查询 SQLite 服务配置
  -> 反向代理到目标 URL
  -> 目标服务响应
```

FRP 应将服务域名的 HTTP 请求转发到 TunnelHub 的前端入口，并保留原始 Host 请求头。TLS 可以在你现有的公网入口终止；TunnelHub 容器入口当前接收 HTTP。TunnelHub 不会修改 FRP、DNS 或 Caddy 配置。

## Docker 部署

要求已安装 Docker Engine 和 Docker Compose v2。

```bash
git clone https://github.com/hjkl01/tunnelhub.git
cd tunnelhub
docker compose up -d --build
```

默认管理界面地址为 http://localhost:8088，默认管理域名是 localhost。管理界面/API 目前没有用户登录认证，因此**不要将管理域名加入 FRP 公网映射**。FRP 只需把你希望公开的业务服务域名转发到宿主机的 8088 端口。这样管理界面保持宿主机本地访问，业务域名进入反向代理。若需要从其他设备管理，可使用 SSH 本地端口转发，或先为管理界面增加认证后再公开。

## 添加服务

在管理界面填写服务名称、访问域名和目标 URL。目标 URL 是 TunnelHub 容器需要访问的地址，例如：

- 宿主机上的 Web 应用：http://host.docker.internal:3000
- 宿主机上的 Home Assistant：http://host.docker.internal:8123
- 局域网另一台机器：http://192.168.1.20:8080

保存后，访问域名必须与请求中的 Host 一致。你还需要自行配置 DNS 和 FRP，让该域名的 HTTP 流量进入 TunnelHub 入口。服务启用状态控制 TunnelHub 是否接受该域名的代理请求；它不会修改 FRP 配置。

Compose 已为 Linux Docker 配置 host.docker.internal:host-gateway。目标服务必须监听可从 Docker 网桥到达的接口；只监听宿主机 127.0.0.1 的服务通常无法直接从 bridge 网络容器访问。

## 本地开发

需要 Rust stable、Node.js LTS、npm。

```bash
make dev
make check
make build
```

- 管理界面：http://127.0.0.1:5173
- API：http://127.0.0.1:8080
- 健康检查：GET /api/health

本地开发时，Vite 会为管理 API 请求添加内部标记。可通过 TUNNELHUB_LISTEN 设置 API 监听地址，通过 TUNNELHUB_DATABASE_URL 指定 SQLite 文件。

## 当前限制

- 已实现服务配置 CRUD 和按 Host 匹配的 HTTP 反向代理，支持流式请求/响应体。
- 管理员认证、访问日志和 FRP 自动化集成未实现。
- 当前代理不跟随上游 HTTP 重定向；若目标服务返回重定向，由客户端按 Location 处理。
- WebSocket 代理、自动 HTTPS 证书、自动 DNS/FRP 配置尚未实现。
