CREATE INDEX idx_ledger_entries_sender_received ON ledger_entries(sender_id, received_at DESC);
