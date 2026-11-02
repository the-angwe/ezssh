//! Native open / save / folder dialogs (rfd: the desktop portal on Linux,
//! the system dialogs on Windows and macOS). Each runs on the core runtime
//! and hands the chosen path to a continuation on the UI thread, like a
//! core call; `None` means the user cancelled.

use std::path::PathBuf;

use crate::app::App;

/// A file type filter: label and extensions without dots.
pub struct Filter {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
}

/// What to ask for.
pub enum Pick {
    Open { title: &'static str, filters: Vec<Filter> },
    Save { title: &'static str, file_name: String, filters: Vec<Filter> },
    Folder { title: &'static str },
}

impl App {
    /// Show a native file dialog; `then(app, path)` runs once it closes.
    pub(crate) fn pick_path(&self, what: Pick, then: impl FnOnce(&mut App, Option<PathBuf>) + Send + 'static) {
        self.core.call(
            "file dialog",
            move |_st| async move {
                let mut d = rfd::AsyncFileDialog::new();
                let filters = match &what {
                    Pick::Open { filters, .. } | Pick::Save { filters, .. } => filters.as_slice(),
                    Pick::Folder { .. } => &[],
                };
                for f in filters {
                    d = d.add_filter(f.name, f.extensions);
                }
                let picked = match what {
                    Pick::Open { title, .. } => d.set_title(title).pick_file().await,
                    Pick::Save { title, file_name, .. } => d.set_title(title).set_file_name(file_name).save_file().await,
                    Pick::Folder { title } => d.set_title(title).pick_folder().await,
                };
                let path = picked.map(|h| h.path().to_path_buf());
                tracing::debug!(path = ?path, "files: dialog closed");
                path
            },
            then,
        );
    }
}

impl App {
    /// Native multi-file open dialog; `then(app, paths)` gets an empty list
    /// when the user cancelled.
    pub(crate) fn pick_files(&self, title: &'static str, then: impl FnOnce(&mut App, Vec<PathBuf>) + Send + 'static) {
        self.core.call(
            "file dialog (many)",
            move |_st| async move {
                let picked = rfd::AsyncFileDialog::new().set_title(title).pick_files().await.unwrap_or_default();
                let paths: Vec<PathBuf> = picked.iter().map(|h| h.path().to_path_buf()).collect();
                tracing::debug!(n = paths.len(), "files: dialog closed");
                paths
            },
            then,
        );
    }
}

/// A fresh instance number for a dialog, so a late result (a file dialog
/// closed, a core call finished) only lands in the dialog that started it.
pub fn next_serial() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Shown while a native file dialog is open: keeps the window modal (the
/// sidebar menus would otherwise stay live underneath it).
pub fn waiting_for_picker(ctx: &egui::Context, id: egui::Id) {
    let t = crate::style::tokens(ctx);
    crate::widgets::dialog(
        ctx,
        id,
        360.0,
        |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                crate::form::spinner(ui, 14.0);
                ui.label(egui::RichText::new("Choose a file in the system dialog…").font(crate::style::font(crate::style::size::SM)).color(t.fg));
            });
        },
        None,
    );
}
