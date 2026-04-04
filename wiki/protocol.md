# Protocol Reference

## Control Messages

Control messages are JSON, serialized with `serde` using `#[serde(tag = "type")]`. They are sent as text WebSocket frames. Over server-to-server connections they are encrypted (see [encryption.md](./encryption.md)).

### Enum: `ControlMessage` (`src/protocol/messages.rs`)

| Variant | Fields | Direction | Purpose |
|---------|--------|-----------|---------|
| `KeyExchange` | `public_key: String` (base64) | bidirectional | X25519 handshake |
| `HandshakeComplete` | — | server→client | Signals encryption is ready |
| `InfoRequest` | — | either→other | Request hostname + root_dir |
| `InfoResponse` | `hostname`, `root_dir`, `has_remote` | response | Reply to InfoRequest |
| `BrowseRequest` | `path: String` | either→other | List a directory |
| `BrowseResponse` | `hostname`, `cwd`, `entries: Vec<FileEntry>` | response | Directory listing |
| `TransferRequest` | `id: Uuid`, `entries: Vec<TransferEntry>`, `direction: Direction` | initiator→remote | Start a transfer |
| `TransferAccepted` | `id: Uuid`, `resume_offsets: HashMap<String, u64>` | remote→initiator | Accept and ready |
| `TransferProgress` | `id`, `path`, `bytes_done`, `bytes_total` | sender→browser | Progress update |
| `TransferComplete` | `id: Uuid`, `total_bytes: u64` | sender→receiver | All data sent; receiver verifies byte count |
| `TransferFinalized` | `id: Uuid` | receiver→sender | Receiver has written and finalized all data |
| `TransferError` | `id: Uuid`, `error: String` | either | Failure |
| `ConnectionStatus` | `has_remote: bool` | server→browser | Pushed to browsers when remote connects/disconnects |
| `Ping` / `Pong` | — | bidirectional | Keep-alive |
| `Error` | `message: String` | either | Generic error |

### Enum: `Direction`

```rust
pub enum Direction {
    Push,  // sender → receiver (local files sent to remote)
    Pull,  // requester → remote asks remote to send files back
}
```

### Struct: `TransferEntry`

```rust
pub struct TransferEntry {
    pub relative_path: String,  // relative to root_dir
    pub size: u64,
    pub is_dir: bool,
    pub permissions: Option<u32>,
}
```

## Frame Format (Encrypted Connection)

After the handshake, **all** server-to-server messages are binary WebSocket frames. Each encrypted payload starts with a **type byte** that identifies the content:

```
┌─────────────┬──────────────────────────────────────────────────────────────────┐
│  Type (1B)  │  Payload                                                        │
├─────────────┼──────────────────────────────────────────────────────────────────┤
│  0x00       │  Data: [16B UUID][8B offset BE][chunk ≤64 KB]                   │
│  0x01       │  Control: [JSON bytes]                                          │
└─────────────┴──────────────────────────────────────────────────────────────────┘
```

- **0x00 — Data frame**: transfer UUID + cumulative byte offset + chunk data
- **0x01 — Control frame**: JSON-serialized `ControlMessage`

Both data and control frames travel through a single unified FIFO channel (`FrameChannel`). The write task encrypts each frame and sends it as a binary WS frame. The read task decrypts, checks the type byte, and dispatches accordingly.

### Encoding/Decoding (`src/protocol/codec.rs`)

```rust
encode_data_frame(transfer_id, offset, data) -> Vec<u8>   // [0x00][UUID][offset][data]
encode_control_frame(json_bytes)             -> Vec<u8>   // [0x01][json]
decode_frame_type(frame)                     -> (u8, &[u8])  // (type, payload)
decode_data_frame(payload)                   -> (Uuid, u64, &[u8])
```

## Connection Types

### Browser connection (plaintext)
- Browser sends first message that is NOT a `KeyExchange` JSON
- Server detects this and stays in plaintext mode
- Control messages are raw JSON text frames
- No binary frames from browser to server

### Server-to-server connection (encrypted)
- Server sends `KeyExchange` immediately on connect (text frame)
- Client responds with its own `KeyExchange` (text frame)
- Server sends `HandshakeComplete` (text frame)
- All subsequent messages are **binary** WS frames with the type-byte prefix
- A single `FrameChannel` carries both data and control frames (FIFO)

## Request / Response Pattern

`ControlMessage::is_request()` identifies messages that expect a response. Each side maintains a `HashMap<Uuid, oneshot::Sender<ControlMessage>>` (`pending`) to match responses back to callers.

Requests are sent via `request_tx`, tagged with a generated UUID inserted into `pending`. Responses are matched against the oldest pending entry (FIFO assumption — only one in-flight request at a time per connection).
