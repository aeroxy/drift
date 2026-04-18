pub mod file_api;
pub mod ws_handler;
pub mod transfer_handler;
pub mod browser_transfer;
pub mod transfer_receiver;

use crate::config::AppConfig;
use crate::frontend::static_handler;
use crate::protocol::messages::ControlMessage;
use axum::{
    Router,
    routing::get,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, RwLock};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

pub struct AppState {
    pub config: AppConfig,
    pub remote: RwLock<Option<RemoteConnection>>,
    pub transfer_receiver: transfer_receiver::TransferReceiver,
    /// Oneshot channels fired when a remote confirms TransferFinalized.
    /// push_entries registers here before sending TransferComplete.
    pub pending_completions: Mutex<HashMap<Uuid, oneshot::Sender<()>>>,
    /// Broadcast channel for pushing events (ConnectionStatus etc.) to all browsers.
    pub browser_events: broadcast::Sender<ControlMessage>,
    /// Short hex fingerprint of the DH shared secret (for visual MITM verification).
    pub fingerprint: RwLock<Option<String>>,
}

pub type ResponseChannel = oneshot::Sender<ControlMessage>;
pub type RequestChannel = mpsc::UnboundedSender<(ControlMessage, ResponseChannel)>;

/// Unified outgoing channel. Carries pre-encoded frames (type byte + payload, NOT yet
/// encrypted). Both data chunks (`encode_data_frame`) and control messages
/// (`encode_control_frame`) travel through this single FIFO queue, preserving send
/// order without priority starvation.
pub type FrameChannel = mpsc::UnboundedSender<Vec<u8>>;

pub struct RemoteConnection {
    pub hostname: String,
    pub root_dir: String,
    /// For browser-initiated requests that need a response (e.g. BrowseRequest).
    pub tx: RequestChannel,
    /// Unified outbound: send pre-encoded frames via `encode_data_frame` or
    /// `encode_control_frame` from `crate::protocol::codec`.
    pub frame_tx: FrameChannel,
    /// Abort handles for all tasks driving this connection (read, write, request handler).
    /// Aborting these cleanly tears down the connection from either side.
    pub task_handles: Vec<tokio::task::AbortHandle>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        let transfer_receiver = transfer_receiver::TransferReceiver::new(config.root_dir.clone());
        let (browser_events, _) = broadcast::channel(16);
        Self {
            config,
            remote: RwLock::new(None),
            transfer_receiver,
            pending_completions: Mutex::new(HashMap::new()),
            browser_events,
            fingerprint: RwLock::new(None),
        }
    }
}

/// Tear down the current remote connection (if any) from either side.
/// Aborts all read/write tasks, clears state, and broadcasts ConnectionStatus false.
pub async fn disconnect_remote(state: &AppState) {
    let connection = {
        let mut remote = state.remote.write().await;
        remote.take()
    };
    if let Some(conn) = connection {
        for handle in conn.task_handles {
            handle.abort();
        }
    }
    *state.fingerprint.write().await = None;
    let _ = state.browser_events.send(ControlMessage::ConnectionStatus { has_remote: false });
}

pub async fn run(state: Arc<AppState>, port: Option<u16>) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/browse", get(file_api::browse))
        .route("/api/info", get(file_api::info))
        .route("/api/connect", axum::routing::post(file_api::connect))
        .route("/api/disconnect", axum::routing::post(file_api::disconnect))
        .route("/ws", get(ws_handler::ws_upgrade))
        .fallback(static_handler)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port.unwrap_or(0)));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let actual_port = listener.local_addr()?.port();

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
