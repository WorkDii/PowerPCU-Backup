//! Everything the engine and API need at runtime, passed explicitly (no globals,
//! no env reads) so tests can point `mysqldump` at a fake script.

use std::path::PathBuf;

use crate::db::Pool;

#[derive(Clone)]
pub struct AppState {
    pub pool: Pool,
    /// Full path to `mysqldump.exe` (or a fake `.cmd` in tests).
    pub mysqldump: PathBuf,
    /// Working folder for `.sql` / `.zip` during a run.
    pub temp_dir: PathBuf,
    /// Local-time `YYYY-MM-DD HH:MM:SS` of service start (shown by `/api/status`).
    pub started_at: String,
}

impl AppState {
    pub fn new(pool: Pool, mysqldump: PathBuf, temp_dir: PathBuf) -> Self {
        AppState {
            pool,
            mysqldump,
            temp_dir,
            started_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        }
    }
}
