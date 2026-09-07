//! One backup run (spec §6): claim → runs row → dump (one at a time) → zip → ship to
//! every destination → record → prune per destination → close the runs row.

use chrono::Local;
use tokio::sync::Semaphore;

use crate::backup::{archive, files, mysqldump, plans::Plan, running, runs, storage, storages};
use crate::secret;
use crate::state::AppState;

/// Only one mysqldump at a time across all plans (two plans on one DB at 03:30).
static DUMP_SEMAPHORE: Semaphore = Semaphore::const_new(1);

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub run_id: i64,
    pub status: String,
    pub message: String,
}

struct RunningGuard(i64);
impl Drop for RunningGuard {
    fn drop(&mut self) {
        running::end(self.0);
    }
}

pub async fn run_backup(state: &AppState, plan: &Plan, trigger: &str) -> RunOutcome {
    if !running::try_start(plan.id) {
        return RunOutcome { run_id: 0, status: "skipped".into(), message: "กำลังสำรองข้อมูลอยู่".into() };
    }
    let _guard = RunningGuard(plan.id);

    let run_id = match runs::start(&state.pool, plan.id, trigger).await {
        Ok(id) => id,
        Err(e) => return RunOutcome { run_id: 0, status: "failed".into(), message: format!("บันทึกประวัติไม่สำเร็จ: {e}") },
    };
    tracing::info!(plan = plan.id, run = run_id, trigger, "backup: start");

    let (status, message, size) = match run_inner(state, plan, run_id).await {
        Ok((s, m, size)) => (s, m, Some(size)),
        Err(m) => ("failed", m, None),
    };
    if let Err(e) = runs::finish(&state.pool, run_id, status, &message, size).await {
        tracing::error!(run = run_id, "backup: could not close run row: {e}");
    }
    if let Err(e) = runs::prune_old(&state.pool, plan.id, 90).await {
        tracing::warn!(plan = plan.id, "runs: prune_old failed: {e}");
    }
    tracing::info!(plan = plan.id, run = run_id, status, "backup: {message}");
    RunOutcome { run_id, status: status.into(), message }
}

async fn run_inner(state: &AppState, plan: &Plan, run_id: i64) -> Result<(&'static str, String, i64), String> {
    let destinations = storages::list_for_plan(&state.pool, plan.id).await.map_err(|e| format!("อ่านรายการปลายทางไม่สำเร็จ: {e}"))?;
    if destinations.is_empty() {
        return Err("แผนนี้ยังไม่ได้เลือกปลายทาง".into());
    }
    let db_password = secret::unprotect(&plan.password)?;
    let zip_password = secret::unprotect(&plan.encryption_password)?;
    std::fs::create_dir_all(&state.temp_dir).map_err(|e| format!("สร้างโฟลเดอร์ชั่วคราวไม่สำเร็จ: {e}"))?;

    let now = Local::now();
    let base = format!("{}_{}_{}", plan.prefix_name, plan.database_name, now.format("%y%m%d%H%M"));
    let sql_path = state.temp_dir.join(format!("plan{}_{base}.sql", plan.id));

    // §6 step 4.5: dumps are serialized across plans.
    {
        let _permit = match DUMP_SEMAPHORE.try_acquire() {
            Ok(p) => p,
            Err(_) => {
                let _ = runs::set_message(&state.pool, run_id, "รอคิว dump").await;
                DUMP_SEMAPHORE.acquire().await.map_err(|e| e.to_string())?
            }
        };
        let src = mysqldump::Source { mysqldump: &state.mysqldump, database: &plan.database_name, host: &plan.host, port: plan.port as u16, username: &plan.username, password: &db_password };
        mysqldump::dump(&src, &sql_path).await.map_err(|e| redact(mysqldump::error_message(e), &db_password))?;
    }

    let entry = format!("{base}.sql");
    let (sql_for_zip, pw) = (sql_path.clone(), zip_password);
    let zipped = tokio::task::spawn_blocking(move || archive::compress_to_zip(&sql_for_zip, &entry, &pw)).await;
    let _ = std::fs::remove_file(&sql_path);
    let zip_path = match zipped {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return Err(format!("บีบอัดไฟล์ไม่สำเร็จ: {e}")),
        Err(e) => return Err(format!("บีบอัดไฟล์ไม่สำเร็จ: {e}")),
    };
    let size = std::fs::metadata(&zip_path).map(|m| m.len() as i64).unwrap_or(0);
    let file_name = format!("{base}.zip");
    let created_at = now.format("%Y-%m-%d %H:%M:%S").to_string();

    let mut ok = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for dest in &destinations {
        let rel = storage::rel_path(&plan.prefix_name, &plan.database_name, &file_name);
        let stored = match storage::for_storage(dest) {
            Ok(p) => p.store(&zip_path, &rel).await,
            Err(m) => Err(m),
        };
        match stored {
            Ok(location) => {
                ok += 1;
                if let Err(e) = files::supersede(&state.pool, plan.id, dest.id, &location).await {
                    tracing::warn!(run = run_id, "backup: supersede failed for {location}: {e}");
                }
                if let Err(e) = files::record(&state.pool, run_id, plan.id, dest.id, dest.config.provider_str(), &location, size, &created_at).await {
                    tracing::error!(run = run_id, "backup: record failed for {location}: {e}");
                }
                match files::apply_prune(&state.pool, plan, dest, now.naive_local()).await {
                    Ok(n) if n > 0 => tracing::info!(plan = plan.id, storage = dest.id, "retention: {n} file(s) pruned"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(plan = plan.id, storage = dest.id, "retention failed: {e}"),
                }
            }
            Err(m) => failures.push(format!("{}: {m}", dest.name)),
        }
    }
    let _ = std::fs::remove_file(&zip_path);

    let total = destinations.len();
    if ok == total {
        Ok(("ok", format!("สำเร็จ {ok}/{total} ปลายทาง"), size))
    } else if ok == 0 {
        Err(format!("ล้มเหลวทุกปลายทาง: {}", failures.join("; ")))
    } else {
        Ok(("partial", format!("สำเร็จ {ok}/{total} ปลายทาง; {}", failures.join("; ")), size))
    }
}

/// Never let the DB password reach a log or the history table.
fn redact(msg: String, secret: &str) -> String {
    if secret.is_empty() { msg } else { msg.replace(secret, "***") }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::backup::plans::{self, NewPlan};

    /// `running::RUNNING` is a process-global set keyed by plan_id, and every fresh
    /// `db::open_memory()` restarts the autoincrement at 1 — so two of these tests running
    /// in parallel (the default `cargo test` behavior) would likely claim the same plan_id
    /// and spuriously see each other as "already running". Serialize this module's tests.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn temp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ppb-run-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    async fn setup(tag: &str, exit_code: i32) -> (AppState, Plan, std::path::PathBuf, std::path::PathBuf) {
        let pool = crate::db::open_memory().await;
        let work = temp(tag);
        let dest_dir = work.join("dest");
        let temp_dir = work.join("temp");
        std::fs::create_dir_all(&temp_dir).unwrap();
        // Fake mysqldump: argv is `db --single-transaction --routines --events --triggers
        // --result-file=<path> …`, so %~6 is the result-file flag (the stamped path is only
        // known at run time). Rust quotes .cmd/.bat arguments as a CVE-2024-24576 mitigation,
        // so %~6 is the whole `--result-file=<path>` token, not split on `=`; a `for /f` on
        // `=` pulls out everything after the first `=` (the path may itself contain none).
        let script = work.join("mysqldump.cmd");
        let body = if exit_code == 0 {
            "@echo off\r\nfor /f \"tokens=1,* delims==\" %%A in (\"%~6\") do set \"rf=%%B\"\r\necho -- fake dump> \"%rf%\"\r\nexit /b 0\r\n".to_string()
        } else {
            format!("@echo off\r\necho mysqldump: Got error: 1045: Access denied 1>&2\r\nexit /b {exit_code}\r\n")
        };
        std::fs::write(&script, body).unwrap();

        let sid = crate::backup::storages::insert(&pool, "โฟลเดอร์", "local", &format!(r#"{{"path":{}}}"#, serde_json::to_string(&dest_dir.to_string_lossy()).unwrap())).await.unwrap();
        let np = NewPlan { name: "t".into(), database_name: "jhcisdb".into(), host: "127.0.0.1".into(), port: 3333, username: "root".into(), password: "pw".into(), prefix_name: "10999".into(), encryption_password: "zipzipzip".into(), schedule_cron: None, active: true, catch_up: true, keep_daily: 1, keep_weekly: 0, keep_monthly: 0, keep_yearly: 0, storage_ids: vec![], copy_password_from_plan_id: None };
        let pid = plans::insert(&pool, &np).await.unwrap();
        plans::replace_links(&pool, pid, &[sid]).await.unwrap();
        let plan = plans::find(&pool, pid).await.unwrap().unwrap();
        (AppState::new(pool, script, temp_dir), plan, work, dest_dir)
    }

    #[tokio::test]
    async fn full_run_stores_zip_records_and_prunes() {
        let _serial = TEST_LOCK.lock().unwrap();
        let (state, plan, work, dest_dir) = setup("ok", 0).await;
        let out = run_backup(&state, &plan, "manual").await;
        assert_eq!(out.status, "ok", "{}", out.message);
        let run = runs::find(&state.pool, out.run_id).await.unwrap().unwrap();
        assert_eq!(run.status, "ok");
        assert!(run.size_bytes.unwrap() > 0);

        let files = files::list_for_run(&state.pool, out.run_id).await.unwrap();
        assert_eq!(files.len(), 1);
        let loc = std::path::PathBuf::from(&files[0].location);
        assert!(loc.starts_with(dest_dir.join("10999").join("jhcisdb")), "nested prefix/db layout: {loc:?}");
        assert!(loc.exists());
        assert!(!std::fs::read_dir(&state.temp_dir).unwrap().any(|e| e.unwrap().path().extension().is_some()), "temp cleaned");

        // second run in the same minute → same file name: the old row is superseded,
        // keep_daily=1 keeps exactly one live row, and the (overwritten) file still exists
        let out2 = run_backup(&state, &plan, "manual").await;
        assert_eq!(out2.status, "ok");
        let live = files::live_for(&state.pool, plan.id, files[0].storage_id).await.unwrap();
        assert_eq!(live.len(), 1);
        // Same-minute runs share a file name, so if the clock rolled over to a new
        // minute between the two runs, the surviving row points at a different file
        // than `loc` — check the row's own location, not the first run's `loc`.
        assert!(std::path::Path::new(&live[0].location).exists(), "prune must not delete the file the surviving row points at");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[tokio::test]
    async fn dump_failure_is_failed_run_with_thai_message() {
        let _serial = TEST_LOCK.lock().unwrap();
        let (state, plan, work, _) = setup("fail", 2).await;
        let out = run_backup(&state, &plan, "schedule").await;
        assert_eq!(out.status, "failed");
        assert!(out.message.contains("Access denied"));
        assert!(runs::find(&state.pool, out.run_id).await.unwrap().unwrap().finished_at.is_some());
        assert!(!running::is_running(plan.id));
        let _ = std::fs::remove_dir_all(&work);
    }

    #[tokio::test]
    async fn no_destination_fails_fast() {
        let _serial = TEST_LOCK.lock().unwrap();
        let (state, plan, work, _) = setup("nodest", 0).await;
        plans::replace_links(&state.pool, plan.id, &[]).await.unwrap();
        let out = run_backup(&state, &plan, "manual").await;
        assert_eq!(out.status, "failed");
        assert!(out.message.contains("ยังไม่ได้เลือกปลายทาง"));
        let _ = std::fs::remove_dir_all(&work);
    }

    #[tokio::test]
    async fn two_plans_dump_one_at_a_time() {
        let _serial = TEST_LOCK.lock().unwrap();
        let (state, plan, work, _) = setup("sem", 0).await;
        // A slow fake: sleep 2s before writing, so overlapping runs would overlap here.
        let slow = work.join("slow.cmd");
        std::fs::write(&slow, "@echo off\r\nfor /f \"tokens=1,* delims==\" %%A in (\"%~6\") do set \"rf=%%B\"\r\nping -n 3 127.0.0.1 >nul\r\necho -- fake dump> \"%rf%\"\r\nexit /b 0\r\n").unwrap();
        let state = AppState { mysqldump: slow, ..state };
        let mut p2 = plan.clone();
        p2.id = plans::insert(&state.pool, &NewPlan { name: "p2".into(), database_name: "jhcisdb".into(), host: "h".into(), port: 1, username: "u".into(), password: "p".into(), prefix_name: "10998".into(), encryption_password: "zipzipzip".into(), schedule_cron: None, active: true, catch_up: true, keep_daily: 1, keep_weekly: 0, keep_monthly: 0, keep_yearly: 0, storage_ids: vec![], copy_password_from_plan_id: None }).await.unwrap();
        plans::replace_links(&state.pool, p2.id, &plans::storage_ids_for(&state.pool, plan.id).await.unwrap()).await.unwrap();
        let p2 = plans::find(&state.pool, p2.id).await.unwrap().unwrap();

        let t0 = std::time::Instant::now();
        let (a, b) = tokio::join!(run_backup(&state, &plan, "manual"), run_backup(&state, &p2, "manual"));
        assert_eq!((a.status.as_str(), b.status.as_str()), ("ok", "ok"), "{} / {}", a.message, b.message);
        assert!(t0.elapsed() >= std::time::Duration::from_secs(4), "serialized: two 2s dumps take ≥4s, took {:?}", t0.elapsed());
        let _ = std::fs::remove_dir_all(&work);
    }
}
