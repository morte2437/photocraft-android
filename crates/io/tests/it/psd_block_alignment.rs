//! Every layer-level tagged block of an exported PSD/PSB is a multiple of 4 bytes long.
//!
//! Photoshop pads layer-level blocks to 4 inside the length field; all of its own files do (567
//! fixtures checked), and a regenerated block that stopped at an even but unaligned size left the
//! layer record's extra data unaligned: a Pattern Fill's `PtFl` came out 2 bytes short of the
//! boundary for half of all pattern names.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Adjustment, Document, Fill, Layer, LayerContent, Pattern};
use photocraft_geom::{Rect, Size};
use photocraft_io::*;
use photocraft_psd::PsdFile;
use photocraft_raster::Surface;

fn pattern(name: &str) -> Pattern {
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.fill_rect(Rect::new(0, 0, 8, 8), &[0.8, 0.8, 0.8, 1.0]);
    s.fill_rect(Rect::new(0, 0, 4, 4), &[0.2, 0.4, 0.9, 1.0]);
    Pattern { id: "059289fb-0000-4000-8000-000000000001".into(), name: name.into(), width: 8, height: 8, surface: s }
}

/// One Pattern Fill layer per name; the name's length moves the descriptor's size by two bytes
/// per character, so consecutive lengths cover every residue modulo 4.
fn doc(depth: SampleType, names: &[&str]) -> Document {
    let mut d = Document::new("p", Size::new(40, 24), ColorMode::Rgb, depth);
    for name in names {
        let p = pattern(name);
        let fill = Fill::Pattern { name: (*name).into(), scale: 1.0, id: p.id.clone(), angle: 0.0, link: true, phase: (0.0, 0.0) };
        d.patterns.push(p);
        d.layers.push(Layer::new(format!("Fill {name}"), LayerContent::Fill(fill)));
    }
    d
}

#[test]
fn layer_blocks_are_four_byte_aligned() {
    let names = ["A", "Ab", "Abc", "Abcd", "Dots", "Checker", "Wood grain"];
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for path in ["x.psd", "x.psb"] {
            let bytes = export(&doc(depth, &names), path, &ExportOptions::default()).expect("export").bytes;
            let file = PsdFile::from_bytes(&bytes).expect("reparse");
            let mut fills = 0;
            for layer in file.layers() {
                for b in &layer.blocks {
                    let key = String::from_utf8_lossy(&b.key).into_owned();
                    assert_eq!(b.data.len() % 4, 0, "{depth:?} {path}: layer {:?} block {key} is {} bytes", layer.name(), b.data.len());
                }
                fills += usize::from(layer.block(b"PtFl").is_some());
            }
            assert_eq!(fills, names.len(), "{depth:?} {path}: every pattern fill keeps its PtFl block");
        }
    }
}

#[test]
fn generated_gradient_map_blocks_are_four_byte_aligned() {
    let cases = [(vec![(0.0, [0.04, 0.1, 0.18]), (1.0, [1.0; 3])], 128), (vec![(0.0, [0.04, 0.1, 0.18]), (0.5, [0.4, 0.2, 0.1]), (1.0, [1.0; 3])], 148)];
    for (stops, expected_length) in cases {
        let mut doc = Document::new("gradient", Size::new(16, 16), ColorMode::Rgb, SampleType::U8);
        doc.layers.push(Layer::new("Gradient Map", LayerContent::Adjustment(Adjustment::GradientMap { stops, reverse: false, dither: false })));
        let bytes = export(&doc, "gradient.psd", &ExportOptions::default()).expect("export").bytes;
        let file = PsdFile::from_bytes(&bytes).expect("reparse");
        let block = file.layers().iter().find_map(|layer| layer.block(b"grdm")).expect("Gradient Map block");
        assert_eq!(block.data.len(), expected_length, "grdm declared length must include padding");
    }
}

/// A file saved by an older build carries the 2-mod-4 `grdm` Photoshop refuses; saving it again,
/// with the stops unchanged or only Reverse toggled, must write an aligned block (#8070).
#[test]
fn legacy_unaligned_gradient_map_is_repaired_on_resave() {
    let stops = vec![(0.0, [0.04, 0.1, 0.18]), (0.5, [0.4, 0.2, 0.1]), (1.0, [1.0; 3])];
    for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
        let mut doc = Document::new("gradient", Size::new(16, 16), mode, SampleType::U8);
        doc.layers.push(Layer::new("Gradient Map", LayerContent::Adjustment(Adjustment::GradientMap { stops: stops.clone(), reverse: false, dither: false })));
        let mut file = PsdFile::from_bytes(&export(&doc, "gradient.psd", &ExportOptions::default()).expect("export").bytes).expect("reparse");
        let legacy = file.layers_mut().iter_mut().find_map(|layer| layer.block_mut(b"grdm")).expect("Gradient Map block");
        let unpadded = legacy.data.len() - 2;
        legacy.data.truncate(unpadded);
        let legacy = file.to_bytes().expect("legacy file");
        for reverse in [false, true] {
            let mut reopened = import("gradient.psd", &legacy).expect("import").document;
            let Some(LayerContent::Adjustment(Adjustment::GradientMap { reverse: r, .. })) = reopened.layers.iter_mut().map(|l| &mut l.content).last() else {
                panic!("no Gradient Map layer")
            };
            *r = reverse;
            let expected = reopened.layers.last().map(|l| l.content.clone());
            let bytes = export(&reopened, "gradient.psd", &ExportOptions::default()).expect("re-save").bytes;
            let file = PsdFile::from_bytes(&bytes).expect("reparse");
            let block = file.layers().iter().find_map(|layer| layer.block(b"grdm")).expect("Gradient Map block");
            assert_eq!(block.data.len() % 4, 0, "{mode:?} reverse {reverse}: re-saved grdm is {} bytes", block.data.len());
            let again = import("gradient.psd", &bytes).expect("reopen").document;
            assert_eq!(again.layers.last().map(|l| l.content.clone()), expected, "{mode:?} reverse {reverse}: Gradient Map changed on re-save");
        }
    }
}
