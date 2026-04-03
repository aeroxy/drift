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

        use crate::crypto::handshake::{decode_public_key, derive_shared_secret};
        use crate::crypto::stream::CryptoStream;

        let client_public = match decode_public_key(&public_key) {
            Ok(pk) => pk,
            Err(e) => { tracing::error!("Invalid public key: {}", e); return; }
        };

        let shared_secret = derive_shared_secret(server_keypair.secret, &client_public);
        let crypto = Arc::new(CryptoStream::from_shared_secret(&shared_secret, true));

        if sender.send(Message::Text(serde_json::to_string(&ControlMessage::HandshakeComplete).unwrap().into())).await.is_err() {
            return;
        }

        tracing::info!("Handshake complete, encrypted connection established");

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
                                        if let Err(e) = state_read.transfer_receiver
                                            .receive_chunk(transfer_id, offset, chunk).await
                                        {
                                            tracing::error!("Failed to write chunk: {}", e);
                                            break;
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
                                if let ControlMessage::TransferComplete { id } = control_msg {
                                    tracing::info!("Received TransferComplete: {}", id);
                                    if let Err(e) = state_read.transfer_receiver.finalize_transfer(id).await {
                                        tracing::error!("Failed to finalize transfer: {}", e);
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
        ControlMessage::TransferRequest { id, entries, direction } => {
            tracing::info!("Browser TransferRequest: id={}, entries={}, direction={:?}", id, entries.len(), direction);
            tokio::spawn(async move {
                browser_transfer::handle_browser_transfer(
                    state,
                    id,
                    entries,
                    direction,
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
        ControlMessage::TransferRequest { id, entries, direction } => {
            tracing::info!("Server received TransferRequest: id={}, entries={}, direction={:?}", id, entries.len(), direction);

            use crate::protocol::messages::Direction;
            match direction {
                Direction::Push => {
                    state.transfer_receiver.start_transfer(id, entries.clone()).await;
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

/// Handle requests from browser (forward to remote if available, otherwise local).
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
