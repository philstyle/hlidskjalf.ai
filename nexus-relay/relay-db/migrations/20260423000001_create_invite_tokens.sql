-- Invite tokens for self-service namespace creation.
-- Single-use: consumed when a namespace is created.
CREATE TABLE invite_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key_prefix TEXT UNIQUE NOT NULL,
    key_hash TEXT NOT NULL,
    label TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    used_at TIMESTAMPTZ,
    used_by_namespace TEXT
);
