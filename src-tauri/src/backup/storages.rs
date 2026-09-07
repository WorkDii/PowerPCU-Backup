//! The only module that talks to the `storages` table. Handlers/plans call these
//! and never write `storages` SQL themselves.

use crate::backup::storage::{Storage, StorageConfig};
use crate::db::Pool;

type StorageRow = (i64, String, String, String); // id, name, provider, config

fn to_storage(r: StorageRow) -> Option<Storage> {
    match StorageConfig::parse(&r.2, &r.3) {
        Ok(config) => Some(Storage {
            id: r.0,
            name: r.1,
            config,
        }),
        Err(e) => {
            tracing::warn!("storage {} config parse error: {e}", r.0);
            None
        }
    }
}

/// Insert a storage; returns its new id.
pub async fn insert(
    pool: &Pool,
    name: &str,
    provider: &str,
    config_json: &str,
) -> Result<i64, sqlx::Error> {
    let r = sqlx::query("INSERT INTO storages (name, provider, config) VALUES (?, ?, ?)")
        .bind(name)
        .bind(provider)
        .bind(config_json)
        .execute(pool)
        .await?;
    Ok(r.last_insert_rowid())
}

/// All storages, newest first.
pub async fn list_all(pool: &Pool) -> Result<Vec<Storage>, sqlx::Error> {
    let rows = sqlx::query_as::<_, StorageRow>(
        "SELECT id, name, provider, config FROM storages ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().filter_map(to_storage).collect())
}

/// One storage by id, or `None`.
pub async fn find(pool: &Pool, id: i64) -> Result<Option<Storage>, sqlx::Error> {
    let row = sqlx::query_as::<_, StorageRow>(
        "SELECT id, name, provider, config FROM storages WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(to_storage))
}

/// Update name/provider/config in place.
pub async fn update(
    pool: &Pool,
    id: i64,
    name: &str,
    provider: &str,
    config_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE storages SET name = ?, provider = ?, config = ? WHERE id = ?")
        .bind(name)
        .bind(provider)
        .bind(config_json)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Remove a storage (caller must check [`is_in_use`] first).
pub async fn delete(pool: &Pool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM storages WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// True if any plan links this storage, or any live (not-yet-deleted) backup file
/// is stored on it. The backup-file check keeps a destination undeletable while it
/// still holds copies — so retention/manual-delete always has the storage row (and
/// its S3 creds) available to remove those copies.
pub async fn is_in_use(pool: &Pool, id: i64) -> Result<bool, sqlx::Error> {
    let used: i64 = sqlx::query_scalar(
        "SELECT (EXISTS(SELECT 1 FROM plan_storages WHERE storage_id = ?1)
              OR EXISTS(SELECT 1 FROM backup_files WHERE storage_id = ?1 AND deleted_at IS NULL))",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok(used != 0)
}

/// Storages linked to a plan (with full config — used by the run path).
pub async fn list_for_plan(
    pool: &Pool,
    plan_id: i64,
) -> Result<Vec<Storage>, sqlx::Error> {
    let rows = sqlx::query_as::<_, StorageRow>(
        "SELECT s.id, s.name, s.provider, s.config
         FROM storages s JOIN plan_storages ps ON ps.storage_id = s.id
         WHERE ps.plan_id = ? ORDER BY s.id",
    )
    .bind(plan_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().filter_map(to_storage).collect())
}

/// Keep only ids that exist, preserving order.
pub async fn existing_ids(pool: &Pool, ids: &[i64]) -> Result<Vec<i64>, sqlx::Error> {
    let mut out = Vec::new();
    for &id in ids {
        if find(pool, id).await?.is_some() && !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> Pool {
        crate::db::open_memory().await
    }

    #[tokio::test]
    async fn insert_find_update_roundtrip() {
        let pool = pool().await;
        let id = insert(
            &pool,
            "S3 A",
            "s3",
            r#"{"endpoint":"e","region":"r","bucket":"b","access_key":"AK","secret_key":"SK"}"#,
        )
        .await
        .unwrap();
        let got = find(&pool, id).await.unwrap().unwrap();
        assert_eq!(got.name, "S3 A");
        assert_eq!(got.config.provider_str(), "s3");

        update(
            &pool,
            id,
            "S3 B",
            "s3",
            r#"{"endpoint":"e2","region":"r","bucket":"b","access_key":"AK","secret_key":"SK"}"#,
        )
        .await
        .unwrap();
        let got = find(&pool, id).await.unwrap().unwrap();
        assert_eq!(got.name, "S3 B");
    }

    #[tokio::test]
    async fn is_in_use_tracks_plan_links_and_files() {
        let pool = pool().await;
        let sid = insert(&pool, "local", "local", r#"{"path":"/tmp"}"#)
            .await
            .unwrap();
        assert!(!is_in_use(&pool, sid).await.unwrap(), "unused at first");

        sqlx::query("INSERT INTO plan_storages (plan_id, storage_id) VALUES (1, ?)")
            .bind(sid)
            .execute(&pool)
            .await
            .unwrap();
        assert!(is_in_use(&pool, sid).await.unwrap(), "linked by a plan");

        sqlx::query("DELETE FROM plan_storages WHERE storage_id = ?")
            .bind(sid)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO backup_files (run_id, plan_id, storage_id, provider, location, size_bytes, created_at) VALUES (1, 1, ?, 'local', '/tmp/x.zip', 1, '2026-01-01 00:00:00')",
        )
        .bind(sid)
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            is_in_use(&pool, sid).await.unwrap(),
            "has a live backup file"
        );
    }

    #[tokio::test]
    async fn list_for_plan_returns_only_linked() {
        let pool = pool().await;
        let a = insert(&pool, "a", "local", r#"{"path":"/a"}"#)
            .await
            .unwrap();
        let b = insert(&pool, "b", "local", r#"{"path":"/b"}"#)
            .await
            .unwrap();
        sqlx::query("INSERT INTO plan_storages (plan_id, storage_id) VALUES (7, ?)")
            .bind(a)
            .execute(&pool)
            .await
            .unwrap();
        let linked = list_for_plan(&pool, 7).await.unwrap();
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].id, a);
        assert_ne!(linked[0].id, b);
    }
}
