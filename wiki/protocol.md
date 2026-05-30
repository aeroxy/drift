# Protocol Reference

## Control Messages

Control messages are JSON, serialized with `serde` using `#[serde(tag = "type")]`. They are sent as text WebSocket frames. Over server-to-server connections they are encrypted (see [encryption.md](./encryption.md)).

### Enum: `ControlMessage` (`src/protocol/messages.rs`)

| Variant | Fields | Direction | Purpose |
|---------|--------|-----------|---------|
| `KeyExchange` | `public_key: String` (base64) | bidirectional | X25519 handshake |
| `HandshakeComplete` | — | server→client | Signals encryption is ready |
| `InfoRequest` | `request_id?: Uuid` | either→other | Request hostname + root_dir |
| `InfoResponse` | `request_id?: Uuid`, `hostname`, `root_dir`, `has_remote` | response | Reply to InfoRequest |
| `BrowseRequest` | `request_id?: Uuid`, `path: String` | either→other | List a directory |
| `BrowseResponse` | `request_id?: Uuid`, `hostname`, `cwd`, `entries: Vec<FileEntry>` | response | Directory listing |
| `TransferRequest` | `id: Uuid`, `entries: Vec<TransferEntry>`, `direction: Direction` | initiator→remote | Start a transfer |
| `TransferAccepted` | `id: Uuid`, `resume_offsets: HashMap<String, u64>` | remote→initiator | Accept and ready |
| `TransferProgress` | `id`, `path`, `bytes_done`, `bytes_total` | sender→browser | Progress update |
| `TransferComplete` | `id: Uuid`, `total_bytes: u64` | sender→receiver | All data sent; receiver verifies byte count |
| `TransferFinalized` | `id: Uuid` | receiver→sender | Receiver has written and finalized all data |
| `TransferError` | `id: Uuid`, `error: String` | either | Failure |
| `ConnectionStatus` | `has_remote: bool` | server→browser | Pushed to browsers when remote connects/disconnects |
| `Ping` / `Pong` | `request_id?: Uuid` | bidirectional | Keep-alive |
| `Error` | `request_id?: Uuid`, `message: String` | either | Generic error |

## Protocol Versions

- **v1**: Original encrypted control/data framing
- **v2**: Multi-file data frame support (`file_index` in binary frames)
- **v3**: Correlated browse/info/ping request ids

New binaries advertise protocol v3 during the `KeyExchange`. Older v2 peers remain supported: they ignore request ids on incoming requests and may reply without a `request_id`.

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

`ControlMessage::is_request()` identifies messages that expect a response. Each side maintains a `HashMap<Uuid, oneshot::Sender<ControlMessage>>` (`pending`) keyed by the protocol `request_id`.

On v3 peers, browse/info/ping requests carry an optional `request_id` that is echoed back on `BrowseResponse`, `InfoResponse`, `Pong`, and request-scoped `Error` replies. `TransferRequest`/`TransferAccepted` already use the transfer UUID as their correlation key.

Pending requests are registered before enqueueing the outbound control message and are removed on success, timeout, or disconnect.

For legacy v2 peers that do not echo `request_id`, responses fall back to FIFO delivery order. That preserves interoperability with older binaries while keeping full correlation for v3 peers.

## REST API

| Endpoint            | Method | Body                              | Notes |
|---------------------|--------|-----------------------------------|-------|
| `/api/browse`       | GET    | `?path=`                          | Local file listing |
| `/api/info`         | GET    | —                                 | `{ hostname, root_dir, has_remote, fingerprint, can_reconnect, last_target }` |
| `/api/connect`      | POST   | `{ target, password? }`           | Establish a new server-to-server connection |
| `/api/reconnect`    | POST   | —                                 | Re-establish using credentials remembered from the last successful connect |
| `/api/disconnect`   | POST   | —                                 | Tear down current connection **and** forget stored credentials |

`can_reconnect` is `true` when there is no live connection but the server still remembers a target (CLI-initiated startup or a prior successful `/api/connect`). The FE renders a "Reconnect" button when this flag is set. `last_target` is the bare target string for display; the password is never returned.
