//! Reproduce numeric subfamilies without redistributing proprietary fonts.
use super::*;
use photocraft_doc::text::CharStyle;

fn numeric_font(source: &[u8], style: &str, ps: &str) -> Vec<u8> {
    let mut bytes = source.to_vec();
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let mut name_record = 0;
    for i in 0..count {
        let r = 12 + i * 16;
        let offset = u32::from_be_bytes(bytes[r + 8..r + 12].try_into().unwrap()) as usize;
        match &bytes[r..r + 4] {
            b"name" => name_record = r,
            b"OS/2" => bytes[offset + 4..offset + 6].copy_from_slice(&400u16.to_be_bytes()),
            _ => {}
        }
    }
    let legacy = format!("Numeric Test {style}");
    let names = [(1u16, legacy.as_str()), (2, "Regular"), (6, ps), (16, "Numeric Test"), (17, style)];
    let mut table = Vec::new();
    let mut strings = Vec::new();
    for value in [0u16, names.len() as u16, (6 + names.len() * 12) as u16] {
        table.extend(value.to_be_bytes());
    }
    for (id, value) in names {
        let encoded: Vec<_> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for n in [3, 1, 0x409, id, encoded.len() as u16, strings.len() as u16] {
            table.extend(n.to_be_bytes());
        }
        strings.extend(encoded);
    }
    table.extend(strings);
    let offset = bytes.len() as u32;
    bytes[name_record + 8..name_record + 12].copy_from_slice(&offset.to_be_bytes());
    bytes[name_record + 12..name_record + 16].copy_from_slice(&(table.len() as u32).to_be_bytes());
    bytes.extend(table);
    bytes
}

#[test]
fn numeric_subfamilies_with_identical_weights_select_distinct_faces() {
    let mut db = FontDb::new();
    db.register_font_data(numeric_font(&INTER_REGULAR, "20", "UnrelatedPS20"));
    db.register_font_data(numeric_font(&INTER_SEMIBOLD, "30", "UnrelatedPS30"));
    let faces = db.faces("Numeric Test");
    assert_eq!(faces.iter().map(|f| f.style.as_str()).collect::<Vec<_>>(), ["20", "30"]);
    assert!(faces.iter().all(|f| f.weight == 400.0));
    assert_eq!(db.resolve_postscript("UnrelatedPS30").family, "Numeric Test");
    assert!(db.resolve_postscript("UnrelatedPS30").exact);
    let mut a = CharStyle { font_family: "Numeric Test".into(), font_style: "20".into(), ..Default::default() };
    let mut b = CharStyle { font_style: "30".into(), ..a.clone() };
    db.select_named_face(&mut a);
    db.select_named_face(&mut b);
    assert_ne!(a.font_family, b.font_family);
    assert_eq!(db.faces(&a.font_family)[0].postscript_name.as_deref(), Some("UnrelatedPS20"));
    assert_eq!(db.faces(&b.font_family)[0].postscript_name.as_deref(), Some("UnrelatedPS30"));
    assert!(!db.families().iter().any(|f| f.starts_with(".PhotoCraft-face-")));
    let alias = b.font_family.clone();
    b.font_family = "Numeric Test".into();
    db.select_named_face(&mut b);
    assert_eq!(b.font_family, alias);
}

#[test]
fn numeric_styles_shape_different_outlines_and_keep_psd_identity() {
    use photocraft_doc::{TextLayer, text::TextRun};
    let mut engine = crate::TextEngine::new();
    engine.fonts.register_font_data(numeric_font(&INTER_REGULAR, "20", "UnrelatedPS20"));
    engine.fonts.register_font_data(numeric_font(&INTER_SEMIBOLD, "30", "UnrelatedPS30"));
    let layer = |style: &str| TextLayer {
        text: "Hamburgefonstiv".into(),
        runs: vec![TextRun {
            len: 14,
            style: CharStyle {
                font_family: "Numeric Test".into(),
                font_style: style.into(),
                size_pt: 32.0,
                postscript_name: Some(format!("UnrelatedPS{style}")),
                ..Default::default()
            },
        }],
        ..Default::default()
    };
    let a = engine.layout(&layer("20"), 72.0);
    let b = engine.layout(&layer("30"), 72.0);
    assert_ne!(a.lines[0].x1, b.lines[0].x1);
    let original = layer("30");
    let bytes = crate::psd::build_tysh(&original, 72.0, None);
    let reopened = crate::psd::text_layer_from_tysh(&bytes, 72.0).unwrap();
    assert_eq!(reopened.char_runs()[0].style.postscript_name.as_deref(), Some("UnrelatedPS30"));
    let c = engine.layout(&reopened, 72.0);
    assert!((b.lines[0].x1 - c.lines[0].x1).abs() < 0.01);
}

#[test]
fn explicit_weight_is_not_overridden_by_a_stale_style_name() {
    let mut db = FontDb::new();
    let mut style = CharStyle { font_family: "Inter".into(), font_style: "Regular".into(), weight: 600, ..Default::default() };
    db.select_named_face(&mut style);
    assert_eq!(style.font_family, "Inter");
    assert_eq!(style.weight, 600);
    assert!(db.named_face("Inter", "Medium").is_some());
    assert!(db.named_face("missing", "20").is_none());
    assert!(isolated_face(Blob::new(Arc::new(vec![0; 16])), 0).is_none());
}

#[test]
fn collection_isolation_preserves_requested_face() {
    // Two faces with identical attributes: the second must not fall back to the first.
    let sources = [numeric_font(&INTER_REGULAR, "20", "Collection20"), numeric_font(&INTER_SEMIBOLD, "30", "Collection30")];
    let mut bytes = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
    bytes.extend(20u32.to_be_bytes());
    bytes.extend((20 + sources[0].len() as u32).to_be_bytes());
    for source in sources {
        let start = bytes.len();
        bytes.extend(source);
        let count = u16::from_be_bytes(bytes[start + 4..start + 6].try_into().unwrap()) as usize;
        for i in 0..count {
            let r = start + 12 + i * 16 + 8;
            let offset = u32::from_be_bytes(bytes[r..r + 4].try_into().unwrap()) + start as u32;
            bytes[r..r + 4].copy_from_slice(&offset.to_be_bytes());
        }
    }
    let blob = Blob::new(Arc::new(bytes));
    assert!(isolated_face(blob.clone(), 2).is_none());
    let mut db = FontDb::new();
    db.register_blob(blob.clone(), None);
    let mut style = CharStyle { font_family: "Numeric Test".into(), font_style: "30".into(), ..Default::default() };
    db.select_named_face(&mut style);
    assert_eq!(db.faces(&style.font_family)[0].postscript_name.as_deref(), Some("Collection30"));
    let isolated = isolated_face(blob, 1).unwrap();
    let font = skrifa::FontRef::from_index(isolated.as_ref(), 0).unwrap();
    assert_eq!(font.localized_strings(StringId::POSTSCRIPT_NAME).english_or_first().unwrap().to_string(), "Collection30");
}

#[test]
#[ignore = "requires locally installed Yoon 500 fonts; no font files are distributed"]
fn installed_yoon_numeric_faces() {
    let mut db = FontDb::with_system_fonts();
    for family in ["Yoon YoonGothic500", "Yoon YoonMyungjo500"] {
        let faces = db.faces(family);
        for name in ["20", "30", "40", "50"] {
            let face = faces.iter().find(|f| f.style == name).expect("installed numeric face");
            let mut style =
                CharStyle { font_family: family.into(), font_style: name.into(), postscript_name: face.postscript_name.clone(), ..Default::default() };
            db.select_named_face(&mut style);
            assert_eq!(db.faces(&style.font_family)[0].postscript_name, face.postscript_name);
        }
    }
}

/// A copy of `source` whose name table spells the family in English and in Japanese.
fn localized_font(source: &[u8], english: &str, japanese: &str) -> Vec<u8> {
    let mut bytes = source.to_vec();
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let name_record = (0..count).map(|i| 12 + i * 16).find(|r| &bytes[*r..r + 4] == b"name").unwrap();
    let ps = english.replace(' ', "");
    let names = [(0x409u16, 1u16, english), (0x409, 2, "Regular"), (0x409, 6, ps.as_str()), (0x409, 16, english), (0x411, 1, japanese), (0x411, 16, japanese)];
    let mut table = Vec::new();
    let mut strings = Vec::new();
    for value in [0u16, names.len() as u16, (6 + names.len() * 12) as u16] {
        table.extend(value.to_be_bytes());
    }
    for (lang, id, value) in names {
        let encoded: Vec<_> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for n in [3, 1, lang, id, encoded.len() as u16, strings.len() as u16] {
            table.extend(n.to_be_bytes());
        }
        strings.extend(encoded);
    }
    table.extend(strings);
    let offset = bytes.len() as u32;
    bytes[name_record + 8..name_record + 12].copy_from_slice(&offset.to_be_bytes());
    bytes[name_record + 12..name_record + 16].copy_from_slice(&(table.len() as u32).to_be_bytes());
    bytes.extend(table);
    bytes
}

/// Show Font Names in English off: a font that names its family in Japanese is listed by that
/// name, keyed by the English family the menus and documents keep; English-only fonts are not.
#[test]
fn localized_family_names_map_english_families_to_native_names() {
    let mut db = FontDb::new();
    assert_eq!(db.register_font_data(localized_font(&INTER_REGULAR, "Gothic Test", "ゴシック テスト")), ["Gothic Test"]);
    assert!(db.families().iter().any(|f| f == "Gothic Test"));
    let local = db.localized_family_names();
    assert_eq!(local.get("Gothic Test").map(String::as_str), Some("ゴシック テスト"));
    assert!(!local.contains_key(DEFAULT_FAMILY));
    assert!(local.values().all(|v| !v.is_empty()));
}

/// A craft-fonts manifest entry for `bytes`, as `build.rs` writes one per manifest line.
fn craft(family: &'static str, scripts: &'static [&'static str], bytes: Vec<u8>) -> crate::CraftFont {
    crate::CraftFont { family, style: "Regular", scripts, bytes: bytes.leak() }
}

/// A TrueType collection of single-face fonts.
fn collection(faces: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"ttcf\0\x01\0\0".to_vec();
    bytes.extend((faces.len() as u32).to_be_bytes());
    let mut start = 12 + 4 * faces.len();
    for face in faces {
        bytes.extend((start as u32).to_be_bytes());
        start += face.len();
    }
    for face in faces {
        let start = bytes.len();
        bytes.extend(face);
        let count = u16::from_be_bytes(bytes[start + 4..start + 6].try_into().unwrap()) as usize;
        for i in 0..count {
            let r = start + 12 + i * 16 + 8;
            let offset = u32::from_be_bytes(bytes[r..r + 4].try_into().unwrap()) + start as u32;
            bytes[r..r + 4].copy_from_slice(&offset.to_be_bytes());
        }
    }
    bytes
}

/// Single-face craft fonts of every script register under their manifest family names, whatever
/// family their `name` tables give, and the ones the fallback lists name are in the fallback
/// stack. The Hans and Arab families are fallback-list names, and no craft-fonts manifest uses
/// any of the three, so `FontDb::new` does not register them from the build's own craft fonts.
#[test]
fn craft_fonts_of_every_script_register_under_their_manifest_families() {
    let fonts = [
        craft("WenQuanYi Micro Hei", &["Hans", "Latn"], localized_font(&INTER_REGULAR, "WenQuanYi Micro Hei", "WenQuanYi Micro Hei")),
        // The `name` tables of these two give the family "Numeric Test".
        craft("Geeza Pro", &["Arab"], numeric_font(&INTER_REGULAR, "20", "CraftTestArabic")),
        craft("Craft Test Gothic", &["Jpan", "Latn"], numeric_font(&INTER_SEMIBOLD, "30", "CraftTestGothic")),
    ];
    let mut db = FontDb::new();
    assert!(fonts.iter().all(|f| !db.has_family(f.family)));
    db.register_craft_fonts(&fonts);
    let families = db.families();
    for f in &fonts {
        assert!(families.iter().any(|n| n == f.family), "{} registered under its manifest family: {families:?}", f.family);
    }
    assert!(!db.has_family("Numeric Test"), "{families:?}");
    for family in ["WenQuanYi Micro Hei", "Geeza Pro"] {
        assert!(db.fallback_stack().any(|f| f == family), "{family} in {:?}", db.fallback_stack().collect::<Vec<_>>());
    }
}

/// Each face of a craft font collection keeps the family its `name` table gives.
#[test]
fn craft_font_collections_keep_their_face_families() {
    let faces = [localized_font(&INTER_REGULAR, "Craft Test A", "クラフト A"), localized_font(&INTER_SEMIBOLD, "Craft Test B", "クラフト B")];
    let fonts = [craft("Craft Test Collection", &["Jpan", "Latn"], collection(&faces))];
    let mut db = FontDb::new();
    db.register_craft_fonts(&fonts);
    assert!(db.has_family("Craft Test A") && db.has_family("Craft Test B"), "{:?}", db.families());
    assert!(!db.has_family("Craft Test Collection"));
}

/// A 16.16 fixed-point number as `fvar` stores it.
fn fixed(v: f32) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

/// `source` rebuilt as a variable font with `wght` 100–900 and `wdth` 75–100 axes and two named
/// instances, without redistributing a variable font (#8797): "Light Condensed" (300, 75) has no
/// PostScript name of its own, so its name is derived as `InstanceTest-LightCondensed`;
/// "Heavy" (900, 100) names itself `InstanceTest-Black`. Outlines do not vary, but an `HVAR` table
/// (see [`width_hvar`]) narrows every advance on `wdth`, so layout width shows which instance drew.
fn instance_font(source: &[u8]) -> Vec<u8> {
    let names = [
        (1u16, "Instance Test"),
        (2, "Regular"),
        (6, "InstanceTest-Regular"),
        (16, "Instance Test"),
        (256, "Weight"),
        (257, "Width"),
        (258, "Light Condensed"),
        (259, "InstanceTest-Black"),
        (260, "Heavy"),
    ];
    let mut name = Vec::new();
    let mut strings = Vec::new();
    for value in [0u16, names.len() as u16, (6 + names.len() * 12) as u16] {
        name.extend(value.to_be_bytes());
    }
    for (id, value) in names {
        let encoded: Vec<_> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for n in [3, 1, 0x409, id, encoded.len() as u16, strings.len() as u16] {
            name.extend(n.to_be_bytes());
        }
        strings.extend(encoded);
    }
    name.extend(strings);
    // Two axes of 20 bytes, then two instances of 4 + 2 * 4 + 2 bytes (with PostScript name IDs).
    let mut fvar = Vec::new();
    for n in [1u16, 0, 16, 2, 2, 20, 2, 14] {
        fvar.extend(n.to_be_bytes());
    }
    for (tag, min, default, max, name_id) in [(b"wght", 100.0, 400.0, 900.0, 256u16), (b"wdth", 75.0, 100.0, 100.0, 257)] {
        fvar.extend(tag);
        for v in [min, default, max] {
            fvar.extend(fixed(v));
        }
        fvar.extend([0, 0]);
        fvar.extend(name_id.to_be_bytes());
    }
    for (subfamily, coords, ps) in [(258u16, [300.0, 75.0], 0xFFFFu16), (260, [900.0, 100.0], 259)] {
        fvar.extend(subfamily.to_be_bytes());
        fvar.extend([0, 0]);
        for v in coords {
            fvar.extend(fixed(v));
        }
        fvar.extend(ps.to_be_bytes());
    }
    // Rebuild the table directory: every table but `name` as it was, plus the new `name`, `fvar`
    // and `HVAR`.
    let count = u16::from_be_bytes(source[4..6].try_into().unwrap()) as usize;
    let mut tables: Vec<([u8; 4], Vec<u8>)> = (0..count)
        .map(|i| {
            let r = 12 + i * 16;
            let at = u32::from_be_bytes(source[r + 8..r + 12].try_into().unwrap()) as usize;
            let len = u32::from_be_bytes(source[r + 12..r + 16].try_into().unwrap()) as usize;
            (source[r..r + 4].try_into().unwrap(), source[at..at + len].to_vec())
        })
        .filter(|(tag, _)| tag != b"name")
        .collect();
    let table = |tag: &[u8; 4]| tables.iter().find(|(t, _)| t == tag).map(|(_, data)| data.as_slice()).unwrap();
    let hvar = width_hvar(table(b"hhea"), table(b"hmtx"), table(b"maxp"));
    tables.push((*b"name", name));
    tables.push((*b"fvar", fvar));
    tables.push((*b"HVAR", hvar));
    tables.sort_by_key(|(tag, _)| *tag);
    let mut out = source[0..4].to_vec();
    for n in [tables.len() as u16, 0, 0, 0] {
        out.extend(n.to_be_bytes());
    }
    let mut at = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.extend(tag);
        out.extend(0u32.to_be_bytes());
        out.extend((at as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        at += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

/// An `HVAR` table for [`instance_font`]'s axes (`wght`, then `wdth`) that takes 30% off every
/// glyph's advance at `wdth` 75 and nothing at the default 100, scaled linearly between. Advances
/// come from `hmtx`; glyph IDs map to deltas directly, as no advance mapping is given.
fn width_hvar(hhea: &[u8], hmtx: &[u8], maxp: &[u8]) -> Vec<u8> {
    let glyphs = u16::from_be_bytes(maxp[4..6].try_into().unwrap());
    let metrics = u16::from_be_bytes(hhea[34..36].try_into().unwrap()) as usize;
    let advance = |gid: usize| {
        let at = 4 * gid.min(metrics - 1);
        i32::from(u16::from_be_bytes(hmtx[at..at + 2].try_into().unwrap()))
    };
    let mut out = Vec::new();
    // Header: version 1.0, item variation store at 20, no advance, LSB or RSB mappings.
    for n in [1u16, 0] {
        out.extend(n.to_be_bytes());
    }
    for n in [20u32, 0, 0, 0] {
        out.extend(n.to_be_bytes());
    }
    // Item variation store: format 1, region list at 12, one item variation data at 28.
    out.extend(1u16.to_be_bytes());
    out.extend(12u32.to_be_bytes());
    out.extend(1u16.to_be_bytes());
    out.extend(28u32.to_be_bytes());
    // One region over two axes as (start, peak, end) in F2Dot14: `wght` unused, `wdth` -1..0
    // peaking at -1 (75).
    for n in [2u16, 1, 0, 0, 0, 0xC000, 0xC000, 0] {
        out.extend(n.to_be_bytes());
    }
    // Item variation data: one 16-bit delta per glyph for region 0.
    for n in [glyphs, 1, 1, 0] {
        out.extend(n.to_be_bytes());
    }
    for gid in 0..glyphs as usize {
        out.extend((-(advance(gid) * 3 / 10) as i16).to_be_bytes());
    }
    out
}

/// The axis values `select_named_face` gives a style of the synthesized variable family that
/// carries the PostScript name `ps`, sorted by axis.
fn instance_axes(db: &mut FontDb, ps: &str, variations: Vec<photocraft_doc::text::FontVariation>) -> Vec<(String, f32)> {
    let mut style = CharStyle { font_family: "Instance Test".into(), postscript_name: Some(ps.into()), weight: 300, variations, ..Default::default() };
    db.select_named_face(&mut style);
    let mut axes: Vec<_> = style.variations.into_iter().map(|v| (v.axis, v.value)).collect();
    axes.sort_by(|a, b| a.0.cmp(&b.0));
    axes
}

/// #8797: a PostScript name that names a named instance of a variable font (as a PSD stores
/// "Bahnschrift-LightCondensed") selects that instance's axis values, whether the instance names
/// itself or its name is derived from the family and subfamily names. A name no instance has
/// leaves the style at the default instance, and axes the style sets itself are kept.
#[test]
fn postscript_name_of_a_named_instance_selects_its_axis_values() {
    use photocraft_doc::text::FontVariation;
    let mut db = FontDb::new();
    db.register_font_data(instance_font(&INTER_REGULAR));
    assert_eq!(db.faces("Instance Test").len(), 1, "a variable family is one face");
    let axes = |pairs: &[(&str, f32)]| pairs.iter().map(|(a, v)| (a.to_string(), *v)).collect::<Vec<_>>();
    assert_eq!(instance_axes(&mut db, "InstanceTest-LightCondensed", vec![]), axes(&[("wdth", 75.0), ("wght", 300.0)]));
    assert_eq!(instance_axes(&mut db, "InstanceTest-Black", vec![]), axes(&[("wdth", 100.0), ("wght", 900.0)]));
    assert_eq!(instance_axes(&mut db, "InstanceTest-Missing", vec![]), axes(&[]));
    let own = vec![FontVariation { axis: "wght".into(), value: 350.0 }];
    assert_eq!(instance_axes(&mut db, "InstanceTest-LightCondensed", own), axes(&[("wdth", 75.0), ("wght", 350.0)]));
}

/// A static face whose PostScript name matches exactly is selected as before (#8797 only adds
/// a fallback for names no face has), and gets no axis values.
#[test]
fn exact_static_face_gets_no_instance_axes() {
    let mut db = FontDb::new();
    db.register_font_data(numeric_font(&INTER_SEMIBOLD, "30", "StaticPS30"));
    let mut style = CharStyle { font_family: "Numeric Test".into(), postscript_name: Some("StaticPS30".into()), ..Default::default() };
    db.select_named_face(&mut style);
    assert_ne!(style.font_family, "Numeric Test", "the exact face is selected through its alias");
    assert!(style.variations.is_empty(), "{:?}", style.variations);
}

/// A style whose `font_style` and weight also match the variable face itself ("Regular", 400) still
/// gets the named instance its PostScript name gives: the instance is tried before the
/// style-and-weight fallback, which would select the face at its default instance (#8797).
#[test]
fn named_instance_wins_over_a_matching_font_style() {
    let mut db = FontDb::new();
    db.register_font_data(instance_font(&INTER_REGULAR));
    let mut style = CharStyle {
        font_family: "Instance Test".into(),
        font_style: "Regular".into(),
        weight: 400,
        postscript_name: Some("InstanceTest-LightCondensed".into()),
        ..Default::default()
    };
    db.select_named_face(&mut style);
    assert_ne!(style.font_family, "Instance Test", "the variable face is selected through its alias");
    let mut axes: Vec<_> = style.variations.into_iter().map(|v| (v.axis, v.value)).collect();
    axes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(axes, vec![("wdth".to_string(), 75.0), ("wght".to_string(), 300.0)]);
}

/// #8797 end to end without an installed font: text whose style carries a named instance's
/// PostScript name lays out as wide as that instance's axis values, which the synthesized `HVAR`
/// makes narrower than the default instance. The style's `font_style` and weight match the
/// variable face itself, so the default instance would be chosen if the instance were skipped.
#[test]
fn postscript_name_of_a_named_instance_lays_out_at_that_instance() {
    use photocraft_doc::TextLayer;
    use photocraft_doc::text::{FontVariation, TextRun};
    let mut engine = crate::TextEngine::new();
    engine.fonts.register_font_data(instance_font(&INTER_REGULAR));
    let text = "Hamburgefonstiv 0123";
    let mut width = |style: CharStyle| {
        let layer = TextLayer { text: text.into(), runs: vec![TextRun { len: text.len(), style }], ..Default::default() };
        let l = engine.layout(&layer, 96.0);
        l.lines[0].x1 - l.lines[0].x0
    };
    let base = CharStyle { font_family: "Instance Test".into(), font_style: "Regular".into(), size_pt: 72.0, weight: 400, ..Default::default() };
    let axes = vec![FontVariation { axis: "wght".into(), value: 300.0 }, FontVariation { axis: "wdth".into(), value: 75.0 }];
    let family = width(base.clone());
    let instance = width(CharStyle { postscript_name: Some("InstanceTest-LightCondensed".into()), ..base.clone() });
    let explicit = width(CharStyle { variations: axes, ..base });
    assert!(explicit < family * 0.8, "wdth 75 is narrower: {explicit} vs {family}");
    assert!((instance - explicit).abs() < 0.5, "instance: {instance} px, its axes: {explicit} px, default instance: {family} px");
}
