//! Fonts a host serves next to the app, for the platform shell that fetches them (the web build;
//! see `photocraft_text::served`): reading the manifest, the families to fetch, and installing
//! what arrived, which registers the files and re-renders the type layers drawn without them.
//! A served script fallback (Arabic, Japanese…) also joins the UI's font stack: the web build has
//! no system fonts, so it is what draws an Arabic or Japanese interface.

use std::sync::{Arc, Mutex, PoisonError};

use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{FontData, FontFamily};

use photocraft_doc::{DocId, LayerContent, LayerId, TextLayer};
pub use photocraft_text::served::{ServedFont, add_families, add_fonts, is_relative_path, parse_manifest, request_for_languages, take_requests};

use crate::PhotocraftApp;

/// Layers whose document was busy with a background job when their font arrived.
static PENDING: Mutex<Vec<(DocId, LayerId)>> = Mutex::new(Vec::new());

/// Served script fallbacks in the UI's font stack, by egui font name, unscaled: kept so the stack
/// `theme::install_fonts` rebuilds on a language change includes them again.
static UI_FACES: Mutex<Vec<(String, Arc<FontData>)>> = Mutex::new(Vec::new());

/// The served faces for the end of the UI's font stack (see [`UI_FACES`]), the interface
/// language's own first: a Japanese and a Simplified Chinese face both cover Han, and egui draws a
/// character with the first face in the stack that has it, so a Chinese interface must not get
/// the Japanese face only because it arrived first.
pub fn ui_faces() -> Vec<(String, Arc<FontData>)> {
    let mut faces = UI_FACES.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(own) = ui_language_face() {
        faces.sort_by_key(|(n, _)| *n != own);
    }
    faces
}

/// The egui font name of the interface language's served script fallback, if the host serves one.
fn ui_language_face() -> Option<String> {
    photocraft_text::served::fallback_for_language(crate::i18n::current().code()).map(|f| face_name(&f))
}

fn face_name(family: &str) -> String {
    format!("served-{family}")
}

/// Puts a delivered script fallback at the end of every UI font family, once. A file egui cannot
/// read is left out: egui panics on a configured font it cannot parse.
fn add_ui_face(ctx: &egui::Context, family: &str, bytes: &[u8]) {
    if !photocraft_text::cjk::font_face_is_readable(bytes, 0) {
        log::warn!("the served font {family:?} is not a readable font file; the interface does not use it");
        return;
    }
    let name = face_name(family);
    let mut data = {
        let mut faces = UI_FACES.lock().unwrap_or_else(PoisonError::into_inner);
        if faces.iter().any(|(n, _)| *n == name) {
            return;
        }
        let data = FontData::from_owned(bytes.to_vec());
        faces.push((name.clone(), Arc::new(data.clone())));
        data
    };
    // The interface language's face arriving after another served face must go ahead of it, which
    // appending cannot do: rebuild the stack, as a language change does.
    let others = ctx.fonts(|f| f.definitions().font_data.keys().any(|n| n.starts_with("served-")));
    if others && ui_language_face().as_ref() == Some(&name) {
        crate::theme::install_fonts(ctx);
        return;
    }
    crate::theme::size_ui_font(ctx, &name, &mut data);
    let mut families: Vec<FontFamily> = ctx.fonts(|f| f.definitions().families.keys().cloned().collect());
    for f in [FontFamily::Proportional, FontFamily::Monospace] {
        if !families.contains(&f) {
            families.push(f);
        }
    }
    let families = families.into_iter().map(|family| InsertFontFamily { family, priority: FontPriority::Lowest }).collect();
    ctx.add_font(FontInsert { name, data, families });
}

/// Registers delivered font files (`(family, bytes)`, the family as the manifest lists it) and
/// re-renders the type layers that use those families, or whose text falls back to them (a script
/// fallback, e.g. Arabic set in Inter), and show our own layout. A PSD layer still
/// showing Photoshop's pixels (not edited yet) keeps them, as it does when its font is installed
/// from the start. A script fallback also joins the UI's font stack. Call it every frame: it also
/// retries the layers of a document that was busy.
pub fn install(app: &mut PhotocraftApp, ctx: &egui::Context, delivered: Vec<(String, Vec<u8>)>) {
    let mut layers = std::mem::take(&mut *PENDING.lock().unwrap_or_else(PoisonError::into_inner));
    if !delivered.is_empty() {
        let families: Vec<String> = delivered.iter().map(|(f, _)| f.clone()).collect();
        // Decided before the fonts are in: "shows our own layout" compares the layer's pixels
        // with what the engine draws now, that is with the fallback font.
        for st in app.session.documents() {
            for (_, _, l) in st.doc.walk() {
                if let LayerContent::Text(t) = &l.content
                    && redraws_for(t, &families)
                    && crate::type_tool::shows_own_layout(&st.doc, t)
                    && !layers.contains(&(st.doc.id, l.id))
                {
                    layers.push((st.doc.id, l.id));
                }
            }
        }
        let fallbacks = photocraft_text::served::script_fallback_families();
        for (family, bytes) in &delivered {
            if fallbacks.contains(family) {
                add_ui_face(ctx, family, bytes);
            }
        }
        let mut eng = photocraft_text::shared().lock().unwrap_or_else(PoisonError::into_inner);
        for (family, bytes) in delivered {
            let added = eng.fonts.register_font_data(bytes);
            if !added.contains(&family) {
                log::warn!("a served font file listed as {family:?} registers {added:?}; the family in fonts/manifest.txt must be the font's own");
            }
        }
    }
    if !layers.is_empty() {
        let busy = app.session.refresh_type_layers(&layers);
        PENDING.lock().unwrap_or_else(PoisonError::into_inner).extend(busy);
    }
}

/// Should the layer be redrawn when `families` arrive: it uses one of them, or its text is in a
/// script one of them is the served fallback for?
fn redraws_for(t: &TextLayer, families: &[String]) -> bool {
    uses_family(t, families) || families.iter().any(|f| photocraft_text::served::falls_back_to(&t.text, f))
}

/// Does the layer use one of `families`, by name or through the PostScript name a PSD gave?
fn uses_family(t: &TextLayer, families: &[String]) -> bool {
    t.char_runs().iter().any(|r| {
        families.contains(&r.style.font_family)
            || r.style.postscript_name.as_deref().is_some_and(|ps| families.contains(&photocraft_text::fonts::guess_from_postscript(ps).family))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layer_uses_a_family_by_name_or_postscript_name() {
        let t = TextLayer { text: "Hi".into(), font_family: "Open Sans".into(), ..Default::default() };
        assert!(uses_family(&t, &["Open Sans".into()]));
        assert!(!uses_family(&t, &["Lato".into()]));
        let mut psd = TextLayer { text: "Hi".into(), font_family: "Inter".into(), ..Default::default() };
        psd.runs = vec![photocraft_doc::text::TextRun {
            len: 2,
            style: photocraft_doc::text::CharStyle { font_family: "Inter".into(), postscript_name: Some("Lato-Bold".into()), ..Default::default() },
        }];
        assert!(uses_family(&psd, &["Lato".into()]));
    }

    #[test]
    fn a_layer_in_a_script_redraws_when_that_scripts_fallback_arrives() {
        // Thai: other tests use other scripts (the served catalog is process-wide).
        let thai = "Served Test Thai";
        photocraft_text::served::add_fonts(&parse_manifest(&format!("{thai} | Regular | fonts/t/T.ttf | Thai\n")).0);
        let layer = |text: &str| TextLayer { text: text.into(), font_family: "Inter".into(), ..Default::default() };
        assert!(redraws_for(&layer("สวัสดี"), &[thai.into()]), "Thai text set in Inter falls back to it");
        assert!(!redraws_for(&layer("hello"), &[thai.into()]));
        assert!(redraws_for(&layer("hello"), &["Inter".into()]), "and a layer still redraws for its own family");
    }

    /// #10006: the web build has no system fonts, so a served Arabic fallback is what draws an
    /// Arabic interface, also after a language change rebuilds the UI's font stack.
    #[test]
    fn a_served_script_fallback_draws_the_ui_and_survives_a_font_rebuild() {
        // The web build's sources: no system fonts, no embedded craft fonts.
        let web = || crate::cjk_fonts::Sources {
            locale: || None,
            files: |_| Vec::new(),
            last_resort: Vec::new,
            pending: || false,
            scripts: |_| Vec::new(),
            covers: |_, _| None,
            embedded: crate::cjk_fonts::no_embedded,
            eager_craft_fonts: false,
        };
        let ctx = egui::Context::default();
        crate::theme::install_fonts_with(&ctx, web());
        ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
        // Font changes take effect on the next pass: one empty pass, then draw Arabic.
        let draw = |ctx: &egui::Context, text: &str| {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.label(text);
            })
            .textures_delta
            .clear();
        };
        draw(&ctx, "File");
        add_ui_face(&ctx, "Served Test Arabic", &crate::cjk_fonts::tests::synthetic_font(&[(0x0600, 0x06FF)]));
        draw(&ctx, "File");
        draw(&ctx, "ملف");
        // A language change rebuilds the stack; the served face comes back with it.
        crate::theme::install_fonts_with(&ctx, web());
        draw(&ctx, "File");
        draw(&ctx, "ملف");
    }

    /// #10006: a host serving a broken file as a script fallback must not stop the interface;
    /// egui panics on a configured font it cannot parse.
    #[test]
    fn an_unreadable_served_font_stays_out_of_the_ui() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let draw = |ctx: &egui::Context| {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.label("File ملف");
            })
            .textures_delta
            .clear();
        };
        draw(&ctx);
        add_ui_face(&ctx, "Served Test Broken", b"<html>not found</html>");
        draw(&ctx);
        crate::theme::install_fonts(&ctx);
        draw(&ctx);
        assert!(!ui_faces().iter().any(|(n, _)| n == "served-Served Test Broken"));
    }

    /// #10006: a Japanese and a Simplified Chinese face both cover Han. A Chinese interface draws
    /// it with the Chinese one, even when the Japanese face arrived first.
    #[test]
    fn the_interface_languages_served_face_comes_first_in_the_ui_stack() {
        let (jpan, hans) = ("Served Test Jpan", "Served Test Hans");
        photocraft_text::served::add_fonts(&parse_manifest(&format!("{jpan} | Regular | fonts/j/J.ttf | Jpan\n{hans} | Regular | fonts/h/H.ttf | Hans\n")).0);
        crate::i18n::set_current(crate::i18n::Lang::from_code("zh-hans").expect("Simplified Chinese"));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let draw = |ctx: &egui::Context| {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.label("文件");
            })
            .textures_delta
            .clear();
        };
        let order = |ctx: &egui::Context| {
            let stack = ctx.fonts(|f| f.definitions().families[&FontFamily::Proportional].clone());
            let at = |family: &str| stack.iter().position(|n| *n == format!("served-{family}"));
            (at(hans), at(jpan))
        };
        draw(&ctx);
        let han = crate::cjk_fonts::tests::synthetic_font(&[(0x4E00, 0x9FFF)]);
        add_ui_face(&ctx, jpan, &han);
        draw(&ctx);
        add_ui_face(&ctx, hans, &han);
        draw(&ctx);
        let (h, j) = order(&ctx);
        assert!(h.is_some() && h < j, "the Chinese face goes ahead of the Japanese one: {h:?} {j:?}");
        crate::theme::install_fonts(&ctx);
        draw(&ctx);
        let (h, j) = order(&ctx);
        assert!(h.is_some() && h < j, "also after a rebuild: {h:?} {j:?}");
        crate::i18n::set_current(crate::i18n::Lang::EN);
    }
}
