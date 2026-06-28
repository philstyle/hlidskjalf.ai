-- Participant description for agent discovery.
-- Agents set this to describe their current repo/function so
-- other agents can find the right participant to message.
ALTER TABLE participants ADD COLUMN description TEXT;
