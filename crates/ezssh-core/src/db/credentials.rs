use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::Result;

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CredentialMeta {
    pub id: i64,
    /// 'password' | 'private_key' | 'key_passphrase' (the last only for
    /// rows left from before passphrases moved onto their key).
    pub kind: String,
    pub label: String,
    /// A private key stored together with its passphrase.
    #[serde(default)]
    pub has_passphrase: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialSessionRef {
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialDetail {
    pub id: i64,
    pub kind: String,
    pub label: String,
    pub created_at: String,
    /// A private key stored together with its passphrase.
    pub has_passphrase: bool,
    /// Sessions referencing this credential via `credential_id` or
    /// `key_passphrase_credential_id` — lets the UI warn before delete.
    pub used_by: Vec<CredentialSessionRef>,
}

#[derive(sqlx::FromRow)]
pub(crate) struct CredentialRow {
    pub kind: String,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    /// A private key's passphrase (both `None`: none stored).
    pub passphrase_nonce: Option<Vec<u8>>,
    pub passphrase_ciphertext: Option<Vec<u8>>,
}

/// What an update does to a private key's passphrase. `Set` carries
/// `(nonce, ciphertext)` already encrypted under the vault key.
#[derive(Clone, Copy, Debug)]
pub enum PassphraseOp<'a> {
    Keep,
    Set(&'a [u8], &'a [u8]),
    Clear,
}

/// One row of [`list_for_export`].
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ExportRow {
    pub id: i64,
    pub kind: String,
    pub label: String,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    /// Owning session of a session password; `None` for shared rows.
    pub session_id: Option<i64>,
    /// A private key's passphrase (both `None`: none stored).
    pub passphrase_nonce: Option<Vec<u8>>,
    pub passphrase_ciphertext: Option<Vec<u8>>,
}

/// The shared credentials (private keys with their passphrases; leftover
/// passphrase rows). Session passwords belong to their session and are
/// not listed.
pub async fn list(pool: &SqlitePool) -> Result<Vec<CredentialMeta>> {
    Ok(sqlx::query_as::<_, CredentialMeta>(
        "SELECT id, kind, label, passphrase_ciphertext IS NOT NULL AS has_passphrase \
         FROM credentials WHERE session_id IS NULL ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn list_detailed(pool: &SqlitePool) -> Result<Vec<CredentialDetail>> {
    let creds = sqlx::query_as::<_, (i64, String, String, String, bool)>(
        "SELECT id, kind, label, created_at, passphrase_ciphertext IS NOT NULL \
         FROM credentials WHERE session_id IS NULL ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await?;

    let refs = sqlx::query_as::<_, (i64, String, Option<i64>, Option<i64>)>(
        "SELECT id, name, credential_id, key_passphrase_credential_id FROM sessions \
         WHERE credential_id IS NOT NULL OR key_passphrase_credential_id IS NOT NULL \
         ORDER BY name COLLATE NOCASE, id",
    )
    .fetch_all(pool)
    .await?;

    let mut used: HashMap<i64, Vec<CredentialSessionRef>> = HashMap::new();
    for (sid, name, cred, passphrase) in refs {
        for cid in [cred, passphrase].into_iter().flatten() {
            let refs_for_cred = used.entry(cid).or_default();
            // Both columns can point at the same credential; list the
            // session once. Adjacent check suffices — rows are per-session.
            if refs_for_cred.last().map(|r| r.id) != Some(sid) {
                refs_for_cred.push(CredentialSessionRef {
                    id: sid,
                    name: name.clone(),
                });
            }
        }
    }

    Ok(creds
        .into_iter()
        .map(
            |(id, kind, label, created_at, has_passphrase)| CredentialDetail {
                id,
                kind,
                label,
                created_at,
                has_passphrase,
                used_by: used.remove(&id).unwrap_or_default(),
            },
        )
        .collect())
}

/// Every row, session passwords included (backups and sync bundles).
pub async fn list_for_export(pool: &SqlitePool) -> Result<Vec<ExportRow>> {
    Ok(sqlx::query_as::<_, ExportRow>(
        "SELECT id, kind, label, nonce, ciphertext, session_id, passphrase_nonce, \
         passphrase_ciphertext FROM credentials ORDER BY id",
    )
    .fetch_all(pool)
    .await?)
}

/// Store a credential row; a private key may carry its passphrase
/// (`(nonce, ciphertext)`) in the same row.
pub(crate) async fn insert(
    pool: &SqlitePool,
    kind: &str,
    label: &str,
    nonce: &[u8],
    ciphertext: &[u8],
    passphrase: Option<(&[u8], &[u8])>,
) -> Result<i64> {
    let (pn, pc) = passphrase.unzip();
    let id = sqlx::query(
        "INSERT INTO credentials (kind, label, nonce, ciphertext, passphrase_nonce, \
         passphrase_ciphertext) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(kind)
    .bind(label)
    .bind(nonce)
    .bind(ciphertext)
    .bind(pn)
    .bind(pc)
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

pub(crate) async fn get(pool: &SqlitePool, id: i64) -> Result<CredentialRow> {
    sqlx::query_as::<_, CredentialRow>(
        "SELECT kind, nonce, ciphertext, passphrase_nonce, passphrase_ciphertext \
         FROM credentials WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(crate::error::AppError::NotFound)
}

/// Update a credential's label, its secret and its passphrase
/// independently; `None` / [`PassphraseOp::Keep`] keep the stored values.
/// A passphrase can only be set on a private key.
pub(crate) async fn update(
    pool: &SqlitePool,
    id: i64,
    label: Option<&str>,
    secret: Option<(&[u8], &[u8])>,
    passphrase: PassphraseOp<'_>,
) -> Result<()> {
    let (nonce, ciphertext) = secret.unzip();
    let (pp_write, pp_nonce, pp_ct) = match passphrase {
        PassphraseOp::Keep => (false, None, None),
        PassphraseOp::Set(n, c) => (true, Some(n), Some(c)),
        PassphraseOp::Clear => (true, None, None),
    };
    let mut tx = pool.begin().await?;
    if pp_write {
        let kind: Option<String> =
            sqlx::query_scalar("SELECT kind FROM credentials WHERE id = ? AND session_id IS NULL")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        match kind.as_deref() {
            None => return Err(crate::error::AppError::NotFound),
            Some("private_key") => {}
            Some(other) => {
                return Err(crate::error::AppError::Validation(format!(
                    "a passphrase belongs to a private key, not a {other}"
                )));
            }
        }
    }
    let n = sqlx::query(
        "UPDATE credentials SET label = COALESCE(?, label), \
         nonce = COALESCE(?, nonce), ciphertext = COALESCE(?, ciphertext), \
         passphrase_nonce = CASE WHEN ? THEN ? ELSE passphrase_nonce END, \
         passphrase_ciphertext = CASE WHEN ? THEN ? ELSE passphrase_ciphertext END \
         WHERE id = ? AND session_id IS NULL",
    )
    .bind(label)
    .bind(nonce)
    .bind(ciphertext)
    .bind(pp_write)
    .bind(pp_nonce)
    .bind(pp_write)
    .bind(pp_ct)
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    if n == 0 {
        return Err(crate::error::AppError::NotFound);
    }
    Ok(())
}

/// Delete a shared credential. A session password is not reachable here
/// (it goes with its session): `NotFound`.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    let n = sqlx::query("DELETE FROM credentials WHERE id = ? AND session_id IS NULL")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(crate::error::AppError::NotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let p = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init_pool_from_pool(&p).await.unwrap();
        p
    }

    #[tokio::test]
    async fn insert_list_delete() {
        let p = pool().await;
        let id = insert(&p, "password", "prod-db", &[0u8; 12], &[1, 2, 3], None)
            .await
            .unwrap();
        assert_eq!(list(&p).await.unwrap().len(), 1);
        let row = get(&p, id).await.unwrap();
        assert_eq!(row.nonce.len(), 12);
        delete(&p, id).await.unwrap();
        assert_eq!(list(&p).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn passphrase_lives_on_the_key_row() {
        let p = pool().await;
        let id = insert(
            &p,
            "private_key",
            "k",
            &[0u8; 12],
            &[1],
            Some((&[3u8; 12], &[4])),
        )
        .await
        .unwrap();
        let row = get(&p, id).await.unwrap();
        assert_eq!(row.passphrase_ciphertext.as_deref(), Some(&[4u8][..]));
        assert!(list(&p).await.unwrap()[0].has_passphrase);
        assert!(list_detailed(&p).await.unwrap()[0].has_passphrase);

        // Rename and rotate the key: the passphrase stays.
        update(
            &p,
            id,
            Some("k2"),
            Some((&[5u8; 12], &[6])),
            PassphraseOp::Keep,
        )
        .await
        .unwrap();
        assert_eq!(
            get(&p, id).await.unwrap().passphrase_nonce,
            Some(vec![3u8; 12])
        );

        update(&p, id, None, None, PassphraseOp::Set(&[7u8; 12], &[8]))
            .await
            .unwrap();
        let row = get(&p, id).await.unwrap();
        assert_eq!(row.ciphertext, vec![6]);
        assert_eq!(row.passphrase_ciphertext, Some(vec![8]));

        update(&p, id, None, None, PassphraseOp::Clear)
            .await
            .unwrap();
        let row = get(&p, id).await.unwrap();
        assert!(row.passphrase_nonce.is_none() && row.passphrase_ciphertext.is_none());
        assert!(!list(&p).await.unwrap()[0].has_passphrase);

        // Only a private key takes one.
        let pw = insert(&p, "key_passphrase", "pp", &[0u8; 12], &[1], None)
            .await
            .unwrap();
        assert!(
            update(&p, pw, None, None, PassphraseOp::Set(&[0u8; 12], &[1]))
                .await
                .is_err()
        );
        assert!(
            update(&p, 9999, None, None, PassphraseOp::Clear)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn update_label_and_secret_independently() {
        let p = pool().await;
        let id = insert(&p, "private_key", "old-name", &[0u8; 12], &[1, 2, 3], None)
            .await
            .unwrap();

        // Rename only — secret untouched.
        update(&p, id, Some("new-name"), None, PassphraseOp::Keep)
            .await
            .unwrap();
        let detail = &list_detailed(&p).await.unwrap()[0];
        assert_eq!(detail.label, "new-name");
        assert!(!detail.created_at.is_empty());
        assert_eq!(get(&p, id).await.unwrap().ciphertext, vec![1, 2, 3]);

        // Rotate only — label untouched.
        update(
            &p,
            id,
            None,
            Some((&[9u8; 12], &[4, 5, 6])),
            PassphraseOp::Keep,
        )
        .await
        .unwrap();
        let row = get(&p, id).await.unwrap();
        assert_eq!(row.nonce, vec![9u8; 12]);
        assert_eq!(row.ciphertext, vec![4, 5, 6]);
        assert_eq!(list(&p).await.unwrap()[0].label, "new-name");

        // Both at once.
        update(
            &p,
            id,
            Some("both"),
            Some((&[7u8; 12], &[8])),
            PassphraseOp::Keep,
        )
        .await
        .unwrap();
        assert_eq!(list(&p).await.unwrap()[0].label, "both");
        assert_eq!(get(&p, id).await.unwrap().ciphertext, vec![8]);

        assert!(
            update(&p, 9999, Some("x"), None, PassphraseOp::Keep)
                .await
                .is_err()
        );
        assert!(
            update(&p, 9999, None, Some((&[0u8; 12], &[1])), PassphraseOp::Keep)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn list_detailed_reports_session_usage() {
        let p = pool().await;
        let key = insert(&p, "private_key", "key", &[0u8; 12], &[1], None)
            .await
            .unwrap();
        let pp = insert(&p, "key_passphrase", "pp", &[0u8; 12], &[2], None)
            .await
            .unwrap();
        let unused = insert(&p, "password", "pw", &[0u8; 12], &[3], None)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO sessions \
             (name, host, username, auth_type, credential_id, key_passphrase_credential_id) \
             VALUES ('box-a', 'a.example', 'root', 'key', ?, ?)",
        )
        .bind(key)
        .bind(pp)
        .execute(&p)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions (name, host, username, auth_type, credential_id) \
             VALUES ('box-b', 'b.example', 'root', 'key', ?)",
        )
        .bind(key)
        .execute(&p)
        .await
        .unwrap();

        let details = list_detailed(&p).await.unwrap();
        let by_id = |id: i64| details.iter().find(|d| d.id == id).unwrap();

        let key_usage: Vec<&str> = by_id(key).used_by.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(key_usage, vec!["box-a", "box-b"]);
        assert_eq!(by_id(pp).used_by.len(), 1);
        assert!(by_id(unused).used_by.is_empty());
    }
}
