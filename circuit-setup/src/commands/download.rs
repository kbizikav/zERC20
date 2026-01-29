use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::{Client, Response};
use tokio::io::AsyncWriteExt;

use crate::manifest::{sha256_file, Manifest};

const MAX_RETRIES: u32 = 3;

/// Download artifacts from a public URL and verify hashes.
pub async fn download(artifacts_dir: &Path, version: &str, base_url: &str) -> Result<()> {
    std::fs::create_dir_all(artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;

    let base_url = base_url.trim_end_matches('/');
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()?;

    // Download manifest first
    let manifest_url = format!("{}/{}/manifest.json", base_url, version);
    log::info!("Downloading manifest from {}...", manifest_url);

    let manifest_bytes = download_bytes(&client, &manifest_url)
        .await
        .context("failed to download manifest.json")?;

    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).context("failed to parse manifest.json")?;

    // Verify version matches
    if manifest.version != version {
        anyhow::bail!(
            "manifest version ({}) does not match requested version ({})",
            manifest.version,
            version
        );
    }

    // Show manifest digest
    let digest = manifest.digest()?;
    log::info!("Manifest digest: {}", digest);

    // Save manifest
    let manifest_path = artifacts_dir.join("manifest.json");
    std::fs::write(&manifest_path, &manifest_bytes)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    log::info!("Saved manifest.json");

    // Download and verify each artifact
    let artifacts = manifest.all_artifacts();
    let total = artifacts.len();

    log::info!("Processing {} artifacts...", total);

    let mut downloaded_count = 0;
    let mut skipped_count = 0;
    let mut verification_results = Vec::new();

    for (i, (local_filename, s3_path)) in artifacts.iter().enumerate() {
        let local_path = artifacts_dir.join(local_filename);
        let file_url = format!("{}/{}", base_url, s3_path);

        // Get expected hash from manifest
        let expected_hash = get_expected_hash(&manifest, local_filename)?;

        // Check if file already exists and has correct hash
        if local_path.exists() {
            log::info!("[{}/{}] Checking {}...", i + 1, total, local_filename);

            match sha256_file(&local_path) {
                Ok(actual_hash) if actual_hash == expected_hash => {
                    log::info!("[{}/{}] Skipped {} (hash OK)", i + 1, total, local_filename);
                    verification_results.push((local_filename.clone(), true));
                    skipped_count += 1;
                    continue;
                }
                _ => {
                    log::info!(
                        "[{}/{}] Re-downloading {} (hash mismatch)",
                        i + 1,
                        total,
                        local_filename
                    );
                }
            }
        }

        log::info!("[{}/{}] Downloading {}...", i + 1, total, local_filename);

        download_file_with_progress(&client, &file_url, &local_path, local_filename)
            .await
            .with_context(|| format!("failed to download {}", file_url))?;

        downloaded_count += 1;

        // Verify hash
        let actual_hash = sha256_file(&local_path)?;
        let is_valid = actual_hash == expected_hash;
        verification_results.push((local_filename.clone(), is_valid));

        if is_valid {
            log::info!("[{}/{}] {} verified OK", i + 1, total, local_filename);
        } else {
            log::error!("[{}/{}] {} FAILED!", i + 1, total, local_filename);
        }
    }

    // Summary
    println!();
    log::info!("=== Download Summary ===");
    log::info!("Downloaded: {} files", downloaded_count);
    log::info!("Skipped (already exists): {} files", skipped_count);

    let failed: Vec<_> = verification_results
        .iter()
        .filter(|(_, valid)| !valid)
        .collect();

    if failed.is_empty() {
        log::info!("All {} files verified successfully!", total);
    } else {
        log::error!("{} file(s) failed verification:", failed.len());
        for (filename, _) in &failed {
            log::error!("  - {}", filename);
        }
        anyhow::bail!("hash verification failed for {} file(s)", failed.len());
    }

    println!();
    log::info!("Manifest digest: {}", digest);
    log::info!("Download complete: {}", artifacts_dir.display());

    Ok(())
}

/// Send an HTTP GET request with retry and exponential backoff.
async fn download_with_retry(client: &Client, url: &str, retries: u32) -> Result<Response> {
    for attempt in 0..retries {
        match client.get(url).send().await {
            Ok(r) if r.status().is_success() => return Ok(r),
            Ok(r) => {
                if attempt == retries - 1 {
                    anyhow::bail!("HTTP {}", r.status())
                }
            }
            Err(e) => {
                if attempt == retries - 1 {
                    return Err(e.into());
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
    }
    unreachable!()
}

/// Download bytes from a URL.
async fn download_bytes(client: &Client, url: &str) -> Result<Vec<u8>> {
    let response = download_with_retry(client, url, MAX_RETRIES)
        .await
        .with_context(|| format!("failed to fetch {}", url))?;

    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("failed to read response from {}", url))?;

    Ok(bytes.to_vec())
}

/// Download a file from a URL with progress bar.
async fn download_file_with_progress(
    client: &Client,
    url: &str,
    local_path: &Path,
    filename: &str,
) -> Result<()> {
    let response = download_with_retry(client, url, MAX_RETRIES)
        .await
        .with_context(|| format!("failed to fetch {}", url))?;

    let total_size = response.content_length().unwrap_or(0);

    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("  {spinner:.green} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
            .unwrap()
            .progress_chars("#>-")
    );
    pb.set_message(filename.to_string());

    let mut file = tokio::fs::File::create(local_path)
        .await
        .with_context(|| format!("failed to create {}", local_path.display()))?;

    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk_result) = stream.next().await {
        let chunk_result: std::result::Result<bytes::Bytes, reqwest::Error> = chunk_result;
        let chunk = chunk_result
            .map_err(|e| anyhow::anyhow!("failed to read chunk from {}: {}", url, e))?;
        file.write_all(&chunk)
            .await
            .with_context(|| format!("failed to write to {}", local_path.display()))?;
        downloaded += chunk.len() as u64;
        pb.set_position(downloaded);
    }

    pb.finish_and_clear();

    Ok(())
}

/// Get the expected SHA256 hash for a file from the manifest.
fn get_expected_hash(manifest: &Manifest, filename: &str) -> Result<String> {
    // Parse filename to get circuit name and artifact type
    // Format: <circuit>_<type>.bin (e.g., "root_nova_pp.bin")
    let parts: Vec<&str> = filename.trim_end_matches(".bin").split('_').collect();

    if parts.len() < 3 {
        anyhow::bail!("invalid filename format: {}", filename);
    }

    // Determine circuit name and artifact type
    let (circuit_name, artifact_type) = if parts[0] == "withdraw" {
        // withdraw_local_* or withdraw_global_*
        let circuit = format!("{}_{}", parts[0], parts[1]);
        let artifact = parts[2..].join("_");
        (circuit, artifact)
    } else {
        // root_*
        let circuit = parts[0].to_string();
        let artifact = parts[1..].join("_");
        (circuit, artifact)
    };

    let circuit_artifacts = manifest
        .circuits
        .get(&circuit_name)
        .with_context(|| format!("circuit '{}' not found in manifest", circuit_name))?;

    let hash = match artifact_type.as_str() {
        "nova_pp" => &circuit_artifacts.nova_pp.sha256,
        "nova_vp" => &circuit_artifacts.nova_vp.sha256,
        "decider_pp" => &circuit_artifacts.decider_pp.sha256,
        "decider_vp" => &circuit_artifacts.decider_vp.sha256,
        "groth16_pk" => circuit_artifacts
            .groth16_pk
            .as_ref()
            .with_context(|| format!("groth16_pk not found for circuit '{}'", circuit_name))?
            .sha256
            .as_str(),
        "groth16_vk" => circuit_artifacts
            .groth16_vk
            .as_ref()
            .with_context(|| format!("groth16_vk not found for circuit '{}'", circuit_name))?
            .sha256
            .as_str(),
        _ => anyhow::bail!("unknown artifact type: {}", artifact_type),
    };

    Ok(hash.to_string())
}
