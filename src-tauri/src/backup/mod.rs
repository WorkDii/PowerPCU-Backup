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
