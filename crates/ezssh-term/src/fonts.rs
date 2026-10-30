//! Terminal fonts: a named system font in up to four faces (regular, bold,
//! italic, bold italic) registered as egui font families.
//!
//! The session's `font_family` value is resolved the way the webview did
//! it: a CSS-like stack, tried left to right, ending in the same default
//! stack as `ui/lib/xterm.ts` (`DEFAULT_FONT_STACK`). Glyphs the chosen
//! font lacks fall back to the bundled "Symbols Nerd Font Mono" (Nerd Font
//! icons, Powerline) and then to eframe's system fallback (CJK, emoji).

use std::collections::HashMap;

use egui::FontFamily;
use egui::epaint::text::{FontData, FontInsert, FontPriority, InsertFontFamily};
use fontique::{
    Attributes, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, GenericFamily, QueryFamily,
    QueryStatus, SourceCache, SourceCacheOptions,
};

/// Nerd Font icons and Powerline glyphs, MIT (see
/// `assets/SymbolsNerdFontMono-LICENSE.txt`), from nerd-fonts v3.4.0.
const SYMBOLS_TTF: &[u8] = include_bytes!("../assets/SymbolsNerdFontMono-Regular.ttf");
const SYMBOLS_NAME: &str = "Symbols Nerd Font Mono";

/// What the webview's `DEFAULT_FONT_STACK` resolves to on each OS
/// (`ui-monospace` is honoured only by WebKit on macOS, where it is SF Mono).
pub fn default_stack() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["SF Mono", "Cascadia Mono", "Menlo", "DejaVu Sans Mono", "Consolas"]
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
        Self { regular: FontFamily::Monospace, bold: None, italic: None, bold_italic: None, loaded: None }
    }
}

impl Faces {
    /// Best family for the cell attributes. A missing face falls back to
    /// the nearest one (bold italic → bold → italic → regular).
    pub fn pick(&self, bold: bool, italic: bool) -> &FontFamily {
        let found = match (bold, italic) {
            (true, true) => self.bold_italic.as_ref().or(self.bold.as_ref()).or(self.italic.as_ref()),
            (true, false) => self.bold.as_ref(),
            (false, true) => self.italic.as_ref(),
            (false, false) => None,
        };
        found.unwrap_or(&self.regular)
    }
}

/// Split a stored font-family value into names: `"Fira Code", monospace`
/// → `["Fira Code", "monospace"]`. Empty means "default stack".
pub fn parse_stack(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_generic(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "monospace" | "ui-monospace")
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
            collection: Collection::new(CollectionOptions { shared: false, system_fonts: true }),
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
        let faces = faces.or_else(|| self.install_generic(ctx)).unwrap_or_else(|| {
            tracing::warn!(?names, "term.fonts: no monospace font found, using egui's built-in one");
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
        let id = self.collection.generic_families(GenericFamily::Monospace).next()?;
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
        if !matches!(face, Face::Regular) && (font.synthesis.embolden() || font.synthesis.skew().is_some()) {
            return None;
        }
        let (blob, _id) = font.blob.into_raw_parts();
        Some(FontData::from_blob(blob, font.index))
    }

    fn install(&mut self, ctx: &egui::Context, name: &str, id: Option<QueryFamily<'_>>) -> Option<Faces> {
        let family = id.unwrap_or(QueryFamily::Named(name));
        let regular = self.find(family, Face::Regular)?;
        let regular = self.register(ctx, name, Face::Regular, regular);
        let mut faces = Faces { regular, bold: None, italic: None, bold_italic: None, loaded: Some(name.to_string()) };
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

    fn register(&mut self, ctx: &egui::Context, name: &str, face: Face, data: FontData) -> FontFamily {
        let family = FontFamily::Name(format!("ezssh:{name}:{}", face.tag()).into());
        ctx.add_font(FontInsert {
            name: format!("{name} {}", face.tag()),
            data,
            families: vec![InsertFontFamily { family: family.clone(), priority: FontPriority::Highest }],
        });
        self.add_symbols(ctx, &family);
        family
    }

    /// Append the Nerd Font symbols to `family` as its last face.
    fn add_symbols(&mut self, ctx: &egui::Context, family: &FontFamily) {
        if !self.symbols_installed {
            tracing::debug!(bytes = SYMBOLS_TTF.len(), "term.fonts: loading bundled Nerd Font symbols");
            self.symbols_installed = true;
        }
        ctx.add_font(FontInsert {
            name: SYMBOLS_NAME.into(),
            data: FontData::from_static(SYMBOLS_TTF),
            families: vec![InsertFontFamily { family: family.clone(), priority: FontPriority::Lowest }],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacks_split_and_unquote() {
        assert_eq!(parse_stack(""), Vec::<String>::new());
        assert_eq!(parse_stack("Fira Code"), vec!["Fira Code"]);
        assert_eq!(parse_stack("\"Fira Code\", 'JetBrains Mono', monospace"), vec!["Fira Code", "JetBrains Mono", "monospace"]);
        assert_eq!(parse_stack(" , Consolas ,"), vec!["Consolas"]);
    }

    #[test]
    fn missing_faces_fall_back() {
        let b = FontFamily::Name("b".into());
        let i = FontFamily::Name("i".into());
        let f = Faces { bold: Some(b.clone()), italic: Some(i.clone()), ..Faces::default() };
        assert_eq!(f.pick(false, false), &FontFamily::Monospace);
        assert_eq!(f.pick(true, false), &b);
        assert_eq!(f.pick(false, true), &i);
        assert_eq!(f.pick(true, true), &b);
        assert_eq!(Faces::default().pick(true, true), &FontFamily::Monospace);
    }

    #[test]
    fn bundled_symbols_font_is_a_font() {
        assert_eq!(&SYMBOLS_TTF[..4], &[0, 1, 0, 0]);
    }
}
