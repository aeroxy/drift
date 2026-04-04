pub mod reconnect;
pub mod send;

use std::sync::Arc;
use std::collections::HashMap;
use crate::crypto::{handshake::{KeyPair, decode_public_key, derive_shared_secret}, stream::CryptoStream};
use crate::protocol::messages::ControlMessage;
use crate::protocol::codec::{
    decode_frame_type, decode_data_frame, encode_control_frame,
    FRAME_TYPE_DATA, FRAME_TYPE_CONTROL,
};
use crate::server::{AppState, RemoteConnection, ResponseChannel};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

pub async fn connect_to_remote(
    target: &str,
    password: &Option<String>,
    state: Arc<AppState>,
) -> anyhow::Result<()> {
    let url = format!("ws://{}/ws", target);
    tracing::info!("Connecting to remote: {}", url);

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await?;
    tracing::info!("Connected to remote: {}", target);

    let (mut ws_write, mut ws_read) = ws_stream.split();

    let crypto = perform_client_handshake(&mut ws_write, &mut ws_read, password).await?;
    tracing::info!("Handshake complete, connection encrypted");

    let crypto = Arc::new(crypto);

    // Single unified outbound channel: pre-encoded frames (type byte + payload).
    let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // Separate request channel for forwarded browser API requests
    let (request_tx, mut request_rx) = mpsc::unbounded_channel::<(ControlMessage, ResponseChannel)>();

    let pending = Arc::new(Mutex::new(HashMap::<Uuid, ResponseChannel>::new()));
    let pending_request = pending.clone();
    let pending_read = pending.clone();
    let crypto_write = crypto.clone();
    let crypto_read = crypto.clone();
    let state_read = state.clone();
    let frame_tx_read = frame_tx.clone();

    // Request handler: tracks pending responses
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
                    if ws_write.send(Message::Binary(ciphertext.into())).await.is_err() {
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
    let read_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_read.next().await {
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
                                tracing::info!("Received TransferComplete from server: {} ({} bytes)", id, total_bytes);
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
                                tracing::debug!("Client handling request from server: {:?}", control_msg);
                                if let Some(response) = handle_incoming_request(&state_read.clone(), control_msg).await {
                                    let json = serde_json::to_string(&response).unwrap();
                                    let _ = frame_tx_read.send(encode_control_frame(json.as_bytes()));
                                }
                            } else {
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
                // Handshake text frames only appear before encryption
                Message::Text(_) => {}
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    // Store remote connection info
    {
        let mut remote = state.remote.write().await;
        *remote = Some(RemoteConnection {
            hostname: target.to_string(),
            root_dir: "/".to_string(),
            tx: request_tx.clone(),
            frame_tx: frame_tx.clone(),
        });
    }
    let _ = state.browser_events.send(ControlMessage::ConnectionStatus { has_remote: true });

    // Send InfoRequest to get remote hostname and root_dir
    let (info_tx, info_rx) = tokio::sync::oneshot::channel();
    if request_tx.send((ControlMessage::InfoRequest, info_tx)).is_ok() {
        if let Ok(Ok(ControlMessage::InfoResponse { hostname, root_dir })) =
            tokio::time::timeout(std::time::Duration::from_secs(5), info_rx).await
        {
            let mut remote = state.remote.write().await;
            if let Some(ref mut remote_conn) = *remote {
                remote_conn.hostname = hostname;
                remote_conn.root_dir = root_dir;
            }
        }
    }

    tokio::select! {
        _ = write_task => {},
        _ = read_task => {},
    }

    {
        let mut remote = state.remote.write().await;
        *remote = None;
    }
    let _ = state.browser_events.send(ControlMessage::ConnectionStatus { has_remote: false });

    Ok(())
}

async fn handle_incoming_request(
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
            tracing::info!("Client received TransferRequest from server: id={}, entries={}, direction={:?}", id, entries.len(), direction);

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
                    tracing::info!("Accepting pull transfer from server, will send {} entries", entries.len());

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
                        crate::server::browser_transfer::send_entries(
                            &root_dir, id, &entries, &frame_tx,
                        ).await;
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

pub async fn perform_client_handshake(
    ws_write: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>,
    ws_read: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    password: &Option<String>,
) -> anyhow::Result<CryptoStream> {
    let client_keypair = KeyPair::generate();

    let server_public = match ws_read.next().await {
        Some(Ok(Message::Text(text))) => {
            if let Ok(ControlMessage::KeyExchange { public_key }) = serde_json::from_str(&text) {
                decode_public_key(&public_key)?
            } else {
                anyhow::bail!("Expected KeyExchange message from server");
            }
        }
        _ => anyhow::bail!("Failed to receive server public key"),
    };

    let msg = ControlMessage::KeyExchange {
        public_key: client_keypair.public_key_base64(),
    };
    let json = serde_json::to_string(&msg)?;
    ws_write.send(Message::Text(json.into())).await?;

    let shared_secret = derive_shared_secret(client_keypair.secret, &server_public);

    if password.is_some() {
        tracing::warn!("Password authentication not yet implemented");
    }

    match ws_read.next().await {
        Some(Ok(Message::Text(text))) => {
            if !matches!(serde_json::from_str::<ControlMessage>(&text)?, ControlMessage::HandshakeComplete) {
                anyhow::bail!("Expected HandshakeComplete message");
            }
        }
        _ => anyhow::bail!("Failed to receive HandshakeComplete"),
    }

    Ok(CryptoStream::from_shared_secret(&shared_secret, false))
}
