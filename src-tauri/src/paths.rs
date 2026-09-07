//! Where things live on disk and which port to use. Read once at boot by
//! `service.rs`; the engine never reads env vars itself (it gets an `AppState`).

use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 8720;

/// `%POWERPCU_BACKUP_DATA_DIR%` else `%ProgramData%\PowerPCU-Backup`.
pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("POWERPCU_BACKUP_DATA_DIR") {
        return PathBuf::from(p);
    }
    let base = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    PathBuf::from(base).join("PowerPCU-Backup")
}

pub fn db_path() -> PathBuf {
    data_dir().join("powerpcu-backup.db")
}

pub fn temp_dir() -> PathBuf {
    data_dir().join("temp")
}

/// `%MYSQLDUMP_PATH%` else `<exe dir>\bin\mysqldump.exe`. In `cargo run` the exe dir is
/// `target/debug`, so set `MYSQLDUMP_PATH` (see CLAUDE.md) for local development.
pub fn mysqldump_path() -> PathBuf {
    if let Ok(p) = std::env::var("MYSQLDUMP_PATH") {
        return PathBuf::from(p);
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe_dir.join("bin").join("mysqldump.exe")
}

/// `%POWERPCU_BACKUP_PORT%` else 8720.
pub fn port() -> u16 {
    std::env::var("POWERPCU_BACKUP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}
