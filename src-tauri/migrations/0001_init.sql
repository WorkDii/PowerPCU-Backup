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
