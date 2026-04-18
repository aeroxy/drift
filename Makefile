.PHONY: build check run dev test clean bump-patch bump-minor bump-major update-formula

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

## Bump the patch version (0.1.3 → 0.1.4) and update all version references
bump-patch:
	@old=$$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	major=$$(echo $$old | cut -d. -f1); \
	minor=$$(echo $$old | cut -d. -f2); \
	patch=$$(echo $$old | cut -d. -f3); \
	new="$$major.$$minor.$$((patch+1))"; \
	sed -i '' "s/^version = \"$$old\"/version = \"$$new\"/" Cargo.toml; \
	sed -i '' "s/v$$old/v$$new/" frontend/src/App.tsx; \
	sed -i '' "s/version \"$$old\"/version \"$$new\"/" Formula/drift.rb; \
	echo "$$old → $$new"

## Bump the minor version (0.1.4 → 0.2.0) and update all version references
bump-minor:
	@old=$$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	major=$$(echo $$old | cut -d. -f1); \
	minor=$$(echo $$old | cut -d. -f2); \
	new="$$major.$$((minor+1)).0"; \
	sed -i '' "s/^version = \"$$old\"/version = \"$$new\"/" Cargo.toml; \
	sed -i '' "s/v$$old/v$$new/" frontend/src/App.tsx; \
	sed -i '' "s/version \"$$old\"/version \"$$new\"/" Formula/drift.rb; \
	echo "$$old → $$new"

## Bump the major version (0.1.4 → 1.0.0) and update all version references
bump-major:
	@old=$$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	major=$$(echo $$old | cut -d. -f1); \
	new="$$((major+1)).0.0"; \
	sed -i '' "s/^version = \"$$old\"/version = \"$$new\"/" Cargo.toml; \
	sed -i '' "s/v$$old/v$$new/" frontend/src/App.tsx; \
	sed -i '' "s/version \"$$old\"/version \"$$new\"/" Formula/drift.rb; \
	echo "$$old → $$new"

## Update Formula/drift.rb SHA256 after uploading a release zip.
## Run this once the GitHub release zip is live:
##   make update-formula
update-formula:
	@ver=$$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	url="https://github.com/aeroxy/drift/releases/download/$$ver/drift_macos_arm64.zip"; \
	echo "Fetching $$url …"; \
	sha=$$(curl -sL "$$url" | shasum -a 256 | cut -d' ' -f1); \
	echo "SHA256: $$sha"; \
	sed -i '' "s/sha256 \"[a-f0-9]*\"/sha256 \"$$sha\"/" Formula/drift.rb; \
	echo "Formula/drift.rb updated"
