-- Allow ledger_entries.ledger_id to reference either a participant or a channel.
-- Drop the FK to participants(id) since channels also use ledger_entries.
-- The application layer ensures ledger_id points to a valid participant or channel.
ALTER TABLE ledger_entries DROP CONSTRAINT ledger_entries_ledger_id_fkey;
