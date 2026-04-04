# Push Transfer

A **push** sends files from the machine where the browser is open to the remote peer.

## Flow

```
Browser (Machine A)    Server A            Server B (remote)
       │                   │                       │
       │ TransferRequest   │                       │
       │ direction=Push    │                       │
       │──────────────────▶│                       │
       │                   │ TransferRequest(Push) │
       │                   │──────────────────────▶│
       │                   │   TransferAccepted    │
       │                   │◀──────────────────────│
       │  TransferAccepted │                       │
       │◀──────────────────│                       │
       │                   │ binary chunks ───────▶│
       │  TransferProgress │                       │
       │◀──────────────────│                       │
       │                   │ TransferComplete ────▶│
       │                   │  (with total_bytes)   │
       │                   │   TransferFinalized   │
       │                   │◀──────────────────────│
       │  TransferComplete │                       │
       │◀──────────────────│                       │
```

Server A waits for `TransferFinalized` from Server B before forwarding `TransferComplete` to the browser. This guarantees the browser only shows success after all bytes are confirmed written on the remote.

## Code Path

1. **Browser** calls `handleCopyToRemote()` in [frontend/src/App.tsx](../frontend/src/App.tsx), sends `TransferRequest { direction: "Push" }` over the plaintext WebSocket.
2. **Server A** (`ws_handler.rs:handle_browser_message`) routes it to `handle_browser_transfer()` in [src/server/browser_transfer.rs](../src/server/browser_transfer.rs).
3. `handle_browser_transfer()` forwards the `TransferRequest` to **Server B** via the encrypted server-to-server channel.
4. **Server B** (`ws_handler.rs:handle_server_to_server_request`) calls `transfer_receiver.start_transfer()` and responds with `TransferAccepted`.
5. `handle_browser_transfer()` calls `push_entries()`, which:
   - Compresses any directories to `.drift/{name}.tar.gz` via `compress_directory()`
   - Opens each file with `ChunkedReader` (64 KB chunks)
   - Encodes binary frames: `[16B UUID][8B offset][chunk]` via `encode_data_frame()`
   - Sends encrypted binary frames to Server B via `binary_tx`
   - Sends `TransferProgress` updates to the browser
6. After all data is sent, `push_entries()` sends `TransferComplete { total_bytes }` to Server B and waits for a `TransferFinalized` acknowledgment.
7. **Server B**'s read loop receives `TransferComplete`, calls `transfer_receiver.signal_completion(id, total_bytes)`:
   - If all bytes already received, finalizes immediately and sends `TransferFinalized` back
   - Otherwise sets `expected_total`; the final chunk auto-triggers finalization and sends `TransferFinalized`
8. Once `TransferFinalized` is received, `push_entries()` sends `TransferComplete` to the browser.
9. **Server B**'s `finalize_transfer()`:
   - Renames `.part` file to the final name
   - If the transfer contained directories, decompresses the `.tar.gz` archive

## Key Files

| File | Role |
|------|------|
| `frontend/src/App.tsx` | Initiates push from browser UI |
| `src/server/browser_transfer.rs` | Orchestrates push, calls `push_entries()` |
| `src/server/ws_handler.rs` | Routes browser WS messages, handles Push on server B |
| `src/server/transfer_receiver.rs` | Receives chunks and finalizes on server B |
| `src/fileops/reader.rs` | `ChunkedReader` — reads files in 64 KB chunks |
| `src/fileops/writer.rs` | `ChunkedWriter` — writes chunks to `.part` files |
| `src/fileops/compress.rs` | Directory → tar.gz |
| `src/fileops/decompress.rs` | tar.gz → directory |
| `src/protocol/codec.rs` | Binary frame encode/decode |
