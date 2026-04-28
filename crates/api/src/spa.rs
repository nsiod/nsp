//! Embedded SPA routes.
//!
//! The `ui/dist/` directory is embedded at compile time via `rust-embed`.
//! We mount it under `/ui/` and provide a fallback to `/ui/index.html` so
//! client-side routing works on deep links and hard refresh.

use axum::{
    body::Body,
    extract::Path,
    http::{header, HeaderValue, Response, StatusCode, Uri},
    routing::get,
    Router,
};
use rust_embed::RustEmbed;

use crate::state::AppState;

#[derive(RustEmbed)]
#[folder = "../../ui/dist"]
#[include = "*"]
struct Assets;

pub fn router() -> Router<std::sync::Arc<AppState>> {
    Router::new()
        .route("/", get(root_redirect))
        .route("/ui", get(ui_bare_redirect))
        .route("/ui/", get(ui_index))
        .route("/ui/*path", get(serve_asset))
}

async fn root_redirect() -> Response<Body> {
    redirect_to("/ui/", StatusCode::FOUND)
}

// `/ui` must 308-redirect to `/ui/` so that relative asset paths in the SPA
// (e.g. `<script src="app.js">`) resolve under `/ui/` instead of `/`.
async fn ui_bare_redirect() -> Response<Body> {
    redirect_to("/ui/", StatusCode::PERMANENT_REDIRECT)
}

async fn ui_index() -> Response<Body> {
    render_index()
}

fn redirect_to(location: &'static str, status: StatusCode) -> Response<Body> {
    let mut resp = Response::new(Body::empty());
    *resp.status_mut() = status;
    resp.headers_mut()
        .insert(header::LOCATION, HeaderValue::from_static(location));
    resp
}

async fn serve_asset(Path(path): Path<String>, uri: Uri) -> Response<Body> {
    match Assets::get(&path) {
        Some(file) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            let bytes: Vec<u8> = file.data.into_owned();
            let mut resp = Response::new(Body::from(bytes));
            let ctype = HeaderValue::from_str(mime.as_ref())
                .unwrap_or(HeaderValue::from_static("application/octet-stream"));
            resp.headers_mut().insert(header::CONTENT_TYPE, ctype);
            resp
        }
        None if looks_like_asset(&path) => {
            tracing::debug!(uri = %uri, "asset miss");
            not_found()
        }
        // SPA fallback: any non-asset path under /ui/* renders index.html.
        None => render_index(),
    }
}

fn render_index() -> Response<Body> {
    match Assets::get("index.html") {
        Some(file) => {
            let bytes: Vec<u8> = file.data.into_owned();
            let mut resp = Response::new(Body::from(bytes));
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            resp
        }
        None => missing_bundle(),
    }
}

fn looks_like_asset(path: &str) -> bool {
    matches!(
        std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str()),
        Some(
            "js" | "css"
                | "map"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "svg"
                | "ico"
                | "webp"
                | "woff"
                | "woff2"
                | "ttf"
                | "json"
                | "wasm"
        )
    )
}

fn not_found() -> Response<Body> {
    let mut resp = Response::new(Body::from("not found"));
    *resp.status_mut() = StatusCode::NOT_FOUND;
    resp
}

fn missing_bundle() -> Response<Body> {
    tracing::error!("ui/dist/index.html is missing from the embedded bundle");
    let mut resp = Response::new(Body::from("UI bundle missing"));
    *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    resp
}
