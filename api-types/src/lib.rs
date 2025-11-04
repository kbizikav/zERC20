pub mod prover {
    use serde::{Deserialize, Serialize};
    use std::{fmt, str::FromStr};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum CircuitKind {
        Root,
        WithdrawLocal,
        WithdrawGlobal,
    }

    impl fmt::Display for CircuitKind {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                CircuitKind::Root => write!(f, "root"),
                CircuitKind::WithdrawLocal => write!(f, "withdraw_local"),
                CircuitKind::WithdrawGlobal => write!(f, "withdraw_global"),
            }
        }
    }

    impl FromStr for CircuitKind {
        type Err = String;

        fn from_str(value: &str) -> Result<Self, Self::Err> {
            match value {
                "root" => Ok(CircuitKind::Root),
                "withdraw_local" => Ok(CircuitKind::WithdrawLocal),
                "withdraw_global" => Ok(CircuitKind::WithdrawGlobal),
                other => Err(format!("unsupported circuit kind '{other}'")),
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum JobStatus {
        Queued,
        Processing,
        Completed,
        Failed,
    }

    impl fmt::Display for JobStatus {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                JobStatus::Queued => write!(f, "queued"),
                JobStatus::Processing => write!(f, "processing"),
                JobStatus::Completed => write!(f, "completed"),
                JobStatus::Failed => write!(f, "failed"),
            }
        }
    }

    impl FromStr for JobStatus {
        type Err = String;

        fn from_str(value: &str) -> Result<Self, Self::Err> {
            match value {
                "queued" => Ok(JobStatus::Queued),
                "processing" => Ok(JobStatus::Processing),
                "completed" => Ok(JobStatus::Completed),
                "failed" => Ok(JobStatus::Failed),
                other => Err(format!("unsupported job status '{other}'")),
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct SubmitJobResponse {
        pub job_id: String,
        pub status: JobStatus,
        pub message: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct JobStatusResponse {
        pub job_id: String,
        pub circuit: CircuitKind,
        pub status: JobStatus,
        #[serde(default)]
        pub result: Option<String>,
        #[serde(default)]
        pub error: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct JobInfoResponse {
        pub job_id: String,
        pub circuit: CircuitKind,
        pub status: JobStatus,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct JobRequest {
        pub job_id: String,
        pub circuit: CircuitKind,
        pub ivc_proof: String,
    }
}

mod serde_utils {
    use alloy::primitives::U256;

    fn parse_u256(value: &str) -> Result<U256, String> {
        let trimmed = value.trim();
        let hex = trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
            .unwrap_or(trimmed);
        if hex.is_empty() {
            return Ok(U256::ZERO);
        }
        U256::from_str_radix(hex, 16).map_err(|err| format!("invalid hex U256 '{value}': {err}"))
    }

    pub mod u256_hex {
        use super::parse_u256;
        use alloy::primitives::U256;
        use serde::{Deserialize, Deserializer, Serializer};

        pub fn serialize<S>(value: &U256, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&format!("{value:#x}"))
        }

        pub fn deserialize<'de, D>(deserializer: D) -> Result<U256, D::Error>
        where
            D: Deserializer<'de>,
        {
            let input = String::deserialize(deserializer)?;
            parse_u256(&input).map_err(serde::de::Error::custom)
        }
    }

    pub mod u256_vec_hex {
        use super::parse_u256;
        use alloy::primitives::U256;
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        pub fn serialize<S>(values: &[U256], serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let strings: Vec<String> = values.iter().map(|v| format!("{v:#x}")).collect();
            strings.serialize(serializer)
        }

        pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<U256>, D::Error>
        where
            D: Deserializer<'de>,
        {
            let inputs = Vec::<String>::deserialize(deserializer)?;
            inputs
                .into_iter()
                .map(|value| parse_u256(&value).map_err(serde::de::Error::custom))
                .collect()
        }
    }
}

pub mod indexer {
    use alloy::primitives::{Address, U256};
    use serde::{Deserialize, Serialize};
    use serde_with::{DisplayFromStr, serde_as};

    #[serde_as]
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct TokenStatusResponse {
        pub label: String,
        pub chain_id: u64,
        #[serde_as(as = "DisplayFromStr")]
        pub token_address: Address,
        #[serde_as(as = "DisplayFromStr")]
        pub verifier_address: Address,
        #[serde(default)]
        pub onchain_reserved_index: Option<u64>,
        #[serde(default)]
        pub onchain_proved_index: Option<u64>,
        #[serde(default)]
        pub events_synced_index: Option<u64>,
        #[serde(default)]
        pub tree_synced_index: Option<u64>,
        #[serde(default)]
        pub ivc_generated_index: Option<u64>,
    }

    #[serde_as]
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct IndexedEvent {
        pub event_index: u64,
        #[serde_as(as = "DisplayFromStr")]
        pub from: Address,
        #[serde_as(as = "DisplayFromStr")]
        pub to: Address,
        #[serde(with = "crate::serde_utils::u256_hex")]
        pub value: U256,
        pub eth_block_number: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct HistoricalProof {
        pub target_index: u64,
        pub leaf_index: u64,
        #[serde(with = "crate::serde_utils::u256_hex")]
        pub root: U256,
        #[serde(with = "crate::serde_utils::u256_hex")]
        pub hash_chain: U256,
        #[serde(with = "crate::serde_utils::u256_vec_hex")]
        pub siblings: Vec<U256>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct TreeIndexResponse {
        pub tree_index: u64,
    }

    #[serde_as]
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ProveManyRequest {
        pub chain_id: u64,
        #[serde_as(as = "DisplayFromStr")]
        pub token_address: Address,
        pub target_index: u64,
        pub leaf_indices: Vec<u64>,
    }

    #[serde_as]
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct EventsQuery {
        pub chain_id: u64,
        #[serde_as(as = "DisplayFromStr")]
        pub token_address: Address,
        #[serde_as(as = "DisplayFromStr")]
        pub to: Address,
        #[serde(default)]
        pub limit: Option<usize>,
    }

    #[serde_as]
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct TreeIndexQuery {
        pub chain_id: u64,
        #[serde_as(as = "DisplayFromStr")]
        pub token_address: Address,
        #[serde(with = "crate::serde_utils::u256_hex")]
        pub transfer_root: U256,
    }
}
