use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use api_types::prover::CircuitKind;

use crate::errors::ProverError;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub database_url: String,
    #[serde(default = "default_artifacts_dir")]
    pub artifacts_dir: PathBuf,
    #[serde(default = "default_worker_count")]
    pub worker_count: usize,
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
    pub enabled_circuits: CircuitEnablement,
}

pub fn load_config() -> Result<AppConfig, ProverError> {
    let mut cfg: AppConfig =
        envy::from_env().map_err(|err| ProverError::Config(err.to_string()))?;

    if cfg.artifacts_dir.is_relative() {
        cfg.artifacts_dir = workspace_root().join(&cfg.artifacts_dir);
    }

    if cfg.worker_count == 0 {
        return Err(ProverError::Config(
            "worker_count must be greater than zero".to_owned(),
        ));
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

    if !cfg.enabled_circuits.any_enabled() {
        return Err(ProverError::Config(
            "enabled_circuits must contain at least one circuit".to_owned(),
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

fn default_worker_count() -> usize {
    1
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

#[derive(Debug, Clone)]
pub struct CircuitEnablement {
    root: bool,
    withdraw_local: bool,
    withdraw_global: bool,
}

impl CircuitEnablement {
    pub fn root(&self) -> bool {
        self.root
    }

    pub fn withdraw_local(&self) -> bool {
        self.withdraw_local
    }

    pub fn withdraw_global(&self) -> bool {
        self.withdraw_global
    }

    pub fn any_enabled(&self) -> bool {
        self.root || self.withdraw_local || self.withdraw_global
    }

    pub fn contains(&self, circuit: &CircuitKind) -> bool {
        match circuit {
            CircuitKind::Root => self.root,
            CircuitKind::WithdrawLocal => self.withdraw_local,
            CircuitKind::WithdrawGlobal => self.withdraw_global,
        }
    }

    fn none() -> Self {
        Self {
            root: false,
            withdraw_local: false,
            withdraw_global: false,
        }
    }

    fn enable(&mut self, circuit: CircuitKind) {
        match circuit {
            CircuitKind::Root => self.root = true,
            CircuitKind::WithdrawLocal => self.withdraw_local = true,
            CircuitKind::WithdrawGlobal => self.withdraw_global = true,
        }
    }

    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = CircuitKind>,
    {
        let mut enablement = Self::none();
        for circuit in iter {
            enablement.enable(circuit);
        }
        enablement
    }

    fn from_csv(value: &str) -> Result<Self, String> {
        if value.trim().is_empty() {
            return Ok(Self::none());
        }
        let mut circuits = Vec::new();
        for entry in value.split(',') {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                continue;
            }
            let circuit = CircuitKind::from_str(trimmed)?;
            circuits.push(circuit);
        }
        Ok(Self::from_iter(circuits))
    }
}

impl Default for CircuitEnablement {
    fn default() -> Self {
        Self {
            root: true,
            withdraw_local: false,
            withdraw_global: true,
        }
    }
}

impl<'de> Deserialize<'de> for CircuitEnablement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CircuitEnablementVisitor;

        impl<'de> Visitor<'de> for CircuitEnablementVisitor {
            type Value = CircuitEnablement;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a comma-separated string or array of circuit kinds")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                CircuitEnablement::from_csv(value).map_err(E::custom)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut circuits = Vec::new();
                while let Some(kind) = seq.next_element::<CircuitKind>()? {
                    circuits.push(kind);
                }
                Ok(CircuitEnablement::from_iter(circuits))
            }
        }

        deserializer.deserialize_any(CircuitEnablementVisitor)
    }
}
