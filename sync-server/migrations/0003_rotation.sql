ALTER TABLE accounts ADD COLUMN rotation TEXT;
ALTER TABLE auth_requests ADD COLUMN link_account TEXT;
ALTER TABLE auth_requests ADD COLUMN link_session TEXT;
CREATE TABLE notifications(id TEXT PRIMARY KEY,account TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,kind TEXT NOT NULL,created_at INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0);
