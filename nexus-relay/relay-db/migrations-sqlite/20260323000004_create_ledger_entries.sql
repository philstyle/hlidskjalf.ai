-- SQLite dialect. Ids are TEXT, sequence is INTEGER, json columns are TEXT,
-- timestamps are app-bound TEXT. ledger_id has no FK (Postgres drops it in
-- migration 331000002; never creating it is equivalent).
CREATE TABLE ledger_entries (
    id             TEXT PRIMARY KEY,
    ledger_id      TEXT NOT NULL,
    sequence       INTEGER NOT NULL,
    received_at    TEXT NOT NULL,
    sender_id      TEXT NOT NULL REFERENCES participants(id),
    msg_type       TEXT NOT NULL,
    correlation_id TEXT,
    sent_at        TEXT,
    payload        TEXT NOT NULL DEFAULT '{}',
    attachments    TEXT,
    UNIQUE(ledger_id, sequence)
);
CREATE INDEX idx_ledger_entries_ledger_seq ON ledger_entries(ledger_id, sequence);
