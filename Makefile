.PHONY: dev build build-server build-agent build-frontend fmt check clean

dev:
	@echo "启动后端与前端开发服务。需要分别在终端执行 make dev-server 和 make dev-frontend。"

dev-server:
	cargo run -p tunnelhub-server

dev-agent:
	cargo run -p tunnelhub-agent

dev-frontend:
	cd frontend && npm run dev

build: build-server build-agent build-frontend

build-server:
	cargo build --release -p tunnelhub-server

build-agent:
	cargo build --release -p tunnelhub-agent

build-frontend:
	cd frontend && npm install && npm run build

fmt:
	cargo fmt --all
	cd frontend && npx tsc --noEmit

check:
	cargo check --workspace
	cd frontend && npm install && npm run build

clean:
	cargo clean
	rm -rf frontend/dist frontend/node_modules
