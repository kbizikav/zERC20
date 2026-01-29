use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A single artifact entry in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactEntry {
    pub path: String,
    pub sha256: String,
}

/// Circuit-specific artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitArtifacts {
    pub nova_pp: ArtifactEntry,
    pub nova_vp: ArtifactEntry,
    pub decider_pp: ArtifactEntry,
    pub decider_vp: ArtifactEntry,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groth16_pk: Option<ArtifactEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groth16_vk: Option<ArtifactEntry>,
}

/// The manifest.json structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub created_at: String,
    pub circuits: BTreeMap<String, CircuitArtifacts>,
}

impl Manifest {
    /// Create a new manifest for the given version.
    pub fn new(version: String) -> Self {
        let created_at = chrono::Utc::now().to_rfc3339();
        Self {
            version,
            created_at,
            circuits: BTreeMap::new(),
        }
    }

    /// Add a circuit's artifacts to the manifest.
    pub fn add_circuit(&mut self, name: &str, artifacts: CircuitArtifacts) {
        self.circuits.insert(name.to_string(), artifacts);
    }

    /// Compute the canonical JSON representation.
    pub fn to_canonical_json(&self) -> Result<String> {
        serde_json_canonicalizer::to_string(self).context("failed to canonicalize manifest to JSON")
    }

    /// Compute the SHA256 digest of the canonical JSON.
    pub fn digest(&self) -> Result<String> {
        let canonical = self.to_canonical_json()?;
        let hash = Sha256::digest(canonical.as_bytes());
        Ok(hex::encode(hash))
    }

    /// Load manifest from a file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read manifest from {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("failed to parse manifest from {}", path.display()))
    }

    /// Save manifest to a file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self).context("failed to serialize manifest")?;
        fs::write(path, content)
            .with_context(|| format!("failed to write manifest to {}", path.display()))
    }

    /// Get all artifact entries as (local_filename, s3_path) pairs.
    pub fn all_artifacts(&self) -> Vec<(String, String)> {
        let mut result = Vec::new();
        for (circuit_name, artifacts) in &self.circuits {
            result.push((
                format!("{}_nova_pp.bin", circuit_name),
                artifacts.nova_pp.path.clone(),
            ));
            result.push((
                format!("{}_nova_vp.bin", circuit_name),
                artifacts.nova_vp.path.clone(),
            ));
            result.push((
                format!("{}_decider_pp.bin", circuit_name),
                artifacts.decider_pp.path.clone(),
            ));
            result.push((
                format!("{}_decider_vp.bin", circuit_name),
                artifacts.decider_vp.path.clone(),
            ));
            if let Some(ref pk) = artifacts.groth16_pk {
                result.push((format!("{}_groth16_pk.bin", circuit_name), pk.path.clone()));
            }
            if let Some(ref vk) = artifacts.groth16_vk {
                result.push((format!("{}_groth16_vk.bin", circuit_name), vk.path.clone()));
            }
        }
        result
    }
}

/// Compute SHA256 hash of a file using streaming to avoid loading entire file into memory.
pub fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file =
        fs::File::open(path).with_context(|| format!("failed to open file {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024]; // 64KB buffer

    loop {
        let n = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read file {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Create an artifact entry for a file.
pub fn create_artifact_entry(
    version: &str,
    filename: &str,
    artifacts_dir: &Path,
) -> Result<ArtifactEntry> {
    let file_path = artifacts_dir.join(filename);
    let sha256 = sha256_file(&file_path)?;
    let s3_path = format!("{}/{}", version, filename);
    Ok(ArtifactEntry {
        path: s3_path,
        sha256,
    })
}
