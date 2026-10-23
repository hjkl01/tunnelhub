# 架构说明

## 组件边界

- **FRP**：由部署者自行部署和维护，负责公网与内网之间的穿透及流量映射。
- **TunnelHub Server**：服务配置 CRUD API，不作为 HTTP 反向代理，不处理 FRP 控制面。
- **Frontend**：管理服务名称、访问域名、目标 URL 和启用状态。
- **SQLite**：保存服务目录和平台元数据。
- **Docker**：以容器运行 TunnelHub；通过 host.docker.internal 访问宿主机服务。

TunnelHub 不安装、配置、启停或升级 FRP、Caddy，也不会自动更改 DNS、FRP 配置或宿主机防火墙。

## 访问模型

1. 管理员在 TunnelHub 中登记服务、域名和目标 URL。
2. 管理员独立配置 FRP（以及需要的 DNS、TLS 入口），让公网访问映射到相应内网服务。
3. 用户访问服务时，请求由 FRP 和部署者自行配置的入口组件处理；TunnelHub 不处于业务请求的数据路径中。

服务配置只是目录数据，不是自动生效的代理规则。TunnelHub 不负责创建隧道或转发业务流量。

## Docker 网络

容器访问宿主机服务时，使用 http://host.docker.internal:端口。Docker Compose 为 Linux 配置 host-gateway 别名。目标服务需要监听可从 Docker 网桥到达的接口；仅绑定宿主机 loopback 的服务通常不可从 bridge 网络容器访问。

## 安全边界

- 当前版本尚无管理员登录认证，不应将管理 UI/API 直接暴露到不可信网络。
- 目标 URL 仅作为配置数据保存，当前版本不会主动请求该 URL。
- FRP、Caddy、DNS、TLS 证书和宿主机防火墙均由部署者单独管理。
