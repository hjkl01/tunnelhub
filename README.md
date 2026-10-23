# TunnelHub

基于 FRP 的内网服务统一访问入口。FRP 负责公网与内网之间的隧道，TunnelHub 负责按 Host 域名把 HTTP 请求反向代理到宿主机或局域网内的目标服务。

TunnelHub 不实现隧道协议，也不安装、配置、启停或升级 FRP、Caddy。

## 请求链路

```text
公网用户
  -> 你自行维护的 FRP 入口
  -> TunnelHub 单容器端口（默认宿主机 8088）
  -> 按 Host 查询 SQLite 服务配置
  -> 反向代理到目标 URL
  -> 目标服务响应
```

FRP 应将服务域名的 HTTP 请求转发到 TunnelHub 的统一入口，并保留原始 Host 请求头。TLS 可以在你现有的公网入口终止；TunnelHub 容器入口当前接收 HTTP。TunnelHub 不会修改 FRP、DNS 或 Caddy 配置。

## Docker 部署

要求已安装 Docker Engine 和 Docker Compose v2。

```bash
git clone https://github.com/hjkl01/tunnelhub.git
cd tunnelhub
docker compose up -d --build
```

从旧版“server + frontend”双容器升级时，先停止并移除旧容器，保留数据库卷，再构建新版本：

```bash
docker compose down --remove-orphans
docker compose up -d --build
```

不要添加 `-v`，否则会删除保存用户、权限和服务配置的命名卷。

默认管理界面地址为 http://localhost:8088，默认管理域名是 localhost。首次打开时创建管理员账户、设置密码并绑定 TOTP；后续登录必须同时提供密码和动态验证码；无法使用验证器时，可用一次性恢复码登录。**管理域名仍建议保持本机访问，不要加入 FRP 公网映射**，并可在“用户与权限”页面为业务服务配置访问策略。FRP 只需把需要公开的业务服务域名转发到宿主机的 8088 端口。

## 管理员登录与 TOTP

首次访问管理界面时，按页面提示创建唯一管理员账户（密码至少 12 个字符），然后用 Aegis、Google Authenticator 等兼容应用扫描二维码或手动输入密钥，并提交 6 位验证码确认绑定。成功后页面会展示 8 个一次性恢复码，请立即保存；每个恢复码只能使用一次。

登录需要用户名、密码和 TOTP 验证码，也可用一个未使用的恢复码替代 TOTP 验证码。会话使用 HttpOnly、SameSite=Strict Cookie，默认 8 小时过期；登录失败次数按用户名限流。管理员密码使用 Argon2id 哈希存储，恢复码只保存哈希。默认启用 `TUNNELHUB_COOKIE_SECURE=true`，推荐通过 HTTPS 访问管理界面。若仅使用纯 HTTP，且浏览器未在 localhost 场景发送 Secure Cookie，才在 `.env` 中显式设置 `TUNNELHUB_COOKIE_SECURE=false`。业务访问会话也遵循同一设置。

前端由 Vite 构建后嵌入 Rust 可执行文件，运行时不需要单独的 Nginx 或前端容器。Docker 使用 Node 与 Rust 多阶段构建，最终只运行一个 TunnelHub 容器。构建时默认使用 DaoCloud Docker Hub 镜像、npmmirror、rsproxy.cn 和清华 Debian 镜像，以减少国内环境下载依赖的耗时。若 Docker 基础镜像镜像站不可用，可改用官方 Docker Hub 地址重新构建：`docker compose build --build-arg DOCKER_MIRROR=docker.io`，然后执行 `docker compose up -d`。认证表和服务配置保存在 Docker 命名卷 `tunnelhub_data` 中。请备份该卷；若丢失 TOTP 设备，可使用一次性恢复码登录。当前管理后台仍是单管理员模型；业务用户由管理员在“用户与权限”页面创建，使用用户名和密码登录受限服务。业务用户目前不支持自行注册或 TOTP 绑定。

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

- 管理界面：http://localhost:5173
- API：http://127.0.0.1:8080
- 健康检查：GET /api/health

本地开发时，Vite 代理管理 API 到 Rust 后端；运行构建版时，Rust 会直接提供嵌入的前端资源。可通过 TUNNELHUB_LISTEN 设置监听地址、TUNNELHUB_UI_HOST 设置管理界面允许使用的 Host、TUNNELHUB_DATABASE_URL 指定 SQLite 文件。

## 当前限制

- 已实现服务配置 CRUD 和按 Host 匹配的 HTTP 反向代理，支持流式请求/响应体。
- 管理后台支持管理员密码 + TOTP、一次性恢复码和会话校验；业务访问支持用户、用户组和按服务授权。
- 业务用户通过受限服务自己的域名登录，访问会话按主机名隔离；公开服务不要求登录。
- 业务用户目前使用密码登录，尚未支持业务用户 TOTP、细粒度读写权限或访问日志。
- 当前代理不跟随上游 HTTP 重定向；若目标服务返回重定向，由客户端按 Location 处理。
- WebSocket 代理、自动 HTTPS 证书、自动 DNS/FRP 配置尚未实现。
