//! `backup_files`: one row per stored copy. Retention (`apply_prune`) only ever
//! touches rows here — never a folder or bucket listing.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use serde::Serialize;

use crate::backup::plans::Plan;
use crate::backup::prune::{self, Candidate, Reason};
use crate::backup::storage::{self, Storage};
use crate::db::Pool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct BackupFile {
    pub id: i64,
    pub run_id: i64,
    pub plan_id: i64,
    pub storage_id: i64,
    pub storage_name: Option<String>,
    pub provider: String,
    pub location: String,
    pub size_bytes: i64,
    pub created_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LiveFile {
    pub id: i64,
    pub created_at: String,
    pub location: String,
    pub provider: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn record(pool: &Pool, run_id: i64, plan_id: i64, storage_id: i64, provider: &str, location: &str, size: i64, created_at: &str) -> Result<i64, sqlx::Error> {
    let r = sqlx::query("INSERT INTO backup_files (run_id, plan_id, storage_id, provider, location, size_bytes, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(run_id).bind(plan_id).bind(storage_id).bind(provider).bind(location).bind(size).bind(created_at)
        .execute(pool).await?;
    Ok(r.last_insert_rowid())
}

/// Not-yet-deleted copies of one plan on one storage, newest first.
pub async fn live_for(pool: &Pool, plan_id: i64, storage_id: i64) -> Result<Vec<LiveFile>, sqlx::Error> {
    sqlx::query_as::<_, LiveFile>("SELECT id, created_at, location, provider FROM backup_files WHERE plan_id = ? AND storage_id = ? AND deleted_at IS NULL ORDER BY created_at DESC, id DESC")
        .bind(plan_id).bind(storage_id).fetch_all(pool).await
}

pub async fn mark_deleted(pool: &Pool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE backup_files SET deleted_at = ? WHERE id = ?")
        .bind(crate::backup::runs::now_string()).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn list_for_run(pool: &Pool, run_id: i64) -> Result<Vec<BackupFile>, sqlx::Error> {
    sqlx::query_as::<_, BackupFile>(
        "SELECT bf.id, bf.run_id, bf.plan_id, bf.storage_id, s.name AS storage_name, bf.provider, bf.location, bf.size_bytes, bf.created_at, bf.deleted_at
         FROM backup_files bf LEFT JOIN storages s ON s.id = bf.storage_id WHERE bf.run_id = ? ORDER BY bf.id",
    )
    .bind(run_id).fetch_all(pool).await
}

fn parse_ts(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()
}

fn candidates(live: &[LiveFile]) -> Vec<Candidate> {
    live.iter()
        .filter_map(|f| parse_ts(&f.created_at).map(|t| Candidate { id: f.id, created_at: t }))
        .collect()
}

/// Delete this plan's copies on `storage` that no keep rule claims. Each successful
/// provider delete (NotFound counts) marks the row deleted; failures are logged and
/// retried next run. Returns how many were deleted.
pub async fn apply_prune(pool: &Pool, plan: &Plan, storage: &Storage, now: NaiveDateTime) -> Result<usize, String> {
    let live = live_for(pool, plan.id, storage.id).await.map_err(|e| e.to_string())?;
    let decision = prune::decide(&candidates(&live), &plan.keep(), now);
    if decision.delete.is_empty() {
        return Ok(0);
    }
    let provider = storage::for_storage(storage)?;
    let mut deleted = 0;
    for id in decision.delete {
        let Some(file) = live.iter().find(|f| f.id == id) else { continue };
        if file.provider != storage.config.provider_str() {
            tracing::warn!(plan = plan.id, storage = storage.id, location = %file.location, "retention: provider changed since this copy was stored; leaving it");
            continue;
        }
        match provider.delete(&file.location).await {
            Ok(()) => {
                mark_deleted(pool, id).await.map_err(|e| e.to_string())?;
                tracing::info!(plan = plan.id, storage = storage.id, location = %file.location, "retention: deleted");
                deleted += 1;
            }
            Err(e) => tracing::warn!(plan = plan.id, location = %file.location, "retention: delete failed, will retry: {e}"),
        }
    }
    Ok(deleted)
}

/// Two runs in the same minute produce the same file name, so the second `store`
/// overwrites the first copy. Retire the old rows first, or prune would later delete
/// the fresh file through them.
pub async fn supersede(pool: &Pool, plan_id: i64, storage_id: i64, location: &str) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("UPDATE backup_files SET deleted_at = ? WHERE plan_id = ? AND storage_id = ? AND location = ? AND deleted_at IS NULL")
        .bind(crate::backup::runs::now_string()).bind(plan_id).bind(storage_id).bind(location)
        .execute(pool).await?;
    Ok(r.rows_affected())
}

/// Dry run for the history page: which rules currently keep each live file.
pub async fn kept_as(pool: &Pool, plan: &Plan, storage_id: i64, now: NaiveDateTime) -> Result<HashMap<i64, Vec<Reason>>, String> {
    let live = live_for(pool, plan.id, storage_id).await.map_err(|e| e.to_string())?;
    let decision = prune::decide(&candidates(&live), &plan.keep(), now);
    Ok(decision.keep.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn record_live_mark_deleted_list() {
        let pool = crate::db::open_memory().await;
        let a = record(&pool, 1, 1, 9, "local", r"C:\b\a.zip", 10, "2026-09-01 03:30:00").await.unwrap();
        let _b = record(&pool, 1, 1, 9, "local", r"C:\b\b.zip", 10, "2026-09-02 03:30:00").await.unwrap();
        let _other_plan = record(&pool, 2, 2, 9, "local", r"C:\b\c.zip", 10, "2026-09-03 03:30:00").await.unwrap();

        let live = live_for(&pool, 1, 9).await.unwrap();
        assert_eq!(live.iter().map(|f| f.location.as_str()).collect::<Vec<_>>(), vec![r"C:\b\b.zip", r"C:\b\a.zip"], "newest first, own plan only");

        mark_deleted(&pool, a).await.unwrap();
        assert_eq!(live_for(&pool, 1, 9).await.unwrap().len(), 1);
        let listed = list_for_run(&pool, 1).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed[0].deleted_at.is_some() && listed[1].deleted_at.is_none());
    }

    #[tokio::test]
    async fn apply_prune_deletes_only_own_plan_files_in_shared_folder() {
        use crate::backup::plans::NewPlan;
        let pool = crate::db::open_memory().await;
        let dir = std::env::temp_dir().join(format!("ppb-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sid = crate::backup::storages::insert(&pool, "local", "local", &format!(r#"{{"path":{}}}"#, serde_json::to_string(&dir.to_string_lossy()).unwrap())).await.unwrap();
        let storage = crate::backup::storages::find(&pool, sid).await.unwrap().unwrap();
        let mut np = NewPlan { name: "a".into(), database_name: "db".into(), host: "h".into(), port: 1, username: "u".into(), password: "p".into(), prefix_name: "pre".into(), encryption_password: "e".into(), schedule_cron: None, active: true, catch_up: true, keep_daily: 1, keep_weekly: 0, keep_monthly: 0, keep_yearly: 0, storage_ids: vec![], copy_password_from_plan_id: None };
        let a = crate::backup::plans::insert(&pool, &np).await.unwrap();
        np.name = "b".into();
        let b = crate::backup::plans::insert(&pool, &np).await.unwrap();
        let plan_a = crate::backup::plans::find(&pool, a).await.unwrap().unwrap();

        let old_a = dir.join("a_old.zip");
        let new_a = dir.join("a_new.zip");
        let old_b = dir.join("b_old.zip");
        for f in [&old_a, &new_a, &old_b] { std::fs::write(f, b"x").unwrap(); }
        record(&pool, 1, a, sid, "local", &old_a.to_string_lossy(), 1, "2026-09-01 03:30:00").await.unwrap();
        record(&pool, 2, a, sid, "local", &new_a.to_string_lossy(), 1, "2026-09-02 03:30:00").await.unwrap();
        record(&pool, 3, b, sid, "local", &old_b.to_string_lossy(), 1, "2026-09-01 03:30:00").await.unwrap();

        let now = chrono::NaiveDate::from_ymd_opt(2026, 9, 7).unwrap().and_hms_opt(12, 0, 0).unwrap();
        assert_eq!(apply_prune(&pool, &plan_a, &storage, now).await.unwrap(), 1);
        assert!(!old_a.exists() && new_a.exists() && old_b.exists(), "only plan a's old copy removed");
        assert_eq!(live_for(&pool, a, sid).await.unwrap().len(), 1);
        assert_eq!(live_for(&pool, b, sid).await.unwrap().len(), 1);
        let kept = kept_as(&pool, &plan_a, sid, now).await.unwrap();
        assert_eq!(kept.len(), 1);
        assert!(kept.values().next().unwrap().contains(&Reason::Daily));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn apply_prune_leaves_row_whose_provider_no_longer_matches_storage() {
        use crate::backup::plans::NewPlan;
        let pool = crate::db::open_memory().await;
        let dir = std::env::temp_dir().join(format!("ppb-prune-provider-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sid = crate::backup::storages::insert(&pool, "local", "local", &format!(r#"{{"path":{}}}"#, serde_json::to_string(&dir.to_string_lossy()).unwrap())).await.unwrap();
        let storage = crate::backup::storages::find(&pool, sid).await.unwrap().unwrap();
        let np = NewPlan { name: "a".into(), database_name: "db".into(), host: "h".into(), port: 1, username: "u".into(), password: "p".into(), prefix_name: "pre".into(), encryption_password: "e".into(), schedule_cron: None, active: true, catch_up: true, keep_daily: 1, keep_weekly: 0, keep_monthly: 0, keep_yearly: 0, storage_ids: vec![], copy_password_from_plan_id: None };
        let plan_id = crate::backup::plans::insert(&pool, &np).await.unwrap();
        let plan = crate::backup::plans::find(&pool, plan_id).await.unwrap().unwrap();

        // Old copy was written when this storage was still s3; the storage's config
        // has since been changed to local (same row id, different provider).
        record(&pool, 1, plan_id, sid, "s3", "s3://old/x.zip", 1, "2026-09-01 03:30:00").await.unwrap();
        let new_zip = dir.join("new.zip");
        std::fs::write(&new_zip, b"x").unwrap();
        record(&pool, 2, plan_id, sid, "local", &new_zip.to_string_lossy(), 1, "2026-09-02 03:30:00").await.unwrap();

        let now = chrono::NaiveDate::from_ymd_opt(2026, 9, 7).unwrap().and_hms_opt(12, 0, 0).unwrap();
        // keep_daily = 1 keeps only the newer (local) day-bucket, so the s3 row is
        // the one prune wants to delete — but its provider no longer matches.
        let deleted = apply_prune(&pool, &plan, &storage, now).await.unwrap();
        assert_eq!(deleted, 0, "a provider mismatch must not count as a deletion");

        let live = live_for(&pool, plan_id, sid).await.unwrap();
        assert!(live.iter().any(|f| f.location == "s3://old/x.zip"), "the s3 row must stay live, not be silently deleted through the local provider");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
