# CLI Commands

drift uses clap subcommands. Legacy flat args (`--port`, `--file --target`) are still supported for backward compatibility.

## `serve` — Start the server

```bash
drift serve --port <PORT> [--target <HOST:PORT>] [--password <PW>]
```

Starts the web UI on the given port. If `--target` is provided, connects to the remote server for bidirectional file browsing and transfer.

## `send` — Send a file or folder

```bash
drift send --target <HOST:PORT> <PATH> [--password <PW>]
```

Connects to a running drift server, sends the file or folder, prints progress, and exits. Folders are automatically compressed to tar.gz before transfer.

## `ls` — List remote files

```bash
drift ls --target <HOST:PORT> [PATH] [--password <PW>]
```

Connects to a running drift server and lists files at the given path (or root if omitted). Output format is similar to `ls -lh`:

```
hostname:/path/to/dir
drwxr-xr-x  4.0K  2026-04-05 14:30  Documents/
-rw-r--r--  1.2M  2026-04-04 09:15  report.pdf
```

## `pull` — Pull a file or folder

```bash
drift pull --target <HOST:PORT> <REMOTE_PATH> [--output <DIR>] [--password <PW>]
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

## Backward compatibility

The old flat-arg syntax still works:

```bash
drift --port 8000                              # same as: drift serve --port 8000
drift --target host:8000 --file path           # same as: drift send --target host:8000 path
```
