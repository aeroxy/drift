# drift

Encrypted file transfer over WebSocket with an embedded React UI.

## Project Overview

drift is a single Rust binary that enables bidirectional, encrypted file/folder transfer between two machines. It embeds a React frontend served at the configured port, providing a two-pane file browser with multi-select (Cmd/Ctrl-click to toggle, Shift-click to range-select). It also supports CLI commands (`send`, `ls`, `pull`) for headless operation without the web UI.

## Architecture

- **Rust backend** (axum + tokio): HTTP server, WebSocket handler, file I/O, encryption
- **React frontend** (Vite + TypeScript + Tailwind): two-pane file browser, embedded via `rust-embed`
- **Protocol**: JSON control messages (text frames) + binary file chunks (binary frames), all encrypted after handshake. v3 adds correlated request/response IDs for concurrent requests.

## Key Directories

- `src/server/` — axum router, WS handler, REST API, transfer orchestration
  - `ws_handler.rs` — WebSocket connection handler (browser + server-to-server)
  - `browser_transfer.rs` — Transfer orchestration for browser-initiated transfers; `send_entries()` is shared by push and pull
  - `transfer_receiver.rs` — Incoming file writer + tar.gz decompression; `start_transfer_with_notify()` for pull completion signaling
  - `file_api.rs` — REST endpoints (/api/browse, /api/browse-remote, /api/info, /api/connect, /api/reconnect, /api/disconnect). `/api/info` exposes `can_reconnect` + `last_target` so the FE can render a Reconnect button after an unexpected drop. `/api/browse-remote` fetches remote directory entries via REST without WS side effects (for address bar autocomplete).
- `src/client/` — outbound WS connection to `--target`
  - `mod.rs` — Bidirectional encrypted WS connection; shared types (`WsWrite`, `WsRead`, `DecryptedFrame`) and `recv_encrypted_frame()`
  - `send.rs` — Direct file send mode (connect, transfer, exit); shared helpers `send_encrypted_control()`, `recv_encrypted_control()`, `format_bytes()`
  - `browse.rs` — Remote file listing (`ls` command)
  - `pull.rs` — Remote file pull (`pull` command)
- `src/protocol/` — message types (`ControlMessage` enum), binary codec
- `src/crypto/` — X25519 key exchange, ChaCha20-Poly1305 stream cipher
- `src/fileops/` — directory listing, chunked async reader/writer, tar.gz compress/decompress
  - `browse.rs` — Directory listing (hides `.drift/` temp dir)
  - `compress.rs` — Folder → tar.gz compression for transfer
  - `decompress.rs` — tar.gz → folder extraction after receive
- `src/frontend.rs` — `rust-embed` static asset serving with SPA fallback
- `frontend/` — React app (Vite + TypeScript + Tailwind v4)
- `frontend/test/` — integration tests (vitest); see README.md for test-resources setup
- `wiki/` — feature documentation (see [Wiki](#wiki) section below)

## Build & Run

**Always run both steps before manual testing** — the frontend must be built first, then Cargo embeds it:

```bash
# build the React frontend
cd frontend && bun run build && cd ..

# build the Rust backend
cargo build

# Run without a port (OS picks a free port and logs it)
cargo run

# Run server
cargo run -- --port 8000
cargo run -- --port 8000 --target 192.168.0.2:8000 --password secret

# Run in background (logs → ./drift.log)
cargo run -- --port 8000 --daemon

# Expose only /ws (no UI/REST), useful behind a reverse proxy
cargo run -- --port 8000 --disable-ui

# Connect to a wss:// target (TLS-terminating reverse proxy)
cargo run -- --port 8000 --target wss://example.com/drift
cargo run -- ls --target wss://example.com/drift --allow-insecure-tls

# List files on a remote host
cargo run -- ls --target 192.168.0.2:8000 [path]

# Pull a file or folder from a remote host
cargo run -- pull --target 192.168.0.2:8000 <remote-path> [--output dir]

# Send a file directly (no web UI)
cargo run -- send --target 192.168.0.2:8000 test.mp4

# Legacy --file flat arg still works:
cargo run -- --target 192.168.0.2:8000 --file test.mp4

# Frontend dev (hot reload, proxies API/WS to Rust backend)
cd frontend && bun dev
```

## Conventions

- Use `bun` (not npm) for frontend package management
- Module naming: `fileops` (not `fs`) to avoid std lib conflict
- Protocol messages: serde tagged enum `ControlMessage` with `#[serde(tag = "type")]`
- Binary frames: `[16-byte UUID][8-byte BE offset][chunk data]`
- Encryption: encrypt-then-MAC via ChaCha20-Poly1305 AEAD, monotonic nonce counters
- Path safety: all user-supplied paths canonicalized and checked against root dir before any I/O. Transfer `destination_path` is additionally rejected by `validate_destination_path()` if absolute or containing `..`, at the receiver entry points (`start_transfer`/`start_transfer_with_notify`) before any I/O
- Folder transfers: compressed to tar.gz in `.drift/` temp dir, decompressed on receiver
- Receiver temp staging lives under the **destination** dir (`root_dir/<destination_path>/.drift`), not the served root — so a read-only root (e.g. drift launched from `/`) still works when the destination is writable, and finalize renames stay on one filesystem
- `.drift/` directory is hidden from the web panel browse listing
- When updating features, update README.md, this file (AGENTS.md / CLAUDE.md), **and the relevant wiki doc(s)**

## Wiki

The `wiki/` directory contains canonical documentation for each feature. **Always read the relevant wiki doc before working on a feature, and update it whenever behavior changes.**

| Doc | Covers |
|-----|--------|
| [wiki/push-transfer.md](wiki/push-transfer.md) | Push flow: browser → local server → remote server |
| [wiki/pull-transfer.md](wiki/pull-transfer.md) | Pull flow: browser requests files from remote |
| [wiki/protocol.md](wiki/protocol.md) | `ControlMessage` enum, binary frame format, connection types |
| [wiki/encryption.md](wiki/encryption.md) | X25519 handshake, HKDF key derivation, ChaCha20-Poly1305 nonces |
| [wiki/cli.md](wiki/cli.md) | CLI subcommands: serve, send, ls, pull |
| [wiki/address-bar-browsing.md](wiki/address-bar-browsing.md) | Address bar autocomplete, /api/browse-remote endpoint |

## Requirements

- Every new feature or CLI command must ship with a corresponding integration test in `frontend/test/integration.test.ts`. If the feature is a one-shot CLI command (like `ls` or `pull`), use `runDriftCli()` from `drift-process.ts` to exercise it against a live server and assert on output or file integrity.

## Preparing Release

1. Bump the version: `make bump-patch` (or `bump-minor` / `bump-major`)
2. Build release binaries for all platforms: `make release-all`
3. Zip the binaries:
   ```bash
   zip -j drift_macos_arm64.zip target/release/drift
   zip -j drift_linux_x86_64.zip target/x86_64-unknown-linux-gnu/release/drift
   ```
4. Update `Formula/drift.rb` with the correct SHA256s: `make update-formula`
