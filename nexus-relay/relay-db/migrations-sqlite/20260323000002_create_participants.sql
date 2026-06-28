-- SQLite dialect. Ids are app-minted TEXT, booleans are INTEGER, json is TEXT,
-- timestamps are app-bound TEXT. No generated defaults.
-- The broad uniqueness on (namespace_id,host,agent_name) is omitted here because
-- migration 000327000001 creates the scoped partial index and the old table
-- constraint cannot be removed in SQLite.
CREATE TABLE participants (
    id               TEXT PRIMARY KEY,
    namespace_id     TEXT NOT NULL REFERENCES namespaces(id),
    host             TEXT,
    agent_name       TEXT,
    participant_type TEXT NOT NULL DEFAULT 'agent',
    is_operator      INTEGER NOT NULL DEFAULT 0,
    api_key_prefix   TEXT NOT NULL,
    api_key_hash     TEXT NOT NULL,
    notify_config    TEXT,
    status           TEXT NOT NULL DEFAULT 'active',
    created_at       TEXT NOT NULL
);
CREATE INDEX idx_participants_api_key_prefix ON participants(api_key_prefix) WHERE status = 'active';
CREATE INDEX idx_participants_namespace ON participants(namespace_id);
