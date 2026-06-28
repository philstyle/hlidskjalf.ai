-- Org namespaces use a channel as their public "operator equivalent" — agents
-- escalating to @{org-ns} land in this channel. Nullable: only org namespaces
-- set it. ON DELETE SET NULL: deleting the channel reverts the namespace to the
-- helpful 404 behavior rather than orphaning the FK.
ALTER TABLE namespaces
    ADD COLUMN gateway_channel_id UUID REFERENCES channels(id) ON DELETE SET NULL;
