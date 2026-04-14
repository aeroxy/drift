.PHONY: build check run dev test clean bump-patch bump-minor bump-major

PORT ?= 8000
TARGET ?=

## Build the full project (frontend + backend, debug)
build:
	cd frontend && bun run build
	cargo build

## Release build (frontend + backend) — use this before manual testing
release:
	cd frontend && bun run build
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

## Bump the patch version (0.1.3 → 0.1.4) and update frontend version badge
bump-patch:
	@old=$$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	major=$$(echo $$old | cut -d. -f1); \
	minor=$$(echo $$old | cut -d. -f2); \
	patch=$$(echo $$old | cut -d. -f3); \
	new="$$major.$$minor.$$((patch+1))"; \
	sed -i '' "s/^version = \"$$old\"/version = \"$$new\"/" Cargo.toml; \
	sed -i '' "s/v$$old/v$$new/" frontend/src/App.tsx; \
	echo "$$old → $$new"

## Bump the minor version (0.1.4 → 0.2.0) and update frontend version badge
bump-minor:
	@old=$$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	major=$$(echo $$old | cut -d. -f1); \
	minor=$$(echo $$old | cut -d. -f2); \
	new="$$major.$$((minor+1)).0"; \
	sed -i '' "s/^version = \"$$old\"/version = \"$$new\"/" Cargo.toml; \
	sed -i '' "s/v$$old/v$$new/" frontend/src/App.tsx; \
	echo "$$old → $$new"

## Bump the major version (0.1.4 → 1.0.0) and update frontend version badge
bump-major:
	@old=$$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	major=$$(echo $$old | cut -d. -f1); \
	new="$$((major+1)).0.0"; \
	sed -i '' "s/^version = \"$$old\"/version = \"$$new\"/" Cargo.toml; \
	sed -i '' "s/v$$old/v$$new/" frontend/src/App.tsx; \
	echo "$$old → $$new"
