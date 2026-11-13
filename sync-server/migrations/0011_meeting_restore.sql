-- Retired objects carry no body; restoring them must not reopen writes to inactive meetings.
DROP TRIGGER meeting_insert_guard;
CREATE TRIGGER meeting_insert_guard BEFORE INSERT ON objects WHEN NEW.meeting IS NOT NULL AND NEW.deleted=0 AND NOT EXISTS(SELECT 1 FROM meetings WHERE account=NEW.account AND id=NEW.meeting AND (state='active' OR NEW.id='m/'||NEW.meeting||'/head')) BEGIN SELECT RAISE(ABORT,'meeting_inactive'); END;
-- Offline restores own only the account rows recorded here. A failed or interrupted
-- import can clean its partial rows and retry the same completed snapshot.
CREATE TABLE restore_run(singleton INTEGER PRIMARY KEY CHECK(singleton=1),prefix TEXT NOT NULL);
CREATE TABLE restore_accounts(id TEXT PRIMARY KEY);
-- Batched orphan checks and trash draining resolve R2 keys through indexes.
CREATE INDEX objects_by_body ON objects(body_key);
CREATE INDEX rekey_pages_by_body ON rekey_pages(body_key);

-- Short-lived, session-bound permission for the separate rotation read budget.
CREATE TABLE rotation_reads(account TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE, session TEXT NOT NULL REFERENCES sessions(hash) ON DELETE CASCADE, epoch INTEGER NOT NULL, expires_at INTEGER NOT NULL);
