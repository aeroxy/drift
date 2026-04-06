# drift

Encrypted file transfer over WebSocket with an embedded React UI.

## Project Overview

drift is a single Rust binary that enables bidirectional, encrypted file/folder transfer between two machines. It embeds a React frontend served at the configured port, providing a two-pane file browser. It also supports CLI commands (`send`, `ls`, `pull`) for headless operation without the web UI.

## Architecture

- **Rust backend** (axum + tokio): HTTP server, WebSocket handler, file I/O, encryption
- **React frontend** (Vite + TypeScript + Tailwind): two-pane file browser, embedded via `rust-embed`
- **Protocol**: JSON control messages (text frames) + binary file chunks (binary frames), all encrypted after handshake

## Key Directories

- `src/server/` — axum router, WS handler, REST API, transfer orchestration
  - `ws_handler.rs` — WebSocket connection handler (browser + server-to-server)
  - `browser_transfer.rs` — Transfer orchestration for browser-initiated transfers; `send_entries()` is shared by push and pull
  - `transfer_receiver.rs` — Incoming file writer + tar.gz decompression; `start_transfer_with_notify()` for pull completion signaling
  - `file_api.rs` — REST endpoints (/api/browse, /api/info)
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

```bash
# Build (auto-builds frontend via build.rs)
cargo build

# Run server
cargo run -- serve --port 8000
cargo run -- serve --port 8000 --target 192.168.0.2:8000 --password secret

# List files on a remote host
cargo run -- ls --target 192.168.0.2:8000 [path]

# Pull a file or folder from a remote host
cargo run -- pull --target 192.168.0.2:8000 <remote-path> [--output dir]

# Send a file directly (no web UI)
cargo run -- send --target 192.168.0.2:8000 test.mp4

# Legacy flat args still work:
cargo run -- --port 8000
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
- Path safety: all user-supplied paths canonicalized and checked against root dir before any I/O
- Folder transfers: compressed to tar.gz in `.drift/` temp dir, decompressed on receiver
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

## Requirements

- Every new feature or CLI command must ship with a corresponding integration test in `frontend/test/integration.test.ts`. If the feature is a one-shot CLI command (like `ls` or `pull`), use `runDriftCli()` from `drift-process.ts` to exercise it against a live server and assert on output or file integrity.

## Dependencies

Rust: axum, tokio, clap, rust-embed, x25519-dalek, chacha20poly1305, serde, tokio-tungstenite, walkdir, uuid, hkdf, sha2, hmac, tar, flate2

Frontend: React 19, TypeScript, Vite, Tailwind CSS v4, lucide-react
