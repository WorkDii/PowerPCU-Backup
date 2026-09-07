//! S3-compatible storage provider (AWS S3, MinIO, Wasabi, Ceph RadosGW, …) via the
//! official **aws-sdk-s3**. The client is built by hand from the stored config
//! (endpoint, region, path-style, static credentials) — no environment/profile
//! lookup. Pinned to rustls + ring (no openssl, no aws-lc-rs) to match the tree.
//!
//! Why the official SDK and not `rust-s3`: rust-s3's header-signed `delete_object`
//! signs `content-type`/`content-length` on the DELETE, which Ceph RadosGW rejects
//! as `403 SignatureDoesNotMatch` (PUT worked, DELETE didn't — verified against the
//! live server). The AWS SDK signs SigV4 correctly across S3-compatible servers.

use std::path::Path;

use aws_sdk_s3::config::{
    BehaviorVersion, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use aws_smithy_http_client::tls::{rustls_provider::CryptoMode, Provider};

use super::S3Config;

pub struct S3Provider {
    pub cfg: S3Config,
}

impl S3Provider {
    /// Build an S3 client from the stored config: static credentials, explicit
    /// endpoint (trailing slash trimmed), optional path-style, and a rustls+ring
    /// HTTPS connector so no openssl/aws-lc-rs is pulled in. Region only feeds the
    /// SigV4 scope; S3-compatible servers ignore it — default `us-east-1` when blank.
    fn client(&self) -> Client {
        let region = match self.cfg.region.trim() {
            "" => "us-east-1".to_string(),
            r => r.to_string(),
        };
        let creds = Credentials::new(
            self.cfg.access_key.clone(),
            self.cfg.secret_key.clone(),
            None,
            None,
            "powerpcu",
        );
        let http = aws_smithy_http_client::Builder::new()
            .tls_provider(Provider::Rustls(CryptoMode::Ring))
            .build_https();
        let conf = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region))
            .endpoint_url(self.cfg.endpoint.trim_end_matches('/').to_string())
            .credentials_provider(creds)
            .force_path_style(self.cfg.path_style)
            // aws-sdk-s3 1.x adds a default CRC32 checksum + `aws-chunked` streaming
            // trailer, which S3-compatible servers (Ceph RadosGW, MinIO, R2 …) reject
            // (XAmzContentSHA256Mismatch). Only checksum when the operation requires
            // it so uploads use a plain signed payload these servers accept.
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            .http_client(http)
            .build();
        Client::from_conf(conf)
    }

    fn key(&self, file_name: &str) -> String {
        format!("{}{}", self.cfg.prefix, file_name)
    }

    /// `s3://bucket/key` location string from a stored key.
    fn location(&self, key: &str) -> String {
        format!("s3://{}/{}", self.cfg.bucket, key)
    }

    /// Recover the object key from a stored `s3://bucket/key` location.
    fn key_from_location(&self, location: &str) -> String {
        let p = format!("s3://{}/", self.cfg.bucket);
        location.strip_prefix(&p).unwrap_or(location).to_string()
    }

    /// Stream the local `.zip` up to `prefix + rel`; returns the location.
    /// `ByteStream::from_path` streams from disk (sets Content-Length from the file
    /// size), so multi-MB dumps never load into memory.
    pub async fn store(&self, zip: &Path, rel: &str) -> Result<String, String> {
        let key = self.key(rel);
        let body = ByteStream::from_path(zip)
            .await
            .map_err(|e| format!("เปิดไฟล์เพื่ออัปโหลดไม่สำเร็จ: {e}"))?;
        self.client()
            .put_object()
            .bucket(&self.cfg.bucket)
            .key(&key)
            .body(body)
            .send()
            .await
            .map_err(|e| format!("อัปโหลดขึ้น S3 ไม่สำเร็จ: {}", sdk_err(e)))?;
        Ok(self.location(&key))
    }

    /// Delete one stored copy by its recorded `s3://bucket/key` location. The SDK
    /// returns `Err` on any non-2xx, so a rejected delete is a real error (it won't
    /// be falsely marked deleted). A missing key is success on S3 (idempotent).
    pub async fn delete(&self, location: &str) -> Result<(), String> {
        let key = self.key_from_location(location);
        self.client()
            .delete_object()
            .bucket(&self.cfg.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| format!("ลบไฟล์บน S3 ไม่สำเร็จ: {}", sdk_err(e)))?;
        Ok(())
    }

    /// Connection test: write then delete a tiny probe object (checks creds,
    /// bucket, and write/delete permission).
    pub async fn test(&self) -> Result<(), String> {
        let client = self.client();
        let probe = self.key(".powerpcu_probe");
        client
            .put_object()
            .bucket(&self.cfg.bucket)
            .key(&probe)
            .body(ByteStream::from_static(b"ok"))
            .send()
            .await
            .map_err(|e| format!("ทดสอบเขียน S3 ไม่สำเร็จ: {}", sdk_err(e)))?;
        // Best-effort cleanup — a leaked probe must not fail the connection test.
        let _ = client
            .delete_object()
            .bucket(&self.cfg.bucket)
            .key(&probe)
            .send()
            .await;
        Ok(())
    }
}

/// Flatten an aws-sdk error to its most specific message. `SdkError`'s own Display
/// is terse ("service error" / "dispatch failure"); the real detail (the S3
/// `<Message>`, a signature/credential error, a DNS failure) lives in the source
/// chain, so walk to the deepest source.
fn sdk_err<E, R>(e: SdkError<E, R>) -> String
where
    E: std::error::Error + 'static,
    R: std::fmt::Debug,
{
    use std::error::Error;
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        msg = s.to_string();
        src = s.source();
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_round_trips_through_location() {
        let p = S3Provider {
            cfg: S3Config {
                endpoint: "https://e".into(),
                region: String::new(),
                bucket: "test".into(),
                access_key: "AK".into(),
                secret_key: "SK".into(),
                prefix: "inspace_cloud/".into(),
                path_style: true,
            },
        };
        let key = p.key("jhcis_db_2601010000.sql.zip");
        assert_eq!(key, "inspace_cloud/jhcis_db_2601010000.sql.zip");
        let loc = p.location(&key);
        assert_eq!(loc, "s3://test/inspace_cloud/jhcis_db_2601010000.sql.zip");
        assert_eq!(p.key_from_location(&loc), key, "delete must recover the exact key");
    }
}
