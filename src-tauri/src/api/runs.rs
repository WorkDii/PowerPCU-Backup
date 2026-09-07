use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde_json::{json, Value};

use crate::api::ApiError;
use crate::backup::prune::Reason;
use crate::backup::{files, plans, runs};
use crate::state::AppState;

pub async fn list_for_plan(State(st): State<AppState>, Path(id): Path<i64>, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, ApiError> {
    let limit = q.get("limit").and_then(|v| v.parse::<i64>().ok()).unwrap_or(50).clamp(1, 500);
    Ok(Json(json!(runs::list_for_plan(&st.pool, id, limit).await?)))
}

/// Files of one run, each with `kept_as` = the rules currently keeping it (empty when deleted).
pub async fn files_for_run(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    let list = files::list_for_run(&st.pool, id).await?;
    let plan = match list.first() { Some(f) => plans::find(&st.pool, f.plan_id).await?, None => None };
    let now = chrono::Local::now().naive_local();
    let mut by_storage: HashMap<i64, HashMap<i64, Vec<Reason>>> = HashMap::new();
    let mut out = Vec::new();
    for f in list {
        let mut reasons: Vec<Reason> = Vec::new();
        if f.deleted_at.is_none() {
            if let Some(p) = &plan {
                if !by_storage.contains_key(&f.storage_id) {
                    let kept = files::kept_as(&st.pool, p, f.storage_id, now).await.unwrap_or_default();
                    by_storage.insert(f.storage_id, kept);
                }
                reasons = by_storage[&f.storage_id].get(&f.id).cloned().unwrap_or_default();
            }
        }
        let mut v = serde_json::to_value(&f).unwrap_or(Value::Null);
        v["kept_as"] = json!(reasons);
        out.push(v);
    }
    Ok(Json(Value::Array(out)))
}
