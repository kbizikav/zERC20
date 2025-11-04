use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::errors::ProverError;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub redis_url: String,
    #[serde(default = "default_artifacts_dir")]
    pub artifacts_dir: PathBuf,
    #[serde(default = "default_queue_key")]
    pub queue_key: String,
    #[serde(default = "default_job_key_prefix")]
    pub job_key_prefix: String,
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_json_body_limit")]
    pub json_body_limit_bytes: usize,
    #[serde(default = "default_job_ttl_seconds")]
    pub job_ttl_seconds: u64,
    #[serde(default)]
    pub enable_withdraw_local: bool,
}

pub fn load_config() -> Result<AppConfig, ProverError> {
    let mut cfg: AppConfig =
        envy::from_env().map_err(|err| ProverError::Config(err.to_string()))?;

    if cfg.artifacts_dir.is_relative() {
        cfg.artifacts_dir = workspace_root().join(&cfg.artifacts_dir);
    }

    if cfg.job_ttl_seconds == 0 {
        return Err(ProverError::Config(
            "job_ttl_seconds must be greater than zero".to_owned(),
        ));
    }

    Ok(cfg)
}

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_artifacts_dir() -> PathBuf {
    workspace_root().join("nova_artifacts")
}

fn default_queue_key() -> String {
    "prover:queue".to_owned()
}

fn default_job_key_prefix() -> String {
    "prover:job".to_owned()
}

fn default_listen_addr() -> String {
    "0.0.0.0:8080".to_owned()
}

fn default_json_body_limit() -> usize {
    40 * 1024 * 1024
}

fn default_job_ttl_seconds() -> u64 {
    24 * 60 * 60
}
