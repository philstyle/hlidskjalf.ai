CREATE TABLE root_tokens (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key_prefix      TEXT NOT NULL,
    key_hash        TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_root_tokens_prefix ON root_tokens(key_prefix);
