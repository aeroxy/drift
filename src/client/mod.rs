pub mod reconnect;
pub mod send;

use std::sync::Arc;
use std::collections::HashMap;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use crate::crypto::{handshake::{KeyPair, decode_public_key, derive_shared_secret}, stream::CryptoStream};
use crate::protocol::messages::ControlMessage;
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

    // Perform encryption handshake
    let crypto = perform_client_handshake(&mut ws_write, &mut ws_read, password).await?;
    tracing::info!("Handshake complete, connection encrypted");

    let crypto = Arc::new(crypto);

    // Create channel for outgoing control messages (both requests and responses)
    let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<ControlMessage>();

    // Create channel for outgoing binary data
    let (binary_tx, mut binary_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // Create channel for API requests (from app to remote)
    let (request_tx, mut request_rx) = mpsc::unbounded_channel::<(ControlMessage, ResponseChannel)>();

    // Map to track pending responses
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

    // Spawn task to write outgoing messages to WS (both control and binary)
    let write_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                // biased: binary data always takes priority over control messages
                // so that TransferComplete is never sent before pending binary chunks.
                biased;
                Some(binary_data) = binary_rx.recv() => {
                    // Send binary data (encrypted binary frame)
                    match crypto_write.encrypt(&binary_data) {
                        Ok(ciphertext) => {
                            if ws_write.send(Message::Binary(ciphertext.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::error!("Binary encryption failed: {}", e);
                            break;
                        }
                    }
                }
                Some(msg) = outgoing_rx.recv() => {
                    // Send control message (encrypted text frame)
                    let json = serde_json::to_string(&msg).unwrap();
                    match crypto_write.encrypt(json.as_bytes()) {
                        Ok(ciphertext) => {
                            let encoded = BASE64.encode(&ciphertext);
                            if ws_write.send(Message::Text(encoded.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::error!("Encryption failed: {}", e);
                            break;
                        }
                    }
                }
                else => break,
            }
        }
    });

    // Read messages from WS (both responses to our requests and incoming requests)
    let read_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_read.next().await {
            match msg {
                Message::Text(text) => {
                    // Decode and decrypt message
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
                                    tracing::info!("Received TransferComplete from server: {}", id);
                                    if let Err(e) = state_read.transfer_receiver.finalize_transfer(id).await {
                                        tracing::error!("Failed to finalize transfer: {}", e);
                                    } else {
                                        tracing::info!("Transfer finalized successfully: {}", id);
                                    }
                                }

                                if control_msg.is_request() {
                                    // Handle incoming request from server
                                    tracing::debug!("Client handling request from server: {:?}", control_msg);
                                    if let Some(response) = handle_incoming_request(&state_read, control_msg).await {
                                        // Send response back via outgoing channel
                                        let _ = outgoing_tx_read.send(response);
                                    }
                                } else {
                                    // This is a response to our request - forward to pending channel
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
                    // Decrypt binary frame
                    match crypto_read.decrypt(&encrypted_data) {
                        Ok(plaintext) => {
                            // Decode data frame: [16B UUID][8B offset][chunk data]
                            use crate::protocol::codec::decode_data_frame;
                            match decode_data_frame(&plaintext) {
                                Ok((transfer_id, offset, chunk)) => {
                                    // Write chunk to disk
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

    // Store remote connection info
    {
        let mut remote = state.remote.write().await;
        *remote = Some(RemoteConnection {
            hostname: target.to_string(),
            root_dir: "/".to_string(),
            tx: request_tx.clone(),
            binary_tx: binary_tx.clone(),
            outgoing_tx: outgoing_tx.clone(),
        });
    }

    // Send initial InfoRequest to get remote hostname and root_dir
    let (info_tx, info_rx) = tokio::sync::oneshot::channel();
    if request_tx.send((ControlMessage::InfoRequest, info_tx)).is_ok() {
        if let Ok(Ok(ControlMessage::InfoResponse { hostname, root_dir })) =
            tokio::time::timeout(std::time::Duration::from_secs(5), info_rx).await
        {
            // Update remote connection with real info
            let mut remote = state.remote.write().await;
            if let Some(ref mut remote_conn) = *remote {
                remote_conn.hostname = hostname;
                remote_conn.root_dir = root_dir;
            }
        }
    }

    // Wait for tasks (they run forever until connection drops)
    tokio::select! {
        _ = write_task => {},
        _ = read_task => {},
    }

    // Clear remote on disconnect
    {
        let mut remote = state.remote.write().await;
        *remote = None;
    }

    Ok(())
}

async fn handle_incoming_request(
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
            tracing::info!("Client received TransferRequest from server: id={}, entries={}, direction={:?}", id, entries.len(), direction);

            use crate::protocol::messages::Direction;
            match direction {
                Direction::Push => {
                    // Server wants to push files to us - accept and prepare to receive
                    tracing::info!("Accepting push transfer from server, preparing to receive {} files", entries.len());

                    // Initialize transfer receiver
                    state.transfer_receiver.start_transfer(id, entries.clone()).await;

                    Some(ControlMessage::TransferAccepted {
                        id,
                        resume_offsets: std::collections::HashMap::new(),
                    })
                }
                Direction::Pull => {
                    // Server wants to pull files from us - we need to send them
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

pub async fn perform_client_handshake(
    ws_write: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>,
    ws_read: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    password: &Option<String>,
) -> anyhow::Result<CryptoStream> {
    // Generate client keypair
    let client_keypair = KeyPair::generate();

    // Receive server public key first (server sends first)
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

    // Send client public key
    let msg = ControlMessage::KeyExchange {
        public_key: client_keypair.public_key_base64(),
    };
    let json = serde_json::to_string(&msg)?;
    ws_write.send(Message::Text(json.into())).await?;

    // Derive shared secret
    let shared_secret = derive_shared_secret(client_keypair.secret, &server_public);

    // TODO: Implement password authentication if provided
    if password.is_some() {
        tracing::warn!("Password authentication not yet implemented");
    }

    // Wait for handshake complete
    match ws_read.next().await {
        Some(Ok(Message::Text(text))) => {
            if !matches!(serde_json::from_str::<ControlMessage>(&text)?, ControlMessage::HandshakeComplete) {
                anyhow::bail!("Expected HandshakeComplete message");
            }
        }
        _ => anyhow::bail!("Failed to receive HandshakeComplete"),
    }

    // Create crypto stream (client = false)
    Ok(CryptoStream::from_shared_secret(&shared_secret, false))
}
