-- Allow re-registration after soft-delete by scoping uniqueness to active participants only.
-- The old constraint blocked re-registration because inactive rows still held the (namespace_id, host, agent_name) tuple.
ALTER TABLE participants DROP CONSTRAINT participants_namespace_id_host_agent_name_key;
CREATE UNIQUE INDEX participants_active_address_unique ON participants(namespace_id, host, agent_name) WHERE status = 'active';
