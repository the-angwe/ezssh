use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::Result;

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CredentialMeta {
    pub id: i64,
    pub kind: String, // 'password' | 'private_key' | 'key_passphrase'
    pub label: String,
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
    /// Sessions referencing this credential via `credential_id` or
    /// `key_passphrase_credential_id` — lets the UI warn before delete.
    pub used_by: Vec<CredentialSessionRef>,
}

#[derive(sqlx::FromRow)]
pub(crate) struct CredentialRow {
    pub kind: String,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// One row of [`list_for_export`].
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ExportRow {
    pub id: i64,
    pub kind: String,
    pub label: String,
    /// Owning session of a session password; `None` for shared rows.
    pub session_id: Option<i64>,
}

/// The shared credentials (private keys, key passphrases). Session
/// passwords belong to their session and are not listed.
pub async fn list(pool: &SqlitePool) -> Result<Vec<CredentialMeta>> {
    Ok(sqlx::query_as::<_, CredentialMeta>(
        "SELECT id, kind, label FROM credentials WHERE session_id IS NULL ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn list_detailed(pool: &SqlitePool) -> Result<Vec<CredentialDetail>> {
    let creds = sqlx::query_as::<_, (i64, String, String, String)>(
        "SELECT id, kind, label, created_at FROM credentials WHERE session_id IS NULL ORDER BY id DESC",
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
        .map(|(id, kind, label, created_at)| CredentialDetail {
            id,
            kind,
            label,
            created_at,
            used_by: used.remove(&id).unwrap_or_default(),
        })
        .collect())
}

/// The Tauri picker's list: the shared rows plus every session password,
/// labelled with its session's name. The old webview selects a session's
/// password by id and drops a selection it cannot find, so it must see
/// them; saving a session that names another session's password copies
/// it (see `db::sessions`).
pub async fn list_with_session_passwords(pool: &SqlitePool) -> Result<Vec<CredentialMeta>> {
    Ok(sqlx::query_as::<_, CredentialMeta>(
        "SELECT c.id, c.kind, COALESCE(s.name, c.label) AS label FROM credentials c \
         LEFT JOIN sessions s ON s.id = c.session_id ORDER BY c.id DESC",
    )
    .fetch_all(pool)
    .await?)
}

/// Every row, session passwords included (backups and sync bundles).
pub async fn list_for_export(pool: &SqlitePool) -> Result<Vec<ExportRow>> {
    Ok(sqlx::query_as::<_, ExportRow>("SELECT id, kind, label, session_id FROM credentials ORDER BY id")
        .fetch_all(pool)
        .await?)
}

pub(crate) async fn insert(
    pool: &SqlitePool,
    kind: &str,
    label: &str,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<i64> {
    let id = sqlx::query(
        "INSERT INTO credentials (kind, label, nonce, ciphertext) VALUES (?, ?, ?, ?)",
    )
    .bind(kind)
    .bind(label)
    .bind(nonce)
    .bind(ciphertext)
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

pub(crate) async fn get(pool: &SqlitePool, id: i64) -> Result<CredentialRow> {
    sqlx::query_as::<_, CredentialRow>(
        "SELECT kind, nonce, ciphertext FROM credentials WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(crate::error::AppError::NotFound)
}

/// Update a credential's label and/or its secret independently; `None`
/// keeps the existing column values (COALESCE against NULL binds).
pub(crate) async fn update(
    pool: &SqlitePool,
    id: i64,
    label: Option<&str>,
    secret: Option<(&[u8], &[u8])>,
) -> Result<()> {
    let (nonce, ciphertext) = match secret {
        Some((n, c)) => (Some(n), Some(c)),
        None => (None, None),
    };
    let n = sqlx::query(
        "UPDATE credentials SET label = COALESCE(?, label), \
         nonce = COALESCE(?, nonce), ciphertext = COALESCE(?, ciphertext) \
         WHERE id = ? AND session_id IS NULL",
    )
    .bind(label)
    .bind(nonce)
    .bind(ciphertext)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
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
        let id = insert(&p, "password", "prod-db", &[0u8; 12], &[1, 2, 3])
            .await
            .unwrap();
        assert_eq!(list(&p).await.unwrap().len(), 1);
        let row = get(&p, id).await.unwrap();
        assert_eq!(row.nonce.len(), 12);
        delete(&p, id).await.unwrap();
        assert_eq!(list(&p).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn update_label_and_secret_independently() {
        let p = pool().await;
        let id = insert(&p, "private_key", "old-name", &[0u8; 12], &[1, 2, 3])
            .await
            .unwrap();

        // Rename only — secret untouched.
        update(&p, id, Some("new-name"), None).await.unwrap();
        let detail = &list_detailed(&p).await.unwrap()[0];
        assert_eq!(detail.label, "new-name");
        assert!(!detail.created_at.is_empty());
        assert_eq!(get(&p, id).await.unwrap().ciphertext, vec![1, 2, 3]);

        // Rotate only — label untouched.
        update(&p, id, None, Some((&[9u8; 12], &[4, 5, 6])))
            .await
            .unwrap();
        let row = get(&p, id).await.unwrap();
        assert_eq!(row.nonce, vec![9u8; 12]);
        assert_eq!(row.ciphertext, vec![4, 5, 6]);
        assert_eq!(list(&p).await.unwrap()[0].label, "new-name");

        // Both at once.
        update(&p, id, Some("both"), Some((&[7u8; 12], &[8])))
            .await
            .unwrap();
        assert_eq!(list(&p).await.unwrap()[0].label, "both");
        assert_eq!(get(&p, id).await.unwrap().ciphertext, vec![8]);

        assert!(update(&p, 9999, Some("x"), None).await.is_err());
        assert!(update(&p, 9999, None, Some((&[0u8; 12], &[1]))).await.is_err());
    }

    #[tokio::test]
    async fn list_detailed_reports_session_usage() {
        let p = pool().await;
        let key = insert(&p, "private_key", "key", &[0u8; 12], &[1]).await.unwrap();
        let pp = insert(&p, "key_passphrase", "pp", &[0u8; 12], &[2]).await.unwrap();
        let unused = insert(&p, "password", "pw", &[0u8; 12], &[3]).await.unwrap();

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

        let key_usage: Vec<&str> =
            by_id(key).used_by.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(key_usage, vec!["box-a", "box-b"]);
        assert_eq!(by_id(pp).used_by.len(), 1);
        assert!(by_id(unused).used_by.is_empty());
    }
}
