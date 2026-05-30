use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::fileops::browse;
use crate::protocol::messages::{ControlMessage, FileEntry};
use crate::server::AppState;

fn default_path() -> String {
    ".".to_string()
}

#[derive(Deserialize)]
pub struct BrowseParams {
    #[serde(default = "default_path")]
    pub path: String,
}

#[derive(Serialize)]
pub struct BrowseResponse {
    pub hostname: String,
    pub cwd: String,
    pub entries: Vec<FileEntry>,
}

pub async fn browse(
    State(state): State<Arc<AppState>>,
    Query(params): Query<BrowseParams>,
) -> Result<Json<BrowseResponse>, axum::http::StatusCode> {
    let entries = browse::list_directory(&state.config.root_dir, &params.path)
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    let cwd = state
        .config
        .root_dir
        .join(&params.path)
        .canonicalize()
        .unwrap_or_else(|_| state.config.root_dir.clone())
        .to_string_lossy()
        .to_string();

    Ok(Json(BrowseResponse {
        hostname: state.config.hostname.clone(),
        cwd,
        entries,
    }))
}

pub async fn browse_remote(
    State(state): State<Arc<AppState>>,
    Query(params): Query<BrowseParams>,
) -> Result<Json<BrowseResponse>, StatusCode> {
    let request_id = Uuid::new_v4();
    let (response_rx, pending_requests, pending_request_order) = {
        let remote = state.remote.read().await;
        let remote_conn = remote.as_ref().ok_or(StatusCode::BAD_REQUEST)?;

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let msg = ControlMessage::BrowseRequest {
            request_id: Some(request_id),
            path: params.path.clone(),
        };
        crate::server::register_pending_response(
            &remote_conn.pending_requests,
            &remote_conn.pending_request_order,
            request_id,
            response_tx,
            remote_conn.peer_version,
        )
        .await;
        if remote_conn.tx.send(msg).is_err() {
            let _ = crate::server::remove_pending_response(
                &remote_conn.pending_requests,
                &remote_conn.pending_request_order,
                request_id,
            )
            .await;
            return Err(StatusCode::BAD_REQUEST);
        }
        (
            response_rx,
            remote_conn.pending_requests.clone(),
            remote_conn.pending_request_order.clone(),
        )
    }; // lock released here

    match tokio::time::timeout(std::time::Duration::from_secs(10), response_rx).await {
        Ok(Ok(ControlMessage::BrowseResponse {
            request_id: response_id,
            hostname,
            cwd,
            entries,
        })) if response_id.is_none() || response_id == Some(request_id) => Ok(Json(BrowseResponse {
            hostname,
            cwd,
            entries,
        })),
        Ok(Ok(ControlMessage::Error {
            request_id: response_id,
            message,
        })) if response_id.is_none() || response_id == Some(request_id) => {
            tracing::warn!("Remote browse error: {}", message);
            Err(StatusCode::BAD_GATEWAY)
        }
        Ok(Ok(_)) => Err(StatusCode::BAD_GATEWAY),
        Ok(Err(_)) => Err(StatusCode::BAD_GATEWAY),
        Err(_) => {
            let _ = crate::server::remove_pending_response(
                &pending_requests,
                &pending_request_order,
                request_id,
            )
            .await;
            Err(StatusCode::GATEWAY_TIMEOUT)
        }
    }
}

#[derive(Serialize)]
pub struct InfoResponse {
    pub hostname: String,
    pub root_dir: String,
    pub has_remote: bool,
    pub fingerprint: Option<String>,
    /// True when the BE has a remembered target the FE can reconnect to without
    /// re-prompting (i.e. a previous connect succeeded and was not torn down via
    /// `/api/disconnect`).
    pub can_reconnect: bool,
    /// The remembered target string (e.g. "192.168.0.2:8000"), shown in the FE
    /// for context. Never includes the password.
    pub last_target: Option<String>,
}

pub async fn info(State(state): State<Arc<AppState>>) -> Json<InfoResponse> {
    let has_remote = state.remote.read().await.is_some();
    let fingerprint = state.fingerprint.read().await.clone();
    let last_target = state.last_target.read().await.clone();
    let can_reconnect = !has_remote && last_target.is_some();
    Json(InfoResponse {
        hostname: state.config.hostname.clone(),
        root_dir: state.config.root_dir.to_string_lossy().to_string(),
        has_remote,
        fingerprint,
        can_reconnect,
        last_target,
    })
}

// ── Connection management ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ConnectParams {
    pub target: String,
    pub password: Option<String>,
}

#[derive(Serialize)]
pub struct ConnectResponse {
    pub success: bool,
    pub error: Option<String>,
    pub fingerprint: Option<String>,
}

pub async fn connect(
    State(state): State<Arc<AppState>>,
    Json(params): Json<ConnectParams>,
) -> Json<ConnectResponse> {
    do_connect(state, params.target, params.password).await
}

pub async fn reconnect(State(state): State<Arc<AppState>>) -> Json<ConnectResponse> {
    let target = state.last_target.read().await.clone();
    let password = state.last_password.read().await.clone();
    match target {
        Some(t) => do_connect(state, t, password).await,
        None => Json(ConnectResponse {
            success: false,
            error: Some("No previous connection to reconnect to".to_string()),
            fingerprint: None,
        }),
    }
}

/// Shared connect-and-wait body for `/api/connect` and `/api/reconnect`.
/// Spawns connect_to_remote() and races a 10s timeout against the success
/// broadcast and the failure channel. Stored credentials are written by
/// connect_to_remote() itself on successful handshake.
async fn do_connect(
    state: Arc<AppState>,
    target: String,
    password: Option<String>,
) -> Json<ConnectResponse> {
    // Tear down any existing connection first (keep stored creds — they'll be
    // overwritten on success, and we want to keep them on failure too so the
    // user can retry).
    crate::server::disconnect_remote(&state, false).await;

    // Subscribe before spawning so we don't miss the success signal
    let mut event_rx = state.browser_events.subscribe();

    let allow_insecure_tls = state.config.allow_insecure_tls;
    let state_clone = state.clone();
    let target_clone = target.clone();
    let password_clone = password.clone();

    // Channel to receive a connection error (only sent on failure; on success
    // connect_to_remote keeps running until the connection closes)
    let (err_tx, err_rx) = tokio::sync::oneshot::channel::<String>();

    tokio::spawn(async move {
        if let Err(e) = crate::client::connect_to_remote(
            &target_clone,
            &password_clone,
            allow_insecure_tls,
            state_clone,
        )
        .await
        {
            let _ = err_tx.send(e.to_string());
        }
    });

    // Race: ConnectionStatus{true} = success, err channel = failure, timeout = give up
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::select! {
            event = async {
                loop {
                    match event_rx.recv().await {
                        Ok(ControlMessage::ConnectionStatus { has_remote: true }) => break Ok(()),
                        Err(_) => break Err("Event channel closed".to_string()),
                        _ => continue,
                    }
                }
            } => event,
            err = err_rx => {
                Err(err.unwrap_or_else(|_| "Connection task ended unexpectedly".to_string()))
            }
        }
    })
    .await;

    match result {
        Ok(Ok(())) => {
            let fp = state.fingerprint.read().await.clone();
            Json(ConnectResponse {
                success: true,
                error: None,
                fingerprint: fp,
            })
        }
        Ok(Err(e)) => Json(ConnectResponse {
            success: false,
            error: Some(e),
            fingerprint: None,
        }),
        Err(_) => {
            crate::server::disconnect_remote(&state, false).await;
            Json(ConnectResponse {
                success: false,
                error: Some("Connection timed out".to_string()),
                fingerprint: None,
            })
        }
    }
}

pub async fn disconnect(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    crate::server::disconnect_remote(&state, true).await;
    Json(serde_json::json!({ "success": true }))
}
