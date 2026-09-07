//! Local-folder storage provider: copy the temp `.zip` into a destination folder,
//! and delete a copy by its absolute path during retention. Fully async via
//! `tokio::fs` so a large copy never blocks the runtime.

use std::path::Path;

pub struct LocalProvider {
    /// Destination folder. Empty when only deleting by recorded path (retention).
    pub dir: String,
}

impl LocalProvider {
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

    /// Remove a stored copy. A file already gone is treated as success.
    pub async fn delete(&self, location: &str) -> Result<(), String> {
        match tokio::fs::remove_file(location).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("ลบไฟล์ไม่สำเร็จ: {e}")),
        }
    }

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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(tag: &str) -> std::path::PathBuf {
        // No Date::now in this env; use the test's own name + a counter file-free
        // uniqueness via process id + tag.
        std::env::temp_dir().join(format!("powerpcu-local-{}-{}", std::process::id(), tag))
    }

    #[tokio::test]
    async fn store_then_delete() {
        let src_dir = unique_dir("src");
        let dst_dir = unique_dir("dst");
        tokio::fs::create_dir_all(&src_dir).await.unwrap();
        let src = src_dir.join("backup.zip");
        tokio::fs::write(&src, b"zipbytes").await.unwrap();

        let p = LocalProvider {
            dir: dst_dir.to_string_lossy().into_owned(),
        };
        let location = p.store(&src, "10999/jhcisdb/backup.zip").await.unwrap();
        assert!(Path::new(&location).exists(), "copied file should exist");
        assert_eq!(tokio::fs::read(&location).await.unwrap(), b"zipbytes");
        assert!(
            location.ends_with(r"10999\jhcisdb\backup.zip"),
            "location should preserve nested path: {location}"
        );

        p.delete(&location).await.unwrap();
        assert!(!Path::new(&location).exists(), "delete should remove it");
        // deleting again is a no-op success
        p.delete(&location).await.unwrap();

        let _ = tokio::fs::remove_dir_all(&src_dir).await;
        let _ = tokio::fs::remove_dir_all(&dst_dir).await;
    }
}
