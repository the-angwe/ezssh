//! Update check and download for the native app (the Tauri build keeps
//! tauri-plugin-updater in `src-tauri`). Not gated on the vault: updates
//! expose no vault data.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::events::Event;
use crate::state::AppState;
use crate::updater::{self, Config, Update};

/// Look for a newer release on the chosen channel.
pub async fn updater_check(cfg: &Config, pre_release: bool) -> Result<Option<Update>> {
    updater::check(cfg, pre_release).await
}

/// Download and verify `update`'s installer, reporting
/// [`Event::UpdateProgress`] at most every 100 ms (and at the end).
/// Returns the verified file for [`updater::install::install`].
pub async fn updater_download(state: &AppState, cfg: &Config, update: &Update) -> Result<PathBuf> {
    let events = state.events.clone();
    let mut last: Option<Instant> = None;
    let dir = updater::download_dir();
    updater::download(cfg, update, &dir, move |downloaded, total| {
        let finished = total == Some(downloaded);
        if finished || last.is_none_or(|t| t.elapsed() >= Duration::from_millis(100)) {
            last = Some(Instant::now());
            events.emit(Event::UpdateProgress { downloaded, total });
        }
    })
    .await
}
