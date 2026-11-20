use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;

pub mod credentials;
pub mod folders;
pub mod forwards;
pub mod known_hosts;
pub mod sessions;
pub mod settings;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

pub async fn init_pool(db_path: &Path) -> crate::error::Result<SqlitePool> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Pass the path directly: a profile directory containing `?` or `#`
    // would otherwise be parsed as URL query / fragment syntax.
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
    heal_line_ending_checksums(&pool).await?;
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

#[cfg(test)]
pub async fn init_pool_from_pool(pool: &SqlitePool) -> crate::error::Result<()> {
    MIGRATOR.run(pool).await?;
    Ok(())
}

/// Silently repair stored migration checksums that differ from the embedded
/// ones only by line endings. `sqlx::migrate!` hashes raw bytes, so a
/// migration that lives as LF on disk in one build and CRLF in another
/// produces two different sha384s for the same SQL. When a DB populated
/// under the first build boots under the second, sqlx panics with
/// `VersionMismatch` — even though the SQL is semantically identical.
///
/// This is a one-shot heal per row: if the stored checksum matches either
/// the LF or CRLF rendering of the currently embedded SQL, rewrite it to
/// the embedded checksum. Anything that *isn't* a pure line-ending variant
/// is left alone so genuine tampering still trips the sqlx check.
async fn heal_line_ending_checksums(pool: &SqlitePool) -> crate::error::Result<()> {
    use sha2::{Digest, Sha384};

    // Fresh DBs don't have this table yet — nothing to heal.
    let table_exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await?;
    if table_exists.is_none() {
        return Ok(());
    }

    let rows = sqlx::query("SELECT version, checksum FROM _sqlx_migrations WHERE success = 1")
        .fetch_all(pool)
        .await?;

    for row in rows {
        let version: i64 = row.try_get("version")?;
        let stored: Vec<u8> = row.try_get("checksum")?;

        let Some(embedded) = MIGRATOR.iter().find(|m| m.version == version) else {
            continue;
        };
        if embedded.checksum.as_ref() == stored.as_slice() {
            continue;
        }

        let lf = embedded.sql.as_str().replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        let lf_hash = Sha384::digest(lf.as_bytes());
        let crlf_hash = Sha384::digest(crlf.as_bytes());

        let is_line_ending_variant =
            stored.as_slice() == lf_hash.as_slice() || stored.as_slice() == crlf_hash.as_slice();

        if is_line_ending_variant {
            sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
                .bind(embedded.checksum.as_ref())
                .bind(version)
                .execute(pool)
                .await?;
            tracing::info!(
                version,
                "healed migration checksum (line-ending variant of embedded SQL)"
            );
        }
        // Otherwise leave the row alone — MIGRATOR.run() will surface the
        // real mismatch, which is what we want for genuine edits.
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::AssertSqlSafe;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Apply every migration before `version` to `pool`; returns the
    /// migration `version` itself, for the test to run on its data.
    async fn migrate_up_to(pool: &SqlitePool, version: i64) -> &'static sqlx::migrate::Migration {
        let mut target = None;
        for m in MIGRATOR.iter() {
            if m.version < version {
                sqlx::raw_sql(AssertSqlSafe(m.sql.as_str()))
                    .execute(pool)
                    .await
                    .unwrap();
            } else if m.version == version {
                target = Some(m);
            }
        }
        target.expect("migration exists")
    }

    /// `20261101120000_session_owned_passwords` on data written by older
    /// builds: shared passwords are split per session, unused ones dropped,
    /// keys and passphrases untouched.
    #[tokio::test]
    async fn owned_passwords_migration_splits_shared_rows() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        let newer = migrate_up_to(&pool, 20261101120000).await;
        let q = |sql: &'static str| sqlx::query(sql);
        q(
            "INSERT INTO credentials (id, kind, label, nonce, ciphertext) VALUES \
           (1, 'password', 'shared', x'00', x'AA'), \
           (2, 'password', 'unused', x'00', x'BB'), \
           (3, 'private_key', 'key', x'00', x'CC'), \
           (4, 'key_passphrase', 'pp', x'00', x'DD'), \
           (5, 'password', 'solo', x'00', x'EE')",
        )
        .execute(&pool)
        .await
        .unwrap();
        q("INSERT INTO sessions (id, name, host, username, auth_type, credential_id, key_passphrase_credential_id) VALUES \
           (10, 'a', 'h', 'u', 'password', 1, NULL), \
           (11, 'b', 'h', 'u', 'password', 1, NULL), \
           (12, 'c', 'h', 'u', 'key', 3, 4), \
           (13, 'd', 'h', 'u', 'password', 5, NULL), \
           (14, 'e', 'h', 'u', 'agent', 1, NULL)")
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(AssertSqlSafe(newer.sql.as_str()))
            .execute(&pool)
            .await
            .unwrap();

        let sessions: Vec<(i64, Option<i64>, Option<i64>)> = sqlx::query_as(
            "SELECT id, credential_id, key_passphrase_credential_id FROM sessions ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let creds: Vec<(i64, String, Vec<u8>, Option<i64>)> =
            sqlx::query_as("SELECT id, kind, ciphertext, session_id FROM credentials ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        let cred = |id: Option<i64>| creds.iter().find(|c| Some(c.0) == id).cloned();
        // a keeps row 1, b gets a clone with the same secret, each owned.
        assert_eq!(sessions[0].1, Some(1));
        assert_eq!(cred(sessions[0].1).unwrap().3, Some(10));
        let b = cred(sessions[1].1).unwrap();
        assert_ne!(b.0, 1);
        assert_eq!(
            (b.1.as_str(), b.2.clone(), b.3),
            ("password", vec![0xAA], Some(11))
        );
        // c keeps its shared key and passphrase.
        assert_eq!((sessions[2].1, sessions[2].2), (Some(3), Some(4)));
        assert_eq!(cred(Some(3)).unwrap().3, None);
        assert_eq!(cred(Some(4)).unwrap().3, None);
        // d owns its solo password; e (agent) lost the stale reference.
        assert_eq!(cred(sessions[3].1).unwrap().3, Some(13));
        assert_eq!(sessions[4].1, None);
        // The unused password is gone; nothing else was dropped.
        assert!(cred(Some(2)).is_none());
        assert_eq!(creds.len(), 5);

        // Deleting a session cascades to its own password.
        q("DELETE FROM sessions WHERE id = 11")
            .execute(&pool)
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM credentials")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 4);
    }

    /// `20261120120000_key_passphrase_on_key`: each key takes the
    /// passphrase its first key session paired it with; folded passphrase
    /// rows go, others stay; no session names a passphrase any more.
    #[tokio::test]
    async fn key_passphrase_migration_folds_into_keys() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let m = migrate_up_to(&pool, 20261120120000).await;
        let q = |sql: &'static str| sqlx::query(sql);
        q(
            "INSERT INTO credentials (id, kind, label, nonce, ciphertext) VALUES \
           (1, 'private_key', 'k1', x'01', x'A1'), \
           (2, 'key_passphrase', 'p1', x'02', x'B1'), \
           (3, 'key_passphrase', 'p1-other', x'03', x'B2'), \
           (4, 'private_key', 'k2', x'04', x'A2'), \
           (5, 'key_passphrase', 'never used', x'05', x'B3'), \
           (6, 'private_key', 'k3', x'06', x'A3')",
        )
        .execute(&pool)
        .await
        .unwrap();
        // k1: two sessions with different passphrases (lowest id wins);
        // k2: no passphrase; k3: only a password session points at it.
        q("INSERT INTO sessions (id, name, host, username, auth_type, credential_id, key_passphrase_credential_id) VALUES \
           (10, 'a', 'h', 'u', 'key', 1, 2), \
           (11, 'b', 'h', 'u', 'key', 1, 3), \
           (12, 'c', 'h', 'u', 'key', 4, NULL), \
           (13, 'd', 'h', 'u', 'password', NULL, 2)")
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(AssertSqlSafe(m.sql.as_str()))
            .execute(&pool)
            .await
            .unwrap();

        type Row = (i64, String, Option<Vec<u8>>, Option<Vec<u8>>);
        let creds: Vec<Row> = sqlx::query_as(
            "SELECT id, kind, passphrase_nonce, passphrase_ciphertext FROM credentials ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let ids: Vec<i64> = creds.iter().map(|c| c.0).collect();
        assert_eq!(ids, vec![1, 3, 4, 5, 6], "only the folded passphrase went");
        assert_eq!(
            (creds[0].2.clone(), creds[0].3.clone()),
            (Some(vec![0x02]), Some(vec![0xB1]))
        );
        assert!(creds[2].2.is_none() && creds[2].3.is_none(), "k2 has none");
        assert!(creds[4].3.is_none(), "k3 has none");
        let named: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sessions WHERE key_passphrase_credential_id IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(named, 0);
        let keys: Vec<Option<i64>> =
            sqlx::query_scalar("SELECT credential_id FROM sessions ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            keys,
            vec![Some(1), Some(1), Some(4), None],
            "key refs untouched"
        );
    }
}
