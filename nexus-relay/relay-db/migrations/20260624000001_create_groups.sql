CREATE TABLE groups (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id UUID NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    is_default BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (namespace_id, name)
);

CREATE TABLE group_membership (
    group_id UUID NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    participant_id UUID NOT NULL REFERENCES participants(id) ON DELETE CASCADE,
    added_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (group_id, participant_id)
);

CREATE INDEX idx_group_membership_participant ON group_membership(participant_id);

INSERT INTO groups (namespace_id, name, is_default)
    SELECT id, name, true FROM namespaces;

INSERT INTO group_membership (group_id, participant_id)
    SELECT g.id, p.id
    FROM groups g
    JOIN participants p ON p.namespace_id = g.namespace_id
    WHERE g.is_default = true AND p.status = 'active';
