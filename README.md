# TunnelHub

自托管服务目录与访问入口管理界面。TunnelHub 不实现内网穿透，也不安装、配置、启停或升级 FRP、Caddy。

## 设计边界

- **FRP**：由部署者独立维护，负责公网与内网之间的网络穿透和端口/域名映射。
- **TunnelHub**：管理服务名称、访问域名、目标 URL 和启停状态；不自动修改 FRP、Caddy、DNS 或宿主机网络配置。
- **Docker**：TunnelHub 以容器方式运行，并通过 host.docker.internal 访问宿主机服务。
- **SQLite**：存储服务配置，数据持久化到宿主机目录。

服务配置中的目标地址是给管理员记录和查看的，不意味着 TunnelHub 会代理该流量。公网访问能否成功，取决于你单独配置的 FRP、DNS 和其他入口组件。

## Docker 部署

要求已安装 Docker Engine 和 Docker Compose v2。

```bash
git clone https://github.com/hjkl01/tunnelhub.git
cd tunnelhub
docker compose up -d --build
```

打开 http://localhost:8088。

默认 SQLite 数据保存在当前目录的 ./data/。可在 .env 中设置 TUNNELHUB_PORT，修改宿主机映射端口。

### 从容器访问宿主机服务

添加服务时，目标 URL 使用容器能够解析和访问的地址。例如：

- 宿主机上的 Web 应用：http://host.docker.internal:3000
- 宿主机上的 Home Assistant：http://host.docker.internal:8123
- 局域网另一台机器：http://192.168.1.20:8080

Compose 已配置 Linux Docker 的 host.docker.internal:host-gateway 映射。请确认宿主机服务监听的地址允许 Docker 网桥访问；只监听宿主机 127.0.0.1 的服务通常无法直接从容器访问，应调整该服务的监听地址或采用适当的宿主机网络方案。

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

可以通过 TUNNELHUB_LISTEN 设置 API 监听地址，通过 TUNNELHUB_DATABASE_URL 指定 SQLite 文件。

## 当前限制

当前提供服务配置 CRUD 和基础管理界面。管理员认证、访问日志和与 FRP 的自动化集成均未实现；TunnelHub 不会替你配置或验证 FRP 的实际转发。不要将未认证的管理界面直接暴露到不可信公网。
