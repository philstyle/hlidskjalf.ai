-- Channels: shared ledgers for cross-namespace topic-based messaging.
-- channel.id IS the ledger_id — reuses ledger_entries unchanged.
CREATE TABLE channels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT UNIQUE NOT NULL,
    description TEXT,
    created_by UUID NOT NULL REFERENCES participants(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
