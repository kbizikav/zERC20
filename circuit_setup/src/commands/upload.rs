use std::path::Path;

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};

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

    log::info!("Uploading {} artifacts (version: {})...", total, version);

    // Upload each artifact
    for (i, (local_filename, s3_path)) in artifacts.iter().enumerate() {
        let local_path = artifacts_dir.join(local_filename);

        if !local_path.exists() {
            anyhow::bail!("artifact not found: {}", local_path.display());
        }

        let file_size = std::fs::metadata(&local_path)?.len();

        log::info!("[{}/{}] Uploading {}...", i + 1, total, local_filename);

        upload_file_with_progress(storage, s3_path, &local_path, local_filename, file_size).await
            .with_context(|| format!("failed to upload {}", local_filename))?;

        log::info!("[{}/{}] {} uploaded", i + 1, total, local_filename);
    }

    // Upload manifest
    log::info!("Uploading manifest.json...");
    let manifest_s3_path = format!("{}/manifest.json", version);

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    storage.upload_bytes(&manifest_s3_path, manifest_json.into_bytes()).await
        .context("failed to upload manifest.json")?;

    let digest = manifest.digest()?;
    println!();
    log::info!("Upload complete!");
    log::info!("Uploaded {} artifacts + manifest.json", total);
    log::info!("Manifest digest: {}", digest);

    Ok(())
}

/// Upload a file to S3 with progress bar.
async fn upload_file_with_progress(
    storage: &Storage,
    s3_path: &str,
    local_path: &Path,
    filename: &str,
    file_size: u64,
) -> Result<()> {
    let pb = ProgressBar::new(file_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("  {spinner:.green} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
            .unwrap()
            .progress_chars("#>-")
    );
    pb.set_message(filename.to_string());

    // For S3 upload, we show progress during the upload
    // The actual progress tracking happens inside storage.upload_file
    storage.upload_file_with_progress(s3_path, local_path, &pb).await?;

    pb.finish_and_clear();

    Ok(())
}
