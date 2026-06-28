-- SQLite dialect. Ids are app-minted TEXT, timestamps are app-bound TEXT.
CREATE TABLE root_tokens (
    id         TEXT PRIMARY KEY,
    key_prefix TEXT NOT NULL,
    key_hash   TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_root_tokens_prefix ON root_tokens(key_prefix);
