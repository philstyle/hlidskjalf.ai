-- SQLite dialect: ids TEXT, booleans INTEGER, timestamps TEXT.
CREATE TABLE host_policy (
    namespace_id      TEXT NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    host              TEXT NOT NULL,
    isolation_enabled INTEGER NOT NULL DEFAULT 0,
    updated_at        TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (namespace_id, host)
);
