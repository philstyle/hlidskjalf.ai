-- SQLite dialect. Types: ids are app-minted TEXT, timestamps are app-bound TEXT.
-- Folds: operator FK (migration 3), namespace_type+CHECK (migration 523),
-- gateway_channel_id (migration 524) — constraint-altering DDL is not supported.
CREATE TABLE namespaces (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL UNIQUE,
    operator_id        TEXT REFERENCES participants(id),
    admin_key_prefix   TEXT NOT NULL,
    admin_key_hash     TEXT NOT NULL,
    created_at         TEXT NOT NULL,
    namespace_type     TEXT NOT NULL DEFAULT 'operator' CHECK(namespace_type IN ('operator', 'org')),
    gateway_channel_id TEXT REFERENCES channels(id) ON DELETE SET NULL
);
CREATE INDEX idx_namespaces_admin_key_prefix ON namespaces(admin_key_prefix);
