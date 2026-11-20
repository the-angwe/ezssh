use zeroize::Zeroizing;

use crate::api::require_unlocked;
use crate::db::credentials::{self, CredentialDetail, CredentialMeta, PassphraseOp};
use crate::error::{AppError, Result};
use crate::state::AppState;
use crate::vault;

// 64 KiB accommodates even large OpenSSH/PEM private keys while bounding
// memory and ciphertext blob size written to SQLite.
const MAX_PLAINTEXT_LEN: usize = 65_536;
const MAX_LABEL_LEN: usize = 256;

pub async fn credential_list(state: &AppState) -> Result<Vec<CredentialMeta>> {
    require_unlocked(state).await?;
    credentials::list(&state.db).await
}

/// What [`credential_update`] does to a private key's passphrase.
#[derive(Debug, Default)]
pub enum PassphraseChange {
    #[default]
    Keep,
    /// Store this passphrase (an empty one clears it).
    Set(String),
    Clear,
}

/// Store a credential. A private key takes its passphrase along
/// (`passphrase`: `None` or empty = the key has none); key passphrases are
/// not created on their own any more (they belong to their key).
pub async fn credential_create(
    state: &AppState,
    kind: String,
    label: String,
    plaintext: String,
    passphrase: Option<String>,
) -> Result<CredentialMeta> {
    require_unlocked(state).await?;
    // Wrap immediately so every return path — validation errors and a
    // failed encrypt alike — zeroizes the secrets on drop.
    let plaintext = Zeroizing::new(plaintext);
    let passphrase = passphrase.map(Zeroizing::new).filter(|p| !p.is_empty());
    if !matches!(kind.as_str(), "password" | "private_key") {
        return Err(AppError::Validation("invalid kind".into()));
    }
    if passphrase.is_some() && kind != "private_key" {
        return Err(AppError::Validation(
            "only a private key takes a passphrase".into(),
        ));
    }
    if passphrase
        .as_ref()
        .is_some_and(|p| p.len() > MAX_PLAINTEXT_LEN)
    {
        return Err(AppError::Validation("passphrase too long".into()));
    }
    if label.trim().is_empty() {
        return Err(AppError::Validation("label required".into()));
    }
    if label.len() > MAX_LABEL_LEN {
        return Err(AppError::Validation("label too long".into()));
    }
    if plaintext.len() > MAX_PLAINTEXT_LEN {
        return Err(AppError::Validation("secret too long".into()));
    }

    // Scope the vault read guard so it is dropped before the DB insert.
    let ((nonce, ct), pp) = {
        let vault_state = state.vault.read().await;
        let key = vault::encrypt_with(&vault_state, plaintext.as_bytes())?;
        let pp = passphrase
            .as_ref()
            .map(|p| vault::encrypt_with(&vault_state, p.as_bytes()))
            .transpose()?;
        (key, pp)
    };

    let id = credentials::insert(
        &state.db,
        &kind,
        label.trim(),
        &nonce,
        &ct,
        pp.as_ref().map(|(n, c)| (n.as_slice(), c.as_slice())),
    )
    .await?;
    tracing::debug!(id, kind = %kind, has_passphrase = pp.is_some(), "credentials: created");
    state.sync.trigger();
    Ok(CredentialMeta {
        id,
        kind,
        label: label.trim().into(),
        has_passphrase: pp.is_some(),
    })
}

pub async fn credential_list_detailed(state: &AppState) -> Result<Vec<CredentialDetail>> {
    require_unlocked(state).await?;
    credentials::list_detailed(&state.db).await
}

/// Rename a credential, replace its secret and/or change a private key's
/// passphrase. Each field is independent: `label: None` rotates the secret
/// without touching the name (so a stale UI label can't silently revert a
/// rename), `plaintext: None` (or empty) keeps the secret, and
/// [`PassphraseChange::Keep`] keeps the passphrase.
pub async fn credential_update(
    state: &AppState,
    id: i64,
    label: Option<String>,
    plaintext: Option<String>,
    passphrase: PassphraseChange,
) -> Result<()> {
    require_unlocked(state).await?;
    // Wrap immediately so every return path zeroizes the secrets on drop.
    let plaintext = plaintext.map(Zeroizing::new);
    // None: keep; Some(None): clear; Some(Some(p)): store `p`.
    let passphrase: Option<Option<Zeroizing<String>>> = match passphrase {
        PassphraseChange::Keep => None,
        PassphraseChange::Clear => Some(None),
        PassphraseChange::Set(p) => Some(Some(Zeroizing::new(p)).filter(|p| !p.is_empty())),
    };
    if let Some(Some(p)) = &passphrase
        && p.len() > MAX_PLAINTEXT_LEN
    {
        return Err(AppError::Validation("passphrase too long".into()));
    }

    if let Some(l) = &label {
        if l.trim().is_empty() {
            return Err(AppError::Validation("label required".into()));
        }
        if l.len() > MAX_LABEL_LEN {
            return Err(AppError::Validation("label too long".into()));
        }
    }

    let secret = match &plaintext {
        Some(pt) if !pt.is_empty() => {
            if pt.len() > MAX_PLAINTEXT_LEN {
                return Err(AppError::Validation("secret too long".into()));
            }
            // Scope the vault read guard so it is dropped before the DB write.
            let vault_state = state.vault.read().await;
            Some(vault::encrypt_with(&vault_state, pt.as_bytes())?)
        }
        _ => None,
    };
    let pp = match &passphrase {
        Some(Some(p)) => {
            let vault_state = state.vault.read().await;
            Some(vault::encrypt_with(&vault_state, p.as_bytes())?)
        }
        _ => None,
    };
    let (pp_op, pp_what) = match (&passphrase, &pp) {
        (Some(Some(_)), Some((n, c))) => (PassphraseOp::Set(n, c), "set"),
        (Some(_), _) => (PassphraseOp::Clear, "clear"),
        (None, _) => (PassphraseOp::Keep, "keep"),
    };
    if label.is_none() && secret.is_none() && passphrase.is_none() {
        return Err(AppError::Validation("nothing to update".into()));
    }
    tracing::debug!(
        id,
        label = label.is_some(),
        secret = secret.is_some(),
        passphrase = pp_what,
        "credentials: update"
    );

    credentials::update(
        &state.db,
        id,
        label.as_deref().map(str::trim),
        secret.as_ref().map(|(n, c)| (n.as_slice(), c.as_slice())),
        pp_op,
    )
    .await?;
    state.sync.trigger();
    Ok(())
}

pub async fn credential_delete(state: &AppState, id: i64) -> Result<()> {
    require_unlocked(state).await?;
    credentials::delete(&state.db, id).await?;
    state.sync.trigger();
    Ok(())
}
