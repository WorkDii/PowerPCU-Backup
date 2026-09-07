//! `backup_files`: one row per stored copy. Retention (`apply_prune`) only ever
//! touches rows here — never a folder or bucket listing.

use serde::Serialize;

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
    sqlx::query_as::<_, LiveFile>("SELECT id, created_at, location FROM backup_files WHERE plan_id = ? AND storage_id = ? AND deleted_at IS NULL ORDER BY created_at DESC, id DESC")
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
}
