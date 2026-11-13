-- Staging defaults are deliberately small; test/allowlisted accounts opt into 100 GB.
UPDATE accounts SET meeting_quota=2000000000 WHERE meeting_quota=100000000000 AND is_test=0;
CREATE TRIGGER meeting_default_quota AFTER INSERT ON accounts WHEN NEW.is_test=0 AND NEW.meeting_quota=100000000000 BEGIN UPDATE accounts SET meeting_quota=2000000000 WHERE id=NEW.id; END;
ALTER TABLE meetings ADD COLUMN refs_version INTEGER NOT NULL DEFAULT -1;
ALTER TABLE meetings ADD COLUMN refs_head INTEGER NOT NULL DEFAULT 0;
ALTER TABLE meetings ADD COLUMN refs_token TEXT NOT NULL DEFAULT '';
CREATE TABLE meeting_ref_runs(account TEXT NOT NULL,meeting TEXT NOT NULL,token TEXT NOT NULL,version INTEGER NOT NULL,head_rev INTEGER NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(account,meeting,token),FOREIGN KEY(account,meeting) REFERENCES meetings(account,id) ON DELETE CASCADE);
CREATE TABLE meeting_refs(account TEXT NOT NULL,meeting TEXT NOT NULL,token TEXT NOT NULL,object TEXT NOT NULL,PRIMARY KEY(account,meeting,token,object),FOREIGN KEY(account,meeting,token) REFERENCES meeting_ref_runs(account,meeting,token) ON DELETE CASCADE);
CREATE TABLE meeting_gc(account TEXT NOT NULL,meeting TEXT NOT NULL,object TEXT NOT NULL,purge_at INTEGER NOT NULL,PRIMARY KEY(account,object),FOREIGN KEY(account,meeting) REFERENCES meetings(account,id) ON DELETE CASCADE);
CREATE INDEX meeting_gc_due ON meeting_gc(purge_at);
CREATE TABLE meeting_live_refs(account TEXT NOT NULL,meeting TEXT NOT NULL,object TEXT NOT NULL,PRIMARY KEY(account,meeting,object),FOREIGN KEY(account,meeting) REFERENCES meetings(account,id) ON DELETE CASCADE);
CREATE TABLE meeting_upload_bytes(account TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,day INTEGER NOT NULL,bytes INTEGER NOT NULL);
CREATE TABLE rekey_pages(account TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,epoch INTEGER NOT NULL,nonce TEXT NOT NULL,page INTEGER NOT NULL,body_key TEXT NOT NULL,size INTEGER NOT NULL,expires_at INTEGER NOT NULL,PRIMARY KEY(account,epoch,nonce,page));
CREATE TABLE meeting_writes(account TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,object TEXT NOT NULL,previous INTEGER NOT NULL CHECK(previous>=0),epoch INTEGER NOT NULL,verifier TEXT NOT NULL);
CREATE TRIGGER meeting_write_guard BEFORE INSERT ON meeting_writes WHEN NOT EXISTS(SELECT 1 FROM accounts WHERE id=NEW.account AND epoch=NEW.epoch AND auth_verifier=NEW.verifier) OR COALESCE((SELECT rev FROM objects WHERE account=NEW.account AND id=NEW.object),0)!=NEW.previous BEGIN SELECT RAISE(ABORT,'write_changed'); END;
CREATE TRIGGER meeting_partial_trash BEFORE UPDATE ON objects WHEN NEW.deleted=4 AND OLD.body_key!='' BEGIN INSERT OR REPLACE INTO trash VALUES(OLD.body_key,OLD.account,NEW.updated_at); END;
-- An evicted meeting can still receive an authenticated deletion head; its
-- non-active state prevents that head from resurrecting content on clients.
DROP TRIGGER meeting_insert_guard;
DROP TRIGGER meeting_update_guard;
CREATE TRIGGER meeting_insert_guard BEFORE INSERT ON objects WHEN NEW.meeting IS NOT NULL AND NOT EXISTS(SELECT 1 FROM meetings WHERE account=NEW.account AND id=NEW.meeting AND (state='active' OR NEW.id='m/'||NEW.meeting||'/head')) BEGIN SELECT RAISE(ABORT,'meeting_inactive'); END;
CREATE TRIGGER meeting_update_guard BEFORE UPDATE ON objects WHEN NEW.meeting IS NOT NULL AND NEW.deleted=0 AND NOT EXISTS(SELECT 1 FROM meetings WHERE account=NEW.account AND id=NEW.meeting AND (state='active' OR NEW.id='m/'||NEW.meeting||'/head')) BEGIN SELECT RAISE(ABORT,'meeting_inactive'); END;
