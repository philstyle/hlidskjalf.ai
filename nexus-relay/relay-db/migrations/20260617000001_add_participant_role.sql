-- Host-isolation Slice 1: supervisory visibility role on participants.
-- NULL = plain participant (host-scoped discovery). 'observer' = namespace-wide
-- read visibility. 'orchestrator' = read + (Phase 2) cross-host messaging.
-- Deny-by-default: the column defaults to NULL and the CHECK rejects any unknown
-- value, so a malformed role can never resolve to a supervisor tier at the DB layer.
ALTER TABLE participants
    ADD COLUMN role text
    CHECK (role IS NULL OR role IN ('observer', 'orchestrator'));
