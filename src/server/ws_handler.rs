use axum::{
    extract::{State, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::IntoResponse,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use crate::protocol::messages::ControlMessage;
use crate::protocol::codec::decode_data_frame;
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

    // Check if this is a server-to-server connection by attempting handshake
    // Send our public key first (in case this is a server connection)
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

    // Check if this is a KeyExchange response (server-to-server)
    if let Ok(ControlMessage::KeyExchange { public_key }) = serde_json::from_str(&first_msg) {
        // This is a server-to-server connection, complete handshake
        tracing::info!("Server-to-server connection detected, completing handshake");

        use crate::crypto::handshake::{decode_public_key, derive_shared_secret};
        use crate::crypto::stream::CryptoStream;

        let client_public = match decode_public_key(&public_key) {
            Ok(pk) => pk,
            Err(e) => {
                tracing::error!("Invalid public key: {}", e);
                return;
            }
        };

        let shared_secret = derive_shared_secret(server_keypair.secret, &client_public);
        let crypto = Arc::new(CryptoStream::from_shared_secret(&shared_secret, true));

        // Send handshake complete
        if sender.send(Message::Text(serde_json::to_string(&ControlMessage::HandshakeComplete).unwrap().into())).await.is_err() {
            return;
        }

        tracing::info!("Handshake complete, encrypted connection established");

        // Create bidirectional channel setup (similar to client)
        let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<ControlMessage>();
        let (binary_tx, mut binary_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (request_tx, mut request_rx) = mpsc::unbounded_channel::<(ControlMessage, ResponseChannel)>();

        let pending = Arc::new(Mutex::new(HashMap::<Uuid, ResponseChannel>::new()));
        let pending_request = pending.clone();
        let pending_read = pending.clone();

        let crypto_write = crypto.clone();
        let crypto_read = crypto.clone();
        let state_read = state.clone();
        let outgoing_tx_read = outgoing_tx.clone();

        // Spawn task to handle API requests (convert to outgoing messages with pending)
        let outgoing_for_request = outgoing_tx.clone();
        tokio::spawn(async move {
            while let Some((msg, response_tx)) = request_rx.recv().await {
                let id = Uuid::new_v4();
                pending_request.lock().await.insert(id, response_tx);
                let _ = outgoing_for_request.send(msg);
            }
        });

        // Spawn task to write outgoing messages (control + binary)
        let write_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(msg) = outgoing_rx.recv() => {
                        let json = serde_json::to_string(&msg).unwrap();
                        match crypto_write.encrypt(json.as_bytes()) {
                            Ok(ciphertext) => {
                                let encoded = BASE64.encode(&ciphertext);
                                if sender.send(Message::Text(encoded.into())).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::error!("Encryption failed: {}", e);
                                break;
                            }
                        }
                    }
                    Some(binary_data) = binary_rx.recv() => {
                        match crypto_write.encrypt(&binary_data) {
                            Ok(ciphertext) => {
                                if sender.send(Message::Binary(ciphertext.into())).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::error!("Binary encryption failed: {}", e);
                                break;
                            }
                        }
                    }
                    else => break,
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
                binary_tx: binary_tx.clone(),
                outgoing_tx: outgoing_tx.clone(),
            });
        }

        // Send InfoRequest to get client's info
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

        // Read messages from client
        let read_task = tokio::spawn(async move {
            while let Some(Ok(msg)) = receiver.next().await {
                match msg {
                    Message::Text(text) => {
                        let ciphertext = match BASE64.decode(text.as_bytes()) {
                            Ok(ct) => ct,
                            Err(e) => {
                                tracing::error!("Base64 decode failed: {}", e);
                                break;
                            }
                        };
                        match crypto_read.decrypt(&ciphertext) {
                            Ok(plaintext) => {
                                if let Ok(control_msg) = serde_json::from_str::<ControlMessage>(std::str::from_utf8(&plaintext).unwrap_or("")) {
                                    // Check if this is TransferComplete
                                    if let ControlMessage::TransferComplete { id } = control_msg {
                                        tracing::info!("Received TransferComplete: {}", id);
                                        if let Err(e) = state_read.transfer_receiver.finalize_transfer(id).await {
                                            tracing::error!("Failed to finalize transfer: {}", e);
                                        } else {
                                            tracing::info!("Transfer finalized successfully: {}", id);
                                        }
                                    }

                                    if control_msg.is_request() {
                                        // Handle incoming request from client
                                        tracing::debug!("Server handling request from client: {:?}", control_msg);
                                        if let Some(response) = handle_server_to_server_request(&state_read, control_msg).await {
                                            let _ = outgoing_tx_read.send(response);
                                        }
                                    } else {
                                        // This is a response to our request
                                        let mut pending_lock = pending_read.lock().await;
                                        if let Some(id) = pending_lock.keys().next().copied() {
                                            if let Some(response_tx) = pending_lock.remove(&id) {
                                                let _ = response_tx.send(control_msg);
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("Decryption failed: {}", e);
                                break;
                            }
                        }
                    }
                    Message::Binary(encrypted_data) => {
                        match crypto_read.decrypt(&encrypted_data) {
                            Ok(plaintext) => {
                                match decode_data_frame(&plaintext) {
                                    Ok((transfer_id, offset, chunk)) => {
                                        if let Err(e) = state_read.transfer_receiver.receive_chunk(transfer_id, offset, chunk).await {
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
                            Err(e) => {
                                tracing::error!("Binary decryption failed: {}", e);
                                break;
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        });

        // Wait for tasks
        tokio::select! {
            _ = write_task => {},
            _ = read_task => {},
        }

        // Clear remote on disconnect
        {
            let mut remote = state.remote.write().await;
            *remote = None;
        }

        tracing::info!("Server-to-server connection closed");
        return;
    }

    // This is a browser connection (plaintext)
    tracing::info!("Browser connection detected, using plaintext");

    // Create channel for outgoing messages (allows transfer handler to send updates)
    let (outgoing_tx, mut outgoing_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

    // Spawn task to send outgoing messages
    let write_task = tokio::spawn(async move {
        while let Some(msg) = outgoing_rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Handle the first message we already received
    if let Ok(control_msg) = serde_json::from_str::<ControlMessage>(&first_msg) {
        handle_browser_message(state.clone(), control_msg, outgoing_tx.clone()).await;
    }

    let state_clone = state.clone();
    // Handle remaining plaintext messages from browser
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

    // Wait for tasks
    tokio::select! {
        _ = write_task => {},
        _ = read_task => {},
    }

    tracing::info!("WebSocket connection closed");
}

// Handle messages from browser (may spawn async tasks for transfers)
async fn handle_browser_message(
    state: Arc<AppState>,
    msg: ControlMessage,
    ws_tx: mpsc::UnboundedSender<Message>,
) {
    tracing::debug!("Browser message received: {:?}", msg);
    match msg {
        ControlMessage::TransferRequest { id, entries, direction } => {
            tracing::info!("Browser TransferRequest: id={}, entries={}, direction={:?}", id, entries.len(), direction);
            // Spawn dedicated transfer task
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
            // Handle synchronous messages
            if let Some(response) = handle_control_message(&state, msg).await {
                let json = serde_json::to_string(&response).unwrap();
                let _ = ws_tx.send(Message::Text(json.into()));
            }
        }
    }
}

// Handle requests from server-to-server connection (always local, never forward)
async fn handle_server_to_server_request(
    state: &AppState,
    msg: ControlMessage,
) -> Option<ControlMessage> {
    match msg {
        ControlMessage::BrowseRequest { path } => {
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
                        hostname: state.config.hostname.clone(),
                        cwd,
                        entries,
                    })
                }
                Err(e) => Some(ControlMessage::Error {
                    message: e.to_string(),
                }),
            }
        }
        ControlMessage::InfoRequest => Some(ControlMessage::InfoResponse {
            hostname: state.config.hostname.clone(),
            root_dir: state.config.root_dir.to_string_lossy().to_string(),
        }),
        ControlMessage::TransferRequest { id, entries, direction } => {
            tracing::info!("Server received TransferRequest from client: id={}, entries={}, direction={:?}", id, entries.len(), direction);

            use crate::protocol::messages::Direction;
            match direction {
                Direction::Push => {
                    // Client wants to push files to us - accept and prepare to receive
                    tracing::info!("Accepting push transfer, preparing to receive {} files", entries.len());

                    // Initialize transfer receiver
                    state.transfer_receiver.start_transfer(id, entries.clone()).await;

                    Some(ControlMessage::TransferAccepted {
                        id,
                        resume_offsets: std::collections::HashMap::new(),
                    })
                }
                Direction::Pull => {
                    // Client wants to pull files from us - we need to send them
                    tracing::info!("Pull not yet implemented");
                    Some(ControlMessage::TransferError {
                        id,
                        error: "Pull transfer not yet implemented".to_string(),
                    })
                }
            }
        }
        ControlMessage::Ping => Some(ControlMessage::Pong),
        _ => None,
    }
}

// Handle requests from browser (forward to remote if available, otherwise handle locally)
async fn handle_control_message(
    state: &AppState,
    msg: ControlMessage,
) -> Option<ControlMessage> {
    // If we have a remote connection, forward all messages to it
    // (except InfoRequest which is always local)
    if !matches!(msg, ControlMessage::InfoRequest) {
        let remote = state.remote.read().await;
        if let Some(ref remote_conn) = *remote {
            // Create oneshot channel for response
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();

            // Forward to remote
            if remote_conn.tx.send((msg.clone(), response_tx)).is_ok() {
                // Wait for response with timeout
                match tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    response_rx
                ).await {
                    Ok(Ok(response)) => return Some(response),
                    Ok(Err(_)) => {
                        tracing::error!("Remote response channel closed");
                        return Some(ControlMessage::Error {
                            message: "Remote connection lost".to_string(),
                        });
                    }
                    Err(_) => {
                        tracing::error!("Remote response timeout");
                        return Some(ControlMessage::Error {
                            message: "Remote timeout".to_string(),
                        });
                    }
                }
            }
        }
    }

    // Handle locally if no remote or forward failed
    match msg {
        ControlMessage::BrowseRequest { path } => {
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
                        hostname: state.config.hostname.clone(),
                        cwd,
                        entries,
                    })
                }
                Err(e) => Some(ControlMessage::Error {
                    message: e.to_string(),
                }),
            }
        }
        ControlMessage::InfoRequest => Some(ControlMessage::InfoResponse {
            hostname: state.config.hostname.clone(),
            root_dir: state.config.root_dir.to_string_lossy().to_string(),
        }),
        ControlMessage::TransferRequest { id, entries, direction } => {
            tracing::info!("Received TransferRequest: id={}, entries={}, direction={:?}", id, entries.len(), direction);
            // TODO: Implement actual file transfer
            Some(ControlMessage::TransferError {
                id,
                error: "File transfer not yet implemented".to_string(),
            })
        }
        ControlMessage::Ping => Some(ControlMessage::Pong),
        _ => None,
    }
}
