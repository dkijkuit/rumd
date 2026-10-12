//! Bundled fonts and their registration with egui.
//!
//! rumd ships Inter (SIL Open Font License 1.1 — see
//! `assets/fonts/Inter-OFL.txt`) for proportional text, replacing egui's
//! default Ubuntu-Light, which is thin (light weight) and has very tight
//! vertical metrics (1.15em line box), both of which made rendered
//! markdown read as a dense wall of text.
//!
//! Inter has no meaningful built-in line gap either (1.21em), so line
//! spacing is set explicitly per text (`TextFormat::line_height`) in the
//! vendored `egui_commonmark*` crates — the font files here only
//! provide the letterforms and real weights.
//!
//! egui has no bold/italic synthesis: `RichText::strong()` only changes
//! color. Real weights therefore live in separate font families that the
//! vendored markdown crates select per run (see the `rumd-*` families).

use std::sync::Arc;

use eframe::egui;

/// Marker stored in egui memory so [`install_once`] is a no-op after the
/// first call on a context.
fn installed_id() -> egui::Id {
    egui::Id::new("rumd_fonts_installed")
}

/// Font family names shared with the vendored `egui_commonmark*` crates.
/// Keep the literals there in sync with these constants.
pub const FAMILY_HEADING: &str = "rumd-heading";
pub const FAMILY_BOLD: &str = "rumd-bold";
pub const FAMILY_ITALIC: &str = "rumd-italic";
pub const FAMILY_BOLD_ITALIC: &str = "rumd-bold-italic";

/// Install rumd's fonts if they have not been installed on this context
/// yet. Safe (and cheap) to call every frame; the fonts become active at
/// the start of the next pass. Returns true when fonts were installed
/// (i.e. this pass still saw the old definitions).
pub fn install_once(ctx: &egui::Context) -> bool {
    let id = installed_id();
    if !ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
        install(ctx);
        ctx.data_mut(|d| d.insert_temp(id, true));
        true
    } else {
        false
    }
}

/// Register rumd's fonts. Call once at startup, before any text is laid
/// out. Replaces the primary proportional font with Inter and adds
/// real-weight families; the monospace family stays egui's bundled Hack.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let inter = |bytes: &'static [u8]| Arc::new(egui::FontData::from_static(bytes));
    fonts.font_data.insert(
        "Inter-Regular".into(),
        inter(include_bytes!("../assets/fonts/Inter-Regular.ttf")),
    );
    fonts.font_data.insert(
        "Inter-SemiBold".into(),
        inter(include_bytes!("../assets/fonts/Inter-SemiBold.ttf")),
    );
    fonts.font_data.insert(
        "Inter-Bold".into(),
        inter(include_bytes!("../assets/fonts/Inter-Bold.ttf")),
    );
    fonts.font_data.insert(
        "Inter-Italic".into(),
        inter(include_bytes!("../assets/fonts/Inter-Italic.ttf")),
    );
    fonts.font_data.insert(
        "Inter-BoldItalic".into(),
        inter(include_bytes!("../assets/fonts/Inter-BoldItalic.ttf")),
    );

    // Every custom family keeps the default emoji fallback tail; only the
    // primary face changes. Without the tail, emoji would vanish inside
    // bold/italic runs.
    let inter_first = |fonts: &egui::FontDefinitions, primary: &str| {
        let mut chain = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .cloned()
            .unwrap_or_default();
        if chain.is_empty() {
            chain.push(primary.to_owned());
        } else {
            // The default chain is [Ubuntu-Light, NotoEmoji, …]; the
            // second entry (Ubuntu-Light fallback for √ etc.) goes too —
            // Inter covers it and a lighter fallback would look off.
            chain[0] = primary.to_owned();
        }
        chain
    };

    let add_family = |fonts: &mut egui::FontDefinitions, name: &str, primary: &str| {
        fonts
            .families
            .insert(egui::FontFamily::Name(name.into()), inter_first(fonts, primary));
    };
    add_family(&mut fonts, FAMILY_HEADING, "Inter-SemiBold");
    add_family(&mut fonts, FAMILY_BOLD, "Inter-Bold");
    add_family(&mut fonts, FAMILY_ITALIC, "Inter-Italic");
    add_family(&mut fonts, FAMILY_BOLD_ITALIC, "Inter-BoldItalic");

    // Body and UI chrome: Inter Regular.
    if let Some(chain) = fonts.families.get_mut(&egui::FontFamily::Proportional)
        && !chain.is_empty()
    {
        chain[0] = "Inter-Regular".to_owned();
    }

    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_custom_families_with_fallback_tail() {
        let ctx = egui::Context::default();
        install(&ctx);
        ctx.begin_pass(Default::default());
        let chain_of = |family: &egui::FontFamily| -> Vec<String> {
            ctx.fonts(|f| {
                f.definitions()
                    .families
                    .get(family)
                    .cloned()
                    .unwrap_or_default()
            })
        };
        let proportional = chain_of(&egui::FontFamily::Proportional);
        assert_eq!(
            proportional.first().map(String::as_str),
            Some("Inter-Regular"),
            "proportional primary must be Inter"
        );
        assert!(
            proportional.len() > 1,
            "emoji fallback tail must survive the primary swap: {proportional:?}"
        );
        for (family, primary) in [
            (FAMILY_HEADING, "Inter-SemiBold"),
            (FAMILY_BOLD, "Inter-Bold"),
            (FAMILY_ITALIC, "Inter-Italic"),
            (FAMILY_BOLD_ITALIC, "Inter-BoldItalic"),
        ] {
            let chain = chain_of(&egui::FontFamily::Name(family.into()));
            assert_eq!(
                chain.first().map(String::as_str),
                Some(primary),
                "{family} family primary"
            );
            assert!(
                chain.len() > 1,
                "{family} family must keep the emoji fallback tail: {chain:?}"
            );
        }
        // Inter must be able to shape ordinary text through each family.
        for family in [
            egui::FontFamily::Proportional,
            egui::FontFamily::Name(FAMILY_HEADING.into()),
            egui::FontFamily::Name(FAMILY_BOLD.into()),
            egui::FontFamily::Name(FAMILY_ITALIC.into()),
            egui::FontFamily::Name(FAMILY_BOLD_ITALIC.into()),
        ] {
            let id = egui::FontId::new(16.0, family.clone());
            assert!(
                ctx.fonts_mut(|f| f.has_glyphs(&id, "rumd typography 123")),
                "family {family:?} cannot shape basic latin text"
            );
        }
        // The emoji fallback tail must be preserved by name: egui lazy-loads
        // fallback fonts, so a glyph probe before any layout would not see
        // them, but the chain entries are what real rendering resolves.
        assert!(
            proportional
                .iter()
                .any(|name| name.contains("Emoji") || name.contains("emoji")),
            "emoji fallback tail lost: {proportional:?}"
        );
        // Hand the font-texture delta to a no-op sink: without a real
        // renderer, dropping it unapplied panics.
        ctx.end_pass().textures_delta.clear();
    }
}
