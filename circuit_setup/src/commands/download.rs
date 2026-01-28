use std::path::Path;

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;

use crate::manifest::{verify_file_hash, Manifest};

/// Download artifacts from a public URL and verify hashes.
pub async fn download(
    artifacts_dir: &Path,
    version: &str,
    base_url: &str,
) -> Result<()> {
    std::fs::create_dir_all(artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;

    let base_url = base_url.trim_end_matches('/');
    let client = reqwest::Client::new();

    // Download manifest first
    let manifest_url = format!("{}/{}/manifest.json", base_url, version);
    log::info!("Downloading manifest from {}...", manifest_url);

    let manifest_bytes = download_bytes(&client, &manifest_url).await
        .context("failed to download manifest.json")?;

    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .context("failed to parse manifest.json")?;

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

    log::info!("Downloading {} artifacts...", total);

    let mut verification_results = Vec::new();

    for (i, (local_filename, s3_path)) in artifacts.iter().enumerate() {
        let local_path = artifacts_dir.join(local_filename);
        let file_url = format!("{}/{}", base_url, s3_path);

        log::info!(
            "[{}/{}] Downloading {} <- {}",
            i + 1,
            total,
            local_filename,
            file_url
        );

        download_file(&client, &file_url, &local_path).await
            .with_context(|| format!("failed to download {}", file_url))?;

        // Get expected hash from manifest
        let expected_hash = get_expected_hash(&manifest, local_filename)?;

        // Verify hash
        let is_valid = verify_file_hash(&local_path, &expected_hash)?;
        verification_results.push((local_filename.clone(), is_valid));

        if is_valid {
            log::info!("  Hash verified: OK");
        } else {
            log::error!("  Hash verification FAILED!");
        }
    }

    // Summary
    log::info!("");
    log::info!("=== Verification Summary ===");

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

    log::info!("");
    log::info!("Manifest digest: {}", digest);
    log::info!("Download complete: {}", artifacts_dir.display());

    Ok(())
}

/// Download bytes from a URL.
async fn download_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = client.get(url)
        .send()
        .await
        .with_context(|| format!("failed to fetch {}", url))?;

    if !response.status().is_success() {
        anyhow::bail!("HTTP {} for {}", response.status(), url);
    }

    let bytes = response.bytes().await
        .with_context(|| format!("failed to read response from {}", url))?;

    Ok(bytes.to_vec())
}

/// Download a file from a URL with streaming.
async fn download_file(client: &reqwest::Client, url: &str, local_path: &Path) -> Result<()> {
    let response = client.get(url)
        .send()
        .await
        .with_context(|| format!("failed to fetch {}", url))?;

    if !response.status().is_success() {
        anyhow::bail!("HTTP {} for {}", response.status(), url);
    }

    let mut file = tokio::fs::File::create(local_path).await
        .with_context(|| format!("failed to create {}", local_path.display()))?;

    let bytes = response.bytes().await
        .with_context(|| format!("failed to read response from {}", url))?;

    file.write_all(&bytes).await
        .with_context(|| format!("failed to write {}", local_path.display()))?;

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

    let circuit_artifacts = manifest.circuits.get(&circuit_name)
        .with_context(|| format!("circuit '{}' not found in manifest", circuit_name))?;

    let hash = match artifact_type.as_str() {
        "nova_pp" => &circuit_artifacts.nova_pp.sha256,
        "nova_vp" => &circuit_artifacts.nova_vp.sha256,
        "decider_pp" => &circuit_artifacts.decider_pp.sha256,
        "decider_vp" => &circuit_artifacts.decider_vp.sha256,
        "groth16_pk" => circuit_artifacts.groth16_pk.as_ref()
            .with_context(|| format!("groth16_pk not found for circuit '{}'", circuit_name))?
            .sha256.as_str(),
        "groth16_vk" => circuit_artifacts.groth16_vk.as_ref()
            .with_context(|| format!("groth16_vk not found for circuit '{}'", circuit_name))?
            .sha256.as_str(),
        _ => anyhow::bail!("unknown artifact type: {}", artifact_type),
    };

    Ok(hash.to_string())
}
