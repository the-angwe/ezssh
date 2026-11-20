-- A private key and its passphrase are one credential.
--
-- The passphrase is stored on the key row itself (`passphrase_nonce` /
-- `passphrase_ciphertext`, encrypted under the vault key like every other
-- secret; both NULL = the key has no passphrase). Sessions no longer pick
-- a passphrase: `sessions.key_passphrase_credential_id` stays for old
-- backups and is always NULL from here on.
--
-- Existing data: each private key takes the passphrase of the lowest-id
-- key session that paired the two (the ciphertext is copied as is: same
-- vault key, no associated data). Passphrase rows folded into a key are
-- dropped; one that no key took (never attached, or a second session
-- pairing the same key with a different passphrase) stays as an
-- unattached row the credentials manager lists for deletion.
ALTER TABLE credentials ADD COLUMN passphrase_nonce BLOB;
ALTER TABLE credentials ADD COLUMN passphrase_ciphertext BLOB;

UPDATE credentials
SET (passphrase_nonce, passphrase_ciphertext) = (
    SELECT p.nonce, p.ciphertext
    FROM sessions s
    JOIN credentials p ON p.id = s.key_passphrase_credential_id
    WHERE s.credential_id = credentials.id
      AND s.auth_type = 'key'
      AND p.kind = 'key_passphrase'
    ORDER BY s.id
    LIMIT 1
)
WHERE kind = 'private_key';

UPDATE sessions SET key_passphrase_credential_id = NULL;

DELETE FROM credentials
WHERE kind = 'key_passphrase'
  AND EXISTS (
      SELECT 1 FROM credentials k
      WHERE k.kind = 'private_key'
        AND k.passphrase_nonce = credentials.nonce
        AND k.passphrase_ciphertext = credentials.ciphertext
  );
