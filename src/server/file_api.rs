use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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

#[derive(Serialize)]
pub struct InfoResponse {
    pub hostname: String,
    pub root_dir: String,
    pub has_remote: bool,
    pub fingerprint: Option<String>,
}

pub async fn info(State(state): State<Arc<AppState>>) -> Json<InfoResponse> {
    let has_remote = state.remote.read().await.is_some();
    let fingerprint = state.fingerprint.read().await.clone();
    Json(InfoResponse {
        hostname: state.config.hostname.clone(),
        root_dir: state.config.root_dir.to_string_lossy().to_string(),
        has_remote,
        fingerprint,
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
    // Tear down any existing connection first
    crate::server::disconnect_remote(&state).await;

    // Subscribe before spawning so we don't miss the success signal
    let mut event_rx = state.browser_events.subscribe();

    let target = params.target.clone();
    let password = params.password.clone();
    let allow_insecure_tls = state.config.allow_insecure_tls;
    let state_clone = state.clone();

    // Channel to receive a connection error (only sent on failure; on success
    // connect_to_remote keeps running until the connection closes)
    let (err_tx, err_rx) = tokio::sync::oneshot::channel::<String>();

    tokio::spawn(async move {
        if let Err(e) = crate::client::connect_to_remote(&target, &password, allow_insecure_tls, state_clone).await {
            let _ = err_tx.send(e.to_string());
        }
    });

    // Race: ConnectionStatus{true} = success, err channel = failure, timeout = give up
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        async {
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
        },
    )
    .await;

    match result {
        Ok(Ok(())) => {
            let fp = state.fingerprint.read().await.clone();
            Json(ConnectResponse { success: true, error: None, fingerprint: fp })
        }
        Ok(Err(e)) => {
            Json(ConnectResponse { success: false, error: Some(e), fingerprint: None })
        }
        Err(_) => {
            crate::server::disconnect_remote(&state).await;
            Json(ConnectResponse {
                success: false,
                error: Some("Connection timed out".to_string()),
                fingerprint: None,
            })
        }
    }
}

pub async fn disconnect(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    crate::server::disconnect_remote(&state).await;
    Json(serde_json::json!({ "success": true }))
}
