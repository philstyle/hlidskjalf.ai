CREATE TABLE flush_state (
    ledger_id             UUID PRIMARY KEY REFERENCES participants(id),
    last_flushed_sequence BIGINT NOT NULL DEFAULT 0,
    last_flushed_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
