//! `--service` boot (spec §3): data dir → SQLite + migrations → mark stale runs failed →
//! scheduler → catch-up → axum on 127.0.0.1:<port>. Logs go to stdout; NSSM (Plan 3)
//! writes and rotates the file.

use std::net::SocketAddr;

use tracing_subscriber::fmt::time::ChronoLocal;

use crate::backup::{runs, schedule};
use crate::state::AppState;
use crate::{api, db, paths};

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_timer(ChronoLocal::new("%Y-%m-%d %H:%M:%S".into()))
        .with_ansi(false)
        .with_target(false)
        .init();

    let data = paths::data_dir();
    std::fs::create_dir_all(&data)?;
    // Nothing can still be running at boot, so any leftovers from a crash/kill (multi-GB .sql, probe_*.sql) are stale — wipe them.
    let _ = std::fs::remove_dir_all(paths::temp_dir());
    std::fs::create_dir_all(paths::temp_dir())?;
    let db_path = paths::db_path();
    let pool = db::open(&db_path).await?;
    db::migrate(&pool).await?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), db = %db_path.display(), "service: database ready");

    let stale = runs::fail_stale_running(&pool).await?;
    if stale > 0 {
        tracing::warn!("service: {stale} run(s) were still 'running' from the previous process → marked failed");
    }

    let mysqldump = paths::mysqldump_path();
    if !mysqldump.exists() {
        tracing::warn!(path = %mysqldump.display(), "service: mysqldump.exe not found — backups will fail until it is (set MYSQLDUMP_PATH or install bin\\mysqldump.exe)");
    }
    let state = AppState::new(pool, mysqldump, paths::temp_dir());

    if let Err(e) = schedule::start(state.clone()).await {
        tracing::error!("service: scheduler failed to start, API keeps serving: {e}");
    }
    schedule::catch_up(state.clone()).await;

    let addr = SocketAddr::from(([127, 0, 0, 1], paths::port()));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("service: listening on http://{addr}/");
    axum::serve(listener, api::router(state)).await?;
    Ok(())
}
