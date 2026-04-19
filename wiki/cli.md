# CLI Commands

drift uses clap subcommands. Run `drift` with no subcommand to start the server.

## Starting the server

```bash
drift [--port <PORT>] [--target <HOST>] [--password <PW>]
      [--disable-ui] [--allow-insecure-tls] [--daemon]
```

Starts the web UI on the given port (default: random free port). If `--target` is provided, connects to the remote server for bidirectional file browsing and transfer.

| Flag | Description |
|------|-------------|
| `--port <PORT>` | Port to listen on (default: random) |
| `--target <HOST>` | Remote to connect to, e.g. `192.168.0.2:8000` or `wss://example.com` |
| `--password <PW>` | Optional password for authentication |
| `--disable-ui` | Expose only `/ws` — disable the REST API and embedded frontend. Use when serving behind a public reverse proxy. |
| `--allow-insecure-tls` | Accept self-signed / invalid TLS certificates when connecting to a `wss://` target |
| `--daemon` | Start the server in the background. Logs are appended to `./drift.log` in the current directory. Prints PID on exit. |

### Background daemon

```bash
drift --port 8000 --daemon
# drift daemon started (PID: 12345)
# Logs: /path/to/cwd/drift.log

tail -f drift.log     # follow logs
kill 12345            # stop the daemon
```

### Reverse-proxy deployment (e.g. caddy with wss://)

Run drift with `--disable-ui` so only the encrypted `/ws` endpoint is reachable from the internet:

```bash
# Server side
drift --port 8000 --disable-ui --daemon

# Client side (connecting via wss://)
drift ls --target wss://aero.example.com/drift
drift pull --target wss://aero.example.com/drift somefile.txt
```

Caddy config (strips `/drift` prefix before proxying):
```caddy
aero.example.com {
    handle_path /drift/* {
        reverse_proxy localhost:8000
    }
}
```

## `send` — Send a file or folder

```bash
drift send --target <HOST> <PATH> [--password <PW>] [--allow-insecure-tls]
```

Connects to a running drift server, sends the file or folder, prints progress, and exits. Folders are automatically compressed to tar.gz before transfer.

## `ls` — List remote files

```bash
drift ls --target <HOST> [PATH] [--password <PW>] [--allow-insecure-tls]
```

Connects to a running drift server and lists files at the given path (or root if omitted). Output format is similar to `ls -lh`:

```
hostname:/path/to/dir
drwxr-xr-x  4.0K  2026-04-05 14:30  Documents/
-rw-r--r--  1.2M  2026-04-04 09:15  report.pdf
```

## `pull` — Pull a file or folder

```bash
drift pull --target <HOST> <REMOTE_PATH> [--output <DIR>] [--password <PW>] [--allow-insecure-tls]
```

Connects to a running drift server and downloads the specified file or folder. The remote path is relative to the server's root directory.

- `--output` / `-o`: local directory to write to (defaults to current directory)
- Folders are received as tar.gz and automatically extracted
- Progress is logged during transfer

### Pull flow

1. Client connects and performs encrypted handshake
2. Client sends `BrowseRequest` for the parent directory to discover file metadata
3. Client sends `TransferRequest` with `Direction::Pull`
4. Server accepts and streams file data as encrypted binary frames
5. Client writes chunks to disk via `ChunkedWriter`
6. On `TransferComplete`, client finalizes the write and sends `TransferFinalized`
7. For directories: the received tar.gz archive is extracted and cleaned up

## Target format

All commands accept `--target` in three forms:

| Form | Scheme |
|------|--------|
| `192.168.0.2:8000` | `ws://` (default) |
| `ws://192.168.0.2:8000` | explicit plain |
| `wss://example.com/drift` | TLS; `/ws` appended to path |

## Backward compatibility

The old flat-arg syntax still works:

```bash
drift --port 8000                              # start server
drift --target host:8000 --file path           # same as: drift send --target host:8000 path
```
