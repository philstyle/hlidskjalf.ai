CREATE TABLE host_policy (
    namespace_id uuid NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    host text NOT NULL,
    isolation_enabled boolean NOT NULL DEFAULT false,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (namespace_id, host)
);

