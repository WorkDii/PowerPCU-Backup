//! `runs` table: one row per backup attempt (history + tray/dashboard status).

use serde::Serialize;

use crate::db::Pool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Run {
    pub id: i64,
    pub plan_id: i64,
    pub trigger: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub message: String,
    pub size_bytes: Option<i64>,
}

const COLS: &str = "id, plan_id, trigger, status, started_at, finished_at, message, size_bytes";

pub fn now_string() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub async fn start(pool: &Pool, plan_id: i64, trigger: &str) -> Result<i64, sqlx::Error> {
    let r = sqlx::query("INSERT INTO runs (plan_id, trigger, status, started_at) VALUES (?, ?, 'running', ?)")
        .bind(plan_id).bind(trigger).bind(now_string()).execute(pool).await?;
    Ok(r.last_insert_rowid())
}

pub async fn set_message(pool: &Pool, id: i64, message: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE runs SET message = ? WHERE id = ?").bind(message).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn finish(pool: &Pool, id: i64, status: &str, message: &str, size: Option<i64>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE runs SET status = ?, message = ?, size_bytes = ?, finished_at = ? WHERE id = ?")
        .bind(status).bind(message).bind(size).bind(now_string()).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn find(pool: &Pool, id: i64) -> Result<Option<Run>, sqlx::Error> {
    sqlx::query_as::<_, Run>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM runs WHERE id = ?"))).bind(id).fetch_optional(pool).await
}

pub async fn list_for_plan(pool: &Pool, plan_id: i64, limit: i64) -> Result<Vec<Run>, sqlx::Error> {
    sqlx::query_as::<_, Run>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM runs WHERE plan_id = ? ORDER BY id DESC LIMIT ?")))
        .bind(plan_id).bind(limit).fetch_all(pool).await
}

pub async fn last_for_plan(pool: &Pool, plan_id: i64) -> Result<Option<Run>, sqlx::Error> {
    Ok(list_for_plan(pool, plan_id, 1).await?.into_iter().next())
}

/// `started_at` of the newest ok/partial run, if any.
pub async fn last_success_at(pool: &Pool, plan_id: i64) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT MAX(started_at) FROM runs WHERE plan_id = ? AND status IN ('ok','partial')")
        .bind(plan_id).fetch_one(pool).await
}

/// Delete history older than `days`. Returns rows removed.
pub async fn prune_old(pool: &Pool, plan_id: i64, days: i64) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("DELETE FROM runs WHERE plan_id = ? AND started_at < datetime('now', 'localtime', ?)")
        .bind(plan_id).bind(format!("-{days} days")).execute(pool).await?;
    Ok(r.rows_affected())
}

/// At boot: a row still `running` means the service died mid-backup.
pub async fn fail_stale_running(pool: &Pool) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("UPDATE runs SET status = 'failed', finished_at = ?, message = 'service หยุดระหว่างสำรองข้อมูล' WHERE status = 'running'")
        .bind(now_string()).execute(pool).await?;
    Ok(r.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lifecycle_and_queries() {
        let pool = crate::db::open_memory().await;
        let id = start(&pool, 7, "manual").await.unwrap();
        set_message(&pool, id, "รอคิว dump").await.unwrap();
        assert_eq!(find(&pool, id).await.unwrap().unwrap().status, "running");
        assert!(last_success_at(&pool, 7).await.unwrap().is_none());

        finish(&pool, id, "ok", "สำเร็จ 1/1 ปลายทาง", Some(123)).await.unwrap();
        let r = last_for_plan(&pool, 7).await.unwrap().unwrap();
        assert_eq!((r.status.as_str(), r.size_bytes), ("ok", Some(123)));
        assert!(r.finished_at.is_some());
        assert_eq!(last_success_at(&pool, 7).await.unwrap(), Some(r.started_at.clone()));
        assert_eq!(list_for_plan(&pool, 7, 50).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stale_running_failed_at_boot_and_old_pruned() {
        let pool = crate::db::open_memory().await;
        let id = start(&pool, 1, "schedule").await.unwrap();
        assert_eq!(fail_stale_running(&pool).await.unwrap(), 1);
        let r = find(&pool, id).await.unwrap().unwrap();
        assert_eq!(r.status, "failed");
        assert!(r.message.contains("service หยุด"));

        sqlx::query("INSERT INTO runs (plan_id, trigger, status, started_at) VALUES (1, 'schedule', 'ok', '2020-01-01 00:00:00')")
            .execute(&pool).await.unwrap();
        assert_eq!(prune_old(&pool, 1, 90).await.unwrap(), 1);
        assert_eq!(list_for_plan(&pool, 1, 50).await.unwrap().len(), 1);
    }
}
