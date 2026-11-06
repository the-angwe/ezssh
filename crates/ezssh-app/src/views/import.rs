//! The MobaXterm import dialog: preview a MobaXterm session file
//! (`.mxtsessions` / `MobaXterm.ini`), choose how to treat duplicates,
//! import.

use std::collections::BTreeSet;
use std::path::PathBuf;

use egui::{
    Color32, CornerRadius, CursorIcon, Frame, Id, Margin, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui, vec2,
};
use ezssh_core::api;
use ezssh_core::api::import::{MobaImportPreview, MobaImportResult};
use ezssh_core::import::mobaxterm::ParsedMobaSession;

use super::backup::{count, join_or};
use crate::app::App;
use crate::files::{Filter, Pick};
use crate::form;
use crate::icons::Icon;
use crate::style::{alpha, font, font_semibold, radius, size, tokens};
use crate::widgets::{self, Button, Dialog, DialogFooter, DialogPart};

/// Rows listed before "… and N more".
const LIST_MAX: usize = 200;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Strategy {
    #[default]
    Skip,
    Overwrite,
    Rename,
}

impl Strategy {
    fn id(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Overwrite => "overwrite",
            Self::Rename => "rename",
        }
    }
}

const STRATEGIES: [(Strategy, &str, &str); 3] = [
    (
        Strategy::Skip,
        "Skip",
        "Leave the existing session untouched.",
    ),
    (
        Strategy::Overwrite,
        "Overwrite",
        "Refresh host / port / username / auth. Attached credentials are kept when the auth type matches, cleared when it changes. Terminal settings stay.",
    ),
    (
        Strategy::Rename,
        "Rename",
        "Import as a new session named \"<name> (imported)\".",
    ),
];

#[derive(Default)]
pub struct ImportDialog {
    /// The session file. `None` asks with the native open dialog first.
    pub path: Option<PathBuf>,
    /// Instance number (see `files::next_serial`).
    serial: u64,
    picking: bool,
    loading: bool,
    pub(crate) preview: Option<MobaImportPreview>,
    pub(crate) error: Option<String>,
    pub(crate) strategy: Strategy,
    pub(crate) busy: bool,
}

impl ImportDialog {
    pub fn from_path(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            ..Self::default()
        }
    }
}

enum Act {
    Close,
    Import,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut dlg) = app.dialogs.import.take() else {
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
        crate::files::waiting_for_picker(ctx, Id::unique("import-picking"));
        app.dialogs.import = Some(dlg);
        return;
    };
    if dlg.preview.is_none() && dlg.error.is_none() && !dlg.loading {
        load_preview(app, &mut dlg, path.clone());
    }
    let t = tokens(ctx);
    let path_text = path.display().to_string();
    let busy = dlg.busy;
    let mut act = None;
    let out = Dialog::data(Id::unique("import-dialog"), 620.0)
        .header(
            Icon::Upload,
            t.accent,
            "Import from MobaXterm",
            Some(&path_text),
        )
        .footer(DialogFooter::Strip)
        .busy(busy)
        .show(ctx, |ui, part| match part {
            DialogPart::Body => body(ui, &mut dlg),
            DialogPart::Footer => widgets::trailing(ui, |ui| {
                if Button::secondary("Cancel")
                    .enabled(!busy)
                    .show(ui)
                    .clicked()
                {
                    act = Some(Act::Close);
                }
                let can = dlg.preview.as_ref().is_some_and(|p| !p.sessions.is_empty()) && !busy;
                let icon = if busy { Icon::Loader2 } else { Icon::Upload };
                if widgets::Button::primary("Import")
                    .icon(icon)
                    .enabled(can)
                    .show(ui)
                    .clicked()
                {
                    act = Some(Act::Import);
                }
            }),
        });
    if out.dismissed() && !busy {
        act = Some(Act::Close);
    }
    match act {
        Some(Act::Close) => {
            tracing::debug!(path = %path.display(), "import: closed");
            return;
        }
        Some(Act::Import) if !busy => run_import(app, &mut dlg),
        _ => {}
    }
    app.dialogs.import = Some(dlg);
}

fn pick_file(app: &App, serial: u64) {
    app.pick_path(
        Pick::Open {
            title: "Import MobaXterm sessions",
            filters: vec![
                Filter {
                    name: "MobaXterm sessions",
                    extensions: &["mxtsessions", "ini"],
                },
                Filter {
                    name: "All files",
                    extensions: &["*"],
                },
            ],
        },
        move |app, path| {
            let Some(d) = app.dialogs.import.as_mut().filter(|d| d.serial == serial) else {
                return;
            };
            d.picking = false;
            match path {
                Some(p) => {
                    tracing::debug!(path = %p.display(), "import: file chosen");
                    d.path = Some(p);
                }
                None => {
                    tracing::debug!("import: open dialog cancelled");
                    app.dialogs.import = None;
                }
            }
        },
    );
}

fn load_preview(app: &App, dlg: &mut ImportDialog, path: PathBuf) {
    dlg.loading = true;
    let file = path.to_string_lossy().into_owned();
    tracing::debug!(path = %file, "import: reading the session file");
    let serial = dlg.serial;
    app.core.call(
        "mobaxterm_preview",
        move |st| async move { api::import::mobaxterm_preview(&st, file).await },
        move |app, r| {
            let Some(d) = app.dialogs.import.as_mut().filter(|d| d.serial == serial) else {
                return;
            };
            d.loading = false;
            match r {
                Ok(p) => {
                    tracing::debug!(
                        sessions = p.sessions.len(),
                        new_folders = p.new_folder_paths.len(),
                        duplicates = p.duplicate_indices.len(),
                        skipped_non_ssh = p.skipped_non_ssh,
                        skipped_malformed = p.skipped_malformed,
                        "import: preview"
                    );
                    d.preview = Some(p);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), "import: preview failed: {e}");
                    d.error = Some(e.to_string());
                }
            }
        },
    );
}

fn run_import(app: &mut App, dlg: &mut ImportDialog) {
    let Some(p) = dlg.preview.as_ref() else {
        return;
    };
    let sessions = p.sessions.clone();
    let strategy = dlg.strategy.id().to_string();
    dlg.busy = true;
    dlg.error = None;
    tracing::info!(sessions = sessions.len(), strategy = %strategy, "import: committing");
    let serial = dlg.serial;
    app.core.call(
        "mobaxterm_commit",
        move |st| async move { api::import::mobaxterm_commit(&st, sessions, strategy).await },
        move |app, r| match r {
            Ok(result) => {
                tracing::info!(?result, "import: done");
                if app
                    .dialogs
                    .import
                    .as_ref()
                    .is_some_and(|d| d.serial == serial)
                {
                    app.dialogs.import = None;
                }
                app.sidebar.reload(&app.core);
                app.creds.reload(&app.core);
                let (title, body, missing) = result_toast(&result);
                if missing {
                    app.toasts.danger(title, body);
                } else {
                    app.toasts.success(title, body);
                }
            }
            Err(e) => {
                tracing::warn!("import: commit failed: {e}");
                if let Some(d) = app.dialogs.import.as_mut().filter(|d| d.serial == serial) {
                    d.busy = false;
                    d.error = Some(e.to_string());
                }
            }
        },
    );
}

/// The result toast: (title, body, missing keys).
fn result_toast(r: &MobaImportResult) -> (&'static str, String, bool) {
    let parts = [
        count(r.created, "created", "created"),
        count(r.updated, "updated", "updated"),
        count(r.skipped_duplicate, "skipped", "skipped"),
        count(r.created_folders, "folder", "folders"),
        count(r.imported_keys.len(), "key", "keys"),
    ];
    let body = join_or(&parts, "Nothing to import");
    if r.missing_keys.is_empty() {
        return ("Import complete", body, false);
    }
    let shown = r
        .missing_keys
        .iter()
        .take(5)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let more = if r.missing_keys.len() > 5 {
        format!(" (+{} more)", r.missing_keys.len() - 5)
    } else {
        String::new()
    };
    (
        "Import complete with missing keys",
        format!("{body}. Could not read: {shown}{more}"),
        true,
    )
}

fn body(ui: &mut Ui, dlg: &mut ImportDialog) {
    let t = tokens(ui.ctx());
    if dlg.preview.is_none() && dlg.error.is_none() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            form::spinner(ui, 14.0);
            ui.label(
                RichText::new("Reading session file…")
                    .font(font(size::SM))
                    .color(t.muted),
            );
        });
    }
    if let Some(e) = &dlg.error {
        form::note(ui, Icon::AlertTriangle, t.danger, e);
    }
    let Some(p) = dlg.preview.as_ref() else {
        return;
    };
    let key_files: BTreeSet<&str> = p
        .sessions
        .iter()
        .filter_map(|s| s.private_key_path.as_deref())
        .collect();
    stat_row(ui, p, key_files.len());

    let folders = p.new_folder_paths.len();
    if folders > 0 {
        let title = format!("{folders} new folder{} will be created", plural(folders));
        group(ui, Some((Icon::FolderPlus, t.muted)), &title, |ui| {
            list(ui, ("import-folders", 0), 128.0, |ui| {
                for f in &p.new_folder_paths {
                    mono_line(ui, &f.join(" / "), t.muted);
                }
            });
        });
    }

    let dups = p.duplicate_indices.len();
    if dups > 0 {
        let title = format!("{dups} duplicate{} detected", plural(dups));
        group(ui, Some((Icon::AlertTriangle, t.warning)), &title, |ui| {
            form::hint(
                ui,
                "A session with the same folder, name, host, port, and user already exists.",
            );
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            for (s, label, hint) in STRATEGIES {
                if strategy_option(ui, dlg.strategy == s, label, hint).clicked()
                    && dlg.strategy != s
                {
                    tracing::debug!(strategy = s.id(), "import: duplicate strategy");
                    dlg.strategy = s;
                }
            }
        });
    }

    let n = p.sessions.len();
    if n > 0 {
        group(
            ui,
            None,
            &format!("{n} session{} to import", plural(n)),
            |ui| {
                list(ui, ("import-sessions", 0), 160.0, |ui| {
                    for s in p.sessions.iter().take(LIST_MAX) {
                        session_line(ui, s);
                    }
                    if n > LIST_MAX {
                        ui.label(
                            RichText::new(format!("… and {} more", n - LIST_MAX))
                                .font(font(size::SM))
                                .italics()
                                .color(t.muted),
                        );
                    }
                });
            },
        );
    } else {
        form::hint(
            ui,
            "No SSH sessions were found in this file. MobaXterm stores other session types (RDP, VNC, Telnet, …) in the same file; only SSH rows are imported.",
        );
    }

    form::hint(
        ui,
        "Auth method comes from the source row. Referenced key files are read from disk and stored as encrypted private_key credentials in your vault, then attached to the matching sessions. Passwords aren't exported by MobaXterm, so password sessions arrive without one — edit each session to enter its password. Passphrase-protected keys: attach a key_passphrase credential in the session dialog to avoid per-connect prompts.",
    );
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Five bordered count cells (`grid-cols-5`).
fn stat_row(ui: &mut Ui, p: &MobaImportPreview, key_files: usize) {
    let t = tokens(ui.ctx());
    let skipped = p.skipped_non_ssh + p.skipped_malformed;
    let cells = [
        ("Sessions", p.sessions.len(), false),
        ("New folders", p.new_folder_paths.len(), false),
        ("Key files", key_files, false),
        (
            "Duplicates",
            p.duplicate_indices.len(),
            !p.duplicate_indices.is_empty(),
        ),
        ("Skipped", skipped, p.skipped_malformed > 0),
    ];
    let gap = 8.0;
    let w = (ui.available_width() - gap * 4.0) / 5.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::hover());
    for (i, (label, value, warn)) in cells.into_iter().enumerate() {
        let r = Rect::from_min_size(
            Pos2::new(rect.left() + i as f32 * (w + gap), rect.top()),
            vec2(w, rect.height()),
        );
        ui.painter().rect_filled(r, radius::LG, t.surface2);
        let color = if warn { t.warning } else { t.fg };
        let v = ui
            .painter()
            .layout_no_wrap(value.to_string(), font_semibold(size::XL), color);
        let vh = v.size().y;
        ui.painter()
            .galley(Pos2::new(r.left() + 10.0, r.top() + 8.0), v, color);
        widgets::paint_text(
            ui,
            Rect::from_min_size(
                Pos2::new(r.left() + 10.0, r.top() + 8.0 + vh),
                vec2(w - 20.0, 14.0),
            ),
            &label.to_uppercase(),
            font_semibold(size::XS),
            t.muted,
        );
        let id = ui.scope_id().with(("stat", i));
        let resp = ui.interact(r, id, Sense::hover());
        widgets::a11y(&resp, egui::Role::Label, &format!("{label}: {value}"));
    }
}

/// `Section` of the import dialog: optional icon and an uppercase title.
fn group(ui: &mut Ui, icon: Option<(Icon, Color32)>, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some((icon, color)) = icon {
                icon.show(ui, 14.0, color);
            }
            form::caption(ui, title);
        });
        add(ui);
    });
}

fn list(ui: &mut Ui, salt: (&str, u8), max_h: f32, add: impl FnOnce(&mut Ui)) {
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .max_height(max_h)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            add(ui);
        });
}

fn mono_line(ui: &mut Ui, text: &str, color: Color32) {
    let w = ui.available_width();
    let g = widgets::elide(ui, text, egui::FontId::monospace(size::SM), w);
    ui.label(
        RichText::new(g.text())
            .font(egui::FontId::monospace(size::SM))
            .color(color),
    );
}

/// `name · user@host[:port] [auth] key-file folder/path`.
fn session_line(ui: &mut Ui, s: &ParsedMobaSession) {
    let t = tokens(ui.ctx());
    let mono = |text: &str, color: Color32| {
        RichText::new(text.to_string())
            .font(egui::FontId::monospace(size::SM))
            .color(color)
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(mono(&s.name, t.fg));
        ui.label(mono(" · ", alpha(t.muted, 0.5)));
        let port = if s.port != 22 {
            format!(":{}", s.port)
        } else {
            String::new()
        };
        ui.label(mono(&format!("{}@{}{port}", s.username, s.host), t.muted));
        ui.add_space(8.0);
        let (badge, color) = if s.session_kind == "wsl" {
            ("wsl", t.success)
        } else if s.auth_type == "key" {
            (s.auth_type.as_str(), t.accent)
        } else {
            (s.auth_type.as_str(), t.warning)
        };
        Frame::new()
            .fill(alpha(color, 0.16))
            .corner_radius(CornerRadius::same(radius::PILL as u8))
            .inner_margin(Margin::symmetric(6, 1))
            .show(ui, |ui| {
                ui.label(RichText::new(badge).font(font(size::XS)).color(color));
            });
        if let Some(k) = &s.private_key_path {
            ui.add_space(4.0);
            ui.label(mono(key_file_name(k), alpha(t.muted, 0.8)));
        }
        if !s.folder_path.is_empty() {
            ui.add_space(8.0);
            ui.label(mono(&s.folder_path.join(" / "), alpha(t.muted, 0.6)));
        }
    });
}

/// The file name of a MobaXterm key path (either slash).
fn key_file_name(raw: &str) -> &str {
    raw.rsplit(['\\', '/'])
        .find(|p| !p.is_empty())
        .unwrap_or(raw)
}

/// `StrategyOption`: a bordered radio row with a title and hint.
fn strategy_option(ui: &mut Ui, on: bool, label: &str, hint: &str) -> egui::Response {
    let t = tokens(ui.ctx());
    let w = ui.available_width();
    let text_w = w - 24.0 - 30.0;
    let title_g = ui
        .painter()
        .layout(label.to_string(), font_semibold(size::BASE), t.fg, text_w);
    let hint_g = ui
        .painter()
        .layout(hint.to_string(), font(size::SM), t.muted, text_w);
    let h = 10.0 + title_g.size().y + 2.0 + hint_g.size().y + 10.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let hovered = resp.hovered();
    let (border, fill) = if on {
        (t.accent, t.accent_soft)
    } else if hovered {
        (t.border_strong, t.surface2)
    } else {
        (t.border_strong, Color32::TRANSPARENT)
    };
    ui.painter().rect_filled(rect, radius::LG, fill);
    ui.painter().rect_stroke(
        rect,
        radius::LG,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let c = Pos2::new(
        rect.left() + 12.0 + 8.0,
        rect.top() + 10.0 + title_g.size().y / 2.0,
    );
    ui.painter().circle_stroke(
        c,
        7.0,
        Stroke::new(1.5, if on { t.accent } else { t.border_strong }),
    );
    if on {
        ui.painter().circle_filled(c, 3.5, t.accent);
    }
    let x = rect.left() + 12.0 + 16.0 + 12.0;
    let th = title_g.size().y;
    ui.painter()
        .galley(Pos2::new(x, rect.top() + 10.0), title_g, t.fg);
    ui.painter()
        .galley(Pos2::new(x, rect.top() + 10.0 + th + 2.0), hint_g, t.muted);
    widgets::focus_ring(ui, &resp, rect);
    widgets::a11y_selected(&resp, egui::Role::RadioButton, on, label);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_and_toast() {
        assert_eq!(key_file_name("_MyDocuments_\\ssh-keys\\x.pem"), "x.pem");
        assert_eq!(key_file_name("/home/u/.ssh/id"), "id");
        assert_eq!(key_file_name("plain"), "plain");
        let r = MobaImportResult {
            created: 2,
            created_folders: 1,
            ..MobaImportResult::default()
        };
        assert_eq!(
            result_toast(&r),
            ("Import complete", "2 created, 1 folder".to_string(), false)
        );
        let r = MobaImportResult {
            missing_keys: vec!["a.pem".into()],
            ..MobaImportResult::default()
        };
        assert_eq!(
            result_toast(&r),
            (
                "Import complete with missing keys",
                "Nothing to import. Could not read: a.pem".to_string(),
                true
            )
        );
    }
}
