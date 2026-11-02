//! #6272: a Photoshop noise `GdFl` gradient that is not provably constant-RGB still opens as
//! a fill layer — the placeholder ramp stands in where the model has no stops, Photoshop's
//! own pixels ride along as the fill cache — and its raw `GdFl` block survives a re-save byte
//! for byte. Before the fix, `fill_from_desc` returned `None` for those gradients: the layer
//! imported as pixels and the export dropped the `GdFl` (`RASTER_STALE_KEYS`), so opening and
//! re-saving a file silently turned the fill layer into a plain pixel layer.

use photocraft_doc::{Fill, LayerContent};
use photocraft_io::*;
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value, VersionedDescriptor};
use photocraft_psd::testgen;
use photocraft_psd::{Compression, LayerRecord, PsdFile, TaggedBlock, Version};

/// Photoshop's default Noise gradient (RGB, min 0/0/0, max 100/100/100) as `GdFl` block data.
fn default_noise_gdfl() -> Vec<u8> {
    let values = |v: [i32; 4]| Value::List(v.into_iter().map(Value::Integer).collect());
    let gradient = Descriptor::new("Grdn")
        .with("GrdF", Value::Enumerated { type_id: Id::new("GrdF"), value: Id::new("ClNs") })
        .with("ClrS", Value::Enumerated { type_id: Id::new("ClrS"), value: Id::new("RGBC") })
        .with("ShTr", Value::Boolean(false))
        .with("VctC", Value::Boolean(true))
        .with("RndS", Value::Integer(9052369))
        .with("Smth", Value::Integer(2048))
        .with("Mnm ", values([0, 0, 0, 0]))
        .with("Mxm ", values([100, 100, 100, 100]));
    let d = Descriptor::new("null")
        .with("Grad", Value::Descriptor(gradient))
        .with("Angl", Value::UnitFloat { unit: *b"#Ang", value: 0.0 })
        .with("Type", Value::Enumerated { type_id: Id::new("Type"), value: Id::new("CstS") });
    VersionedDescriptor::new(d).to_bytes()
}

#[test]
fn variable_noise_gradient_stays_a_fill_layer_and_keeps_its_gdfl() {
    let _ = UnicodeString::new_nul("x");
    let mut f = testgen::small(Version::Psd, Compression::Raw);
    let gdfl = default_noise_gdfl();
    f.layers_mut().push(LayerRecord {
        name: b"noise fill".to_vec(),
        blocks: vec![TaggedBlock::unicode_name("noise fill"), TaggedBlock::new(*b"GdFl", gdfl.clone())],
        ..Default::default()
    });
    for b in f.layers().last().unwrap().blocks.iter() {
        b.check_structure().unwrap();
    }
    let bytes = f.to_bytes().unwrap();
    let imp = import("in.psd", &bytes).unwrap();
    let l = imp.document.layers.iter().find(|l| l.name == "noise fill").unwrap();
    assert!(matches!(l.content, LayerContent::Fill(Fill::Gradient { .. })), "a variable noise GdFl opens as a gradient fill layer, not pixels");
    let out = export(&imp.document, "out.psd", &ExportOptions::default()).unwrap().bytes;
    let back = PsdFile::from_bytes(&out).unwrap();
    let rec = back.layers().iter().find(|l| l.block(b"GdFl").is_some()).expect("the re-saved layer carries its GdFl");
    assert_eq!(rec.block(b"GdFl").unwrap().data, gdfl, "the raw GdFl survives the round trip byte for byte");
}
