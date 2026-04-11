# Encryption

All server-to-server communication is end-to-end encrypted. Browser connections are plaintext (the browser is local to the server).

## Key Exchange (X25519 ECDH)

Implemented in `src/crypto/handshake.rs`.

1. **Server** generates an ephemeral X25519 keypair and sends its public key as `KeyExchange { public_key: base64 }`.
2. **Client** generates its own ephemeral keypair, sends its public key, and computes the shared secret via Diffie-Hellman: `shared_secret = DH(client_secret, server_public)`.
3. **Server** computes the same shared secret: `shared_secret = DH(server_secret, client_public)`.
4. If `--password` is set on the server, **password authentication** occurs (see below).
5. **Server** sends `HandshakeComplete`.

The shared secret is 32 bytes of X25519 output.

## Password Authentication (MITM Protection)

When the server has `--password` configured, a challenge-response exchange occurs after the DH key exchange but before `HandshakeComplete`. This binds the password to the shared secret, preventing man-in-the-middle attacks — an attacker completing independent DH with each side gets different shared secrets and cannot produce valid proofs.

```
Server                          Client
  |--- AuthChallenge {nonce} ---->|
  |<-- AuthResponse {proof}  ----|
  |   (verify proof)             |
  |--- HandshakeComplete ------->|   (or Error if proof invalid)
```

- **Nonce**: 32 random bytes (base64-encoded), generated per connection
- **Proof**: `HMAC-SHA256(password, nonce || shared_secret)` (base64-encoded)
- **Verification**: Server recomputes the HMAC and compares (constant-time via `hmac` crate)

If the client has no password but the server requires one, the client exits with an error. If the client has a password but the server doesn't challenge, a warning is logged.

## Key Derivation (HKDF-SHA256)

Two symmetric keys are derived from the shared secret via HKDF (in `src/crypto/stream.rs`):

| Key | Salt | Info | Used for |
|-----|------|------|----------|
| `c2s_key` | `b"drift-c2s"` | `b"ws-connector"` | Client→Server encryption |
| `s2c_key` | `b"drift-s2c"` | `b"ws-connector"` | Server→Client encryption |

Each side uses its send key to encrypt and its receive key to decrypt — so there are always two distinct cipher instances per connection.

## Stream Cipher (ChaCha20-Poly1305)

Implemented in `src/crypto/stream.rs` via the `chacha20poly1305` crate.

```rust
pub struct CryptoStream {
    send_cipher: ChaCha20Poly1305,
    recv_cipher: ChaCha20Poly1305,
    send_nonce_prefix: [u8; 4],   // 4-byte direction-specific prefix
    recv_nonce_prefix: [u8; 4],
    send_counter: AtomicU64,       // monotonically increasing
    recv_counter: AtomicU64,
}
```

### Nonce Construction (12 bytes)

```
┌──────────────────────────┬──────────────────────────────────────────┐
│  4-byte prefix           │  8-byte counter (big-endian)             │
└──────────────────────────┴──────────────────────────────────────────┘
```

The prefix is derived from the shared secret using a separate HKDF expansion (direction-specific). The counter starts at 0 and increments atomically with each message — guaranteeing nonce uniqueness for the lifetime of a connection.

### Encrypt / Decrypt

```rust
// encrypt: fetch-and-increment counter, build nonce, encrypt with AEAD tag
pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError>

// decrypt: same counter logic on receiver side
pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError>
```

The AEAD tag (16 bytes) is appended to the ciphertext. Decryption verifies it, providing both confidentiality and integrity.

## Wire Encoding

**Control messages (text frames):**
```
encrypt(JSON bytes) → ciphertext → base64(ciphertext) → WS text frame
```

**Binary data frames:**
```
encode_data_frame(id, offset, chunk) → raw bytes → encrypt(raw bytes) → WS binary frame
```

## Important Notes

- **No nonce persistence across reconnects** — counters reset to 0 on each new connection. A dropped connection requires a full new handshake.
- **Password authentication** — when `--password` is set, HMAC-SHA256 proof binds the password to the DH shared secret, preventing MITM attacks. Both sides must use the same password.
- **Fingerprint verification** — after every handshake, a 6-character hex fingerprint (`SHA-256(shared_secret)[0..3]`) is logged in both terminals and shown in the web UI toolbar. Users can compare out-of-band (e.g. via Telegram) to detect MITM even without a password.
- **Ephemeral keys** — each connection generates fresh X25519 keypairs; there is no long-term identity.
