-- Connection pacts: bilateral authorization for cross-namespace
-- agent-to-agent messaging. Both operators must consent.
-- participant_a always stores the lower UUID for order-independent uniqueness.
CREATE TABLE pacts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    participant_a UUID NOT NULL REFERENCES participants(id),
    participant_b UUID NOT NULL REFERENCES participants(id),
    proposed_by UUID NOT NULL REFERENCES namespaces(id),
    proposed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    approved_by UUID REFERENCES namespaces(id),
    approved_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    revoked_by UUID REFERENCES namespaces(id),
    UNIQUE(participant_a, participant_b)
);
