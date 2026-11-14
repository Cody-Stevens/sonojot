-- Plain facts for a notice, such as the platform of a computer that joined.
ALTER TABLE notifications ADD COLUMN detail TEXT;
-- When the last send attempt started. Retries wait longer after each attempt,
-- and a send in progress is not picked up a second time.
ALTER TABLE notifications ADD COLUMN tried_at INTEGER;
