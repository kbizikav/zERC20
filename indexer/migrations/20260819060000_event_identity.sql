ALTER TABLE indexed_transfer_events
    ADD COLUMN IF NOT EXISTS transaction_hash BYTEA,
    ADD COLUMN IF NOT EXISTS log_index BIGINT;

CREATE INDEX IF NOT EXISTS indexed_transfer_events_tx_log_idx
    ON indexed_transfer_events (token_id, transaction_hash, log_index);
