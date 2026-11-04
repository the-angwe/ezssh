//! Window layout: view mode, tile grid, sidebar and SFTP pane widths, SFTP
//! columns, the last update check. Kept in `layout.json` next to the
//! database. It is per-install convenience,
//! not a preference, so backups and sync never carry it.
//!
//! Minimized tabs are not persisted: tabs do not survive a restart, so their ids would never match again.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::tabs::ViewMode;

pub const SIDEBAR_MIN: f32 = 180.0;
pub const SIDEBAR_MAX: f32 = 520.0;
pub const SIDEBAR_DEFAULT: f32 = 240.0;

pub const SFTP_PANE_MIN: f32 = 200.0;
pub const SFTP_PANE_MAX: f32 = 640.0;
pub const SFTP_PANE_DEFAULT: f32 = 320.0;

/// Narrowest terminal column the resize handles leave free (px).
pub const TERMINAL_MIN_W: f32 = 320.0;
/// Fixed width of the per-tab Forwards pane (px).
pub const FORWARDS_PANE_W: f32 = 280.0;

/// SFTP file-list Name / Size column bounds (min, max, default); the Mode
/// column is fixed. The defaults plus `views::sftp::MODE_COL_W` and the row
/// padding must fit `SFTP_PANE_DEFAULT`, or a fresh pane opens with a
/// horizontal scroll bar and a clipped Mode column.
pub const SFTP_COL_NAME: (f32, f32, f32) = (80.0, 1200.0, 128.0);
pub const SFTP_COL_SIZE: (f32, f32, f32) = (48.0, 160.0, 64.0);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileGrid {
    pub rows: u8,
    pub cols: u8,
}

impl Default for TileGrid {
    fn default() -> Self {
        Self { rows: 2, cols: 2 }
    }
}

impl TileGrid {
    pub fn clamped(rows: i64, cols: i64) -> Self {
        Self {
            rows: rows.clamp(1, 8) as u8,
            cols: cols.clamp(1, 8) as u8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SftpColumns {
    pub name: f32,
    pub size: f32,
}

impl Default for SftpColumns {
    fn default() -> Self {
        Self {
            name: SFTP_COL_NAME.2,
            size: SFTP_COL_SIZE.2,
        }
    }
}

/// Clamp a column width to its bounds; non-finite input gives the default.
pub fn clamp_col((min, max, default): (f32, f32, f32), w: f32) -> f32 {
    if w.is_finite() {
        w.clamp(min, max).round()
    } else {
        default
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub view_mode: ViewMode,
    pub tile_grid: TileGrid,
    pub sidebar_width: f32,
    pub sftp_pane_width: f32,
    pub sftp_columns: SftpColumns,
    /// Unix time of the last successful update check.
    pub last_update_check: Option<i64>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::Tabs,
            tile_grid: TileGrid::default(),
            sidebar_width: SIDEBAR_DEFAULT,
            sftp_pane_width: SFTP_PANE_DEFAULT,
            sftp_columns: SftpColumns::default(),
            last_update_check: None,
        }
    }
}

/// On-disk shape. Every field optional so a file from an older or newer
/// build still loads; bad values fall back per field.
#[derive(Default, Serialize, Deserialize)]
struct Stored {
    view_mode: Option<String>,
    tile_grid: Option<serde_json::Value>,
    sidebar_width: Option<f64>,
    sftp_pane_width: Option<f64>,
    sftp_columns: Option<serde_json::Value>,
    last_update_check: Option<i64>,
}

fn in_range(v: Option<f64>, min: f32, max: f32) -> Option<f32> {
    v.filter(|v| v.is_finite() && *v >= min as f64 && *v <= max as f64)
        .map(|v| v.round() as f32)
}

impl Layout {
    /// Parse `layout.json` text. Malformed input gives the defaults.
    pub fn parse(text: &str) -> Self {
        let stored: Stored = match serde_json::from_str(text) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("layout: unreadable layout.json, using defaults: {e}");
                return Self::default();
            }
        };
        let d = Self::default();
        let tile_grid = stored
            .tile_grid
            .map(|v| {
                let n = |k: &str| {
                    v.get(k)
                        .and_then(|x| x.as_f64())
                        .filter(|x| x.is_finite() && *x >= 1.0)
                        .map(|x| x as i64)
                };
                TileGrid::clamped(n("rows").unwrap_or(2), n("cols").unwrap_or(2))
            })
            .unwrap_or(d.tile_grid);
        let sftp_columns = match stored.sftp_columns {
            Some(v) => {
                let col = |k: &str, (min, max, default): (f32, f32, f32)| {
                    in_range(v.get(k).and_then(|x| x.as_f64()), min, max).unwrap_or(default)
                };
                SftpColumns {
                    name: col("name", SFTP_COL_NAME),
                    size: col("size", SFTP_COL_SIZE),
                }
            }
            None => d.sftp_columns,
        };
        Self {
            view_mode: stored
                .view_mode
                .as_deref()
                .and_then(ViewMode::from_id)
                .unwrap_or(d.view_mode),
            tile_grid,
            sidebar_width: in_range(stored.sidebar_width, SIDEBAR_MIN, SIDEBAR_MAX)
                .unwrap_or(d.sidebar_width),
            sftp_pane_width: in_range(stored.sftp_pane_width, SFTP_PANE_MIN, SFTP_PANE_MAX)
                .unwrap_or(d.sftp_pane_width),
            sftp_columns,
            last_update_check: stored.last_update_check.filter(|t| *t > 0),
        }
    }

    pub fn to_json(&self) -> String {
        let stored = Stored {
            view_mode: Some(self.view_mode.id().to_string()),
            tile_grid: Some(
                serde_json::json!({ "rows": self.tile_grid.rows, "cols": self.tile_grid.cols }),
            ),
            sidebar_width: Some(self.sidebar_width as f64),
            sftp_pane_width: Some(self.sftp_pane_width as f64),
            sftp_columns: Some(
                serde_json::json!({ "name": self.sftp_columns.name, "size": self.sftp_columns.size }),
            ),
            last_update_check: self.last_update_check,
        };
        match serde_json::to_string_pretty(&stored) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("layout: serialize failed: {e}");
                String::new()
            }
        }
    }
}

/// `layout.json` beside `db_path`.
pub fn path_for(db_path: &Path) -> PathBuf {
    db_path.with_file_name("layout.json")
}

pub fn load(path: &Path) -> Layout {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let layout = Layout::parse(&text);
            tracing::debug!(path = %path.display(), ?layout, "layout: loaded");
            layout
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Layout::default(),
        Err(e) => {
            tracing::warn!(path = %path.display(), "layout: read failed, using defaults: {e}");
            Layout::default()
        }
    }
}

/// Write atomically (temp file + rename). Failures are logged and
/// returned; layout is a convenience, the caller only reports it.
pub fn save(path: &Path, layout: &Layout) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let result = std::fs::write(&tmp, layout.to_json()).and_then(|()| std::fs::rename(&tmp, path));
    match &result {
        Ok(()) => tracing::debug!(path = %path.display(), "layout: saved"),
        Err(e) => tracing::warn!(path = %path.display(), "layout: save failed: {e}"),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_for_missing_or_malformed_files() {
        assert_eq!(Layout::parse(""), Layout::default());
        assert_eq!(Layout::parse("not json"), Layout::default());
        assert_eq!(Layout::parse("42"), Layout::default());
        assert_eq!(Layout::parse("{}"), Layout::default());
    }

    #[test]
    fn round_trips() {
        let l = Layout {
            view_mode: ViewMode::Cascade,
            tile_grid: TileGrid { rows: 3, cols: 4 },
            sidebar_width: 300.0,
            sftp_pane_width: 400.0,
            sftp_columns: SftpColumns {
                name: 300.0,
                size: 90.0,
            },
            last_update_check: Some(1_793_181_296),
        };
        assert_eq!(Layout::parse(&l.to_json()), l);
    }

    #[test]
    fn bad_fields_fall_back_one_by_one() {
        let l = Layout::parse(
            r#"{"view_mode":"spiral","tile_grid":{"rows":99,"cols":0},"sidebar_width":10,
                "sftp_pane_width":300.4,"sftp_columns":{"name":1201,"size":100,"mode":999}}"#,
        );
        assert_eq!(l.view_mode, ViewMode::Tabs);
        // 99 clamps to 8; 0 is not a usable value and falls back to 2.
        assert_eq!(l.tile_grid, TileGrid { rows: 8, cols: 2 });
        assert_eq!(l.sidebar_width, SIDEBAR_DEFAULT);
        assert_eq!(l.sftp_pane_width, 300.0);
        assert_eq!(
            l.sftp_columns,
            SftpColumns {
                name: SFTP_COL_NAME.2,
                size: 100.0
            }
        );
    }

    #[test]
    fn clamp_col_bounds() {
        assert_eq!(clamp_col(SFTP_COL_NAME, 10.0), 80.0);
        assert_eq!(clamp_col(SFTP_COL_NAME, 99_999.0), 1200.0);
        assert_eq!(clamp_col(SFTP_COL_SIZE, 100.6), 101.0);
        assert_eq!(clamp_col(SFTP_COL_SIZE, f32::NAN), SFTP_COL_SIZE.2);
        assert_eq!(clamp_col(SFTP_COL_NAME, f32::INFINITY), SFTP_COL_NAME.2);
    }

    #[test]
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_for(&dir.path().join("ezterm.sqlite"));
        assert_eq!(path.file_name().unwrap(), "layout.json");
        assert_eq!(load(&path), Layout::default(), "missing file");
        let l = Layout {
            sidebar_width: 333.0,
            ..Layout::default()
        };
        save(&path, &l).unwrap();
        assert_eq!(load(&path), l);
    }
}
