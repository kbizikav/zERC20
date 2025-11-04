CREATE TABLE IF NOT EXISTS merkle_nodes_current (
    token_id BIGINT NOT NULL,
    node_path BYTEA NOT NULL,
    hash BYTEA NOT NULL,
    updated_at_index BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (token_id, node_path),
    FOREIGN KEY (token_id) REFERENCES tokens (id)
) PARTITION BY LIST (token_id);

CREATE TABLE IF NOT EXISTS merkle_node_updates (
    token_id BIGINT NOT NULL,
    tree_index BIGINT NOT NULL,
    node_path BYTEA NOT NULL,
    old_hash BYTEA NOT NULL,
    new_hash BYTEA NOT NULL,
    PRIMARY KEY (token_id, tree_index, node_path),
    FOREIGN KEY (token_id) REFERENCES tokens (id)
) PARTITION BY LIST (token_id);

CREATE INDEX IF NOT EXISTS merkle_node_updates_index_idx
    ON merkle_node_updates (token_id, tree_index);

CREATE TABLE IF NOT EXISTS merkle_snapshots (
    token_id BIGINT NOT NULL,
    tree_index BIGINT NOT NULL,
    root_hash BYTEA NOT NULL,
    hash_chain BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (token_id, tree_index),
    FOREIGN KEY (token_id) REFERENCES tokens (id)
) PARTITION BY LIST (token_id);

CREATE INDEX IF NOT EXISTS merkle_snapshots_root_idx
    ON merkle_snapshots (token_id, root_hash, tree_index DESC);
