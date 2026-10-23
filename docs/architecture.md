# 架构说明

## 组件边界

- **FRP**：由部署者自行部署和维护，负责公网与内网之间的隧道和流量送达。
- **TunnelHub Server**：管理服务配置；根据请求 Host 查询启用的服务，并将 HTTP 请求反向代理到目标 URL。
- **Frontend / Nginx**：在管理域名提供静态管理界面与 API 入口；其他 Host 转发到 Server 的动态服务路由。
- **SQLite**：保存服务名称、Host、目标 URL 和启用状态。
- **Docker**：容器化运行；通过 host.docker.internal 访问宿主机服务。

TunnelHub 不安装、配置、启停或升级 FRP、Caddy，也不会自动更改 DNS、FRP 配置或宿主机防火墙。

## 请求链路

1. 管理员在 TunnelHub 中登记服务域名和目标 URL。
2. 管理员自行配置 DNS 与 FRP，使相应域名的 HTTP 请求进入 TunnelHub 前端入口，并保留原始 Host。
3. Nginx 根据管理域名提供 UI；其他 Host 的请求转发给 TunnelHub Server。
4. Server 根据 Host 查找启用的服务配置，将请求路径、查询参数、方法、请求头和流式请求体转发到目标 URL。
5. Server 将上游状态码、响应头和流式响应体返回给客户端。

TunnelHub 只处理 HTTP 代理流量，不建立隧道，不控制 FRP。当前不支持 WebSocket 上游代理和自动 TLS。

## Docker 网络

宿主机服务目标地址使用 http://host.docker.internal:端口。Docker Compose 为 Linux 配置 host-gateway 别名。目标服务需要监听可从 Docker 网桥到达的接口；仅绑定宿主机 loopback 的服务通常不可从 bridge 网络容器访问。

## 安全边界

- 管理域名 API 通过 Nginx 注入内部标记，以避免服务域名直接访问管理 API；这不是管理员身份认证。
- 管理员登录认证尚未实现，管理界面应限制在可信网络中。
- 目标 URL 可访问宿主机和局域网服务，因此仅允许可信管理员管理服务配置，避免滥用为 SSRF 或内网探测入口。
- FRP、Caddy、DNS、TLS 证书和宿主机防火墙均由部署者单独管理。
