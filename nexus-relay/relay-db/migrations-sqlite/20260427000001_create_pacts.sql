-- Connection pacts: bilateral authorization for cross-namespace
-- agent-to-agent messaging. Both operators must consent.
-- participant_a always stores the lower id for order-independent uniqueness.
CREATE TABLE pacts (
    id            TEXT PRIMARY KEY,
    participant_a TEXT NOT NULL REFERENCES participants(id),
    participant_b TEXT NOT NULL REFERENCES participants(id),
    proposed_by   TEXT NOT NULL REFERENCES namespaces(id),
    proposed_at   TEXT NOT NULL,
    approved_by   TEXT REFERENCES namespaces(id),
    approved_at   TEXT,
    revoked_at    TEXT,
    revoked_by    TEXT REFERENCES namespaces(id),
    UNIQUE(participant_a, participant_b)
);
