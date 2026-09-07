# PowerPCU Backup v2 — Plan 1: Service Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A headless `powerpcu-backup.exe --service` that dumps MySQL on a local-time cron, encrypts to zip, ships to many destinations, prunes with restic-style keep-by-rule retention, and exposes a JSON API on 127.0.0.1:8720. No GUI yet (Plan 2), no installer yet (Plan 3).

**Architecture:** One Rust crate at `src-tauri/` with a library (`powerpcu_backup`) and a thin binary. The engine (`backup/*`) is framework-free and ported from the POC's Rust module; `api/*` is a thin axum layer; `service.rs` boots DB → scheduler → catch-up → HTTP. All engine functions take an `AppState` (pool + mysqldump path + temp dir) so tests never touch env vars or real MySQL.

**Tech Stack:** Rust stable 1.96, tokio, axum 0.8, sqlx 0.9 (SQLite), tokio-cron-scheduler 0.15, zip 2 (Zstd + AES-256), aws-sdk-s3 1.x (rustls/ring), rust-embed 8, windows-sys 0.61 (DPAPI), tracing.

**Spec:** `docs/superpowers/specs/2026-09-07-powerpcu-backup-v2-design.md` (sections 3–10, 13–16, 18). Read it first.

## Global Constraints

- Windows 10+ x64 only; all tests run on Windows (DPAPI, `.cmd` fakes).
- Rust edition 2021 (so `std::env::set_var` stays safe; tests must NOT set env vars anyway — pass paths through `AppState`).
- No openssl, no aws-lc-rs, no NASM: `aws-smithy-http-client` with `rustls-ring` only.
- All user-facing strings (API `message`, run `message`) are Thai. Log lines may be English.
- Business errors are HTTP 200 `{ "ok": false, "message": "…" }`; only JSON parse failures / DB failures are 4xx/5xx.
- Secrets (`plans.password`, `plans.encryption_password`, S3 `secret_key`) are stored as `dpapi:<base64>` and never returned by the API (masked to `""`).
- Retention never scans directories or buckets; it deletes only rows recorded in `backup_files`.
- Timestamps stored in SQLite are local time strings `YYYY-MM-DD HH:MM:SS`.
- Port 8720, bind 127.0.0.1 only. Header `x-powerpcu-client: backup-ui` required on `/api/*`.
- POC source for ported files: `git -C C:\Users\usman\repo\powerpcu_poc show f45ee96^:user-api/src/backup/<file>`.
- Commit after every task with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` as the last line of the message.

---

## File Structure (this plan)

```
src-tauri/
  Cargo.toml
  migrations/0001_init.sql
  src/lib.rs            module tree
  src/main.rs           --service | --version dispatch
  src/paths.rs          data dir, db path, temp dir, mysqldump path, port
  src/db.rs             Pool, open(), migrate(), open_memory()
  src/state.rs          AppState { pool, mysqldump, temp_dir, started_at }
  src/secret.rs         DPAPI protect/unprotect
  src/service.rs        boot sequence + axum serve
  src/backup/mod.rs
  src/backup/mysqldump.rs   dump() + probe()          (ported)
  src/backup/archive.rs     compress_to_zip()         (ported)
  src/backup/storage/mod.rs StorageConfig, Storage, Provider, for_storage, rel_path (ported)
  src/backup/storage/local.rs                          (ported)
  src/backup/storage/s3.rs                             (ported verbatim)
  src/backup/storages.rs    storages table             (ported)
  src/backup/plans.rs       plans + plan_storages tables, NewPlan/Plan
  src/backup/runs.rs        runs table
  src/backup/files.rs       backup_files table + apply_prune + kept_as
  src/backup/prune.rs       pure keep-by-rule
  src/backup/running.rs     in-memory running set
  src/backup/run.rs         one backup run
  src/backup/schedule.rs    cron scheduler + catch-up
  src/api/mod.rs            router, AppState extractor, guard, ApiError, ok_false
  src/api/status.rs
  src/api/storages.rs
  src/api/plans.rs
  src/api/runs.rs
  src/api/static_files.rs
  tests/api.rs              end-to-end through the router with a fake mysqldump
web/index.html              placeholder (Plan 2 replaces)
lib/mysql-5.6.45-winx64/mysqldump.exe, lib/nssm-2.24/win64/nssm.exe  (kept from v1)
CLAUDE.md                   rewritten for v2
```

Interfaces are stated per task. Names below are the ones every later task uses.

---

### Task 1: Branch cleanup + crate skeleton

**Files:**
- Delete: `main.ts`, `src/`, `deno.json`, `deno.lock`, `preBuild.ts`, `postBuild.ts`, `preBuild/`, `setup.iss`, `power_pcu_backup.zip`, `yarn.lock`, `node_modules/`, `lib/7-Zip/`
- Create: `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`, `src-tauri/src/main.rs`, `.gitignore`

**Interfaces:**
- Produces: crate `powerpcu-backup` with lib name `powerpcu_backup`; binary prints version with `--version`.

- [ ] **Step 1: Remove v1 files (branch `v2` only)**

```powershell
git rm -r -q main.ts src deno.json deno.lock preBuild.ts postBuild.ts preBuild setup.iss power_pcu_backup.zip yarn.lock node_modules/.yarn-integrity lib/7-Zip
Remove-Item -Recurse -Force node_modules -ErrorAction SilentlyContinue
```

- [ ] **Step 2: Write `.gitignore`**

```
src-tauri/target/
*.db
*.db-journal
*.db-wal
*.db-shm
```

- [ ] **Step 3: Write `src-tauri/Cargo.toml`**

```toml
[package]
name = "powerpcu-backup"
version = "2.0.0"
edition = "2021"
description = "PowerPCU Backup v2 — MySQL (JHCIS) backup service + GUI"

[lib]
name = "powerpcu_backup"
path = "src/lib.rs"

[[bin]]
name = "powerpcu-backup"
path = "src/main.rs"

[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "process", "sync", "fs", "time", "net"] }
axum = "0.8"
rust-embed = "8"
sqlx = { version = "0.9", default-features = false, features = ["runtime-tokio", "sqlite", "migrate", "macros"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", default-features = false, features = ["clock", "std"] }
tokio-cron-scheduler = "0.15"
uuid = { version = "1", features = ["v4"] }
# Zstd + AES-256 zip, no 7-Zip binary. Same crate/feature set the POC shipped.
zip = { version = "2", default-features = false, features = ["zstd", "aes-crypto", "deflate"] }
# Official SDK (rust-s3's header-signed DELETE is rejected by Ceph RadosGW). rustls+ring only.
aws-sdk-s3 = { version = "1", default-features = false, features = ["behavior-version-latest", "rt-tokio"] }
aws-smithy-http-client = { version = "1", default-features = false, features = ["rustls-ring"] }
# DPAPI for secrets at rest.
windows-sys = { version = "0.61", features = ["Win32_Foundation", "Win32_Security_Cryptography"] }
base64 = "0.23"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["chrono"] }

[dev-dependencies]
tower = { version = "0.5", features = ["util"] }

[profile.release]
strip = true
lto = "thin"
codegen-units = 1
panic = "abort"
```

- [ ] **Step 4: Write `src-tauri/src/lib.rs`**

```rust
//! PowerPCU Backup v2 library: engine (`backup`), HTTP API (`api`), boot (`service`).
//! The binary in `main.rs` only dispatches `--service` / `--version` (GUI comes in Plan 2).

pub mod api;
pub mod backup;
pub mod db;
pub mod paths;
pub mod secret;
pub mod service;
pub mod state;
```

Also create empty module files so the crate compiles now:

```powershell
New-Item -ItemType Directory -Force src-tauri/src/api, src-tauri/src/backup/storage, src-tauri/migrations, web | Out-Null
"//! HTTP API (filled in Task 10)." | Set-Content -Encoding utf8 src-tauri/src/api/mod.rs
"//! Backup engine (filled in Tasks 4-9)." | Set-Content -Encoding utf8 src-tauri/src/backup/mod.rs
"//! Boot (filled in Task 11)." | Set-Content -Encoding utf8 src-tauri/src/service.rs
"//! Paths (filled in Task 2)." | Set-Content -Encoding utf8 src-tauri/src/paths.rs
"//! DB (filled in Task 2)." | Set-Content -Encoding utf8 src-tauri/src/db.rs
"//! AppState (filled in Task 2)." | Set-Content -Encoding utf8 src-tauri/src/state.rs
"//! DPAPI (filled in Task 3)." | Set-Content -Encoding utf8 src-tauri/src/secret.rs
```

- [ ] **Step 5: Write `src-tauri/src/main.rs`**

```rust
//! Binary entry. `--service` runs the headless backup service, `--version` prints
//! the version. Any other invocation is the GUI (Plan 2); until then it prints a hint.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--version") => println!("{}", env!("CARGO_PKG_VERSION")),
        Some("--service") => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            if let Err(e) = rt.block_on(powerpcu_backup::service::run()) {
                eprintln!("service error: {e}");
                std::process::exit(1);
            }
        }
        _ => println!("PowerPCU Backup {}: GUI mode is not built yet. Use --service or --version.", env!("CARGO_PKG_VERSION")),
    }
}
```

Temporarily make `service.rs` compile:

```rust
//! Boot (filled in Task 11).
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
```

- [ ] **Step 6: Build and smoke-run**

Run: `cd src-tauri; cargo run -q -- --version`
Expected: prints `2.0.0` (first build downloads and compiles deps; several minutes).

- [ ] **Step 7: Commit**

```powershell
git add -A
git commit -q -F - @'
chore(v2): remove v1 Deno sources, add Rust crate skeleton

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
'@
```

---

### Task 2: Paths, database, migration, AppState

**Files:**
- Create: `src-tauri/migrations/0001_init.sql`, `src-tauri/src/paths.rs`, `src-tauri/src/db.rs`, `src-tauri/src/state.rs`

**Interfaces:**
- Produces:
  - `paths::data_dir() -> PathBuf`, `paths::db_path()`, `paths::temp_dir()`, `paths::mysqldump_path()`, `paths::port() -> u16`
  - `db::Pool` (= `sqlx::SqlitePool`), `db::open(path: &Path) -> Result<Pool, sqlx::Error>`, `db::migrate(&Pool)`, `db::open_memory() -> Pool` (migrated, for tests)
  - `state::AppState { pool: Pool, mysqldump: PathBuf, temp_dir: PathBuf, started_at: String }` with `AppState::new(pool, mysqldump, temp_dir)` and `Clone`

- [ ] **Step 1: Write the migration `src-tauri/migrations/0001_init.sql`** (exactly spec §5)

```sql
CREATE TABLE storages (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT NOT NULL,
  provider    TEXT NOT NULL,
  config      TEXT NOT NULL,
  created_at  TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE plans (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  name                TEXT NOT NULL,
  database_name       TEXT NOT NULL,
  host                TEXT NOT NULL,
  port                INTEGER NOT NULL,
  username            TEXT NOT NULL,
  password            TEXT NOT NULL,
  prefix_name         TEXT NOT NULL,
  encryption_password TEXT NOT NULL,
  schedule_cron       TEXT,
  active              INTEGER NOT NULL DEFAULT 1,
  catch_up            INTEGER NOT NULL DEFAULT 1,
  keep_daily          INTEGER NOT NULL DEFAULT 14,
  keep_weekly         INTEGER NOT NULL DEFAULT 12,
  keep_monthly        INTEGER NOT NULL DEFAULT 24,
  keep_yearly         INTEGER NOT NULL DEFAULT 5,
  created_at          TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE plan_storages (
  plan_id     INTEGER NOT NULL,
  storage_id  INTEGER NOT NULL,
  PRIMARY KEY (plan_id, storage_id)
);
CREATE INDEX idx_plan_storages_storage ON plan_storages(storage_id);

CREATE TABLE runs (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  plan_id      INTEGER NOT NULL,
  trigger      TEXT NOT NULL,
  status       TEXT NOT NULL,
  started_at   TEXT NOT NULL,
  finished_at  TEXT,
  message      TEXT NOT NULL DEFAULT '',
  size_bytes   INTEGER
);
CREATE INDEX idx_runs_plan ON runs(plan_id, started_at);

CREATE TABLE backup_files (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id      INTEGER NOT NULL,
  plan_id     INTEGER NOT NULL,
  storage_id  INTEGER NOT NULL,
  provider    TEXT NOT NULL,
  location    TEXT NOT NULL,
  size_bytes  INTEGER NOT NULL,
  created_at  TEXT NOT NULL,
  deleted_at  TEXT
);
CREATE INDEX idx_backup_files_live ON backup_files(plan_id, storage_id, deleted_at);
```

- [ ] **Step 2: Write the failing test in `src-tauri/src/db.rs`**

```rust
//! The service's own SQLite database: pool type, open, migrations.

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

pub type Pool = SqlitePool;

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
```

- [ ] **Step 3: Run to verify it fails**

Run: `cd src-tauri; cargo test -q db::` — Expected: compile error, `open_memory` not found.

- [ ] **Step 4: Implement `db.rs`** (append above the tests)

```rust
/// Open (creating if missing) the SQLite file at `path`. `filename()` avoids URL
/// parsing of Windows drive letters.
pub async fn open(path: &Path) -> Result<Pool, sqlx::Error> {
    let options = SqliteConnectOptions::new().filename(path).create_if_missing(true);
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
```

- [ ] **Step 5: Write `paths.rs`**

```rust
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
```

- [ ] **Step 6: Write `state.rs`**

```rust
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
```

- [ ] **Step 7: Run tests**

Run: `cd src-tauri; cargo test -q db::` — Expected: `test db::tests::migrate_creates_tables ... ok`.

- [ ] **Step 8: Commit**

```powershell
git add -A
git commit -q -F - @'
feat(v2): sqlite schema, db pool, paths, AppState

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
'@
```

---

### Task 3: DPAPI secrets (`secret.rs`)

**Files:**
- Create/replace: `src-tauri/src/secret.rs`

**Interfaces:**
- Produces: `secret::protect(plain: &str) -> Result<String, String>` (returns `""` for `""`, else `dpapi:<base64>`), `secret::unprotect(stored: &str) -> Result<String, String>` (pass-through when not prefixed), `secret::is_protected(&str) -> bool`.

- [ ] **Step 1: Write the failing tests**

```rust
//! Secrets at rest: Windows DPAPI, machine scope (`CRYPTPROTECT_LOCAL_MACHINE`), so
//! a copied database file cannot be read elsewhere. Stored form: `dpapi:<base64>`.
//! Unprefixed values pass through `unprotect` unchanged (tests, hand-edited DBs).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let stored = protect("p@ss ไทย").unwrap();
        assert!(stored.starts_with("dpapi:"));
        assert!(is_protected(&stored));
        assert_eq!(unprotect(&stored).unwrap(), "p@ss ไทย");
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(protect("").unwrap(), "");
        assert_eq!(unprotect("").unwrap(), "");
    }

    #[test]
    fn plain_passes_through() {
        assert!(!is_protected("plain"));
        assert_eq!(unprotect("plain").unwrap(), "plain");
    }

    #[test]
    fn tampered_blob_errors() {
        let err = unprotect("dpapi:AAAA").unwrap_err();
        assert!(err.contains("ถอดรหัส"), "thai message, got {err}");
    }
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -q secret::` → compile error.

- [ ] **Step 3: Implement**

```rust
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN,
    CRYPT_INTEGER_BLOB,
};

const PREFIX: &str = "dpapi:";

pub fn is_protected(s: &str) -> bool {
    s.starts_with(PREFIX)
}

pub fn protect(plain: &str) -> Result<String, String> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    let bytes = plain.as_bytes();
    let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: input points at a live slice; out is written by the API and freed below.
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err(format!("เข้ารหัสค่าลับไม่สำเร็จ (DPAPI {})", unsafe { GetLastError() }));
    }
    let data = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe { LocalFree(out.pbData as *mut _) };
    Ok(format!("{PREFIX}{}", B64.encode(data)))
}

pub fn unprotect(stored: &str) -> Result<String, String> {
    let Some(b64) = stored.strip_prefix(PREFIX) else {
        return Ok(stored.to_string());
    };
    let blob = B64.decode(b64).map_err(|_| "ถอดรหัสค่าลับไม่สำเร็จ (รูปแบบไม่ถูกต้อง)".to_string())?;
    let input = CRYPT_INTEGER_BLOB { cbData: blob.len() as u32, pbData: blob.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err(format!("ถอดรหัสค่าลับไม่สำเร็จ (DPAPI {}) — ค่านี้ถูกเข้ารหัสบนเครื่องอื่นหรือถูกแก้ไข", unsafe { GetLastError() }));
    }
    let data = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe { LocalFree(out.pbData as *mut _) };
    String::from_utf8(data).map_err(|_| "ถอดรหัสค่าลับไม่สำเร็จ (ไม่ใช่ข้อความ)".to_string())
}
```

If `CryptUnprotectData`'s second parameter type is `*mut PWSTR` in this windows-sys version and the compiler complains about `null_mut()`, use `std::ptr::null_mut::<*mut u16>()`.

- [ ] **Step 4: Run tests** — `cargo test -q secret::` → 4 passed.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -q -F - @'
feat(v2): DPAPI machine-scope secret protect/unprotect

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
'@
```

---

### Task 4: mysqldump + archive (ported)

**Files:**
- Create: `src-tauri/src/backup/mod.rs`, `src-tauri/src/backup/mysqldump.rs`, `src-tauri/src/backup/archive.rs`

**Interfaces:**
- Produces:
  - `mysqldump::Source<'a> { mysqldump: &'a Path, database, host: &'a str, port: u16, username, password: &'a str }`
  - `mysqldump::DumpError { Launch(io::Error), Exit { code: ExitStatus, stderr: String } }`
  - `mysqldump::dump(&Source, dest: &Path) -> Result<(), DumpError>`
  - `mysqldump::probe(&Source, temp_dir: &Path) -> Result<(), DumpError>` (schema-only dump to a temp file, then removed)
  - `mysqldump::error_message(DumpError) -> String` (Thai)
  - `archive::compress_to_zip(sql_path: &Path, entry_name: &str, password: &str) -> Result<PathBuf, String>`
  - test helper (pub, `#[doc(hidden)]`): `mysqldump::fake_script(dir: &Path, sql_out: &Path, exit_code: i32, stderr: &str) -> PathBuf` writes a `.cmd` that writes `-- fake dump` to `sql_out`, prints `stderr`, exits with `exit_code`.

- [ ] **Step 1: `backup/mod.rs`**

```rust
//! Backup engine: framework-free. `run` orchestrates one backup; `schedule` fires runs on cron.

pub mod archive;
pub mod files;
pub mod mysqldump;
pub mod plans;
pub mod prune;
pub mod run;
pub mod running;
pub mod runs;
pub mod schedule;
pub mod storage;
pub mod storages;
```

Create placeholder files for modules not yet written so the crate compiles: `files.rs`, `plans.rs`, `prune.rs`, `run.rs`, `running.rs`, `runs.rs`, `schedule.rs`, `storages.rs`, `storage/mod.rs` each containing only a `//!` comment line.

- [ ] **Step 2: Failing tests for archive** — `src-tauri/src/backup/archive.rs`

```rust
//! Compress a dump into an AES-256 zip (Zstd method). Opens in 7-Zip 21+/WinRAR 6+,
//! not in Windows Explorer (which reads neither AES nor Zstd). Ported from the POC.

use std::fs::File;
use std::path::{Path, PathBuf};

use zip::{write::SimpleFileOptions, AesMode, CompressionMethod, ZipWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ppb-archive-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn zip_round_trips_with_password_and_entry_name() {
        let d = temp("ok");
        let sql = d.join("plan1_pre_db_2609070330.sql");
        std::fs::write(&sql, b"-- dump\nCREATE TABLE t (id INT);\n").unwrap();

        let zip_path = compress_to_zip(&sql, "pre_db_2609070330.sql", "secret123").unwrap();
        assert_eq!(zip_path, d.join("plan1_pre_db_2609070330.zip"));

        let mut archive = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        assert_eq!(archive.len(), 1);
        let mut entry = archive.by_name_decrypt("pre_db_2609070330.sql", b"secret123").unwrap();
        let mut body = String::new();
        entry.read_to_string(&mut body).unwrap();
        assert!(body.contains("CREATE TABLE t"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn wrong_password_cannot_read() {
        let d = temp("wrong");
        let sql = d.join("x.sql");
        std::fs::write(&sql, b"data").unwrap();
        let zip_path = compress_to_zip(&sql, "x.sql", "right").unwrap();
        let mut archive = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        assert!(archive.by_name_decrypt("x.sql", b"wrong").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
```

- [ ] **Step 3: Run** — `cargo test -q archive::` → compile error.

- [ ] **Step 4: Implement archive** (between the imports and the tests)

```rust
/// Zstd level 3: fast, good ratio on SQL text; compress dominates run time.
const ZSTD_LEVEL: i64 = 3;

/// Write `<sql_path with .zip>` holding one entry `entry_name` (AES-256, Zstd, ZIP64).
/// Streams the file; on failure removes the partial archive.
pub fn compress_to_zip(sql_path: &Path, entry_name: &str, password: &str) -> Result<PathBuf, String> {
    let zip_path = sql_path.with_extension("zip");
    let zip_file = File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(zip_file);
    let opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Zstd)
        .compression_level(Some(ZSTD_LEVEL))
        .large_file(true)
        .with_aes_encryption(AesMode::Aes256, password);

    let run = || -> Result<(), String> {
        zip.start_file(entry_name, opts).map_err(|e| e.to_string())?;
        let mut sql = File::open(sql_path).map_err(|e| e.to_string())?;
        std::io::copy(&mut sql, &mut zip).map_err(|e| e.to_string())?;
        zip.finish().map_err(|e| e.to_string())?;
        Ok(())
    };
    match run() {
        Ok(()) => Ok(zip_path),
        Err(e) => {
            let _ = std::fs::remove_file(&zip_path);
            Err(e)
        }
    }
}
```

- [ ] **Step 5: Run** — `cargo test -q archive::` → 2 passed.

- [ ] **Step 6: Failing tests for mysqldump** — `src-tauri/src/backup/mysqldump.rs`

```rust
//! Runs `mysqldump` (bundled 5.6.45). Password via env `MYSQL_PWD`, never argv.
//! Ported from the POC; `probe` added for the "test connection" button.

use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use tokio::process::Command;

pub struct Source<'a> {
    pub mysqldump: &'a Path,
    pub database: &'a str,
    pub host: &'a str,
    pub port: u16,
    pub username: &'a str,
    pub password: &'a str,
}

pub enum DumpError {
    Launch(std::io::Error),
    Exit { code: ExitStatus, stderr: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ppb-dump-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn src<'a>(fake: &'a Path) -> Source<'a> {
        Source { mysqldump: fake, database: "jhcisdb", host: "127.0.0.1", port: 3333, username: "root", password: "pw" }
    }

    #[tokio::test]
    async fn dump_ok_writes_file() {
        let d = temp("ok");
        let out = d.join("out.sql");
        let fake = fake_script(&d, &out, 0, "");
        dump(&src(&fake), &out).await.ok().expect("dump ok");
        assert!(std::fs::read_to_string(&out).unwrap().contains("fake dump"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn dump_failure_returns_stderr_and_removes_partial() {
        let d = temp("fail");
        let out = d.join("out.sql");
        let fake = fake_script(&d, &out, 2, "mysqldump: Got error: 1045: Access denied");
        let err = dump(&src(&fake), &out).await.err().expect("must fail");
        let msg = error_message(err);
        assert!(msg.contains("Access denied"), "got {msg}");
        assert!(!out.exists(), "partial file removed");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn launch_failure_is_thai() {
        let d = temp("launch");
        let missing = d.join("nope.exe");
        let err = dump(&src(&missing), &d.join("o.sql")).await.err().unwrap();
        assert!(error_message(err).contains("เรียกใช้ mysqldump ไม่สำเร็จ"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn probe_ok_leaves_no_file() {
        let d = temp("probe");
        let out = d.join("ignored.sql");
        let fake = fake_script(&d, &out, 0, "");
        probe(&src(&fake), &d).await.ok().expect("probe ok");
        let leftovers: Vec<_> = std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with("probe_")).collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
```

- [ ] **Step 7: Run** — `cargo test -q mysqldump::` → compile error.

- [ ] **Step 8: Implement mysqldump** (between `DumpError` and the tests)

```rust
const DUMP_FLAGS: [&str; 4] = ["--single-transaction", "--routines", "--events", "--triggers"];

fn command(src: &Source<'_>, extra: &[&str], result_file: &Path) -> Command {
    let mut cmd = Command::new(src.mysqldump);
    cmd.arg(src.database)
        .args(extra)
        .arg(format!("--result-file={}", result_file.display()))
        .arg(format!("--host={}", src.host))
        .arg(format!("--port={}", src.port))
        .arg(format!("--user={}", src.username))
        .env("MYSQL_PWD", src.password);
    cmd
}

async fn run(mut cmd: Command, result_file: &Path) -> Result<(), DumpError> {
    match cmd.output().await {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let _ = std::fs::remove_file(result_file);
            Err(DumpError::Exit { code: out.status, stderr: String::from_utf8_lossy(&out.stderr).into_owned() })
        }
        Err(e) => {
            let _ = std::fs::remove_file(result_file);
            Err(DumpError::Launch(e))
        }
    }
}

/// Full dump to `dest`. Partial `dest` is removed on failure.
pub async fn dump(src: &Source<'_>, dest: &Path) -> Result<(), DumpError> {
    run(command(src, &DUMP_FLAGS, dest), dest).await
}

/// Connection test: schema-only dump into `temp_dir/probe_<pid>_<nanos>.sql`, then removed.
/// Exercises the real binary, credentials and database name.
pub async fn probe(src: &Source<'_>, temp_dir: &Path) -> Result<(), DumpError> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let file = temp_dir.join(format!("probe_{}_{nanos}.sql", std::process::id()));
    let flags = ["--no-data", "--skip-triggers", "--skip-routines", "--skip-events"];
    let result = run(command(src, &flags, &file), &file).await;
    let _ = std::fs::remove_file(&file);
    result
}

/// Thai, user-facing message for a dump failure.
pub fn error_message(e: DumpError) -> String {
    match e {
        DumpError::Launch(io) => format!("เรียกใช้ mysqldump ไม่สำเร็จ: {io}"),
        DumpError::Exit { code, stderr } => {
            let detail = stderr.trim();
            if detail.is_empty() { format!("mysqldump ล้มเหลว (exit {code})") } else { detail.to_string() }
        }
    }
}

/// Test helper: a `.cmd` standing in for mysqldump. Writes `-- fake dump` to `sql_out`
/// (ignoring `--result-file`), prints `stderr` to stderr, exits `exit_code`.
#[doc(hidden)]
pub fn fake_script(dir: &Path, sql_out: &Path, exit_code: i32, stderr: &str) -> PathBuf {
    let script = dir.join(format!("fake-mysqldump-{exit_code}.cmd"));
    let mut body = String::from("@echo off\r\n");
    if exit_code == 0 {
        body.push_str(&format!("echo -- fake dump> \"{}\"\r\n", sql_out.display()));
    }
    if !stderr.is_empty() {
        body.push_str(&format!("echo {stderr} 1>&2\r\n"));
    }
    body.push_str(&format!("exit /b {exit_code}\r\n"));
    std::fs::write(&script, body).expect("write fake script");
    script
}
```

- [ ] **Step 9: Run** — `cargo test -q mysqldump::` → 4 passed. (Rust runs `.cmd` files through `cmd.exe` automatically.)

- [ ] **Step 10: Commit**

```powershell
git add -A
git commit -q -F - @'
feat(v2): port mysqldump runner (+probe) and AES/Zstd archive from POC

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
'@
```

---

### Task 5: Storage providers (ported)

**Files:**
- Create: `src-tauri/src/backup/storage/mod.rs`, `storage/local.rs`, `storage/s3.rs`

**Interfaces:**
- Produces:
  - `storage::StorageConfig { Local(LocalConfig{path}), S3(S3Config{endpoint,region,bucket,access_key,secret_key,prefix,path_style}) }` with `parse(provider, json) -> Result<Self,String>`, `provider_str()`, `to_json()`, `to_public_value()`, `protect_secret(&mut self) -> Result<(),String>`, `secret_is_empty() -> bool`, `keep_secret_from(&mut self, old: &StorageConfig)`
  - `storage::Storage { id: i64, name: String, config: StorageConfig }` with `to_public_value()`
  - `storage::Provider` with `async store(&self, zip: &Path, rel: &str) -> Result<String,String>`, `async delete(&self, location: &str) -> Result<(),String>`, `async test(&self) -> Result<(),String>`
  - `storage::for_storage(&Storage) -> Result<Provider,String>` (unprotects the S3 secret)
  - `storage::rel_path(prefix_name, database_name, file_name) -> String` = `"<prefix>/<db>/<file>"`

- [ ] **Step 1: Extract the POC files**

```powershell
$P = "C:\Users\usman\repo\powerpcu_poc"
git -C $P show "f45ee96^:user-api/src/backup/storage/mod.rs"   | Set-Content -Encoding utf8 src-tauri/src/backup/storage/mod.rs
git -C $P show "f45ee96^:user-api/src/backup/storage/local.rs" | Set-Content -Encoding utf8 src-tauri/src/backup/storage/local.rs
git -C $P show "f45ee96^:user-api/src/backup/storage/s3.rs"    | Set-Content -Encoding utf8 src-tauri/src/backup/storage/s3.rs
```

- [ ] **Step 2: Edit `storage/mod.rs`**

1. Change every `pub(crate)` to `pub`.
2. Delete `delete_local` (retention now always has the storage row).
3. Add after `impl StorageConfig { … to_public_value … }` these three methods inside the same `impl`:

```rust
    /// Encrypt the S3 secret with DPAPI if it is set and not already protected.
    pub fn protect_secret(&mut self) -> Result<(), String> {
        if let StorageConfig::S3(c) = self {
            if !c.secret_key.is_empty() && !crate::secret::is_protected(&c.secret_key) {
                c.secret_key = crate::secret::protect(&c.secret_key)?;
            }
        }
        Ok(())
    }

    pub fn secret_is_empty(&self) -> bool {
        match self {
            StorageConfig::S3(c) => c.secret_key.is_empty(),
            StorageConfig::Local(_) => false,
        }
    }

    /// Update-without-secret: carry the stored (already protected) secret over.
    pub fn keep_secret_from(&mut self, old: &StorageConfig) {
        if let (StorageConfig::S3(new), StorageConfig::S3(old)) = (self, old) {
            if new.secret_key.is_empty() {
                new.secret_key = old.secret_key.clone();
            }
        }
    }
```

4. Replace `for_storage` with:

```rust
/// Build the runtime provider. The S3 secret is decrypted here and only here.
pub fn for_storage(s: &Storage) -> Result<Provider, String> {
    match &s.config {
        StorageConfig::Local(c) => Ok(Provider::Local(LocalProvider { dir: c.path.clone() })),
        StorageConfig::S3(c) => {
            let mut cfg = c.clone();
            cfg.secret_key = crate::secret::unprotect(&c.secret_key)?;
            Ok(Provider::S3(S3Provider { cfg }))
        }
    }
}

/// Destination-relative path of a backup file: `<prefix_name>/<database_name>/<file>`.
pub fn rel_path(prefix_name: &str, database_name: &str, file_name: &str) -> String {
    format!("{prefix_name}/{database_name}/{file_name}")
}
```

5. Rename the parameter `file_name` to `rel` in `Provider::store` (it is now a relative path with `/`).
6. Append to the tests module:

```rust
    #[test]
    fn rel_path_is_prefix_db_file() {
        assert_eq!(rel_path("10999", "jhcisdb", "a.zip"), "10999/jhcisdb/a.zip");
    }

    #[test]
    fn protect_and_keep_secret() {
        let mut cfg = StorageConfig::parse("s3", r#"{"endpoint":"e","bucket":"b","access_key":"AK","secret_key":"SK"}"#).unwrap();
        cfg.protect_secret().unwrap();
        assert!(cfg.to_json().contains("dpapi:"));
        assert!(!cfg.to_json().contains("\"SK\""));
        let mut update = StorageConfig::parse("s3", r#"{"endpoint":"e","bucket":"b","access_key":"AK","secret_key":""}"#).unwrap();
        assert!(update.secret_is_empty());
        update.keep_secret_from(&cfg);
        assert_eq!(update.to_json(), cfg.to_json());
        // for_storage decrypts
        let s = Storage { id: 1, name: "s".into(), config: cfg };
        match for_storage(&s).unwrap() { Provider::S3(p) => assert_eq!(p.cfg.secret_key, "SK"), _ => panic!() }
    }
```

- [ ] **Step 3: Edit `storage/local.rs`**

Replace `store` so `rel` (with `/`) becomes nested folders under `dir`:

```rust
    /// Copy `zip` to `dir/<rel>` (creating the sub-folders); returns the absolute path.
    pub async fn store(&self, zip: &Path, rel: &str) -> Result<String, String> {
        let mut dest = std::path::PathBuf::from(&self.dir);
        for part in rel.split('/') {
            dest.push(part);
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("สร้างโฟลเดอร์ปลายทางไม่สำเร็จ: {e}"))?;
        }
        tokio::fs::copy(zip, &dest)
            .await
            .map_err(|e| format!("คัดลอกไฟล์ไปยังปลายทางไม่สำเร็จ: {e}"))?;
        Ok(dest.to_string_lossy().into_owned())
    }
```

Replace `test` with a real write probe:

```rust
    /// The folder must be creatable and writable: write then delete a probe file.
    pub async fn test(&self) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|e| format!("โฟลเดอร์เขียนไม่ได้: {e}"))?;
        let probe = Path::new(&self.dir).join(".powerpcu_probe");
        tokio::fs::write(&probe, b"ok")
            .await
            .map_err(|e| format!("โฟลเดอร์เขียนไม่ได้: {e}"))?;
        let _ = tokio::fs::remove_file(&probe).await;
        Ok(())
    }
```

Change `pub(crate)` to `pub`. In the existing test `store_then_delete`, call `p.store(&src, "10999/jhcisdb/backup.zip")` and assert the location ends with `10999\jhcisdb\backup.zip`.

- [ ] **Step 4: Edit `storage/s3.rs`**

Only: `pub(crate)` → `pub`; rename the `store` parameter `file_name` → `rel` (the key is `prefix + rel`, unchanged logic); delete the `live_full_loop` test (it read `powerpcu.db` from the POC layout). Keep the `key_round_trips_through_location` test. Do not touch the checksum/path-style/TLS settings.

- [ ] **Step 5: Run** — `cargo test -q storage::` → all pass (local store/delete, config parse/mask, protect/keep, rel_path, s3 key round trip).

- [ ] **Step 6: Commit**

```powershell
git add -A
git commit -q -F - @'
feat(v2): port local + S3 storage providers, DPAPI-protected S3 secret, nested rel paths

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
'@
```

---

### Task 6: Tables — storages, plans, runs, backup_files

**Files:**
- Create/replace: `src-tauri/src/backup/storages.rs`, `plans.rs`, `runs.rs`, `files.rs`

**Interfaces:**
- Produces (all `async`, all take `&Pool`, all return `Result<_, sqlx::Error>` unless noted):
  - `storages::{insert(name, provider, config_json) -> i64, list_all() -> Vec<Storage>, find(id) -> Option<Storage>, update(id, name, provider, config_json), delete(id), is_in_use(id) -> bool, list_for_plan(plan_id) -> Vec<Storage>, existing_ids(&[i64]) -> Vec<i64>}`
  - `plans::NewPlan` (Deserialize; fields in spec §10 plus `copy_password_from_plan_id: Option<i64>`), `plans::Plan` (Clone, Serialize; `keep() -> prune::Keep`, `to_public_value(storage_ids: &[i64]) -> Value` masks both passwords to `""`), `plans::Conflict { plan_name, storage_name }`
  - `plans::{insert(&NewPlan) -> i64, update(id, &NewPlan), list_all() -> Vec<Plan>, find(id) -> Option<Plan>, delete(id), replace_links(plan_id, &[i64]), storage_ids_for(plan_id) -> Vec<i64>, find_conflict(exclude: Option<i64>, prefix, db, &[i64]) -> Option<Conflict>}`
  - `runs::Run { id, plan_id, trigger, status, started_at, finished_at: Option<String>, message, size_bytes: Option<i64> }` (Serialize, Clone)
  - `runs::{start(plan_id, trigger) -> i64, set_message(id, msg), finish(id, status, message, size: Option<i64>), find(id) -> Option<Run>, list_for_plan(plan_id, limit: i64) -> Vec<Run>, last_for_plan(plan_id) -> Option<Run>, last_success_at(plan_id) -> Option<String>, prune_old(plan_id, days: i64) -> u64, fail_stale_running() -> u64}`
  - `files::BackupFile { id, run_id, plan_id, storage_id, storage_name: Option<String>, provider, location, size_bytes, created_at, deleted_at: Option<String> }` (Serialize)
  - `files::LiveFile { id, created_at: String, location: String }`
  - `files::{record(run_id, plan_id, storage_id, provider, location, size: i64, created_at) -> i64, live_for(plan_id, storage_id) -> Vec<LiveFile>, mark_deleted(id), list_for_run(run_id) -> Vec<BackupFile>}` (`apply_prune` and `kept_as` are added in Task 8)
  - `runs::now_string() -> String` (local `YYYY-MM-DD HH:MM:SS`)

- [ ] **Step 1: `storages.rs`** — extract `git show f45ee96^:user-api/src/backup/storages/store.rs`, then: replace the two `use` lines with `use crate::backup::storage::{Storage, StorageConfig}; use crate::db::Pool;`, replace every `PowerpcuPool` with `Pool`, every `pub(crate)` with `pub`, `eprintln!` with `tracing::warn!`, and in tests replace the `pool()` helper body with `crate::db::open_memory().await`. In `is_in_use_tracks_plan_links_and_files`, the `backup_files` insert becomes `INSERT INTO backup_files (run_id, plan_id, storage_id, provider, location, size_bytes, created_at) VALUES (1, 1, ?, 'local', '/tmp/x.zip', 1, '2026-01-01 00:00:00')`. Append:

```rust
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
```

- [ ] **Step 2: `plans.rs`** — write in full:

```rust
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

#[derive(Debug, Clone, Serialize)]
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

type PlanRow = (i64, String, String, String, i64, String, String, String, String, Option<String>, bool, bool, i64, i64, i64, i64, String);

impl From<PlanRow> for Plan {
    fn from(r: PlanRow) -> Self {
        Plan { id: r.0, name: r.1, database_name: r.2, host: r.3, port: r.4, username: r.5, password: r.6, prefix_name: r.7, encryption_password: r.8, schedule_cron: r.9, active: r.10, catch_up: r.11, keep_daily: r.12, keep_weekly: r.13, keep_monthly: r.14, keep_yearly: r.15, created_at: r.16 }
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
    let rows = sqlx::query_as::<_, PlanRow>(&sql).fetch_all(pool).await?;
    Ok(rows.into_iter().map(Plan::from).collect())
}

pub async fn find(pool: &Pool, id: i64) -> Result<Option<Plan>, sqlx::Error> {
    let sql = format!("SELECT {COLS} FROM plans WHERE id = ?");
    let row = sqlx::query_as::<_, PlanRow>(&sql).bind(id).fetch_optional(pool).await?;
    Ok(row.map(Plan::from))
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
```

- [ ] **Step 3: `runs.rs`** — write in full:

```rust
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
    sqlx::query_as::<_, Run>(&format!("SELECT {COLS} FROM runs WHERE id = ?")).bind(id).fetch_optional(pool).await
}

pub async fn list_for_plan(pool: &Pool, plan_id: i64, limit: i64) -> Result<Vec<Run>, sqlx::Error> {
    sqlx::query_as::<_, Run>(&format!("SELECT {COLS} FROM runs WHERE plan_id = ? ORDER BY id DESC LIMIT ?"))
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
```

- [ ] **Step 4: `files.rs`** — write the table part now (prune application is Task 8):

```rust
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
```

- [ ] **Step 5: Run** — `cargo test -q backup::` → storages, plans, runs, files tests pass (prune/run/schedule modules are still empty placeholders; `plans.rs` imports `prune::Keep`, so add to `prune.rs` now the single line `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct Keep { pub daily: u32, pub weekly: u32, pub monthly: u32, pub yearly: u32 }` — Task 7 replaces the file).

- [ ] **Step 6: Commit**

```powershell
git add -A
git commit -q -F - @'
feat(v2): storages/plans/runs/backup_files stores with conflict check and run history

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
'@
```

---

### Task 7: Keep-by-rule retention (`prune.rs`)

**Files:**
- Replace: `src-tauri/src/backup/prune.rs`

**Interfaces:**
- Produces: `prune::Keep`, `prune::Candidate { id: i64, created_at: NaiveDateTime }`, `prune::Reason { Latest, Daily, Weekly, Monthly, Yearly }` (Serialize lowercase), `prune::Decision { keep: Vec<(i64, Vec<Reason>)>, delete: Vec<i64> }`, `prune::decide(&[Candidate], &Keep, now: NaiveDateTime) -> Decision`, `prune::fiscal_year(NaiveDate) -> i32`

- [ ] **Step 1: Write the file with tests first**

```rust
//! restic-style keep-by-rule (spec §7). Pure: no IO, no clock. A file is kept if
//! ANY rule claims it; everything else is deleted. Rules count only periods that
//! actually contain a file, so gaps (PC off at the weekend) never lose a tier.

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keep {
    pub daily: u32,
    pub weekly: u32,
    pub monthly: u32,
    pub yearly: u32,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub id: i64,
    pub created_at: NaiveDateTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    Latest,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Default, PartialEq)]
pub struct Decision {
    pub keep: Vec<(i64, Vec<Reason>)>,
    pub delete: Vec<i64>,
}

/// Thai fiscal year: Oct–Sep. 2025-10-01 → 2026, 2025-09-30 → 2025.
pub fn fiscal_year(d: NaiveDate) -> i32 {
    if d.month() >= 10 { d.year() + 1 } else { d.year() }
}

fn bucket(reason: Reason, d: NaiveDate) -> (i32, u32) {
    match reason {
        Reason::Daily => (d.year(), d.ordinal()),
        Reason::Weekly => (d.iso_week().year(), d.iso_week().week()),
        Reason::Monthly => (d.year(), d.month()),
        Reason::Yearly => (fiscal_year(d), 0),
        Reason::Latest => (0, 0),
    }
}

pub fn decide(files: &[Candidate], keep: &Keep, now: NaiveDateTime) -> Decision {
    let mut sorted: Vec<&Candidate> = files.iter().collect();
    sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    let mut reasons: Vec<Vec<Reason>> = vec![Vec::new(); sorted.len()];

    let all_zero = keep.daily == 0 && keep.weekly == 0 && keep.monthly == 0 && keep.yearly == 0;
    let mut newest_seen = false;
    for (i, c) in sorted.iter().enumerate() {
        // Future-dated (clock went backwards) → keep; newest real file → keep; all rules 0 → keep all.
        if c.created_at > now || !newest_seen || all_zero {
            reasons[i].push(Reason::Latest);
        }
        if c.created_at <= now {
            newest_seen = true;
        }
    }

    for (reason, n) in [(Reason::Daily, keep.daily), (Reason::Weekly, keep.weekly), (Reason::Monthly, keep.monthly), (Reason::Yearly, keep.yearly)] {
        if n == 0 {
            continue;
        }
        let mut seen: Vec<(i32, u32)> = Vec::new();
        for (i, c) in sorted.iter().enumerate() {
            if c.created_at > now {
                continue;
            }
            let b = bucket(reason, c.created_at.date());
            if seen.contains(&b) {
                continue;
            }
            seen.push(b);
            reasons[i].push(reason);
            if seen.len() as u32 >= n {
                break;
            }
        }
    }

    let mut out = Decision::default();
    for (c, rs) in sorted.iter().zip(reasons) {
        if rs.is_empty() { out.delete.push(c.id) } else { out.keep.push((c.id, rs)) }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Weekday};

    fn at(y: i32, m: u32, d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, 30, 0).unwrap()
    }
    fn daily(from: NaiveDateTime, days: i64, weekdays_only: bool) -> Vec<Candidate> {
        (0..days)
            .map(|i| from + Duration::days(i))
            .filter(|t| !weekdays_only || !matches!(t.weekday(), Weekday::Sat | Weekday::Sun))
            .enumerate()
            .map(|(i, t)| Candidate { id: i as i64 + 1, created_at: t })
            .collect()
    }
    fn count(d: &Decision, r: Reason) -> usize {
        d.keep.iter().filter(|(_, rs)| rs.contains(&r)).count()
    }
    fn keep(daily: u32, weekly: u32, monthly: u32, yearly: u32) -> Keep {
        Keep { daily, weekly, monthly, yearly }
    }

    #[test]
    fn three_years_of_daily_backups() {
        let files = daily(at(2023, 10, 1, 3), 1073, false); // to 2026-09-07
        let now = at(2026, 9, 7, 12);
        let d = decide(&files, &keep(14, 12, 24, 5), now);
        assert_eq!(count(&d, Reason::Daily), 14);
        assert_eq!(count(&d, Reason::Weekly), 12);
        assert_eq!(count(&d, Reason::Monthly), 24);
        assert_eq!(count(&d, Reason::Yearly), 3, "FY2024, FY2025, FY2026 present");
        assert_eq!(d.keep.len() + d.delete.len(), files.len());
        assert!(d.keep.len() < 60 && d.keep.len() > 40);
        // newest file has Latest + Daily
        let newest = d.keep.iter().find(|(id, _)| *id == files.last().unwrap().id).unwrap();
        assert!(newest.1.contains(&Reason::Latest) && newest.1.contains(&Reason::Daily));
    }

    #[test]
    fn weekend_gap_keeps_friday_as_weekly() {
        let files = daily(at(2026, 8, 3, 3), 35, true); // Mon 3 Aug .. Sun 6 Sep, weekdays only
        let d = decide(&files, &keep(1, 2, 0, 0), at(2026, 9, 7, 12));
        let weekly: Vec<Weekday> = d.keep.iter().filter(|(_, rs)| rs.contains(&Reason::Weekly))
            .map(|(id, _)| files.iter().find(|f| f.id == *id).unwrap().created_at.weekday()).collect();
        assert_eq!(weekly, vec![Weekday::Fri, Weekday::Fri]);
    }

    #[test]
    fn newest_always_kept_and_all_zero_keeps_everything() {
        let files = daily(at(2026, 9, 1, 3), 5, false);
        let d = decide(&files, &keep(0, 0, 0, 1), at(2026, 9, 7, 12));
        assert!(d.keep.iter().any(|(id, rs)| *id == 5 && rs.contains(&Reason::Latest)));
        let all = decide(&files, &keep(0, 0, 0, 0), at(2026, 9, 7, 12));
        assert!(all.delete.is_empty());
        assert_eq!(all.keep.len(), 5);
    }

    #[test]
    fn fiscal_year_boundary_is_october() {
        assert_eq!(fiscal_year(NaiveDate::from_ymd_opt(2025, 9, 30).unwrap()), 2025);
        assert_eq!(fiscal_year(NaiveDate::from_ymd_opt(2025, 10, 1).unwrap()), 2026);
        let files = vec![
            Candidate { id: 1, created_at: at(2025, 9, 30, 3) },
            Candidate { id: 2, created_at: at(2025, 10, 1, 3) },
        ];
        let d = decide(&files, &keep(0, 0, 0, 2), at(2026, 9, 7, 12));
        assert_eq!(count(&d, Reason::Yearly), 2);
        assert!(d.delete.is_empty());
    }

    #[test]
    fn several_runs_same_day_keep_newest_only() {
        let files = vec![
            Candidate { id: 1, created_at: at(2026, 9, 7, 3) },
            Candidate { id: 2, created_at: at(2026, 9, 7, 9) },
            Candidate { id: 3, created_at: at(2026, 9, 7, 15) },
        ];
        let d = decide(&files, &keep(1, 0, 0, 0), at(2026, 9, 7, 20));
        assert_eq!(d.keep.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![3]);
        assert_eq!(d.delete, vec![2, 1]);
    }

    #[test]
    fn future_dated_file_is_kept_not_counted() {
        let files = vec![
            Candidate { id: 1, created_at: at(2026, 9, 6, 3) },
            Candidate { id: 2, created_at: at(2027, 1, 1, 3) },
        ];
        let d = decide(&files, &keep(1, 0, 0, 0), at(2026, 9, 7, 12));
        assert!(d.delete.is_empty());
        assert!(d.keep.iter().any(|(id, rs)| *id == 1 && rs.contains(&Reason::Daily)));
    }
}
```

- [ ] **Step 2: Run** — `cargo test -q prune::` → 6 passed. If `three_years_of_daily_backups` reports a Yearly count of 4, the generated range crossed into FY2027 (it must end 2026-09-07); check the `days` argument.

- [ ] **Step 3: Commit**

```powershell
git add -A
git commit -q -F - @'
feat(v2): restic-style keep-by-rule prune (daily/weekly/monthly/fiscal-year)

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
'@
```

---

### Task 8: One backup run (`running.rs`, `files::apply_prune`, `run.rs`)

**Files:**
- Replace: `src-tauri/src/backup/running.rs`, `src-tauri/src/backup/run.rs`
- Modify: `src-tauri/src/backup/files.rs` (append `apply_prune`, `kept_as`)

**Interfaces:**
- Consumes: Tasks 4–7.
- Produces:
  - `running::{try_start(plan_id) -> bool, end(plan_id), is_running(plan_id) -> bool}`
  - `files::apply_prune(pool, plan: &Plan, storage: &Storage, now: NaiveDateTime) -> Result<usize, String>` (deleted count)
  - `files::kept_as(pool, plan: &Plan, storage_id: i64, now) -> Result<HashMap<i64, Vec<Reason>>, String>` (dry run; storage row loaded inside)
  - `files::supersede(pool, plan_id, storage_id, location) -> Result<u64, sqlx::Error>` (retire live rows whose file was just overwritten)
  - `run::RunOutcome { run_id: i64, status: String, message: String }`
  - `run::run_backup(state: &AppState, plan: &Plan, trigger: &str) -> RunOutcome` (never panics, never returns Err; status ∈ `ok|partial|failed|skipped`)

- [ ] **Step 1: `running.rs`**

```rust
//! Which plans are backing up right now (process-global, ephemeral).

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

static RUNNING: OnceLock<Mutex<HashSet<i64>>> = OnceLock::new();

fn set() -> &'static Mutex<HashSet<i64>> {
    RUNNING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Claim the plan. `false` = already running (caller must skip).
pub fn try_start(plan_id: i64) -> bool {
    set().lock().unwrap().insert(plan_id)
}

pub fn end(plan_id: i64) {
    set().lock().unwrap().remove(&plan_id);
}

pub fn is_running(plan_id: i64) -> bool {
    set().lock().unwrap().contains(&plan_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_is_exclusive_until_end() {
        assert!(try_start(9001));
        assert!(!try_start(9001));
        assert!(is_running(9001));
        end(9001);
        assert!(!is_running(9001));
        assert!(try_start(9001));
        end(9001);
    }
}
```

- [ ] **Step 2: Append to `files.rs`** (above the tests module; add `use std::collections::HashMap; use chrono::NaiveDateTime; use crate::backup::plans::Plan; use crate::backup::prune::{self, Candidate, Reason}; use crate::backup::storage::{self, Storage};` at the top)

```rust
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
```

Append to the `files.rs` tests:

```rust
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
```

- [ ] **Step 3: `run.rs`** — write in full:

```rust
//! One backup run (spec §6): claim → runs row → dump (one at a time) → zip → ship to
//! every destination → record → prune per destination → close the runs row.

use std::path::Path;

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
    use super::*;
    use crate::backup::plans::{self, NewPlan};

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
        // --result-file=<path> …`, so %~6 is the result-file flag; strip the flag name and
        // write a stub dump there (the stamped path is only known at run time).
        let script = work.join("mysqldump.cmd");
        let body = if exit_code == 0 {
            "@echo off\r\nset \"rf=%~6\"\r\nset \"rf=%rf:--result-file=%\"\r\necho -- fake dump> \"%rf%\"\r\nexit /b 0\r\n".to_string()
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
        assert_eq!(files::live_for(&state.pool, plan.id, files[0].storage_id).await.unwrap().len(), 1);
        assert!(loc.exists(), "prune must not delete the freshly overwritten file");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[tokio::test]
    async fn dump_failure_is_failed_run_with_thai_message() {
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
        let (state, plan, work, _) = setup("nodest", 0).await;
        plans::replace_links(&state.pool, plan.id, &[]).await.unwrap();
        let out = run_backup(&state, &plan, "manual").await;
        assert_eq!(out.status, "failed");
        assert!(out.message.contains("ยังไม่ได้เลือกปลายทาง"));
        let _ = std::fs::remove_dir_all(&work);
    }

    #[tokio::test]
    async fn two_plans_dump_one_at_a_time() {
        let (state, plan, work, _) = setup("sem", 0).await;
        // A slow fake: sleep 2s before writing, so overlapping runs would overlap here.
        let slow = work.join("slow.cmd");
        std::fs::write(&slow, "@echo off\r\nset \"rf=%~6\"\r\nset \"rf=%rf:--result-file=%\"\r\nping -n 3 127.0.0.1 >nul\r\necho -- fake dump> \"%rf%\"\r\nexit /b 0\r\n").unwrap();
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
```

The fake `.cmd` in `setup` reads the 6th argument (`%~6`) because the arguments are: `database, --single-transaction, --routines, --events, --triggers, --result-file=…`. The `%rf:--result-file=%` substring-removal strips the flag name.

- [ ] **Step 4: Run** — `cargo test -q backup::run` and `cargo test -q backup::files` → all pass. If `two_plans_dump_one_at_a_time` fails on elapsed time, confirm `ping -n 3` sleeps ~2 s on this machine (it waits 1 s between 3 pings).

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -q -F - @'
feat(v2): backup run — serialized dump, zip, ship to all destinations, record, prune

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
'@
```

---

### Task 9: Scheduler + catch-up (`schedule.rs`)

**Files:**
- Replace: `src-tauri/src/backup/schedule.rs`

**Interfaces:**
- Consumes: `run::run_backup`, `plans::list_all`, `runs::last_success_at`
- Produces: `schedule::{start(state: AppState) -> Result<(), JobSchedulerError>, register_plan(&AppState, &Plan), unregister_plan(id), next_run_at(plan_id) -> Option<DateTime<Local>>, catch_up(state: AppState), needs_catch_up(last_success: Option<NaiveDateTime>, now: NaiveDateTime) -> bool, to_scheduler_expr(&str) -> String}`

- [ ] **Step 1: Write the file**

```rust
//! Cron scheduler (local time) + boot catch-up. Ported from the POC; timezone
//! changed from UTC to `chrono::Local`, overlap handled by `running::try_start`.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDateTime};
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};
use uuid::Uuid;

use crate::backup::{plans::{self, Plan}, run, runs};
use crate::state::AppState;

struct Handle {
    scheduler: JobScheduler,
    jobs: Mutex<HashMap<i64, Uuid>>,
}
static HANDLE: OnceLock<Handle> = OnceLock::new();

pub fn to_scheduler_expr(five_field: &str) -> String {
    format!("0 {}", five_field.trim())
}

fn is_scheduled(plan: &Plan) -> bool {
    plan.active && plan.schedule_cron.as_deref().map(|c| !c.trim().is_empty()).unwrap_or(false)
}

/// Build the scheduler, register every active scheduled plan, start ticking.
pub async fn start(state: AppState) -> Result<(), JobSchedulerError> {
    let scheduler = JobScheduler::new().await?;
    if HANDLE.set(Handle { scheduler: scheduler.clone(), jobs: Mutex::new(HashMap::new()) }).is_err() {
        tracing::warn!("scheduler: already started");
        return Ok(());
    }
    match plans::list_all(&state.pool).await {
        Ok(all) => for p in all.iter().filter(|p| is_scheduled(p)) { register_plan(&state, p).await; },
        Err(e) => tracing::error!("scheduler: cannot load plans: {e}"),
    }
    scheduler.start().await
}

pub async fn register_plan(state: &AppState, plan: &Plan) {
    let Some(handle) = HANDLE.get() else { return };
    if !is_scheduled(plan) {
        return;
    }
    let expr = to_scheduler_expr(plan.schedule_cron.as_deref().unwrap_or(""));
    let (st, pl) = (state.clone(), plan.clone());
    let job = Job::new_async_tz(expr.as_str(), Local, move |_id, _l| {
        let (st, pl) = (st.clone(), pl.clone());
        Box::pin(async move {
            let out = run::run_backup(&st, &pl, "schedule").await;
            if out.status == "skipped" {
                tracing::warn!(plan = pl.id, "scheduler: previous run still going, skipped tick");
            }
        })
    });
    match job {
        Ok(job) => match handle.scheduler.add(job).await {
            Ok(id) => {
                handle.jobs.lock().await.insert(plan.id, id);
                tracing::info!(plan = plan.id, cron = expr, "scheduler: registered (local time)");
            }
            Err(e) => tracing::error!(plan = plan.id, "scheduler: add failed: {e}"),
        },
        Err(e) => tracing::warn!(plan = plan.id, cron = ?plan.schedule_cron, "scheduler: invalid cron, skipped: {e}"),
    }
}

pub async fn unregister_plan(id: i64) {
    let Some(handle) = HANDLE.get() else { return };
    if let Some(job_id) = handle.jobs.lock().await.remove(&id) {
        if let Err(e) = handle.scheduler.remove(&job_id).await {
            tracing::error!(plan = id, "scheduler: remove failed: {e}");
        }
    }
}

/// Next tick in local time, or `None` when not scheduled / scheduler not running.
pub async fn next_run_at(plan_id: i64) -> Option<DateTime<Local>> {
    let handle = HANDLE.get()?;
    let job_id = *handle.jobs.lock().await.get(&plan_id)?;
    let mut scheduler = handle.scheduler.clone();
    scheduler.next_tick_for_job(job_id).await.ok().flatten().map(|t| t.with_timezone(&Local))
}

/// No ok/partial run in the last 24 h → run soon after boot.
pub fn needs_catch_up(last_success: Option<NaiveDateTime>, now: NaiveDateTime) -> bool {
    match last_success {
        None => true,
        Some(t) => now - t > chrono::Duration::hours(24),
    }
}

/// Spec §8: for each active, scheduled, catch_up plan that missed its window, start a
/// run 120 s + 30 s·i after boot (staggered so they queue on the dump semaphore).
pub async fn catch_up(state: AppState) {
    let Ok(all) = plans::list_all(&state.pool).await else { return };
    let now = Local::now().naive_local();
    let mut i = 0u64;
    for plan in all.into_iter().filter(|p| is_scheduled(p) && p.catch_up) {
        let last = runs::last_success_at(&state.pool, plan.id).await.ok().flatten()
            .and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok());
        if !needs_catch_up(last, now) {
            continue;
        }
        let delay = Duration::from_secs(120 + 30 * i);
        i += 1;
        tracing::info!(plan = plan.id, ?delay, "catch-up scheduled");
        let st = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            run::run_backup(&st, &plan, "catch_up").await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn t(d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, d).unwrap().and_hms_opt(h, 0, 0).unwrap()
    }

    #[test]
    fn expr_prepends_seconds() {
        assert_eq!(to_scheduler_expr("30 3 * * *"), "0 30 3 * * *");
        assert_eq!(to_scheduler_expr("  0 20 * * 0 "), "0 0 20 * * 0");
    }

    #[test]
    fn catch_up_rule_is_24h() {
        assert!(needs_catch_up(None, t(7, 12)));
        assert!(needs_catch_up(Some(t(5, 3)), t(7, 12)));
        assert!(!needs_catch_up(Some(t(7, 3)), t(7, 12)));
    }

    #[tokio::test]
    async fn invalid_cron_does_not_panic_and_no_next_run() {
        // HANDLE is unset in unit tests → register is a no-op and next_run_at is None.
        let pool = crate::db::open_memory().await;
        let state = AppState::new(pool, "x".into(), std::env::temp_dir());
        let plan = Plan { id: 1, name: "p".into(), database_name: "d".into(), host: "h".into(), port: 1, username: "u".into(), password: "p".into(), prefix_name: "x".into(), encryption_password: "e".into(), schedule_cron: Some("not a cron".into()), active: true, catch_up: true, keep_daily: 1, keep_weekly: 0, keep_monthly: 0, keep_yearly: 0, created_at: "".into() };
        register_plan(&state, &plan).await;
        assert!(next_run_at(1).await.is_none());
    }
}
```

If `Job::new_async_tz` rejects `Local` because the bound needs `chrono_tz::Tz`, add `chrono-tz = "0.10"` to `Cargo.toml` and pass `chrono_tz::Asia::Bangkok` instead (the filename stays local time either way).

- [ ] **Step 2: Run** — `cargo test -q schedule::` → 3 passed; `cargo build -q` clean.

- [ ] **Step 3: Commit**

```powershell
git add -A
git commit -q -F - @'
feat(v2): local-time cron scheduler with boot catch-up

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
'@
```

---

### Task 10: HTTP API (axum) + embedded static + end-to-end test

**Files:**
- Replace: `src-tauri/src/api/mod.rs`
- Create: `src-tauri/src/api/{status,storages,plans,runs,static_files}.rs`, `web/index.html`, `src-tauri/tests/api.rs`

**Interfaces:**
- Consumes: everything from Tasks 2–9.
- Produces: `api::router(state: AppState) -> axum::Router`, `api::{CLIENT_HEADER, CLIENT_TOKEN, ApiError, ok_false, ok_true}`. Routes exactly as spec §10 (axum 0.8 path params use `{id}`).

- [ ] **Step 1: `web/index.html` placeholder** (Plan 2 replaces it)

```html
<!doctype html>
<html lang="th"><head><meta charset="utf-8"><title>PowerPCU Backup</title></head>
<body><h1>PowerPCU Backup service กำลังทำงาน</h1><p>หน้าตั้งค่าจะมาใน Plan 2 — ตอนนี้ดูสถานะได้ที่ <code>/api/status</code> (ต้องส่ง header x-powerpcu-client)</p></body></html>
```

- [ ] **Step 2: `api/mod.rs`**

```rust
//! axum layer over the engine: parse → validate → engine → JSON. Business outcomes are
//! 200 `{ok, message}`; only DB/unexpected failures are 500. Every `/api/*` request must
//! carry `x-powerpcu-client: backup-ui` (blocks cross-site form posts to localhost).

pub mod plans;
pub mod runs;
pub mod static_files;
pub mod status;
pub mod storages;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::state::AppState;

pub const CLIENT_HEADER: &str = "x-powerpcu-client";
pub const CLIENT_TOKEN: &str = "backup-ui";

pub struct ApiError(pub String);
impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self { ApiError(e.to_string()) }
}
impl From<String> for ApiError {
    fn from(e: String) -> Self { ApiError(e) }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        tracing::error!("api: {}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": self.0 }))).into_response()
    }
}

pub fn ok_false(message: impl Into<String>) -> Json<Value> {
    Json(json!({ "ok": false, "message": message.into() }))
}
pub fn ok_true(message: impl Into<String>) -> Json<Value> {
    Json(json!({ "ok": true, "message": message.into() }))
}

async fn client_guard(req: Request, next: Next) -> Response {
    let ok = req.headers().get(CLIENT_HEADER).and_then(|v| v.to_str().ok()) == Some(CLIENT_TOKEN);
    if !ok {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "forbidden" }))).into_response();
    }
    next.run(req).await
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/status", get(status::get_status))
        .route("/storages", get(storages::list).post(storages::create))
        .route("/storages/test", post(storages::test_config))
        .route("/storages/{id}", put(storages::update).delete(storages::remove))
        .route("/storages/{id}/test", post(storages::test_saved))
        .route("/plans", get(plans::list).post(plans::create))
        .route("/plans/test-connection", post(plans::test_connection))
        .route("/plans/{id}", get(plans::get_one).put(plans::update).delete(plans::remove))
        .route("/plans/{id}/run", post(plans::run_now))
        .route("/plans/{id}/runs", get(runs::list_for_plan))
        .route("/runs/{id}/files", get(runs::files_for_run))
        .layer(middleware::from_fn(client_guard))
        .with_state(state);
    Router::new().nest("/api", api).fallback(static_files::serve)
}
```

- [ ] **Step 3: `api/status.rs`**

```rust
use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::api::ApiError;
use crate::backup::{plans, running, runs, schedule};
use crate::state::AppState;

/// Dashboard + tray poll: version, uptime, and per-plan running/next/last.
pub async fn get_status(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let mut out = Vec::new();
    for p in plans::list_all(&st.pool).await? {
        let last = runs::last_for_plan(&st.pool, p.id).await?;
        let next = schedule::next_run_at(p.id).await.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string());
        out.push(json!({
            "id": p.id, "name": p.name, "active": p.active,
            "running": running::is_running(p.id),
            "next_run_at": next,
            "last_run": last,
        }));
    }
    Ok(Json(json!({ "version": env!("CARGO_PKG_VERSION"), "started_at": st.started_at, "plans": out })))
}
```

- [ ] **Step 4: `api/storages.rs`**

```rust
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{ok_false, ok_true, ApiError};
use crate::backup::storage::{self, Storage, StorageConfig};
use crate::backup::storages;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct StorageBody {
    pub name: String,
    pub provider: String,
    pub config: Value,
    /// test_config only: reuse the stored secret of this storage when the form left it blank.
    #[serde(default)]
    pub storage_id: Option<i64>,
}

fn parse(body: &StorageBody) -> Result<StorageConfig, String> {
    if body.name.trim().is_empty() {
        return Err("กรุณาตั้งชื่อปลายทาง".into());
    }
    StorageConfig::parse(&body.provider, &body.config.to_string())
}

pub async fn list(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let mut out = Vec::new();
    for s in storages::list_all(&st.pool).await? {
        let mut v = s.to_public_value();
        v["in_use"] = json!(storages::is_in_use(&st.pool, s.id).await?);
        out.push(v);
    }
    Ok(Json(Value::Array(out)))
}

pub async fn create(State(st): State<AppState>, Json(body): Json<StorageBody>) -> Result<Json<Value>, ApiError> {
    let mut cfg = match parse(&body) { Ok(c) => c, Err(m) => return Ok(ok_false(m)) };
    if cfg.secret_is_empty() {
        return Ok(ok_false("กรุณากรอก secret key"));
    }
    cfg.protect_secret()?;
    let id = storages::insert(&st.pool, body.name.trim(), cfg.provider_str(), &cfg.to_json()).await?;
    Ok(Json(json!({ "ok": true, "id": id })))
}

pub async fn update(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<StorageBody>) -> Result<Json<Value>, ApiError> {
    let Some(old) = storages::find(&st.pool, id).await? else { return Ok(ok_false("ไม่พบปลายทาง")) };
    let mut cfg = match parse(&body) { Ok(c) => c, Err(m) => return Ok(ok_false(m)) };
    cfg.keep_secret_from(&old.config);
    if cfg.secret_is_empty() {
        return Ok(ok_false("กรุณากรอก secret key"));
    }
    cfg.protect_secret()?;
    storages::update(&st.pool, id, body.name.trim(), cfg.provider_str(), &cfg.to_json()).await?;
    Ok(ok_true("บันทึกแล้ว"))
}

pub async fn remove(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    if storages::find(&st.pool, id).await?.is_none() {
        return Ok(ok_false("ไม่พบปลายทาง"));
    }
    if storages::is_in_use(&st.pool, id).await? {
        return Ok(ok_false("ลบไม่ได้: ปลายทางนี้ยังถูกใช้โดยแผน หรือยังมีไฟล์สำรองอยู่"));
    }
    storages::delete(&st.pool, id).await?;
    Ok(ok_true("ลบแล้ว"))
}

pub async fn test_saved(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    let Some(s) = storages::find(&st.pool, id).await? else { return Ok(ok_false("ไม่พบปลายทาง")) };
    Ok(run_test(&s).await)
}

pub async fn test_config(State(st): State<AppState>, Json(body): Json<StorageBody>) -> Result<Json<Value>, ApiError> {
    let mut cfg = match parse(&body) { Ok(c) => c, Err(m) => return Ok(ok_false(m)) };
    if cfg.secret_is_empty() {
        if let Some(old) = match body.storage_id { Some(id) => storages::find(&st.pool, id).await?, None => None } {
            cfg.keep_secret_from(&old.config);
        }
    }
    if cfg.secret_is_empty() {
        return Ok(ok_false("กรุณากรอก secret key"));
    }
    Ok(run_test(&Storage { id: 0, name: body.name, config: cfg }).await)
}

async fn run_test(s: &Storage) -> Json<Value> {
    match storage::for_storage(s) {
        Ok(p) => match p.test().await {
            Ok(()) => ok_true("เชื่อมต่อสำเร็จ"),
            Err(m) => ok_false(m),
        },
        Err(m) => ok_false(m),
    }
}
```

- [ ] **Step 5: `api/plans.rs`**

```rust
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{ok_false, ok_true, ApiError};
use crate::backup::plans::{self, NewPlan};
use crate::backup::{mysqldump, run, running, schedule, storages};
use crate::secret;
use crate::state::AppState;

fn validate(p: &NewPlan) -> Result<(), String> {
    if p.name.trim().is_empty() {
        return Err("กรุณาตั้งชื่อแผน".into());
    }
    if p.database_name.trim().is_empty() || p.host.trim().is_empty() || p.username.trim().is_empty() {
        return Err("กรุณากรอกข้อมูลการเชื่อมต่อฐานข้อมูลให้ครบ".into());
    }
    if p.prefix_name.is_empty() || !p.prefix_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("รหัส/ชื่อย่อหน่วยบริการต้องเป็นตัวอักษรอังกฤษ ตัวเลข _ หรือ - เท่านั้น".into());
    }
    if let Some(c) = p.schedule_cron.as_deref() {
        if c.split_whitespace().count() != 5 {
            return Err("รูปแบบ cron ต้องมี 5 ช่อง (นาที ชั่วโมง วันที่ เดือน วันในสัปดาห์)".into());
        }
    }
    Ok(())
}

/// Blank cron → manual (`None`); otherwise trimmed.
fn normalize_cron(p: &mut NewPlan) {
    p.schedule_cron = p.schedule_cron.take().map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
}

async fn resolve_storages(st: &AppState, p: &NewPlan, exclude: Option<i64>) -> Result<Result<Vec<i64>, String>, ApiError> {
    let ids = storages::existing_ids(&st.pool, &p.storage_ids).await?;
    if ids.is_empty() {
        return Ok(Err("ต้องเลือกปลายทางอย่างน้อย 1 แห่ง".into()));
    }
    if let Some(c) = plans::find_conflict(&st.pool, exclude, &p.prefix_name, &p.database_name, &ids).await? {
        return Ok(Err(format!(
            "แผน '{}' ใช้รหัสหน่วยบริการ+ฐานข้อมูลเดียวกันบนปลายทาง '{}' อยู่แล้ว เปลี่ยนรหัสหน่วยบริการหรือปลายทาง",
            c.plan_name, c.storage_name
        )));
    }
    Ok(Ok(ids))
}

const ZIP_PW_MSG: &str = "รหัสเข้ารหัสไฟล์ต้องมีอย่างน้อย 8 ตัวอักษร";

pub async fn list(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let mut out = Vec::new();
    for p in plans::list_all(&st.pool).await? {
        let ids = plans::storage_ids_for(&st.pool, p.id).await?;
        out.push(p.to_public_value(&ids));
    }
    Ok(Json(Value::Array(out)))
}

pub async fn get_one(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    Ok(Json(match plans::find(&st.pool, id).await? {
        Some(p) => p.to_public_value(&plans::storage_ids_for(&st.pool, id).await?),
        None => Value::Null,
    }))
}

pub async fn create(State(st): State<AppState>, Json(mut body): Json<NewPlan>) -> Result<Json<Value>, ApiError> {
    normalize_cron(&mut body);
    if let Err(m) = validate(&body) {
        return Ok(ok_false(m));
    }
    let ids = match resolve_storages(&st, &body, None).await? { Ok(ids) => ids, Err(m) => return Ok(ok_false(m)) };
    if body.password.is_empty() {
        let src = match body.copy_password_from_plan_id { Some(id) => plans::find(&st.pool, id).await?, None => None };
        match src {
            Some(p) => body.password = p.password, // already a dpapi blob
            None => return Ok(ok_false("กรุณากรอกรหัสผ่านฐานข้อมูล")),
        }
    } else {
        body.password = secret::protect(&body.password)?;
    }
    if body.encryption_password.chars().count() < 8 {
        return Ok(ok_false(ZIP_PW_MSG));
    }
    body.encryption_password = secret::protect(&body.encryption_password)?;

    let id = plans::insert(&st.pool, &body).await?;
    plans::replace_links(&st.pool, id, &ids).await?;
    if let Some(p) = plans::find(&st.pool, id).await? {
        schedule::register_plan(&st, &p).await;
    }
    Ok(Json(json!({ "ok": true, "id": id })))
}

pub async fn update(State(st): State<AppState>, Path(id): Path<i64>, Json(mut body): Json<NewPlan>) -> Result<Json<Value>, ApiError> {
    let Some(old) = plans::find(&st.pool, id).await? else { return Ok(ok_false("ไม่พบแผน")) };
    normalize_cron(&mut body);
    if let Err(m) = validate(&body) {
        return Ok(ok_false(m));
    }
    let ids = match resolve_storages(&st, &body, Some(id)).await? { Ok(ids) => ids, Err(m) => return Ok(ok_false(m)) };
    body.password = if body.password.is_empty() { old.password.clone() } else { secret::protect(&body.password)? };
    body.encryption_password = if body.encryption_password.is_empty() {
        old.encryption_password.clone()
    } else {
        if body.encryption_password.chars().count() < 8 {
            return Ok(ok_false(ZIP_PW_MSG));
        }
        secret::protect(&body.encryption_password)?
    };
    plans::update(&st.pool, id, &body).await?;
    plans::replace_links(&st.pool, id, &ids).await?;
    schedule::unregister_plan(id).await;
    if let Some(p) = plans::find(&st.pool, id).await? {
        schedule::register_plan(&st, &p).await;
    }
    Ok(ok_true("บันทึกแล้ว"))
}

pub async fn remove(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    if plans::find(&st.pool, id).await?.is_none() {
        return Ok(ok_false("ไม่พบแผน"));
    }
    if running::is_running(id) {
        return Ok(ok_false("แผนนี้กำลังสำรองข้อมูลอยู่ รอให้เสร็จก่อน"));
    }
    schedule::unregister_plan(id).await;
    plans::delete(&st.pool, id).await?;
    Ok(ok_true("ลบแผนแล้ว (ไฟล์สำรองที่ปลายทางยังอยู่)"))
}

pub async fn run_now(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    let Some(plan) = plans::find(&st.pool, id).await? else { return Ok(ok_false("ไม่พบแผน")) };
    if running::is_running(id) {
        return Ok(ok_false("กำลังสำรองข้อมูลอยู่"));
    }
    let st2 = st.clone();
    tokio::spawn(async move {
        run::run_backup(&st2, &plan, "manual").await;
    });
    Ok(ok_true("เริ่มสำรองข้อมูลแล้ว"))
}

#[derive(Deserialize)]
pub struct TestConn {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub password: String,
    pub database_name: String,
    #[serde(default)]
    pub plan_id: Option<i64>,
}

/// Runs a schema-only mysqldump with the given (or the plan's stored) password.
pub async fn test_connection(State(st): State<AppState>, Json(body): Json<TestConn>) -> Result<Json<Value>, ApiError> {
    let password = if !body.password.is_empty() {
        body.password.clone()
    } else {
        match body.plan_id {
            Some(id) => match plans::find(&st.pool, id).await? {
                Some(p) => secret::unprotect(&p.password)?,
                None => return Ok(ok_false("ไม่พบแผน")),
            },
            None => return Ok(ok_false("กรุณากรอกรหัสผ่านฐานข้อมูล")),
        }
    };
    let _ = std::fs::create_dir_all(&st.temp_dir);
    let src = mysqldump::Source { mysqldump: &st.mysqldump, database: &body.database_name, host: &body.host, port: body.port, username: &body.username, password: &password };
    Ok(match mysqldump::probe(&src, &st.temp_dir).await {
        Ok(()) => ok_true("เชื่อมต่อฐานข้อมูลสำเร็จ"),
        Err(e) => ok_false(mysqldump::error_message(e).replace(&password, "***")),
    })
}
```

- [ ] **Step 6: `api/runs.rs`**

```rust
use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde_json::{json, Value};

use crate::api::ApiError;
use crate::backup::prune::Reason;
use crate::backup::{files, plans, runs};
use crate::state::AppState;

pub async fn list_for_plan(State(st): State<AppState>, Path(id): Path<i64>, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, ApiError> {
    let limit = q.get("limit").and_then(|v| v.parse::<i64>().ok()).unwrap_or(50).clamp(1, 500);
    Ok(Json(json!(runs::list_for_plan(&st.pool, id, limit).await?)))
}

/// Files of one run, each with `kept_as` = the rules currently keeping it (empty when deleted).
pub async fn files_for_run(State(st): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    let list = files::list_for_run(&st.pool, id).await?;
    let plan = match list.first() { Some(f) => plans::find(&st.pool, f.plan_id).await?, None => None };
    let now = chrono::Local::now().naive_local();
    let mut by_storage: HashMap<i64, HashMap<i64, Vec<Reason>>> = HashMap::new();
    let mut out = Vec::new();
    for f in list {
        let mut reasons: Vec<Reason> = Vec::new();
        if f.deleted_at.is_none() {
            if let Some(p) = &plan {
                if !by_storage.contains_key(&f.storage_id) {
                    let kept = files::kept_as(&st.pool, p, f.storage_id, now).await.unwrap_or_default();
                    by_storage.insert(f.storage_id, kept);
                }
                reasons = by_storage[&f.storage_id].get(&f.id).cloned().unwrap_or_default();
            }
        }
        let mut v = serde_json::to_value(&f).unwrap_or(Value::Null);
        v["kept_as"] = json!(reasons);
        out.push(v);
    }
    Ok(Json(Value::Array(out)))
}
```

- [ ] **Step 7: `api/static_files.rs`**

```rust
//! Serves the embedded `web/` folder (Alpine UI in Plan 2). Unknown paths fall back to
//! `index.html`; `index.html` is `no-cache` so an upgrade never shows a stale UI.

use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::RustEmbed)]
#[folder = "../web"]
struct Web;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let key = if path.is_empty() { "index.html" } else { path };
    let (file, name) = match Web::get(key) {
        Some(f) => (f, key),
        None => match Web::get("index.html") {
            Some(f) => (f, "index.html"),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    let mime = file.metadata.mimetype().to_string();
    let mut res = (StatusCode::OK, [(header::CONTENT_TYPE, mime)], file.data.into_owned()).into_response();
    if name == "index.html" {
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    res
}
```

- [ ] **Step 8: Build** — `cargo build -q` → clean (fix any unused-import warnings).

- [ ] **Step 9: End-to-end test `src-tauri/tests/api.rs`**

```rust
//! Drives the real router in-process (no port) with an in-memory DB and a fake mysqldump.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use powerpcu_backup::api::{self, CLIENT_HEADER, CLIENT_TOKEN};
use powerpcu_backup::state::AppState;

struct Env {
    app: Router,
    work: std::path::PathBuf,
    dest: std::path::PathBuf,
}

async fn setup(tag: &str) -> Env {
    let pool = powerpcu_backup::db::open_memory().await;
    let work = std::env::temp_dir().join(format!("ppb-api-{}-{tag}", std::process::id()));
    let temp = work.join("temp");
    let dest = work.join("dest");
    std::fs::create_dir_all(&temp).unwrap();
    let fake = work.join("mysqldump.cmd");
    std::fs::write(&fake, "@echo off\r\nset \"rf=%~6\"\r\nset \"rf=%rf:--result-file=%\"\r\necho -- fake dump> \"%rf%\"\r\nexit /b 0\r\n").unwrap();
    Env { app: api::router(AppState::new(pool, fake, temp)), work, dest }
}

async fn call(app: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(path).header(CLIENT_HEADER, CLIENT_TOKEN);
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn plan_body(name: &str, prefix: &str, storage_ids: Vec<i64>) -> Value {
    json!({
        "name": name, "database_name": "jhcisdb", "host": "127.0.0.1", "port": 3333, "username": "root",
        "password": "secret-pw", "prefix_name": prefix, "encryption_password": "zip-pass-123",
        "schedule_cron": "30 3 * * *", "storage_ids": storage_ids, "keep_daily": 1, "keep_weekly": 0, "keep_monthly": 0, "keep_yearly": 0
    })
}

#[tokio::test]
async fn guard_rejects_missing_header_and_serves_index() {
    let e = setup("guard").await;
    let res = e.app.clone().oneshot(Request::get("/api/status").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = e.app.clone().oneshot(Request::get("/").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["cache-control"], "no-cache");
    let _ = std::fs::remove_dir_all(&e.work);
}

#[tokio::test]
async fn full_flow_storage_plan_run_history() {
    let e = setup("flow").await;
    let app = &e.app;

    // storage
    let (_, v) = call(app, "POST", "/api/storages", Some(json!({"name": "โฟลเดอร์", "provider": "local", "config": {"path": e.dest.to_string_lossy()}}))).await;
    assert_eq!(v["ok"], true, "{v}");
    let sid = v["id"].as_i64().unwrap();
    let (_, v) = call(app, "POST", &format!("/api/storages/{sid}/test"), None).await;
    assert_eq!(v["ok"], true, "{v}");
    let (_, v) = call(app, "GET", "/api/storages", None).await;
    assert_eq!(v[0]["in_use"], false);

    // plan validation
    let mut bad = plan_body("x", "10999", vec![sid]);
    bad["password"] = json!("");
    let (_, v) = call(app, "POST", "/api/plans", Some(bad)).await;
    assert_eq!(v["ok"], false);
    assert!(v["message"].as_str().unwrap().contains("รหัสผ่านฐานข้อมูล"));
    let mut bad = plan_body("x", "10999", vec![sid]);
    bad["encryption_password"] = json!("short");
    let (_, v) = call(app, "POST", "/api/plans", Some(bad)).await;
    assert_eq!(v["ok"], false);
    let (_, v) = call(app, "POST", "/api/plans", Some(plan_body("x", "bad prefix!", vec![sid]))).await;
    assert_eq!(v["ok"], false);

    // plan ok, masked on read
    let (_, v) = call(app, "POST", "/api/plans", Some(plan_body("รพ.สต.", "10999", vec![sid]))).await;
    assert_eq!(v["ok"], true, "{v}");
    let pid = v["id"].as_i64().unwrap();
    let (_, v) = call(app, "GET", &format!("/api/plans/{pid}"), None).await;
    assert_eq!(v["password"], "");
    assert_eq!(v["encryption_password"], "");
    assert_eq!(v["storage_ids"], json!([sid]));

    // conflict: same prefix+db on same storage
    let (_, v) = call(app, "POST", "/api/plans", Some(plan_body("ซ้ำ", "10999", vec![sid]))).await;
    assert_eq!(v["ok"], false);
    assert!(v["message"].as_str().unwrap().contains("รพ.สต."));

    // duplicate plan copying the password server-side
    let mut dup = plan_body("ส่วนกลาง", "10999-c", vec![sid]);
    dup["password"] = json!("");
    dup["copy_password_from_plan_id"] = json!(pid);
    let (_, v) = call(app, "POST", "/api/plans", Some(dup)).await;
    assert_eq!(v["ok"], true, "{v}");

    // test-connection via fake mysqldump using the stored password
    let (_, v) = call(app, "POST", "/api/plans/test-connection", Some(json!({"host": "h", "port": 3333, "username": "root", "database_name": "jhcisdb", "plan_id": pid}))).await;
    assert_eq!(v["ok"], true, "{v}");

    // run + poll
    let (_, v) = call(app, "POST", &format!("/api/plans/{pid}/run"), None).await;
    assert_eq!(v["ok"], true, "{v}");
    let mut run = Value::Null;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (_, list) = call(app, "GET", &format!("/api/plans/{pid}/runs?limit=5"), None).await;
        if list[0]["status"] != "running" && !list[0].is_null() { run = list[0].clone(); break; }
    }
    assert_eq!(run["status"], "ok", "{run}");
    let run_id = run["id"].as_i64().unwrap();
    let (_, files) = call(app, "GET", &format!("/api/runs/{run_id}/files"), None).await;
    assert_eq!(files.as_array().unwrap().len(), 1);
    assert!(std::path::Path::new(files[0]["location"].as_str().unwrap()).exists());
    assert!(files[0]["kept_as"].as_array().unwrap().iter().any(|r| r == "daily"));

    // status
    let (_, s) = call(app, "GET", "/api/status", None).await;
    assert_eq!(s["plans"][0]["last_run"]["status"], "ok");
    assert_eq!(s["version"], env!("CARGO_PKG_VERSION"));

    // storage in use → cannot delete; plan delete keeps files → still in use
    let (_, v) = call(app, "DELETE", &format!("/api/storages/{sid}"), None).await;
    assert_eq!(v["ok"], false);
    let (_, v) = call(app, "DELETE", &format!("/api/plans/{pid}"), None).await;
    assert_eq!(v["ok"], true);
    let (_, v) = call(app, "DELETE", &format!("/api/storages/{sid}"), None).await;
    assert_eq!(v["ok"], false, "live backup file still references it");
    let _ = std::fs::remove_dir_all(&e.work);
}
```

- [ ] **Step 10: Run** — `cargo test -q --test api` → 2 passed; `cargo test -q` → everything green.

- [ ] **Step 11: Commit**

```powershell
git add -A
git commit -q -F - @'
feat(v2): axum API (status, storages, plans, runs), client-header guard, embedded web, e2e test

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
'@
```

---

### Task 11: Service boot, logging, CLAUDE.md

**Files:**
- Replace: `src-tauri/src/service.rs`, `CLAUDE.md`

**Interfaces:**
- Consumes: `paths`, `db`, `runs::fail_stale_running`, `schedule::{start, catch_up}`, `api::router`
- Produces: `service::run() -> Result<(), Box<dyn Error>>` used by `main.rs --service`.

- [ ] **Step 1: `service.rs`**

```rust
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
```

- [ ] **Step 2: Manual verification (no MySQL needed for the API; MySQL needed for a real dump)**

```powershell
cd src-tauri
$env:POWERPCU_BACKUP_DATA_DIR = "$env:TEMP\ppb-dev"
$env:MYSQLDUMP_PATH = "$PWD\..\lib\mysql-5.6.45-winx64\mysqldump.exe"
cargo run -q -- --service
```

In another shell:

```powershell
$h = @{ 'x-powerpcu-client' = 'backup-ui'; 'content-type' = 'application/json' }
Invoke-RestMethod http://127.0.0.1:8720/api/status -Headers $h
Invoke-RestMethod http://127.0.0.1:8720/api/storages -Method Post -Headers $h -Body '{"name":"โฟลเดอร์","provider":"local","config":{"path":"C:\\PowerPCU-Backup-test"}}'
Invoke-RestMethod http://127.0.0.1:8720/api/plans -Method Post -Headers $h -Body '{"name":"รพ.สต.","database_name":"jhcisdb","host":"127.0.0.1","port":3333,"username":"root","password":"<pw>","prefix_name":"10999","encryption_password":"zip-pass-123","schedule_cron":"30 3 * * *","storage_ids":[1]}'
Invoke-RestMethod http://127.0.0.1:8720/api/plans/1/run -Method Post -Headers $h
Invoke-RestMethod http://127.0.0.1:8720/api/plans/1/runs -Headers $h
```

Expected with a real JHCIS MySQL on 3333: the run reaches `ok`, a `.zip` appears under `C:\PowerPCU-Backup-test\10999\jhcisdb\`, and 7-Zip opens it with the password. Without MySQL: the run is `failed` with mysqldump's Thai/English error in `message`, the API stays up.

- [ ] **Step 3: Rewrite `CLAUDE.md` for v2** (replaces the v1 one)

```markdown
# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

PowerPCU Backup v2 (branch `v2`): a Rust service that dumps a MySQL database (JHCIS, port 3333), zips it with AES-256 + Zstd, ships it to many destinations (local folder / S3-compatible), keeps restic-style tiered history, and serves a JSON API + web UI on 127.0.0.1:8720. One exe, two modes: `--service` (headless, run by NSSM) and GUI (Tauri, Plan 2). Windows 10+ only.

Design spec: `docs/superpowers/specs/2026-09-07-powerpcu-backup-v2-design.md`. Plans: `docs/superpowers/plans/`.

## Commands

```powershell
cd src-tauri
cargo test -q                          # all tests (Windows only: DPAPI + .cmd fakes)
cargo test -q prune::                  # one module
cargo test -q --test api               # end-to-end through the router (fake mysqldump, in-memory DB)
$env:POWERPCU_BACKUP_DATA_DIR = "$env:TEMP\ppb-dev"; $env:MYSQLDUMP_PATH = "$PWD\..\lib\mysql-5.6.45-winx64\mysqldump.exe"; cargo run -q -- --service
cargo run -q -- --version
```

Tests never read env vars; everything comes through `state::AppState` (pool, mysqldump path, temp dir). A fake `mysqldump.cmd` stands in for the real binary (`backup::mysqldump::fake_script`, or the `%~6` variant in `run.rs`/`tests/api.rs`).

## Layout

- `src-tauri/src/backup/` — engine, framework-free. `run.rs` is one backup (dump → zip → ship → record → prune); `schedule.rs` fires runs on a local-time cron and does boot catch-up; `prune.rs` is the pure keep-by-rule policy; `files.rs` applies it against `backup_files` rows only.
- `src-tauri/src/api/` — axum handlers; business outcomes are 200 `{ok,message}` in Thai. Every `/api/*` request needs `x-powerpcu-client: backup-ui`.
- `src-tauri/src/secret.rs` — DPAPI machine scope; stored form `dpapi:<base64>`; API masks secrets to `""`, and an empty secret on update means keep.
- `web/` — embedded by rust-embed (served from disk in debug builds, so UI edits need no rebuild).
- `lib/` — bundled `mysqldump.exe` 5.6.45 and `nssm.exe` (installer, Plan 3).

## Rules that are easy to break

- Retention deletes only rows in `backup_files` for that plan + storage. Never list a folder or bucket to decide deletions.
- Exactly one `mysqldump` at a time (`run::DUMP_SEMAPHORE`); per-plan overlap is refused by `running::try_start`.
- Two plans may not share (prefix_name, database_name) on the same storage (`plans::find_conflict`).
- S3: keep `RequestChecksumCalculation::WhenRequired` / `ResponseChecksumValidation::WhenRequired` and path-style; MinIO/Ceph/R2 reject the SDK defaults. Do not switch to `rust-s3`.
- Cron is 5-field local time; `schedule::to_scheduler_expr` prepends seconds.
- axum 0.8 route params are `{id}`, not `:id`.
```

- [ ] **Step 4: Full test run** — `cargo test -q` → all green; `cargo build --release -q` succeeds.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -q -F - @'
feat(v2): service boot (db, stale runs, scheduler, catch-up, axum) + CLAUDE.md for v2

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
'@
```

---

## Self-review against the spec

**Coverage (spec section → task):** §3 process model (service half) → 1, 11; §4 layout + POC port map → 1, 4, 5, 6, 8, 9; §5 tables → 2; §5.2 multi-plan (semaphore, conflict, copy password) → 8, 6/10, 10; §6 run flow incl. 4.5 and stale-run cleanup → 8, 11; §7 prune + kept_as → 7, 8, 10; §8 scheduler + catch-up → 9; §9 providers → 5; §10 API → 10; §13 secrets/guard → 3, 5, 10; §14 logging → 11; §16 tests → every task. Not in this plan by design: §11 web UI and §12 Tauri (Plan 2), §15 installer (Plan 3), the `POWERPCU_BACKUP_PORT` override is read but not exposed (spec §3 says no UI for it).

**Known compile-risk points (fix inline, do not redesign):** `Job::new_async_tz(.., Local, ..)` bound (fallback documented in Task 9); `CryptUnprotectData` second parameter pointer type (Task 3); rust-embed derive name (`RustEmbed` used); `base64 0.23` engine import path (if `general_purpose::STANDARD` moved, use `base64::prelude::BASE64_STANDARD`).

**Edge case handled beyond the spec text:** two runs of one plan within the same minute share a file name; `files::supersede` retires the overwritten row so prune never deletes the fresh copy (Task 8).

**Type consistency checked:** `Plan.keep() -> prune::Keep`; `files::record(pool, run_id, plan_id, storage_id, provider, location, size: i64, created_at)` matches its call in `run.rs`; `storage::for_storage(&Storage) -> Result<Provider,String>` used by `files::apply_prune`, `run.rs`, `api/storages.rs`; `run_backup(&AppState, &Plan, &str) -> RunOutcome` used by `schedule.rs` and `api/plans.rs`; `mysqldump::Source.mysqldump: &Path` supplied from `AppState.mysqldump` everywhere.

## Next plans

- Plan 2 — `web/` Alpine UI (spec §11) + Tauri shell (§12): written after this plan lands so it targets the real API.
- Plan 3 — Inno Setup + NSSM installer (§15).
