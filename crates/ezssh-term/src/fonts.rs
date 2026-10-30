//! Loading a named system monospace font (regular + bold) as the
//! terminal font. Glyphs it lacks (CJK, emoji, Nerd Font symbols) come
//! from eframe's system font fallback provider.

use egui::FontFamily;
use egui::epaint::text::{FontData, FontInsert, FontPriority, InsertFontFamily};
use fontique::{
    Attributes, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, QueryFamily, QueryStatus,
    SourceCache, SourceCacheOptions,
};

pub const REGULAR_FAMILY: &str = "ezssh-mono";
pub const BOLD_FAMILY: &str = "ezssh-mono-bold";

/// What [`install`] found.
#[derive(Clone, Debug)]
pub struct TermFonts {
    pub family: FontFamily,
    pub bold_family: Option<FontFamily>,
    /// Family name actually loaded, or `None` when egui's built-in monospace font is used.
    pub loaded: Option<String>,
}

impl Default for TermFonts {
    fn default() -> Self {
        Self { family: FontFamily::Monospace, bold_family: None, loaded: None }
    }
}

fn find(collection: &mut Collection, cache: &mut SourceCache, name: &str, weight: FontWeight) -> Option<FontData> {
    let mut query = collection.query(cache);
    query.set_families([QueryFamily::Named(name)]);
    query.set_attributes(Attributes::new(FontWidth::NORMAL, FontStyle::Normal, weight));
    let mut found = None;
    query.matches_with(|font| {
        found = Some(font.clone());
        QueryStatus::Stop
    });
    let font = found?;
    if weight == FontWeight::BOLD && font.synthesis.embolden() {
        // Only a regular face exists; egui cannot embolden it.
        return None;
    }
    let (blob, _id) = font.blob.into_raw_parts();
    Some(FontData::from_blob(blob, font.index))
}

/// Try each name in `candidates` in order and install the first one found
/// as the terminal font. Falls back to egui's monospace font.
pub fn install(ctx: &egui::Context, candidates: &[&str]) -> TermFonts {
    let started = std::time::Instant::now();
    let mut collection = Collection::new(CollectionOptions { shared: false, system_fonts: true });
    let mut cache = SourceCache::new(SourceCacheOptions::default());
    for name in candidates {
        let Some(regular) = find(&mut collection, &mut cache, name, FontWeight::NORMAL) else {
            tracing::debug!(font = name, "term.fonts: not installed");
            continue;
        };
        ctx.add_font(FontInsert {
            name: format!("{name} regular"),
            data: regular,
            families: vec![InsertFontFamily { family: FontFamily::Name(REGULAR_FAMILY.into()), priority: FontPriority::Highest }],
        });
        let bold_family = match find(&mut collection, &mut cache, name, FontWeight::BOLD) {
            Some(bold) => {
                ctx.add_font(FontInsert {
                    name: format!("{name} bold"),
                    data: bold,
                    families: vec![InsertFontFamily { family: FontFamily::Name(BOLD_FAMILY.into()), priority: FontPriority::Highest }],
                });
                Some(FontFamily::Name(BOLD_FAMILY.into()))
            }
            None => {
                tracing::info!(font = name, "term.fonts: no bold face, bold uses the regular face");
                None
            }
        };
        tracing::info!(font = name, bold = bold_family.is_some(), elapsed_ms = started.elapsed().as_millis() as u64, "term.fonts: installed");
        return TermFonts { family: FontFamily::Name(REGULAR_FAMILY.into()), bold_family, loaded: Some(name.to_string()) };
    }
    tracing::warn!(?candidates, "term.fonts: none of the candidate fonts is installed, using egui's monospace font");
    TermFonts::default()
}
