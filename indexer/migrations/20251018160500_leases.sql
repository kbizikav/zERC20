-- Create dedicated lease table for advisory-lock replacement with expiration semantics
CREATE TABLE leases (
    lease_key BIGINT PRIMARY KEY,
    holder UUID NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX leases_expires_idx ON leases (expires_at);
