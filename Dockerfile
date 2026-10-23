FROM rust:1-bookworm AS builder
WORKDIR /app
COPY Cargo.toml ./
COPY server ./server
RUN cargo build --release -p tunnelhub-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* && useradd --system --uid 10001 --create-home tunnelhub && mkdir -p /data && chown -R tunnelhub:tunnelhub /data
COPY --from=builder /app/target/release/tunnelhub-server /usr/local/bin/tunnelhub-server
ENV TUNNELHUB_LISTEN=0.0.0.0:8080
ENV TUNNELHUB_DATABASE_URL=sqlite:///data/tunnelhub.db
EXPOSE 8080
USER tunnelhub
ENTRYPOINT ["/usr/local/bin/tunnelhub-server"]
