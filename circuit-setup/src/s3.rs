use std::path::Path;

use anyhow::{Context, Result};
use aws_sdk_s3::{
    primitives::ByteStream,
    types::{CompletedMultipartUpload, CompletedPart},
    Client as S3Client,
};
use indicatif::ProgressBar;
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

    /// Check if an object exists in S3.
    pub async fn exists(&self, s3_key: &str) -> Result<bool> {
        let full_key = self.key(s3_key);

        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(err) => {
                if err
                    .as_service_error()
                    .map(|e| e.is_not_found())
                    .unwrap_or(false)
                {
                    Ok(false)
                } else {
                    Err(err).with_context(|| {
                        format!("failed to check existence of s3://{}/{}", self.bucket, full_key)
                    })
                }
            }
        }
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

    /// Upload a file to S3 with progress bar.
    pub async fn upload_file_with_progress(
        &self,
        s3_key: &str,
        local_path: &Path,
        pb: &ProgressBar,
    ) -> Result<()> {
        let metadata = tokio::fs::metadata(local_path)
            .await
            .with_context(|| format!("failed to get metadata for {}", local_path.display()))?;
        let file_size = metadata.len() as usize;

        if file_size < MULTIPART_THRESHOLD {
            self.upload_file_simple_with_progress(s3_key, local_path, pb)
                .await
        } else {
            self.upload_file_multipart_with_progress(s3_key, local_path, file_size, pb)
                .await
        }
    }

    /// Simple upload for small files with progress.
    async fn upload_file_simple_with_progress(
        &self,
        s3_key: &str,
        local_path: &Path,
        pb: &ProgressBar,
    ) -> Result<()> {
        let full_key = self.key(s3_key);

        // Read file with progress
        let mut file = File::open(local_path)
            .await
            .with_context(|| format!("failed to open {}", local_path.display()))?;

        let file_size = file.metadata().await?.len();
        let mut buffer = Vec::with_capacity(file_size as usize);
        let mut chunk = vec![0u8; 64 * 1024]; // 64KB chunks for progress updates
        let mut total_read: u64 = 0;

        loop {
            let n = file
                .read(&mut chunk)
                .await
                .with_context(|| format!("failed to read {}", local_path.display()))?;
            if n == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..n]);
            total_read += n as u64;
            pb.set_position(total_read / 2); // Reading is ~50% of the work
        }

        // Upload
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .content_type("application/octet-stream")
            .body(ByteStream::from(buffer))
            .send()
            .await
            .with_context(|| {
                format!(
                    "failed to upload {} to s3://{}/{}",
                    local_path.display(),
                    self.bucket,
                    full_key
                )
            })?;

        pb.set_position(file_size);

        Ok(())
    }

    /// Multipart upload for large files with progress.
    async fn upload_file_multipart_with_progress(
        &self,
        s3_key: &str,
        local_path: &Path,
        file_size: usize,
        pb: &ProgressBar,
    ) -> Result<()> {
        let full_key = self.key(s3_key);
        let num_parts = file_size.div_ceil(PART_SIZE);

        // Create multipart upload
        let create_resp = self
            .client
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
        let mut file = File::open(local_path)
            .await
            .with_context(|| format!("failed to open {}", local_path.display()))?;

        let mut uploaded: u64 = 0;

        // Upload each part
        for part_number in 1..=num_parts {
            let start = (part_number - 1) * PART_SIZE;
            let end = std::cmp::min(start + PART_SIZE, file_size);
            let part_size = end - start;

            let mut buffer = vec![0u8; part_size];
            file.read_exact(&mut buffer).await.with_context(|| {
                format!(
                    "failed to read part {} from {}",
                    part_number,
                    local_path.display()
                )
            })?;

            let upload_resp = self
                .client
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
                    uploaded += part_size as u64;
                    pb.set_position(uploaded);
                }
                Err(e) => {
                    // Abort multipart upload on failure
                    let _ = self
                        .client
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

        Ok(())
    }
}

/// Create an S3 client from environment configuration.
pub async fn create_s3_client() -> Result<S3Client> {
    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    Ok(S3Client::new(&config))
}
