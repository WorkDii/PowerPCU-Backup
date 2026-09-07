use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::api::ApiError;
use crate::backup::{plans, running, runs, schedule};
use crate::state::AppState;

/// Dashboard + tray poll: version, uptime, and per-plan running/next/last.
pub async fn get_status(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let mut out = Vec::new();
    for p in plans::list_all(&st.pool).await? {
        let last = runs::last_for_plan(&st.pool, p.id).await?;
        let next = schedule::next_run_at(p.id).await.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string());
        out.push(json!({
            "id": p.id, "name": p.name, "active": p.active,
            "running": running::is_running(p.id),
            "next_run_at": next,
            "last_run": last,
        }));
    }
    Ok(Json(json!({ "version": env!("CARGO_PKG_VERSION"), "started_at": st.started_at, "plans": out })))
}
