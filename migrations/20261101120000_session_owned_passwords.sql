-- Passwords belong to their session instead of a shared credentials list.
--
-- A row with `session_id` set is owned by that session: it is created,
-- replaced and deleted together with the session and never shows up in
-- the credentials manager. Private keys and key passphrases stay shared
-- (`session_id` NULL).
--
-- Existing data: every session that used a shared password gets its own
-- copy (the lowest session id keeps the original row, the others get a
-- clone with the same ciphertext), then password rows no session uses are
-- dropped. The app had no way to show or reuse those, so nothing usable is
-- lost. Sessions whose auth type is not 'password' but still pointed at a
-- password row (a stale attachment) lose that reference.
ALTER TABLE credentials
    ADD COLUMN session_id INTEGER REFERENCES sessions(id) ON DELETE CASCADE;

UPDATE credentials
SET session_id = (
    SELECT MIN(s.id) FROM sessions s
    WHERE s.credential_id = credentials.id AND s.auth_type = 'password'
)
WHERE kind = 'password';

INSERT INTO credentials (kind, label, nonce, ciphertext, created_at, session_id)
SELECT c.kind, c.label, c.nonce, c.ciphertext, c.created_at, s.id
FROM sessions s
JOIN credentials c ON c.id = s.credential_id
WHERE c.kind = 'password'
  AND s.auth_type = 'password'
  AND c.session_id IS NOT NULL
  AND s.id <> c.session_id;

UPDATE sessions
SET credential_id = (
    SELECT c.id FROM credentials c
    WHERE c.session_id = sessions.id AND c.kind = 'password'
)
WHERE credential_id IN (SELECT id FROM credentials WHERE kind = 'password');

-- A key passphrase slot pointing at a password row is stale too.
UPDATE sessions
SET key_passphrase_credential_id = NULL
WHERE key_passphrase_credential_id IN (SELECT id FROM credentials WHERE kind = 'password');

DELETE FROM credentials WHERE kind = 'password' AND session_id IS NULL;

-- One owned secret per session.
CREATE UNIQUE INDEX idx_credentials_session ON credentials(session_id)
    WHERE session_id IS NOT NULL;
