.PHONY: build check run dev test clean

PORT ?= 8000
TARGET ?=

## Build the full project (frontend + backend)
build:
	cargo build

## Release build
release:
	cargo build --release

## Type-check without producing a binary
check:
	cargo check

## Start the server (PORT=8000 by default)
##   make run
##   make run PORT=9000
##   make run PORT=9000 TARGET=192.168.0.2:8000
run:
	cargo run -- serve --port $(PORT) $(if $(TARGET),--target $(TARGET),)

## Start the frontend dev server (hot reload, proxies to Rust backend)
dev:
	cd frontend && bun dev

## Run integration tests
test:
	cd frontend && bun run test

## Remove build artifacts
clean:
	cargo clean
