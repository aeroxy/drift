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
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock};
use tower_http::cors::CorsLayer;

pub struct AppState {
    pub config: AppConfig,
    pub remote: RwLock<Option<RemoteConnection>>,
    pub transfer_receiver: transfer_receiver::TransferReceiver,
}

pub type ResponseChannel = oneshot::Sender<ControlMessage>;
pub type RequestChannel = mpsc::UnboundedSender<(ControlMessage, ResponseChannel)>;
pub type BinaryChannel = mpsc::UnboundedSender<Vec<u8>>;
pub type OutgoingChannel = mpsc::UnboundedSender<ControlMessage>;

pub struct RemoteConnection {
    pub hostname: String,
    pub root_dir: String,
    pub tx: RequestChannel,
    pub binary_tx: BinaryChannel,
    pub outgoing_tx: OutgoingChannel,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        let transfer_receiver = transfer_receiver::TransferReceiver::new(config.root_dir.clone());
        Self {
            config,
            remote: RwLock::new(None),
            transfer_receiver,
        }
    }
}

pub async fn run(state: Arc<AppState>, port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/browse", get(file_api::browse))
        .route("/api/info", get(file_api::info))
        .route("/ws", get(ws_handler::ws_upgrade))
        .fallback(static_handler)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));

    // Get local IP addresses
    let local_ips = get_local_ip_addresses();
    if local_ips.is_empty() {
        tracing::info!("drift server listening on http://localhost:{}", port);
    } else {
        tracing::info!("drift server listening on:");
        tracing::info!("  http://localhost:{}", port);
        for ip in local_ips {
            tracing::info!("  http://{}:{}", ip, port);
        }
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn get_local_ip_addresses() -> Vec<std::net::IpAddr> {
    use std::net::UdpSocket;

    let mut ips = Vec::new();

    // Try to get the primary local IP by connecting to a public DNS
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                ips.push(addr.ip());
            }
        }
    }

    ips
}
