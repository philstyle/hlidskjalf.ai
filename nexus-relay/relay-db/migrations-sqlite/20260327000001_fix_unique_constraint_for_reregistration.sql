-- Allow re-registration after soft-delete by scoping uniqueness to active participants only.
-- SQLite: the broad non-partial uniqueness was not included in migration 2 (cannot be removed
-- later in SQLite), so only the scoped partial index is created here.
CREATE UNIQUE INDEX participants_active_address_unique ON participants(namespace_id, host, agent_name) WHERE status = 'active';
