//! `sync-dialog.tsx`: cloud sync to a local folder (Dropbox, OneDrive, …)
//! or an S3-compatible bucket. Status polls every 3 s while open; "Pull
//! from cloud" downloads the S3 bundle to a temp file and hands it to the
//! restore dialog.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Align, CornerRadius, Frame, Id, Layout, Margin, RichText, Stroke, Ui};
use ezssh_core::api;
use ezssh_core::api::sync::S3ConfigInput;
use ezssh_core::sync::SyncStatus;
use zeroize::Zeroizing;

use super::backup::{self as shared, FooterKind, Head, Part};
use super::restore::RestoreDialog;
use crate::app::App;
use crate::files::Pick;
use crate::form::{self, InputOpts};
use crate::icons::Icon;
use crate::style::{alpha, size, tokens};
use crate::widgets::{self, Tone};

/// The webview's `setInterval(refresh, 3000)`.
const POLL: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    #[default]
    Local,
    S3,
}

impl Kind {
    fn id(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::S3 => "s3",
        }
    }
}

pub struct SyncDialog {
    pub(crate) status: Option<SyncStatus>,
    /// The form was filled from the stored config. Done on open and after a
    /// save or disable, not on every poll, so a poll never overwrites what
    /// is being typed or flips the target radio back.
    seeded: bool,
    polling: bool,
    next_poll: Option<Instant>,
    pub(crate) kind: Kind,
    pub(crate) path: String,
    local_pass: Zeroizing<String>,
    local_pass2: Zeroizing<String>,
    s3_endpoint: String,
    s3_region: String,
    s3_bucket: String,
    s3_prefix: String,
    s3_access_key_id: String,
    s3_secret: Zeroizing<String>,
    s3_pass: Zeroizing<String>,
    s3_pass2: Zeroizing<String>,
    show: bool,
    pub(crate) busy: bool,
    pub(crate) err: Option<String>,
    pub(crate) msg: Option<String>,
}

impl Default for SyncDialog {
    fn default() -> Self {
        Self {
            status: None,
            seeded: false,
            polling: false,
            next_poll: None,
            kind: Kind::Local,
            path: String::new(),
            local_pass: Zeroizing::default(),
            local_pass2: Zeroizing::default(),
            s3_endpoint: String::new(),
            s3_region: "auto".into(),
            s3_bucket: String::new(),
            s3_prefix: String::new(),
            s3_access_key_id: String::new(),
            s3_secret: Zeroizing::default(),
            s3_pass: Zeroizing::default(),
            s3_pass2: Zeroizing::default(),
            show: false,
            busy: false,
            err: None,
            msg: None,
        }
    }
}

impl SyncDialog {
    fn enabled(&self) -> bool {
        self.status.as_ref().is_some_and(|s| s.kind != "none")
    }

    /// Take a fresh status; fill the form from it when not seeded yet.
    fn apply_status(&mut self, s: SyncStatus) {
        if !self.seeded {
            self.seeded = true;
            match s.kind.as_str() {
                "local" => self.kind = Kind::Local,
                "s3" => self.kind = Kind::S3,
                _ => {}
            }
            let fill = |dst: &mut String, src: &Option<String>| {
                if let Some(v) = src {
                    *dst = v.clone();
                }
            };
            fill(&mut self.path, &s.local_path);
            fill(&mut self.s3_endpoint, &s.s3_endpoint);
            fill(&mut self.s3_region, &s.s3_region);
            fill(&mut self.s3_bucket, &s.s3_bucket);
            fill(&mut self.s3_prefix, &s.s3_prefix);
            fill(&mut self.s3_access_key_id, &s.s3_access_key_id);
        }
        self.status = Some(s);
    }

    fn clear_secrets(&mut self) {
        for s in [&mut self.local_pass, &mut self.local_pass2, &mut self.s3_secret, &mut self.s3_pass, &mut self.s3_pass2] {
            *s = Zeroizing::default();
        }
    }
}

enum Act {
    Close,
    Browse,
    Save,
    Disable,
    Push,
    Pull,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut dlg) = app.dialogs.sync.take() else {
        return;
    };
    let now = Instant::now();
    if !dlg.polling && dlg.next_poll.is_none_or(|t| now >= t) {
        refresh(app, &mut dlg);
    }
    if let Some(t) = dlg.next_poll {
        ctx.request_repaint_after(t.saturating_duration_since(now));
    }

    let t = tokens(ctx);
    let enabled = dlg.enabled();
    let head = if enabled {
        Head { icon: Icon::Cloud, icon_color: t.accent, title: "Cloud sync", subtitle: None }
    } else {
        Head { icon: Icon::CloudOff, icon_color: t.muted, title: "Cloud sync", subtitle: None }
    };
    let busy = dlg.busy;
    let mut act = None;
    let closed = shared::shell(ctx, Id::unique("sync-dialog"), 620.0, &head, FooterKind::Strip, busy, |ui, part| match part {
        Part::Body => body(ui, &mut dlg, &mut act),
        Part::Footer => footer(ui, &dlg, &mut act),
    });
    if closed {
        act = Some(Act::Close);
    }
    match act {
        Some(Act::Close) => {
            tracing::debug!("sync: closed");
            return; // dropping `dlg` zeroizes the typed secrets
        }
        Some(Act::Browse) => browse(app),
        Some(_) if busy => {}
        Some(Act::Save) => match dlg.kind {
            Kind::Local => save_local(app, &mut dlg),
            Kind::S3 => save_s3(app, &mut dlg),
        },
        Some(Act::Disable) => disable(app, &mut dlg),
        Some(Act::Push) => push(app, &mut dlg),
        Some(Act::Pull) => pull(app, &mut dlg),
        None => {}
    }
    app.dialogs.sync = Some(dlg);
}

fn body(ui: &mut Ui, dlg: &mut SyncDialog, act: &mut Option<Act>) {
    form::hint(
        ui,
        "Encrypted backup syncs to one of two backends. Either way, the passphrase wraps the archive on-device — the sync service never sees plaintext.",
    );

    // Target picker.
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        form::caption(ui, "Target");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let w = (ui.available_width() - 8.0) / 2.0;
            let cards = [
                (Kind::Local, Icon::FolderOpen, "Local folder", "Dropbox / OneDrive / iCloud / Google Drive sync folder"),
                (Kind::S3, Icon::Server, "S3-compatible", "AWS, Cloudflare R2, Backblaze B2, Wasabi, MinIO, …"),
            ];
            for (kind, icon, title, hint) in cards {
                if form::option_card(ui, dlg.kind == kind, icon, title, hint, w).clicked() && dlg.kind != kind {
                    tracing::debug!(kind = kind.id(), "sync: target picked");
                    dlg.kind = kind;
                }
            }
        });
    });

    if let Some(s) = &dlg.status {
        status_strip(ui, s);
    }

    match dlg.kind {
        Kind::Local => {
            shared::labeled(ui, "Sync folder", |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let browse_w = 86.0;
                    let w = ui.available_width() - browse_w - 8.0;
                    let r = form::input(ui, &mut dlg.path, InputOpts::new(r"C:\Users\you\Dropbox\ezterm").mono().width(w));
                    if shared::outline(ui, Some(Icon::FolderOpen), "Browse", None, true).clicked() {
                        *act = Some(Act::Browse);
                    }
                    r
                })
                .inner
            });
            passphrase_fields(ui, &mut dlg.local_pass, &mut dlg.local_pass2, &mut dlg.show);
        }
        Kind::S3 => {
            shared::labeled(ui, "Endpoint URL", |ui| {
                form::input(ui, &mut dlg.s3_endpoint, InputOpts::new("https://s3.us-east-005.backblazeb2.com").mono())
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let w = (ui.available_width() - 8.0) / 2.0;
                ui.vertical(|ui| {
                    ui.set_width(w);
                    shared::labeled(ui, "Bucket", |ui| form::input(ui, &mut dlg.s3_bucket, InputOpts::new("ezterm-sync").mono().width(w)));
                });
                ui.vertical(|ui| {
                    ui.set_width(w);
                    shared::labeled(ui, "Region", |ui| form::input(ui, &mut dlg.s3_region, InputOpts::new("auto").mono().width(w)));
                });
            });
            shared::labeled(ui, "Key prefix (optional)", |ui| form::input(ui, &mut dlg.s3_prefix, InputOpts::new("ezterm/").mono()));
            shared::labeled(ui, "Access key ID", |ui| form::input(ui, &mut dlg.s3_access_key_id, InputOpts::new("").mono()));
            shared::labeled(ui, "Secret access key", |ui| form::secret(ui, &mut dlg.s3_secret, "", &mut dlg.show, None));
            passphrase_fields(ui, &mut dlg.s3_pass, &mut dlg.s3_pass2, &mut dlg.show);
            form::hint(
                ui,
                "Saving tests connectivity by HEAD-ing the sync object — a bad endpoint or wrong secret fails here rather than later when the auto-pusher fires.",
            );
        }
    }

    if let Some(m) = &dlg.msg {
        shared::success_line(ui, m);
    }
    shared::error_line(ui, dlg.err.as_deref());
}

fn passphrase_fields(ui: &mut Ui, pass: &mut String, confirm: &mut String, shown: &mut bool) {
    shared::labeled(ui, "Sync passphrase (≥ 8 chars — wraps the bundle; distinct from master password)", |ui| {
        form::secret(ui, pass, "", shown, None)
    });
    let s = *shown;
    shared::labeled(ui, "Confirm passphrase", |ui| shared::password(ui, confirm, s));
}

/// The always-present status box (mono, `bg-surface2/30`).
fn status_strip(ui: &mut Ui, s: &SyncStatus) {
    let t = tokens(ui.ctx());
    let enabled = s.kind != "none";
    let mono = |text: &str, color| RichText::new(text.to_string()).font(egui::FontId::monospace(size::XS)).color(color);
    Frame::new()
        .fill(alpha(t.surface2, 0.3))
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(2))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            // Text rows, not control rows: no 24 px minimum height.
            ui.spacing_mut().interact_size.y = 0.0;
            let line = |ui: &mut Ui, key: &str, value: &str| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label(mono(key, t.muted));
                    ui.label(mono(value, t.fg));
                });
            };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(mono("Status", t.muted));
                if enabled {
                    Icon::Check.show(ui, 12.0, t.success);
                    ui.label(mono(&format!("Enabled — {}", s.kind), t.fg));
                } else {
                    ui.label(mono("Not configured", t.muted));
                }
            });
            if s.kind == "local"
                && let Some(p) = &s.local_path
            {
                line(ui, "Path: ", p);
            }
            if s.kind == "s3"
                && let Some(ep) = &s.s3_endpoint
            {
                line(ui, "Bucket: ", &format!("{}@{ep}", s.s3_bucket.as_deref().unwrap_or("")));
                if let Some(p) = &s.s3_prefix {
                    line(ui, "Prefix: ", p);
                }
                if let Some(e) = &s.s3_last_etag {
                    line(ui, "Last ETag: ", e);
                }
            }
            if enabled {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label(mono("Last sync: ", t.muted));
                    ui.label(mono(&s.last_success_at.as_deref().map_or_else(|| "—".to_string(), display_time), t.fg));
                    if s.pending {
                        ui.add_space(8.0);
                        ui.label(mono("(writing…)", t.accent));
                    }
                });
            }
            if let Some(e) = &s.last_error {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label(mono("Last error: ", t.muted));
                    ui.label(mono(e, t.danger));
                });
            }
        });
}

/// An RFC 3339 timestamp as `YYYY-MM-DD HH:MM:SS UTC` (the core stores
/// UTC; the app has no time zone database, so no local time as in the
/// webview's `toLocaleString`).
fn display_time(s: &str) -> String {
    let b = s.as_bytes();
    if s.len() < 19 || b[10] != b'T' {
        return s.to_string();
    }
    let rest = s[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let zone = match rest {
        "" | "Z" | "+00:00" => "UTC".to_string(),
        other => format!("UTC{other}"),
    };
    format!("{} {} {zone}", &s[..10], &s[11..19])
}

fn footer(ui: &mut Ui, dlg: &SyncDialog, act: &mut Option<Act>) {
    let t = tokens(ui.ctx());
    let busy = dlg.busy;
    let enabled = dlg.enabled();
    let same_kind = dlg.status.as_ref().is_some_and(|s| s.kind == dlg.kind.id());
    let label = if enabled && same_kind { "Update & push" } else { "Enable sync" };
    if widgets::primary(ui, label, Tone::Accent, !busy, size::SM).clicked() {
        *act = Some(Act::Save);
    }
    if busy {
        form::spinner(ui, 12.0);
    }
    if enabled {
        if shared::outline(ui, Some(Icon::RefreshCw), "Push now", None, !busy).clicked() {
            *act = Some(Act::Push);
        }
        if dlg.status.as_ref().is_some_and(|s| s.kind == "s3")
            && shared::outline(ui, Some(Icon::Download), "Pull from cloud", None, !busy)
                .on_hover_text("Download the remote bundle and open the Restore dialog")
                .clicked()
        {
            *act = Some(Act::Pull);
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            if shared::outline(ui, None, "Disable", Some(t.danger), !busy).clicked() {
                *act = Some(Act::Disable);
            }
        });
    }
}

// ------------------------------------------------------------------ actions

fn refresh(app: &App, dlg: &mut SyncDialog) {
    dlg.polling = true;
    app.core.call(
        "sync_status",
        |st| async move { api::sync::sync_status(&st).await },
        |app, r| {
            let Some(d) = app.dialogs.sync.as_mut() else {
                return;
            };
            d.polling = false;
            d.next_poll = Some(Instant::now() + POLL);
            match r {
                Ok(s) => {
                    tracing::trace!(kind = %s.kind, pending = s.pending, last = ?s.last_success_at, "sync: status");
                    d.apply_status(s);
                }
                Err(e) => {
                    tracing::warn!("sync: status failed: {e}");
                    d.err = Some(e.to_string());
                }
            }
        },
    );
}

/// Re-read the status now and refill the form from it.
fn reseed(d: &mut SyncDialog) {
    d.seeded = false;
    d.next_poll = None;
}

fn browse(app: &App) {
    app.pick_path(Pick::Folder { title: "Sync folder (Dropbox / OneDrive / iCloud / …)" }, |app, path| {
        let Some(d) = app.dialogs.sync.as_mut() else {
            return;
        };
        match path {
            Some(p) => {
                tracing::debug!(path = %p.display(), "sync: folder chosen");
                d.path = p.to_string_lossy().into_owned();
            }
            None => tracing::debug!("sync: folder dialog cancelled"),
        }
    });
}

/// Run a sync call with the dialog busy; `ok` updates the dialog after a
/// success.
fn run<T: Send + 'static>(
    app: &App,
    dlg: &mut SyncDialog,
    what: &'static str,
    call: impl std::future::Future<Output = ezssh_core::Result<T>> + Send + 'static,
    ok: impl FnOnce(&mut App, T) + Send + 'static,
) {
    dlg.err = None;
    dlg.msg = None;
    dlg.busy = true;
    tracing::debug!(what, "sync: calling");
    app.core.call(what, move |_st| call, move |app, r| match r {
        Ok(v) => {
            tracing::debug!(what, "sync: ok");
            if let Some(d) = app.dialogs.sync.as_mut() {
                d.busy = false;
            }
            ok(app, v);
        }
        Err(e) => {
            tracing::warn!(what, "sync: failed: {e}");
            if let Some(d) = app.dialogs.sync.as_mut() {
                d.busy = false;
                d.err = Some(e.to_string());
            }
        }
    });
}

fn fail(dlg: &mut SyncDialog, msg: &str) {
    dlg.msg = None;
    dlg.err = Some(msg.to_string());
}

fn save_local(app: &mut App, dlg: &mut SyncDialog) {
    let path = dlg.path.trim().to_string();
    if path.is_empty() {
        return fail(dlg, "Pick a folder first");
    }
    if let Err(e) = check(&dlg.local_pass, &dlg.local_pass2) {
        return fail(dlg, e);
    }
    let pass = dlg.local_pass.to_string();
    tracing::info!(path = %path, "sync: enabling the local folder target");
    let st = app.core.state.clone();
    run(app, dlg, "sync_configure_local", async move { api::sync::sync_configure_local(&st, path, pass).await }, |app, ()| {
        if let Some(d) = app.dialogs.sync.as_mut() {
            d.local_pass = Zeroizing::default();
            d.local_pass2 = Zeroizing::default();
            d.msg = Some("Local-folder sync enabled. Initial push queued.".into());
            reseed(d);
        }
    });
}

fn save_s3(app: &mut App, dlg: &mut SyncDialog) {
    let required = [
        (dlg.s3_endpoint.trim().is_empty(), "Endpoint required"),
        (dlg.s3_bucket.trim().is_empty(), "Bucket required"),
        (dlg.s3_access_key_id.trim().is_empty(), "Access key ID required"),
        (dlg.s3_secret.is_empty(), "Secret access key required"),
    ];
    if let Some((_, msg)) = required.iter().find(|(missing, _)| *missing) {
        return fail(dlg, msg);
    }
    if let Err(e) = check(&dlg.s3_pass, &dlg.s3_pass2) {
        return fail(dlg, e);
    }
    let region = dlg.s3_region.trim();
    let cfg = S3ConfigInput {
        endpoint: dlg.s3_endpoint.trim().to_string(),
        region: if region.is_empty() { "auto".into() } else { region.to_string() },
        bucket: dlg.s3_bucket.trim().to_string(),
        prefix: dlg.s3_prefix.trim().to_string(),
        access_key_id: dlg.s3_access_key_id.trim().to_string(),
        secret_access_key: dlg.s3_secret.to_string(),
        passphrase: dlg.s3_pass.to_string(),
    };
    tracing::info!(endpoint = %cfg.endpoint, bucket = %cfg.bucket, region = %cfg.region, "sync: enabling the S3 target");
    let st = app.core.state.clone();
    run(app, dlg, "sync_configure_s3", async move { api::sync::sync_configure_s3(&st, cfg).await }, |app, ()| {
        if let Some(d) = app.dialogs.sync.as_mut() {
            d.s3_secret = Zeroizing::default();
            d.s3_pass = Zeroizing::default();
            d.s3_pass2 = Zeroizing::default();
            d.msg = Some("S3 sync enabled. Initial push queued.".into());
            reseed(d);
        }
    });
}

/// The sync dialog's passphrase wording ("≥ 8").
fn check(pass: &str, confirm: &str) -> Result<(), &'static str> {
    shared::check_passphrase(pass, confirm).map_err(|e| match e {
        "Passphrases do not match" => e,
        _ => "Passphrase must be ≥ 8 characters",
    })
}

fn disable(app: &mut App, dlg: &mut SyncDialog) {
    tracing::info!("sync: disabling");
    let st = app.core.state.clone();
    run(app, dlg, "sync_disable", async move { api::sync::sync_disable(&st).await }, |app, ()| {
        if let Some(d) = app.dialogs.sync.as_mut() {
            d.msg = Some("Sync disabled.".into());
            d.clear_secrets();
            reseed(d);
        }
    });
}

fn push(app: &mut App, dlg: &mut SyncDialog) {
    let st = app.core.state.clone();
    run(app, dlg, "sync_push_now", async move { api::sync::sync_push_now(&st).await }, |app, ()| {
        if let Some(d) = app.dialogs.sync.as_mut() {
            d.msg = Some("Push queued — will write within a few seconds.".into());
            d.next_poll = None;
        }
    });
}

fn pull(app: &mut App, dlg: &mut SyncDialog) {
    let st = app.core.state.clone();
    run(app, dlg, "sync_pull_to_temp", async move { api::sync::sync_pull_to_temp(&st).await }, |app, path| {
        // Closed or locked meanwhile: do not pop a restore dialog uninvited.
        if app.dialogs.sync.is_none() {
            tracing::info!(%path, "sync: pull finished after the dialog closed, not opening restore");
            return;
        }
        hand_off_pull(app, PathBuf::from(path));
    });
}

/// Close the sync dialog and open the restore dialog on a pulled bundle.
pub(crate) fn hand_off_pull(app: &mut App, path: PathBuf) {
    tracing::info!(path = %path.display(), "sync: pulled, opening restore");
    app.dialogs.sync = None;
    app.dialogs.restore = Some(RestoreDialog::from_path(path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_passphrase_wording() {
        assert_eq!(display_time("2026-11-01T12:34:56.123456+00:00"), "2026-11-01 12:34:56 UTC");
        assert_eq!(display_time("2026-11-01T12:34:56Z"), "2026-11-01 12:34:56 UTC");
        assert_eq!(display_time("2026-11-01T12:34:56+02:00"), "2026-11-01 12:34:56 UTC+02:00");
        assert_eq!(display_time("soon"), "soon");
        assert_eq!(check("short", "short"), Err("Passphrase must be ≥ 8 characters"));
        assert_eq!(check("long enough", "nope nope"), Err("Passphrases do not match"));
    }

    #[test]
    fn status_seeds_the_form_once() {
        let mut d = SyncDialog::default();
        assert_eq!(d.s3_region, "auto");
        d.apply_status(SyncStatus { kind: "s3".into(), s3_bucket: Some("b".into()), ..SyncStatus::default() });
        assert_eq!((d.kind, d.s3_bucket.as_str()), (Kind::S3, "b"));
        d.kind = Kind::Local;
        d.s3_bucket = "typing".into();
        d.apply_status(SyncStatus { kind: "s3".into(), s3_bucket: Some("b".into()), ..SyncStatus::default() });
        assert_eq!((d.kind, d.s3_bucket.as_str()), (Kind::Local, "typing"), "a poll keeps what the user changed");
    }
}
