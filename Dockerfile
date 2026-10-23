# 国内镜像默认值；如镜像不可用，可在构建时传 --build-arg DOCKER_MIRROR=docker.io
ARG DOCKER_MIRROR=docker.m.daocloud.io

FROM ${DOCKER_MIRROR}/library/node:22-alpine AS frontend-builder
WORKDIR /app/frontend
# 使用国内 npm 镜像，减少前端依赖下载耗时
RUN npm config set registry https://registry.npmmirror.com
COPY frontend/package*.json ./
RUN npm install --no-audit --no-fund
COPY frontend/ ./
RUN npm run build

FROM ${DOCKER_MIRROR}/library/rust:1-bookworm AS rust-builder
WORKDIR /app
# 使用 rsproxy.cn 的 crates.io 镜像下载 Rust 依赖
RUN mkdir -p /usr/local/cargo && printf '[source.crates-io]\nreplace-with = "rsproxy-sparse"\n\n[source.rsproxy-sparse]\nregistry = "sparse+https://rsproxy.cn/index/"\n' > /usr/local/cargo/config.toml
COPY Cargo.toml ./
COPY server ./server
COPY --from=frontend-builder /app/frontend/dist ./frontend/dist
RUN cargo build --release -p tunnelhub-server

FROM ${DOCKER_MIRROR}/library/debian:bookworm-slim
# Debian 软件源切换到清华镜像；同时兼容传统 sources.list 与 deb822 配置
RUN set -eux; \
    if [ -f /etc/apt/sources.list.d/debian.sources ]; then \
      sed -i 's|https\?://deb.debian.org/debian|https://mirrors.tuna.tsinghua.edu.cn/debian|g; s|https\?://security.debian.org/debian-security|https://mirrors.tuna.tsinghua.edu.cn/debian-security|g' /etc/apt/sources.list.d/debian.sources; \
    fi; \
    if [ -f /etc/apt/sources.list ]; then \
      sed -i 's|https\?://deb.debian.org/debian|https://mirrors.tuna.tsinghua.edu.cn/debian|g; s|https\?://security.debian.org/debian-security|https://mirrors.tuna.tsinghua.edu.cn/debian-security|g' /etc/apt/sources.list; \
    fi; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates; \
    rm -rf /var/lib/apt/lists/*; \
    useradd --system --uid 10001 --create-home tunnelhub; \
    mkdir -p /data; \
    chown -R tunnelhub:tunnelhub /data
COPY --from=rust-builder /app/target/release/tunnelhub-server /usr/local/bin/tunnelhub-server
ENV TUNNELHUB_LISTEN=0.0.0.0:8080
ENV TUNNELHUB_DATABASE_URL=sqlite:///data/tunnelhub.db
ENV TUNNELHUB_UI_HOST=localhost
EXPOSE 8080
USER tunnelhub
ENTRYPOINT ["/usr/local/bin/tunnelhub-server"]
