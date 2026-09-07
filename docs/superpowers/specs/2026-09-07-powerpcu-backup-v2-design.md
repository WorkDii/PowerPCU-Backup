# PowerPCU Backup v2 — Design Spec

- **วันที่:** 2026-09-07
- **Branch:** `v2` (repo `powerpcu_backup`)
- **สถานะ:** อนุมัติดีไซน์แล้ว รอเขียน implementation plan
- **แทนที่:** v1 (Deno, branch `main`) และ MySQL Administrator ที่ รพ.สต. ใช้สำรอง JHCIS

## 1. เป้าหมาย

โปรแกรมสำรองฐานข้อมูล MySQL (JHCIS) สำหรับเครื่อง server ของ รพ.สต. ที่

1. ทำงานเบื้องหลังเป็น Windows service ไม่ต้องมีใคร login
2. มี GUI ตั้งค่าง่าย ค่าเริ่มต้นตรงกับ JHCIS กรอกน้อยที่สุด
3. ส่งสำเนาไปได้หลายปลายทางพร้อมกัน (โฟลเดอร์/network share + S3-compatible)
4. เก็บประวัติระยะยาวแบบ keep-by-rule (รายวัน/สัปดาห์/เดือน/ปี) โดยไม่มีจุดตายของ v1
5. เห็นสถานะได้จาก dashboard และ tray icon

### ขอบเขตรอบนี้ (in)

- exe เดียว 2 โหมด: `--service` (headless) และ GUI (Tauri)
- ปลายทาง `local` และ `s3`
- แผน (plan) หลายแผน 1 แผน = 1 ฐานข้อมูล ผูกปลายทางได้หลายอัน (M:N) และ**หลายแผนชี้ฐานเดียวกันได้** (เช่น แผนของส่วนกลางกับแผนของ รพ.สต. ที่ใช้รหัส zip, retention, ปลายทาง, เวลา ต่างกัน ดู §5.2)
- schedule แบบง่ายใน UI แปลงเป็น cron เวลาท้องถิ่น
- retention keep-by-rule แบบ restic + prune หลังรันสำเร็จเท่านั้น
- ประวัติการรัน + ไฟล์ต่อรัน
- ตัวติดตั้ง Inno Setup + NSSM

### นอกขอบเขตรอบนี้ (out, ดู §19)

Restore ในโปรแกรม, backup ครั้งเดียวไปโฟลเดอร์ที่เลือก, แจ้งเตือนภายนอก (MOPH Alert/LINE), PIN/login, auto-update, Windows 8, import ค่าจาก v1, ลบไฟล์เก่าที่ v2 ไม่ได้สร้าง, per-storage retention

## 2. การตัดสินใจที่ล็อกแล้ว

| เรื่อง | ตัดสินใจ | เหตุผลย่อ |
|---|---|---|
| ภาษา/เฟรมเวิร์ก | Rust stable (1.96) + Tauri 2 | ตามโจทย์ |
| OS | Windows 10+ x64 เท่านั้น | Rust 1.78+ ต้องการ Win10; Win8 ต้อง nightly tier-3 + VM ทดสอบ ไม่คุ้ม |
| Background | exe เดียว 2 โหมด, service ผ่าน NSSM | รันได้โดยไม่ login; NSSM มีใน `lib/` แล้ว ไม่ต้องเขียน SCM |
| GUI ↔ service | service เสิร์ฟ UI + JSON API บน `127.0.0.1:8720`; Tauri เป็นหน้าต่างชี้ URL นั้น + tray | แบบเดียวกับ POC launcher; dev UI ได้โดยไม่ต้อง Tauri |
| HTTP framework | axum + tower-http | ทีม tokio ดูแล, handler ธรรมดา; โค้ด engine ของ POC ไม่ผูก framework |
| Web UI | HTML + Alpine.js (vendored) **ไม่มี build step, ไม่มี npm** | โจทย์ต้องการ minimal ไม่ใช้ React |
| Retention | keep-by-rule แบบ restic (นับจำนวน, OR ทุกกฎ) | ไม่ลบจนหมดเมื่อ backup พัง, ไม่หาย weekly เมื่อปิดเครื่อง, ไม่มีไฟล์ซ้ำ |
| ปลายทาง | local (รวม UNC) + S3-compatible | เท่า POC; provider trait เผื่อเพิ่ม |
| Secret at rest | DPAPI machine scope | ถอดได้เฉพาะเครื่องนั้น, 1 Win32 call |
| บีบอัด | `zip` crate: Zstd + AES-256 (ยกจาก POC) | ไม่ต้องมี 7-Zip; เปิดด้วย 7-Zip 21+/WinRAR 6+ |
| mysqldump | bundle 5.6.45 จาก `lib/` (ของ v1) | ตรงกับ MySQL ของ JHCIS |
| ข้อความ | ไทยทั้ง UI และ API message | ผู้ใช้เป็นเจ้าหน้าที่ รพ.สต. |

## 3. สถาปัตยกรรม

```
┌──────────────────────────── powerpcu-backup.exe ────────────────────────────┐
│                                                                              │
│  --service (NSSM, LocalSystem, cwd = C:\ProgramData\PowerPCU-Backup)         │
│    SQLite ─ migrations ─ scheduler (tokio-cron-scheduler, Local tz)          │
│    axum 127.0.0.1:8720 ── /api/*  ── /  (web/ ฝังด้วย rust-embed)            │
│                                                                              │
│  (ไม่มี arg) GUI (Tauri 2, user session, autostart)                          │
│    tray icon ── poll /api/status ทุก 60 วิ                                   │
│    window → WebviewUrl::External(http://127.0.0.1:8720/)                     │
│    service ไม่ตอบ → splash (app origin) + ปุ่ม "เริ่ม service" (runas)       │
│                                                                              │
│  --version                                                                   │
└──────────────────────────────────────────────────────────────────────────────┘
```

### ผังบนดิสก์

| ที่ | เนื้อหา |
|---|---|
| `C:\Program Files\PowerPCU-Backup\` | `powerpcu-backup.exe`, `bin\mysqldump.exe`, `bin\nssm.exe` |
| `C:\ProgramData\PowerPCU-Backup\` | `powerpcu-backup.db` (SQLite), `logs\` (NSSM หมุนไฟล์), `temp\` (dump/zip ระหว่างรัน) |

`mysqldump` หาจาก `<โฟลเดอร์ของ exe>\bin\mysqldump.exe` (ไม่ใช่ cwd) override ได้ด้วย env `MYSQLDUMP_PATH`

### พอร์ต

คงที่ `8720` (ทั้งสองโหมดใช้ค่าคงที่เดียวกัน) เปลี่ยนได้ด้วย env `POWERPCU_BACKUP_PORT` ที่ตั้งใน NSSM และในระบบ; รอบนี้ไม่ทำ UI ตั้งพอร์ต

## 4. โครงสร้าง repo (branch `v2`)

ไฟล์ v1 (Deno) ถูกลบออกบน branch นี้ใน commit แรกของการ implement: `main.ts`, `src/`, `deno.json`, `deno.lock`, `preBuild*`, `postBuild.ts`, `setup.iss`, `power_pcu_backup.zip`, `lib/7-Zip/`, `yarn.lock`, `node_modules/` เก็บ `lib/mysql-5.6.45-winx64/mysqldump.exe`, `lib/nssm-2.24/win64/nssm.exe`, `License.md` และเขียน `CLAUDE.md` ใหม่สำหรับ v2

```
src-tauri/
  Cargo.toml                 crate เดียว ชื่อ powerpcu-backup, version = จุดเดียวของทั้งโปรเจกต์
  tauri.conf.json            bundle.active=false (ใช้ Inno Setup แทน), frontendDist = "splash"
  build.rs                   tauri_build
  icons/                     icon.ico + tray icons 4 สถานะ (ok/running/warn/error)
  splash/index.html          หน้ารอ service (origin ของแอป)
  migrations/0001_init.sql
  src/main.rs                dispatch: --service | --version | GUI
  src/service.rs             boot: paths, db, migrate, scheduler, catch-up, axum serve
  src/api/mod.rs             router + client-header guard + error type
  src/api/status.rs          GET /api/status
  src/api/storages.rs        CRUD + test
  src/api/plans.rs           CRUD + run + test-connection
  src/api/runs.rs            runs + files
  src/api/static_files.rs    rust-embed web/
  src/backup/mod.rs
  src/backup/mysqldump.rs    (POC mysqldump.rs)
  src/backup/archive.rs      (POC archive.rs)
  src/backup/storage/{mod,local,s3}.rs   (POC storage/*)
  src/backup/storages.rs     store + masking (POC storages/store.rs + storage/mod.rs masking)
  src/backup/plans.rs        model + store + links (POC plans/{model,store,links}.rs)
  src/backup/run.rs          1 รอบการสำรอง (POC plans/run.rs ปรับ)
  src/backup/files.rs        backup_files record + delete (POC plans/files.rs ปรับ)
  src/backup/prune.rs        keep-by-rule pure function (ใหม่)
  src/backup/schedule.rs     scheduler + catch-up (POC schedule.rs + running.rs)
  src/backup/runs.rs         runs table (ใหม่)
  src/secret.rs              DPAPI protect/unprotect (ใหม่)
  src/gui/mod.rs             Tauri builder, plugins, window
  src/gui/tray.rs            tray + status poll
web/
  index.html  app.js  style.css  vendor/alpine.min.js
lib/
  mysql-5.6.45-winx64/mysqldump.exe   nssm-2.24/win64/nssm.exe
installer/
  setup.iss  build.ps1  vendor/MicrosoftEdgeWebview2Setup.exe
docs/superpowers/specs/2026-09-07-powerpcu-backup-v2-design.md
```

### ยกโค้ดจาก POC

โค้ด Rust ของโมดูล backup ใน POC อยู่ใน git history ของ `C:\Users\usman\repo\powerpcu_poc` ที่ commit `f45ee96^` (ก่อน migrate ไป Go) ดึงด้วย

```
git -C C:\Users\usman\repo\powerpcu_poc show f45ee96^:user-api/src/backup/archive.rs
```

| POC (`user-api/src/backup/`) | v2 | สิ่งที่เปลี่ยน |
|---|---|---|
| `archive.rs` | `backup/archive.rs` | รับชื่อ entry ในไฟล์ zip เป็นพารามิเตอร์ (ไม่ใช้ชื่อไฟล์ temp) |
| `mysqldump.rs`, `config.rs` | `backup/mysqldump.rs` | path จากโฟลเดอร์ exe; เพิ่ม `probe()` สำหรับทดสอบการเชื่อมต่อ |
| `storage/mod.rs`, `local.rs`, `s3.rs` | `backup/storage/` | secret ผ่าน `secret.rs`; ผังไฟล์ `<prefix>/<db>/<file>`; `test()` |
| `storages/store.rs` | `backup/storages.rs` | ตาราง `storages` เหมือนเดิม |
| `plans/{model,store,links}.rs` | `backup/plans.rs` | คอลัมน์ใหม่ (§5) |
| `plans/run.rs` | `backup/run.rs` | เขียน `runs`, ไม่มี tier, prune ตาม §7 |
| `plans/files.rs` | `backup/files.rs` | ตัด expire_at; ลบตามรายการที่ `prune.rs` คืน |
| `schedule.rs`, `running.rs` | `backup/schedule.rs` | `Job::new_async_tz(.., chrono::Local, ..)`; catch-up |
| `plans/handlers.rs`, `storages/handlers.rs`, `dashboard.rs` (Rocket) | `api/*.rs` | เขียนใหม่เป็น axum (~500 บรรทัด) |
| `web_static.rs` (Rocket) | `api/static_files.rs` | เขียนใหม่เป็น axum |
| `launcher/src/main.rs` (Tauri) | `gui/` | ตัด discovery/self-update; เพิ่ม tray |

ส่วนที่ไม่ยกมา: notify (MOPH), auth, appointment, discovery, update, DuckDB

## 5. Data model (SQLite, sqlx migrations)

```sql
CREATE TABLE storages (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT NOT NULL,
  provider    TEXT NOT NULL,              -- 'local' | 's3'
  config      TEXT NOT NULL,              -- JSON, secret เข้ารหัส DPAPI (§13)
  created_at  TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE plans (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  name                TEXT NOT NULL,
  database_name       TEXT NOT NULL,
  host                TEXT NOT NULL,
  port                INTEGER NOT NULL,
  username            TEXT NOT NULL,
  password            TEXT NOT NULL,      -- 'dpapi:<base64>'
  prefix_name         TEXT NOT NULL,
  encryption_password TEXT NOT NULL,      -- 'dpapi:<base64>' บังคับกรอก
  schedule_cron       TEXT,               -- 5 ช่อง เวลาท้องถิ่น; NULL = manual
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
  trigger      TEXT NOT NULL,             -- 'schedule' | 'manual' | 'catch_up'
  status       TEXT NOT NULL,             -- 'running' | 'ok' | 'partial' | 'failed'
  started_at   TEXT NOT NULL,
  finished_at  TEXT,
  message      TEXT NOT NULL DEFAULT '',
  size_bytes   INTEGER                    -- ขนาด zip (NULL ถ้า dump พัง)
);
CREATE INDEX idx_runs_plan ON runs(plan_id, started_at);

CREATE TABLE backup_files (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id      INTEGER NOT NULL,
  plan_id     INTEGER NOT NULL,
  storage_id  INTEGER NOT NULL,
  provider    TEXT NOT NULL,              -- denormalized ให้ลบได้แม้ storage หาย
  location    TEXT NOT NULL,              -- path เต็ม หรือ s3://bucket/key
  size_bytes  INTEGER NOT NULL,
  created_at  TEXT NOT NULL,              -- เวลาท้องถิ่นของรัน (ใช้จัดกลุ่ม prune)
  deleted_at  TEXT                        -- NULL = ยังอยู่
);
CREATE INDEX idx_backup_files_live ON backup_files(plan_id, storage_id, deleted_at);
```

หมายเหตุ

- ไม่มี `expire_at` และไม่มี tier: การเก็บ/ลบตัดสินโดย `prune.rs` ทุกครั้งจากไฟล์ที่ยังอยู่
- `storages` ลบไม่ได้ถ้ามี `plan_storages` หรือ `backup_files` ที่ `deleted_at IS NULL` อ้างถึง (กติกา POC)
- ลบ plan → ลบ `plan_storages` ของมัน; `runs`/`backup_files` เก็บไว้เป็นประวัติ ไฟล์จริงไม่ถูกลบ
- `runs` เก่ากว่า 90 วันถูกลบหลังทุกรัน (แถว `backup_files` ที่อ้างถึง run นั้นยังอยู่)

### 5.2 หลาย plan บนฐานเดียวกัน

กรณีจริง: ส่วนกลางต้องการสำเนาขึ้น S3 กลางด้วยรหัส zip และ retention ของส่วนกลาง ขณะที่ รพ.สต. เก็บสำเนาไว้ในเครื่องด้วยรหัสของตัวเอง

| | plan "ส่วนกลาง" | plan "รพ.สต." |
|---|---|---|
| ฐานข้อมูล | jhcisdb (host/port/user เดียวกัน) | jhcisdb |
| ปลายทาง | S3 ส่วนกลาง | โฟลเดอร์/NAS |
| รหัส zip | ของส่วนกลาง | ของ รพ.สต. |
| retention | เช่น 7/8/36/10 | เช่น 14/4/6/0 |
| เวลา | 03:30 | 20:00 |

ทำได้ด้วย model ปัจจุบันโดยไม่เปลี่ยนตาราง: 2 แถวใน `plans` ราคาคือ mysqldump รัน 2 ครั้ง (ยอมรับ ไม่ทำ dump-once-encrypt-many เพราะต้องย้ายรหัส/retention ไปไว้ที่การผูกปลายทาง) สิ่งที่ต้องมีเพิ่ม 3 อย่าง:

1. **dump ทีละ 1**: semaphore ระดับโปรแกรมขนาด 1 ครอบขั้น mysqldump (§6 ข้อ 4.5) plan ที่ชนเวลากันรอคิว ไม่ dump พร้อมกัน
2. **กันชื่อไฟล์ชน**: ห้ามสอง plan ที่มี (`prefix_name`, `database_name`) เท่ากันผูกปลายทางเดียวกัน (§10 validation) มิฉะนั้นจะเขียนทับที่ `<prefix>/<db>/<base>.zip` แล้ว prune ของอีก plan ลบทิ้ง
3. **ทำสำเนาแผน**: ปุ่มบน dashboard สร้าง plan ใหม่จากการเชื่อมต่อของแผนเดิม (§11) รหัสฐานข้อมูลคัดลอกฝั่ง server ผ่าน `copy_password_from_plan_id` เพราะ API ไม่ส่งรหัสออกมา

### รูปแบบชื่อและที่อยู่ไฟล์

- `base = {prefix_name}_{database_name}_{YYMMDDHHmm}` (เวลาท้องถิ่น) ตรงกับ v1; `prefix_name` = รหัส/ชื่อย่อหน่วยบริการ (อักษร ตัวเลข `_` `-` เท่านั้น)
- temp: `ProgramData\PowerPCU-Backup\temp\plan{id}_{base}.sql` → `.zip`
- ใน zip: entry เดียวชื่อ `{base}.sql`
- ปลายทาง local: `<path>\<prefix_name>\<database_name>\<base>.zip`
- ปลายทาง s3: key `<prefix><prefix_name>/<database_name>/<base>.zip` → location `s3://<bucket>/<key>`

## 6. ลำดับการรัน 1 ครั้ง (`backup/run.rs`)

1. ขอ lock ต่อ plan (`try_lock`) ไม่ได้ → manual ตอบ `{ok:false, message:"กำลังสำรองข้อมูลอยู่"}`, schedule/catch-up log แล้วข้าม
2. `runs` แถวใหม่ `status='running'`
3. โหลด plan + storages ที่ผูก ไม่มี storage → ปิด run `failed` "แผนนี้ยังไม่ได้เลือกที่จัดเก็บ"
4. ถอดรหัส `password`, `encryption_password` (DPAPI)
   - 4.5 ขอ **semaphore dump ระดับโปรแกรม (1 ที่)** ถ้า plan อื่นกำลัง dump อยู่ → รอ (run ยังสถานะ `running`, `message` ชั่วคราว "รอคิว dump") ปล่อยทันทีที่ขั้น 5 จบ
5. `mysqldump <db> --single-transaction --routines --events --triggers --result-file=<temp.sql> --host --port --user` รหัสผ่านผ่าน env `MYSQL_PWD` ล้มเหลว → ลบ temp, ปิด run `failed` พร้อม stderr
6. zip (Zstd level 3 + AES-256 + ZIP64) → ลบ `.sql`
7. ต่อ storage: `provider.store(zip, rel_path)` สำเร็จ → บันทึก `backup_files`; ล้มเหลว → เก็บข้อความ
8. ลบ zip temp
9. **prune** ต่อ storage ที่ข้อ 7 สำเร็จ (§7)
10. ปิด run: `ok` ทุก storage สำเร็จ / `partial` บางส่วน / `failed` ไม่สำเร็จเลย `message` = สรุปไทย เช่น `สำเร็จ 2/2 ปลายทาง` หรือ `S3 A: เชื่อมต่อไม่สำเร็จ: …`
11. ลบ `runs` เก่ากว่า 90 วันของ plan นี้

ทุกขั้นที่ล้มเหลวหลังข้อ 2 ต้องปิดแถว `runs` เสมอ (ใช้ guard/`finally`) ไม่ให้ค้าง `running`; ตอน service boot ให้ตั้งแถว `running` ที่ค้างอยู่เป็น `failed` "service หยุดระหว่างสำรอง"

## 7. Retention: keep-by-rule (`backup/prune.rs`)

ฟังก์ชัน pure ไม่แตะ IO

```rust
pub struct Keep { pub daily: u32, pub weekly: u32, pub monthly: u32, pub yearly: u32 }
pub struct Candidate { pub id: i64, pub created_at: NaiveDateTime }
pub enum Reason { Daily, Weekly, Monthly, Yearly, Latest }
pub struct Decision { pub keep: Vec<(i64, Vec<Reason>)>, pub delete: Vec<i64> }

pub fn decide(files: &[Candidate], keep: &Keep, now: NaiveDateTime) -> Decision
```

กติกา (ตาม restic `forget`)

1. เรียง `files` ใหม่→เก่า; ไฟล์ที่ `created_at > now` ถือว่าเก็บ (`Latest`) และไม่นับ
2. ไฟล์ใหม่ที่สุดเก็บเสมอ (`Latest`)
3. ต่อกฎที่ค่า > 0: เดินจากใหม่ไปเก่า จัดกลุ่มตาม bucket ของกฎ เก็บ **ไฟล์แรกที่พบ (ใหม่สุด) ของแต่ละ bucket** จนครบ n bucket ที่มีไฟล์จริง
   - daily: วันที่ (`YYYY-MM-DD`)
   - weekly: ISO week (จันทร์–อาทิตย์) `(iso_year, iso_week)`
   - monthly: `(year, month)`
   - yearly: **ปีงบประมาณ** `fy = if month >= 10 { year + 1 } else { year }` → ได้สภาพ ~30 ก.ย. ของแต่ละปี
4. ไฟล์ถูกเก็บถ้าเข้ากฎ**ข้อใดข้อหนึ่ง** (OR) `Reason` สะสมได้หลายค่า
5. ที่เหลือ → `delete`
6. ถ้า `keep` ทุกค่าเป็น 0 → เก็บทั้งหมด (กันตั้งค่าผิดแล้วลบเกลี้ยง)

การใช้งาน (`files.rs`): หลังรันสำเร็จบน storage S ของ plan P → `decide(live_rows(P,S), plan.keep, now)` → ต่อ `delete`: `provider.delete(location)` สำเร็จหรือ NotFound → `deleted_at = now`; ล้มเหลว → log ปล่อยไว้ลองใหม่รอบหน้า **ไม่ scan โฟลเดอร์/bucket** จึงไม่แตะไฟล์ของ plan อื่น, ไฟล์ของ v1, หรือไฟล์ที่ผู้ใช้วางเอง

หน้าประวัติ: เรียก `decide` ซ้ำแบบ dry-run เพื่อแสดงป้าย "เก็บในฐานะ: รายวัน / รายสัปดาห์ 36 / รายเดือน ส.ค. 69 / รายปีงบ 69" ต่อไฟล์ (API `GET /api/runs/{id}/files` คืน `kept_as: ["daily","monthly"]`)

ตัวอย่าง (keep 14/12/24/5, backup ทุกวันมา 3 ปี): เหลือประมาณ 14 + 12 + 24 + 5 − ซ้อนทับ ≈ 50 ไฟล์; ถ้าเครื่องปิดวันเสาร์-อาทิตย์ weekly = ไฟล์วันศุกร์; ถ้า backup พัง 3 สัปดาห์ ไม่มีอะไรถูกลบเพิ่ม

## 8. Scheduler + catch-up (`backup/schedule.rs`)

- `tokio-cron-scheduler` process-global (`OnceLock`) ยกจาก POC
- expression = `"0 " + schedule_cron` (6 ช่อง) ด้วย `Job::new_async_tz(expr, chrono::Local, ..)` → เวลาเครื่อง ถ้า `chrono::Local` ไม่ผ่าน trait bound ตอน implement ให้ใช้ `chrono_tz::Asia::Bangkok` (ตรวจสอบตอนเขียนโค้ด)
- boot: ลงทะเบียนทุก plan ที่ `active=1` และ `schedule_cron IS NOT NULL`; create/update/delete plan → unregister แล้ว register ใหม่
- cron ผิดรูป → log แล้วข้าม plan นั้น (UI กันไว้ก่อนแล้ว)
- overlap ต่อ plan: `try_lock` (§6 ข้อ 1)
- `next_run_at` ต่อ plan อ่านจาก `scheduler.next_tick_for_job(uuid)` ให้ `/api/status`
- **catch-up**: หลัง register ตอน boot ต่อ plan ที่ `active && catch_up && schedule_cron IS NOT NULL`: ถ้าไม่มี `runs` ที่ `status IN ('ok','partial')` และ `started_at > now - 24h` → spawn รันหลัง `120 + 30*i` วินาที (`trigger='catch_up'`)

## 9. Storage providers (`backup/storage/`)

```rust
#[async_trait]
pub trait Provider {
    async fn test(&self) -> Result<(), String>;                       // ไทย
    async fn store(&self, zip: &Path, rel: &str) -> Result<String, String>; // คืน location
    async fn delete(&self, location: &str) -> Result<(), String>;    // NotFound = Ok
}
```

- config JSON: local `{"path":"D:\\backup"}`; s3 `{"endpoint","region","bucket","access_key","secret_key","prefix","path_style"}` `secret_key` เก็บเป็น `dpapi:<base64>`
- **local**: `create_dir_all(<path>\<prefix>\<db>)` + `fs::copy`; `test()` = สร้างโฟลเดอร์ + เขียน/ลบไฟล์ probe รองรับ UNC แต่ service ต้องรันด้วยบัญชีที่เข้า share ได้ (ตั้ง `nssm set PowerPCU-Backup ObjectName …`) เขียนไว้ใน README
- **s3**: `aws-sdk-s3` + `aws-smithy-http-client` rustls/ring (ยกจาก POC ทั้งไฟล์) จุดสำคัญที่ห้ามตัด: `RequestChecksumCalculation::WhenRequired` + `ResponseChecksumValidation::WhenRequired` (MinIO/Ceph/R2 ไม่รับ aws-chunked), `force_path_style` ตาม config, region ว่าง = `us-east-1`, ใช้ SDK ทางการไม่ใช่ `rust-s3` (RadosGW ปฏิเสธ DELETE ของ rust-s3); `test()` = `list_objects_v2(max_keys=1)`; `store` = `put_object` จาก `ByteStream::from_path`

## 10. API (axum, `127.0.0.1:8720`)

กติกาเดียวกับ POC: 200 เสมอสำหรับผลเชิงธุรกิจ `{ok:bool, message:string}`; 4xx/5xx เฉพาะ JSON ผิด/DB พัง; ข้อความไทย

| Method/Path | Body → Response |
|---|---|
| `GET /api/status` | `{version, started_at, plans:[{id,name,active,running,next_run_at,last_run:{id,status,started_at,finished_at,message,size_bytes}\|null}]}` |
| `GET /api/storages` | `[{id,name,provider,config(masked),in_use:bool}]` |
| `POST /api/storages` | `{name,provider,config}` → `{ok,id}` |
| `PUT /api/storages/{id}` | เหมือน POST; `secret_key` ว่าง = คงเดิม |
| `DELETE /api/storages/{id}` | `{ok,message}` (บล็อกถ้าใช้อยู่) |
| `POST /api/storages/{id}/test` | `{ok,message}` ใช้ค่าที่บันทึกไว้ |
| `POST /api/storages/test` | `{provider,config,storage_id?}` → `{ok,message}` ทดสอบก่อนบันทึก (secret ว่าง + `storage_id` = ใช้ของเดิม) |
| `GET /api/plans` | `[{…plan (password/encryption_password masked ""), storage_ids:[..]}]` |
| `POST /api/plans` | `{name,database_name,host,port,username,password,prefix_name,encryption_password,schedule_cron,active,catch_up,keep_daily,keep_weekly,keep_monthly,keep_yearly,storage_ids,copy_password_from_plan_id?}` → `{ok,id}` ต้องมี storage ≥1, รหัส zip ≥ 8 ตัว; `password` ว่าง + `copy_password_from_plan_id` = คัดลอก blob รหัสฐานข้อมูลจากแผนนั้นฝั่ง server |
| `PUT /api/plans/{id}` | เหมือน POST; รหัสว่าง = คงเดิม; แทนที่ `plan_storages`; re-register scheduler |
| (validation POST/PUT) | ห้ามมี plan อื่นที่ (`prefix_name`, `database_name`) เท่ากันและมี `storage_id` ร่วมกัน → `{ok:false, message:"แผน '<ชื่อ>' ใช้รหัสหน่วยบริการ+ฐานข้อมูลเดียวกันบนปลายทาง '<ชื่อ>' อยู่แล้ว เปลี่ยนรหัสหน่วยบริการหรือปลายทาง"}` |
| `DELETE /api/plans/{id}` | `{ok}` |
| `POST /api/plans/{id}/run` | `{ok,message}` (คืนทันที รันเบื้องหลัง) |
| `POST /api/plans/test-connection` | `{host,port,username,password,database_name,plan_id?}` → `{ok,message}` ใช้ `mysqldump --no-data --skip-triggers --skip-routines --skip-events --result-file=<temp>` แล้วลบ (ทดสอบทั้ง binary และสิทธิ์จริง) |
| `GET /api/plans/{id}/runs?limit=50` | `[{id,trigger,status,started_at,finished_at,message,size_bytes}]` |
| `GET /api/runs/{id}/files` | `[{id,storage_id,storage_name,provider,location,size_bytes,created_at,deleted_at,kept_as:[..]}]` |

- middleware: ทุก `/api/*` ต้องมี header `X-PowerPCU-Client: backup-ui` ไม่มี → 403 (กันเว็บอื่นยิง localhost) หน้าเว็บใส่ header นี้ใน `fetch` wrapper เดียว
- `/` และไฟล์อื่น: rust-embed จาก `web/` (`Cache-Control: no-cache` สำหรับ `index.html`)
- ผูก `127.0.0.1` เท่านั้น

## 11. หน้าเว็บ (`web/`, Alpine.js)

- `index.html` ไฟล์เดียว, `app.js` (Alpine `x-data` หลัก + hash-router ง่าย ๆ `#/`, `#/storages`, `#/plans/new`, `#/plans/:id`, `#/history/:planId`), `style.css` เขียนเอง (ฟอนต์ระบบ Segoe UI/Tahoma อ่านไทยได้), `vendor/alpine.min.js` (3.x) ไม่มี CDN เพราะเครื่องอาจไม่มีเน็ต
- `api()` wrapper เดียว: ใส่ header, แปลง JSON, โยน error ไทย

หน้า

1. **ตั้งค่าครั้งแรก** (แสดงเมื่อ `plans` ว่าง) 3 ขั้นในหน้าเดียว
   1. ฐานข้อมูล: default `127.0.0.1` / `3333` / `root` / `jhcisdb` กรอกรหัส → ปุ่ม "ทดสอบการเชื่อมต่อ" (ต้องผ่านก่อนไปต่อ)
   2. ปลายทาง: เพิ่มโฟลเดอร์ (พิมพ์ path, default `C:\PowerPCU-Backup`) และ/หรือ S3 ทดสอบได้ ต้องมี ≥1
   3. เวลาและรหัส: default "ทุกวัน 03:30", รหัสเข้ารหัส zip (บังคับ ≥8 ตัว, ช่องยืนยัน, เตือนว่าลืมแล้วเปิดไฟล์ไม่ได้ ให้จดไว้), `prefix_name` = "รหัส/ชื่อย่อหน่วยบริการ" บังคับกรอก ไม่มี default (เช่น `10999`) เพราะเป็นโฟลเดอร์แรกในปลายทางที่หลายหน่วยใช้ bucket ร่วมกัน (ความหมายเดียวกับ `PREFIX_NAME` ของ v1)
   → สร้าง storages + plan → ถาม "สำรองเลยตอนนี้?"
2. **Dashboard** (`#/`): การ์ดต่อ plan: ชื่อ, สถานะล่าสุด (สี), เวลาล่าสุด, รอบถัดไป, ปุ่ม "รันตอนนี้" / "แก้ไข" / "ประวัติ" / "ทำสำเนาแผน"; ปุ่ม "สร้างแผนใหม่"; แถบล่างสุด version service; poll `/api/status` ทุก 5 วิเมื่อมี `running` ไม่งั้น 30 วิ
   - **ทำสำเนาแผน** → `#/plans/new?from=<id>`: เติม host/port/username/database/prefix จากแผนเดิม ช่องรหัสฐานข้อมูลว่างพร้อมข้อความ "ใช้รหัสเดียวกับแผน '<ชื่อ>'" (ส่ง `copy_password_from_plan_id`); ชื่อแผน, ปลายทาง, รหัส zip, retention, เวลา ให้กรอกใหม่ (ปลายทางไม่ติ๊กไว้ก่อน เพื่อไม่ให้ชน validation §10)
3. **ปลายทาง** (`#/storages`): ตาราง + ฟอร์ม inline (local: path; s3: endpoint, region, bucket, access key, secret, prefix, path-style) + ทดสอบ + ลบ (แสดงเหตุผลถ้าลบไม่ได้)
4. **แผน** (`#/plans/new`, `#/plans/:id`): ข้อมูลฐาน (+ทดสอบ), ปลายทาง (checkbox หลายอัน), ตารางเวลา (ดู builder ด้านล่าง), retention: ช่องเดียว "เก็บสำเนาล่าสุด 14 ชุด" + พับ "ขั้นสูง": รายสัปดาห์ 12 / รายเดือน 24 / รายปีงบ 5 + `catch_up` + `active`
5. **ประวัติ** (`#/history/:planId`): ตาราง runs (สถานะ, trigger, เวลา, ระยะเวลา, ขนาด, ข้อความ) คลิกดูไฟล์: ปลายทาง, ที่อยู่, ขนาด, ป้าย "เก็บในฐานะ…", ลบแล้ว/ยังอยู่

### Schedule builder ↔ cron

| UI | cron (เวลาท้องถิ่น) |
|---|---|
| ทุกวัน เวลา HH:MM | `M H * * *` |
| ทุกสัปดาห์ วัน D (อา=0..ส=6) เวลา | `M H * * D` |
| ทุกเดือน วันที่ N (1–28) เวลา | `M H N * *` |
| ขั้นสูง | พิมพ์ 5 ช่องเอง |
| ไม่ตั้งเวลา (manual) | `null` |

โหลดแผนกลับ: parse cron เป็นแบบง่ายถ้าตรง 3 รูปแบบข้างบน ไม่ตรง → โหมดขั้นสูง ตัว parse/format เป็นฟังก์ชัน JS 2 ตัว มี self-check ใน console (`app.js` ท้ายไฟล์ `if (location.hash === '#selftest')`)

## 12. Tauri shell (`src/gui/`)

- plugins: `tauri-plugin-single-instance` (เปิดซ้ำ → focus หน้าต่างเดิม), `tauri-plugin-autostart` (GUI ขึ้นตอน login), `tauri-plugin-window-state`; tray ใช้ feature `tray-icon` ของ tauri
- boot: probe `GET /api/status` ด้วย `ureq` (timeout 1 วิ) ตอบ → สร้าง `main` window `WebviewUrl::External("http://127.0.0.1:8720/")` ขนาด 1100×720 ไม่ตอบ → `splash` window จาก `splash/index.html` แสดง "service ยังไม่ทำงาน" + ปุ่ม "เริ่ม service" (Tauri command → `ShellExecuteW("runas", "sc", "start PowerPCU-Backup")`) + retry ทุก 2 วิ ตอบเมื่อไรสลับเป็น main
- ปิดหน้าต่าง = ซ่อน (`api.prevent_close()` + `hide`); เปิดจาก tray = show + focus
- tray: poll `/api/status` ทุก 60 วิ (และทันทีเมื่อเปิดหน้าต่าง) → icon: `ok` (ทุก plan ล่าสุด ok/ไม่มี plan), `running`, `warn` (มี partial), `error` (มี failed หรือ service ไม่ตอบ) tooltip เช่น "PowerPCU Backup: สำรองล่าสุด 07/09/69 03:31 สำเร็จ" เมนู: เปิดหน้าต่าง / ออกจากโปรแกรม (GUI เท่านั้น)
- ไม่มี Tauri command จาก origin ของ service (ไม่ต้องตั้ง remote capability) หน้าเว็บทำงานเหมือนเปิดใน browser ทุกประการ
- WebView2 หาย → dialog ไทยเหมือน POC แล้วออก

## 13. ความปลอดภัย

- **Secret at rest** (`secret.rs`): `CryptProtectData`/`CryptUnprotectData` flag `CRYPTPROTECT_LOCAL_MACHINE` (crate `windows-sys`, feature `Win32_Security_Cryptography`) เก็บเป็น `dpapi:<base64>` ใช้กับ `plans.password`, `plans.encryption_password`, `storages.config.secret_key` ถอดรหัสเฉพาะใน service ตอนใช้งาน ไฟล์ DB ที่ถูกคัดลอกไปเครื่องอื่นเปิด secret ไม่ได้
- API ไม่คืน secret (mask `""`); PUT ค่าว่าง = คงเดิม
- API ผูก `127.0.0.1` + header guard; **ไม่มี login** (เครื่องเดี่ยว เหมือน v1 ที่ `.env` อ่านได้ทั้งเครื่อง) จดเป็นรอบถัดไป: PIN
- รหัส MySQL ผ่าน env `MYSQL_PWD` ไม่ผ่าน command line
- รหัส zip บังคับ ≥8 ตัว; ไฟล์ที่ปลายทางเป็น AES-256 เสมอ
- log ไม่พิมพ์ secret; `message` ของ run ต้องกรอง `MYSQL_PWD`/รหัสออกจาก stderr ของ mysqldump (mysqldump ไม่พิมพ์รหัสอยู่แล้ว แต่ให้ redact string ที่ตรงกับรหัสเผื่อไว้)

## 14. Error handling & logging

- scheduler ไม่ล้มเพราะ plan เดียว; service ไม่ล้มเพราะ scheduler
- retention: ลบไม่สำเร็จ = log + ลองรอบหน้า ไม่กระทบผลรัน
- ทุก run ปิดสถานะเสมอ (§6)
- `tracing` + `tracing-subscriber` fmt (เวลาท้องถิ่น, ไม่มีสี) → stdout ให้ NSSM เขียน `logs\service.log` หมุนที่ 10 MB
- GUI: error สำคัญเป็น dialog ไทย ที่เหลือ `eprintln`

## 15. Build, ติดตั้ง, version

- `cargo tauri build` → `src-tauri/target/release/powerpcu-backup.exe` (bundler ปิด)
- `installer/build.ps1`: อ่าน version จาก `src-tauri/Cargo.toml` → `cargo tauri build` → `iscc /DAppVersion=<v> installer\setup.iss` → `installer\Output\powerpcu-backup-setup-<v>.exe`
- `installer/setup.iss` (ปรับจาก POC `installer/setup.iss`)
  - ติดตั้งที่ `{autopf}\PowerPCU-Backup` (exe, `bin\mysqldump.exe`, `bin\nssm.exe`); สร้าง `{commonappdata}\PowerPCU-Backup\{logs,temp}`
  - [Tasks] `removev1` "หยุดและถอน service ของเวอร์ชันเก่า (power_pcu_backup)" default ติ๊ก; `autostart` "เปิด PowerPCU Backup ตอน login" default ติ๊ก
  - [Run] ก่อนติดตั้ง: `sc stop PowerPCU-Backup` (อัปเกรด); หลังคัดลอก: `nssm install PowerPCU-Backup "{app}\powerpcu-backup.exe" --service`, `nssm set … AppDirectory {commonappdata}\PowerPCU-Backup`, `AppStdout/AppStderr {commonappdata}\PowerPCU-Backup\logs\service.log`, `AppRotateFiles 1`, `AppRotateOnline 1`, `AppRotateBytes 10485760`, `Start SERVICE_AUTO_START`, `nssm start PowerPCU-Backup`; ถ้า `removev1`: `sc stop power_pcu_backup` + `sc delete power_pcu_backup`
  - WebView2: ถ้า registry `HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}\pv` ไม่มี → รัน `vendor\MicrosoftEdgeWebview2Setup.exe /silent /install`
  - หลังติดตั้ง: เปิด GUI
  - uninstall: `nssm stop` + `nssm remove PowerPCU-Backup confirm`, ถามลบ `{commonappdata}\PowerPCU-Backup`
- version จุดเดียว: `Cargo.toml` (Tauri อ่านเอง; `/api/status` คืน `env!("CARGO_PKG_VERSION")`)
- ไม่ code-sign รอบนี้ (SmartScreen เตือน ยอมรับได้ เหมือน POC)

## 16. การทดสอบ

Rust (`cargo test` บน Windows)

- `archive`: zip → เปิดคืนด้วยรหัสได้, รหัสผิดเปิดไม่ได้, entry ชื่อ `{base}.sql`
- `run`: รูปแบบ `base` จากวันที่กำหนด
- `prune::decide`: (1) ชุดวันละไฟล์ 3 ปี keep 14/12/24/5 → จำนวนและเหตุผลถูก, (2) ข้ามเสาร์-อาทิตย์ → weekly = ศุกร์, (3) ไฟล์ล่าสุดอยู่เสมอ, (4) keep ทั้งหมด 0 → ไม่ลบ, (5) ปีงบ: ไฟล์ 30 ก.ย. และ 1 ต.ค. อยู่คนละปี, (6) หลายไฟล์ในวันเดียวเก็บใหม่สุด
- `files`: prune ของ plan A ไม่แตะแถวของ plan B บน storage เดียวกัน; delete NotFound = ตั้ง `deleted_at`
- `schedule`: `"0 " + cron` mapping; cron ผิด → skip; catch-up เงื่อนไข 24 ชม.
- `run`: 2 plan รันพร้อมกันกับ mysqldump ปลอมที่ sleep → ช่วง dump ไม่ทับกัน (semaphore)
- `plans`: validation คู่ (prefix, db, storage) ซ้ำข้ามแผน → `ok:false`; `copy_password_from_plan_id` คัดลอก blob ได้และถอดรหัสแล้วตรงกัน
- `storages`: mask/คงค่า secret; ลบถูกบล็อกเมื่อใช้อยู่
- `secret`: DPAPI round-trip; ค่า `dpapi:` เสีย → error ไทย
- `storage::local`: store แล้ว delete บน temp dir
- `api`: guard header 403; `test-connection` กับ `mysqldump` ปลอม (env `MYSQLDUMP_PATH` ชี้ไป script) 
- integration (ข้ามถ้าไม่มี env `TEST_MYSQL_URL`): รันจริงกับ MySQL local → `runs.ok`, ไฟล์อยู่, เปิด zip ได้
- S3: ทดสอบมือกับ MinIO (docker) ตาม README (ตามธรรมเนียม POC)

หน้าเว็บ (ไม่มี test runner): `#selftest` รัน assert ของ cron builder/parser ใน console; checklist ทดสอบมือ: wizard ครบ 3 ขั้น, สร้าง/แก้/ลบ storage และ plan, รันตอนนี้แล้ว dashboard เปลี่ยนสถานะ, ประวัติแสดงป้าย kept_as, ลบ storage ที่ใช้อยู่ถูกบล็อก, ปิดหน้าต่างแล้ว tray ยังอยู่, service หยุดแล้ว splash โผล่และปุ่มเริ่ม service ทำงาน

## 17. ค่าเริ่มต้นที่ได้จากการศึกษา MySQL Administrator

- JHCIS: host `127.0.0.1`, port `3333`, user `root`, db `jhcisdb`
- MySQL Administrator ตั้ง schedule ผ่าน Windows Task Scheduler ของ user → ไม่รันถ้าไม่ login/รหัสเปลี่ยน และไม่มี log v2 แก้ด้วย service + `runs` + tray
- ผู้ใช้คุ้นกับ "Complete backup" = dump ทั้ง schema (routines/events/triggers) ตรงกับ flags ใน §6
- งานที่ผู้ใช้เคยทำแล้ว v2 รอบนี้ยังไม่มี: Restore (Open Backup File → Ignore Errors + Create database if not exists → Start Restore) และ backup ครั้งเดียวไปไฟล์ที่เลือก

## 18. Dependencies (Cargo)

`tauri 2` (features `tray-icon`), `tauri-plugin-single-instance`, `tauri-plugin-autostart`, `tauri-plugin-window-state`, `tokio` (rt-multi-thread, macros, process, sync, fs, time), `axum`, `tower-http` (set-header), `rust-embed`, `sqlx` (runtime-tokio, sqlite, migrate, chrono), `serde`, `serde_json`, `chrono` (clock), `tokio-cron-scheduler`, `uuid` (v4), `zip 2` (zstd, aes-crypto, deflate), `aws-sdk-s3` + `aws-smithy-http-client` (rustls-ring), `async-trait`, `windows-sys` (Win32_Security_Cryptography, Win32_UI_Shell), `ureq`, `base64`, `tracing`, `tracing-subscriber` กติกาเดียวกับ POC: ไม่มี openssl/aws-lc-rs/NASM

## 19. รอบถัดไป (ไม่ทำรอบนี้)

- Restore ในโปรแกรม (ต้อง bundle `mysql.exe` 5.6)
- backup ครั้งเดียวไปโฟลเดอร์ที่เลือก / ไฟล์ที่ "pin" ไม่ให้ prune (เช่น ก่อน upgrade JHCIS)
- แจ้งเตือนภายนอก (MOPH Alert แบบ POC / webhook) hook ที่ปลาย `run.rs` หลังปิด run
- PIN หน้าเว็บ
- retention ต่อปลายทาง (local เก็บน้อย S3 เก็บมาก)
- auto-update
- Windows 8 (nightly `x86_64-win7-windows-msvc`)
- ตั้งพอร์ตจาก UI

## 20. อ้างอิง

- POC: `C:\Users\usman\repo\powerpcu_poc` specs `docs/superpowers/specs/2026-06-0{4,6,8}-backup-*.md`, Rust source ที่ `f45ee96^:user-api/src/backup/`, Tauri launcher `launcher/src/main.rs`, installer `installer/setup.iss`
- restic forget policy: https://restic.readthedocs.io/en/stable/060_forget.html
- MySQL Administrator schedule: http://download.nust.na/pub6/mysql/doc/administrator/en/mysql-administrator-backup-schedule.html ; JHCIS backup/restore: https://7-sadas.gitbook.io/jhcis/backup-restored ; Windows scheduler quirk: https://kedar.nitty-witty.com/blog/scheduled-backup-mysql-administrator-windows-scheduler-odd
- Rust Windows targets: `x86_64-pc-windows-msvc` ต้องการ Windows 10+; Win7/8 = tier-3 `*-win7-windows-msvc`
- Tauri 2 Windows installer/WebView2: https://tauri.app/distribute/windows-installer/
