//! Kerning of PSD type layers (#206): manual kerning in EngineData (`AutoKerning`/`Kerning`
//! style-run fields), the automatic mode in `Txt2`, and the layout against Photoshop's own
//! glyph positions.
//!
//! The Photoshop-authored oracles (`corpus/photoshop/text/kerning-*.psd`) run under the
//! `corpus` feature (`cargo xtask test-corpus`).

#[cfg(feature = "corpus")]
use std::path::Path;

use photocraft_doc::text::{CharStyle, Kerning, TextRun};
use photocraft_doc::{Document, Layer, LayerContent, TextLayer};
use photocraft_text::engine_data::Value as E;

/// `(AutoKerning, Kerning)` of every UTF-16 unit of the engine text of a `TySh` block.
fn kerning_fields(tysh: &[u8]) -> Vec<(bool, i64)> {
    let t = photocraft_text::psd::parse_tysh(tysh).unwrap();
    let e = photocraft_text::psd::engine_data(&t.text).unwrap();
    let runs = e.path(&["EngineDict", "StyleRun", "RunArray"]).and_then(E::as_array).unwrap();
    let lens = e.path(&["EngineDict", "StyleRun", "RunLengthArray"]).and_then(E::as_array).unwrap();
    let mut out = Vec::new();
    for (r, n) in runs.iter().zip(lens) {
        let d = r.path(&["StyleSheet", "StyleSheetData"]).unwrap();
        let f = (d.get("AutoKerning").and_then(E::as_bool).unwrap_or(true), d.get("Kerning").and_then(E::as_i64).unwrap_or(0));
        out.extend(std::iter::repeat_n(f, n.as_i64().unwrap() as usize));
    }
    out
}

/// Per character: (mode, manual kern).
fn chars(t: &TextLayer) -> Vec<(Kerning, f32)> {
    let mut out = Vec::new();
    let mut at = 0;
    for r in t.char_runs() {
        let n = t.text.get(at..at + r.len).map_or(0, |s| s.chars().count());
        out.extend(std::iter::repeat_n((r.style.kerning, r.style.kern), n));
        at += r.len;
    }
    out
}

fn layer(text: &str, runs: &[(usize, Kerning, f32)]) -> TextLayer {
    let base = CharStyle { font_family: "Inter".into(), size_pt: 24.0, ..Default::default() };
    let mut t = TextLayer {
        text: text.into(),
        runs: runs.iter().map(|&(len, kerning, kern)| TextRun { len, style: CharStyle { kerning, kern, ..base.clone() } }).collect(),
        ..Default::default()
    };
    t.sync_summary();
    t
}

/// The model survives a PSD round trip, and the EngineData fields follow Photoshop's form: a
/// manual kern after character c sits on c + 1; "no automatic kerning" on c is
/// `AutoKerning false` on c + 2 (how Photoshop reads it).
#[test]
fn manual_and_off_kerning_round_trip_through_psd() {
    use Kerning::{Metrics as M, Off as O};
    // "AVATAR Wave": +100 after A, -50 after T, characters 6..8 without automatic kerning.
    let t = layer("AVATAR Wave", &[(1, O, 100.0), (2, M, 0.0), (1, O, -50.0), (2, M, 0.0), (3, O, 0.0), (2, M, 0.0)]);
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut t2 = t.clone();
    photocraft_text::TextEngine::new().render_layer(&mut t2, doc.resolution_dpi, doc.pixel_format());
    doc.layers.push(Layer::new("kerned", LayerContent::Text(t2)));
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    let LayerContent::Text(b) = &back.layers[0].content else { panic!("not text") };
    assert_eq!(chars(b), chars(&t));

    let tysh = photocraft_text::psd::build_tysh(&t, 72.0, None);
    let f = kerning_fields(&tysh);
    // 11 characters + the trailing paragraph break.
    assert_eq!(f.len(), 12);
    assert_eq!(f[1], (false, 100), "kern after A on V, Photoshop's form");
    assert_eq!(f[4], (true, -50), "kern after T on A; character 2 isn't manual");
    for j in [8, 9, 10] {
        assert!(!f[j].0, "character {} without automatic kerning", j - 2);
    }
    assert!(f.iter().enumerate().filter(|(j, _)| ![1, 4, 8, 9, 10].contains(j)).all(|(_, x)| *x == (true, 0)), "{f:?}");
}

/// #1348: the automatic mode (Metrics/Optical) lives in the document `Txt2` block, one model
/// object per `TextIndex`. A PhotoCraft-authored PSD must carry it or a reopened layer falls
/// back to Metrics — Optical used to be silently lost.
#[test]
fn optical_kerning_round_trips_through_psd() {
    use Kerning::{Metrics as M, Optical as P};
    let t1 = layer("AVATAR", &[(4, P, 0.0), (2, M, 0.0)]);
    let t2 = layer("Wave", &[(2, M, 0.0), (2, P, 0.0)]);
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    for (name, want) in [("one", &t1), ("two", &t2)] {
        let mut t = want.clone();
        photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
        doc.layers.push(Layer::new(name, LayerContent::Text(t)));
    }
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    for (want, l) in [(&t1, &back.layers[0]), (&t2, &back.layers[1])] {
        let LayerContent::Text(b) = &l.content else { panic!("not text") };
        assert_eq!(chars(b), chars(want), "{}", l.name);
    }
}

/// #3514: a fresh document of ordinary Metrics type writes no document `Txt2` block — Photoshop
/// never writes its empty `<< /0 << >> >>` stub, and readers such as Photopea reject it — while an
/// Optical layer still does, because its mode lives only in `Txt2`.
#[test]
fn fresh_metrics_text_writes_no_txt2_block() {
    use photocraft_doc::text::Kerning;
    let saves_txt2 = |kerning: Kerning| {
        let t = layer("AV", &[(2, kerning, 0.0)]);
        let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let mut r = t.clone();
        photocraft_text::TextEngine::new().render_layer(&mut r, doc.resolution_dpi, doc.pixel_format());
        doc.layers.push(Layer::new("t", LayerContent::Text(r)));
        let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
        photocraft_psd::PsdFile::from_bytes(&bytes).unwrap().global_blocks.iter().any(|b| &b.key == b"Txt2")
    };
    assert!(!saves_txt2(Kerning::Metrics), "a plain Metrics layer must not write the empty Txt2 stub");
    assert!(saves_txt2(Kerning::Optical), "Optical lives only in Txt2");
}

/// Two imported layers can name the same Txt2 object; export must update the second
/// preserved TySh pointer so each layer restores its own automatic kerning mode.
#[test]
fn duplicate_preserved_text_indices_get_distinct_txt2_objects() {
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let cases = [("optical", "AV", Kerning::Optical), ("metrics", "BC", Kerning::Metrics)];
    for (name, text, mode) in cases {
        let mut t = layer(text, &[(text.len(), mode, 0.0)]);
        photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
        let raw = photocraft_text::psd::build_tysh(&t, doc.resolution_dpi, None);
        t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&raw, 0).unwrap()));
        doc.layers.push(Layer::new(name, LayerContent::Text(t)));
    }
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    let txt2 = file.global_blocks.iter().find(|b| &b.key == b"Txt2").and_then(|b| photocraft_text::psd::parse_txt2(&b.data)).unwrap();
    assert_eq!(txt2.path(&["1", "1"]).and_then(E::as_array).unwrap().len(), 2);
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    for (i, expected) in [Kerning::Optical, Kerning::Metrics].into_iter().enumerate() {
        let LayerContent::Text(t) = &back.layers[i].content else { panic!("expected text layer") };
        assert!(chars(t).iter().all(|(mode, _)| *mode == expected), "layer {i} used another layer's Txt2 object");
    }
}

/// A type layer added below preserved ones takes a number past theirs: claiming the next free
/// number on the way up renumbered every layer above it onto its neighbour's `Txt2` object
/// (#7773, 70 of 74 layers on the #7698 file).
#[test]
fn new_text_layer_below_keeps_the_preserved_text_indices() {
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    for (i, text) in ["AV", "BC"].into_iter().enumerate() {
        let mut t = layer(text, &[(2, Kerning::Optical, 0.0)]);
        photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
        let raw = photocraft_text::psd::build_tysh(&t, doc.resolution_dpi, None);
        t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&raw, i as i32).unwrap()));
        doc.layers.push(Layer::new(text, LayerContent::Text(t)));
    }
    let mut t = layer("new", &[(3, Kerning::Metrics, 0.0)]);
    photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
    doc.layers.insert(0, Layer::new("new", LayerContent::Text(t)));
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    let indices: Vec<_> = back
        .layers
        .iter()
        .map(|l| {
            let LayerContent::Text(t) = &l.content else { panic!("not text") };
            (l.name.as_str(), t.psd_raw.as_deref().and_then(|d| photocraft_text::psd::text_index(d)))
        })
        .collect();
    assert_eq!(indices, [("new", Some(2)), ("AV", Some(0)), ("BC", Some(1))]);
}

/// A Photoshop-shaped `Txt2` block: one object per `(text, style keys)` pair, each with its view
/// naming its own frame.
fn photoshop_txt2(objects: &[(&str, &str)]) -> Vec<u8> {
    let bom = "\u{fe}\u{ff}";
    let frame = "<< /0 << /2 << /2 [ 1.0 0.0 0.0 1.0 5.0 6.0 ] >> >> >> ";
    let mut frames = String::new();
    let mut body = String::new();
    for (i, (text, style)) in objects.iter().enumerate() {
        let utf16: String = format!("{text}\r").chars().flat_map(|c| ['\0', c]).collect();
        let len = text.len() + 1;
        frames.push_str(frame);
        body.push_str(&format!(
            "<< /0 << /0 ({bom}{utf16}) /5 << /0 [ << /0 << /0 << /0 ({bom}) /5 << /7 1.2 >> >> >> /1 {len} >> ] >> /6 << /0 [ << /0 << /0 << /0 ({bom}) /6 << /0 0 /1 12.0 /11 1 /53 << /99 /SimplePaint /0 << /0 1 /1 [ 1.0 0.0 0.0 0.0 ] >> >> {style} >> >> >> /1 {len} >> ] >> >> /1 << /0 [ << /0 {i} >> ] /1 << /0 [ << /0 << /1 1 >> /1 {len} >> ] >> /2 [ << /99 /PC /6 [ << /99 /F /10 {i} >> ] >> ] >> >> "
        ));
    }
    let font: String = "MyriadPro-Regular".chars().flat_map(|c| ['\0', c]).collect();
    format!("\n\n/98 << /0 7 >> /0 << /1 << /0 [ << /0 << /99 /CoolTypeFont /0 << /0 ({bom}{font}) /2 1 >> >> >> ] >> /8 << /0 [ {frames}] >> >> /1 << /0 << /0 1 >> /1 [ {body}] /2 << /0 7 >> >>").chars().map(|c| c as u32 as u8).collect()
}

/// A reopened type layer: its name, `TextIndex` and font features.
type Reopened = (String, Option<i32>, Vec<String>);

/// A Photoshop file whose `Txt2` holds `objects` and whose type layers are `kept` (text and
/// `TextIndex`, bottom first), with a type layer "Hi" made the way the Type tool makes one
/// (`build_tysh`, no index of its own) inserted at `at`, saved and reopened: each layer's
/// index and font features (but `calt`, on by default and written on as Photoshop does), and
/// "Hi"'s style run in the saved `Txt2`.
fn add_type_layer(objects: &[(&str, &str)], kept: &[(&str, i32)], at: usize) -> (Vec<Reopened>, E) {
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    for (text, index) in kept {
        let mut t = layer(text, &[(text.len(), Kerning::Metrics, 0.0)]);
        photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
        let raw = photocraft_text::psd::build_tysh(&t, doc.resolution_dpi, None);
        t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&raw, *index).unwrap()));
        doc.layers.push(Layer::new(*text, LayerContent::Text(t)));
    }
    doc.metadata.psd_global_blocks = vec![(*b"8BIM", *b"Txt2", std::sync::Arc::new(photoshop_txt2(objects)))];
    let mut t = layer("Hi", &[(2, Kerning::Metrics, 0.0)]);
    t.runs[0].style.size_pt = 30.0;
    photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
    t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::build_tysh(&t, doc.resolution_dpi, None)));
    doc.layers.insert(at, Layer::new("Hi", LayerContent::Text(t)));
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    let txt2 = file.global_blocks.iter().find(|b| &b.key == b"Txt2").and_then(|b| photocraft_text::psd::parse_txt2(&b.data)).unwrap();
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    let layers: Vec<_> = back
        .layers
        .iter()
        .map(|l| {
            let LayerContent::Text(t) = &l.content else { panic!("not text") };
            let index = t.psd_raw.as_deref().and_then(|d| photocraft_text::psd::text_index(d));
            (l.name.clone(), index, t.char_runs().iter().flat_map(|r| r.style.features.iter().filter(|f| f.tag != "calt").map(|f| f.tag.clone())).collect())
        })
        .collect();
    let hi = layers.iter().find(|l| l.0 == "Hi").and_then(|l| l.1).unwrap() as usize;
    let run = txt2.path(&["1", "1"]).and_then(E::as_array).unwrap()[hi].path(&["0", "6", "0"]).and_then(E::as_array).unwrap()[0].clone();
    (layers, run)
}

/// The layer `name` in `add_type_layer`'s result.
fn named<'a>(layers: &'a [Reopened], name: &str) -> &'a Reopened {
    layers.iter().find(|l| l.0 == name).unwrap()
}

/// A new type layer takes the structure of one of the file's objects but not its style: the
/// template's style runs carried its font features (here `salt` and `ss01`) into the new layer.
/// Its style run is the template's complete one (Photoshop quit on a run holding only the
/// auto-kern mode, and reads the character style from it) with the layer's own size.
#[test]
fn new_text_layer_takes_no_style_from_the_txt2_template() {
    let (layers, run) = add_type_layer(&[("AV", ""), ("BC", "/28 true /85 1")], &[("AV", 0), ("BC", 1)], 2);
    assert_eq!(named(&layers, "Hi").1..=named(&layers, "Hi").1, Some(2)..=Some(2));
    assert!(named(&layers, "Hi").2.is_empty(), "{layers:?}");
    let style = run.path(&["0", "0", "6"]).unwrap();
    assert_eq!(style.get("1").and_then(E::as_f64), Some(30.0), "the layer's own size");
    assert_eq!(style.get("28"), Some(&E::Bool(false)));
    assert_eq!(style.path(&["53", "0", "0"]), Some(&E::Int(1)), "the layer's fill colour, RGB");
    assert_eq!(style.get("11"), Some(&E::Int(1)));
}

/// A new type layer is numbered past the file's `Txt2` objects, not only past the kept indices:
/// a deleted type layer's object ("Gone") is not reused with its style, whether it held the
/// highest index or index 0 (the Type tool's layer used to bring a `TextIndex` 0 of its own).
#[test]
fn new_text_layer_skips_a_deleted_layers_txt2_object() {
    let (layers, _) = add_type_layer(&[("AV", ""), ("BC", ""), ("Gone", "/28 true")], &[("AV", 0), ("BC", 1)], 2);
    assert_eq!((named(&layers, "Hi").1, named(&layers, "Hi").2.is_empty()), (Some(3), true));
    let (layers, _) = add_type_layer(&[("Gone", "/28 true"), ("AV", ""), ("BC", "")], &[("AV", 1), ("BC", 2)], 2);
    assert_eq!((named(&layers, "Hi").1, named(&layers, "Hi").2.is_empty()), (Some(3), true));
}

/// A type layer added below the one holding index 0 neither takes that index nor its style.
#[test]
fn new_text_layer_at_the_bottom_keeps_index_0_where_it_was() {
    let (layers, _) = add_type_layer(&[("AV", "/28 true"), ("BC", "")], &[("AV", 0), ("BC", 1)], 0);
    assert_eq!(named(&layers, "AV").1, Some(0));
    assert_eq!(named(&layers, "AV").2, ["salt"]);
    assert_eq!((named(&layers, "Hi").1, named(&layers, "Hi").2.is_empty()), (Some(2), true));
}

/// An edited object takes the layer's style, not the old text's: "Hello world" with Stylistic
/// Alternates on "Hello " edited to "world" without them comes back without `salt`.
#[test]
fn edited_text_layer_takes_its_own_style_runs() {
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut t = layer("world", &[(5, Kerning::Metrics, 0.0)]);
    photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
    let raw = photocraft_text::psd::build_tysh(&t, doc.resolution_dpi, None);
    t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&raw, 0).unwrap()));
    doc.layers.push(Layer::new("edited", LayerContent::Text(t)));
    doc.metadata.psd_global_blocks = vec![(*b"8BIM", *b"Txt2", std::sync::Arc::new(photoshop_txt2(&[("Hello world", "/28 true")])))];
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    let LayerContent::Text(t) = &back.layers[0].content else { panic!("not text") };
    assert_eq!(t.text, "world");
    assert!(t.char_runs().iter().all(|r| r.style.features.iter().all(|f| f.tag == "calt")), "the old text's salt came back");
}

/// A file-controlled `TextIndex` must not size the save: `i32::MAX` is ignored (the regenerated
/// `Txt2` is sized by real text objects, never by a file's numbers) and the document still saves
/// and reopens with its text.
#[test]
fn hostile_text_index_never_sizes_the_save() {
    let t = layer("AV", &[(1, Kerning::Metrics, 0.0), (1, Kerning::Optical, 0.0)]);
    let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut r = t.clone();
    photocraft_text::TextEngine::new().render_layer(&mut r, doc.resolution_dpi, doc.pixel_format());
    // An invalid index is remapped into the saved Txt2 rather than preserved.
    r.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&photocraft_text::psd::build_tysh(&r, 72.0, None), i32::MAX).unwrap()));
    doc.layers.push(Layer::new("hostile", LayerContent::Text(r)));
    let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
    let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    let txt2 = file.global_blocks.iter().find(|b| &b.key == b"Txt2").and_then(|b| photocraft_text::psd::parse_txt2(&b.data)).unwrap();
    let slots = txt2.path(&["1", "1"]).and_then(E::as_array).unwrap();
    assert!(slots.len() <= 8, "a file-controlled TextIndex sized the save: {} slots", slots.len());
    let back = photocraft_io::import("k.psd", &bytes).unwrap().document;
    let LayerContent::Text(b) = &back.layers[0].content else { panic!("not text") };
    assert_eq!(b.text, "AV");
    assert_eq!(chars(b), chars(&t), "a remapped TextIndex must still restore Optical kerning");
}

/// A save must not drop what the file held beyond the style runs: the previous `Txt2`'s glyph
/// pen positions (`/21 /1`) survive a resave of unchanged text, and go away with the text they
/// described rather than lying about the new one.
#[test]
fn txt2_extras_survive_a_resave_of_unchanged_text() {
    let obj = E::Dict(vec![
        ("0".into(), E::Dict(vec![("0".into(), E::String("AV\r".into())), ("6".into(), E::Dict(vec![("0".into(), E::Array(vec![]))]))])),
        ("21".into(), E::Dict(vec![("1".into(), E::Array(vec![E::Real(10.0), E::Real(20.0)]))])),
    ]);
    let prev = photocraft_text::engine_data::write_bare(&[
        ("98".into(), E::Dict(vec![("0".into(), E::Int(14))])),
        ("0".into(), E::dict()),
        ("1".into(), E::Dict(vec![("1".into(), E::Array(vec![obj]))])),
    ]);
    let save = |text: &str| {
        let t = layer(text, &[(text.len(), Kerning::Metrics, 0.0)]);
        let mut doc = Document::new("k", photocraft_geom::Size::new(64, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let mut r = t.clone();
        photocraft_text::TextEngine::new().render_layer(&mut r, doc.resolution_dpi, doc.pixel_format());
        // The file's layer: its `TySh` names object 0.
        let raw = photocraft_text::psd::build_tysh(&r, doc.resolution_dpi, None);
        r.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::set_text_index(&raw, 0).unwrap()));
        doc.layers.push(Layer::new("kept", LayerContent::Text(r)));
        doc.metadata.psd_global_blocks.push((*b"8BIM", *b"Txt2", std::sync::Arc::new(prev.clone())));
        let bytes = photocraft_io::export(&doc, "k.psd", &Default::default()).unwrap().bytes;
        let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
        photocraft_text::psd::parse_txt2(&file.global_blocks.iter().find(|b| &b.key == b"Txt2").unwrap().data).unwrap()
    };
    let kept = save("AV");
    let object = kept.path(&["1", "1"]).and_then(E::as_array).unwrap()[0].clone();
    assert_eq!(object.path(&["21", "1"]).and_then(E::as_array).map(|items| items.len()), Some(2), "pen positions survive a resave of unchanged text");
    let stale = save("AX");
    let object = stale.path(&["1", "1"]).and_then(E::as_array).unwrap()[0].clone();
    assert!(object.get("21").is_none(), "positions of another text are stale and dropped");
}

/// Kerning that can't be represented (non-finite) and bad PSD fields never panic and stay sane.
#[test]
fn hostile_kerning_values() {
    let t = layer("AV", &[(1, Kerning::Off, f32::NAN), (1, Kerning::Metrics, f32::INFINITY)]);
    let tysh = photocraft_text::psd::build_tysh(&t, 72.0, None);
    assert!(kerning_fields(&tysh).iter().all(|f| f.1 == 0));
    let back = photocraft_text::psd::text_layer_from_tysh(&tysh, 72.0).unwrap();
    assert!(chars(&back).iter().all(|c| c.1 == 0.0));
    // Huge values clamp.
    let t = layer("AV", &[(1, Kerning::Off, 1e30), (1, Kerning::Metrics, 0.0)]);
    let f = kerning_fields(&photocraft_text::psd::build_tysh(&t, 72.0, None));
    assert_eq!(f[1].1, 10_000);
}

/// Glyph pen positions Photoshop stored for text object `index` in `Txt2` (`/21 /1`).
#[cfg(feature = "corpus")]
fn photoshop_positions(txt2: &E, index: usize) -> Option<Vec<f64>> {
    fn find(v: &E, depth: usize) -> Option<Vec<f64>> {
        if depth > 100 {
            return None;
        }
        match v {
            E::Dict(items) => {
                if let Some(p) = v.get("21").and_then(|d| d.get("1")).and_then(E::as_array) {
                    return Some(p.iter().filter_map(E::as_f64).collect());
                }
                items.iter().find_map(|(_, x)| find(x, depth + 1))
            }
            E::Array(a) => a.iter().find_map(|x| find(x, depth + 1)),
            _ => None,
        }
    }
    find(txt2.path(&["1", "1"])?.as_array()?.get(index)?, 0)
}

/// Photoshop-authored kerning oracles (`corpus/photoshop/text/kerning-*.psd`, made from scratch
/// by photocraft-corpus's generator): the automatic mode read from `Txt2` per character, the
/// EngineData kerning fields after export, and our layout against Photoshop's glyph positions.
#[cfg(feature = "corpus")]
#[test]
fn photoshop_kerning_oracles() {
    use Kerning::{Metrics as M, Off as O, Optical as P};
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/photoshop/text");
    assert!(dir.is_dir(), "{} is missing: run `cargo xtask corpus --all`", dir.display());
    let mut engine = photocraft_text::TextEngine::with_system_fonts();
    let arial = engine.fonts.has_family("Arial");
    // "AVATAR Wavy To. LT" (18 characters).
    let all = |m: Kerning| vec![(m, 0.0f32); 18];
    let partial: Vec<(Kerning, f32)> = (0..18).map(|i| (if (1..=3).contains(&i) { O } else { M }, 0.0)).collect();
    // (file, expected modes, EngineData fields as Photoshop wrote them survive our export,
    // worst / mean advance error allowed in px).
    type Case = (&'static str, Vec<(Kerning, f32)>, bool, f64, f64);
    let cases: [Case; 5] = [
        ("kerning-metrics.psd", all(M), true, 0.02, 0.02),
        ("kerning-off.psd", all(O), false, 0.02, 0.02),
        ("kerning-off-partial.psd", partial, false, 0.02, 0.02),
        // The fitted optical model: within ~30/1000 em per pair, ~12 on average.
        ("kerning-optical.psd", all(P), true, 1.0, 0.4),
        ("kerning-optical-24pt.psd", all(P), true, 0.75, 0.3),
    ];
    for (name, want, same_fields, worst_max, mean_max) in cases {
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
        let txt2 = file.global_blocks.iter().find(|b| &b.key == b"Txt2").and_then(|b| photocraft_text::psd::parse_txt2(&b.data)).unwrap();
        let doc = photocraft_io::import(name, &bytes).unwrap().document;
        let t = doc
            .layers
            .iter()
            .find_map(|l| match &l.content {
                LayerContent::Text(t) => Some(t),
                _ => None,
            })
            .unwrap();
        assert_eq!(chars(t), want, "{name}");

        let original = t.psd_raw.as_ref().map(|r| r.to_vec()).unwrap();
        let tysh = photocraft_text::psd::parse_tysh(&original).unwrap();
        let rebuilt = photocraft_text::psd::build_tysh(t, doc.resolution_dpi, None);
        if same_fields {
            assert_eq!(kerning_fields(&rebuilt), kerning_fields(&original), "{name}");
        }
        // Re-import (with the document's Txt2, as an exported file keeps it): same model.
        let mut back = photocraft_text::psd::text_layer_from_tysh(&rebuilt, doc.resolution_dpi).unwrap();
        photocraft_text::psd::apply_txt2(&mut back, &rebuilt, &txt2);
        assert_eq!(chars(&back), want, "{name} after export");

        // Layout against Photoshop's glyph positions (needs Arial, as Photoshop used).
        if !arial {
            eprintln!("{name}: Arial not installed, layout comparison skipped");
            continue;
        }
        let index = match &tysh.text.get("TextIndex") {
            Some(photocraft_psd::descriptor::Value::Integer(i)) => *i as usize,
            _ => panic!("{name}: no TextIndex"),
        };
        let ps = photoshop_positions(&txt2, index).unwrap();
        let x: Vec<f64> = engine.layout(t, doc.resolution_dpi).glyphs.iter().map(|g| f64::from(g.x)).collect();
        let (mut worst, mut sum) = (0.0f64, 0.0f64);
        for i in 0..x.len() - 1 {
            let d = (x[i + 1] - x[i]) - (ps[i + 1] - ps[i]);
            worst = worst.max(d.abs());
            sum += d.abs();
        }
        let mean = sum / (x.len() - 1) as f64;
        eprintln!("{name}: advance error vs Photoshop: mean {mean:.3} px, worst {worst:.3} px");
        assert!(worst <= worst_max && mean <= mean_max, "{name}: mean {mean}, worst {worst}");
    }
}
