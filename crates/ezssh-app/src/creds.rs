//! The shared credentials list (private keys and key passphrases; session
//! passwords live on their session). The session dialog's pickers and the
//! credentials manager read it; whoever changes a credential calls
//! [`CredCache::reload`], which replaces `credential-events.ts`'s
//! broadcast: every picker sees the new list on the next frame.

use ezssh_core::api;
use ezssh_core::db::credentials::CredentialMeta;

use crate::core::Core;

#[derive(Default)]
pub struct CredCache {
    pub list: Vec<CredentialMeta>,
    /// Bumped on every successful reload, so views can notice a change
    /// (clamp a selection that was deleted, …).
    pub generation: u64,
    pub loading: bool,
}

impl CredCache {
    /// Credentials of one kind (`private_key` / `key_passphrase`).
    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a CredentialMeta> + 'a {
        self.list.iter().filter(move |c| c.kind == kind)
    }

    pub fn reload(&mut self, core: &Core) {
        self.loading = true;
        core.call(
            "credential_list",
            |st| async move { api::credentials::credential_list(&st).await },
            |app, r| {
                app.creds.loading = false;
                match r {
                    Ok(list) => {
                        tracing::debug!(n = list.len(), "creds: reloaded");
                        app.creds.list = list;
                        app.creds.generation += 1;
                    }
                    Err(e) => app.toasts.danger("Could not load credentials", e.to_string()),
                }
            },
        );
    }
}
