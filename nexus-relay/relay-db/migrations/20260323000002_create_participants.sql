CREATE TABLE participants (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id     UUID NOT NULL REFERENCES namespaces(id),
    host             TEXT,
    agent_name       TEXT,
    participant_type TEXT NOT NULL DEFAULT 'agent',
    is_operator      BOOLEAN NOT NULL DEFAULT false,
    api_key_prefix   TEXT NOT NULL,
    api_key_hash     TEXT NOT NULL,
    notify_config    JSONB,
    status           TEXT NOT NULL DEFAULT 'active',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(namespace_id, host, agent_name)
);
CREATE INDEX idx_participants_api_key_prefix ON participants(api_key_prefix) WHERE status = 'active';
CREATE INDEX idx_participants_namespace ON participants(namespace_id);
