CREATE TABLE IF NOT EXISTS indexed_blocks (
    token_id BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    block_hash BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (token_id, block_number),
    FOREIGN KEY (token_id) REFERENCES tokens (id)
) PARTITION BY LIST (token_id);

CREATE INDEX IF NOT EXISTS indexed_blocks_token_block_idx
    ON indexed_blocks (token_id, block_number DESC);
