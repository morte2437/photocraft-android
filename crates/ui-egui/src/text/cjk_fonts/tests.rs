use super::*;
use egui::FontProvider;
use std::sync::Mutex;

#[test]
fn lazy_fallbacks_follow_ui_font_size_and_survive_size_changes() {
    use photocraft_engine::prefs::UiFontSize;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    crate::theme::set_ui_font_size(&ctx, UiFontSize::Large);
    let name = "test-lazy-fallback".to_string();
    add_to_all_families(&ctx, name.clone(), FontData::from_static(photocraft_text::fonts::INTER_REGULAR.as_slice()));
    for (size, scale) in [(UiFontSize::Large, 16.0 / 12.0), (UiFontSize::Tiny, 10.0 / 12.0), (UiFontSize::Small, 1.0)] {
        crate::theme::set_ui_font_size(&ctx, size);
        for _ in 0..2 {
            ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        }
        let fonts = ctx.fonts(|f| f.definitions().clone());
        let fallback = fonts.font_data.get(&name).unwrap();
        assert_eq!(fallback.tweak.scale, scale);
        assert_eq!(fallback.tweak.y_offset_factor, 0.0, "same face as Inter keeps its baseline");
        for stack in fonts.families.values() {
            assert_eq!(stack.iter().filter(|n| *n == &name).count(), 1);
            assert_eq!(stack.last(), Some(&name), "fallback stays at the lowest priority");
        }
    }
}

/// A face discovered by [`CraftFontProvider`] during a pass that also changes Interface > UI
/// Font Size must use the scale of the definitions active in that pass. The size plugin moves
/// the provider to the new scale only after `ctx.set_fonts` queued the rescaled definitions;
/// updating the provider in `set_ui_font_size` would put the new scale on a face laid out
/// with the old stack.
#[test]
fn discovered_faces_use_the_scale_of_the_definitions_they_were_laid_out_with() {
    use photocraft_engine::prefs::UiFontSize;
    fn thai_reader(_: &std::path::Path) -> Option<Vec<u8>> {
        Some(synthetic_font(&[(0x0E01, 0x0E5B)]))
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts_with(&ctx, craft_only(|| None));
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    let mut provider = CraftFontProvider::new(
        &Sources {
            locale: || Some("en".into()),
            files: no_files,
            last_resort: Vec::new,
            pending: || false,
            scripts: |script| if script == Script::Thai { vec![PathBuf::from("synthetic-thai.ttf")] } else { Vec::new() },
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: false,
        },
        1.0,
    );
    provider.reader = thai_reader;
    let provider = std::sync::Arc::new(provider);
    ctx.data_mut(|d| d.insert_temp(craft_provider_id(), provider.clone()));
    let as_provider: std::sync::Arc<dyn FontProvider> = provider.clone();
    ctx.set_font_providers(vec![as_provider]);

    // The scale of the configured Inter face (part of `FontDefinitions`) and of the face the
    // provider discovered for Thai.
    let inter_scale = |ctx: &egui::Context| -> Option<f32> { ctx.fonts(|f| f.definitions().font_data.get("Inter").map(|data| data.tweak.scale)) };
    let thai_scale = |ctx: &egui::Context| -> Option<f32> {
        ctx.fonts(|f| f.discovered_fonts().iter().find(|insert| insert.name.starts_with("craft-system-")).map(|insert| insert.data.tweak.scale))
    };
    // A frame that checks the state during the change pass, before laying out Thai (the
    // character that makes the provider discover a face).
    let frame = |check: &dyn Fn(&egui::Context)| {
        ctx.run_ui(Default::default(), |ui| {
            check(&ctx);
            ui.label("ก");
        })
        .textures_delta
        .clear();
    };
    // Small (the install default) is active. Walk Small -> Large -> Small, repeat Large
    // (the same size must leave both the definitions and the provider alone, not accumulate)
    // and back to Small.
    assert_eq!(provider.scale(), 1.0);
    for (size, want, same_size) in [
        (UiFontSize::Large, 16.0 / 12.0, false),
        (UiFontSize::Small, 1.0, false),
        (UiFontSize::Large, 16.0 / 12.0, false),
        (UiFontSize::Large, 16.0 / 12.0, true),
        (UiFontSize::Small, 1.0, false),
    ] {
        let old = provider.scale();
        let inter_before = inter_scale(&ctx);
        crate::theme::set_ui_font_size(&ctx, size);
        assert_eq!(provider.scale(), old, "the provider follows the definitions still active in this pass");
        frame(&|ctx: &egui::Context| {
            assert_eq!(inter_scale(ctx), inter_before, "the configured Inter keeps its old scale during the change pass");
            assert_eq!(provider.scale(), old, "the provider does not move before the pass ends");
        });
        assert_eq!(provider.scale(), want, "the provider follows the rescaled definitions");
        if same_size {
            assert_eq!(inter_scale(&ctx), inter_before, "a same-size request leaves the stack alone");
        }
        assert_eq!(thai_scale(&ctx), Some(old), "the pass laid the discovered face out with the old definitions");
        // The queued definitions with the new scale are active now, and a new pass drops the
        // discovered face and rediscovers it at the provider's new scale.
        frame(&|ctx: &egui::Context| {
            assert_eq!(inter_scale(ctx), Some(want), "the rescaled stack is active in the next pass");
            assert_eq!(provider.scale(), want);
        });
        assert_eq!(thai_scale(&ctx), Some(want), "the next pass rediscovers the face at the provider's scale");
    }
}

/// A font holding only an `fvar` table with the given axes (tag, min, default, max), as a
/// single face or as face 1 of a two-face collection.
fn fvar_font(axes: &[(&[u8; 4], f32, f32, f32)], collection: bool) -> Vec<u8> {
    let fixed = |v: f32| ((v * 65536.0) as i32).to_be_bytes();
    let mut fvar = Vec::new();
    for x in [1u16, 0, 16, 2, axes.len() as u16, 20, 0, 0] {
        fvar.extend(x.to_be_bytes());
    }
    for (tag, min, def, max) in axes {
        fvar.extend(*tag);
        for v in [*min, *def, *max] {
            fvar.extend(fixed(v));
        }
        fvar.extend([0, 0, 1, 0]);
    }
    let face = |at: u32| {
        let mut d = vec![0, 1, 0, 0, 0, 1, 0, 16, 0, 0, 0, 0];
        d.extend(b"fvar");
        d.extend([0; 4]);
        d.extend(at.to_be_bytes());
        d.extend((fvar.len() as u32).to_be_bytes());
        d
    };
    if !collection {
        let mut f = face(28);
        f.extend(&fvar);
        return f;
    }
    // ttcf header (12 + 2×4), two 28-byte directories, then the shared table.
    let mut f = b"ttcf".to_vec();
    f.extend([0, 1, 0, 0, 0, 0, 0, 2]);
    f.extend(20u32.to_be_bytes());
    f.extend(48u32.to_be_bytes());
    f.extend(face(76));
    f.extend(face(76));
    f.extend(&fvar);
    f
}

/// #4820: Noto Sans CJK's variable collection defaults to Thin; the fallback asks for Regular.
#[test]
fn variable_fallbacks_are_drawn_at_regular_weight() {
    let thin = fvar_font(&[(b"wght", 100.0, 100.0, 900.0)], true);
    assert_eq!(regular_weight(&thin, 1), Some(400.0), "Noto Sans CJK VF, face 1 of a collection");
    assert_eq!(regular_weight(&fvar_font(&[(b"wdth", 75.0, 100.0, 100.0), (b"wght", 100.0, 100.0, 900.0)], false), 0), Some(400.0));
    assert_eq!(regular_weight(&fvar_font(&[(b"wght", 500.0, 600.0, 900.0)], false), 0), Some(500.0), "Regular clamped into the axis");
    assert_eq!(regular_weight(&fvar_font(&[(b"wght", 100.0, 400.0, 900.0)], false), 0), None, "already Regular");
    assert_eq!(regular_weight(&fvar_font(&[(b"wdth", 75.0, 100.0, 100.0)], false), 0), None, "no weight axis");
    assert_eq!(regular_weight(photocraft_text::fonts::INTER_REGULAR.as_slice(), 0), None, "static font");
    for bad in [&thin[..thin.len() - 9], &thin[..30], &b"ttcf"[..], &[][..]] {
        assert_eq!(regular_weight(bad, 1), None);
    }
    // Registered as a fallback, a static face keeps the default (no variation) coordinates.
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    add_to_all_families(&ctx, "test-static".into(), FontData::from_static(photocraft_text::fonts::INTER_REGULAR.as_slice()));
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    let fonts = ctx.fonts(|f| f.definitions().clone());
    assert_eq!(fonts.font_data.get("test-static").map(|d| d.tweak.coords.clone()), Some(Default::default()));
}

/// A corrupt or non-font blob has no `hhea`, `head` or `fvar` table: the fallback tweaks
/// must leave it untouched rather than deriving a baseline or weight from garbage.
#[test]
fn font_tweaks_ignore_bytes_without_metrics_or_weight_axes() {
    let mut data = FontData::from_static(b"");
    apply_font_tweaks(&mut data);
    assert_eq!(data.tweak.y_offset_factor, 0.0);
    assert_eq!(data.tweak.coords, Default::default());
}

static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

#[test]
fn changing_language_resets_the_loader_once_and_reorders_scripts() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let ctx = egui::Context::default();
        crate::i18n::sync_context(&ctx, "ja");
        ctx.with_plugin::<CjkFontPlugin, _>(|plugin| {
            assert_eq!(plugin.0.order()[0], CjkScript::Japanese);
            plugin.0.tried = plugin.0.order().to_vec();
            plugin.0.last_resort_tried = true;
        })
        .expect("font plugin");
        crate::i18n::sync_context(&ctx, "ja");
        assert!(ctx.with_plugin::<CjkFontPlugin, _>(|plugin| plugin.0.exhausted()).expect("font plugin"), "idle frames must not reload fonts");
        crate::i18n::sync_context(&ctx, "ko");
        ctx.with_plugin::<CjkFontPlugin, _>(|plugin| {
            assert!(!plugin.0.exhausted());
            assert_eq!(plugin.0.order()[0], CjkScript::Korean);
        })
        .expect("font plugin");
    });
}

fn fake_dir() -> PathBuf {
    let mut g = DIR.lock().unwrap();
    g.get_or_insert_with(|| {
        let dir = std::env::temp_dir().join(format!("photocraft-cjk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("font.ttf"), include_bytes!("../../../../../assets/fonts/Inter-Regular.ttf")).unwrap();
        std::fs::write(dir.join("empty.ttf"), b"").unwrap();
        std::fs::write(dir.join("thai.ttf"), synthetic_font(&[(0x0E01, 0x0E5B)])).unwrap();
        std::fs::write(dir.join("arabic.ttf"), synthetic_font(&[(0x0600, 0x06FF)])).unwrap();
        std::fs::write(dir.join("devanagari.ttf"), synthetic_font(&[(0x0900, 0x097F)])).unwrap();
        std::fs::write(dir.join("korean.ttf"), synthetic_font(&[(0xCE74, 0xCE74)])).unwrap();
        std::fs::write(dir.join("tamil.ttf"), synthetic_font(&[(0x0B80, 0x0BFF)])).unwrap();
        std::fs::write(dir.join("ethiopic.ttf"), synthetic_font(&[(0x1200, 0x137F)])).unwrap();
        std::fs::write(dir.join("hant.ttf"), synthetic_font(&[(0x9F8D, 0x9F8D)])).unwrap();
        std::fs::write(
            dir.join("families.ttc"),
            two_face_ttc(
                include_bytes!("../../../../../assets/fonts/Inter-Regular.ttf"),
                include_bytes!("../../../../../assets/fonts/JetBrainsMono-Regular.ttf"),
            ),
        )
        .unwrap();
        dir
    })
    .clone()
}

fn fake(name: &str) -> FontFile {
    FontFile { path: fake_dir().join(name), family: String::new() }
}

#[test]
fn font_reads_skip_missing_empty_oversized_and_non_file_candidates() {
    let dir = fake_dir();
    assert!(read_font(&dir.join("missing-read-test.ttf")).is_none(), "a missing path is skipped");
    assert!(read_font(&dir).is_none(), "a directory is not a font file");
    assert!(read_font(&dir.join("empty.ttf")).is_none(), "an empty file is skipped");

    let oversized = dir.join("oversized-read-test.ttf");
    let file = std::fs::File::create(&oversized).expect("create a sparse oversized test file");
    file.set_len(MAX_FONT_BYTES + 1).expect("set the sparse file size");
    drop(file);
    assert!(read_font(&oversized).is_none(), "an oversized file is not read");
    std::fs::remove_file(&oversized).expect("remove the sparse test file");

    let valid = dir.join("font.ttf");
    assert_eq!(read_font(&valid).expect("the valid font is read").as_slice(), std::fs::read(&valid).expect("test font bytes").as_slice());
}

fn provider_corrupt_thai_candidates(script: Script) -> Vec<PathBuf> {
    if script == Script::Thai {
        vec![
            fake_dir().join("provider-missing-thai.ttf"),
            fake_dir().join("provider-corrupt-thai.ttf"),
            fake_dir().join("provider-corrupt-selected-thai.ttc"),
            fake_dir().join("font.ttf"),
            fake_dir().join("thai.ttf"),
        ]
    } else {
        Vec::new()
    }
}

fn provider_missing_thai_candidate(script: Script) -> Vec<PathBuf> {
    if script == Script::Thai { vec![fake_dir().join("provider-only-missing-thai.ttf")] } else { Vec::new() }
}

/// The provider's non-CJK script route must reject a structurally invalid file, a readable
/// collection whose selected covering face is unreadable, and a missing file, then use the
/// next real candidate. The rejected and missing paths stay negative-cached.
#[test]
fn provider_skips_missing_and_corrupt_script_candidates_before_registering() {
    let malformed = fake_dir().join("provider-corrupt-thai.ttf");
    let collection = fake_dir().join("provider-corrupt-selected-thai.ttc");
    std::fs::write(&malformed, font_with_out_of_bounds_table()).expect("write the malformed test font");
    std::fs::write(&collection, two_face_ttc(include_bytes!("../../../../../assets/fonts/Inter-Regular.ttf"), &font_with_out_of_bounds_table()))
        .expect("write the malformed selected-face collection");

    let sources = Sources {
        locale: || Some("en".into()),
        files: no_files,
        last_resort: Vec::new,
        pending: || false,
        scripts: provider_corrupt_thai_candidates,
        covers: scripts::covering_face,
        embedded: no_embedded,
        eager_craft_fonts: false,
    };
    let provider = CraftFontProvider::new(&sources, 1.0);
    let insert = provided_insert(&provider, 'ก').expect("the valid Thai candidate is reached");
    assert_eq!(insert.data.index, 0);
    assert_eq!(insert.data.bytes(), std::fs::read(fake_dir().join("thai.ttf")).expect("Thai test font").as_slice());
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(cache.failed_reads.contains(&fake_dir().join("provider-missing-thai.ttf")), "the missing file is remembered");
    assert!(cache.invalid_fonts.contains(&malformed), "the readable non-font is remembered as invalid");
    assert!(cache.files.iter().any(|file| file.path == collection), "the readable collection is retained for its other faces");
    drop(cache);

    let missing_only = Sources { scripts: provider_missing_thai_candidate, ..sources };
    let provider = CraftFontProvider::new(&missing_only, 1.0);
    assert!(provided(&provider, 'ก').is_none(), "no readable script candidate leaves the character missing");
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(cache.failed_reads.len(), 1);
    assert_eq!(cache.reads, 0);
}

fn fake_sources(locale: fn() -> Option<String>) -> Sources {
    Sources {
        locale,
        // Korean has a readable Hangul font, Japanese only broken files, Chinese none.
        files: |s| match s {
            CjkScript::Korean => vec![fake("missing.ttc"), fake("empty.ttf"), fake("korean.ttf")],
            CjkScript::Japanese => vec![fake("empty.ttf")],
            _ => vec![],
        },
        last_resort: || vec![fake("font.ttf")],
        pending: || false,
        scripts: |_| Vec::new(),
        covers: scripts::covering_face,
        embedded: no_embedded,
        eager_craft_fonts: false,
    }
}

#[test]
fn picks_preferred_script_skips_unreadable_and_exhausts() {
    let mut fb = CjkFallback::new(fake_sources(|| Some("en_US".into())));
    assert!(!fb.exhausted());
    // Hangul: Korean first; missing and empty files are skipped.
    let (name, data) = fb.next_font('카', CjkChar::Hangul).expect("korean font");
    assert!(name.starts_with(FONT_PREFIX));
    assert_eq!(data.index, 0);
    assert_eq!(fb.tried, vec![CjkScript::Korean]);
    // Han on an English locale: system fallback keeps the locale order (SC first); the
    // Japanese-first craft-manifest baseline is a separate provider/lazy-craft path.
    // SC, TC, JA have nothing readable here, and the last-resort file was already loaded,
    // so nothing new; everything is now tried.
    assert!(fb.next_font('日', CjkChar::Han).is_none());
    assert!(fb.exhausted());
    assert_eq!(fb.tried, vec![CjkScript::Korean, CjkScript::SimplifiedChinese, CjkScript::TraditionalChinese, CjkScript::Japanese]);
    assert_eq!(fb.registered.len(), 1, "a file is read at most once");
    assert!(fb.next_font('あ', CjkChar::Kana).is_none());
}

#[test]
fn han_follows_locale() {
    for (loc, first) in [
        ("ja_JP.UTF-8", CjkScript::Japanese),
        ("zh-Hans-CN", CjkScript::SimplifiedChinese),
        ("zh-Hant-TW", CjkScript::TraditionalChinese),
        ("ko_KR", CjkScript::Korean),
    ] {
        let mut fb = CjkFallback::new(Sources {
            locale: || None,
            files: |_| vec![],
            last_resort: Vec::new,
            pending: || false,
            scripts: |_| Vec::new(),
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: false,
        });
        fb.order = Some(cjk::script_order(Some(loc)));
        assert!(fb.next_font('日', CjkChar::Han).is_none());
        assert_eq!(fb.tried[0], first, "{loc}");
    }
}

/// Runs frames showing `text` until fonts settle; returns whether every glyph is covered.
fn render(ctx: &egui::Context, text: &str) -> bool {
    for _ in 0..12 {
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.label(text);
        });
        out.textures_delta.clear();
    }
    ctx.fonts_mut(|f| f.has_glyphs(&FontId::proportional(12.0), &text.replace(' ', "")))
}

/// A two-face TrueType collection built at runtime from the repository's OFL-licensed
/// Inter and JetBrains Mono files. Each face directory is rewritten with offsets relative to
/// the collection, so no font fixture is committed.
fn two_face_ttc(first: &[u8], second: &[u8]) -> Vec<u8> {
    let font_count = 2usize;
    let tables = |font: &[u8]| u16::from_be_bytes([font[4], font[5]]) as usize;
    let dir_lengths = [12 + 16 * tables(first), 12 + 16 * tables(second)];
    let mut dir_offsets = Vec::with_capacity(font_count);
    let mut cursor = 12 + 4 * font_count;
    for length in dir_lengths {
        dir_offsets.push(cursor);
        cursor += length;
    }
    let data_start = cursor.next_multiple_of(4);
    let mut bases = Vec::with_capacity(font_count);
    let mut total = data_start;
    for font in [first, second] {
        bases.push(total);
        total += font.len().next_multiple_of(4);
    }
    let mut out = vec![0u8; total];
    out[..4].copy_from_slice(b"ttcf");
    out[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    out[8..12].copy_from_slice(&u32::try_from(font_count).expect("two faces").to_be_bytes());
    for (i, offset) in dir_offsets.iter().enumerate() {
        let at = 12 + i * 4;
        out[at..at + 4].copy_from_slice(&u32::try_from(*offset).expect("collection fits").to_be_bytes());
    }
    for (i, font) in [first, second].into_iter().enumerate() {
        let dir = dir_offsets[i];
        let base = bases[i];
        out[dir..dir + 12].copy_from_slice(&font[..12]);
        for record in 0..tables(font) {
            let src = 12 + 16 * record;
            let dst = dir + 12 + 16 * record;
            out[dst..dst + 8].copy_from_slice(&font[src..src + 8]);
            let table_offset = u32::from_be_bytes(font[src + 8..src + 12].try_into().expect("four bytes"));
            let collection_offset = u32::try_from(base).expect("collection fits") + table_offset;
            out[dst + 8..dst + 12].copy_from_slice(&collection_offset.to_be_bytes());
            out[dst + 12..dst + 16].copy_from_slice(&font[src + 12..src + 16]);
        }
        out[base..base + font.len()].copy_from_slice(font);
    }
    out
}

/// A minimal TrueType font mapping the `ranges` (inclusive) to box glyphs, so tests have a
/// font for a script without committing font files.
pub(crate) fn synthetic_font(ranges: &[(u32, u32)]) -> Vec<u8> {
    let be16 = |v: &mut Vec<u8>, x: i32| v.extend((x as u16).to_be_bytes());
    let be32 = |v: &mut Vec<u8>, x: u32| v.extend(x.to_be_bytes());
    let chars: u32 = ranges.iter().map(|(a, b)| b - a + 1).sum();
    let glyphs = chars as i32 + 1;
    // One 500x700 box (contour of four on-curve points); glyph 0 (.notdef) is empty.
    let mut boxed = Vec::new();
    for x in [1, 50, 0, 550, 700, 3, 0] {
        be16(&mut boxed, x);
    }
    boxed.extend([1u8; 4]);
    for x in [50, 0, 500, 0, 0, 700, 0, -700] {
        be16(&mut boxed, x);
    }
    boxed.extend([0, 0]);
    let glyf: Vec<u8> = boxed.repeat(chars as usize);
    let mut loca = Vec::new();
    be32(&mut loca, 0);
    for i in 0..chars {
        be32(&mut loca, i * boxed.len() as u32);
    }
    be32(&mut loca, chars * boxed.len() as u32);
    let mut head = Vec::new();
    for x in [0x0001_0000, 0x0001_0000, 0, 0x5F0F_3CF5] {
        be32(&mut head, x);
    }
    for x in [0x000B, 1000, 0, 0, 0, 0, 0, 0, 0, 0, 0, -200, 600, 800, 0, 8, 2, 1, 0] {
        be16(&mut head, x);
    }
    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    for x in [800, -200, 0, 600, 0, 0, 550, 1, 0, 0, 0, 0, 0, 0, 0, glyphs] {
        be16(&mut hhea, x);
    }
    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    for x in [glyphs, 4, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut maxp, x);
    }
    let mut hmtx = Vec::new();
    for _ in 0..glyphs {
        be16(&mut hmtx, 600);
        be16(&mut hmtx, 50);
    }
    let mut cmap = Vec::new();
    for x in [0, 1, 3, 10] {
        be16(&mut cmap, x);
    }
    be32(&mut cmap, 12);
    be16(&mut cmap, 12);
    be16(&mut cmap, 0);
    be32(&mut cmap, 16 + 12 * ranges.len() as u32);
    be32(&mut cmap, 0);
    be32(&mut cmap, ranges.len() as u32);
    let mut gid = 1;
    for (a, b) in ranges {
        for x in [*a, *b, gid] {
            be32(&mut cmap, x);
        }
        gid += b - a + 1;
    }
    let tables: [(&[u8; 4], Vec<u8>); 7] =
        [(b"cmap", cmap), (b"glyf", glyf), (b"head", head), (b"hhea", hhea), (b"hmtx", hmtx), (b"loca", loca), (b"maxp", maxp)];
    let mut font = Vec::new();
    be32(&mut font, 0x0001_0000);
    for x in [7, 64, 2, 48] {
        be16(&mut font, x);
    }
    let mut offset = 12 + 16 * tables.len() as u32;
    for (tag, data) in &tables {
        font.extend(*tag);
        be32(&mut font, 0);
        be32(&mut font, offset);
        be32(&mut font, data.len() as u32);
        offset += (data.len() as u32).next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    font
}

/// Thai, Arabic and Devanagari each have a synthetic font; Thai's list starts with unreadable
/// files and Inter, which doesn't map Thai. No other script has a font.
fn script_sources() -> Sources {
    Sources {
        locale: || Some("en".into()),
        files: |_| vec![],
        last_resort: Vec::new,
        pending: || false,
        scripts: |s| {
            let f = |name: &str| fake_dir().join(name);
            match s {
                Script::Thai => vec![f("missing.ttf"), f("empty.ttf"), f("font.ttf"), f("thai.ttf")],
                Script::Arabic => vec![f("arabic.ttf")],
                Script::Devanagari => vec![f("devanagari.ttf")],
                _ => vec![],
            }
        },
        covers: scripts::covering_face,
        embedded: no_embedded,
        eager_craft_fonts: false,
    }
}

fn fallback_files(ctx: &egui::Context) -> Vec<String> {
    ctx.with_plugin::<CjkFontPlugin, _>(|p| p.0.registered.iter().map(|(_, path, _)| path.file_name().unwrap().to_string_lossy().into_owned()).collect())
        .unwrap()
}

#[test]
fn layer_names_in_other_scripts_load_a_covering_font_once() {
    let ctx = egui::Context::default();
    for _ in 0..2 {
        crate::theme::install_fonts_with(&ctx, script_sources());
        assert!(render(&ctx, "Layer 1 – café"));
        ctx.with_plugin::<CjkFontPlugin, _>(|p| assert!(p.0.scripts_tried.is_empty(), "Latin text reads no font")).unwrap();
        assert!(fallback_files(&ctx).is_empty());
        // A Thai type layer's name (#2275), Arabic (#1693) and Devanagari: no missing glyphs.
        for text in ["สวัสดี", "ภาพถ่าย/วันหยุดพักผ่อน", "مرحبا بالعالم", "नमस्ते दुनिया"]
        {
            assert!(render(&ctx, text), "tofu in {text}");
        }
        assert_eq!(fallback_files(&ctx), ["thai.ttf", "arabic.ttf", "devanagari.ttf"], "the first file that maps the character, never Inter");
        // Hebrew has no font here: tried once, then left alone (no reload or repaint loop).
        assert!(!render(&ctx, "שלום"));
        assert!(!render(&ctx, "שלום"));
        ctx.with_plugin::<CjkFontPlugin, _>(|p| {
            assert!(p.0.scripts_tried.contains(&Script::Hebrew));
            assert_eq!(p.0.registered.len(), 3);
        })
        .unwrap();
        let fonts = ctx.fonts(|f| f.definitions().clone());
        assert_eq!(fonts.families[&FontFamily::Proportional].first().map(String::as_str), Some("Inter"));
        for stack in fonts.families.values() {
            assert_eq!(stack.iter().filter(|n| n.starts_with(FONT_PREFIX)).count(), 3);
            assert_eq!(stack.last().map(String::as_str), Some("system-cjk-2"), "appended last so Latin keeps Inter");
        }
    }
}

#[test]
fn a_missing_cjk_font_does_not_block_other_scripts_and_vice_versa() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts_with(&ctx, script_sources());
    assert!(!render(&ctx, "日本語/ภาพถ่าย/วันหยุดพักผ่อน.png"));
    assert!(render(&ctx, "ภาพถ่าย/วันหยุดพักผ่อน.png"));
    ctx.with_plugin::<CjkFontPlugin, _>(|p| assert!(p.0.exhausted())).unwrap();
    assert_eq!(fallback_files(&ctx), ["thai.ttf"]);
    // A script with no font leaves CJK available.
    let mut fallback = CjkFallback::new(fake_sources(|| Some("ko".into())));
    assert!(fallback.next_script_font('ส').is_none());
    assert!(fallback.next_script_font('ส').is_none());
    assert!(fallback.next_script_font('A').is_none(), "Latin is never a fallback script");
    assert_eq!(fallback.scripts_tried, [Script::Thai]);
    assert!(fallback.next_font('카', CjkChar::Hangul).is_some());
}

#[test]
fn script_candidates_are_read_a_bounded_number_of_times() {
    static CHECKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut fallback = CjkFallback::new(Sources {
        scripts: |_| (0..20).map(|i| fake_dir().join(if i % 2 == 0 { "font.ttf" } else { "thai.ttf" })).collect(),
        covers: |_, _| {
            CHECKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            None
        },
        ..script_sources()
    });
    assert!(fallback.next_script_font('ش').is_none());
    assert!(fallback.next_script_font('ش').is_none());
    assert_eq!(CHECKED.load(std::sync::atomic::Ordering::Relaxed), MAX_SCRIPT_READS, "one bounded pass per script");
    assert!(fallback.registered.is_empty());
    assert!(!fallback.scripts_exhausted());
}

#[test]
fn system_fonts_cover_other_script_samples() {
    // Like the CJK system-font test, run per script when this machine has a font for it.
    // The deterministic tests above do not need installed fonts.
    let samples = [("ภาพถ่าย/วันหยุดพักผ่อน", 'ภ'), ("مرحبا", 'م'), ("שלום", 'ש'), ("नमस्ते", 'न')];
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    for (text, c) in samples {
        let Some(script) = scripts::classify(c) else { continue };
        if !scripts::system_candidates(script).iter().any(|p| read_font(p).and_then(|b| scripts::covering_face(&b, c)).is_some()) {
            eprintln!("skipping {text}: no {script:?} system font");
            continue;
        }
        assert!(render(&ctx, text), "{text} still has missing glyphs");
    }
}

#[test]
fn vertical_metrics_parse_and_reject_garbage() {
    let (asc, desc, gap) = vertical_metrics(include_bytes!("../../../../../assets/fonts/Inter-Regular.ttf"), 0).unwrap();
    assert!((asc - 1984.0 / 2048.0).abs() < 1e-4 && (desc + 494.0 / 2048.0).abs() < 1e-4 && gap == 0.0);
    for bad in [&b""[..], b"ttcf", b"ttcf\0\0\0\0\0\0\0\0\xff\xff\xff\xff", b"\0\x01\0\0\0\xff\0\0\0\0\0\0"] {
        assert!(vertical_metrics(bad, 0).is_none());
        assert!(vertical_metrics(bad, u32::MAX).is_none());
    }
}

#[test]
fn a_face_with_a_big_line_gap_is_shifted_down_onto_the_primary_baseline() {
    let inter = (1984.0 / 2048.0, -494.0 / 2048.0, 0.0);
    let hiragino = (0.88, -0.12, 0.5);
    assert!((baseline_offset(hiragino, inter) - 0.2338).abs() < 1e-3);
    assert_eq!(baseline_offset(inter, inter), 0.0);
    assert_eq!(baseline_offset((f32::NAN, 0.0, 0.0), inter), 0.0);
    let jb = (1.02, -0.3, 0.0);
    assert!((baseline_offset(hiragino, jb) - baseline_offset(hiragino, inter)).abs() < 0.005, "one offset serves the monospace family too");
}

#[test]
fn lazy_registration_triggers_only_for_supported_scripts() {
    let ctx = egui::Context::default();
    install_with(&ctx, fake_sources(|| Some("ko".into())));
    let _ = render(&ctx, "Layer 1 – café");
    let n = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
    assert_eq!(n, 0, "Latin text loads nothing");
    // The Korean candidate covers the first missing Hangul character and is registered.
    let _ = render(&ctx, "카드 배경");
    let fonts = ctx.fonts(|f| f.definitions().clone());
    let name = format!("{FONT_PREFIX}-0");
    assert!(fonts.font_data.contains_key(&name));
    for (fam, stack) in &fonts.families {
        assert_eq!(stack.last(), Some(&name), "{fam:?}: appended last so Latin keeps Inter");
    }
}

/// Craft fonts available through the first-frame provider, as the desktop shell does.
fn eager_sources(locale: fn() -> Option<String>) -> Sources {
    Sources {
        locale,
        files: |_| vec![],
        last_resort: Vec::new,
        pending: || false,
        scripts: |_| Vec::new(),
        covers: scripts::covering_face,
        embedded: craft_embedded_iso,
        eager_craft_fonts: true,
    }
}

/// No system fonts at all: craft-fonts alone draws CJK through the lazy loader.
fn craft_only(locale: fn() -> Option<String>) -> Sources {
    Sources {
        locale,
        files: |_| vec![],
        last_resort: Vec::new,
        pending: || false,
        scripts: |_| Vec::new(),
        covers: scripts::covering_face,
        embedded: craft_embedded_iso,
        eager_craft_fonts: false,
    }
}

/// The migration baseline's first craft face covering `c`: the first non-empty face in
/// manifest order (what the old family stack appended first), not the UI-first sort.
fn baseline_face(c: char) -> Option<&'static CraftFont> {
    craft_fonts::CRAFT_FONTS.iter().find(|f| !f.bytes.is_empty() && scripts::face_covers(f.bytes, 0, c))
}

/// A test-only craft face backed by a collection built with [`two_face_ttc`]: face 0 maps
/// Thai only, face 1 maps 龍. `craft_insert` would install face 0, so the craft coverage
/// helpers must not accept it for 龍 even though a later face covers the character.
fn ttc_craft_face() -> &'static CraftFont {
    static FACE: std::sync::OnceLock<CraftFont> = std::sync::OnceLock::new();
    FACE.get_or_init(|| {
        let bytes: &'static [u8] = Box::leak(two_face_ttc(&synthetic_font(&[(0x0E01, 0x0E5B)]), &synthetic_font(&[(0x9F8D, 0x9F8D)])).into_boxed_slice());
        CraftFont { family: "Synthetic Collection", style: "Regular", scripts: &["Hans"], bytes }
    })
}

/// The craft helpers must test face 0, the face `FontData::from_static` installs. The old
/// any-face coverage check returned the face for a character only a later collection face
/// maps, which then drew tofu in the installed face 0.
#[test]
fn craft_face_coverage_matches_the_face_craft_insert_installs() {
    let face = ttc_craft_face();
    // Precondition: an any-face check really accepts 龍 (face 1), so this regression would
    // fail if the helpers went back to `scripts::covering_face`.
    assert_eq!(scripts::covering_face(face.bytes, '龍'), Some(1), "face 1 maps 龍");
    assert!(!scripts::face_covers(face.bytes, 0, '龍'), "the installed face 0 does not map 龍");
    assert!(!craft_face_installs_coverage(face, '龍'), "face 0 lacks the glyph; craft_insert installs face 0");
    assert!(craft_face_installs_coverage(face, 'ก'), "face 0 maps Thai");
}

static EMPTY_CRAFT_FACE: CraftFont = CraftFont { family: "Empty", style: "Regular", scripts: &["Zsym"], bytes: &[] };

fn only_empty_craft_face(_: &'static str) -> Vec<&'static CraftFont> {
    vec![&EMPTY_CRAFT_FACE]
}

/// A craft entry without bytes, or a face injected only by a test that is absent from the
/// global manifest, has no installable face index: selection must return `None`, not a face
/// name egui cannot resolve.
#[test]
fn craft_face_selection_ignores_empty_and_unindexed_faces() {
    assert!(!craft_face_installs_coverage(&EMPTY_CRAFT_FACE, '☃'));
    assert!(craft_face_for(only_empty_craft_face, "Zsym", '☃').is_none());
    assert!(craft_face_manifest(only_empty_craft_face, "Zsym", '☃').is_none());
}

/// Every script the lazy fallback can see has a valid ISO 15924 tag, so its injected craft
/// faces and the CJK scripts are selected by the same tags the manifest declares.
#[test]
fn fallback_scripts_map_to_iso_15924_tags() {
    assert_eq!(cjk_iso(CjkScript::Japanese), "Jpan");
    assert_eq!(cjk_iso(CjkScript::SimplifiedChinese), "Hans");
    assert_eq!(cjk_iso(CjkScript::TraditionalChinese), "Hant");
    assert_eq!(cjk_iso(CjkScript::Korean), "Kore");
    let expected = [
        (Script::Thai, "Thai"),
        (Script::Lao, "Laoo"),
        (Script::Khmer, "Khmr"),
        (Script::Myanmar, "Mymr"),
        (Script::Tibetan, "Tibt"),
        (Script::Arabic, "Arab"),
        (Script::Hebrew, "Hebr"),
        (Script::Armenian, "Armn"),
        (Script::Georgian, "Geor"),
        (Script::Ethiopic, "Ethi"),
        (Script::Devanagari, "Deva"),
        (Script::Bengali, "Beng"),
        (Script::Gurmukhi, "Guru"),
        (Script::Gujarati, "Gujr"),
        (Script::Oriya, "Orya"),
        (Script::Tamil, "Taml"),
        (Script::Telugu, "Telu"),
        (Script::Kannada, "Knda"),
        (Script::Malayalam, "Mlym"),
        (Script::Sinhala, "Sinh"),
    ];
    assert_eq!(expected.len(), Script::ALL.len(), "the test lists every fallback script");
    for (script, iso) in expected {
        assert_eq!(script_iso(script), iso, "{script:?}");
    }
    assert_eq!(craft_embedded(CjkScript::Japanese).len(), craft_embedded_iso("Jpan").len());
}

/// No system fonts at all: craft-fonts alone draws CJK, in the locale's script order.
#[test]
fn craft_fonts_render_cjk_without_system_fonts() {
    let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied() else {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    };
    let Some(chinese) = craft_fonts::CRAFT_FONTS.iter().find(|f| f.family == "Noto Sans CJK SC" && f.style == "Regular") else {
        eprintln!("skipping: the craft-fonts manifest has no Simplified Chinese face");
        return;
    };
    // The craft manifest's Japanese faces come before Noto Sans CJK SC and the baseline
    // appended them in manifest order, so a non-CJK/unknown effective locale keeps Shippori
    // Mincho (the first covering face). An explicit locale keeps its own UI-first script face.
    let Some(baseline) = baseline_face('日') else {
        eprintln!("skipping: the craft-fonts manifest has no face covering 日");
        return;
    };
    for (locale, expected) in [
        ((|| Some("en_US".into())) as fn() -> Option<String>, baseline),
        ((|| Some("ja_JP".into())) as fn() -> Option<String>, ui_font),
        ((|| Some("zh-CN".into())) as fn() -> Option<String>, chinese),
        ((|| None) as fn() -> Option<String>, baseline),
    ] {
        let ctx = egui::Context::default();
        crate::theme::install_fonts_with(&ctx, craft_only(locale));
        assert!(render(&ctx, "日本語の文字"), "tofu in 日本語の文字");
        assert!(render(&ctx, "レイヤー 1"), "tofu in kana");
        let fonts = ctx.fonts(|f| f.definitions().clone());
        let name = format!("{FONT_PREFIX}-0");
        assert!(
            fonts.font_data.get(&name).is_some_and(|d| std::ptr::eq(d.bytes().as_ptr(), expected.bytes.as_ptr())),
            "{} first by locale: {name}",
            expected.family
        );
        for (fam, stack) in &fonts.families {
            assert_eq!(stack.last(), Some(&name), "{fam:?}: appended last so Latin keeps Inter");
            assert!(stack.first().is_some_and(|f| f != &name), "{fam:?}");
        }
    }
}

fn provided_insert(provider: &CraftFontProvider, c: char) -> Option<FontInsert> {
    let mut buf = [0u8; 4];
    let cluster = c.encode_utf8(&mut buf);
    provider.font_for(&FallbackRequest { cluster, family: &FontFamily::Proportional })
}

fn provided(provider: &CraftFontProvider, c: char) -> Option<String> {
    provided_insert(provider, c).map(|insert| insert.name)
}

fn test_sources(locale: fn() -> Option<String>) -> Sources {
    Sources {
        locale,
        files: no_files,
        last_resort: Vec::new,
        pending: || false,
        scripts: no_script_files,
        covers: scripts::covering_face,
        embedded: craft_embedded_iso,
        eager_craft_fonts: false,
    }
}

fn test_provider(locale: fn() -> Option<String>) -> CraftFontProvider {
    CraftFontProvider::new(&test_sources(locale), 1.0)
}

#[test]
fn effective_locale_prefers_the_translated_ui_then_the_os_locale() {
    assert_eq!(effective_locale("ja", Some("en_US.UTF-8")), Some("ja".into()));
    assert_eq!(effective_locale("zh-hant", Some("ja_JP.UTF-8")), Some("zh-hant".into()));
    assert_eq!(effective_locale("en", Some("ja_JP.UTF-8")), Some("ja_JP.UTF-8".into()), "English UI on a Japanese OS uses the OS locale");
    assert_eq!(effective_locale("de", Some("ko_KR")), Some("ko_KR".into()));
    assert_eq!(effective_locale("en", None), None, "no UI preference and no OS locale stays unknown");
}

fn no_script_files(_: Script) -> Vec<PathBuf> {
    Vec::new()
}

fn no_files(_: CjkScript) -> Vec<FontFile> {
    Vec::new()
}

/// A stand-in system Traditional Chinese file, so the system-before-SC-craft ordering is
/// testable without depending on the host's fonts. Its content is the SC craft face (which
/// covers the sample Han character).
fn hant_system_file() -> Option<PathBuf> {
    static FILE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    let face = craft_fonts::CRAFT_FONTS.iter().find(|f| f.family == "Noto Sans CJK SC" && !f.bytes.is_empty())?;
    FILE.get_or_init(|| {
        let path = std::env::temp_dir().join(format!("photocraft-hant-{}.otf", std::process::id()));
        std::fs::write(&path, face.bytes).ok().map(|()| path)
    })
    .clone()
}

fn hant_files(script: CjkScript) -> Vec<FontFile> {
    if script == CjkScript::TraditionalChinese {
        hant_system_file().map(|path| vec![FontFile { path, family: String::new() }]).unwrap_or_default()
    } else {
        Vec::new()
    }
}

#[test]
fn craft_font_provider_keeps_the_baseline_face_for_unknown_locales() {
    let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied() else {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    };
    let Some(ui_index) = craft_index(ui_font) else { return };
    let Some((sc_index, sc)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans CJK SC") else {
        eprintln!("skipping: the craft-fonts manifest has no Simplified Chinese face");
        return;
    };
    let Some((korean_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "NanumGothic") else {
        eprintln!("skipping: the craft-fonts manifest has no Korean face");
        return;
    };
    let Some((arabic_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans Arabic") else {
        eprintln!("skipping: the craft-fonts manifest has no Arabic face");
        return;
    };
    let Some((devanagari_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans Devanagari") else {
        eprintln!("skipping: the craft-fonts manifest has no Devanagari face");
        return;
    };
    let Some(baseline_han) = baseline_face('日') else {
        eprintln!("skipping: the craft-fonts manifest has no face covering 日");
        return;
    };
    let Some(baseline_kana) = baseline_face('あ') else {
        eprintln!("skipping: the craft-fonts manifest has no face covering あ");
        return;
    };
    let baseline_han_index = craft_index(baseline_han).expect("a manifest face has an index");
    let baseline_kana_index = craft_index(baseline_kana).expect("a manifest face has an index");
    let baseline_han_name = format!("craft-font-{baseline_han_index}");
    let baseline_kana_name = format!("craft-font-{baseline_kana_index}");
    let ui_name = format!("craft-font-{ui_index}");
    let sc_name = format!("craft-font-{sc_index}");
    let korean_name = format!("craft-font-{korean_index}");
    let arabic_name = format!("craft-font-{arabic_index}");
    let devanagari_name = format!("craft-font-{devanagari_index}");

    let ja = test_provider((|| Some("ja_JP".into())) as fn() -> Option<String>);
    assert_eq!(provided(&ja, 'あ').as_deref(), Some(ui_name.as_str()), "explicit ja keeps the Japanese UI face");
    assert_eq!(provided(&ja, '日').as_deref(), Some(ui_name.as_str()), "Han follows an explicit Japanese locale");

    let en = test_provider((|| Some("en_US".into())) as fn() -> Option<String>);
    assert_eq!(
        provided(&en, 'あ').as_deref(),
        Some(baseline_kana_name.as_str()),
        "kana in a non-CJK effective locale keeps the manifest's first covering face"
    );
    assert_eq!(provided(&en, '한').as_deref(), Some(korean_name.as_str()), "Hangul in a non-CJK effective locale stays on the Korean face");
    assert_eq!(provided(&en, '日').as_deref(), Some(baseline_han_name.as_str()), "Han in a non-CJK effective locale keeps the manifest's first covering face");
    let unknown = test_provider((|| None) as fn() -> Option<String>);
    assert_eq!(provided(&unknown, '日').as_deref(), Some(baseline_han_name.as_str()), "an unknown effective locale has the same baseline face");
    let english_on_japanese_os = test_provider((|| effective_locale("en", Some("ja_JP"))) as fn() -> Option<String>);
    assert_eq!(
        provided(&english_on_japanese_os, '日').as_deref(),
        Some(ui_name.as_str()),
        "English UI on a Japanese OS follows the OS locale; the manifest baseline is not universal for English UI"
    );
    assert_eq!(provided(&en, 'م').as_deref(), Some(arabic_name.as_str()), "Arabic uses its own craft face");
    assert_eq!(provided(&en, 'न').as_deref(), Some(devanagari_name.as_str()), "Devanagari uses its own craft face");

    let hans = test_provider((|| Some("zh-CN".into())) as fn() -> Option<String>);
    assert_eq!(provided(&hans, '日').as_deref(), Some(sc_name.as_str()));
    assert_eq!(provided(&hans, 'あ').as_deref(), Some(ui_name.as_str()), "kana in a Chinese UI keeps the Japanese face");

    let hant = test_provider((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>);
    assert_eq!(provided(&hant, '龍').as_deref(), Some(sc_name.as_str()), "with no system Hant face, the locale order falls through to SC");
    assert_eq!(provided(&hant, '\u{3105}').as_deref(), Some(sc_name.as_str()), "Bopomofo follows the same locale-order fallback");
    assert_eq!(provided(&hant, 'あ').as_deref(), Some(ui_name.as_str()), "kana in a Hant UI keeps the Japanese face");

    // A system Hant file, when present, is read before the Simplified Chinese craft fallback.
    if hant_system_file().is_some() {
        let sources = Sources {
            locale: (|| Some("zh-Hant-TW".into())) as fn() -> Option<String>,
            files: hant_files,
            last_resort: Vec::new,
            pending: || false,
            scripts: no_script_files,
            covers: scripts::covering_face,
            embedded: craft_embedded_iso,
            eager_craft_fonts: false,
        };
        let hant = CraftFontProvider::new(&sources, 1.0);
        let got = provided(&hant, '龍').expect("the system Hant face");
        assert!(got.starts_with("craft-system-"), "the system Hant face comes before the SC craft fallback: {got}");
        assert_ne!(got, sc_name, "the SC craft face must not pre-empt a system Hant face");
    }

    let ko = test_provider((|| Some("ko_KR".into())) as fn() -> Option<String>);
    assert_eq!(provided(&ko, '한').as_deref(), Some(korean_name.as_str()));
    assert_eq!(provided(&ko, 'あ').as_deref(), Some(ui_name.as_str()), "kana in a Korean UI keeps the Japanese face");
    assert_ne!(sc.family, ui_font.family);
}

static PENDING_LAST_RESORT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PENDING_CRAFT_SC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PENDING_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static STRICT_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn pending_last_resort() -> bool {
    PENDING_LAST_RESORT.load(std::sync::atomic::Ordering::Relaxed)
}

fn pending_craft_sc() -> bool {
    PENDING_CRAFT_SC.load(std::sync::atomic::Ordering::Relaxed)
}

fn pending_ready() -> bool {
    !PENDING_READY.load(std::sync::atomic::Ordering::Relaxed)
}

fn strict_pending() -> bool {
    STRICT_PENDING.load(std::sync::atomic::Ordering::Relaxed)
}

fn ready_files(script: CjkScript) -> Vec<FontFile> {
    if script == CjkScript::TraditionalChinese && PENDING_READY.load(std::sync::atomic::Ordering::Relaxed) { vec![fake("hant.ttf")] } else { Vec::new() }
}

fn ready_hant_files(script: CjkScript) -> Vec<FontFile> {
    if script == CjkScript::TraditionalChinese { vec![fake("hant.ttf")] } else { Vec::new() }
}

#[test]
fn provider_uses_the_requested_collection_family_and_its_exact_coverage() {
    let path = fake_dir().join("families.ttc");
    let provider = CraftFontProvider::new(&test_sources(|| Some("en".into())), 1.0);
    let file = |family: &str| vec![FontFile { path: path.clone(), family: family.into() }];

    let insert = provider.system_insert_files(file("JetBrains Mono"), 'A').expect("face 1 covers A");
    assert_eq!(insert.data.index, 1, "the requested family selects face 1, not the first covering face 0");
    assert!(
        provider.system_insert_files(file("JetBrains Mono"), 'ƀ').is_none(),
        "JetBrains Mono does not cover the character; face 0 of Inter must not be used"
    );

    let empty_family = vec![FontFile { path: path.clone(), family: String::new() }];
    let insert = provider.system_insert_files(empty_family, 'ƀ').expect("an empty family is face 0");
    assert_eq!(insert.data.index, 0);
}

#[test]
fn cached_system_face_returns_the_current_scale() {
    let path = fake_dir().join("families.ttc");
    let provider = CraftFontProvider::new(&test_sources(|| Some("en".into())), 1.0);
    let file = || vec![FontFile { path: path.clone(), family: "JetBrains Mono".into() }];

    let first = provider.system_insert_files(file(), 'A').expect("face 1 covers A");
    assert_eq!(first.data.index, 1);
    assert_eq!(first.data.tweak.scale, 1.0);
    provider.set_scale(4.0 / 3.0);
    let second = provider.system_insert_files(file(), 'A').expect("cached face");
    assert_eq!(second.data.index, 1);
    assert_eq!(second.data.tweak.scale, 4.0 / 3.0, "a cached face must not bake in the old UI scale");
}

#[test]
fn provider_honours_the_injected_embedded_faces() {
    let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied() else {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    };
    if craft_index(ui_font).is_none() {
        return;
    }
    let Some(baseline) = baseline_face('あ') else { return };
    let baseline_name = format!("craft-font-{}", craft_index(baseline).expect("a manifest face has an index"));
    let with_craft = test_provider((|| Some("en_US".into())) as fn() -> Option<String>);
    assert_eq!(provided(&with_craft, 'あ').as_deref(), Some(baseline_name.as_str()));

    let no_craft = Sources { embedded: no_embedded, ..test_sources((|| Some("en_US".into())) as fn() -> Option<String>) };
    let provider = CraftFontProvider::new(&no_craft, 1.0);
    assert!(provided(&provider, 'あ').is_none(), "the provider must use Sources::embedded, not the global craft table");
}

#[test]
fn provider_reaches_a_fontconfig_match_after_six_missing_candidates() {
    let mut files: Vec<FontFile> =
        (0..MAX_SCRIPT_READS).map(|i| FontFile { path: fake_dir().join(format!("missing-{i}.ttf")), family: String::new() }).collect();
    files.push(fake("hant.ttf"));
    let provider = CraftFontProvider::new(&test_sources((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>), 1.0);
    let insert = provider.system_insert_files(files, '龍').expect("the seventh (Fontconfig) candidate must be reachable");
    assert!(insert.name.starts_with("craft-system-"));
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(cache.reads, 1, "only the real file counts as a read; absent paths do not");
    assert_eq!(cache.failed_reads.len(), MAX_SCRIPT_READS);
}

#[test]
fn provider_caches_failed_reads_and_face_lookups_across_characters() {
    let provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
    let missing = fake("missing-cache.ttf");
    assert!(provider.system_insert_files(vec![missing.clone()], '日').is_none());
    assert!(provider.system_insert_files(vec![missing.clone()], '本').is_none());
    {
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.failed_reads.len(), 1, "a missing path is remembered");
        assert_eq!(cache.reads, 0, "a missing path performs no font read");
    }

    // Inter is readable but does not cover either Han character: its bytes are read once.
    let inter = fake("font.ttf");
    assert!(provider.system_insert_files(vec![inter.clone()], '日').is_none());
    assert!(provider.system_insert_files(vec![inter.clone()], '本').is_none());
    {
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.reads, 1, "the second character reuses the cached bytes");
        assert_eq!(cache.files.len(), 1);
    }

    // A collection's family lookup is parsed once even when it misses every character.
    let path = fake_dir().join("families.ttc");
    let file = vec![FontFile { path, family: "Noto Sans CJK TC".into() }];
    assert!(provider.system_insert_files(file.clone(), '日').is_none());
    assert!(provider.system_insert_files(file, '本').is_none());
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(cache.family_indices.len(), 1, "a family miss is cached");
    assert_eq!(cache.family_indices[0].2, None);
}

static TTC_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn counted_ttc_read(path: &std::path::Path) -> Option<Vec<u8>> {
    TTC_READS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::fs::read(path).ok()
}

/// A collection's two faces share one blob: selecting both faces reads the file once and
/// charges its bytes once, not once per face.
#[test]
fn provider_charges_a_ttc_blob_once_for_multiple_selected_faces() {
    let path = fake_dir().join("families.ttc");
    let size = std::fs::metadata(&path).expect("test collection").len() as usize;
    let mut provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
    provider.reader = counted_ttc_read;
    TTC_READS.store(0, std::sync::atomic::Ordering::Relaxed);
    let first = provider.system_insert_files(vec![FontFile { path: path.clone(), family: "Inter".into() }], 'A').expect("face 0 covers A");
    let second = provider.system_insert_files(vec![FontFile { path: path.clone(), family: "JetBrains Mono".into() }], 'A').expect("face 1 covers A");
    assert_eq!((first.data.index, second.data.index), (0, 1));
    assert!(std::sync::Arc::ptr_eq(&first.data.font, &second.data.font), "both faces share the collection's blob");
    assert_eq!(TTC_READS.load(std::sync::atomic::Ordering::Relaxed), 1, "the collection is read once");
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(cache.files.len(), 1, "one retained blob serves both faces");
    assert_eq!(cache.retained_bytes, size, "the TTC blob is charged once");
    assert_eq!(cache.faces.len(), 2, "both selected faces keep their own name");
}

/// Selecting a second face under a one-blob budget evicts the earlier blob from the
/// provider, but the earlier `FontInsert` remains valid because it owns its shared blob.
#[test]
fn provider_evicts_an_earlier_selected_blob_under_budget_pressure() {
    let first_path = fake("font.ttf").path;
    let bytes = std::fs::read(&first_path).expect("test font");
    let size = bytes.len();
    let second_path = fake_dir().join("budget-selected-second.ttf");
    std::fs::write(&second_path, &bytes).expect("write a second test font");
    let provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
    {
        let mut cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.max_bytes = size;
    }
    let first = provider.system_insert_files(vec![FontFile { path: first_path.clone(), family: String::new() }], 'A').expect("the first selected face");
    let second = provider.system_insert_files(vec![FontFile { path: second_path.clone(), family: String::new() }], 'A').expect("the second selected face");
    assert_ne!(first.name, second.name);
    assert_eq!(first.data.bytes(), bytes.as_slice(), "evicting the provider's blob does not invalidate egui's copy");
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(cache.files.len(), 1, "only one blob fits");
    assert_eq!(cache.files[0].path, second_path, "the newest selected blob is retained");
    assert_eq!(cache.retained_bytes, size, "the provider cache stays within its one-blob budget");
    assert_eq!(cache.faces.len(), 2, "the earlier face name remains reusable metadata, not bytes");
}

/// A selected face whose blob does not fit the budget is still returned complete for this
/// request. egui may keep it, but the provider retains neither its bytes nor a second copy.
#[test]
fn provider_returns_a_transient_valid_face_when_the_blob_cannot_be_retained() {
    let path = fake("font.ttf").path;
    let size = std::fs::metadata(&path).expect("test font").len() as usize;
    let provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
    {
        let mut cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.max_bytes = size.saturating_sub(1);
    }
    let insert =
        provider.system_insert_files(vec![FontFile { path: path.clone(), family: String::new() }], 'A').expect("the over-budget face is returned transiently");
    assert_eq!(insert.data.index, 0);
    assert_eq!(insert.data.bytes().len(), size, "the transient face carries the whole valid font");
    assert_eq!(insert.data.bytes(), std::fs::read(&path).expect("test font bytes").as_slice());
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(cache.retained_bytes, 0, "an over-budget face retains no provider bytes");
    assert!(cache.files.is_empty(), "the over-budget blob is not retained");
    assert!(cache.retained_bytes <= cache.max_bytes, "the provider cache stays within its budget");
    assert_eq!(cache.reads, 1);
}

/// A valid font file whose character map does not include the requested character is still a
/// valid font: its bytes are cached within the budget and it is not marked invalid.
#[test]
fn provider_cache_evicts_retained_bytes_within_the_budget() {
    let provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
    let bytes = synthetic_font(&[(0x0E01, 0x0E5B)]);
    let size = bytes.len();
    assert!(size > 0);
    {
        let mut cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.max_bytes = size.saturating_mul(2);
    }
    let mut paths = Vec::new();
    for i in 0..5 {
        let path = fake_dir().join(format!("budget-{i}.ttf"));
        std::fs::write(&path, &bytes).expect("write a synthetic test font");
        paths.push(path);
    }
    for path in &paths {
        let file = vec![FontFile { path: path.clone(), family: String::new() }];
        assert!(provider.system_insert_files(file, '日').is_none(), "the Thai-only face does not cover 日");
    }
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(cache.retained_bytes <= cache.max_bytes, "retained {} > budget {}", cache.retained_bytes, cache.max_bytes);
    assert!(cache.files.len() <= 2, "only two files fit the budget");
    assert!(cache.files.iter().any(|file| file.path == paths[4]), "the newest retained file stays");
    assert!(!cache.files.iter().any(|file| file.path == paths[0]), "the oldest retained file was evicted");
    assert_eq!(cache.reads, 5, "each distinct file was read once");
    assert!(cache.invalid_fonts.is_empty(), "a non-covering face is not an invalid font");
}

/// A later covering candidate (such as the Fontconfig match appended last) is reached even
/// when a tiny budget evicts every retained candidate before it.
#[test]
fn provider_reaches_a_later_covering_candidate_under_budget_pressure() {
    let provider = CraftFontProvider::new(&test_sources((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>), 1.0);
    let bytes = synthetic_font(&[(0x0E01, 0x0E5B)]);
    {
        let mut cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.max_bytes = bytes.len();
    }
    let mut files = Vec::new();
    for i in 0..12 {
        let path = fake_dir().join(format!("budget-later-{i}.ttf"));
        std::fs::write(&path, &bytes).expect("write a synthetic test font");
        files.push(FontFile { path, family: String::new() });
    }
    let covering = fake("hant.ttf").path;
    files.push(FontFile { path: covering.clone(), family: String::new() });
    let insert = provider.system_insert_files(files, '龍').expect("the later covering candidate must be reached");
    assert!(insert.name.starts_with("craft-system-"), "{insert:?}");
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(cache.faces.iter().any(|face| face.path == covering), "the covering file became a selected face");
    assert!(cache.reads >= 13, "all retained candidates plus the matching file were read");
}

/// The old lifetime read cap could exhaust before a Fontconfig match appended after many
/// earlier characters. Reads are no longer globally capped; only retained bytes are bounded.
#[test]
fn provider_has_no_lifetime_read_cap_for_the_final_fontconfig_candidate() {
    let provider = CraftFontProvider::new(&test_sources((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>), 1.0);
    let bytes = synthetic_font(&[(0x0E01, 0x0E5B)]);
    {
        let mut cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.max_bytes = bytes.len();
    }
    for i in 0..130 {
        let path = fake_dir().join(format!("lifetime-{i}.ttf"));
        std::fs::write(&path, &bytes).expect("write a synthetic test font");
        let file = vec![FontFile { path, family: String::new() }];
        assert!(provider.system_insert_files(file, '日').is_none());
    }
    let covering = fake("hant.ttf").path;
    let insert = provider
        .system_insert_files(vec![FontFile { path: covering, family: String::new() }], '龍')
        .expect("the final candidate must still be read after 130 successful reads");
    assert!(insert.name.starts_with("craft-system-"), "{insert:?}");
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(cache.reads, 131, "130 non-covering reads plus the final candidate");
}

static CONCURRENT_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn counted_slow_read(path: &std::path::Path) -> Option<Vec<u8>> {
    CONCURRENT_READS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::read(path).ok()
}

/// Layout may ask the provider from several threads. The check/read/insert is serialised, so
/// the same path is read once and every caller sees the same success.
#[test]
fn provider_serialises_concurrent_reads_of_the_same_path() {
    let path = fake("font.ttf").path;
    let mut provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
    provider.reader = counted_slow_read;
    CONCURRENT_READS.store(0, std::sync::atomic::Ordering::Relaxed);
    let provider = std::sync::Arc::new(provider);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let provider = std::sync::Arc::clone(&provider);
        let path = path.clone();
        handles.push(std::thread::spawn(move || {
            let file = vec![FontFile { path, family: String::new() }];
            provider.system_insert_files(file, 'A').is_some()
        }));
    }
    for handle in handles {
        assert!(handle.join().expect("layout thread"), "every caller must get the Inter face");
    }
    assert_eq!(CONCURRENT_READS.load(std::sync::atomic::Ordering::Relaxed), 1, "one read per path even under concurrency");
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(cache.faces.len(), 1, "one selected face is shared");
}

/// A readable non-font is negative-cached as invalid (read/parsed once) and never conflated
/// with a valid font that simply does not cover the requested character.
#[test]
fn provider_negative_caches_invalid_fonts_separately_from_noncovering_faces() {
    let provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
    let invalid = fake("not-a-font-cache.ttf").path;
    std::fs::write(&invalid, b"this is not a font").expect("write invalid bytes");
    let truncated = fake("truncated-font-cache.ttf").path;
    std::fs::write(&truncated, &include_bytes!("../../../../../assets/fonts/Inter-Regular.ttf")[..20]).expect("write truncated font");
    for path in [&invalid, &truncated] {
        for c in ['日', '本'] {
            let file = vec![FontFile { path: path.clone(), family: String::new() }];
            assert!(provider.system_insert_files(file, c).is_none());
        }
    }
    {
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.invalid_fonts.len(), 2, "both invalid paths are remembered");
        assert_eq!(cache.reads, 2, "invalid bytes are read and examined once each");
        assert!(cache.files.is_empty(), "invalid bytes are never retained as a font");
        assert!(cache.failed_reads.is_empty(), "an invalid font is not an I/O failure");
    }
    let inter = fake("font.ttf").path;
    let file = vec![FontFile { path: inter.clone(), family: String::new() }];
    assert!(provider.system_insert_files(file, '日').is_none(), "Inter does not cover 日");
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(cache.files.iter().any(|file| file.path == inter), "a valid non-covering font is cached");
    assert!(!cache.invalid_fonts.contains(&inter), "a non-covering face is not an invalid font");
    drop(cache);
    let file = vec![FontFile { path: inter, family: String::new() }];
    let insert = provider.system_insert_files(file, 'A').expect("the cached valid font still covers A");
    assert!(insert.name.starts_with("craft-system-"), "{insert:?}");
}

#[test]
fn pending_scan_recovers_through_real_layout_and_the_lazy_reload() {
    PENDING_READY.store(false, std::sync::atomic::Ordering::Relaxed);
    let ctx = egui::Context::default();
    let sources = Sources {
        locale: (|| Some("zh-Hant-TW".into())) as fn() -> Option<String>,
        files: ready_files,
        last_resort: Vec::new,
        pending: pending_ready,
        scripts: no_script_files,
        covers: scripts::covering_face,
        embedded: no_embedded,
        eager_craft_fonts: true,
    };
    crate::theme::install_fonts_with(&ctx, sources);
    let frame = |ctx: &egui::Context| {
        ctx.run_ui(Default::default(), |ui| {
            ui.label("龍");
        })
        .textures_delta
        .clear();
    };
    frame(&ctx);
    assert!(!ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(12.0), '龍')), "the pending first frame has no Hant face");

    PENDING_READY.store(true, std::sync::atomic::Ordering::Relaxed);
    for _ in 0..4 {
        frame(&ctx);
    }
    assert!(ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(12.0), '龍')), "the lazy loader's add_font reload recovers after the scan");
    let registered = ctx.fonts(|f| f.definitions().font_data.keys().filter(|name| name.starts_with(FONT_PREFIX)).count());
    assert_eq!(registered, 1, "exactly the Hant file was installed");
    PENDING_READY.store(false, std::sync::atomic::Ordering::Relaxed);
}

#[test]
fn strict_policy_first_frame_after_the_scan_is_accepted() {
    if craft_fonts::CRAFT_FONTS.is_empty() {
        eprintln!("skipping: the eager provider is only installed with craft-fonts");
        return;
    }
    let ctx = egui::Context::default();
    ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
    let sources = Sources {
        locale: (|| Some("zh-Hant-TW".into())) as fn() -> Option<String>,
        files: ready_hant_files,
        last_resort: Vec::new,
        pending: || false,
        scripts: no_script_files,
        covers: scripts::covering_face,
        embedded: no_embedded,
        eager_craft_fonts: true,
    };
    crate::theme::install_fonts_with(&ctx, sources);
    ctx.run_ui(Default::default(), |ui| {
        ui.label("龍");
    })
    .textures_delta
    .clear();
    assert!(ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(12.0), '龍')));
}

/// A strict first frame while Fontconfig is still pending cannot be recovered by the lazy
/// loader: epaint panics during layout before `output_hook` can call `add_font`. The real app
/// uses `MissingGlyphPolicy::Tofu` (brief tofu, then the reload above); strict integration
/// tests such as `camera_raw_localization` must wait for the scan first. This test records
/// that limitation instead of claiming a pending strict startup is accepted.
#[test]
#[should_panic(expected = "No glyph for")]
fn pending_strict_policy_startup_is_not_fake_acceptance() {
    let ctx = egui::Context::default();
    ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
    let sources = Sources {
        locale: (|| Some("zh-Hant-TW".into())) as fn() -> Option<String>,
        files: no_files,
        last_resort: Vec::new,
        pending: strict_pending,
        scripts: no_script_files,
        covers: scripts::covering_face,
        embedded: no_embedded,
        eager_craft_fonts: true,
    };
    crate::theme::install_fonts_with(&ctx, sources);
    let _ = ctx.run_ui(Default::default(), |ui| {
        ui.label("龍");
    });
}

#[test]
fn language_change_reloads_eager_faces_through_the_app_context() {
    let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied() else {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    };
    let Some(ui_index) = craft_index(ui_font) else { return };
    let Some((sc_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans CJK SC") else {
        eprintln!("skipping: the craft-fonts manifest has no Simplified Chinese face");
        return;
    };
    let ui_name = format!("craft-font-{ui_index}");
    let sc_name = format!("craft-font-{sc_index}");
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let ctx = egui::Context::default();
        crate::i18n::sync_context(&ctx, "ja");
        for _ in 0..2 {
            ctx.run_ui(Default::default(), |ui| {
                ui.label("日");
            })
            .textures_delta
            .clear();
        }
        let discovered = ctx.fonts(|f| f.discovered_fonts().to_vec());
        assert!(discovered.iter().any(|insert| insert.name == ui_name), "ja discovers {ui_name}: {discovered:?}");

        crate::i18n::sync_context(&ctx, "zh-hans");
        for _ in 0..2 {
            ctx.run_ui(Default::default(), |ui| {
                ui.label("日");
            })
            .textures_delta
            .clear();
        }
        let discovered = ctx.fonts(|f| f.discovered_fonts().to_vec());
        assert!(discovered.iter().any(|insert| insert.name == sc_name), "zh-hans discovers {sc_name}: {discovered:?}");
        assert!(!discovered.iter().any(|insert| insert.name == ui_name), "the language reload clears the old Japanese face: {discovered:?}");
    });
}

/// A valid synthetic font whose `glyf` table record extends past the end of the file. Its
/// `cmap` still parses and maps Thai, so a coverage-only check accepts it, but the text
/// engine cannot read every declared table.
fn font_with_out_of_bounds_table() -> Vec<u8> {
    let mut font = synthetic_font(&[(0x0E01, 0x0E5B)]);
    let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
    for i in 0..tables {
        let rec = 12 + 16 * i;
        if font.get(rec..rec + 4) == Some(&b"glyf"[..]) {
            font[rec + 12..rec + 16].copy_from_slice(&u32::MAX.to_be_bytes());
            return font;
        }
    }
    panic!("synthetic font has no glyf table");
}

/// A readable collection whose selected family face has an out-of-bounds `glyf` table. Face 0
/// stays readable, and face 1's `cmap`/`name` still work, so a file-level check plus coverage
/// accepts it even though the face cannot be installed safely.
fn collection_with_out_of_bounds_selected_face() -> Vec<u8> {
    let inter = include_bytes!("../../../../../assets/fonts/Inter-Regular.ttf");
    let mut jetbrains = include_bytes!("../../../../../assets/fonts/JetBrainsMono-Regular.ttf").to_vec();
    let tables = u16::from_be_bytes([jetbrains[4], jetbrains[5]]) as usize;
    let glyf = (0..tables).map(|i| 12 + 16 * i).find(|&rec| jetbrains.get(rec..rec + 4) == Some(&b"glyf"[..])).expect("JetBrains Mono has a glyf table");
    jetbrains[glyf + 12..glyf + 16].copy_from_slice(&u32::MAX.to_be_bytes());
    two_face_ttc(inter, &jetbrains)
}

/// A truncated/out-of-bounds table can keep a valid `cmap`, so `load_first` must validate
/// the file before registering it, and must still reach the next, valid candidate.
#[test]
fn lazy_loader_rejects_a_font_with_an_out_of_bounds_table_and_uses_the_next_candidate() {
    let malformed = fake_dir().join("out-of-bounds-thai.ttf");
    std::fs::write(&malformed, font_with_out_of_bounds_table()).unwrap();
    let valid = fake_dir().join("thai.ttf");
    let files = vec![FontFile { path: malformed.clone(), family: String::new() }, FontFile { path: valid.clone(), family: String::new() }];
    let bytes = std::fs::read(&malformed).unwrap();
    assert!(scripts::face_covers(&bytes, 0, 'ก'), "the malformed file's cmap still maps the character");
    assert!(!cjk::font_file_is_readable(&bytes), "an out-of-bounds table makes the file unreadable");

    let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
    let (_, data) = fallback.load_first(&files, 'ก').expect("the valid fallback is reached");
    assert_eq!(fallback.registered.len(), 1);
    assert_eq!(fallback.registered[0].1, valid, "the malformed file is skipped, not registered");
    assert_eq!(data.index, 0);
    assert_eq!(data.bytes(), std::fs::read(&valid).unwrap().as_slice());

    // The non-CJK script route must enforce the same validation before registration.
    let mut sources = test_sources(|| Some("en".into()));
    sources.scripts = |_| vec![fake_dir().join("out-of-bounds-thai.ttf"), fake_dir().join("thai.ttf")];
    let mut fallback = CjkFallback::new(sources);
    let (_, data) = fallback.next_script_font('ก').expect("the valid script fallback is reached");
    assert_eq!(fallback.registered.len(), 1);
    assert_eq!(fallback.registered[0].1, valid, "the script route also skips the malformed file");
    assert_eq!(data.bytes(), std::fs::read(&valid).unwrap().as_slice());

    // A readable collection with an unreadable covering face must also skip that face: the
    // coverage route selects face 1 by its valid cmap, not the readable face 0.
    let collection = fake_dir().join("out-of-bounds-thai.ttc");
    let inter = include_bytes!("../../../../../assets/fonts/Inter-Regular.ttf");
    let collection_bytes = two_face_ttc(inter, &font_with_out_of_bounds_table());
    std::fs::write(&collection, &collection_bytes).unwrap();
    assert!(cjk::font_file_is_readable(&collection_bytes), "face 0 keeps the collection readable");
    assert_eq!(scripts::covering_face(&collection_bytes, 'ก'), Some(1), "coverage selects the malformed face");
    assert!(!cjk::font_face_is_readable(&collection_bytes, 1));
    let mut sources = test_sources(|| Some("en".into()));
    sources.scripts = |_| vec![fake_dir().join("out-of-bounds-thai.ttc"), fake_dir().join("thai.ttf")];
    let mut fallback = CjkFallback::new(sources);
    let (_, data) = fallback.next_script_font('ก').expect("the valid script fallback is reached");
    assert_eq!(fallback.registered.len(), 1);
    assert_eq!(fallback.registered[0].1, valid, "the coverage route skips the malformed selected face");
    assert_eq!(data.bytes(), std::fs::read(&valid).unwrap().as_slice());
}

/// A readable collection may contain one bad face. The selected face (by family name) can
/// keep a valid `cmap` and `name` while another table points outside the file, so checking the
/// file or coverage alone would register a face epaint cannot safely install. Both the lazy
/// loader and the eager provider must skip that face and continue to the valid candidate.
#[test]
fn malformed_selected_collection_face_falls_back_before_lazy_and_provider_registration() {
    let path = fake_dir().join("out-of-bounds-selected.ttc");
    let bytes = collection_with_out_of_bounds_selected_face();
    std::fs::write(&path, &bytes).expect("write the synthetic collection");
    assert!(cjk::font_file_is_readable(&bytes), "face 0 keeps the collection readable");
    assert_eq!(cjk::face_index_for_family_checked(&bytes, "JetBrains Mono"), Some(1), "the selected face's name resolves");
    assert!(scripts::face_covers(&bytes, 1, 'A'), "the selected face's cmap covers the character");
    assert!(!cjk::font_face_is_readable(&bytes, 1), "the selected face's glyf table is out of bounds");

    let valid = fake("font.ttf");
    let candidates = vec![FontFile { path: path.clone(), family: "JetBrains Mono".into() }, FontFile { path: valid.path.clone(), family: String::new() }];
    let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
    let (name, data) = fallback.load_first(&candidates, 'A').expect("the valid alternative is reached");
    assert_eq!(data.index, 0, "the lazy loader registers the valid face 0, not the malformed selected face");
    assert_eq!(fallback.registered.len(), 1);
    assert_eq!(fallback.registered[0].1, valid.path, "the malformed collection is not registered");

    let provider = CraftFontProvider::new(&test_sources(|| Some("en".into())), 1.0);
    let insert = provider.system_insert_files(candidates, 'A').expect("the provider reaches the valid alternative");
    assert_eq!(insert.data.index, 0, "the provider does not hand the malformed selected face to epaint");
    assert!(insert.name.starts_with("craft-system-"), "{insert:?}");

    // Installing the selected font under the strict policy must not panic: only the valid
    // fallback reaches egui's family builder.
    let ctx = egui::Context::default();
    ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
    crate::theme::install_fonts(&ctx);
    // The lazy loader runs after a frame; initialize the font view before matching that path.
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    add_to_all_families(&ctx, name, data);
    ctx.run_ui(Default::default(), |ui| {
        ui.label("A");
    })
    .textures_delta
    .clear();
}

#[test]
fn lazy_loader_uses_the_requested_collection_family_and_checks_coverage() {
    let path = fake_dir().join("families.ttc");
    let file = |family: &str| vec![FontFile { path: path.clone(), family: family.into() }];

    let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
    let (_, data) = fallback.load_first(&file("JetBrains Mono"), 'A').expect("face 1 covers A");
    assert_eq!(data.index, 1);

    let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
    assert!(fallback.load_first(&file("JetBrains Mono"), 'ƀ').is_none(), "coverage is checked on the requested face");

    let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
    let empty_family = vec![FontFile { path: path.clone(), family: String::new() }];
    let (_, data) = fallback.load_first(&empty_family, 'ƀ').expect("empty family is face 0");
    assert_eq!(data.index, 0);
}

fn always_thai_candidate(_: Script) -> Vec<PathBuf> {
    vec![fake_dir().join("thai.ttf")]
}

/// A path already registered through either route is not read or registered a second time.
#[test]
fn lazy_loader_registers_each_loaded_font_once() {
    let valid = fake("font.ttf");
    let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
    let files = vec![valid.clone()];
    fallback.load_first(&files, 'A').expect("the font covers Latin A");
    assert!(fallback.load_first(&files, 'A').is_none(), "a second request for the loaded file registers nothing");
    assert_eq!(fallback.registered.len(), 1);

    let sources = Sources { scripts: always_thai_candidate, ..test_sources(|| Some("en".into())) };
    let mut fallback = CjkFallback::new(sources);
    fallback.loaded.push(fake_dir().join("thai.ttf"));
    assert!(fallback.next_script_font('ก').is_none(), "the script route skips a file the other route already loaded");
    assert!(fallback.registered.is_empty());
    assert_eq!(fallback.scripts_tried, vec![Script::Thai], "the script is still marked tried only once");
}

/// A craft face is registered once: a later request for the same embedded file must not add
/// another font with a new name and a duplicate blob.
#[test]
fn lazy_loader_registers_an_embedded_craft_face_once() {
    if baseline_face('あ').is_none() {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    }
    let mut fallback = CjkFallback::new(craft_only(|| Some("en".into())));
    fallback.load_embedded(CjkScript::Japanese, 'あ').expect("the manifest covers kana");
    assert!(fallback.load_embedded(CjkScript::Japanese, 'あ').is_none(), "the same embedded face is not registered twice");
    assert_eq!(fallback.registered.len(), 1);
    assert_eq!(fallback.loaded.len(), 1);
}

#[test]
fn provider_uses_system_script_candidates_and_last_resort_for_non_cjk() {
    let sources = Sources {
        locale: || Some("en".into()),
        files: no_files,
        last_resort: || vec![FontFile { path: fake_dir().join("ethiopic.ttf"), family: String::new() }],
        pending: || false,
        scripts: |s| match s {
            Script::Tamil => vec![fake_dir().join("tamil.ttf")],
            _ => Vec::new(),
        },
        covers: scripts::covering_face,
        embedded: no_embedded,
        eager_craft_fonts: false,
    };
    let provider = CraftFontProvider::new(&sources, 1.0);
    let tamil = provided(&provider, 'த').expect("Tamil system candidate");
    assert!(tamil.starts_with("craft-system-"), "{tamil}");
    let ethiopic = provided(&provider, 'ሀ').expect("last-resort face");
    assert!(ethiopic.starts_with("craft-system-"), "{ethiopic}");
}

static PROVIDER_SCRIPT_CALLS: Mutex<Vec<CjkScript>> = Mutex::new(Vec::new());

fn recorded_provider_files(script: CjkScript) -> Vec<FontFile> {
    PROVIDER_SCRIPT_CALLS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(script);
    Vec::new()
}

/// A Hangul character's preferred script is the locale order's first script, so the provider
/// builds a list with Korean twice. It must still ask for Korean's files once, then try each
/// remaining script once, not restart the preferred script after a miss.
#[test]
fn provider_tries_each_script_once_when_the_preferred_script_repeats_the_order() {
    PROVIDER_SCRIPT_CALLS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
    let sources = Sources {
        locale: || Some("ko_KR".into()),
        files: recorded_provider_files,
        last_resort: Vec::new,
        pending: || false,
        scripts: no_script_files,
        covers: scripts::covering_face,
        embedded: no_embedded,
        eager_craft_fonts: false,
    };
    let provider = CraftFontProvider::new(&sources, 1.0);
    assert!(provided(&provider, '한').is_none(), "the test source has no Hangul face");
    let calls = PROVIDER_SCRIPT_CALLS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    assert_eq!(calls, vec![CjkScript::Korean, CjkScript::TraditionalChinese, CjkScript::SimplifiedChinese, CjkScript::Japanese]);
}

/// An unclassified symbol (not CJK, not one of the non-CJK scripts) only has the optional
/// craft `Zsym` face. Without one the provider must return missing rather than read unrelated
/// system fonts, and an empty cluster is likewise no request at all.
#[test]
fn provider_missing_symbol_and_empty_cluster_do_not_touch_system_fonts() {
    let sources = Sources { embedded: no_embedded, ..test_sources(|| Some("en".into())) };
    let provider = CraftFontProvider::new(&sources, 1.0);
    assert!(provider.font_for(&FallbackRequest { cluster: "", family: &FontFamily::Proportional }).is_none());
    assert!(provided(&provider, '☃').is_none(), "no injected Zsym craft face covers the symbol");
    let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(cache.reads, 0, "a symbol is not a CJK or script fallback request");
    assert!(cache.failed_reads.is_empty() && cache.invalid_fonts.is_empty());
}

#[test]
fn provider_applies_the_current_scale_to_craft_faces() {
    if baseline_face('あ').is_none() {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    }
    let provider = test_provider(|| Some("en".into()));
    let first = provided_insert(&provider, 'あ').expect("the craft manifest covers kana");
    assert_eq!(first.data.tweak.scale, 1.0);
    assert!(first.families.iter().any(|f| f.family == FontFamily::Proportional));
    assert!(first.families.iter().any(|f| f.family == FontFamily::Monospace));
    provider.set_scale(4.0 / 3.0);
    let second = provided_insert(&provider, 'あ').expect("the craft face is resolved again at the current scale");
    assert_eq!(second.data.tweak.scale, 4.0 / 3.0, "a craft face must not bake in the scale of an earlier pass");
}

/// Face-name metadata is capped independently of the retained-blob budget: forgetting the
/// oldest name only makes egui install that face under a later name, it never drops bytes
/// from a file that still fits.
#[test]
fn system_face_names_are_bounded_independently_of_font_blobs() {
    let mut cache = SystemCache::default();
    for i in 0..MAX_SYSTEM_FACE_NAMES + 4 {
        cache.remember_face(PathBuf::from(format!("face-{i}.ttf")), i as u32, format!("craft-system-{i}"));
    }
    assert_eq!(cache.faces.len(), MAX_SYSTEM_FACE_NAMES);
    assert_eq!(cache.faces.first().map(|face| face.name.as_str()), Some("craft-system-4"), "the oldest names are evicted first");
    assert_eq!(cache.faces.last().map(|face| face.name.as_str()), Some("craft-system-1027"));
    assert!(cache.files.is_empty() && cache.retained_bytes == 0, "face names do not retain or charge font bytes");
}

#[test]
fn pending_fontconfig_defers_the_last_resort_instead_of_returning_none_forever() {
    PENDING_LAST_RESORT.store(true, std::sync::atomic::Ordering::Relaxed);
    let sources = Sources {
        locale: || Some("zh-Hant-TW".into()),
        files: no_files,
        last_resort: || vec![FontFile { path: fake_dir().join("hant.ttf"), family: String::new() }],
        pending: pending_last_resort,
        scripts: no_script_files,
        covers: scripts::covering_face,
        embedded: no_embedded,
        eager_craft_fonts: false,
    };
    let provider = CraftFontProvider::new(&sources, 1.0);
    assert!(provided(&provider, '龍').is_none(), "pending Hant scan must not fall through to a last resort");
    PENDING_LAST_RESORT.store(false, std::sync::atomic::Ordering::Relaxed);
    let got = provided(&provider, '龍').expect("after the scan the last resort is usable");
    assert!(got.starts_with("craft-system-"), "{got}");
}

#[test]
fn pending_fontconfig_does_not_cache_the_lower_priority_sc_craft_face_for_hant() {
    let Some((sc_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans CJK SC") else {
        eprintln!("skipping: the craft-fonts manifest has no Simplified Chinese face");
        return;
    };
    PENDING_CRAFT_SC.store(true, std::sync::atomic::Ordering::Relaxed);
    let sources = Sources {
        locale: || Some("zh-Hant-TW".into()),
        files: no_files,
        last_resort: Vec::new,
        pending: pending_craft_sc,
        scripts: no_script_files,
        covers: scripts::covering_face,
        embedded: craft_embedded_iso,
        eager_craft_fonts: false,
    };
    let provider = CraftFontProvider::new(&sources, 1.0);
    assert!(provided(&provider, '龍').is_none(), "a pending Hant scan must not return (and cache) the lower-priority SC craft face");
    PENDING_CRAFT_SC.store(false, std::sync::atomic::Ordering::Relaxed);
    let expected = format!("craft-font-{sc_index}");
    assert_eq!(provided(&provider, '龍').as_deref(), Some(expected.as_str()));
}

#[test]
fn every_craft_face_is_available_to_both_ui_families() {
    let mut checked = 0;
    for (i, face) in craft_fonts::CRAFT_FONTS.iter().enumerate() {
        if face.bytes.is_empty() {
            continue;
        }
        let insert = craft_insert(i, face, 1.0);
        assert_eq!(insert.name, format!("craft-font-{i}"));
        assert!(insert.families.iter().any(|f| f.family == FontFamily::Proportional), "{} is missing from Proportional", face.family);
        assert!(insert.families.iter().any(|f| f.family == FontFamily::Monospace), "{} is missing from Monospace", face.family);
        checked += 1;
    }
    if checked == 0 {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
    }
}

#[test]
fn the_craft_provider_shapes_the_first_headless_frame_in_both_families() {
    let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied() else {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    };
    let Some(ui_index) = craft_index(ui_font) else { return };
    let Some((sc_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans CJK SC") else {
        eprintln!("skipping: the craft-fonts manifest has no Simplified Chinese face");
        return;
    };
    let Some((korean_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "NanumGothic") else {
        eprintln!("skipping: the craft-fonts manifest has no Korean face");
        return;
    };
    let Some((arabic_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans Arabic") else {
        eprintln!("skipping: the craft-fonts manifest has no Arabic face");
        return;
    };
    let Some((devanagari_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans Devanagari") else {
        eprintln!("skipping: the craft-fonts manifest has no Devanagari face");
        return;
    };
    let Some(baseline_kana) = baseline_face('か') else { return };
    let Some(baseline_han) = baseline_face('日') else { return };
    let baseline_kana_index = craft_index(baseline_kana).expect("a manifest face has an index");
    let baseline_han_index = craft_index(baseline_han).expect("a manifest face has an index");
    for (locale, text, expected) in [
        ((|| Some("ja_JP".into())) as fn() -> Option<String>, "かな", ui_index),
        ((|| Some("en_US".into())) as fn() -> Option<String>, "かな", baseline_kana_index),
        ((|| Some("en_US".into())) as fn() -> Option<String>, "日本", baseline_han_index),
        ((|| None) as fn() -> Option<String>, "日本", baseline_han_index),
        ((|| Some("en_US".into())) as fn() -> Option<String>, "한", korean_index),
        ((|| Some("en_US".into())) as fn() -> Option<String>, "مرحبا", arabic_index),
        ((|| Some("en_US".into())) as fn() -> Option<String>, "नमस्ते", devanagari_index),
        ((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>, "かな", ui_index),
        ((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>, "龍", sc_index),
        ((|| Some("zh-CN".into())) as fn() -> Option<String>, "中文", sc_index),
    ] {
        let ctx = egui::Context::default();
        crate::theme::install_fonts_with(&ctx, eager_sources(locale));
        ctx.run_ui(Default::default(), |ui| {
            ui.label(text);
            ui.label(egui::RichText::new(text).monospace());
        })
        .textures_delta
        .clear();
        assert!(ctx.fonts_mut(|f| f.has_glyphs(&FontId::monospace(12.0), text)), "{text} is missing from Monospace");
        let discovered = ctx.fonts(|f| f.discovered_fonts().to_vec());
        assert!(
            discovered.iter().any(|insert| insert.name == format!("craft-font-{expected}")),
            "{text}: expected craft-font-{expected}, discovered {discovered:?}"
        );
        for insert in &discovered {
            assert!(insert.families.iter().any(|f| f.family == FontFamily::Proportional), "{insert:?}");
            assert!(insert.families.iter().any(|f| f.family == FontFamily::Monospace), "{insert:?}");
        }
    }
    // A Traditional Chinese locale has no Hant craft face. A system Hant file, when present,
    // must be read before the Simplified Chinese craft fallback.
    if hant_system_file().is_some() {
        let mut sources = eager_sources((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>);
        sources.files = hant_files;
        let ctx = egui::Context::default();
        crate::theme::install_fonts_with(&ctx, sources);
        ctx.run_ui(Default::default(), |ui| {
            ui.label("龍");
        })
        .textures_delta
        .clear();
        let discovered = ctx.fonts(|f| f.discovered_fonts().to_vec());
        assert!(discovered.iter().any(|insert| insert.name.starts_with("craft-system-")), "the system Hant face draws the first frame: {discovered:?}");
        assert!(
            !discovered.iter().any(|insert| insert.name == format!("craft-font-{sc_index}")),
            "the Simplified Chinese face must not pre-empt the Hant system font: {discovered:?}"
        );
    }
}

#[test]
fn ui_works_without_craft_fonts() {
    // The pre-craft-fonts behaviour: no embedded fonts, no system fonts. Latin renders,
    // Japanese is tried (and stays missing) without panicking, and the loader exhausts.
    let ctx = egui::Context::default();
    crate::theme::install_fonts_with(
        &ctx,
        Sources {
            locale: || None,
            files: |_| vec![],
            last_resort: Vec::new,
            pending: || false,
            scripts: |_| Vec::new(),
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: false,
        },
    );
    assert!(render(&ctx, "Layer 1 – café"));
    assert!(!render(&ctx, "日本語の文字"));
    let n = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
    assert_eq!(n, 0);
    if craft_fonts::CRAFT_FONTS.is_empty() {
        assert!(craft_embedded(CjkScript::Japanese).is_empty());
    }
}

/// With the real system fonts, every CJK sample renders (skipped per script when the
/// machine has no font for it, e.g. CI Linux without Noto CJK).
#[test]
fn system_fonts_cover_cjk_samples() {
    #[cfg(target_os = "linux")]
    assert!(wait_for_fontconfig_cjk_scan(), "Fontconfig CJK lookup did not finish within 10 seconds");
    let available = |s: CjkScript| cjk::font_files(s).iter().any(|f| f.path.is_file());
    let samples =
        [("图层", CjkScript::SimplifiedChinese), ("圖層", CjkScript::TraditionalChinese), ("카드 배경", CjkScript::Korean), ("レイヤー", CjkScript::Japanese)];
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    for (text, script) in samples {
        if !available(script) {
            eprintln!("skipping {text}: no {script:?} system font");
            continue;
        }
        assert!(render(&ctx, text), "{text} still has missing glyphs");
    }
    // All at once in a fresh context too (several scripts in one frame).
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let all: String = samples.iter().filter(|(_, s)| available(*s)).map(|(t, _)| *t).collect::<Vec<_>>().join(" ");
    assert!(render(&ctx, &all), "{all}");
}
