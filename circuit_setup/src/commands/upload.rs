use std::path::Path;

use anyhow::{Context, Result};

use crate::manifest::Manifest;
use crate::s3::Storage;

/// Upload artifacts to S3.
pub async fn upload(
    artifacts_dir: &Path,
    storage: &Storage,
) -> Result<()> {
    // Load manifest
    let manifest_path = artifacts_dir.join("manifest.json");
    let manifest = Manifest::load(&manifest_path)
        .context("failed to load manifest.json - run 'generate' first")?;

    let version = &manifest.version;

    let artifacts = manifest.all_artifacts();
    let total = artifacts.len();

    log::info!("Uploading {} artifacts to S3...", total);

    // Upload each artifact
    for (i, (local_filename, s3_path)) in artifacts.iter().enumerate() {
        let local_path = artifacts_dir.join(local_filename);

        if !local_path.exists() {
            anyhow::bail!("artifact not found: {}", local_path.display());
        }

        let file_size = std::fs::metadata(&local_path)?.len();
        log::info!(
            "[{}/{}] Uploading {} ({} bytes) -> {}",
            i + 1,
            total,
            local_filename,
            file_size,
            s3_path
        );

        storage.upload_file(s3_path, &local_path).await
            .with_context(|| format!("failed to upload {}", local_filename))?;
    }

    // Upload manifest
    let manifest_s3_path = format!("{}/manifest.json", version);
    log::info!("Uploading manifest.json -> {}", manifest_s3_path);

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    storage.upload_bytes(&manifest_s3_path, manifest_json.into_bytes()).await
        .context("failed to upload manifest.json")?;

    let digest = manifest.digest()?;
    log::info!("Upload complete!");
    log::info!("Manifest digest: {}", digest);

    Ok(())
}
