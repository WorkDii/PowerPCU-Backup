//! PowerPCU Backup v2 library: engine (`backup`), HTTP API (`api`), boot (`service`).
//! The binary in `main.rs` only dispatches `--service` / `--version` (GUI comes in Plan 2).

pub mod api;
pub mod backup;
pub mod db;
pub mod paths;
pub mod secret;
pub mod service;
pub mod state;
