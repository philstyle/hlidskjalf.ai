CREATE TABLE groups (
    id            TEXT PRIMARY KEY,
    namespace_id  TEXT NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    is_default    INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    UNIQUE (namespace_id, name)
);

CREATE TABLE group_membership (
    group_id        TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    participant_id  TEXT NOT NULL REFERENCES participants(id) ON DELETE CASCADE,
    added_at        TEXT NOT NULL,
    PRIMARY KEY (group_id, participant_id)
);

CREATE INDEX idx_group_membership_participant ON group_membership(participant_id);

INSERT INTO groups (id, namespace_id, name, is_default, created_at)
    SELECT
        lower(hex(randomblob(4))) || '-' ||
        lower(hex(randomblob(2))) || '-' ||
        '4' || substr(lower(hex(randomblob(2))), 2) || '-' ||
        substr('89ab', abs(random()) % 4 + 1, 1) ||
        substr(lower(hex(randomblob(2))), 2) || '-' ||
        lower(hex(randomblob(6))),
        id,
        name,
        1,
        CURRENT_TIMESTAMP
    FROM namespaces;

INSERT INTO group_membership (group_id, participant_id, added_at)
    SELECT g.id, p.id, CURRENT_TIMESTAMP
    FROM groups g
    JOIN participants p ON p.namespace_id = g.namespace_id
    WHERE g.is_default = 1 AND p.status = 'active';
