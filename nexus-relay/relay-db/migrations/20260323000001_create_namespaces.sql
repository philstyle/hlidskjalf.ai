CREATE TABLE namespaces (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL UNIQUE,
    operator_id      UUID,
    admin_key_prefix TEXT NOT NULL,
    admin_key_hash   TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_namespaces_admin_key_prefix ON namespaces(admin_key_prefix);
