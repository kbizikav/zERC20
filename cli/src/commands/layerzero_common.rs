use client_common::layerzero::{Destination, LzCompose, LzComposeFailedTx, Stage};

pub fn summarize_stage(stage: Option<&Stage>) -> (String, String, Option<String>) {
    if let Some(stage) = stage {
        let status = stage.status.as_deref().unwrap_or("-").to_string();
        let tx_hash = stage
            .tx
            .as_ref()
            .and_then(|tx| tx.tx_hash.as_deref())
            .unwrap_or("-")
            .to_string();
        let block = stage.tx.as_ref().and_then(|tx| {
            tx.block_number.map(|number| {
                if let Some(ts) = tx.block_timestamp {
                    format!("{number} (timestamp {ts})")
                } else {
                    number.to_string()
                }
            })
        });
        return (status, tx_hash, block);
    }
    ("-".into(), "-".into(), None)
}

pub fn destination_status(destination: &Destination) -> Option<String> {
    destination
        .status
        .clone()
        .or_else(|| {
            destination
                .lz_compose
                .as_ref()
                .and_then(|lz| lz.status.clone())
                .map(|s| format!("lzCompose {}", s))
        })
        .or_else(|| {
            destination
                .native_drop
                .as_ref()
                .and_then(|drop| drop.status.clone())
                .map(|s| format!("nativeDrop {}", s))
        })
}

pub fn destination_tx(destination: &Destination) -> Option<String> {
    destination
        .tx
        .as_ref()
        .and_then(|tx| tx.tx_hash.clone())
        .or_else(|| {
            destination
                .lz_compose
                .as_ref()
                .and_then(|lz| lz.txs.first())
                .and_then(|tx| tx.tx_hash.clone())
        })
        .or_else(|| {
            destination
                .native_drop
                .as_ref()
                .and_then(|drop| drop.tx.as_ref())
                .and_then(|tx| tx.tx_hash.clone())
        })
}

pub fn destination_block(destination: &Destination) -> Option<String> {
    destination
        .tx
        .as_ref()
        .and_then(|tx| block_summary(tx.block_number, tx.block_timestamp))
        .or_else(|| {
            destination
                .lz_compose
                .as_ref()
                .and_then(|lz| lz.txs.first())
                .and_then(|tx| block_summary(tx.block_number, tx.block_timestamp))
        })
        .or_else(|| {
            destination
                .native_drop
                .as_ref()
                .and_then(|drop| drop.tx.as_ref())
                .and_then(|tx| block_summary(tx.block_number, tx.block_timestamp))
        })
}

fn block_summary(block_number: Option<u64>, block_timestamp: Option<u64>) -> Option<String> {
    match (block_number, block_timestamp) {
        (Some(num), Some(ts)) => Some(format!("{num} (timestamp {ts})")),
        (Some(num), None) => Some(num.to_string()),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct ComposeTxSummary {
    pub hash: String,
    pub summary: String,
}

pub fn lz_compose_txs(lz: &LzCompose) -> Vec<ComposeTxSummary> {
    lz.txs
        .iter()
        .enumerate()
        .map(|(idx, tx)| {
            let hash = tx.tx_hash.clone().unwrap_or_else(|| "-".to_string());
            let from = tx.from.as_deref().unwrap_or("-");
            let to = tx.to.as_deref().unwrap_or("-");
            let block = tx
                .block_number
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".to_string());
            let ts = tx.block_timestamp.map(|n| n.to_string());
            let block_desc = ts
                .map(|ts| format!("{block} (timestamp {ts})"))
                .unwrap_or(block);
            ComposeTxSummary {
                hash: hash.clone(),
                summary: format!(
                    "#{} hash={} from={} to={} block={}",
                    idx + 1,
                    hash,
                    from,
                    to,
                    block_desc
                ),
            }
        })
        .collect()
}

pub fn lz_compose_failed_txs(failed: &[LzComposeFailedTx]) -> Vec<String> {
    failed
        .iter()
        .enumerate()
        .map(|(idx, tx)| {
            let hash = tx.tx_hash.as_deref().unwrap_or("-");
            let err = tx.tx_error.as_deref().unwrap_or("-");
            format!("#{} hash={} error={}", idx + 1, hash, err)
        })
        .collect()
}
