use std::path::Path;

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};

use crate::manifest::{create_artifact_entry, CircuitArtifacts, Manifest};
use crate::s3::Storage;

/// Upload artifacts to S3.
pub async fn upload(artifacts_dir: &Path, storage: &Storage, version: &str) -> Result<()> {
    // Check if this version already exists on S3
    let manifest_s3_path = format!("{}/manifest.json", version);
    if storage.exists(&manifest_s3_path).await? {
        anyhow::bail!(
            "Version {} already exists on S3. Cannot overwrite existing artifacts.",
            version
        );
    }

    // Load or create manifest
    let manifest_path = artifacts_dir.join("manifest.json");
    let manifest = if manifest_path.exists() {
        let existing_manifest =
            Manifest::load(&manifest_path).context("failed to load manifest.json")?;

        // Verify version matches
        if existing_manifest.version != version {
            anyhow::bail!(
                "Version mismatch: manifest.json has version '{}' but --version is '{}'. \
                Please use the same version or delete manifest.json to regenerate it.",
                existing_manifest.version,
                version
            );
        }

        log::info!("Using existing manifest.json");
        existing_manifest
    } else {
        log::info!("No manifest.json found, creating from artifacts...");
        create_manifest_from_artifacts(artifacts_dir, version)?
    };

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

        upload_file_with_progress(storage, s3_path, &local_path, local_filename, file_size)
            .await
            .with_context(|| format!("failed to upload {}", local_filename))?;

        log::info!("[{}/{}] {} uploaded", i + 1, total, local_filename);
    }

    // Upload manifest
    log::info!("Uploading manifest.json...");

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    storage
        .upload_bytes(&manifest_s3_path, manifest_json.into_bytes())
        .await
        .context("failed to upload manifest.json")?;

    // Save manifest locally if it was newly created
    if !manifest_path.exists() {
        manifest.save(&manifest_path)?;
        log::info!("Saved manifest.json locally");
    }

    let digest = manifest.digest()?;
    println!();
    log::info!("Upload complete!");
    log::info!("Uploaded {} artifacts + manifest.json", total);
    log::info!("Manifest digest: {}", digest);

    Ok(())
}

/// Create manifest from existing artifact files.
fn create_manifest_from_artifacts(artifacts_dir: &Path, version: &str) -> Result<Manifest> {
    let mut manifest = Manifest::new(version.to_string());

    // Check and add root circuit (no groth16)
    let root_nova_pp = artifacts_dir.join("root_nova_pp.bin");
    if root_nova_pp.exists() {
        log::info!("Found root circuit artifacts");
        manifest.add_circuit(
            "root",
            CircuitArtifacts {
                nova_pp: create_artifact_entry(version, "root_nova_pp.bin", artifacts_dir)?,
                nova_vp: create_artifact_entry(version, "root_nova_vp.bin", artifacts_dir)?,
                decider_pp: create_artifact_entry(version, "root_decider_pp.bin", artifacts_dir)?,
                decider_vp: create_artifact_entry(version, "root_decider_vp.bin", artifacts_dir)?,
                groth16_pk: None,
                groth16_vk: None,
            },
        );
    }

    // Check and add withdraw_local circuit
    let withdraw_local_nova_pp = artifacts_dir.join("withdraw_local_nova_pp.bin");
    if withdraw_local_nova_pp.exists() {
        log::info!("Found withdraw_local circuit artifacts");
        let groth16_pk_path = artifacts_dir.join("withdraw_local_groth16_pk.bin");
        let groth16_vk_path = artifacts_dir.join("withdraw_local_groth16_vk.bin");
        let has_groth16 = groth16_pk_path.exists() && groth16_vk_path.exists();

        manifest.add_circuit(
            "withdraw_local",
            CircuitArtifacts {
                nova_pp: create_artifact_entry(
                    version,
                    "withdraw_local_nova_pp.bin",
                    artifacts_dir,
                )?,
                nova_vp: create_artifact_entry(
                    version,
                    "withdraw_local_nova_vp.bin",
                    artifacts_dir,
                )?,
                decider_pp: create_artifact_entry(
                    version,
                    "withdraw_local_decider_pp.bin",
                    artifacts_dir,
                )?,
                decider_vp: create_artifact_entry(
                    version,
                    "withdraw_local_decider_vp.bin",
                    artifacts_dir,
                )?,
                groth16_pk: if has_groth16 {
                    Some(create_artifact_entry(
                        version,
                        "withdraw_local_groth16_pk.bin",
                        artifacts_dir,
                    )?)
                } else {
                    None
                },
                groth16_vk: if has_groth16 {
                    Some(create_artifact_entry(
                        version,
                        "withdraw_local_groth16_vk.bin",
                        artifacts_dir,
                    )?)
                } else {
                    None
                },
            },
        );
    }

    // Check and add withdraw_global circuit
    let withdraw_global_nova_pp = artifacts_dir.join("withdraw_global_nova_pp.bin");
    if withdraw_global_nova_pp.exists() {
        log::info!("Found withdraw_global circuit artifacts");
        let groth16_pk_path = artifacts_dir.join("withdraw_global_groth16_pk.bin");
        let groth16_vk_path = artifacts_dir.join("withdraw_global_groth16_vk.bin");
        let has_groth16 = groth16_pk_path.exists() && groth16_vk_path.exists();

        manifest.add_circuit(
            "withdraw_global",
            CircuitArtifacts {
                nova_pp: create_artifact_entry(
                    version,
                    "withdraw_global_nova_pp.bin",
                    artifacts_dir,
                )?,
                nova_vp: create_artifact_entry(
                    version,
                    "withdraw_global_nova_vp.bin",
                    artifacts_dir,
                )?,
                decider_pp: create_artifact_entry(
                    version,
                    "withdraw_global_decider_pp.bin",
                    artifacts_dir,
                )?,
                decider_vp: create_artifact_entry(
                    version,
                    "withdraw_global_decider_vp.bin",
                    artifacts_dir,
                )?,
                groth16_pk: if has_groth16 {
                    Some(create_artifact_entry(
                        version,
                        "withdraw_global_groth16_pk.bin",
                        artifacts_dir,
                    )?)
                } else {
                    None
                },
                groth16_vk: if has_groth16 {
                    Some(create_artifact_entry(
                        version,
                        "withdraw_global_groth16_vk.bin",
                        artifacts_dir,
                    )?)
                } else {
                    None
                },
            },
        );
    }

    if manifest.circuits.is_empty() {
        anyhow::bail!(
            "No circuit artifacts found in {}. Expected files like root_nova_pp.bin, withdraw_local_nova_pp.bin, etc.",
            artifacts_dir.display()
        );
    }

    log::info!("Created manifest with {} circuits", manifest.circuits.len());
    Ok(manifest)
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
    storage
        .upload_file_with_progress(s3_path, local_path, &pb)
        .await?;

    pb.finish_and_clear();

    Ok(())
}
