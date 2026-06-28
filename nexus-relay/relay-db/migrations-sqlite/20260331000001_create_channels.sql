-- Channels: shared ledgers for cross-namespace topic-based messaging.
-- channel.id IS the ledger_id — reuses ledger_entries unchanged.
-- SQLite dialect: ids are app-minted TEXT, timestamps are app-bound TEXT.
CREATE TABLE channels (
    id          TEXT PRIMARY KEY,
    name        TEXT UNIQUE NOT NULL,
    description TEXT,
    created_by  TEXT NOT NULL REFERENCES participants(id),
    created_at  TEXT NOT NULL
);
