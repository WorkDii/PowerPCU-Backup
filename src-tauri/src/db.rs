//! The service's own SQLite database: pool type, open, migrations.

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

pub type Pool = SqlitePool;

/// Open (creating if missing) the SQLite file at `path`. `filename()` avoids URL
/// parsing of Windows drive letters.
pub async fn open(path: &Path) -> Result<Pool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    SqlitePoolOptions::new().max_connections(5).connect_with(options).await
}

/// Apply the embedded migrations (compiled in from `migrations/`).
pub async fn migrate(pool: &Pool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

/// Fresh, migrated in-memory database. Used by unit and integration tests.
pub async fn open_memory() -> Pool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");
    migrate(&pool).await.expect("migrate");
    pool
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrate_creates_tables() {
        let pool = open_memory().await;
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('storages','plans','plan_storages','runs','backup_files')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 5);
    }
}
