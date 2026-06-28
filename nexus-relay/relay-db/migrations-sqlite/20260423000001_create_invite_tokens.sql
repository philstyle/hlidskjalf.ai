-- Invite tokens for self-service namespace creation.
-- Single-use: consumed when a namespace is created.
CREATE TABLE invite_tokens (
    id                TEXT PRIMARY KEY,
    key_prefix        TEXT UNIQUE NOT NULL,
    key_hash          TEXT NOT NULL,
    label             TEXT,
    created_at        TEXT NOT NULL,
    used_at           TEXT,
    used_by_namespace TEXT
);
