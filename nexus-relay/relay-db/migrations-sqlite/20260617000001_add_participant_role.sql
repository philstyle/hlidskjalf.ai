-- SQLite dialect. role: NULL=plain, 'observer'/'orchestrator'=supervisory.
-- Deny-by-default CHECK.
ALTER TABLE participants ADD COLUMN role TEXT
    CHECK (role IS NULL OR role IN ('observer', 'orchestrator'));

