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

FROM ${DOCKER_MIRROR}/library/caddy:2 AS caddy-binary

FROM ${DOCKER_MIRROR}/library/debian:bookworm-slim
# 复用 Rust 构建镜像中的 CA 证书，避免运行阶段依赖 APT 软件源
COPY --from=rust-builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN set -eux; \
    useradd --system --uid 10001 --create-home tunnelhub; \
    mkdir -p /data; \
    chown -R tunnelhub:tunnelhub /data
COPY --from=rust-builder /app/target/release/tunnelhub-server /usr/local/bin/tunnelhub-server
COPY --from=caddy-binary /usr/bin/caddy /usr/local/bin/caddy
ENV TUNNELHUB_LISTEN=127.0.0.1:8081
ENV TUNNELHUB_CADDY_ENABLED=true
ENV TUNNELHUB_CADDY_LISTEN=0.0.0.0:8080
ENV TUNNELHUB_CADDY_BIN=/usr/local/bin/caddy
ENV TUNNELHUB_CADDY_CONFIG=/data/Caddyfile
ENV TUNNELHUB_DATABASE_URL=sqlite:///data/tunnelhub.db
EXPOSE 8080
USER tunnelhub
ENTRYPOINT ["/usr/local/bin/tunnelhub-server"]
