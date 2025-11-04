CREATE TABLE IF NOT EXISTS tokens (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    token_address BYTEA NOT NULL,
    verifier_address BYTEA NOT NULL,
    chain_id BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (token_address, chain_id)
);

CREATE TABLE IF NOT EXISTS indexed_transfer_events (
    token_id BIGINT NOT NULL,
    event_index BIGINT NOT NULL,
    from_address BYTEA NOT NULL,
    to_address BYTEA NOT NULL,
    value BYTEA NOT NULL,
    eth_block_number BIGINT NOT NULL,
    PRIMARY KEY (token_id, event_index),
    FOREIGN KEY (token_id) REFERENCES tokens (id)
) PARTITION BY LIST (token_id);

CREATE INDEX IF NOT EXISTS indexed_transfer_events_token_to_idx
    ON indexed_transfer_events (token_id, to_address, event_index);

CREATE TABLE IF NOT EXISTS event_indexer_state (
    token_id BIGINT NOT NULL,
    contiguous_index BIGINT NOT NULL DEFAULT -1,
    contiguous_block BIGINT,
    last_synced_block BIGINT NOT NULL,
    last_seen_contract_index BIGINT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (token_id),
    FOREIGN KEY (token_id) REFERENCES tokens (id)
) PARTITION BY LIST (token_id);
