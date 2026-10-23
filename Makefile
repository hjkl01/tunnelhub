.PHONY: dev dev-server dev-agent dev-frontend build build-server build-agent build-frontend fmt check clean

dev:
	@set -m; cargo run -p tunnelhub-server & server_pid=$$!; (cd frontend && npm run dev) & frontend_pid=$$!; trap 'kill $$server_pid $$frontend_pid 2>/dev/null || true' INT TERM EXIT; wait

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
