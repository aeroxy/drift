use axum::{
    extract::{State, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use crate::protocol::messages::ControlMessage;
use crate::protocol::codec::{
    decode_frame_type, decode_data_frame, encode_control_frame,
    FRAME_TYPE_DATA, FRAME_TYPE_CONTROL,
};
use crate::server::{AppState, browser_transfer, RemoteConnection, ResponseChannel};

pub async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_connection(socket, state))
}

async fn handle_connection(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();

    tracing::info!("New WebSocket connection");

    // Send our public key first (server-to-server probe)
    use crate::crypto::handshake::KeyPair;
    let server_keypair = KeyPair::generate();
    let key_exchange = ControlMessage::KeyExchange {
        public_key: server_keypair.public_key_base64(),
    };
    if sender.send(Message::Text(serde_json::to_string(&key_exchange).unwrap().into())).await.is_err() {
        return;
    }

    // Wait for first message to determine connection type
    let first_msg = match receiver.next().await {
        Some(Ok(Message::Text(text))) => text,
        _ => return,
    };

    // KeyExchange response → server-to-server encrypted connection
    if let Ok(ControlMessage::KeyExchange { public_key }) = serde_json::from_str(&first_msg) {
        tracing::info!("Server-to-server connection detected, completing handshake");

        use crate::crypto::handshake::{decode_public_key, derive_shared_secret, fingerprint, generate_nonce, verify_auth_proof};
        use crate::crypto::stream::CryptoStream;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

        let client_public = match decode_public_key(&public_key) {
            Ok(pk) => pk,
            Err(e) => { tracing::error!("Invalid public key: {}", e); return; }
        };

        let shared_secret = derive_shared_secret(server_keypair.secret, &client_public);
        let crypto = Arc::new(CryptoStream::from_shared_secret(&shared_secret, true));

        // Password authentication: if configured, challenge the client
        if let Some(ref password) = state.config.password {
            let nonce = generate_nonce();
            let challenge = ControlMessage::AuthChallenge {
                nonce: BASE64.encode(&nonce),
            };
            if sender.send(Message::Text(serde_json::to_string(&challenge).unwrap().into())).await.is_err() {
                return;
            }

            let proof_bytes = match receiver.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(ControlMessage::AuthResponse { proof }) = serde_json::from_str(&text) {
                        match BASE64.decode(&proof) {
                            Ok(bytes) => bytes,
                            Err(_) => {
                                tracing::error!("Invalid auth proof encoding");
                                let _ = sender.send(Message::Text(serde_json::to_string(&ControlMessage::Error { message: "authentication failed".into() }).unwrap().into())).await;
                                return;
                            }
                        }
                    } else {
                        tracing::error!("Expected AuthResponse, got: {}", &text[..text.len().min(100)]);
                        let _ = sender.send(Message::Text(serde_json::to_string(&ControlMessage::Error { message: "authentication failed".into() }).unwrap().into())).await;
                        return;
                    }
                }
                _ => {
                    tracing::error!("Connection closed during authentication");
                    return;
                }
            };

            if !verify_auth_proof(password, &nonce, &shared_secret, &proof_bytes) {
                tracing::error!("Authentication failed: invalid password");
                let _ = sender.send(Message::Text(serde_json::to_string(&ControlMessage::Error { message: "authentication failed".into() }).unwrap().into())).await;
                return;
            }

            tracing::info!("Password authentication successful");
        }

        if sender.send(Message::Text(serde_json::to_string(&ControlMessage::HandshakeComplete).unwrap().into())).await.is_err() {
            return;
        }

        let fp = fingerprint(&shared_secret);
        tracing::info!("Handshake complete, encrypted connection established (fingerprint: {})", fp);
        *state.fingerprint.write().await = Some(fp);

        // Single unified outbound channel: pre-encoded frames (type byte + payload).
        // All outbound messages — data chunks AND control messages — go through this
        // FIFO queue. The write task encrypts each frame and sends as a binary WS frame.
        let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        // Separate request channel for browser-forwarded requests needing responses
        let (request_tx, mut request_rx) = mpsc::unbounded_channel::<(ControlMessage, ResponseChannel)>();

        let pending = Arc::new(Mutex::new(HashMap::<Uuid, ResponseChannel>::new()));
        let pending_request = pending.clone();
        let pending_read = pending.clone();

        let crypto_write = crypto.clone();
        let crypto_read = crypto.clone();
        let state_read = state.clone();
        let frame_tx_read = frame_tx.clone();

        // Request handler: routes browser API requests to remote, tracks pending responses
        let frame_tx_request = frame_tx.clone();
        tokio::spawn(async move {
            while let Some((msg, response_tx)) = request_rx.recv().await {
                let id = Uuid::new_v4();
                pending_request.lock().await.insert(id, response_tx);
                let json = serde_json::to_string(&msg).unwrap();
                let _ = frame_tx_request.send(encode_control_frame(json.as_bytes()));
            }
        });

        // Write task: encrypt each frame, send as binary WS frame
        let write_task = tokio::spawn(async move {
            while let Some(frame) = frame_rx.recv().await {
                match crypto_write.encrypt(&frame) {
                    Ok(ciphertext) => {
                        if sender.send(Message::Binary(ciphertext.into())).await.is_err() {
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

        // Store remote connection
        {
            let mut remote = state.remote.write().await;
            *remote = Some(RemoteConnection {
                hostname: "remote".to_string(),
                root_dir: "/".to_string(),
                tx: request_tx.clone(),
                frame_tx: frame_tx.clone(),
            });
        }
        let _ = state.browser_events.send(ControlMessage::ConnectionStatus { has_remote: true });

        // Send InfoRequest to get client's hostname and root_dir
        let (info_tx, info_rx) = tokio::sync::oneshot::channel();
        if request_tx.send((ControlMessage::InfoRequest, info_tx)).is_ok() {
            let state_clone = state.clone();
            tokio::spawn(async move {
                if let Ok(Ok(ControlMessage::InfoResponse { hostname, root_dir })) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), info_rx).await
                {
                    let mut remote = state_clone.remote.write().await;
                    if let Some(ref mut remote_conn) = *remote {
                        remote_conn.hostname = hostname;
                        remote_conn.root_dir = root_dir;
                    }
                }
            });
        }

        // Read task: decrypt each binary frame, dispatch by type byte
        let read_task = tokio::spawn(async move {
            while let Some(Ok(msg)) = receiver.next().await {
                match msg {
                    Message::Binary(encrypted_data) => {
                        let plaintext = match crypto_read.decrypt(&encrypted_data) {
                            Ok(p) => p,
                            Err(e) => { tracing::error!("Decryption failed: {}", e); break; }
                        };

                        let (frame_type, payload) = match decode_frame_type(&plaintext) {
                            Ok(v) => v,
                            Err(e) => { tracing::error!("Frame decode failed: {}", e); break; }
                        };

                        match frame_type {
                            FRAME_TYPE_DATA => {
                                match decode_data_frame(payload) {
                                    Ok((transfer_id, offset, chunk)) => {
                                        match state_read.transfer_receiver
                                            .receive_chunk(transfer_id, offset, chunk).await
                                        {
                                            Ok(true) => {
                                                // Auto-finalized — send TransferFinalized back
                                                let msg = ControlMessage::TransferFinalized { id: transfer_id };
                                                let json = serde_json::to_string(&msg).unwrap();
                                                let _ = frame_tx_read.send(encode_control_frame(json.as_bytes()));
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
                                let control_msg = match serde_json::from_slice::<ControlMessage>(payload) {
                                    Ok(m) => m,
                                    Err(e) => {
                                        tracing::error!("Failed to parse control message: {}", e);
                                        continue;
                                    }
                                };

                                if let ControlMessage::TransferComplete { id, total_bytes } = control_msg {
                                    tracing::info!("Received TransferComplete: {} ({} bytes)", id, total_bytes);
                                    match state_read.transfer_receiver.signal_completion(id, total_bytes).await {
                                        Ok(true) => {
                                            // Finalized — send TransferFinalized back
                                            let msg = ControlMessage::TransferFinalized { id };
                                            let json = serde_json::to_string(&msg).unwrap();
                                            let _ = frame_tx_read.send(encode_control_frame(json.as_bytes()));
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
                                        let _ = tx.send(());
                                    }
                                    continue;
                                }

                                if control_msg.is_request() {
                                    tracing::debug!("Server handling request from client: {:?}", control_msg);
                                    if let Some(response) = handle_server_to_server_request(&state_read.clone(), control_msg).await {
                                        let json = serde_json::to_string(&response).unwrap();
                                        let _ = frame_tx_read.send(encode_control_frame(json.as_bytes()));
                                    }
                                } else {
                                    // Response to one of our outgoing requests
                                    let mut pending_lock = pending_read.lock().await;
                                    if let Some(id) = pending_lock.keys().next().copied() {
                                        if let Some(response_tx) = pending_lock.remove(&id) {
                                            let _ = response_tx.send(control_msg);
                                        }
                                    }
                                }
                            }
                            other => {
                                tracing::warn!("Unknown frame type: {:#x}", other);
                            }
                        }
                    }
                    // Handshake text frames only come before encryption — ignore any after
                    Message::Text(_) => {}
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        });

        tokio::select! {
            _ = write_task => {},
            _ = read_task => {},
        }

        {
            let mut remote = state.remote.write().await;
            *remote = None;
        }
        *state.fingerprint.write().await = None;
        let _ = state.browser_events.send(ControlMessage::ConnectionStatus { has_remote: false });

        tracing::info!("Server-to-server connection closed");
        return;
    }

    // ── Browser connection (plaintext) ─────────────────────────────────────────
    tracing::info!("Browser connection detected, using plaintext");

    let (outgoing_tx, mut outgoing_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

    let write_task = tokio::spawn(async move {
        while let Some(msg) = outgoing_rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    if let Ok(control_msg) = serde_json::from_str::<ControlMessage>(&first_msg) {
        handle_browser_message(state.clone(), control_msg, outgoing_tx.clone()).await;
    }

    // Subscribe to broadcast events and forward to this browser
    let mut event_rx = state.browser_events.subscribe();
    let outgoing_for_events = outgoing_tx.clone();
    let event_task = tokio::spawn(async move {
        while let Ok(msg) = event_rx.recv().await {
            let json = serde_json::to_string(&msg).unwrap();
            if outgoing_for_events.send(Message::Text(json.into())).is_err() {
                break;
            }
        }
    });

    let state_clone = state.clone();
    let read_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    if let Ok(control_msg) = serde_json::from_str::<ControlMessage>(&text) {
                        handle_browser_message(state_clone.clone(), control_msg, outgoing_tx.clone()).await;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = write_task => {},
        _ = read_task => {},
        _ = event_task => {},
    }

    tracing::info!("WebSocket connection closed");
}

async fn handle_browser_message(
    state: Arc<AppState>,
    msg: ControlMessage,
    ws_tx: mpsc::UnboundedSender<Message>,
) {
    tracing::debug!("Browser message received: {:?}", msg);
    match msg {
        ControlMessage::TransferRequest { id, entries, direction, destination_path } => {
            tracing::info!("Browser TransferRequest: id={}, entries={}, direction={:?}, dest={}", id, entries.len(), direction, destination_path);
            tokio::spawn(async move {
                browser_transfer::handle_browser_transfer(
                    state,
                    id,
                    entries,
                    direction,
                    destination_path,
                    ws_tx,
                ).await;
            });
        }
        _ => {
            if let Some(response) = handle_control_message(&state, msg).await {
                let json = serde_json::to_string(&response).unwrap();
                let _ = ws_tx.send(Message::Text(json.into()));
            }
        }
    }
}

/// Handle requests from the remote server (server-to-server). Never forwards — always local.
async fn handle_server_to_server_request(
    state: &Arc<AppState>,
    msg: ControlMessage,
) -> Option<ControlMessage> {
    match msg {
        ControlMessage::BrowseRequest { path } => {
            match crate::fileops::browse::list_directory(&state.config.root_dir, &path) {
                Ok(entries) => {
                    let cwd = state.config.root_dir.join(&path)
                        .canonicalize()
                        .unwrap_or_else(|_| state.config.root_dir.clone())
                        .to_string_lossy().to_string();
                    Some(ControlMessage::BrowseResponse {
                        hostname: state.config.hostname.clone(),
                        cwd,
                        entries,
                    })
                }
                Err(e) => Some(ControlMessage::Error { message: e.to_string() }),
            }
        }
        ControlMessage::InfoRequest => Some(ControlMessage::InfoResponse {
            hostname: state.config.hostname.clone(),
            root_dir: state.config.root_dir.to_string_lossy().to_string(),
        }),
        ControlMessage::TransferRequest { id, entries, direction, destination_path } => {
            tracing::info!("Server received TransferRequest from client: id={}, entries={}, direction={:?}, dest={}", id, entries.len(), direction, destination_path);

            use crate::protocol::messages::Direction;
            match direction {
                Direction::Push => {
                    tracing::info!("Accepting push transfer, preparing to receive {} files", entries.len());
                    state.transfer_receiver.start_transfer(id, entries.clone(), destination_path).await;
                    Some(ControlMessage::TransferAccepted {
                        id,
                        resume_offsets: std::collections::HashMap::new(),
                    })
                }
                Direction::Pull => {
                    tracing::info!("Accepting pull transfer, will send {} entries", entries.len());

                    let frame_tx = {
                        let remote = state.remote.read().await;
                        remote.as_ref().map(|r| r.frame_tx.clone())
                    };

                    let Some(frame_tx) = frame_tx else {
                        return Some(ControlMessage::TransferError {
                            id,
                            error: "No remote connection to send pull data".to_string(),
                        });
                    };

                    let root_dir = state.config.root_dir.clone();
                    tokio::spawn(async move {
                        browser_transfer::send_entries(&root_dir, id, &entries, &frame_tx).await;
                    });

                    Some(ControlMessage::TransferAccepted {
                        id,
                        resume_offsets: std::collections::HashMap::new(),
                    })
                }
            }
        }
        ControlMessage::Ping => Some(ControlMessage::Pong),
        _ => None,
    }
}

/// Handle requests from browser (forward to remote if available, else local).
/// BrowseRequest is NOT handled locally when there is no remote — it returns an error
/// to prevent the remote panel from mirroring local files.
async fn handle_control_message(
    state: &AppState,
    msg: ControlMessage,
) -> Option<ControlMessage> {
    if !matches!(msg, ControlMessage::InfoRequest) {
        let remote = state.remote.read().await;
        if let Some(ref remote_conn) = *remote {
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();
            if remote_conn.tx.send((msg.clone(), response_tx)).is_ok() {
                match tokio::time::timeout(std::time::Duration::from_secs(10), response_rx).await {
                    Ok(Ok(response)) => return Some(response),
                    Ok(Err(_)) => return Some(ControlMessage::Error {
                        message: "Remote connection lost".to_string(),
                    }),
                    Err(_) => return Some(ControlMessage::Error {
                        message: "Remote timeout".to_string(),
                    }),
                }
            }
            // tx.send failed: channel closed
            return Some(ControlMessage::Error {
                message: "Remote connection lost".to_string(),
            });
        }

        // No remote — BrowseRequest must not fall through to local handling
        if matches!(msg, ControlMessage::BrowseRequest { .. }) {
            return Some(ControlMessage::Error {
                message: "No remote connection".to_string(),
            });
        }
    }

    match msg {
        ControlMessage::BrowseRequest { path } => {
            match crate::fileops::browse::list_directory(&state.config.root_dir, &path) {
                Ok(entries) => {
                    let cwd = state.config.root_dir.join(&path)
                        .canonicalize()
                        .unwrap_or_else(|_| state.config.root_dir.clone())
                        .to_string_lossy().to_string();
                    Some(ControlMessage::BrowseResponse {
                        hostname: state.config.hostname.clone(),
                        cwd,
                        entries,
                    })
                }
                Err(e) => Some(ControlMessage::Error { message: e.to_string() }),
            }
        }
        ControlMessage::InfoRequest => Some(ControlMessage::InfoResponse {
            hostname: state.config.hostname.clone(),
            root_dir: state.config.root_dir.to_string_lossy().to_string(),
        }),
        ControlMessage::Ping => Some(ControlMessage::Pong),
        _ => None,
    }
}
