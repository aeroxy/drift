use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::fileops::browse;
use crate::protocol::messages::FileEntry;
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
