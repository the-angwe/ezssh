//! Terminal fonts: a named system font in up to four faces (regular, bold,
//! italic, bold italic) registered as egui font families.
//!
//! The session's `font_family` value is a CSS-like stack (the format the
//! ezTerm app stored), tried left to right, ending in the default stack.
//! Glyphs the chosen font lacks fall back to the bundled "Symbols Nerd Font Mono" (Nerd Font
//! icons, Powerline) and then to eframe's system fallback (CJK, emoji).

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::FontFamily;
use egui::epaint::text::{FontData, FontInsert, FontPriority, InsertFontFamily};
use fontique::{
    Attributes, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, GenericFamily,
    QueryFamily, QueryStatus, SourceCache, SourceCacheOptions,
};

/// Nerd Font icons and Powerline glyphs, MIT (see
/// `assets/SymbolsNerdFontMono-LICENSE.txt`), from nerd-fonts v3.4.0.
const SYMBOLS_TTF: &[u8] = include_bytes!("../assets/SymbolsNerdFontMono-Regular.ttf");
const SYMBOLS_NAME: &str = "Symbols Nerd Font Mono";

/// The default font stack on each OS.
pub fn default_stack() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &[
            "SF Mono",
            "Cascadia Mono",
            "Menlo",
            "DejaVu Sans Mono",
            "Consolas",
        ]
    } else {
        &["Cascadia Mono", "Menlo", "DejaVu Sans Mono", "Consolas"]
    }
}

/// egui families for one terminal font.
#[derive(Clone, Debug, PartialEq)]
pub struct Faces {
    pub regular: FontFamily,
    pub bold: Option<FontFamily>,
    pub italic: Option<FontFamily>,
    pub bold_italic: Option<FontFamily>,
    /// Name of the font actually loaded; `None` for egui's built-in monospace.
    pub loaded: Option<String>,
}

impl Default for Faces {
    fn default() -> Self {
        Self {
            regular: FontFamily::Monospace,
            bold: None,
            italic: None,
            bold_italic: None,
            loaded: None,
        }
    }
}

impl Faces {
    /// Best family for the cell attributes. A missing face falls back to
    /// the nearest one (bold italic → bold → italic → regular).
    pub fn pick(&self, bold: bool, italic: bool) -> &FontFamily {
        let found = match (bold, italic) {
            (true, true) => self
                .bold_italic
                .as_ref()
                .or(self.bold.as_ref())
                .or(self.italic.as_ref()),
            (true, false) => self.bold.as_ref(),
            (false, true) => self.italic.as_ref(),
            (false, false) => None,
        };
        found.unwrap_or(&self.regular)
    }

    /// These faces minus any family egui has not bound yet. `add_font`
    /// only applies at once when no text was laid out this pass; otherwise
    /// the family exists from the next pass. Until then the missing faces
    /// fall back (regular to egui's built-in monospace) and a repaint is
    /// asked for, so the font shows one frame later.
    pub fn usable(&self, ctx: &egui::Context) -> Faces {
        let (regular, bold, italic, bold_italic) = ctx.fonts(|f| {
            let families = &f.definitions().families;
            let bound = |fam: &FontFamily| {
                !matches!(fam, FontFamily::Name(_)) || families.contains_key(fam)
            };
            (
                bound(&self.regular),
                self.bold.as_ref().map(bound),
                self.italic.as_ref().map(bound),
                self.bold_italic.as_ref().map(bound),
            )
        });
        let all = regular
            && [bold, italic, bold_italic]
                .iter()
                .all(|b| b.unwrap_or(true));
        if all {
            return self.clone();
        }
        tracing::debug!(
            font = ?self.loaded,
            regular,
            ?bold,
            ?italic,
            ?bold_italic,
            "term.fonts: font not bound yet this pass, drawing with a fallback"
        );
        ctx.request_repaint();
        let keep =
            |fam: &Option<FontFamily>, ok: Option<bool>| fam.clone().filter(|_| ok == Some(true));
        Faces {
            regular: if regular {
                self.regular.clone()
            } else {
                FontFamily::Monospace
            },
            bold: keep(&self.bold, bold),
            italic: keep(&self.italic, italic),
            bold_italic: keep(&self.bold_italic, bold_italic),
            loaded: if regular { self.loaded.clone() } else { None },
        }
    }
}

/// Split a stored font-family value into names: `"Fira Code", monospace`
/// → `["Fira Code", "monospace"]`. Empty means "default stack".
pub fn parse_stack(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| {
            s.trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_generic(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "monospace" | "ui-monospace"
    )
}

#[derive(Clone, Copy)]
enum Face {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Face {
    fn tag(self) -> &'static str {
        match self {
            Face::Regular => "regular",
            Face::Bold => "bold",
            Face::Italic => "italic",
            Face::BoldItalic => "bold-italic",
        }
    }

    fn attrs(self) -> Attributes {
        let (style, weight) = match self {
            Face::Regular => (FontStyle::Normal, FontWeight::NORMAL),
            Face::Bold => (FontStyle::Normal, FontWeight::BOLD),
            Face::Italic => (FontStyle::Italic, FontWeight::NORMAL),
            Face::BoldItalic => (FontStyle::Italic, FontWeight::BOLD),
        };
        Attributes::new(FontWidth::NORMAL, style, weight)
    }
}

/// Loads terminal fonts on demand and remembers what it registered with
/// egui. One per app; shared by every tab.
pub struct FontRegistry {
    collection: Collection,
    cache: SourceCache,
    by_value: HashMap<String, Faces>,
    symbols_installed: bool,
}

impl Default for FontRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FontRegistry {
    pub fn new() -> Self {
        Self {
            collection: Collection::new(CollectionOptions {
                shared: false,
                system_fonts: true,
            }),
            cache: SourceCache::new(SourceCacheOptions::default()),
            by_value: HashMap::new(),
            symbols_installed: false,
        }
    }

    /// Faces for a session's font-family value (empty = default stack).
    /// The first lookup of a value loads the font; later ones are free.
    pub fn faces(&mut self, ctx: &egui::Context, value: &str) -> Faces {
        if let Some(f) = self.by_value.get(value) {
            return f.clone();
        }
        let started = std::time::Instant::now();
        let mut names = parse_stack(value);
        names.extend(default_stack().iter().map(|s| s.to_string()));
        let mut faces = None;
        for name in &names {
            if is_generic(name) {
                continue;
            }
            if let Some(f) = self.install(ctx, name, None) {
                faces = Some(f);
                break;
            }
            tracing::debug!(font = %name, "term.fonts: not installed");
        }
        let faces = faces
            .or_else(|| self.install_generic(ctx))
            .unwrap_or_else(|| {
                tracing::warn!(
                    ?names,
                    "term.fonts: no monospace font found, using egui's built-in one"
                );
                Faces::default()
            });
        tracing::info!(
            requested = value,
            font = ?faces.loaded,
            bold = faces.bold.is_some(),
            italic = faces.italic.is_some(),
            bold_italic = faces.bold_italic.is_some(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "term.fonts: resolved"
        );
        self.by_value.insert(value.to_string(), faces.clone());
        faces
    }

    /// The system's generic monospace font (fontconfig's `monospace`).
    fn install_generic(&mut self, ctx: &egui::Context) -> Option<Faces> {
        let id = self
            .collection
            .generic_families(GenericFamily::Monospace)
            .next()?;
        let name = self.collection.family_name(id)?.to_string();
        self.install(ctx, &name, Some(QueryFamily::Id(id)))
    }

    fn find(&mut self, family: QueryFamily<'_>, face: Face) -> Option<FontData> {
        let mut query = self.collection.query(&mut self.cache);
        query.set_families([family]);
        query.set_attributes(face.attrs());
        let mut found = None;
        query.matches_with(|font| {
            found = Some(font.clone());
            QueryStatus::Stop
        });
        let font = found?;
        // egui cannot synthesise bold or slant: a face that only exists by
        // synthesis is reported missing and the regular face is used.
        if !matches!(face, Face::Regular)
            && (font.synthesis.embolden() || font.synthesis.skew().is_some())
        {
            return None;
        }
        let (blob, _id) = font.blob.into_raw_parts();
        Some(FontData::from_blob(blob, font.index))
    }

    fn install(
        &mut self,
        ctx: &egui::Context,
        name: &str,
        id: Option<QueryFamily<'_>>,
    ) -> Option<Faces> {
        let family = id.unwrap_or(QueryFamily::Named(name));
        let regular = self.find(family, Face::Regular)?;
        let regular = self.register(ctx, name, Face::Regular, regular);
        let mut faces = Faces {
            regular,
            bold: None,
            italic: None,
            bold_italic: None,
            loaded: Some(name.to_string()),
        };
        for face in [Face::Bold, Face::Italic, Face::BoldItalic] {
            let Some(data) = self.find(family, face) else {
                tracing::debug!(font = name, face = face.tag(), "term.fonts: face missing");
                continue;
            };
            let fam = Some(self.register(ctx, name, face, data));
            match face {
                Face::Bold => faces.bold = fam,
                Face::Italic => faces.italic = fam,
                Face::BoldItalic => faces.bold_italic = fam,
                Face::Regular => {}
            }
        }
        Some(faces)
    }

    fn register(
        &mut self,
        ctx: &egui::Context,
        name: &str,
        face: Face,
        data: FontData,
    ) -> FontFamily {
        let family = FontFamily::Name(format!("ezssh:{name}:{}", face.tag()).into());
        ctx.add_font(FontInsert {
            name: format!("{name} {}", face.tag()),
            data,
            families: vec![InsertFontFamily {
                family: family.clone(),
                priority: FontPriority::Highest,
            }],
        });
        self.add_symbols(ctx, &family);
        family
    }

    /// Append the Nerd Font symbols to `family` as its last face. Each
    /// family gets its own insert name: `add_font` skips a name egui
    /// already has, so a shared name bound the symbols only to the fonts
    /// registered in the first pass.
    fn add_symbols(&mut self, ctx: &egui::Context, family: &FontFamily) {
        if !self.symbols_installed {
            tracing::debug!(
                bytes = SYMBOLS_TTF.len(),
                "term.fonts: loading bundled Nerd Font symbols"
            );
            self.symbols_installed = true;
        }
        ctx.add_font(FontInsert {
            name: format!("{SYMBOLS_NAME} for {family:?}"),
            data: FontData::from_static(SYMBOLS_TTF),
            families: vec![InsertFontFamily {
                family: family.clone(),
                priority: FontPriority::Lowest,
            }],
        });
    }
}

/// Installed families whose regular face is monospaced, sorted without
/// regard to case. The system is scanned once per process (fontconfig on
/// Linux, DirectWrite on Windows, CoreText on macOS); the first call blocks
/// for the scan, so call [`prewarm_monospace_families`] at start-up.
pub fn monospace_families() -> &'static [String] {
    static LIST: OnceLock<Vec<String>> = OnceLock::new();
    LIST.get_or_init(scan_monospace)
}

/// Run the [`monospace_families`] scan on a background thread so the first
/// font dialog does not wait for it.
pub fn prewarm_monospace_families() {
    let spawned = std::thread::Builder::new()
        .name("font-scan".into())
        .spawn(|| {
            monospace_families();
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "term.fonts: font scan thread not started; the first font dialog scans inline");
    }
}

fn scan_monospace() -> Vec<String> {
    let started = std::time::Instant::now();
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: true,
    });
    let mut cache = SourceCache::new(SourceCacheOptions::default());
    let names: Vec<String> = collection.family_names().map(str::to_string).collect();
    let mut found = Vec::new();
    for name in &names {
        let Some(family) = collection.family_by_name(name) else {
            tracing::debug!(font = %name, "term.fonts: scan: listed family not found by name");
            continue;
        };
        let Some(info) = family.match_font(
            FontWidth::NORMAL,
            FontStyle::Normal,
            FontWeight::NORMAL,
            false,
        ) else {
            tracing::debug!(font = %name, "term.fonts: scan: family has no fonts");
            continue;
        };
        let Some(blob) = info.load(Some(&mut cache)) else {
            tracing::debug!(font = %name, "term.fonts: scan: font file not loaded");
            continue;
        };
        match is_monospace(blob.as_ref(), info.index()) {
            Ok(true) => found.push(name.clone()),
            Ok(false) => {}
            Err(e) => {
                tracing::debug!(font = %name, error = %e, "term.fonts: scan: font not readable")
            }
        }
    }
    let mut list = drop_weight_aliases(found);
    list.sort_by_key(|n| n.to_lowercase());
    tracing::info!(
        families = names.len(),
        monospace = list.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "term.fonts: monospace scan done"
    );
    list
}

/// Monospace when the sampled ASCII glyphs the font has share one advance
/// width (`post.isFixedPitch` is unreliable both ways). Fonts without
/// both `i` and `M` (symbol and non-Latin fonts) are not terminal fonts.
fn is_monospace(data: &[u8], index: u32) -> Result<bool, read_fonts::ReadError> {
    use read_fonts::{FontRef, TableProvider};
    let font = FontRef::from_index(data, index)?;
    let cmap = font.cmap()?;
    let hmtx = font.hmtx()?;
    let (Some(i), Some(m)) = (cmap.map_codepoint('i'), cmap.map_codepoint('M')) else {
        return Ok(false);
    };
    let Some(width) = hmtx.advance(i) else {
        return Ok(false);
    };
    if width == 0 || hmtx.advance(m) != Some(width) {
        return Ok(false);
    }
    Ok("lW.0m_"
        .chars()
        .filter_map(|c| cmap.map_codepoint(c))
        .all(|g| hmtx.advance(g) == Some(width)))
}

/// Fontconfig also lists a static font's legacy family name
/// ("JetBrains Mono ExtraBold" next to "JetBrains Mono"). Those name one
/// weight of a family that is already listed, so they are left out.
fn drop_weight_aliases(names: Vec<String>) -> Vec<String> {
    const WEIGHTS: &[&str] = &[
        "Thin",
        "Hairline",
        "ExtraLight",
        "Extra Light",
        "UltraLight",
        "Ultra Light",
        "Light",
        "SemiLight",
        "Semi Light",
        "Book",
        "Regular",
        "Medium",
        "SemiBold",
        "Semi Bold",
        "DemiBold",
        "Demi Bold",
        "Bold",
        "ExtraBold",
        "Extra Bold",
        "UltraBold",
        "Ultra Bold",
        "Black",
        "Heavy",
        "ExtraBlack",
        "Extra Black",
    ];
    let all: std::collections::HashSet<String> = names.iter().cloned().collect();
    names
        .into_iter()
        .filter(|name| {
            let alias = WEIGHTS.iter().any(|w| {
                name.strip_suffix(w)
                    .and_then(|base| base.strip_suffix(' '))
                    .is_some_and(|base| all.contains(base))
            });
            if alias {
                tracing::trace!(font = %name, "term.fonts: scan: weight alias left out");
            }
            !alias
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_aliases_dropped_only_next_to_their_family() {
        let names = [
            "JetBrains Mono",
            "JetBrains Mono ExtraBold",
            "JetBrains Mono NL",
            "JetBrains Mono NL Light",
            "Iosevka Heavy",
        ];
        assert_eq!(
            drop_weight_aliases(names.iter().map(|s| s.to_string()).collect()),
            vec!["JetBrains Mono", "JetBrains Mono NL", "Iosevka Heavy"]
        );
    }

    /// Prints the scan; on a machine with JetBrains Mono NL it must be there.
    #[test]
    fn monospace_scan_lists_installed_families() {
        let list = monospace_families();
        eprintln!("{} monospace families: {list:?}", list.len());
        assert!(
            list.windows(2)
                .all(|w| w[0].to_lowercase() <= w[1].to_lowercase()),
            "sorted"
        );
        let mut probe = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: true,
        });
        for name in ["JetBrains Mono", "JetBrains Mono NL", "DejaVu Sans Mono"] {
            if probe.family_by_name(name).is_some() {
                assert!(
                    list.iter().any(|n| n == name),
                    "{name} is installed but not listed"
                );
            }
        }
        for name in ["DejaVu Sans", "Noto Sans"] {
            assert!(!list.iter().any(|n| n == name), "{name} is proportional");
        }
    }

    #[test]
    fn stacks_split_and_unquote() {
        assert_eq!(parse_stack(""), Vec::<String>::new());
        assert_eq!(parse_stack("Fira Code"), vec!["Fira Code"]);
        assert_eq!(
            parse_stack("\"Fira Code\", 'JetBrains Mono', monospace"),
            vec!["Fira Code", "JetBrains Mono", "monospace"]
        );
        assert_eq!(parse_stack(" , Consolas ,"), vec!["Consolas"]);
    }

    #[test]
    fn missing_faces_fall_back() {
        let b = FontFamily::Name("b".into());
        let i = FontFamily::Name("i".into());
        let f = Faces {
            bold: Some(b.clone()),
            italic: Some(i.clone()),
            ..Faces::default()
        };
        assert_eq!(f.pick(false, false), &FontFamily::Monospace);
        assert_eq!(f.pick(true, false), &b);
        assert_eq!(f.pick(false, true), &i);
        assert_eq!(f.pick(true, true), &b);
        assert_eq!(Faces::default().pick(true, true), &FontFamily::Monospace);
    }

    /// Run one pass that lays out text, then `during` in the same pass.
    fn pass_after_text(ctx: &egui::Context, mut during: impl FnMut(&egui::Context)) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label("text laid out before the font is added");
            during(ui.ctx());
        });
        out.textures_delta.clear();
    }

    /// Opening a session mid-frame: the family is not bound until the next
    /// pass, so `usable` falls back for that pass and keeps it afterwards.
    #[test]
    fn font_added_mid_pass_is_used_from_the_next_pass() {
        let ctx = egui::Context::default();
        let mut reg = FontRegistry::new();
        let mut faces = None;
        pass_after_text(&ctx, |ctx| {
            let f = reg.faces(ctx, "");
            assert_eq!(
                f.usable(ctx).regular,
                FontFamily::Monospace,
                "not bound in the pass that added it"
            );
            faces = Some(f);
        });
        let faces = faces.unwrap();
        if faces.loaded.is_none() {
            eprintln!("no system monospace font; nothing to check");
            return;
        }
        pass_after_text(&ctx, |ctx| {
            assert_eq!(faces.usable(ctx), faces, "bound from the next pass")
        });
    }

    /// Fonts registered in later passes get the Nerd Font symbols too.
    #[test]
    fn symbols_bound_to_families_added_in_later_passes() {
        let ctx = egui::Context::default();
        let mut reg = FontRegistry::new();
        let (a, b) = (
            FontFamily::Name("ezssh-test:a".into()),
            FontFamily::Name("ezssh-test:b".into()),
        );
        pass_after_text(&ctx, |ctx| reg.add_symbols(ctx, &a));
        pass_after_text(&ctx, |ctx| reg.add_symbols(ctx, &b));
        pass_after_text(&ctx, |ctx| {
            ctx.fonts(|f| {
                for fam in [&a, &b] {
                    let names = f.definitions().families.get(fam);
                    assert!(
                        names.is_some_and(|n| n.iter().any(|n| n.starts_with(SYMBOLS_NAME))),
                        "{fam:?}: {names:?}"
                    );
                }
            })
        });
    }

    #[test]
    fn bundled_symbols_font_is_a_font() {
        assert_eq!(&SYMBOLS_TTF[..4], &[0, 1, 0, 0]);
    }
}
