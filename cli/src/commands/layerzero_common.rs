use client_common::layerzero::Stage;

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
