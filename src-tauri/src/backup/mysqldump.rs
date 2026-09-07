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
