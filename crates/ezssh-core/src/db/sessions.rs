use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, SqlitePool};

use crate::error::Result;

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String, // 'password' | 'key' | 'agent'
    pub credential_id: Option<i64>,
    pub key_passphrase_credential_id: Option<i64>,
    pub color: Option<String>,
    pub sort: i64,
    // Terminal + advanced session settings (v0.5).
    pub initial_command: Option<String>,
    pub scrollback_lines: i64,
    pub font_size: i64,
    /// CSS-like font stack (the format ezTerm stored; see
    /// `ezssh_term::fonts`). Empty string means "use the app default".
    pub font_family: String,
    pub cursor_style: String, // 'block' | 'bar' | 'underline'
    pub compression: i64,     // 0 | 1 (SQLite has no bool)
    pub keepalive_secs: i64,
    pub connect_timeout_secs: i64,
    /// 'ssh' | 'wsl' | 'local'. For wsl/local, `host` re-purposes as the
    /// distro name or shell program, and `username` re-purposes as the
    /// wsl user or starting directory. Auth fields are forced to
    /// agent/NULL for non-ssh rows by the command-layer validator.
    pub session_kind: String,
    /// 0/1 — SSH-only. When 1, the connect flow asks russh for X11
    /// forwarding and starts a local VcXsrv display to receive the
    /// forwarded GUI apps. Ignored for wsl/local rows.
    pub forward_x11: i64,
    /// Optional starting directory. For WSL rows it's passed to
    /// `wsl.exe --cd`; empty falls back to `~`. For SSH rows it's
    /// written as `cd <value>\n` into the interactive shell right
    /// after shell_request; empty leaves the shell at the remote
    /// default ($HOME on most servers). Local (cmd/pwsh) rows ignore
    /// this — they use `username` as the Windows cwd.
    pub starting_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SessionInput {
    pub folder_id: Option<i64>,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    pub credential_id: Option<i64>,
    pub key_passphrase_credential_id: Option<i64>,
    pub color: Option<String>,
    pub initial_command: Option<String>,
    pub scrollback_lines: i64,
    pub font_size: i64,
    #[serde(default)]
    pub font_family: String,
    pub cursor_style: String,
    pub compression: i64,
    pub keepalive_secs: i64,
    pub connect_timeout_secs: i64,
    /// Environment variables sent via `channel.set_env` at connect time.
    /// Sent separately from the sessions row; see `session_env` table.
    pub env: Vec<EnvPair>,
    #[serde(default = "default_session_kind")]
    pub session_kind: String,
    #[serde(default)]
    pub forward_x11: i64,
    #[serde(default)]
    pub starting_dir: Option<String>,
    /// New password for a `password` session, typed in the session editor.
    /// Passwords belong to their session (a credentials row with
    /// `session_id` set), so there is no shared password to pick. `None`
    /// or empty keeps the stored one. Ignored for other auth types; the
    /// API layer encrypts it into a [`PasswordOp`].
    #[serde(default)]
    pub password: Option<crate::secret::Secret>,
}

/// What to do with a session's own password row when it is saved.
#[derive(Debug)]
pub enum PasswordOp {
    /// Keep the stored one.
    Keep,
    /// Store this ciphertext, replacing any previous password.
    Set { nonce: Vec<u8>, ciphertext: Vec<u8> },
}

fn default_session_kind() -> String {
    "ssh".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnvPair {
    pub key: String,
    pub value: String,
}

const SELECT_COLS: &str = "id, folder_id, name, host, port, username, auth_type, \
credential_id, key_passphrase_credential_id, color, sort, \
initial_command, scrollback_lines, font_size, font_family, cursor_style, compression, \
keepalive_secs, connect_timeout_secs, session_kind, forward_x11, starting_dir";

pub async fn list(pool: &SqlitePool) -> Result<Vec<Session>> {
    let sql = format!("SELECT {SELECT_COLS} FROM sessions ORDER BY folder_id, sort, id");
    Ok(sqlx::query_as::<_, Session>(AssertSqlSafe(sql))
        .fetch_all(pool)
        .await?)
}

pub async fn get(pool: &SqlitePool, id: i64) -> Result<Session> {
    let sql = format!("SELECT {SELECT_COLS} FROM sessions WHERE id = ?");
    sqlx::query_as::<_, Session>(AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(crate::error::AppError::NotFound)
}

pub async fn env_get(pool: &SqlitePool, session_id: i64) -> Result<Vec<EnvPair>> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT key, value FROM session_env WHERE session_id = ? ORDER BY key",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(key, value)| EnvPair { key, value })
        .collect())
}

async fn env_replace(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_id: i64,
    env: &[EnvPair],
) -> Result<()> {
    // Takes an explicit transaction so callers control atomicity alongside
    // the `sessions` row write. Uses `&mut *tx` dereference so each `execute`
    // borrows the transaction for only the duration of that query, letting
    // the loop reuse it on the next iteration.
    sqlx::query("DELETE FROM session_env WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    for p in env {
        sqlx::query("INSERT INTO session_env (session_id, key, value) VALUES (?, ?, ?)")
            .bind(session_id)
            .bind(&p.key)
            .bind(&p.value)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

pub async fn create(pool: &SqlitePool, input: &SessionInput) -> Result<Session> {
    create_with(pool, input, &PasswordOp::Keep).await
}

/// [`create`] plus the session's own password, in one transaction.
pub async fn create_with(
    pool: &SqlitePool,
    input: &SessionInput,
    password: &PasswordOp,
) -> Result<Session> {
    let mut tx = pool.begin().await?;
    let id = sqlx::query(
        "INSERT INTO sessions (folder_id, name, host, port, username, auth_type, \
         credential_id, key_passphrase_credential_id, color, \
         initial_command, scrollback_lines, font_size, font_family, cursor_style, \
         compression, keepalive_secs, connect_timeout_secs, session_kind, \
         forward_x11, starting_dir) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.folder_id)
    .bind(&input.name)
    .bind(&input.host)
    .bind(input.port)
    .bind(&input.username)
    .bind(&input.auth_type)
    .bind(input.credential_id)
    .bind(input.key_passphrase_credential_id)
    .bind(&input.color)
    .bind(&input.initial_command)
    .bind(input.scrollback_lines)
    .bind(input.font_size)
    .bind(&input.font_family)
    .bind(&input.cursor_style)
    .bind(input.compression)
    .bind(input.keepalive_secs)
    .bind(input.connect_timeout_secs)
    .bind(&input.session_kind)
    .bind(input.forward_x11)
    .bind(&input.starting_dir)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    env_replace(&mut tx, id, &input.env).await?;
    bind_password(&mut tx, id, input, password).await?;
    tx.commit().await?;
    get(pool, id).await
}

pub async fn update(pool: &SqlitePool, id: i64, input: &SessionInput) -> Result<Session> {
    update_with(pool, id, input, &PasswordOp::Keep).await
}

/// [`update`] plus the session's own password, in one transaction.
pub async fn update_with(
    pool: &SqlitePool,
    id: i64,
    input: &SessionInput,
    password: &PasswordOp,
) -> Result<Session> {
    let mut tx = pool.begin().await?;
    let n = sqlx::query(
        "UPDATE sessions SET folder_id = ?, name = ?, host = ?, port = ?, username = ?, \
         auth_type = ?, credential_id = ?, key_passphrase_credential_id = ?, color = ?, \
         initial_command = ?, scrollback_lines = ?, font_size = ?, font_family = ?, cursor_style = ?, \
         compression = ?, keepalive_secs = ?, connect_timeout_secs = ?, \
         session_kind = ?, forward_x11 = ?, starting_dir = ? \
         WHERE id = ?",
    )
    .bind(input.folder_id)
    .bind(&input.name)
    .bind(&input.host)
    .bind(input.port)
    .bind(&input.username)
    .bind(&input.auth_type)
    .bind(input.credential_id)
    .bind(input.key_passphrase_credential_id)
    .bind(&input.color)
    .bind(&input.initial_command)
    .bind(input.scrollback_lines)
    .bind(input.font_size)
    .bind(&input.font_family)
    .bind(&input.cursor_style)
    .bind(input.compression)
    .bind(input.keepalive_secs)
    .bind(input.connect_timeout_secs)
    .bind(&input.session_kind)
    .bind(input.forward_x11)
    .bind(&input.starting_dir)
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(crate::error::AppError::NotFound);
    }
    env_replace(&mut tx, id, &input.env).await?;
    bind_password(&mut tx, id, input, password).await?;
    tx.commit().await?;
    get(pool, id).await
}

/// Point `sessions.credential_id` of a `password` session at its own row
/// (writing or keeping it), or drop the own row when the session
/// uses another auth type. Key and passphrase slots may only name shared
/// rows.
async fn bind_password(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: i64,
    input: &SessionInput,
    op: &PasswordOp,
) -> Result<()> {
    let owned: Option<i64> = sqlx::query_scalar("SELECT id FROM credentials WHERE session_id = ?")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
    if input.auth_type != "password" {
        for slot in [input.credential_id, input.key_passphrase_credential_id]
            .into_iter()
            .flatten()
        {
            let owner: Option<Option<i64>> =
                sqlx::query_scalar("SELECT session_id FROM credentials WHERE id = ?")
                    .bind(slot)
                    .fetch_optional(&mut **tx)
                    .await?;
            if matches!(owner, Some(Some(_))) {
                return Err(crate::error::AppError::Validation(
                    "that credential is a session password, not a shared key".into(),
                ));
            }
        }
        if let Some(o) = owned {
            tracing::debug!(
                session = id,
                credential = o,
                "sessions: auth type changed, dropping the session password"
            );
            sqlx::query("DELETE FROM credentials WHERE id = ?")
                .bind(o)
                .execute(&mut **tx)
                .await?;
        }
        return Ok(());
    }
    let bound = match op {
        PasswordOp::Set { nonce, ciphertext } => {
            Some(store_own(tx, id, &input.name, owned, nonce, ciphertext).await?)
        }
        // `credential_id` is ignored here: a password session only ever
        // uses its own row.
        PasswordOp::Keep => owned,
    };
    sqlx::query(
        "UPDATE sessions SET credential_id = ?, key_passphrase_credential_id = NULL WHERE id = ?",
    )
    .bind(bound)
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Write `nonce` / `ciphertext` as session `id`'s own password, replacing
/// the row `owned` if there is one. Returns the row id.
async fn store_own(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: i64,
    name: &str,
    owned: Option<i64>,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<i64> {
    if let Some(o) = owned {
        sqlx::query("UPDATE credentials SET nonce = ?, ciphertext = ?, label = ? WHERE id = ?")
            .bind(nonce)
            .bind(ciphertext)
            .bind(name)
            .bind(o)
            .execute(&mut **tx)
            .await?;
        return Ok(o);
    }
    Ok(sqlx::query("INSERT INTO credentials (kind, label, nonce, ciphertext, session_id) VALUES ('password', ?, ?, ?, ?)")
        .bind(name)
        .bind(nonce)
        .bind(ciphertext)
        .bind(id)
        .execute(&mut **tx)
        .await?
        .last_insert_rowid())
}

/// Delete a session with its own password (explicitly, not through the FK
/// cascade, as `vault::reset` does).
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM credentials WHERE session_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn mv(pool: &SqlitePool, id: i64, folder_id: Option<i64>, sort: i64) -> Result<()> {
    sqlx::query("UPDATE sessions SET folder_id = ?, sort = ? WHERE id = ?")
        .bind(folder_id)
        .bind(sort)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Renumber all sessions in `folder_id` according to the order in
/// `ids_in_order`. Each position gets a `sort` value of `position * 10`
/// so there's room to insert between items later if we ever switch
/// back to gap-based ordering. Sessions in `folder_id` that aren't in
/// `ids_in_order` are left untouched — the caller guarantees it passes
/// the complete sibling set.
pub async fn reorder(
    pool: &SqlitePool,
    folder_id: Option<i64>,
    ids_in_order: &[i64],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    for (idx, id) in ids_in_order.iter().enumerate() {
        let sort = (idx as i64) * 10;
        sqlx::query("UPDATE sessions SET folder_id = ?, sort = ? WHERE id = ?")
            .bind(folder_id)
            .bind(sort)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Columns copied verbatim from the source row. Everything in the table
/// except `id` (auto), `name` (suffixed) and `sort` (left at its default so
/// the copy lands at the end of its folder, like `create`).
const DUPLICATE_COLS: &str = "folder_id, host, port, username, auth_type, \
credential_id, key_passphrase_credential_id, color, \
initial_command, scrollback_lines, font_size, font_family, cursor_style, compression, \
keepalive_secs, connect_timeout_secs, session_kind, forward_x11, starting_dir";

/// Copy a session and its env rows server-side in one transaction — no
/// read-modify-write round trip through Rust, and no window where the
/// source can change (or vanish) between the read and the insert.
pub async fn duplicate(pool: &SqlitePool, id: i64) -> Result<Session> {
    let mut tx = pool.begin().await?;
    let sql = format!(
        "INSERT INTO sessions (name, {DUPLICATE_COLS}) \
         SELECT name || ' (copy)', {DUPLICATE_COLS} FROM sessions WHERE id = ?"
    );
    let inserted = sqlx::query(AssertSqlSafe(sql))
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if inserted.rows_affected() == 0 {
        return Err(crate::error::AppError::NotFound);
    }
    let new_id = inserted.last_insert_rowid();
    // The copy gets its own copy of the password row.
    let cloned = sqlx::query(
        "INSERT INTO credentials (kind, label, nonce, ciphertext, session_id) \
         SELECT kind, label || ' (copy)', nonce, ciphertext, ? FROM credentials WHERE session_id = ?",
    )
    .bind(new_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if cloned.rows_affected() == 1 {
        sqlx::query("UPDATE sessions SET credential_id = ? WHERE id = ?")
            .bind(cloned.last_insert_rowid())
            .bind(new_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "INSERT INTO session_env (session_id, key, value) \
         SELECT ?, key, value FROM session_env WHERE session_id = ?",
    )
    .bind(new_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO session_forwards \
         (session_id, name, kind, bind_addr, bind_port, dest_addr, dest_port, auto_start, sort) \
         SELECT ?, name, kind, bind_addr, bind_port, dest_addr, dest_port, auto_start, sort \
         FROM session_forwards WHERE session_id = ? ORDER BY sort, id",
    )
    .bind(new_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get(pool, new_id).await
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

    fn input(name: &str) -> SessionInput {
        SessionInput {
            folder_id: None,
            name: name.into(),
            host: "example.com".into(),
            port: 22,
            username: "root".into(),
            auth_type: "agent".into(),
            credential_id: None,
            key_passphrase_credential_id: None,
            color: None,
            initial_command: None,
            scrollback_lines: 5000,
            font_size: 13,
            font_family: String::new(),
            cursor_style: "block".into(),
            compression: 0,
            keepalive_secs: 0,
            connect_timeout_secs: 15,
            env: Vec::new(),
            session_kind: "ssh".into(),
            forward_x11: 0,
            starting_dir: None,
            password: None,
        }
    }

    #[tokio::test]
    async fn duplicate_copies_fields_and_env_atomically() {
        let p = pool().await;
        let mut src = input("alpha");
        src.port = 2222;
        src.color = Some("#ff0000".into());
        src.env = vec![
            EnvPair {
                key: "LANG".into(),
                value: "C.UTF-8".into(),
            },
            EnvPair {
                key: "TERM".into(),
                value: "xterm-256color".into(),
            },
        ];
        let s = create(&p, &src).await.unwrap();

        let dupe = duplicate(&p, s.id).await.unwrap();
        assert_ne!(dupe.id, s.id);
        assert_eq!(dupe.name, "alpha (copy)");
        assert_eq!(dupe.port, 2222);
        assert_eq!(dupe.color.as_deref(), Some("#ff0000"));
        let mut env = env_get(&p, dupe.id).await.unwrap();
        env.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(env.len(), 2);
        assert_eq!(env[0].key, "LANG");
        assert_eq!(env[1].value, "xterm-256color");
        // Source env is untouched (not moved).
        assert_eq!(env_get(&p, s.id).await.unwrap().len(), 2);

        // Saved port forwards come along too (#158 review).
        let fwd = crate::db::forwards::ForwardInput {
            name: "db".into(),
            kind: "local".into(),
            bind_addr: "127.0.0.1".into(),
            bind_port: 15432,
            dest_addr: "db.internal".into(),
            dest_port: 5432,
            auto_start: 1,
        };
        crate::db::forwards::create(&p, s.id, &fwd).await.unwrap();
        let dupe2 = duplicate(&p, s.id).await.unwrap();
        let copied = crate::db::forwards::list_for_session(&p, dupe2.id)
            .await
            .unwrap();
        assert_eq!(copied.len(), 1);
        assert_eq!(copied[0].bind_port, 15432);
        assert_eq!(copied[0].dest_addr, "db.internal");
        assert_eq!(
            crate::db::forwards::list_for_session(&p, s.id)
                .await
                .unwrap()
                .len(),
            1
        );

        // Unknown source id surfaces as NotFound with nothing inserted.
        assert!(matches!(
            duplicate(&p, 999_999).await,
            Err(crate::error::AppError::NotFound)
        ));
        assert_eq!(list(&p).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn crud() {
        let p = pool().await;
        let s = create(&p, &input("alpha")).await.unwrap();
        assert_eq!(list(&p).await.unwrap().len(), 1);
        let dupe = duplicate(&p, s.id).await.unwrap();
        assert_eq!(dupe.name, "alpha (copy)");
        let mut upd = input("alpha2");
        upd.port = 2222;
        upd.keepalive_secs = 60;
        upd.env = vec![EnvPair {
            key: "LANG".into(),
            value: "C.UTF-8".into(),
        }];
        update(&p, s.id, &upd).await.unwrap();
        let got = get(&p, s.id).await.unwrap();
        assert_eq!(got.port, 2222);
        assert_eq!(got.keepalive_secs, 60);
        let env = env_get(&p, s.id).await.unwrap();
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].key, "LANG");
        delete(&p, s.id).await.unwrap();
        assert_eq!(list(&p).await.unwrap().len(), 1); // duplicate remains
        // env cascade: deleting the source session clears its env rows, the
        // duplicate's env row count is validated via env_get above.
        let env_after = env_get(&p, s.id).await.unwrap();
        assert!(env_after.is_empty());
    }
}
