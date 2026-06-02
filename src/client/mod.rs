pub mod browse;
pub mod pull;
pub mod reconnect;
pub mod send;

use std::collections::HashMap;
use std::sync::Arc;

// ── WebSocket connection helpers ───────────────────────────────────────────────

/// Build the ws:// or wss:// URL from a user-supplied target string.
/// The path the user supplies is treated as a base prefix; `/ws` is appended
/// unless it's already there. Bare `host:port` defaults to `ws://` (back-compat).
pub(crate) fn build_ws_url(target: &str) -> String {
    let with_scheme = if target.starts_with("ws://") || target.starts_with("wss://") {
        target.to_string()
    } else {
        format!("ws://{}", target)
    };
    let trimmed = with_scheme.trim_end_matches('/').to_string();
    if trimmed.ends_with("/ws") {
        trimmed
    } else {
        format!("{}/ws", trimmed)
    }
}

/// Connect, applying TLS settings if the URL uses wss://.
pub(crate) async fn open_ws(
    target: &str,
    allow_insecure_tls: bool,
) -> anyhow::Result<(
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::handshake::client::Response,
)> {
    let url = build_ws_url(target);
    tracing::info!("Connecting to {}", url);
    if url.starts_with("wss://") {
        // Always build our own connector for wss:// so we can force ALPN to http/1.1.
        // Without this, TLS negotiates HTTP/2 and WebSocket upgrade hangs.
        let connector = if allow_insecure_tls {
            build_insecure_rustls_connector()?
        } else {
            build_secure_rustls_connector()?
        };
        Ok(
            tokio_tungstenite::connect_async_tls_with_config(&url, None, true, Some(connector))
                .await?,
        )
    } else {
        Ok(tokio_tungstenite::connect_async(&url).await?)
    }
}

/// rustls ClientConfig with native root certs and http/1.1 ALPN (required for WebSocket over TLS).
fn build_secure_rustls_connector() -> anyhow::Result<tokio_tungstenite::Connector> {
    let mut root_store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        root_store.add(cert).ok();
    }
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(tokio_tungstenite::Connector::Rustls(std::sync::Arc::new(
        config,
    )))
}

/// rustls ClientConfig with certificate verification disabled (for self-signed certs).
fn build_insecure_rustls_connector() -> anyhow::Result<tokio_tungstenite::Connector> {
    tracing::warn!("TLS certificate verification DISABLED (--allow-insecure-tls)");
    let mut config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(NoCertVerifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(tokio_tungstenite::Connector::Rustls(std::sync::Arc::new(
        config,
    )))
}

#[derive(Debug)]
struct NoCertVerifier;

impl rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ED448,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::build_ws_url;

    #[test]
    fn test_build_ws_url() {
        assert_eq!(build_ws_url("192.168.0.2:8000"), "ws://192.168.0.2:8000/ws");
        assert_eq!(
            build_ws_url("ws://192.168.0.2:8000"),
            "ws://192.168.0.2:8000/ws"
        );
        assert_eq!(build_ws_url("wss://example.com"), "wss://example.com/ws");
        assert_eq!(build_ws_url("wss://example.com/"), "wss://example.com/ws");
        assert_eq!(
            build_ws_url("wss://example.com/drift"),
            "wss://example.com/drift/ws"
        );
        assert_eq!(
            build_ws_url("wss://example.com/drift/ws"),
            "wss://example.com/drift/ws"
        );
    }
}
use crate::crypto::{
    handshake::{KeyPair, decode_public_key, derive_shared_secret},
    stream::CryptoStream,
};
use crate::protocol::codec::{
    FRAME_TYPE_CONTROL, FRAME_TYPE_DATA, FRAME_TYPE_DATA_V2, decode_data_frame, decode_frame_type,
    encode_control_frame,
};
use crate::protocol::messages::{CURRENT_PROTOCOL_VERSION, ControlMessage};
use crate::server::{AppState, PendingResponseOrder, RemoteConnection, ResponseChannel};
use futures_util::{SinkExt, StreamExt};
use std::collections::VecDeque;
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

/// Type alias for the WebSocket write half.
pub(crate) type WsWrite = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

/// Type alias for the WebSocket read half.
pub(crate) type WsRead = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

/// A decrypted frame from the WebSocket — either a data chunk or a control message.
#[allow(dead_code)]
pub(crate) enum DecryptedFrame {
    Control(ControlMessage),
    Data {
        transfer_id: Uuid,
        file_index: u32,
        offset: u64,
        chunk: Vec<u8>,
    },
}

/// Receive and decrypt the next binary WebSocket frame, returning either a data or control frame.
pub(crate) async fn recv_encrypted_frame(
    crypto: &CryptoStream,
    ws_read: &mut WsRead,
) -> anyhow::Result<DecryptedFrame> {
    loop {
        match ws_read.next().await {
            Some(Ok(Message::Binary(encrypted))) => {
                let plaintext = crypto.decrypt(&encrypted)?;
                let (frame_type, payload) = decode_frame_type(&plaintext)?;
                match frame_type {
                    FRAME_TYPE_DATA | FRAME_TYPE_DATA_V2 => {
                        let (id, file_index, offset, chunk) =
                            decode_data_frame(payload, frame_type)?;
                        return Ok(DecryptedFrame::Data {
                            transfer_id: id,
                            file_index,
                            offset,
                            chunk: chunk.to_vec(),
                        });
                    }
                    FRAME_TYPE_CONTROL => {
                        let msg: ControlMessage = serde_json::from_slice(payload)?;
                        return Ok(DecryptedFrame::Control(msg));
                    }
                    _ => continue,
                }
            }
            Some(Ok(Message::Close(_))) => anyhow::bail!("Connection closed by remote"),
            Some(Err(e)) => anyhow::bail!("WebSocket error: {}", e),
            None => anyhow::bail!("Connection closed"),
            _ => continue,
        }
    }
}

pub async fn connect_to_remote(
    target: &str,
    password: &Option<String>,
    allow_insecure_tls: bool,
    state: Arc<AppState>,
) -> anyhow::Result<()> {
    let connection_id = Uuid::new_v4();
    let (ws_stream, _) = open_ws(target, allow_insecure_tls).await?;
    tracing::info!("Connected to remote: {}", target);

    let (mut ws_write, mut ws_read) = ws_stream.split();

    let (crypto, fp, peer_version) =
        perform_client_handshake(&mut ws_write, &mut ws_read, password).await?;
    tracing::info!(
        "Handshake complete, connection encrypted (fingerprint: {}), peer version: {:?}",
        fp,
        peer_version
    );
    *state.fingerprint.write().await = Some(fp);

    // Remember credentials for FE Reconnect button. Only stored once the
    // handshake succeeds — we never retain creds for a connection that failed.
    *state.last_target.write().await = Some(target.to_string());
    *state.last_password.write().await = password.clone();

    let crypto = Arc::new(crypto);

    // Single unified outbound channel: pre-encoded frames (type byte + payload).
    let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // Separate request channel for forwarded browser API requests
    let (request_tx, mut request_rx) = mpsc::unbounded_channel::<ControlMessage>();

    let pending = Arc::new(Mutex::new(HashMap::<Uuid, ResponseChannel>::new()));
    let pending_order: PendingResponseOrder = Arc::new(Mutex::new(VecDeque::new()));
    let pending_read = pending.clone();
    let pending_order_read = pending_order.clone();
    let crypto_write = crypto.clone();
    let crypto_read = crypto.clone();
    let state_read = state.clone();
    let frame_tx_read = frame_tx.clone();

    // Request handler: tracks pending responses
    let frame_tx_request = frame_tx.clone();
    let request_handle = tokio::spawn(async move {
        while let Some(msg) = request_rx.recv().await {
            let json = serde_json::to_string(&msg).unwrap();
            let _ = frame_tx_request.send(encode_control_frame(json.as_bytes()));
        }
    });

    // Write task: encrypt each frame, send as binary WS frame
    let write_handle = tokio::spawn(async move {
        while let Some(frame) = frame_rx.recv().await {
            match crypto_write.encrypt(&frame) {
                Ok(ciphertext) => {
                    if ws_write
                        .send(Message::Binary(ciphertext.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!("Encryption failed: {}", e);
                    break;
                }
            }
        }
    });

    // Read task: decrypt each binary frame, dispatch by type byte
    let read_handle = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_read.next().await {
            match msg {
                Message::Binary(encrypted_data) => {
                    let plaintext = match crypto_read.decrypt(&encrypted_data) {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::error!("Decryption failed: {}", e);
                            break;
                        }
                    };

                    let (frame_type, payload) = match decode_frame_type(&plaintext) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::error!("Frame decode failed: {}", e);
                            break;
                        }
                    };

                    match frame_type {
                        FRAME_TYPE_DATA | FRAME_TYPE_DATA_V2 => {
                            match decode_data_frame(payload, frame_type) {
                                Ok((transfer_id, file_index, offset, chunk)) => {
                                    match state_read
                                        .transfer_receiver
                                        .receive_chunk(transfer_id, file_index, offset, chunk)
                                        .await
                                    {
                                        Ok(true) => {
                                            // Auto-finalized — send TransferFinalized back
                                            let msg = ControlMessage::TransferFinalized {
                                                id: transfer_id,
                                            };
                                            let json = serde_json::to_string(&msg).unwrap();
                                            let _ = frame_tx_read
                                                .send(encode_control_frame(json.as_bytes()));
                                        }
                                        Ok(false) => {}
                                        Err(e) => {
                                            tracing::error!("Failed to write chunk: {}", e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Failed to decode data frame: {}", e);
                                    break;
                                }
                            }
                        }
                        FRAME_TYPE_CONTROL => {
                            let control_msg =
                                match serde_json::from_slice::<ControlMessage>(payload) {
                                    Ok(m) => m,
                                    Err(e) => {
                                        tracing::error!("Failed to parse control message: {}", e);
                                        continue;
                                    }
                                };

                            if let ControlMessage::TransferComplete { id, total_bytes } =
                                control_msg
                            {
                                tracing::info!(
                                    "Received TransferComplete from server: {} ({} bytes)",
                                    id,
                                    total_bytes
                                );
                                match state_read
                                    .transfer_receiver
                                    .signal_completion(id, total_bytes)
                                    .await
                                {
                                    Ok(true) => {
                                        // Finalized — send TransferFinalized back
                                        let msg = ControlMessage::TransferFinalized { id };
                                        let json = serde_json::to_string(&msg).unwrap();
                                        let _ = frame_tx_read
                                            .send(encode_control_frame(json.as_bytes()));
                                    }
                                    Ok(false) => {
                                        // Waiting for remaining chunks; they will auto-finalize
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to signal completion: {}", e);
                                    }
                                }
                                continue;
                            }

                            if let ControlMessage::TransferFinalized { id } = control_msg {
                                tracing::info!("Received TransferFinalized: {}", id);
                                let mut pending = state_read.pending_completions.lock().await;
                                if let Some(tx) = pending.remove(&id) {
                                    let _ = tx.send(Ok(()));
                                }
                                continue;
                            }

                            if let ControlMessage::TransferError { id, ref error } = control_msg {
                                tracing::error!("Received TransferError for {}: {}", id, error);
                                if state_read.handle_transfer_error(id, error).await {
                                    continue;
                                }
                                // No active transfer or pending completion — likely a
                                // synchronous rejection of a TransferRequest. Fall through
                                // to the generic response handler below.
                            }

                            if control_msg.is_request() {
                                tracing::debug!(
                                    "Client handling request from server: {:?}",
                                    control_msg
                                );
                                if let Some(response) =
                                    handle_incoming_request(&state_read.clone(), control_msg).await
                                {
                                    let json = serde_json::to_string(&response).unwrap();
                                    let _ =
                                        frame_tx_read.send(encode_control_frame(json.as_bytes()));
                                }
                            } else {
                                let delivered = crate::server::deliver_pending_response(
                                    &pending_read,
                                    &pending_order_read,
                                    control_msg.clone(),
                                    peer_version,
                                )
                                .await;
                                if !delivered {
                                    tracing::warn!("Unhandled control message: {:?}", control_msg);
                                }
                            }
                        }
                        other => {
                            tracing::warn!("Unknown frame type: {:#x}", other);
                        }
                    }
                }
                // Handshake text frames only appear before encryption
                Message::Text(_) => {}
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    // Store remote connection info (with abort handles for all tasks)
    {
        let mut remote = state.remote.write().await;
        *remote = Some(RemoteConnection {
            instance_id: connection_id,
            hostname: target.to_string(),
            root_dir: "/".to_string(),
            tx: request_tx.clone(),
            pending_requests: pending.clone(),
            pending_request_order: pending_order.clone(),
            frame_tx: frame_tx.clone(),
            task_handles: vec![
                request_handle.abort_handle(),
                write_handle.abort_handle(),
                read_handle.abort_handle(),
            ],
            peer_version,
        });
    }
    let _ = state
        .browser_events
        .send(ControlMessage::ConnectionStatus { has_remote: true });

    // Send InfoRequest to get remote hostname and root_dir
    let request_id = Uuid::new_v4();
    let (info_tx, info_rx) = tokio::sync::oneshot::channel();
    crate::server::register_pending_response(
        &pending,
        &pending_order,
        request_id,
        info_tx,
        peer_version,
        true,
    )
    .await;
    if request_tx
        .send(ControlMessage::InfoRequest {
            request_id: Some(request_id),
        })
        .is_ok()
    {
        match tokio::time::timeout(std::time::Duration::from_secs(5), info_rx).await {
            Ok(Ok(ControlMessage::InfoResponse {
                hostname, root_dir, ..
            })) => {
                let mut remote = state.remote.write().await;
                if let Some(ref mut remote_conn) = *remote {
                    if remote_conn.instance_id == connection_id {
                        remote_conn.hostname = hostname;
                        remote_conn.root_dir = root_dir;
                    }
                }
            }
            _ => {
                let _ =
                    crate::server::remove_pending_response(&pending, &pending_order, request_id)
                        .await;
            }
        }
    } else {
        let _ = crate::server::remove_pending_response(&pending, &pending_order, request_id).await;
    }

    tokio::select! {
        _ = write_handle => {},
        _ = read_handle => {},
    }

    if crate::server::clear_remote_if_instance(&state, connection_id).await {
        let _ = state
            .browser_events
            .send(ControlMessage::ConnectionStatus { has_remote: false });
    }

    Ok(())
}

async fn handle_incoming_request(
    state: &Arc<AppState>,
    msg: ControlMessage,
) -> Option<ControlMessage> {
    match msg {
        ControlMessage::BrowseRequest { request_id, path } => {
            match crate::fileops::browse::list_directory(&state.config.root_dir, &path) {
                Ok(entries) => {
                    let cwd = state
                        .config
                        .root_dir
                        .join(&path)
                        .canonicalize()
                        .unwrap_or_else(|_| state.config.root_dir.clone())
                        .to_string_lossy()
                        .to_string();
                    Some(ControlMessage::BrowseResponse {
                        request_id,
                        hostname: state.config.hostname.clone(),
                        cwd,
                        entries,
                    })
                }
                Err(e) => Some(ControlMessage::Error {
                    request_id,
                    message: e.to_string(),
                }),
            }
        }
        ControlMessage::InfoRequest { request_id } => Some(ControlMessage::InfoResponse {
            request_id,
            hostname: state.config.hostname.clone(),
            root_dir: state.config.root_dir.to_string_lossy().to_string(),
        }),
        ControlMessage::TransferRequest {
            id,
            entries,
            direction,
            destination_path,
        } => {
            tracing::info!(
                "Client received TransferRequest from server: id={}, entries={}, direction={:?}, dest={}",
                id,
                entries.len(),
                direction,
                destination_path
            );

            use crate::protocol::messages::Direction;
            match direction {
                Direction::Push => {
                    state
                        .transfer_receiver
                        .start_transfer(id, entries, destination_path)
                        .await;
                    Some(ControlMessage::TransferAccepted {
                        id,
                        resume_offsets: std::collections::HashMap::new(),
                    })
                }
                Direction::Pull => {
                    tracing::info!(
                        "Accepting pull transfer from server, will send {} entries",
                        entries.len()
                    );

                    let (frame_tx, peer_version) = {
                        let remote = state.remote.read().await;
                        remote
                            .as_ref()
                            .map(|r| (r.frame_tx.clone(), r.peer_version))
                            .unzip()
                    };
                    let peer_version = peer_version.flatten();

                    let Some(frame_tx) = frame_tx else {
                        return Some(ControlMessage::TransferError {
                            id,
                            error: "No remote connection to send pull data".to_string(),
                        });
                    };

                    let root_dir = state.config.root_dir.clone();
                    tokio::spawn(async move {
                        crate::server::browser_transfer::send_entries(
                            &root_dir,
                            id,
                            &entries,
                            &frame_tx,
                            peer_version,
                        )
                        .await;
                    });

                    Some(ControlMessage::TransferAccepted {
                        id,
                        resume_offsets: std::collections::HashMap::new(),
                    })
                }
            }
        }
        ControlMessage::Ping { request_id } => Some(ControlMessage::Pong { request_id }),
        _ => None,
    }
}

pub async fn perform_client_handshake(
    ws_write: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    ws_read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    password: &Option<String>,
) -> anyhow::Result<(CryptoStream, String, Option<u32>)> {
    let client_keypair = KeyPair::generate();

    let (server_public, peer_version) = match ws_read.next().await {
        Some(Ok(Message::Text(text))) => {
            if let Ok(ControlMessage::KeyExchange {
                public_key,
                protocol_version,
            }) = serde_json::from_str(&text)
            {
                (decode_public_key(&public_key)?, protocol_version)
            } else {
                anyhow::bail!("Expected KeyExchange message from server");
            }
        }
        _ => anyhow::bail!("Failed to receive server public key"),
    };

    let msg = ControlMessage::KeyExchange {
        public_key: client_keypair.public_key_base64(),
        protocol_version: Some(CURRENT_PROTOCOL_VERSION),
    };
    let json = serde_json::to_string(&msg)?;
    ws_write.send(Message::Text(json.into())).await?;

    let shared_secret = derive_shared_secret(client_keypair.secret, &server_public);

    // Wait for either AuthChallenge (password required) or HandshakeComplete (no auth)
    match ws_read.next().await {
        Some(Ok(Message::Text(text))) => {
            let msg: ControlMessage = serde_json::from_str(&text)?;
            match msg {
                ControlMessage::AuthChallenge { nonce } => {
                    let password = password.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("Server requires a password (use --password)")
                    })?;

                    use crate::crypto::handshake::create_auth_proof;
                    use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

                    let nonce_bytes = BASE64.decode(&nonce)?;
                    let proof = create_auth_proof(password, &nonce_bytes, &shared_secret);
                    let response = ControlMessage::AuthResponse {
                        proof: BASE64.encode(&proof),
                    };
                    ws_write
                        .send(Message::Text(serde_json::to_string(&response)?.into()))
                        .await?;

                    // Now wait for HandshakeComplete or Error
                    match ws_read.next().await {
                        Some(Ok(Message::Text(text2))) => {
                            let msg2: ControlMessage = serde_json::from_str(&text2)?;
                            match msg2 {
                                ControlMessage::HandshakeComplete => {}
                                ControlMessage::Error { message, .. } => {
                                    anyhow::bail!("Authentication failed: {}", message);
                                }
                                _ => anyhow::bail!("Expected HandshakeComplete after auth"),
                            }
                        }
                        _ => anyhow::bail!("Connection closed during authentication"),
                    }
                }
                ControlMessage::HandshakeComplete => {
                    if password.is_some() {
                        tracing::warn!(
                            "Connected without authentication — server has no password set"
                        );
                    }
                }
                ControlMessage::Error { message, .. } => {
                    anyhow::bail!("Handshake error: {}", message);
                }
                _ => anyhow::bail!("Expected AuthChallenge or HandshakeComplete"),
            }
        }
        _ => anyhow::bail!("Failed to receive handshake message"),
    }

    let fp = crate::crypto::handshake::fingerprint(&shared_secret);
    Ok((
        CryptoStream::from_shared_secret(&shared_secret, false),
        fp,
        peer_version,
    ))
}
