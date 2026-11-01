//! The window's UI font: the system UI face, as the webview's
//! `'Segoe UI Variable', 'Segoe UI', system-ui, -apple-system, sans-serif`
//! resolved. egui's bundled fonts stay behind it as fallback.

use std::sync::atomic::{AtomicBool, Ordering};

use egui::FontFamily;
use egui::epaint::text::{FontData, FontInsert, FontPriority, InsertFontFamily};
use fontique::{
    Attributes, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, GenericFamily, QueryFamily,
    QueryStatus, SourceCache, SourceCacheOptions,
};

/// egui family name for the semibold UI face.
pub const SEMIBOLD: &str = "ezssh:ui-semibold";

static HAS_SEMIBOLD: AtomicBool = AtomicBool::new(false);

/// Family for semibold UI text: the system's semibold face when one was
/// registered, else the regular proportional family.
pub fn semibold_family() -> FontFamily {
    if HAS_SEMIBOLD.load(Ordering::Relaxed) { FontFamily::Name(SEMIBOLD.into()) } else { FontFamily::Proportional }
}

/// Named families tried before the generic `system-ui` / `sans-serif`.
fn preferred() -> &'static [&'static str] {
    if cfg!(windows) {
        &["Segoe UI Variable Text", "Segoe UI Variable", "Segoe UI"]
    } else {
        &[]
    }
}

fn find(collection: &mut Collection, cache: &mut SourceCache, family: QueryFamily<'_>, weight: f32) -> Option<(FontData, bool)> {
    let mut query = collection.query(cache);
    query.set_families([family]);
    query.set_attributes(Attributes::new(FontWidth::NORMAL, FontStyle::Normal, FontWeight::new(weight)));
    let mut found = None;
    query.matches_with(|font| {
        found = Some(font.clone());
        QueryStatus::Stop
    });
    let font = found?;
    let synthetic = font.synthesis.embolden();
    let (blob, _) = font.blob.into_raw_parts();
    Some((FontData::from_blob(blob, font.index), synthetic))
}

/// Register the system UI font as egui's proportional family (highest
/// priority) and its semibold face as [`SEMIBOLD`]. Returns the family
/// name used, or `None` when only egui's bundled font is available.
pub fn install(ctx: &egui::Context) -> Option<String> {
    let mut collection = Collection::new(CollectionOptions { shared: false, system_fonts: true });
    let mut cache = SourceCache::new(SourceCacheOptions::default());

    let mut chosen: Option<(String, QueryFamily<'static>)> = None;
    for name in preferred() {
        if collection.family_by_name(name).is_some() {
            chosen = Some((name.to_string(), QueryFamily::Named(name)));
            break;
        }
    }
    if chosen.is_none() {
        for generic in [GenericFamily::SystemUi, GenericFamily::SansSerif] {
            let id = collection.generic_families(generic).next();
            if let Some(id) = id
                && let Some(name) = collection.family_name(id)
            {
                chosen = Some((name.to_string(), QueryFamily::Id(id)));
                break;
            }
        }
    }
    let Some((name, family)) = chosen else {
        tracing::warn!("ui_font: no system UI font found; using egui's bundled font");
        return None;
    };

    let Some((regular, _)) = find(&mut collection, &mut cache, family, 400.0) else {
        tracing::warn!(font = %name, "ui_font: family listed but no regular face loaded");
        return None;
    };
    ctx.add_font(FontInsert {
        name: format!("{name} (UI)"),
        data: regular.clone(),
        families: vec![InsertFontFamily { family: FontFamily::Proportional, priority: FontPriority::Highest }],
    });
    match find(&mut collection, &mut cache, family, 600.0) {
        Some((semibold, false)) => {
            // The regular face goes behind it so the family is never empty
            // of Latin glyphs; egui's bundled fonts follow as usual.
            ctx.add_font(FontInsert {
                name: format!("{name} Semibold (UI)"),
                data: semibold,
                families: vec![InsertFontFamily { family: FontFamily::Name(SEMIBOLD.into()), priority: FontPriority::Highest }],
            });
            ctx.add_font(FontInsert {
                name: format!("{name} (UI semibold fallback)"),
                data: regular,
                families: vec![InsertFontFamily { family: FontFamily::Name(SEMIBOLD.into()), priority: FontPriority::Lowest }],
            });
            HAS_SEMIBOLD.store(true, Ordering::Relaxed);
        }
        other => {
            tracing::debug!(font = %name, synthetic = other.is_some(), "ui_font: no real semibold face; using regular");
        }
    }
    tracing::info!(font = %name, "ui_font: installed");
    Some(name)
}
