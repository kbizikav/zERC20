CREATE TABLE IF NOT EXISTS root_ivc_proofs (
    token_id BIGINT NOT NULL,
    start_index BIGINT NOT NULL,
    end_index BIGINT NOT NULL,
    ivc_proof BYTEA NOT NULL,
    state_index BIGINT NOT NULL,
    state_hash_chain BYTEA NOT NULL,
    state_root BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (token_id, end_index),
    FOREIGN KEY (token_id) REFERENCES tokens (id)
);

CREATE INDEX IF NOT EXISTS root_ivc_proofs_token_idx
    ON root_ivc_proofs (token_id, end_index DESC);

CREATE TABLE IF NOT EXISTS root_prover_state (
    token_id BIGINT PRIMARY KEY,
    base_index BIGINT NOT NULL,
    last_compiled_index BIGINT NOT NULL,
    last_submitted_index BIGINT NOT NULL,
    pending_reserved_index BIGINT,
    pending_reserved_hash_chain BYTEA,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (token_id) REFERENCES tokens (id)
);
