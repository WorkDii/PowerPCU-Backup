//! Drives the real router in-process (no port) with an in-memory DB and a fake mysqldump.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use powerpcu_backup::api::{self, CLIENT_HEADER, CLIENT_TOKEN};
use powerpcu_backup::state::AppState;

struct Env {
    app: Router,
    work: std::path::PathBuf,
    dest: std::path::PathBuf,
}

async fn setup(tag: &str) -> Env {
    let pool = powerpcu_backup::db::open_memory().await;
    let work = std::env::temp_dir().join(format!("ppb-api-{}-{tag}", std::process::id()));
    let temp = work.join("temp");
    let dest = work.join("dest");
    std::fs::create_dir_all(&temp).unwrap();
    let fake = work.join("mysqldump.cmd");
    std::fs::write(&fake, "@echo off\r\nfor /f \"tokens=1,* delims==\" %%A in (\"%~6\") do set \"rf=%%B\"\r\necho -- fake dump> \"%rf%\"\r\nexit /b 0\r\n").unwrap();
    Env { app: api::router(AppState::new(pool, fake, temp)), work, dest }
}

async fn call(app: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(path).header(CLIENT_HEADER, CLIENT_TOKEN);
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn plan_body(name: &str, prefix: &str, storage_ids: Vec<i64>) -> Value {
    json!({
        "name": name, "database_name": "jhcisdb", "host": "127.0.0.1", "port": 3333, "username": "root",
        "password": "secret-pw", "prefix_name": prefix, "encryption_password": "zip-pass-123",
        "schedule_cron": "30 3 * * *", "storage_ids": storage_ids, "keep_daily": 1, "keep_weekly": 0, "keep_monthly": 0, "keep_yearly": 0
    })
}

#[tokio::test]
async fn guard_rejects_missing_header_and_serves_index() {
    let e = setup("guard").await;
    let res = e.app.clone().oneshot(Request::get("/api/status").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = e.app.clone().oneshot(Request::get("/").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["cache-control"], "no-cache");
    let _ = std::fs::remove_dir_all(&e.work);
}

#[tokio::test]
async fn full_flow_storage_plan_run_history() {
    let e = setup("flow").await;
    let app = &e.app;

    // storage
    let (_, v) = call(app, "POST", "/api/storages", Some(json!({"name": "โฟลเดอร์", "provider": "local", "config": {"path": e.dest.to_string_lossy()}}))).await;
    assert_eq!(v["ok"], true, "{v}");
    let sid = v["id"].as_i64().unwrap();
    let (_, v) = call(app, "POST", &format!("/api/storages/{sid}/test"), None).await;
    assert_eq!(v["ok"], true, "{v}");
    let (_, v) = call(app, "GET", "/api/storages", None).await;
    assert_eq!(v[0]["in_use"], false);

    // plan validation
    let mut bad = plan_body("x", "10999", vec![sid]);
    bad["password"] = json!("");
    let (_, v) = call(app, "POST", "/api/plans", Some(bad)).await;
    assert_eq!(v["ok"], false);
    assert!(v["message"].as_str().unwrap().contains("รหัสผ่านฐานข้อมูล"));
    let mut bad = plan_body("x", "10999", vec![sid]);
    bad["encryption_password"] = json!("short");
    let (_, v) = call(app, "POST", "/api/plans", Some(bad)).await;
    assert_eq!(v["ok"], false);
    let (_, v) = call(app, "POST", "/api/plans", Some(plan_body("x", "bad prefix!", vec![sid]))).await;
    assert_eq!(v["ok"], false);
    let mut bad = plan_body("x", "10999", vec![sid]);
    bad["database_name"] = json!("../x");
    let (_, v) = call(app, "POST", "/api/plans", Some(bad)).await;
    assert_eq!(v["ok"], false);
    let mut bad = plan_body("x", "10999", vec![sid]);
    bad["schedule_cron"] = json!("99 99 * * *");
    let (_, v) = call(app, "POST", "/api/plans", Some(bad)).await;
    assert_eq!(v["ok"], false);
    assert!(v["message"].as_str().unwrap().contains("cron"), "{v}");

    // plan ok, masked on read
    let (_, v) = call(app, "POST", "/api/plans", Some(plan_body("รพ.สต.", "10999", vec![sid]))).await;
    assert_eq!(v["ok"], true, "{v}");
    let pid = v["id"].as_i64().unwrap();
    let (_, v) = call(app, "GET", &format!("/api/plans/{pid}"), None).await;
    assert_eq!(v["password"], "");
    assert_eq!(v["encryption_password"], "");
    assert_eq!(v["storage_ids"], json!([sid]));

    // conflict: same prefix+db on same storage
    let (_, v) = call(app, "POST", "/api/plans", Some(plan_body("ซ้ำ", "10999", vec![sid]))).await;
    assert_eq!(v["ok"], false);
    assert!(v["message"].as_str().unwrap().contains("รพ.สต."));

    // duplicate plan copying the password server-side
    let mut dup = plan_body("ส่วนกลาง", "10999-c", vec![sid]);
    dup["password"] = json!("");
    dup["copy_password_from_plan_id"] = json!(pid);
    let (_, v) = call(app, "POST", "/api/plans", Some(dup)).await;
    assert_eq!(v["ok"], true, "{v}");

    // test-connection via fake mysqldump using the stored password
    let (_, v) = call(app, "POST", "/api/plans/test-connection", Some(json!({"host": "h", "port": 3333, "username": "root", "database_name": "jhcisdb", "plan_id": pid}))).await;
    assert_eq!(v["ok"], true, "{v}");

    // run + poll
    let (_, v) = call(app, "POST", &format!("/api/plans/{pid}/run"), None).await;
    assert_eq!(v["ok"], true, "{v}");
    let mut run = Value::Null;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (_, list) = call(app, "GET", &format!("/api/plans/{pid}/runs?limit=5"), None).await;
        if list[0]["status"] != "running" && !list[0].is_null() { run = list[0].clone(); break; }
    }
    assert_eq!(run["status"], "ok", "{run}");
    let run_id = run["id"].as_i64().unwrap();
    let (_, files) = call(app, "GET", &format!("/api/runs/{run_id}/files"), None).await;
    assert_eq!(files.as_array().unwrap().len(), 1);
    assert!(std::path::Path::new(files[0]["location"].as_str().unwrap()).exists());
    assert!(files[0]["kept_as"].as_array().unwrap().iter().any(|r| r == "daily"));

    // status
    let (_, s) = call(app, "GET", "/api/status", None).await;
    assert_eq!(s["plans"][0]["last_run"]["status"], "ok");
    assert_eq!(s["version"], env!("CARGO_PKG_VERSION"));

    // storage in use → cannot delete; plan delete keeps files → still in use
    let (_, v) = call(app, "DELETE", &format!("/api/storages/{sid}"), None).await;
    assert_eq!(v["ok"], false);
    let (_, v) = call(app, "DELETE", &format!("/api/plans/{pid}"), None).await;
    assert_eq!(v["ok"], true);
    let (_, v) = call(app, "DELETE", &format!("/api/storages/{sid}"), None).await;
    assert_eq!(v["ok"], false, "live backup file still references it");
    let _ = std::fs::remove_dir_all(&e.work);
}
