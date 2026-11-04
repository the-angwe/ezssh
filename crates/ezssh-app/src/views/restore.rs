//! The restore dialog: open a backup (or a pulled sync bundle), unlock
//! it with its passphrase, pick what to restore from the decrypted tree,
//! then `backup_restore`.
//!
//! Passwords belong to their sessions: they are not in the credentials
//! list, a restored password session brings its own password, and its row
//! says so ("password included"). The "key not selected" warning is for key
//! sessions only.

use std::collections::BTreeSet;
use std::path::PathBuf;

use egui::{Align, CornerRadius, Frame, Id, Layout, Margin, RichText, Ui, vec2};
use ezssh_core::api::backup::{BackupPreview, RestoreSummary, SelectionSpec};
use ezssh_core::{AppError, api};
use zeroize::Zeroizing;

use super::backup::{self as shared, Span, count, join_or};
use crate::app::App;
use crate::files::{Filter, Pick};
use crate::form;
use crate::icons::Icon;
use crate::settings::Settings;
use crate::style::{Theme, alpha, font, font_semibold, radius, size, tokens};
use crate::widgets::{self, Button, Dialog, DialogFooter, DialogPart};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Step {
    #[default]
    Passphrase,
    Select,
}

/// What is ticked; everything starts ticked.
#[derive(Default)]
pub(crate) struct Selection {
    pub(crate) folders: BTreeSet<i64>,
    pub(crate) sessions: BTreeSet<i64>,
    pub(crate) creds: BTreeSet<i64>,
    pub(crate) hosts: BTreeSet<(String, i64)>,
    pub(crate) settings: bool,
}

impl Selection {
    fn all(p: &BackupPreview) -> Self {
        Self {
            folders: p.folders.iter().map(|f| f.id).collect(),
            sessions: p.sessions.iter().map(|s| s.id).collect(),
            creds: p.credentials.iter().map(|c| c.id).collect(),
            hosts: p
                .known_hosts
                .iter()
                .map(|k| (k.host.clone(), k.port))
                .collect(),
            settings: true,
        }
    }

    /// The footer's "N items selected" (settings count as one).
    fn total(&self, p: &BackupPreview) -> usize {
        self.folders.len()
            + self.sessions.len()
            + self.creds.len()
            + self.hosts.len()
            + usize::from(self.settings && p.setting_count > 0)
    }

    fn spec(&self, p: &BackupPreview) -> SelectionSpec {
        SelectionSpec {
            folder_ids: self.folders.iter().copied().collect(),
            session_ids: self.sessions.iter().copied().collect(),
            credential_ids: self.creds.iter().copied().collect(),
            known_hosts: self.hosts.iter().cloned().collect(),
            include_settings: self.settings && p.setting_count > 0,
        }
    }

    /// Selected key sessions whose key (in this backup) is not selected:
    /// they would arrive without it.
    pub(crate) fn sessions_missing_key<'a>(&self, p: &'a BackupPreview) -> Vec<&'a str> {
        p.sessions
            .iter()
            .filter(|s| self.sessions.contains(&s.id))
            .filter(|s| {
                s.credential_id.is_some_and(|c| {
                    p.credentials.iter().any(|k| k.id == c) && !self.creds.contains(&c)
                })
            })
            .map(|s| s.name.as_str())
            .collect()
    }
}

#[derive(Default)]
pub struct RestoreDialog {
    /// A file to start from (sync "Pull" hands over its temp file). `None`
    /// asks with the native open dialog first.
    pub path: Option<PathBuf>,
    /// Instance number (see `files::next_serial`).
    serial: u64,
    picking: bool,
    pub(crate) step: Step,
    passphrase: Zeroizing<String>,
    show: bool,
    pub(crate) busy: bool,
    pub(crate) err: Option<String>,
    pub(crate) preview: Option<BackupPreview>,
    pub(crate) sel: Selection,
    focused: Option<Step>,
}

impl RestoreDialog {
    pub fn from_path(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            ..Self::default()
        }
    }
}

enum Act {
    Close,
    Unlock,
    Restore,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut dlg) = app.dialogs.restore.take() else {
        return;
    };
    if dlg.serial == 0 {
        dlg.serial = crate::files::next_serial();
    }
    let Some(path) = dlg.path.clone() else {
        if !dlg.picking {
            dlg.picking = true;
            pick_file(app, dlg.serial);
        }
        crate::files::waiting_for_picker(ctx, Id::unique("restore-picking"));
        // Nothing to draw until a file is chosen.
        app.dialogs.restore = Some(dlg);
        return;
    };
    let t = tokens(ctx);
    let path_text = path.display().to_string();
    let footer = if dlg.step == Step::Passphrase {
        DialogFooter::Inline
    } else {
        DialogFooter::Strip
    };
    let busy = dlg.busy;
    let mut act = None;
    let out = Dialog::data(Id::unique("restore-dialog"), 620.0)
        .header(
            Icon::PackageCheck,
            t.accent,
            "Restore from backup",
            Some(&path_text),
        )
        .footer(footer)
        .busy(busy)
        .show(ctx, |ui, part| match (part, dlg.step) {
            (DialogPart::Body, Step::Passphrase) => {
                let r = shared::labeled(ui, "Backup passphrase", |ui| {
                    form::secret(ui, &mut dlg.passphrase, "", &mut dlg.show, None)
                });
                shared::autofocus(&r, &mut dlg.focused, Step::Passphrase);
                if crate::form::entered(ui, &r) {
                    act = Some(Act::Unlock);
                }
                shared::error_line(ui, dlg.err.as_deref());
            }
            (DialogPart::Footer, Step::Passphrase) => {
                if widgets::Button::primary("Unlock backup")
                    .enabled(!busy && !dlg.passphrase.is_empty())
                    .show(ui)
                    .clicked()
                {
                    act = Some(Act::Unlock);
                }
                if busy {
                    form::spinner(ui, 12.0);
                }
                if Button::secondary("Cancel")
                    .enabled(!busy)
                    .show(ui)
                    .clicked()
                {
                    act = Some(Act::Close);
                }
            }
            (DialogPart::Body, Step::Select) => {
                if let Some(p) = dlg.preview.as_ref() {
                    select_body(ui, p, &mut dlg.sel);
                }
                shared::error_line(ui, dlg.err.as_deref());
            }
            (DialogPart::Footer, Step::Select) => {
                let total = dlg.preview.as_ref().map_or(0, |p| dlg.sel.total(p));
                if widgets::Button::primary("Import selected")
                    .enabled(!busy && total > 0)
                    .show(ui)
                    .clicked()
                {
                    act = Some(Act::Restore);
                }
                if busy {
                    form::spinner(ui, 12.0);
                }
                if Button::secondary("Cancel")
                    .enabled(!busy)
                    .show(ui)
                    .clicked()
                {
                    act = Some(Act::Close);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{total} items selected"))
                            .font(font(size::SM))
                            .color(t.muted),
                    );
                });
            }
        });
    if out.dismissed() && !busy {
        act = Some(Act::Close);
    }
    match act {
        Some(Act::Close) => {
            tracing::debug!(path = %path.display(), "restore: closed");
            return; // dropping `dlg` zeroizes the passphrase
        }
        Some(Act::Unlock) if !busy => unlock(app, &mut dlg, path),
        Some(Act::Restore) if !busy => restore(app, &mut dlg, path),
        _ => {}
    }
    app.dialogs.restore = Some(dlg);
}

fn pick_file(app: &App, serial: u64) {
    app.pick_path(
        Pick::Open {
            title: "Restore ezSsh backup",
            filters: vec![Filter {
                name: "ezSsh backup",
                extensions: &["json"],
            }],
        },
        move |app, path| {
            let Some(d) = app.dialogs.restore.as_mut().filter(|d| d.serial == serial) else {
                return;
            };
            d.picking = false;
            match path {
                Some(p) => {
                    tracing::debug!(path = %p.display(), "restore: file chosen");
                    d.path = Some(p);
                }
                None => {
                    tracing::debug!("restore: open dialog cancelled");
                    app.dialogs.restore = None;
                }
            }
        },
    );
}

fn unlock(app: &mut App, dlg: &mut RestoreDialog, path: PathBuf) {
    if dlg.passphrase.is_empty() {
        dlg.err = Some("Enter the backup passphrase".into());
        return;
    }
    dlg.err = None;
    dlg.busy = true;
    let pass = dlg.passphrase.to_string();
    let file = path.to_string_lossy().into_owned();
    tracing::debug!(path = %file, "restore: reading the backup");
    let serial = dlg.serial;
    app.core.call(
        "backup_preview",
        move |st| async move { api::backup::backup_preview(&st, file, pass).await },
        move |app, r| {
            let Some(d) = app.dialogs.restore.as_mut().filter(|d| d.serial == serial) else {
                return;
            };
            d.busy = false;
            match r {
                Ok(p) => {
                    tracing::debug!(
                        folders = p.folders.len(),
                        sessions = p.sessions.len(),
                        credentials = p.credentials.len(),
                        known_hosts = p.known_hosts.len(),
                        settings = p.setting_count,
                        "restore: preview"
                    );
                    d.sel = Selection::all(&p);
                    d.preview = Some(p);
                    d.step = Step::Select;
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), "restore: preview failed: {e}");
                    // The core's generic AEAD failure reads "incorrect master
                    // password", which is not what was typed here.
                    d.err = Some(match e {
                        AppError::BadPassword => "Incorrect backup passphrase".to_string(),
                        e => e.to_string(),
                    });
                }
            }
        },
    );
}

fn restore(app: &mut App, dlg: &mut RestoreDialog, path: PathBuf) {
    let Some(p) = dlg.preview.as_ref() else {
        return;
    };
    let spec = dlg.sel.spec(p);
    dlg.err = None;
    dlg.busy = true;
    let pass = dlg.passphrase.to_string();
    let file = path.to_string_lossy().into_owned();
    tracing::info!(
        path = %file,
        folders = spec.folder_ids.len(),
        sessions = spec.session_ids.len(),
        credentials = spec.credential_ids.len(),
        known_hosts = spec.known_hosts.len(),
        settings = spec.include_settings,
        "restore: importing"
    );
    let serial = dlg.serial;
    app.core.call(
        "backup_restore",
        move |st| async move { api::backup::backup_restore(&st, file, pass, spec).await },
        move |app, r| match r {
            Ok(summary) => {
                tracing::info!(path = %path.display(), ?summary, "restore: done");
                if app
                    .dialogs
                    .restore
                    .as_ref()
                    .is_some_and(|d| d.serial == serial)
                {
                    app.dialogs.restore = None;
                }
                after_restore(app, &summary);
                app.toasts
                    .success("Restore complete", summary_text(&summary));
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), "restore: failed: {e}");
                if let Some(d) = app.dialogs.restore.as_mut().filter(|d| d.serial == serial) {
                    d.busy = false;
                    d.err = Some(e.to_string());
                }
            }
        },
    );
}

/// Reload what a restore can change: the tree, the shared credentials and,
/// when settings were applied, the settings and theme.
fn after_restore(app: &mut App, s: &RestoreSummary) {
    app.sidebar.reload(&app.core);
    app.creds.reload(&app.core);
    if s.settings_applied == 0 {
        return;
    }
    app.core.call(
        "settings_list",
        |st| async move { api::settings::settings_list(&st).await },
        |app, rows| match rows {
            Ok(rows) => {
                tracing::debug!(n = rows.len(), "restore: settings reloaded");
                let theme = rows
                    .iter()
                    .find(|(k, _)| k == "theme")
                    .map(|(_, v)| v.as_str());
                let theme = Theme::from_setting(theme);
                app.settings = Settings::from_rows(&rows);
                app.apply_settings();
                app.set_theme(theme);
            }
            Err(e) => {
                tracing::warn!("restore: reloading settings failed: {e}");
                app.toasts.danger("Could not load settings", e.to_string());
            }
        },
    );
}

/// The result toast's body, plus the renamed items.
fn summary_text(s: &RestoreSummary) -> String {
    let parts = [
        count(s.sessions_created, "session", "sessions"),
        count(s.credentials_created, "credential", "credentials"),
        count(s.folders_created, "folder", "folders"),
        count(s.known_hosts_upserted, "host", "hosts"),
        count(s.settings_applied, "setting", "settings"),
    ];
    let mut text = join_or(&parts, "nothing imported");
    if !s.renamed.is_empty() {
        let shown: Vec<&str> = s.renamed.iter().take(5).map(String::as_str).collect();
        text.push_str(&format!(". Imported as: {}", shown.join(", ")));
        if s.renamed.len() > 5 {
            text.push_str(&format!(" (+{} more)", s.renamed.len() - 5));
        }
    }
    text
}

// ------------------------------------------------------------------ select step

fn select_body(ui: &mut Ui, p: &BackupPreview, sel: &mut Selection) {
    let t = tokens(ui.ctx());
    let version = format!("v{}", p.app_version);
    let created = shared::local_time(&p.created_at, true);
    shared::rich(
        ui,
        &[
            (Span::Muted, "Created "),
            (Span::Mono, &created),
            (Span::Muted, " · version "),
            (Span::Mono, &version),
        ],
    );

    // Folders.
    let all = p.folders.iter().map(|f| f.id);
    section(
        ui,
        Icon::Folder,
        "Folders",
        &mut sel.folders,
        all,
        |ui, set| {
            for f in &p.folders {
                row(ui, set, f.id, &f.name, None, None);
            }
        },
    );

    // Sessions, with the password marker and the missing-key warning.
    let missing: BTreeSet<i64> = p
        .sessions
        .iter()
        .filter(|s| {
            s.credential_id
                .is_some_and(|c| p.credentials.iter().any(|k| k.id == c) && !sel.creds.contains(&c))
        })
        .map(|s| s.id)
        .collect();
    let all = p.sessions.iter().map(|s| s.id);
    section(
        ui,
        Icon::Server,
        "Sessions",
        &mut sel.sessions,
        all,
        |ui, set| {
            for s in &p.sessions {
                let port = if s.port != 22 {
                    format!(":{}", s.port)
                } else {
                    String::new()
                };
                let sub = format!("{} · {}@{}{port}", s.session_kind, s.username, s.host);
                let marker = if s.has_password {
                    Some(Marker::Password)
                } else if missing.contains(&s.id) && set.contains(&s.id) {
                    let key = s
                        .credential_id
                        .and_then(|c| p.credentials.iter().find(|k| k.id == c))
                        .map_or("", |k| k.label.as_str());
                    Some(Marker::KeyMissing(key))
                } else {
                    None
                };
                row(ui, set, s.id, &s.name, Some(&sub), marker);
            }
        },
    );
    let without_key = sel.sessions_missing_key(p);
    if !without_key.is_empty() {
        let names = without_key
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        let more = if without_key.len() > 3 {
            format!(" (+{} more)", without_key.len() - 3)
        } else {
            String::new()
        };
        form::note(
            ui,
            Icon::AlertTriangle,
            t.warning,
            &format!(
                "{names}{more}: the session's key is not selected, so it is restored without one."
            ),
        );
    }

    // Credentials (keys and key passphrases).
    let all = p.credentials.iter().map(|c| c.id);
    section(
        ui,
        Icon::KeyRound,
        "Credentials",
        &mut sel.creds,
        all,
        |ui, set| {
            for c in &p.credentials {
                row(ui, set, c.id, &c.label, Some(&c.kind), None);
            }
        },
    );

    // Known hosts.
    let all = p.known_hosts.iter().map(|k| (k.host.clone(), k.port));
    section(
        ui,
        Icon::Shield,
        "Known hosts",
        &mut sel.hosts,
        all,
        |ui, set| {
            for k in &p.known_hosts {
                row(
                    ui,
                    set,
                    (k.host.clone(), k.port),
                    &format!("{}:{}", k.host, k.port),
                    Some(&k.fingerprint_sha256),
                    None,
                );
            }
        },
    );

    if p.setting_count > 0 {
        let n = p.setting_count;
        let label = format!("Apply {n} app setting{}", if n == 1 { "" } else { "s" });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let r = ui.checkbox(
                &mut sel.settings,
                RichText::new(&label).font(font(size::SM)).color(t.fg),
            );
            if r.changed() {
                tracing::debug!(on = sel.settings, "restore: settings toggled");
            }
            Icon::Key.show(ui, 14.0, t.muted);
        });
    }

    shared::rich(
        ui,
        &[
            (
                Span::Muted,
                "Name collisions in folders, sessions, and credential labels are resolved by appending ",
            ),
            (Span::Mono, "(2)"),
            (Span::Muted, ", "),
            (Span::Mono, "(3)"),
            (Span::Muted, ", … Known hosts upsert by "),
            (Span::Mono, "(host, port)"),
            (
                Span::Muted,
                " key. Session passwords come with their sessions.",
            ),
        ],
    );
}

/// One group of the tree: icon, "TITLE (n/m)", All / None, and its rows
/// in a box of at most 160 px (`max-h-40`).
fn section<K: Ord + Clone>(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    set: &mut BTreeSet<K>,
    all: impl Iterator<Item = K>,
    rows: impl FnOnce(&mut Ui, &mut BTreeSet<K>),
) {
    let t = tokens(ui.ctx());
    let all: Vec<K> = all.collect();
    let every = !all.is_empty() && all.iter().all(|k| set.contains(k));
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            icon.show(ui, 14.0, t.muted);
            form::caption(ui, &format!("{title} ({}/{})", set.len(), all.len()));
            if !all.is_empty() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (text, a11y) = if every {
                        ("None", format!("Select no {}", title.to_lowercase()))
                    } else {
                        ("All", format!("Select all {}", title.to_lowercase()))
                    };
                    if shared::link(ui, text, &a11y).clicked() {
                        tracing::debug!(section = title, all = !every, "restore: toggle all");
                        *set = if every {
                            BTreeSet::new()
                        } else {
                            all.iter().cloned().collect()
                        };
                    }
                });
            }
        });
        if all.is_empty() {
            ui.label(
                RichText::new("none in backup")
                    .font(font(size::SM))
                    .italics()
                    .color(t.muted),
            );
        } else {
            egui::ScrollArea::vertical()
                .id_salt(("restore-section", title))
                .max_height(160.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    rows(ui, set);
                });
        }
    });
}

enum Marker<'a> {
    /// A password session whose password is in the backup.
    Password,
    /// A selected key session whose key is not selected (key label).
    KeyMissing(&'a str),
}

/// A checkbox row: label, optional marker, optional mono sub text on the
/// right (`max-w-[220px]`).
fn row<K: Ord + Clone>(
    ui: &mut Ui,
    set: &mut BTreeSet<K>,
    key: K,
    label: &str,
    sub: Option<&str>,
    marker: Option<Marker<'_>>,
) {
    let t = tokens(ui.ctx());
    let w = ui.available_width();
    let mut on = set.contains(&key);
    ui.allocate_ui_with_layout(vec2(w, 24.0), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let hover_rect = ui.max_rect();
        if ui.rect_contains_pointer(hover_rect) {
            ui.painter().rect_filled(hover_rect, radius::MD, t.surface2);
        }
        ui.add_space(4.0);
        let sub_w = sub.map_or(0.0, |s| {
            let g = ui.painter().layout_no_wrap(
                s.to_string(),
                egui::FontId::monospace(size::SM),
                t.muted,
            );
            g.size().x.min(220.0) + 8.0
        });
        let marker_w = match marker {
            Some(Marker::Password) => 120.0,
            Some(Marker::KeyMissing(_)) => 20.0,
            None => 0.0,
        };
        let label_w = (w - sub_w - marker_w - 40.0).max(60.0);
        let g = widgets::elide(ui, label, font(size::BASE), label_w);
        let r = ui.checkbox(
            &mut on,
            RichText::new(g.text()).font(font(size::BASE)).color(t.fg),
        );
        if r.changed() {
            if on {
                set.insert(key.clone());
            } else {
                set.remove(&key);
            }
        }
        match marker {
            Some(Marker::Password) => pill(ui, Icon::KeyRound, "password included", t.success),
            Some(Marker::KeyMissing(k)) => {
                let tip = if k.is_empty() {
                    "Its key is not selected".to_string()
                } else {
                    format!("Its key “{k}” is not selected")
                };
                let r = Icon::AlertTriangle.show(ui, 12.0, t.warning);
                widgets::a11y(&r, egui::Role::Image, &tip);
                r.on_hover_text(tip);
            }
            None => {}
        }
        if let Some(s) = sub {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(4.0);
                let g = widgets::elide(ui, s, egui::FontId::monospace(size::SM), 220.0);
                ui.label(
                    RichText::new(g.text())
                        .font(egui::FontId::monospace(size::SM))
                        .color(t.muted),
                );
            });
        }
    });
}

/// A small tinted badge with an icon.
fn pill(ui: &mut Ui, icon: Icon, text: &str, color: egui::Color32) {
    Frame::new()
        .fill(alpha(color, 0.14))
        .corner_radius(CornerRadius::same(radius::PILL as u8))
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            icon.show(ui, 11.0, color);
            ui.label(
                RichText::new(text)
                    .font(font_semibold(size::XS))
                    .color(color),
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ezssh_core::api::backup::{CredentialPreview, SessionPreview};

    fn preview() -> BackupPreview {
        let session = |id, name: &str, cred, pw| SessionPreview {
            id,
            folder_id: None,
            name: name.into(),
            host: "h".into(),
            port: 22,
            username: "u".into(),
            session_kind: "ssh".into(),
            auth_type: if pw { "password".into() } else { "key".into() },
            credential_id: cred,
            has_password: pw,
        };
        BackupPreview {
            created_at: "2026-11-01T00:00:00+00:00".into(),
            app_version: "0.0.0".into(),
            folders: vec![],
            sessions: vec![
                session(1, "pw", None, true),
                session(2, "keyed", Some(7), false),
            ],
            credentials: vec![CredentialPreview {
                id: 7,
                kind: "private_key".into(),
                label: "id_ed25519".into(),
            }],
            known_hosts: vec![],
            setting_count: 3,
        }
    }

    #[test]
    fn selection_counts_spec_and_key_warning() {
        let p = preview();
        let mut sel = Selection::all(&p);
        assert_eq!(sel.total(&p), 2 + 1 + 1);
        assert!(sel.sessions_missing_key(&p).is_empty());
        sel.creds.clear();
        // Only the key session warns; the password session carries its own.
        assert_eq!(sel.sessions_missing_key(&p), vec!["keyed"]);
        sel.settings = false;
        let spec = sel.spec(&p);
        assert_eq!(spec.session_ids, vec![1, 2]);
        assert!(spec.credential_ids.is_empty() && !spec.include_settings);
    }

    #[test]
    fn summary_lists_renames() {
        let s = RestoreSummary {
            sessions_created: 1,
            renamed: vec!["a (2)".into()],
            ..RestoreSummary::default()
        };
        assert_eq!(summary_text(&s), "1 session. Imported as: a (2)");
        assert_eq!(summary_text(&RestoreSummary::default()), "nothing imported");
    }
}
