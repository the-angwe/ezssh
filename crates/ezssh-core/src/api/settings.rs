
use crate::api::require_unlocked;
use crate::db::settings;
use crate::error::Result;
use crate::state::AppState;

/// Settings keys that may be read without unlocking the vault.
///
/// The unlock screen needs the active theme before the user authenticates.
/// Every other settings key is gated behind `require_unlocked`.
const PUBLIC_SETTINGS_KEYS: &[&str] = &["theme"];

pub async fn settings_get(state: &AppState, key: String) -> Result<Option<String>> {
    if !PUBLIC_SETTINGS_KEYS.contains(&key.as_str()) {
        require_unlocked(state).await?;
    }
    settings::get(&state.db, &key).await
}

/// Every stored key/value pair. Used once at unlock to hydrate the
/// app's settings instead of one round trip per key.
pub async fn settings_list(state: &AppState) -> Result<Vec<(String, String)>> {
    require_unlocked(state).await?;
    settings::list_all(&state.db).await
}

pub async fn settings_set(state: &AppState, key: String, value: String) -> Result<()> {
    require_unlocked(state).await?;
    settings::set(&state.db, &key, &value).await?;
    // Settings travel in the sync bundle like every other table.
    state.sync.trigger();
    Ok(())
}
