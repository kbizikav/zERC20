use std::path::Path;

use anyhow::{Context, Result};
use aws_sdk_s3::{
    primitives::ByteStream,
    types::{CompletedMultipartUpload, CompletedPart},
    Client as S3Client,
};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

/// Multipart upload threshold: 1GB
const MULTIPART_THRESHOLD: usize = 1024 * 1024 * 1024;
/// Part size for multipart upload: 1GB
const PART_SIZE: usize = 1024 * 1024 * 1024;

/// S3 storage client wrapper.
#[derive(Clone)]
pub struct Storage {
    client: S3Client,
    bucket: String,
    prefix: String,
}

impl Storage {
    /// Create a new S3 storage client.
    pub fn new(client: S3Client, bucket: String, prefix: String) -> Self {
        let prefix = prefix.trim_matches('/').to_string();
        Self {
            client,
            bucket,
            prefix,
        }
    }

    /// Build the full S3 key from a suffix.
    fn key(&self, suffix: &str) -> String {
        if self.prefix.is_empty() {
            suffix.to_string()
        } else {
            format!("{}/{}", self.prefix, suffix)
        }
    }

    /// Upload a file to S3, using multipart upload for large files (>1GB).
    pub async fn upload_file(&self, s3_key: &str, local_path: &Path) -> Result<()> {
        let metadata = tokio::fs::metadata(local_path).await
            .with_context(|| format!("failed to get metadata for {}", local_path.display()))?;
        let file_size = metadata.len() as usize;

        if file_size < MULTIPART_THRESHOLD {
            self.upload_file_simple(s3_key, local_path).await
        } else {
            self.upload_file_multipart(s3_key, local_path, file_size).await
        }
    }

    /// Simple upload for small files.
    async fn upload_file_simple(&self, s3_key: &str, local_path: &Path) -> Result<()> {
        let full_key = self.key(s3_key);
        let body = ByteStream::from_path(local_path).await
            .with_context(|| format!("failed to read file {}", local_path.display()))?;

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .content_type("application/octet-stream")
            .body(body)
            .send()
            .await
            .with_context(|| format!("failed to upload {} to s3://{}/{}", local_path.display(), self.bucket, full_key))?;

        Ok(())
    }

    /// Multipart upload for large files.
    async fn upload_file_multipart(&self, s3_key: &str, local_path: &Path, file_size: usize) -> Result<()> {
        let full_key = self.key(s3_key);
        let num_parts = file_size.div_ceil(PART_SIZE);

        log::info!(
            "Starting multipart upload: {} ({} bytes, {} parts)",
            full_key,
            file_size,
            num_parts
        );

        // Create multipart upload
        let create_resp = self.client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(&full_key)
            .content_type("application/octet-stream")
            .send()
            .await
            .context("failed to create multipart upload")?;

        let upload_id = create_resp
            .upload_id()
            .context("no upload_id in response")?;

        let mut completed_parts: Vec<CompletedPart> = Vec::with_capacity(num_parts);
        let mut file = File::open(local_path).await
            .with_context(|| format!("failed to open {}", local_path.display()))?;

        // Upload each part
        for part_number in 1..=num_parts {
            let start = (part_number - 1) * PART_SIZE;
            let end = std::cmp::min(start + PART_SIZE, file_size);
            let part_size = end - start;

            let mut buffer = vec![0u8; part_size];
            file.read_exact(&mut buffer).await
                .with_context(|| format!("failed to read part {} from {}", part_number, local_path.display()))?;

            log::info!(
                "Uploading part {}/{} ({} bytes)",
                part_number,
                num_parts,
                part_size
            );

            let upload_resp = self.client
                .upload_part()
                .bucket(&self.bucket)
                .key(&full_key)
                .upload_id(upload_id)
                .part_number(part_number as i32)
                .body(ByteStream::from(buffer))
                .send()
                .await;

            match upload_resp {
                Ok(resp) => {
                    let etag = resp.e_tag().context("no etag in upload_part response")?;
                    completed_parts.push(
                        CompletedPart::builder()
                            .part_number(part_number as i32)
                            .e_tag(etag)
                            .build(),
                    );
                }
                Err(e) => {
                    // Abort multipart upload on failure
                    log::error!(
                        "Part {} upload failed, aborting multipart upload: {}",
                        part_number,
                        e
                    );
                    let _ = self.client
                        .abort_multipart_upload()
                        .bucket(&self.bucket)
                        .key(&full_key)
                        .upload_id(upload_id)
                        .send()
                        .await;
                    return Err(e).context("failed to upload part")?;
                }
            }
        }

        // Complete multipart upload
        let completed_upload = CompletedMultipartUpload::builder()
            .set_parts(Some(completed_parts))
            .build();

        self.client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(&full_key)
            .upload_id(upload_id)
            .multipart_upload(completed_upload)
            .send()
            .await
            .context("failed to complete multipart upload")?;

        log::info!("Multipart upload completed: {}", full_key);
        Ok(())
    }

    /// Upload bytes directly to S3.
    pub async fn upload_bytes(&self, s3_key: &str, bytes: Vec<u8>) -> Result<()> {
        let full_key = self.key(s3_key);

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .content_type("application/json")
            .body(ByteStream::from(bytes))
            .send()
            .await
            .with_context(|| format!("failed to upload to s3://{}/{}", self.bucket, full_key))?;

        Ok(())
    }

}

/// Create an S3 client from environment configuration.
pub async fn create_s3_client() -> Result<S3Client> {
    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    Ok(S3Client::new(&config))
}
