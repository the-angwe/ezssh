//! The former Tauri commands as plain async functions on `AppState`.
//!
//! One module per command group, one function per command, with the same
//! names and parameters as the old `invoke()` surface, so the Tauri adapter
//! and the native shell call the same code.

pub mod backup;
pub mod credentials;
pub mod folders;
pub mod forwards;
pub mod import;
pub mod local;
pub mod sessions;
pub mod settings;
pub mod sftp;
pub mod ssh;
pub mod sync;
pub mod vault;
pub mod xserver;

/// Returns `Ok(())` only when the vault is currently unlocked.
///
/// Call this at the top of every invoke-handler command that reads or writes
/// vault-protected data. The vault/status commands (`vault_status`,
/// `vault_init`, `vault_unlock`, `vault_lock`) are exempt, as is the narrow
/// whitelist in `settings_get` that allows the unlock screen to load the
/// theme before the user authenticates.
pub async fn require_unlocked(
    state: &crate::state::AppState,
) -> crate::error::Result<()> {
    if !state.vault.read().await.is_unlocked() {
        return Err(crate::error::AppError::VaultLocked);
    }
    Ok(())
}
