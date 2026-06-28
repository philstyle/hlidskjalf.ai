-- SQLite dialect. Ids are TEXT, counts are INTEGER, timestamps are app-bound TEXT.
-- flush_state is created for schema parity; the flush daemon is Postgres-only (gated off on SQLite).
CREATE TABLE flush_state (
    ledger_id             TEXT PRIMARY KEY REFERENCES participants(id),
    last_flushed_sequence INTEGER NOT NULL DEFAULT 0,
    last_flushed_at       TEXT NOT NULL
);
