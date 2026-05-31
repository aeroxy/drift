pub mod browser_transfer;
pub mod file_api;
pub mod transfer_handler;
pub mod transfer_receiver;
pub mod ws_handler;

use crate::config::AppConfig;
use crate::frontend::static_handler;
use crate::protocol::messages::{ControlMessage, MIN_REQUEST_ID_VERSION};
use axum::{Router, routing::get};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

pub struct AppState {
    pub config: AppConfig,
    pub remote: RwLock<Option<RemoteConnection>>,
    pub transfer_receiver: transfer_receiver::TransferReceiver,
    /// Oneshot channels fired when a remote confirms TransferFinalized or sends TransferError.
    /// push_entries registers here before sending TransferComplete.
    pub pending_completions: Mutex<HashMap<Uuid, oneshot::Sender<Result<(), String>>>>,
    /// Broadcast channel for pushing events (ConnectionStatus etc.) to all browsers.
    pub browser_events: broadcast::Sender<ControlMessage>,
    /// Short hex fingerprint of the DH shared secret (for visual MITM verification).
    pub fingerprint: RwLock<Option<String>>,
    /// Most recent successfully-connected target and password, retained across
    /// unexpected disconnects so the FE can offer a Reconnect button. Cleared
    /// only on intentional disconnect (`/api/disconnect`).
    pub last_target: RwLock<Option<String>>,
    pub last_password: RwLock<Option<String>>,
}

pub type ResponseChannel = oneshot::Sender<ControlMessage>;
pub type PendingResponses = Arc<Mutex<HashMap<Uuid, ResponseChannel>>>;
pub type PendingResponseOrder = Arc<Mutex<VecDeque<Uuid>>>;
pub type RequestChannel = mpsc::UnboundedSender<ControlMessage>;

/// Unified outgoing channel. Carries pre-encoded frames (type byte + payload, NOT yet
/// encrypted). Both data chunks (`encode_data_frame`) and control messages
/// (`encode_control_frame`) travel through this single FIFO queue, preserving send
/// order without priority starvation.
pub type FrameChannel = mpsc::UnboundedSender<Vec<u8>>;

pub struct RemoteConnection {
    /// Unique instance id for this live connection. Used to prevent stale tasks
    /// from clearing or restoring over a newer remote connection.
    pub instance_id: Uuid,
    pub hostname: String,
    pub root_dir: String,
    /// For browser-initiated requests that need a response (e.g. BrowseRequest).
    pub tx: RequestChannel,
    /// Pending request/response channels keyed by protocol correlation id.
    pub pending_requests: PendingResponses,
    /// FIFO registration order for peers that do not echo correlation ids.
    pub pending_request_order: PendingResponseOrder,
    /// Unified outbound: send pre-encoded frames via `encode_data_frame` or
    /// `encode_control_frame` from `crate::protocol::codec`.
    pub frame_tx: FrameChannel,
    /// Abort handles for all tasks driving this connection (read, write, request handler).
    /// Aborting these cleanly tears down the connection from either side.
    pub task_handles: Vec<tokio::task::AbortHandle>,
    /// Protocol version of the peer (None if unknown/old version).
    pub peer_version: Option<u32>,
}

pub async fn abort_remote_connection(conn: RemoteConnection) {
    let mut pending = conn.pending_requests.lock().await;
    for (request_id, tx) in pending.drain() {
        let _ = tx.send(ControlMessage::Error {
            request_id: Some(request_id),
            message: "Remote connection lost".to_string(),
        });
    }
    drop(pending);
    conn.pending_request_order.lock().await.clear();

    for handle in conn.task_handles {
        handle.abort();
    }
}

fn peer_supports_request_ids(peer_version: Option<u32>) -> bool {
    peer_version >= Some(MIN_REQUEST_ID_VERSION)
}

pub async fn register_pending_response(
    pending: &PendingResponses,
    pending_order: &PendingResponseOrder,
    request_id: Uuid,
    response_tx: ResponseChannel,
    peer_version: Option<u32>,
    use_fifo_fallback: bool,
) {
    pending.lock().await.insert(request_id, response_tx);
    if use_fifo_fallback && !peer_supports_request_ids(peer_version) {
        pending_order.lock().await.push_back(request_id);
    }
}

pub async fn remove_pending_response(
    pending: &PendingResponses,
    pending_order: &PendingResponseOrder,
    request_id: Uuid,
) -> Option<ResponseChannel> {
    let removed = pending.lock().await.remove(&request_id);
    if removed.is_some() {
        let mut order = pending_order.lock().await;
        if let Some(pos) = order.iter().position(|queued_id| *queued_id == request_id) {
            order.remove(pos);
        }
    }
    removed
}

pub async fn deliver_pending_response(
    pending: &PendingResponses,
    pending_order: &PendingResponseOrder,
    control_msg: ControlMessage,
    peer_version: Option<u32>,
) -> bool {
    if let Some(response_id) = control_msg.response_id() {
        let Some(response_tx) =
            remove_pending_response(pending, pending_order, response_id).await
        else {
            return false;
        };
        let _ = response_tx.send(control_msg);
        return true;
    };

    if peer_supports_request_ids(peer_version) {
        return false;
    }

    loop {
        let next_request_id = {
            let mut order = pending_order.lock().await;
            order.pop_front()
        };

        let Some(request_id) = next_request_id else {
            return false;
        };
        let Some(response_tx) = pending.lock().await.remove(&request_id) else {
            continue;
        };
        let _ = response_tx.send(control_msg);
        return true;
    }
}

pub async fn clear_remote_if_instance(state: &AppState, instance_id: Uuid) -> bool {
    let mut remote = state.remote.write().await;
    if remote.as_ref().map(|conn| conn.instance_id) == Some(instance_id) {
        *remote = None;
        true
    } else {
        false
    }
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        let transfer_receiver = transfer_receiver::TransferReceiver::new(config.root_dir.clone());
        let (browser_events, _) = broadcast::channel(16);
        let last_target = RwLock::new(config.target.clone());
        let last_password = RwLock::new(config.password.clone());
        Self {
            config,
            remote: RwLock::new(None),
            transfer_receiver,
            pending_completions: Mutex::new(HashMap::new()),
            browser_events,
            fingerprint: RwLock::new(None),
            last_target,
            last_password,
        }
    }

    /// Handle an incoming TransferError message.
    /// Returns true if the error was consumed (active transfer or pending completion found).
    /// Returns false if no matching transfer exists — caller should fall through to the
    /// generic FIFO response handler (likely a synchronous rejection of a TransferRequest).
    pub async fn handle_transfer_error(&self, id: Uuid, error: &str) -> bool {
        if self
            .transfer_receiver
            .signal_error(id, error.to_owned())
            .await
        {
            return true;
        }
        if let Some(tx) = self.pending_completions.lock().await.remove(&id) {
            let _ = tx.send(Err(error.to_owned()));
            return true;
        }
        false
    }
}

/// Tear down the current remote connection (if any) from either side.
/// Aborts all read/write tasks, clears state, and broadcasts ConnectionStatus false.
/// `clear_creds`: when true, also forget the last target/password so the FE no
/// longer offers a Reconnect option. Use true on intentional disconnects.
pub async fn disconnect_remote(state: &AppState, clear_creds: bool) {
    let connection = {
        let mut remote = state.remote.write().await;
        remote.take()
    };
    if let Some(conn) = connection {
        abort_remote_connection(conn).await;
    }
    *state.fingerprint.write().await = None;
    if clear_creds {
        *state.last_target.write().await = None;
        *state.last_password.write().await = None;
    }
    let _ = state
        .browser_events
        .send(ControlMessage::ConnectionStatus { has_remote: false });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use std::path::PathBuf;
    use tokio::sync::mpsc;

    fn test_config() -> AppConfig {
        AppConfig {
            target: None,
            password: None,
            root_dir: PathBuf::from("."),
            hostname: "test-host".to_string(),
            allow_insecure_tls: true,
            disable_ui: false,
        }
    }

    #[tokio::test]
    async fn delivers_out_of_order_responses_by_request_id() {
        let pending: PendingResponses = Arc::new(Mutex::new(HashMap::new()));
        let pending_order: PendingResponseOrder = Arc::new(Mutex::new(VecDeque::new()));
        let request_a = Uuid::new_v4();
        let request_b = Uuid::new_v4();
        let (tx_a, rx_a) = oneshot::channel();
        let (tx_b, rx_b) = oneshot::channel();
        register_pending_response(
            &pending,
            &pending_order,
            request_a,
            tx_a,
            Some(MIN_REQUEST_ID_VERSION),
            true,
        )
        .await;
        register_pending_response(
            &pending,
            &pending_order,
            request_b,
            tx_b,
            Some(MIN_REQUEST_ID_VERSION),
            true,
        )
        .await;

        assert!(
            deliver_pending_response(
                &pending,
                &pending_order,
                ControlMessage::BrowseResponse {
                    request_id: Some(request_b),
                    hostname: "remote".to_string(),
                    cwd: "/b".to_string(),
                    entries: vec![],
                },
                Some(MIN_REQUEST_ID_VERSION),
            )
            .await
        );

        match rx_b.await.expect("request b should resolve") {
            ControlMessage::BrowseResponse {
                request_id,
                cwd,
                ..
            } => {
                assert_eq!(request_id, Some(request_b));
                assert_eq!(cwd, "/b");
            }
            other => panic!("unexpected response for request b: {other:?}"),
        }

        assert_eq!(pending.lock().await.len(), 1);

        assert!(
            deliver_pending_response(
                &pending,
                &pending_order,
                ControlMessage::BrowseResponse {
                    request_id: Some(request_a),
                    hostname: "remote".to_string(),
                    cwd: "/a".to_string(),
                    entries: vec![],
                },
                Some(MIN_REQUEST_ID_VERSION),
            )
            .await
        );

        match rx_a.await.expect("request a should resolve") {
            ControlMessage::BrowseResponse {
                request_id,
                cwd,
                ..
            } => {
                assert_eq!(request_id, Some(request_a));
                assert_eq!(cwd, "/a");
            }
            other => panic!("unexpected response for request a: {other:?}"),
        }

        assert!(pending.lock().await.is_empty());
    }

    #[tokio::test]
    async fn disconnect_remote_fails_pending_requests() {
        let state = AppState::new(test_config());
        let pending_requests: PendingResponses = Arc::new(Mutex::new(HashMap::new()));
        let pending_request_order: PendingResponseOrder = Arc::new(Mutex::new(VecDeque::new()));
        let request_id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        register_pending_response(
            &pending_requests,
            &pending_request_order,
            request_id,
            tx,
            Some(MIN_REQUEST_ID_VERSION),
            true,
        )
        .await;
        let (request_tx, _request_rx) = mpsc::unbounded_channel::<ControlMessage>();
        let (frame_tx, _frame_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        *state.remote.write().await = Some(RemoteConnection {
            instance_id: Uuid::new_v4(),
            hostname: "remote".to_string(),
            root_dir: "/".to_string(),
            tx: request_tx,
            pending_requests: pending_requests.clone(),
            pending_request_order,
            frame_tx,
            task_handles: vec![],
            peer_version: None,
        });

        disconnect_remote(&state, false).await;

        match rx.await.expect("pending request should receive disconnect error") {
            ControlMessage::Error {
                request_id: Some(id),
                message,
            } => {
                assert_eq!(id, request_id);
                assert_eq!(message, "Remote connection lost");
            }
            other => panic!("unexpected pending request message: {other:?}"),
        }

        assert!(pending_requests.lock().await.is_empty());
    }

    #[tokio::test]
    async fn clear_remote_if_instance_only_clears_matching_connection() {
        let state = AppState::new(test_config());
        let current_id = Uuid::new_v4();
        let stale_id = Uuid::new_v4();
        let (request_tx, _request_rx) = mpsc::unbounded_channel::<ControlMessage>();
        let (frame_tx, _frame_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let pending_requests: PendingResponses = Arc::new(Mutex::new(HashMap::new()));
        let pending_request_order: PendingResponseOrder = Arc::new(Mutex::new(VecDeque::new()));

        *state.remote.write().await = Some(RemoteConnection {
            instance_id: current_id,
            hostname: "remote".to_string(),
            root_dir: "/".to_string(),
            tx: request_tx,
            pending_requests,
            pending_request_order,
            frame_tx,
            task_handles: vec![],
            peer_version: None,
        });

        assert!(!clear_remote_if_instance(&state, stale_id).await);
        assert_eq!(
            state.remote.read().await.as_ref().map(|conn| conn.instance_id),
            Some(current_id)
        );

        assert!(clear_remote_if_instance(&state, current_id).await);
        assert!(state.remote.read().await.is_none());
    }

    #[tokio::test]
    async fn delivers_fifo_response_for_legacy_peer_without_request_id() {
        let pending: PendingResponses = Arc::new(Mutex::new(HashMap::new()));
        let pending_order: PendingResponseOrder = Arc::new(Mutex::new(VecDeque::new()));
        let request_id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();

        register_pending_response(&pending, &pending_order, request_id, tx, Some(2), true).await;

        assert!(
            deliver_pending_response(
                &pending,
                &pending_order,
                ControlMessage::InfoResponse {
                    request_id: None,
                    hostname: "legacy".to_string(),
                    root_dir: "/".to_string(),
                },
                Some(2),
            )
            .await
        );

        match rx.await.expect("legacy request should resolve") {
            ControlMessage::InfoResponse {
                request_id,
                hostname,
                root_dir,
            } => {
                assert_eq!(request_id, None);
                assert_eq!(hostname, "legacy");
                assert_eq!(root_dir, "/");
            }
            other => panic!("unexpected legacy response: {other:?}"),
        }
    }
}

pub async fn run(state: Arc<AppState>, port: Option<u16>) -> anyhow::Result<()> {
    let disable_ui = state.config.disable_ui;
    let mut app = Router::new().route("/ws", get(ws_handler::ws_upgrade));
    if !disable_ui {
        app = app
            .route("/api/browse", get(file_api::browse))
            .route("/api/browse-remote", get(file_api::browse_remote))
            .route("/api/info", get(file_api::info))
            .route("/api/connect", axum::routing::post(file_api::connect))
            .route("/api/reconnect", axum::routing::post(file_api::reconnect))
            .route("/api/disconnect", axum::routing::post(file_api::disconnect))
            .fallback(static_handler);
    }
    let app = app.layer(CorsLayer::permissive()).with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port.unwrap_or(0)));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let actual_port = listener.local_addr()?.port();

    if disable_ui {
        tracing::info!("UI disabled — only /ws is exposed");
    }

    let local_ips = get_local_ip_addresses();
    if local_ips.is_empty() {
        tracing::info!("drift server listening on http://localhost:{}", actual_port);
    } else {
        tracing::info!("drift server listening on:");
        tracing::info!("  http://localhost:{}", actual_port);
        for ip in local_ips {
            tracing::info!("  http://{}:{}", ip, actual_port);
        }
    }

    axum::serve(listener, app).await?;

    Ok(())
}

fn get_local_ip_addresses() -> Vec<std::net::IpAddr> {
    use std::net::UdpSocket;

    let mut ips = Vec::new();

    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                ips.push(addr.ip());
            }
        }
    }

    ips
}
