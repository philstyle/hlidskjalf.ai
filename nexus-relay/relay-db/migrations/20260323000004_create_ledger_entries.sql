CREATE TABLE ledger_entries (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    ledger_id       UUID NOT NULL REFERENCES participants(id),
    sequence        BIGINT NOT NULL,
    received_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    sender_id       UUID NOT NULL REFERENCES participants(id),
    msg_type        TEXT NOT NULL,
    correlation_id  UUID,
    sent_at         TIMESTAMPTZ,
    payload         JSONB NOT NULL DEFAULT '{}',
    attachments     JSONB,
    UNIQUE(ledger_id, sequence)
);
CREATE INDEX idx_ledger_entries_ledger_seq ON ledger_entries(ledger_id, sequence);
