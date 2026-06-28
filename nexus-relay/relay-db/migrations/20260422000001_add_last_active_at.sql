-- Track participant activity for presence detection.
-- Updated on every authenticated request via middleware.
ALTER TABLE participants ADD COLUMN last_active_at TIMESTAMPTZ;
