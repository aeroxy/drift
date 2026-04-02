use axum::{
    extract::Request,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "frontend/dist"]
struct Assets;

pub async fn static_handler(req: Request) -> Response {
    let path = req.uri().path().trim_start_matches('/');

    // Try the exact path first
    if let Some(content) = <Assets as RustEmbed>::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime.as_ref())],
            content.data.into_owned(),
        )
            .into_response()
    } else {
        // SPA fallback: serve index.html for any unmatched route
        match <Assets as RustEmbed>::get("index.html") {
            Some(content) => Html(String::from_utf8_lossy(&content.data).to_string()).into_response(),
            None => (StatusCode::NOT_FOUND, "Frontend not built").into_response(),
        }
    }
}
