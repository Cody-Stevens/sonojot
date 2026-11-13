ALTER TABLE accounts ADD COLUMN auth_verifier TEXT;
ALTER TABLE accounts ADD COLUMN changes_floor INTEGER NOT NULL DEFAULT 0;
ALTER TABLE pairings ADD COLUMN commitment_a TEXT;
ALTER TABLE pairings ADD COLUMN public_key_a TEXT;
CREATE TABLE pending_resets(account TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,device TEXT NOT NULL,requested_at INTEGER NOT NULL,ready_at INTEGER NOT NULL,epoch INTEGER NOT NULL);
CREATE TABLE backup_rows(run TEXT NOT NULL,table_name TEXT NOT NULL,ordinal INTEGER NOT NULL,payload TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(run,table_name,ordinal));
