//! An adjustment layer whose key PhotoCraft doesn't know (an older or newer adjustment type)
//! opens as an unsupported adjustment with a warning and saves its block back unchanged, instead
//! of opening silently as an empty pixel layer (#6575). Pixel layers that merely carry an unknown
//! block stay pixel layers.

use photocraft_doc::{Adjustment, Document, LayerContent};
use photocraft_io::*;
use photocraft_psd::testgen;
use photocraft_psd::{ChannelData, Compression, LayerFlags, LayerRecord, PsdFile, Rect, TaggedBlock, Version};

/// A layer record as Photoshop writes an adjustment layer: no pixels, "pixel data irrelevant"
/// set, the adjustment block first, then the usual metadata.
fn adjustment_record(name: &str, blocks: Vec<TaggedBlock>) -> LayerRecord {
    let mut flags = LayerFlags::default();
    flags.set_pixel_data_irrelevant(true);
    let mut all = blocks;
    all.extend([TaggedBlock::unicode_name(name), TaggedBlock::layer_id(77)]);
    LayerRecord { name: name.as_bytes().to_vec(), flags, blocks: all, ..Default::default() }
}

fn open(f: &PsdFile) -> (Document, Vec<String>) {
    let bytes = f.to_bytes().expect("write psd");
    let imp = import("x.psd", &bytes).expect("import");
    (imp.document, imp.warnings)
}

fn save(d: &Document) -> PsdFile {
    let r = export(d, "x.psd", &ExportOptions::default()).expect("export");
    PsdFile::from_bytes(&r.bytes).expect("reparse")
}

fn content<'a>(d: &'a Document, name: &str) -> &'a LayerContent {
    &d.layers.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("layer {name}")).content
}

fn record<'a>(f: &'a PsdFile, name: &str) -> &'a LayerRecord {
    f.layers().iter().find(|r| r.name() == name).unwrap_or_else(|| panic!("record {name}"))
}

#[test]
fn unknown_adjustment_key_opens_as_unsupported_and_saves_verbatim() {
    // `hue ` is the Photoshop 4 Hue/Saturation block (in the spec, never mapped); `zNew` stands
    // for an adjustment type newer than PhotoCraft.
    for key in [*b"hue ", *b"zNew"] {
        let data = vec![0, 2, 1, 2, 3, 4, 5, 6];
        let mut f = testgen::small(Version::Psd, Compression::Raw);
        f.layers_mut().push(adjustment_record("Mystery", vec![TaggedBlock::new(key, data.clone())]));
        let (d, warnings) = open(&f);
        let k = String::from_utf8_lossy(&key).into_owned();
        assert!(
            matches!(content(&d, "Mystery"), LayerContent::Adjustment(Adjustment::Unsupported { psd_key, raw }) if *psd_key == k && **raw == data[..]),
            "{k}: {:?}",
            content(&d, "Mystery")
        );
        let w: Vec<&String> = warnings.iter().filter(|w| w.contains("Mystery")).collect();
        assert_eq!(w.len(), 1, "{k}: {warnings:?}");
        assert!(w[0].contains(&format!("\"{k}\"")) && w[0].contains("no effect"), "{k}: {}", w[0]);

        // Saved back: the block exactly once, byte-identical, still an adjustment record.
        let g = save(&d);
        let r = record(&g, "Mystery");
        let same: Vec<&TaggedBlock> = r.blocks.iter().filter(|b| b.key == key).collect();
        assert_eq!(same.len(), 1, "{k}: {:?}", r.blocks.iter().map(TaggedBlock::key_str).collect::<Vec<_>>());
        assert_eq!(same[0].data, data, "{k}");
        assert!(r.flags.bit4_useful() && r.flags.pixel_data_irrelevant(), "{k}");
        assert!(r.rect.is_empty(), "{k}");
        // And it reopens the same way.
        let (d2, _) = open(&g);
        assert_eq!(content(&d2, "Mystery"), content(&d, "Mystery"), "{k}");
    }
}

#[test]
fn pixel_layers_with_unknown_blocks_stay_pixel_layers() {
    let extra = || TaggedBlock::new(*b"zNew", vec![9, 8, 7, 6]);
    let mut f = testgen::small(Version::Psd, Compression::Raw);
    // A normal pixel layer (pixels, flag clear) carrying an unknown block.
    f.layers_mut()[0].blocks.push(extra());
    // Flagged but with pixels: never dropped for an adjustment.
    let plane = |id: i16| ChannelData::new(id, Some(Compression::Raw), vec![200; 4]);
    let mut painted = adjustment_record("painted", vec![extra()]);
    painted.rect = Rect::from_xywh(0, 0, 2, 2);
    painted.channels = (-1..3).map(plane).collect();
    f.layers_mut().push(painted);
    // No pixels and an unknown block, but not marked "pixel data irrelevant": an empty pixel layer.
    let mut unflagged = adjustment_record("unflagged", vec![extra()]);
    unflagged.flags = LayerFlags::default();
    f.layers_mut().push(unflagged);
    // Two unknown blocks: which one is the adjustment is a guess, so nothing is guessed.
    f.layers_mut().push(adjustment_record("ambiguous", vec![extra(), TaggedBlock::new(*b"zOth", vec![1, 1, 1, 1])]));
    let (d, warnings) = open(&f);
    for name in ["a", "painted", "unflagged", "ambiguous"] {
        assert!(matches!(content(&d, name), LayerContent::Raster(_)), "{name}: {:?}", content(&d, name));
        assert!(!warnings.iter().any(|w| w.contains(&format!("\"{name}\""))), "{name}: {warnings:?}");
    }
    let LayerContent::Raster(s) = content(&d, "painted") else { unreachable!() };
    assert!(!s.content_bounds().is_empty(), "painted keeps its pixels");
    // The unknown block is still kept verbatim on save, once.
    let g = save(&d);
    for name in ["a", "painted", "unflagged", "ambiguous"] {
        let r = record(&g, name);
        let kept: Vec<&TaggedBlock> = r.blocks.iter().filter(|b| &b.key == b"zNew").collect();
        assert_eq!(kept.len(), 1, "{name}");
        assert_eq!(kept[0].data, vec![9, 8, 7, 6], "{name}");
    }
}
