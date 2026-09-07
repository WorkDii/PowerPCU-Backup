//! `plans` + `plan_storages` tables and the plan shapes. Secrets are stored as the
//! `dpapi:` blobs the API layer produced; this module never encrypts or decrypts.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::backup::prune::Keep;
use crate::db::Pool;

#[derive(Debug, Clone, Deserialize)]
pub struct NewPlan {
    pub name: String,
    pub database_name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub password: String,
    pub prefix_name: String,
    #[serde(default)]
    pub encryption_password: String,
    #[serde(default)]
    pub schedule_cron: Option<String>,
    #[serde(default = "yes")]
    pub active: bool,
    #[serde(default = "yes")]
    pub catch_up: bool,
    #[serde(default = "d14")]
    pub keep_daily: u32,
    #[serde(default = "d12")]
    pub keep_weekly: u32,
    #[serde(default = "d24")]
    pub keep_monthly: u32,
    #[serde(default = "d5")]
    pub keep_yearly: u32,
    #[serde(default)]
    pub storage_ids: Vec<i64>,
    #[serde(default)]
    pub copy_password_from_plan_id: Option<i64>,
}
fn yes() -> bool { true }
fn d14() -> u32 { 14 }
fn d12() -> u32 { 12 }
fn d24() -> u32 { 24 }
fn d5() -> u32 { 5 }

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Plan {
    pub id: i64,
    pub name: String,
    pub database_name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub password: String,
    pub prefix_name: String,
    pub encryption_password: String,
    pub schedule_cron: Option<String>,
    pub active: bool,
    pub catch_up: bool,
    pub keep_daily: i64,
    pub keep_weekly: i64,
    pub keep_monthly: i64,
    pub keep_yearly: i64,
    pub created_at: String,
}

impl Plan {
    pub fn keep(&self) -> Keep {
        Keep {
            daily: self.keep_daily.max(0) as u32,
            weekly: self.keep_weekly.max(0) as u32,
            monthly: self.keep_monthly.max(0) as u32,
            yearly: self.keep_yearly.max(0) as u32,
        }
    }

    /// API shape: both secrets masked to `""`, plus `storage_ids`.
    pub fn to_public_value(&self, storage_ids: &[i64]) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            o.insert("password".into(), json!(""));
            o.insert("encryption_password".into(), json!(""));
            o.insert("storage_ids".into(), json!(storage_ids));
        }
        v
    }
}

const COLS: &str = "id, name, database_name, host, port, username, password, prefix_name, encryption_password, schedule_cron, active, catch_up, keep_daily, keep_weekly, keep_monthly, keep_yearly, created_at";

pub async fn insert(pool: &Pool, p: &NewPlan) -> Result<i64, sqlx::Error> {
    let r = sqlx::query(
        "INSERT INTO plans (name, database_name, host, port, username, password, prefix_name, encryption_password, schedule_cron, active, catch_up, keep_daily, keep_weekly, keep_monthly, keep_yearly)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&p.name).bind(&p.database_name).bind(&p.host).bind(p.port as i64).bind(&p.username).bind(&p.password)
    .bind(&p.prefix_name).bind(&p.encryption_password).bind(&p.schedule_cron).bind(p.active).bind(p.catch_up)
    .bind(p.keep_daily as i64).bind(p.keep_weekly as i64).bind(p.keep_monthly as i64).bind(p.keep_yearly as i64)
    .execute(pool).await?;
    Ok(r.last_insert_rowid())
}

pub async fn update(pool: &Pool, id: i64, p: &NewPlan) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE plans SET name=?, database_name=?, host=?, port=?, username=?, password=?, prefix_name=?, encryption_password=?, schedule_cron=?, active=?, catch_up=?, keep_daily=?, keep_weekly=?, keep_monthly=?, keep_yearly=? WHERE id=?",
    )
    .bind(&p.name).bind(&p.database_name).bind(&p.host).bind(p.port as i64).bind(&p.username).bind(&p.password)
    .bind(&p.prefix_name).bind(&p.encryption_password).bind(&p.schedule_cron).bind(p.active).bind(p.catch_up)
    .bind(p.keep_daily as i64).bind(p.keep_weekly as i64).bind(p.keep_monthly as i64).bind(p.keep_yearly as i64)
    .bind(id).execute(pool).await?;
    Ok(())
}

pub async fn list_all(pool: &Pool) -> Result<Vec<Plan>, sqlx::Error> {
    let sql = format!("SELECT {COLS} FROM plans ORDER BY id");
    sqlx::query_as::<_, Plan>(sqlx::AssertSqlSafe(sql)).fetch_all(pool).await
}

pub async fn find(pool: &Pool, id: i64) -> Result<Option<Plan>, sqlx::Error> {
    let sql = format!("SELECT {COLS} FROM plans WHERE id = ?");
    sqlx::query_as::<_, Plan>(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(pool).await
}

/// Removes the plan and its links. Runs/files history stays.
pub async fn delete(pool: &Pool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM plan_storages WHERE plan_id = ?").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM plans WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

pub async fn replace_links(pool: &Pool, plan_id: i64, storage_ids: &[i64]) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM plan_storages WHERE plan_id = ?").bind(plan_id).execute(pool).await?;
    for &sid in storage_ids {
        sqlx::query("INSERT OR IGNORE INTO plan_storages (plan_id, storage_id) VALUES (?, ?)")
            .bind(plan_id).bind(sid).execute(pool).await?;
    }
    Ok(())
}

pub async fn storage_ids_for(pool: &Pool, plan_id: i64) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT storage_id FROM plan_storages WHERE plan_id = ? ORDER BY storage_id")
        .bind(plan_id).fetch_all(pool).await
}

#[derive(Debug, PartialEq)]
pub struct Conflict {
    pub plan_name: String,
    pub storage_name: String,
}

/// Another plan with the same (prefix_name, database_name) on one of `storage_ids`
/// would write the same file names → spec §5.2 forbids it.
pub async fn find_conflict(pool: &Pool, exclude: Option<i64>, prefix_name: &str, database_name: &str, storage_ids: &[i64]) -> Result<Option<Conflict>, sqlx::Error> {
    for &sid in storage_ids {
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT p.name, s.name FROM plans p
             JOIN plan_storages ps ON ps.plan_id = p.id
             JOIN storages s ON s.id = ps.storage_id
             WHERE p.prefix_name = ? AND p.database_name = ? AND ps.storage_id = ? AND p.id != ? LIMIT 1",
        )
        .bind(prefix_name).bind(database_name).bind(sid).bind(exclude.unwrap_or(-1))
        .fetch_optional(pool).await?;
        if let Some((plan_name, storage_name)) = row {
            return Ok(Some(Conflict { plan_name, storage_name }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn new_plan(name: &str, prefix: &str, storage_ids: Vec<i64>) -> NewPlan {
        NewPlan {
            name: name.into(), database_name: "jhcisdb".into(), host: "127.0.0.1".into(), port: 3333,
            username: "root".into(), password: "pw".into(), prefix_name: prefix.into(), encryption_password: "zipzipzip".into(),
            schedule_cron: Some("30 3 * * *".into()), active: true, catch_up: true,
            keep_daily: 14, keep_weekly: 12, keep_monthly: 24, keep_yearly: 5, storage_ids, copy_password_from_plan_id: None,
        }
    }

    #[tokio::test]
    async fn insert_find_update_delete_round_trip() {
        let pool = crate::db::open_memory().await;
        let id = insert(&pool, &new_plan("a", "10999", vec![])).await.unwrap();
        let p = find(&pool, id).await.unwrap().unwrap();
        assert_eq!(p.name, "a");
        assert_eq!(p.keep().weekly, 12);
        let v = p.to_public_value(&[1, 2]);
        assert_eq!(v["password"], "");
        assert_eq!(v["encryption_password"], "");
        assert_eq!(v["storage_ids"], serde_json::json!([1, 2]));

        let mut upd = new_plan("b", "10999", vec![]);
        upd.keep_weekly = 0;
        update(&pool, id, &upd).await.unwrap();
        assert_eq!(find(&pool, id).await.unwrap().unwrap().keep_weekly, 0);

        replace_links(&pool, id, &[5, 6]).await.unwrap();
        assert_eq!(storage_ids_for(&pool, id).await.unwrap(), vec![5, 6]);
        delete(&pool, id).await.unwrap();
        assert!(find(&pool, id).await.unwrap().is_none());
        assert!(storage_ids_for(&pool, id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn conflict_same_prefix_db_storage_across_plans() {
        let pool = crate::db::open_memory().await;
        let s = crate::backup::storages::insert(&pool, "โฟลเดอร์", "local", r#"{"path":"C:\\b"}"#).await.unwrap();
        let a = insert(&pool, &new_plan("ส่วนกลาง", "10999", vec![])).await.unwrap();
        replace_links(&pool, a, &[s]).await.unwrap();

        let c = find_conflict(&pool, None, "10999", "jhcisdb", &[s]).await.unwrap().unwrap();
        assert_eq!(c, Conflict { plan_name: "ส่วนกลาง".into(), storage_name: "โฟลเดอร์".into() });
        // same plan editing itself is not a conflict
        assert!(find_conflict(&pool, Some(a), "10999", "jhcisdb", &[s]).await.unwrap().is_none());
        // different prefix is fine
        assert!(find_conflict(&pool, None, "10998", "jhcisdb", &[s]).await.unwrap().is_none());
    }
}
