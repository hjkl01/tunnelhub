.DEFAULT_GOAL := help

# Docker 构建使用的镜像源，可用 `make docker-build DOCKER_MIRROR=docker.io` 覆盖
DOCKER_MIRROR ?= docker.m.daocloud.io

.PHONY: help dev build fmt check clean \
	docker-build docker-up docker-down docker-restart docker-logs docker-ps docker-sh

help: ## 显示可用命令
	@awk 'BEGIN {FS = ":.*##"; printf "用法: make <目标>\n\n可用目标:\n"} /^[a-zA-Z0-9_-]+:.*##/ {printf "  %-18s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

dev: ## 同时启动后端和前端开发服务器
	cd frontend && npm install && npm run build
	@set -m; cargo run -p tunnelhub-server & server_pid=$$!; (cd frontend && npm run dev) & frontend_pid=$$!; trap 'kill $$server_pid $$frontend_pid 2>/dev/null || true' INT TERM EXIT; wait

build: ## 构建前后端，并把 release 二进制复制到当前目录
	cd frontend && npm install && npm run build
	cargo build --release -p tunnelhub-server
	cp target/release/tunnelhub-server ./tunnelhub-server
	@echo "已生成 ./tunnelhub-server"

fmt: ## 格式化 Rust 代码并检查前端 TypeScript
	cargo fmt --all
	cd frontend && npx tsc --noEmit

check: ## 构建前端后检查 Rust 后端
	cd frontend && npm install && npm run build
	cargo check --workspace

clean: ## 清理构建产物和前端依赖
	cargo clean
	rm -rf frontend/dist frontend/node_modules tunnelhub-server

docker-build: ## 构建 Docker 镜像
	docker compose build --build-arg DOCKER_MIRROR=$(DOCKER_MIRROR)

docker-up: docker-build ## 构建并后台启动容器
	docker compose up -d

docker-down: ## 停止并移除容器（保留数据）
	docker compose down --remove-orphans

docker-restart: ## 重启容器
	docker compose restart

docker-logs: ## 实时查看容器日志
	docker compose logs -f

docker-ps: ## 查看容器状态
	docker compose ps

docker-sh: ## 进入容器 Shell
	docker compose exec tunnelhub sh