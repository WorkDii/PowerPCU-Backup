//! Storage providers: reusable backup destinations (a local folder or an
//! S3-compatible bucket). A [`Storage`] is a DB-backed config; a [`Provider`] is
//! the runtime object that writes/deletes a file. Build one with [`for_storage`].
//!
//! Secrets live in `config` JSON (plaintext, single-tenant) but never leave the
//! API: [`StorageConfig::to_public_value`] blanks them for responses.

mod local;
mod s3;

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use local::LocalProvider;
use self::s3::S3Provider;

/// Provider-specific settings, parsed from a storage row's `provider` + `config`.
#[derive(Clone)]
pub enum StorageConfig {
    Local(LocalConfig),
    S3(S3Config),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    pub path: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct S3Config {
    pub endpoint: String,
    /// Optional: only AWS S3 proper needs a real region. S3-compatible servers
    /// ignore it; defaults to `us-east-1` at connection time when left blank.
    #[serde(default)]
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    #[serde(default)]
    pub secret_key: String,
    #[serde(default)]
    pub prefix: String,
    /// Path-style URLs (required by MinIO and most S3-compatible servers).
    #[serde(default = "default_true")]
    pub path_style: bool,
}

fn default_true() -> bool {
    true
}

impl StorageConfig {
    /// Parse from the stored `provider` tag + `config` JSON.
    pub fn parse(provider: &str, config_json: &str) -> Result<Self, String> {
        match provider {
            "local" => serde_json::from_str::<LocalConfig>(config_json)
                .map(StorageConfig::Local)
                .map_err(|e| format!("config local ไม่ถูกต้อง: {e}")),
            "s3" => serde_json::from_str::<S3Config>(config_json)
                .map(StorageConfig::S3)
                .map_err(|e| format!("config s3 ไม่ถูกต้อง: {e}")),
            other => Err(format!("ไม่รู้จักชนิดที่จัดเก็บ: {other}")),
        }
    }

    pub fn provider_str(&self) -> &'static str {
        match self {
            StorageConfig::Local(_) => "local",
            StorageConfig::S3(_) => "s3",
        }
    }

    /// JSON to persist (secret included).
    pub fn to_json(&self) -> String {
        match self {
            StorageConfig::Local(c) => serde_json::to_string(c).unwrap_or_default(),
            StorageConfig::S3(c) => serde_json::to_string(c).unwrap_or_default(),
        }
    }

    /// Masked JSON for API responses (secret blanked).
    pub fn to_public_value(&self) -> Value {
        match self {
            StorageConfig::Local(c) => serde_json::to_value(c).unwrap_or(Value::Null),
            StorageConfig::S3(c) => {
                let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
                if let Some(o) = v.as_object_mut() {
                    o.insert("secret_key".into(), Value::String(String::new()));
                }
                v
            }
        }
    }

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
}

/// A storage row with its parsed config.
pub struct Storage {
    pub id: i64,
    pub name: String,
    pub config: StorageConfig,
}

impl Storage {
    /// `{ id, name, provider, config }` with the secret blanked — for API responses.
    pub fn to_public_value(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "provider": self.config.provider_str(),
            "config": self.config.to_public_value(),
        })
    }
}

/// Runtime provider that writes/deletes one backup copy. Enum dispatch (not a
/// `dyn` trait) keeps us off the `async-trait` dependency.
pub enum Provider {
    Local(LocalProvider),
    S3(S3Provider),
}

impl Provider {
    pub async fn store(&self, zip: &Path, rel: &str) -> Result<String, String> {
        match self {
            Provider::Local(p) => p.store(zip, rel).await,
            Provider::S3(p) => p.store(zip, rel).await,
        }
    }

    pub async fn delete(&self, location: &str) -> Result<(), String> {
        match self {
            Provider::Local(p) => p.delete(location).await,
            Provider::S3(p) => p.delete(location).await,
        }
    }

    pub async fn test(&self) -> Result<(), String> {
        match self {
            Provider::Local(p) => p.test().await,
            Provider::S3(p) => p.test().await,
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_local_round_trips() {
        let cfg = StorageConfig::parse("local", r#"{"path":"D:\\backups"}"#).unwrap();
        assert_eq!(cfg.provider_str(), "local");
        assert_eq!(cfg.to_public_value()["path"], "D:\\backups");
    }

    #[test]
    fn parse_s3_defaults_path_style_true() {
        let json = r#"{"endpoint":"http://localhost:9000","region":"us-east-1","bucket":"b","access_key":"AK","secret_key":"SK"}"#;
        let cfg = StorageConfig::parse("s3", json).unwrap();
        assert_eq!(cfg.provider_str(), "s3");
        // path_style defaults true; prefix defaults empty.
        let pub_v = cfg.to_public_value();
        assert_eq!(pub_v["path_style"], true);
        assert_eq!(pub_v["prefix"], "");
    }

    #[test]
    fn public_value_blanks_s3_secret_but_keeps_access_key() {
        let json = r#"{"endpoint":"e","region":"r","bucket":"b","access_key":"AK","secret_key":"SECRET","prefix":"p/"}"#;
        let cfg = StorageConfig::parse("s3", json).unwrap();
        let v = cfg.to_public_value();
        assert_eq!(v["access_key"], "AK");
        assert_eq!(v["secret_key"], "", "secret must be blanked on read");
        // to_json keeps the secret for persistence.
        assert!(cfg.to_json().contains("SECRET"));
    }

    #[test]
    fn parse_unknown_provider_errors() {
        assert!(StorageConfig::parse("ftp", "{}").is_err());
    }

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
}
