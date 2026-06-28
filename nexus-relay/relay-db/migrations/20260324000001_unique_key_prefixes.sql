-- Enforce prefix uniqueness to prevent brute-force DoS via Argon2 iteration
-- and ensure O(1) token lookups instead of scanning all collisions.

-- Drop the old non-unique indexes
DROP INDEX IF EXISTS idx_namespaces_admin_key_prefix;
DROP INDEX IF EXISTS idx_participants_api_key_prefix;
DROP INDEX IF EXISTS idx_root_tokens_prefix;

-- Replace with unique constraints
CREATE UNIQUE INDEX idx_namespaces_admin_key_prefix ON namespaces(admin_key_prefix);
CREATE UNIQUE INDEX idx_participants_api_key_prefix ON participants(api_key_prefix) WHERE status = 'active';
CREATE UNIQUE INDEX idx_root_tokens_prefix ON root_tokens(key_prefix);
