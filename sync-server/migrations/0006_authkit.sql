-- AuthKit owns identity only. Existing encryption keys and account proofs are unchanged.
DELETE FROM auth_requests;
DROP TABLE email_codes;
ALTER TABLE auth_requests ADD COLUMN browser_hash TEXT;
ALTER TABLE auth_requests ADD COLUMN callback_started INTEGER NOT NULL DEFAULT 0;
ALTER TABLE auth_requests ADD COLUMN workos_user TEXT;
ALTER TABLE auth_requests ADD COLUMN workos_session TEXT;
ALTER TABLE auth_requests ADD COLUMN auth_time INTEGER;
ALTER TABLE sessions ADD COLUMN workos_user TEXT;
ALTER TABLE sessions ADD COLUMN workos_session TEXT;
ALTER TABLE sessions ADD COLUMN provider_revoked INTEGER NOT NULL DEFAULT 0;
CREATE INDEX sessions_workos_user ON sessions(workos_user);
CREATE INDEX sessions_workos_session ON sessions(workos_session);
-- Tombstones prevent late callbacks and webhook replay from restoring provider access.
CREATE TABLE workos_events(id TEXT PRIMARY KEY, at INTEGER NOT NULL);
CREATE TABLE workos_revocations(kind TEXT NOT NULL, subject TEXT NOT NULL, at INTEGER NOT NULL, PRIMARY KEY(kind,subject));
