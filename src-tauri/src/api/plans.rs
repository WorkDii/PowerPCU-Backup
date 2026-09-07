use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{ok_false, ok_true, ApiError};
use crate::backup::plans::{self, NewPlan};
use crate::backup::{mysqldump, run, running, schedule, storages};
use crate::secret;
use crate::state::AppState;

fn validate(p: &NewPlan) -> Result<(), String> {
    if p.name.trim().is_empty() {
        return Err("กรุณาตั้งชื่อแผน".into());
    }
    if p.database_name.trim().is_empty() || p.host.trim().is_empty() || p.username.trim().is_empty() {
        return Err("กรุณากรอกข้อมูลการเชื่อมต่อฐานข้อมูลให้ครบ".into());
    }
    if p.database_name.contains('/') || p.database_name.contains('\\') || p.database_name.contains("..") {
        return Err("ชื่อฐานข้อมูลต้องไม่มี / \\ หรือ ..".into());
    }
    if p.prefix_name.is_empty() || !p.prefix_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("รหัส/ชื่อย่อหน่วยบริการต้องเป็นตัวอักษรอังกฤษ ตัวเลข _ หรือ - เท่านั้น".into());
    }
    if let Some(c) = p.schedule_cron.as_deref() {
        if c.split_whitespace().count() != 5 {
            return Err("รูปแบบ cron ต้องมี 5 ช่อง (นาที ชั่วโมง วันที่ เดือน วันในสัปดาห์)".into());
        }
    }
    Ok(())
}

/// Blank cron → manual (`None`); otherwise trimmed.
fn normalize_cron(p: &mut NewPlan) {
    p.schedule_cron = p.schedule_cron.take().map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
}

async fn resolve_storages(st: &AppState, p: &NewPlan, exclude: Option<i64>) -> Result<Result<Vec<i64>, String>, ApiError> {
    let ids = storages::existing_ids(&st.pool, &p.storage_ids).await?;
    if ids.is_empty() {
        return Ok(Err("ต้องเลือกปลายทางอย่างน้อย 1 แห่ง".into()));
    }
    if let Some(c) = plans::find_conflict(&st.pool, exclude, &p.prefix_name, &p.database_name, &ids).await? {
        return Ok(Err(format!(
            "แผน '{}' ใช้รหัสหน่วยบริการ+ฐานข้อมูลเดียวกันบนปลายทาง '{}' อยู่แล้ว เปลี่ยนรหัสหน่วยบริการหรือปลายทาง",
            c.plan_name, c.storage_name
        )));
    }
    Ok(Ok(ids))
}

const ZIP_PW_MSG: &str = "รหัสเข้ารหัสไฟล์ต้องมีอย่างน้อย 8 ตัวอักษร";

pub async fn list(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let mut out = Vec::new();
    for p in plans::list_all(&st.pool).await? {
        let ids = plans::storage_ids_for(&st.pool, p.id).await?;
        out.push(p.to_public_value(&ids));
    }
    Ok(Json(Value::Array(out)))
}

pub async fn get_one(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    Ok(Json(match plans::find(&st.pool, id).await? {
        Some(p) => p.to_public_value(&plans::storage_ids_for(&st.pool, id).await?),
        None => Value::Null,
    }))
}

pub async fn create(State(st): State<AppState>, Json(mut body): Json<NewPlan>) -> Result<Json<Value>, ApiError> {
    normalize_cron(&mut body);
    if let Err(m) = validate(&body) {
        return Ok(ok_false(m));
    }
    let ids = match resolve_storages(&st, &body, None).await? { Ok(ids) => ids, Err(m) => return Ok(ok_false(m)) };
    if body.password.is_empty() {
        let src = match body.copy_password_from_plan_id { Some(id) => plans::find(&st.pool, id).await?, None => None };
        match src {
            Some(p) => body.password = p.password, // already a dpapi blob
            None => return Ok(ok_false("กรุณากรอกรหัสผ่านฐานข้อมูล")),
        }
    } else {
        body.password = secret::protect(&body.password)?;
    }
    if body.encryption_password.chars().count() < 8 {
        return Ok(ok_false(ZIP_PW_MSG));
    }
    body.encryption_password = secret::protect(&body.encryption_password)?;

    let id = plans::insert(&st.pool, &body).await?;
    plans::replace_links(&st.pool, id, &ids).await?;
    if let Some(p) = plans::find(&st.pool, id).await? {
        schedule::register_plan(&st, &p).await;
    }
    Ok(Json(json!({ "ok": true, "id": id })))
}

pub async fn update(State(st): State<AppState>, Path(id): Path<i64>, Json(mut body): Json<NewPlan>) -> Result<Json<Value>, ApiError> {
    let Some(old) = plans::find(&st.pool, id).await? else { return Ok(ok_false("ไม่พบแผน")) };
    normalize_cron(&mut body);
    if let Err(m) = validate(&body) {
        return Ok(ok_false(m));
    }
    let ids = match resolve_storages(&st, &body, Some(id)).await? { Ok(ids) => ids, Err(m) => return Ok(ok_false(m)) };
    body.password = if body.password.is_empty() { old.password.clone() } else { secret::protect(&body.password)? };
    body.encryption_password = if body.encryption_password.is_empty() {
        old.encryption_password.clone()
    } else {
        if body.encryption_password.chars().count() < 8 {
            return Ok(ok_false(ZIP_PW_MSG));
        }
        secret::protect(&body.encryption_password)?
    };
    plans::update(&st.pool, id, &body).await?;
    plans::replace_links(&st.pool, id, &ids).await?;
    schedule::unregister_plan(id).await;
    if let Some(p) = plans::find(&st.pool, id).await? {
        schedule::register_plan(&st, &p).await;
    }
    Ok(ok_true("บันทึกแล้ว"))
}

pub async fn remove(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    if plans::find(&st.pool, id).await?.is_none() {
        return Ok(ok_false("ไม่พบแผน"));
    }
    if running::is_running(id) {
        return Ok(ok_false("แผนนี้กำลังสำรองข้อมูลอยู่ รอให้เสร็จก่อน"));
    }
    schedule::unregister_plan(id).await;
    plans::delete(&st.pool, id).await?;
    Ok(ok_true("ลบแผนแล้ว (ไฟล์สำรองที่ปลายทางยังอยู่)"))
}

pub async fn run_now(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    let Some(plan) = plans::find(&st.pool, id).await? else { return Ok(ok_false("ไม่พบแผน")) };
    if running::is_running(id) {
        return Ok(ok_false("กำลังสำรองข้อมูลอยู่"));
    }
    let st2 = st.clone();
    tokio::spawn(async move {
        run::run_backup(&st2, &plan, "manual").await;
    });
    Ok(ok_true("เริ่มสำรองข้อมูลแล้ว"))
}

#[derive(Deserialize)]
pub struct TestConn {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub password: String,
    pub database_name: String,
    #[serde(default)]
    pub plan_id: Option<i64>,
}

/// Runs a schema-only mysqldump with the given (or the plan's stored) password.
pub async fn test_connection(State(st): State<AppState>, Json(body): Json<TestConn>) -> Result<Json<Value>, ApiError> {
    let password = if !body.password.is_empty() {
        body.password.clone()
    } else {
        match body.plan_id {
            Some(id) => match plans::find(&st.pool, id).await? {
                Some(p) => secret::unprotect(&p.password)?,
                None => return Ok(ok_false("ไม่พบแผน")),
            },
            None => return Ok(ok_false("กรุณากรอกรหัสผ่านฐานข้อมูล")),
        }
    };
    let _ = std::fs::create_dir_all(&st.temp_dir);
    let src = mysqldump::Source { mysqldump: &st.mysqldump, database: &body.database_name, host: &body.host, port: body.port, username: &body.username, password: &password };
    Ok(match mysqldump::probe(&src, &st.temp_dir).await {
        Ok(()) => ok_true("เชื่อมต่อฐานข้อมูลสำเร็จ"),
        Err(e) => ok_false(mysqldump::error_message(e).replace(&password, "***")),
    })
}
