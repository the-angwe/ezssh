
use crate::api::require_unlocked;
use crate::error::Result;
use crate::state::AppState;
use crate::xserver::{self, XServerStatus};

pub async fn xserver_status(state: &AppState) -> Result<XServerStatus> {
    require_unlocked(state).await?;
    Ok(state.xserver.status().await)
}

/// Download + silent-install VcXsrv into the user's per-user data dir.
/// Returns the absolute path to the installed `vcxsrv.exe` on success.
/// Triggered by the "Install VcXsrv" button in the `XServerMissingDialog`
/// when a session enables X11 forwarding but no VcXsrv is available.
pub async fn xserver_install(state: &AppState) -> Result<String> {
    require_unlocked(state).await?;
    let path = xserver::install_vcxsrv().await?;
    Ok(path.to_string_lossy().into_owned())
}
