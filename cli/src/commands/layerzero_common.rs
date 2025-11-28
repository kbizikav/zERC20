use client_common::layerzero::Stage;
use serde_json::Value;

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

pub fn destination_status(value: &Value) -> Option<String> {
    value
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            value
                .get("lzCompose")
                .and_then(|v| v.get("status"))
                .and_then(|s| s.as_str())
                .map(|s| format!("lzCompose {}", s))
        })
        .or_else(|| {
            value
                .get("nativeDrop")
                .and_then(|v| v.get("status"))
                .and_then(|s| s.as_str())
                .map(|s| format!("nativeDrop {}", s))
        })
}

pub fn destination_tx(value: &Value) -> Option<String> {
    value
        .get("tx")
        .and_then(|v| v.get("txHash"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            value
                .get("payloadStoredTx")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .or_else(|| {
            value
                .get("lzCompose")
                .and_then(|v| v.get("txs"))
                .and_then(|txs| txs.as_array())
                .and_then(|array| array.first())
                .and_then(|v| v.get("txHash"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .or_else(|| {
            value
                .get("nativeDrop")
                .and_then(|v| v.get("tx"))
                .and_then(|v| v.get("txHash"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

pub fn destination_block(value: &Value) -> Option<String> {
    value
        .get("tx")
        .and_then(block_summary)
        .or_else(|| {
            value
                .get("lzCompose")
                .and_then(|v| v.get("txs"))
                .and_then(|txs| txs.as_array())
                .and_then(|array| array.first())
                .and_then(block_summary)
        })
        .or_else(|| {
            value
                .get("nativeDrop")
                .and_then(|v| v.get("tx"))
                .and_then(block_summary)
        })
}

fn block_summary(value: &Value) -> Option<String> {
    let number = value
        .get("blockNumber")
        .and_then(parse_u64_from_value)
        .map(|n| n.to_string());
    let timestamp = value.get("blockTimestamp").and_then(parse_u64_from_value);

    match (number, timestamp) {
        (Some(num), Some(ts)) => Some(format!("{num} (timestamp {ts})")),
        (Some(num), None) => Some(num),
        _ => None,
    }
}

fn parse_u64_from_value(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse::<u64>().ok()))
}

#[derive(Debug, Clone)]
pub struct ComposeTxSummary {
    pub hash: String,
    pub summary: String,
}

pub fn lz_compose_txs(value: &Value) -> Vec<ComposeTxSummary> {
    value
        .get("txs")
        .and_then(|v| v.as_array())
        .map(|txs| {
            txs.iter()
                .enumerate()
                .map(|(idx, tx)| {
                    let hash = tx
                        .get("txHash")
                        .and_then(|v| v.as_str())
                        .unwrap_or("-")
                        .to_string();
                    let from = tx.get("from").and_then(|v| v.as_str()).unwrap_or("-");
                    let to = tx.get("to").and_then(|v| v.as_str()).unwrap_or("-");
                    let block = tx
                        .get("blockNumber")
                        .and_then(parse_u64_from_value)
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let ts = tx
                        .get("blockTimestamp")
                        .and_then(parse_u64_from_value)
                        .map(|n| n.to_string());
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
        })
        .unwrap_or_default()
}

pub fn lz_compose_failed_txs(value: &Value) -> Vec<String> {
    value
        .get("failedTx")
        .and_then(|v| v.as_array())
        .map(|txs| {
            txs.iter()
                .enumerate()
                .map(|(idx, tx)| {
                    let hash = tx.get("txHash").and_then(|v| v.as_str()).unwrap_or("-");
                    let err = tx.get("txError").and_then(|v| v.as_str()).unwrap_or("-");
                    format!("#{} hash={} error={}", idx + 1, hash, err)
                })
                .collect()
        })
        .unwrap_or_default()
}
