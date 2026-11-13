-- Pending authentication is short-lived and never included in backups.
ALTER TABLE auth_requests ADD COLUMN intent TEXT NOT NULL DEFAULT 'sign-in';
ALTER TABLE auth_requests ADD COLUMN browser_origin TEXT;
ALTER TABLE auth_requests ADD COLUMN email TEXT;
ALTER TABLE auth_requests ADD COLUMN code_hash TEXT;
ALTER TABLE auth_requests ADD COLUMN code_tries INTEGER NOT NULL DEFAULT 0;
ALTER TABLE auth_requests ADD COLUMN code_started INTEGER NOT NULL DEFAULT 0;
ALTER TABLE auth_requests ADD COLUMN radar_attempt TEXT;
