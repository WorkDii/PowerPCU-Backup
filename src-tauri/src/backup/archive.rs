//! Compress a dump into an AES-256 zip (Zstd method). Opens in 7-Zip 21+/WinRAR 6+,
//! not in Windows Explorer (which reads neither AES nor Zstd). Ported from the POC.

use std::fs::File;
use std::path::{Path, PathBuf};

use zip::{write::SimpleFileOptions, AesMode, CompressionMethod, ZipWriter};

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

    #[test]
    fn failure_removes_partial_zip() {
        let d = temp("missing");
        let sql = d.join("missing.sql");
        let zip_path = d.join("missing.zip");
        assert!(compress_to_zip(&sql, "missing.sql", "pw").is_err());
        assert!(!zip_path.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
