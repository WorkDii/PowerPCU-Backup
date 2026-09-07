//! Serves the embedded `web/` folder (Alpine UI in Plan 2). Unknown paths fall back to
//! `index.html`; `index.html` is `no-cache` so an upgrade never shows a stale UI.

use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::RustEmbed)]
#[folder = "../web"]
struct Web;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let key = if path.is_empty() { "index.html" } else { path };
    let (file, name) = match Web::get(key) {
        Some(f) => (f, key),
        None => match Web::get("index.html") {
            Some(f) => (f, "index.html"),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    let mime = file.metadata.mimetype().to_string();
    let mut res = (StatusCode::OK, [(header::CONTENT_TYPE, mime)], file.data.into_owned()).into_response();
    if name == "index.html" {
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    res
}
