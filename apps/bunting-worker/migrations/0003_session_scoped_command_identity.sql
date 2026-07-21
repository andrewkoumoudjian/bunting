ALTER TABLE command_guards ADD COLUMN actor_id TEXT;
ALTER TABLE command_guards ADD COLUMN session_id TEXT;
ALTER TABLE command_guards ADD COLUMN local_command_id TEXT;
ALTER TABLE command_guards ADD COLUMN local_order_id TEXT;

CREATE UNIQUE INDEX command_guards_by_client_identity
ON command_guards(run_id, actor_id, session_id, local_command_id)
WHERE session_id IS NOT NULL AND session_id <> '';
