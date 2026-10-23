.DEFAULT_GOAL := help

.PHONY: help dev dev-server dev-frontend build build-server build-frontend fmt check clean

help: ## 显示可用命令
	@awk 'BEGIN {FS = ":.*##"; printf "用法: make <目标>\n\n可用目标:\n"} /^[a-zA-Z0-9_-]+:.*##/ {printf "  %-18s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

dev: build-frontend ## 同时启动后端和前端开发服务器
	@set -m; cargo run -p tunnelhub-server & server_pid=$$!; (cd frontend && npm run dev) & frontend_pid=$$!; trap 'kill $$server_pid $$frontend_pid 2>/dev/null || true' INT TERM EXIT; wait

dev-server: build-frontend ## 仅启动后端开发服务器
	cargo run -p tunnelhub-server

dev-frontend: ## 仅启动前端开发服务器
	cd frontend && npm run dev

build: build-server ## 构建前端并生成内嵌静态资源的后端

build-server: build-frontend ## 构建包含前端资源的 Rust 后端 release 版本
	cargo build --release -p tunnelhub-server

build-frontend: ## 构建前端
	cd frontend && npm install && npm run build

fmt: ## 格式化 Rust 代码并检查前端 TypeScript
	cargo fmt --all
	cd frontend && npx tsc --noEmit

check: build-frontend ## 构建前端后检查 Rust 后端

clean: ## 清理构建产物和前端依赖
	cargo clean
	rm -rf frontend/dist frontend/node_modules
