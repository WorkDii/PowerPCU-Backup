//! axum layer over the engine: parse → validate → engine → JSON. Business outcomes are
//! 200 `{ok, message}`; only DB/unexpected failures are 500. Every `/api/*` request must
//! carry `x-powerpcu-client: backup-ui` (blocks cross-site form posts to localhost).

pub mod plans;
pub mod runs;
pub mod static_files;
pub mod status;
pub mod storages;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::state::AppState;

pub const CLIENT_HEADER: &str = "x-powerpcu-client";
pub const CLIENT_TOKEN: &str = "backup-ui";

pub struct ApiError(pub String);
impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self { ApiError(e.to_string()) }
}
impl From<String> for ApiError {
    fn from(e: String) -> Self { ApiError(e) }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        tracing::error!("api: {}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": self.0 }))).into_response()
    }
}

pub fn ok_false(message: impl Into<String>) -> Json<Value> {
    Json(json!({ "ok": false, "message": message.into() }))
}
pub fn ok_true(message: impl Into<String>) -> Json<Value> {
    Json(json!({ "ok": true, "message": message.into() }))
}

async fn client_guard(req: Request, next: Next) -> Response {
    let ok = req.headers().get(CLIENT_HEADER).and_then(|v| v.to_str().ok()) == Some(CLIENT_TOKEN);
    if !ok {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "forbidden" }))).into_response();
    }
    next.run(req).await
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/status", get(status::get_status))
        .route("/storages", get(storages::list).post(storages::create))
        .route("/storages/test", post(storages::test_config))
        .route("/storages/{id}", put(storages::update).delete(storages::remove))
        .route("/storages/{id}/test", post(storages::test_saved))
        .route("/plans", get(plans::list).post(plans::create))
        .route("/plans/test-connection", post(plans::test_connection))
        .route("/plans/{id}", get(plans::get_one).put(plans::update).delete(plans::remove))
        .route("/plans/{id}/run", post(plans::run_now))
        .route("/plans/{id}/runs", get(runs::list_for_plan))
        .route("/runs/{id}/files", get(runs::files_for_run))
        .layer(middleware::from_fn(client_guard))
        .with_state(state);
    Router::new().nest("/api", api).fallback(static_files::serve)
}
