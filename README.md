<p align="center">
  <img src="frontend/public/logo.svg" alt="drift" width="320" />
</p>

<p align="center">
  <strong>Encrypted file transfer over WebSocket</strong>
</p>

<p align="center">
  <a href="#install">Install</a> &middot;
  <a href="#usage">Usage</a> &middot;
  <a href="#how-it-works">How It Works</a> &middot;
  <a href="#development">Development</a>
</p>

---

**drift** is a single binary that lets you securely copy files and folders between two machines over WebSocket. It includes a built-in web UI with a two-pane file browser — no setup, no cloud, no SSH keys.

## Features

- Single self-contained binary (React frontend embedded)
- End-to-end encryption via X25519 key exchange + ChaCha20-Poly1305
- Optional password authentication
- Two-pane file browser UI (`hostname:/pwd` on each side)
- Large file support with chunked streaming and progress indication
- Recursive directory transfer (auto-compressed via tar.gz)
- Direct CLI file send mode (`--file`) — no web UI needed
- Bidirectional — push files to remote **or** pull files from remote
- Both directions work from the browser UI (left pane = local, right pane = remote)
- Zero configuration — just run it

## Install

### From source

```bash
# Requires Rust 1.82+ and bun (or npm)
git clone https://github.com/aeroxy/drift.git
cd drift
cargo build --release

# Binary at target/release/drift
```

## Usage

### Start a server

```bash
# Serve on port 8000, browsing the current directory
drift --port 8000
```

Visit `http://localhost:8000` to see the file browser.

### Connect two machines

```bash
# Machine A (server)
drift --port 8000

# Machine B (connects to A)
drift --port 9000 --target 192.168.0.2:8000
```

Both machines now show a two-pane UI. Select files on either side and copy them across. Both `localhost:8000` and `localhost:9000` show each other's files.

### Send a file directly (no web UI)

```bash
# Send a file to a running drift server
drift --target 192.168.0.2:8000 --file video.mp4

# Send a folder (automatically compressed)
drift --target 192.168.0.2:8000 --file ./my-project

# With password
drift --target 192.168.0.2:8000 --file data.zip --password secret
```

This connects, transfers the file, prints progress, and exits. No web server is started.

### With password authentication

```bash
# Machine A
drift --port 8000 --password mysecret

# Machine B
drift --port 9000 --target 192.168.0.2:8000 --password mysecret
```

## How It Works

1. **Machine A** starts a WebSocket server on the specified port
2. **Machine B** connects and performs an X25519 Diffie-Hellman key exchange
3. Both derive symmetric keys via HKDF-SHA256 (separate keys for each direction)
4. If `--password` is set, an HMAC challenge-response authenticates the peer
5. All subsequent messages are encrypted with ChaCha20-Poly1305
6. Files are streamed in 64KB chunks over encrypted WebSocket binary frames
7. Folders are compressed to `.tar.gz` before transfer and extracted on the other side
8. The web UI at `localhost:<port>` shows both file trees side by side

### Protocol

| Frame Type | Format | Purpose |
|---|---|---|
| Text (encrypted) | JSON `ControlMessage` | Browse, transfer control, progress |
| Binary (encrypted) | `[16B UUID][8B offset][chunk]` | File data |

### Security

- **Forward secrecy**: ephemeral X25519 keys per session
- **Path traversal protection**: all paths are canonicalized and validated against the root directory
- **Authenticated encryption**: ChaCha20-Poly1305 with monotonic nonce counters

## CLI Reference

```
drift [OPTIONS]

Options:
    --port <PORT>          Port to run the server on (not needed with --file)
    --target <TARGET>      Remote target to connect to (e.g. 192.168.0.2:8000)
    --password <PASSWORD>  Optional password for authentication
    --file <FILE>          Send a file or folder directly without starting a web panel
    -h, --help             Print help
```

### Modes

| Mode | Command | Description |
|---|---|---|
| Server only | `drift --port 8000` | Starts web UI, waits for connections |
| Server + Client | `drift --port 9000 --target host:8000` | Starts web UI and connects to remote |
| Direct send | `drift --target host:8000 --file path` | Sends file/folder and exits |

## Development

```bash
# Frontend dev server (hot reload)
cd frontend && bun dev

# Rust backend (in another terminal)
cargo run -- --port 8000

# Build everything (frontend + backend)
cargo build
```

The `build.rs` script automatically builds the React frontend before compiling Rust. The built assets are embedded in the binary via `rust-embed`.

### Testing

Integration tests live in `frontend/test/`. They start real drift instances, transfer files via WebSocket, and verify MD5 checksums.

**Set up test-resources** (not committed — create your own):

```
test-resources/
├── host/           # Put at least one subdirectory with files here (tests folder transfer)
│   └── some-dir/
│       └── file.ext
└── client/         # Put at least one file here (tests file transfer)
    └── file.ext
```

Any files work — the tests discover entries dynamically. The host directory should contain at least one **subdirectory** to exercise the tar.gz folder transfer path.

```bash
# Run integration tests (builds cargo first)
cd frontend && bun run test
```

The test suite:
1. Backs up `test-resources/` → `test-resources-bak/`
2. Starts two drift instances (host + client) on random ports
3. Pushes host files to client and client files to host via WebSocket
4. Verifies MD5 checksums of all transferred files
5. Verifies `.drift/` temp directories are cleaned up
6. Restores `test-resources/` from backup

### Project Structure

```
drift/
├── src/
│   ├── main.rs              # CLI entry point
│   ├── server/
│   │   ├── mod.rs           # AppState, axum router
│   │   ├── ws_handler.rs    # WebSocket connection handler
│   │   ├── file_api.rs      # REST API (browse, info)
│   │   ├── browser_transfer.rs  # Transfer orchestration (browser-initiated)
│   │   └── transfer_receiver.rs # Incoming file writer + decompression
│   ├── client/
│   │   ├── mod.rs           # Outbound WS connection to --target
│   │   └── send.rs          # Direct --file send mode
│   ├── protocol/
│   │   ├── messages.rs      # ControlMessage enum, TransferEntry
│   │   └── codec.rs         # Binary frame encoding/decoding
│   ├── crypto/
│   │   ├── handshake.rs     # X25519 key exchange
│   │   └── stream.rs        # ChaCha20-Poly1305 encrypt/decrypt
│   ├── fileops/
│   │   ├── browse.rs        # Directory listing with traversal protection
│   │   ├── reader.rs        # Chunked async file reader
│   │   ├── writer.rs        # Chunked async file writer with .part files
│   │   ├── compress.rs      # Folder → tar.gz compression
│   │   └── decompress.rs    # tar.gz → folder extraction
│   └── frontend.rs          # rust-embed static serving
├── frontend/                # React + TypeScript + Tailwind v4
└── build.rs                 # Builds frontend before Rust compile
```

## License

MIT
