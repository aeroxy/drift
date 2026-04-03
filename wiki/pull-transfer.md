# Pull Transfer

A **pull** requests files from the remote peer and downloads them to the machine where the browser is open.

## Flow

```
Browser (Machine A)    Server A            Server B (remote)
       │                   │                       │
       │ TransferRequest   │                       │
       │ direction=Pull    │                       │
       │──────────────────▶│                       │
       │                   │ TransferRequest(Pull) │
       │                   │──────────────────────▶│
       │                   │   TransferAccepted    │
       │                   │◀──────────────────────│
       │  TransferAccepted │                       │
       │◀──────────────────│                       │
       │                   │◀─── binary chunks ────│
       │                   │ (A's read loop writes │
       │                   │  chunks to disk)      │
       │                   │   TransferComplete    │
       │                   │◀──────────────────────│
       │                   │ (finalize_transfer)   │
       │  TransferComplete │                       │
       │◀──────────────────│                       │
```

## Code Path

1. **Browser** calls `handleCopyToLocal()` in [frontend/src/App.tsx](../frontend/src/App.tsx), sends `TransferRequest { direction: "Pull", entries: [remote paths] }` over the plaintext WebSocket.
2. **Server A** (`ws_handler.rs:handle_browser_message`) routes it to `handle_browser_transfer()` in [src/server/browser_transfer.rs](../src/server/browser_transfer.rs).
3. `handle_browser_transfer()` forwards the `TransferRequest` (Pull) to **Server B** via the encrypted server-to-server channel.
4. **Server B** (`ws_handler.rs:handle_server_to_server_request`):
   - Responds immediately with `TransferAccepted`
   - Spawns a tokio task that calls `send_entries()` — reads the requested files and streams binary chunks back to Server A via `binary_tx` + `outgoing_tx`
5. **Server A** in `handle_browser_transfer()`:
   - Calls `transfer_receiver.start_transfer_with_notify()` to prepare to receive chunks and get a completion `oneshot::Receiver`
   - Waits on the completion receiver (max 30 min)
6. **Server A's WS read loop** receives the incoming binary frames, decrypts them, and routes each chunk to `transfer_receiver.receive_chunk()`.
7. When **Server B** is done, it sends `TransferComplete`. Server A's read loop receives it, calls `transfer_receiver.finalize_transfer()`, which fires the completion channel.
8. `handle_browser_transfer()` unblocks, sends `TransferComplete` to the browser.

## Key Files

| File | Role |
|------|------|
| `frontend/src/App.tsx` | Initiates pull from browser UI (`handleCopyToLocal`) |
| `src/server/browser_transfer.rs` | Orchestrates pull; `send_entries()` reads and streams files |
| `src/server/ws_handler.rs` | Handles Pull TransferRequest on server B (spawns send task) |
| `src/client/mod.rs` | Same Pull handling for the client side |
| `src/server/transfer_receiver.rs` | Receives chunks on server A; `start_transfer_with_notify()` for completion signaling |
| `src/fileops/reader.rs` | `ChunkedReader` — reads remote files in 64 KB chunks |
| `src/fileops/writer.rs` | `ChunkedWriter` — writes received chunks locally |
| `src/fileops/compress.rs` | Directory → tar.gz on remote side |
| `src/fileops/decompress.rs` | tar.gz → directory on local side |
| `src/protocol/codec.rs` | Binary frame encode/decode |

## Difference from Push

| Aspect | Push | Pull |
|--------|------|------|
| Who reads files | Server A (requester) | Server B (remote) |
| Binary data direction | A → B | B → A |
| Who calls `start_transfer` | Server B | Server A (via `start_transfer_with_notify`) |
| Completion signal | Browser receives `TransferComplete` directly | Server A waits on oneshot channel, then notifies browser |
