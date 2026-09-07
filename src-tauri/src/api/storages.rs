use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{ok_false, ok_true, ApiError};
use crate::backup::storage::{self, Storage, StorageConfig};
use crate::backup::storages;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct StorageBody {
    pub name: String,
    pub provider: String,
    pub config: Value,
    /// test_config only: reuse the stored secret of this storage when the form left it blank.
    #[serde(default)]
    pub storage_id: Option<i64>,
}

fn parse(body: &StorageBody) -> Result<StorageConfig, String> {
    if body.name.trim().is_empty() {
        return Err("กรุณาตั้งชื่อปลายทาง".into());
    }
    StorageConfig::parse(&body.provider, &body.config.to_string())
}

pub async fn list(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let mut out = Vec::new();
    for s in storages::list_all(&st.pool).await? {
        let mut v = s.to_public_value();
        v["in_use"] = json!(storages::is_in_use(&st.pool, s.id).await?);
        out.push(v);
    }
    Ok(Json(Value::Array(out)))
}

pub async fn create(State(st): State<AppState>, Json(body): Json<StorageBody>) -> Result<Json<Value>, ApiError> {
    let mut cfg = match parse(&body) { Ok(c) => c, Err(m) => return Ok(ok_false(m)) };
    if cfg.secret_is_empty() {
        return Ok(ok_false("กรุณากรอก secret key"));
    }
    cfg.protect_secret()?;
    let id = storages::insert(&st.pool, body.name.trim(), cfg.provider_str(), &cfg.to_json()).await?;
    Ok(Json(json!({ "ok": true, "id": id })))
}

pub async fn update(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<StorageBody>) -> Result<Json<Value>, ApiError> {
    let Some(old) = storages::find(&st.pool, id).await? else { return Ok(ok_false("ไม่พบปลายทาง")) };
    let mut cfg = match parse(&body) { Ok(c) => c, Err(m) => return Ok(ok_false(m)) };
    cfg.keep_secret_from(&old.config);
    if cfg.secret_is_empty() {
        return Ok(ok_false("กรุณากรอก secret key"));
    }
    cfg.protect_secret()?;
    storages::update(&st.pool, id, body.name.trim(), cfg.provider_str(), &cfg.to_json()).await?;
    Ok(ok_true("บันทึกแล้ว"))
}

pub async fn remove(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    if storages::find(&st.pool, id).await?.is_none() {
        return Ok(ok_false("ไม่พบปลายทาง"));
    }
    if storages::is_in_use(&st.pool, id).await? {
        return Ok(ok_false("ลบไม่ได้: ปลายทางนี้ยังถูกใช้โดยแผน หรือยังมีไฟล์สำรองอยู่"));
    }
    storages::delete(&st.pool, id).await?;
    Ok(ok_true("ลบแล้ว"))
}

pub async fn test_saved(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    let Some(s) = storages::find(&st.pool, id).await? else { return Ok(ok_false("ไม่พบปลายทาง")) };
    Ok(run_test(&s).await)
}

pub async fn test_config(State(st): State<AppState>, Json(body): Json<StorageBody>) -> Result<Json<Value>, ApiError> {
    let mut cfg = match parse(&body) { Ok(c) => c, Err(m) => return Ok(ok_false(m)) };
    if cfg.secret_is_empty() {
        if let Some(old) = match body.storage_id { Some(id) => storages::find(&st.pool, id).await?, None => None } {
            cfg.keep_secret_from(&old.config);
        }
    }
    if cfg.secret_is_empty() {
        return Ok(ok_false("กรุณากรอก secret key"));
    }
    Ok(run_test(&Storage { id: 0, name: body.name, config: cfg }).await)
}

async fn run_test(s: &Storage) -> Json<Value> {
    match storage::for_storage(s) {
        Ok(p) => match p.test().await {
            Ok(()) => ok_true("เชื่อมต่อสำเร็จ"),
            Err(m) => ok_false(m),
        },
        Err(m) => ok_false(m),
    }
}
