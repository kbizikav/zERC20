use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::errors::ProverError;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub database_url: String,
    #[serde(default = "default_artifacts_dir")]
    pub artifacts_dir: PathBuf,
    #[serde(default = "default_queue_name")]
    pub queue_name: String,
    #[serde(default = "default_job_table")]
    pub job_table: String,
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_json_body_limit")]
    pub json_body_limit_bytes: usize,
    #[serde(default = "default_job_ttl_seconds")]
    pub job_ttl_seconds: u64,
    #[serde(default = "default_visibility_timeout_seconds")]
    pub visibility_timeout_seconds: i32,
    #[serde(default = "default_visibility_extension_seconds")]
    pub visibility_extension_seconds: i32,
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

    if cfg.visibility_timeout_seconds <= 0 {
        return Err(ProverError::Config(
            "visibility_timeout_seconds must be greater than zero".to_owned(),
        ));
    }

    if cfg.visibility_extension_seconds <= 0 {
        return Err(ProverError::Config(
            "visibility_extension_seconds must be greater than zero".to_owned(),
        ));
    }

    if cfg.visibility_timeout_seconds <= cfg.visibility_extension_seconds {
        return Err(ProverError::Config(
            "visibility_timeout_seconds must be greater than visibility_extension_seconds"
                .to_owned(),
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

fn default_queue_name() -> String {
    "prover_queue".to_owned()
}

fn default_job_table() -> String {
    "prover_jobs".to_owned()
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

fn default_visibility_timeout_seconds() -> i32 {
    15 * 60
}

fn default_visibility_extension_seconds() -> i32 {
    60
}
