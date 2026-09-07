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
