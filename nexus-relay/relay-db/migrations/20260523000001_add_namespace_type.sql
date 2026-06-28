-- Org-level namespaces — adds namespace_type discriminator.
--
-- 'operator' (default): existing semantics. One operator participant,
--   cross-namespace messaging requires operator target or active pact.
-- 'org': shared org commons. No operator participant. Any participant
--   in any namespace can send to org participants and vice versa.
--   Org callers see all participants org-wide in directory listings.
--
-- operator_id becomes nullable because org namespaces have no
-- canonical operator participant.

ALTER TABLE namespaces ADD COLUMN namespace_type TEXT NOT NULL DEFAULT 'operator';
ALTER TABLE namespaces ALTER COLUMN operator_id DROP NOT NULL;
ALTER TABLE namespaces ADD CONSTRAINT namespace_type_check
    CHECK (namespace_type IN ('operator', 'org'));
