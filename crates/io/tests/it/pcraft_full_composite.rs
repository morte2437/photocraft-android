//! With [`ExportOptions::pcraft_full_composite`] on, a `.pcraft` save writes the flattened
//! document at its canvas size as `composite/full.png`, next to the 256 px thumbnail and the
//! 1024 px preview, so other apps can read the whole image without decoding tiles. Off (the
//! default), the save writes the bundle it wrote before the option existed.

use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_format::zip::ZipReader;
use photocraft_geom::{Rect, Size};
use photocraft_io::{ExportOptions, export};
use photocraft_raster::Surface;

/// A background, a soft Multiply layer at 60 % and a layer that leaves part of the canvas clear.
fn doc(w: u32, h: u32, depth: SampleType) -> Document {
    let mut d = Document::new("full", Size::new(w, h), ColorMode::Rgb, depth);
    let fmt = d.pixel_format();
    let layer = |name: &str, seed: usize, r: Rect| {
        let n = r.width() as usize * r.height() as usize * fmt.bytes_per_pixel();
        let px: Vec<u8> = (0..n).map(|i| ((i * 31 + seed * 17 + i / 7) % 251) as u8).collect();
        Layer::new(name, LayerContent::Raster(Surface::from_interleaved(fmt, r, &px)))
    };
    d.layers.push(layer("Back", 1, Rect::new(0, 0, w as i32, h as i32 / 2)));
    let mut soft = layer("Soft", 2, Rect::new(w as i32 / 4, h as i32 / 4, w as i32, h as i32));
    soft.opacity = 0.6;
    soft.blend = BlendMode::Multiply;
    d.layers.push(soft);
    d
}

fn on() -> ExportOptions {
    ExportOptions { pcraft_full_composite: true, ..Default::default() }
}

/// The bundle's member named by the manifest's `full_composite`, decoded: size and RGBA8 pixels.
fn full_composite(bytes: &[u8]) -> ((u32, u32), Vec<u8>) {
    let png = photocraft_format::read_full_composite(bytes).expect("manifest").expect("the manifest names the full-size composite");
    assert_eq!(png, ZipReader::new(bytes).expect("zip").read_by_name("composite/full.png", usize::MAX).expect("entry"));
    let image = photocraft_codecs::decode(&png).expect("png");
    (image.dimensions(), image.to_rgba8())
}

fn flattened(doc: &Document) -> Vec<u8> {
    photocraft_compose::render(doc, doc.bounds()).to_rgba8().pixels
}

#[test]
fn a_default_save_writes_the_bundle_without_the_full_composite() {
    for depth in SampleType::ALL {
        let d = doc(1500, 700, depth);
        let bytes = export(&d, "a.pcraft", &ExportOptions::default()).expect("export").bytes;
        assert_eq!(photocraft_format::read_full_composite(&bytes).expect("manifest"), None, "{depth:?}");
        assert!(ZipReader::new(&bytes).expect("zip").find("composite/full.png").is_none(), "{depth:?}");
        // The bytes a save wrote before the option: the bundle with its two previews alone.
        let mut named = d.clone();
        named.name = "a.pcraft".into();
        let previews = photocraft_format::SaveOptions {
            thumbnail: Some(photocraft_compose::thumbnail(&named, 256)),
            composite: Some(photocraft_compose::thumbnail(&named, 1024)),
            ..Default::default()
        };
        assert!(bytes == photocraft_format::save_to_bytes(&named, &previews).expect("save"), "{depth:?}: the default save changed");
    }
}

#[test]
fn a_pcraft_save_writes_the_flattened_document_at_its_canvas_size() {
    for depth in SampleType::ALL {
        let d = doc(300, 200, depth);
        let bytes = export(&d, "a.pcraft", &on()).expect("export").bytes;
        let (size, pixels) = full_composite(&bytes);
        assert_eq!(size, (300, 200), "{depth:?}");
        assert!(pixels == flattened(&d), "{depth:?}: the full-size composite differs from the flattened document");
        assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] == 0), "{depth:?}: transparent pixels stay transparent");
        let mut saved = d.clone();
        saved.name = "a.pcraft".into();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).expect("load"), saved, "{depth:?}");
    }
}

#[test]
fn a_document_larger_than_the_preview_keeps_its_full_size() {
    let d = doc(1500, 700, SampleType::U8);
    let bytes = export(&d, "a.pcraft", &on()).expect("export").bytes;
    let (size, pixels) = full_composite(&bytes);
    assert_eq!(size, (1500, 700));
    assert!(pixels == flattened(&d), "the full-size composite differs from the flattened document");
    let preview = ZipReader::new(&bytes).expect("zip").read_by_name("composite/preview.png", usize::MAX).expect("preview");
    assert_eq!(photocraft_codecs::decode(&preview).expect("png").dimensions(), (1024, 478));
}

#[test]
fn the_full_composite_carries_the_profile_of_its_pixels() {
    use craft_cms::{Builtin, ColorSpace, Profile};
    let full_icc = |d: &Document| {
        let bytes = export(d, "a.pcraft", &on()).expect("export").bytes;
        let png = ZipReader::new(&bytes).expect("zip").read_by_name("composite/full.png", usize::MAX).expect("entry");
        photocraft_codecs::decode(&png).expect("png").icc.expect("an embedded profile")
    };
    let srgb = Builtin::Srgb.profile().to_bytes().to_vec();
    let p3 = Builtin::DisplayP3.profile().to_bytes().to_vec();
    let mut d = doc(40, 30, SampleType::U8);
    d.icc_profile = Some(std::sync::Arc::new(p3.clone()));
    assert_eq!(full_icc(&d), p3, "the composite is in the document's RGB space");
    d.icc_profile = None;
    assert_eq!(full_icc(&d), srgb);
    // A CMYK document composites to sRGB: its CMYK profile does not describe the PNG.
    let mut cmyk = Document::new("cmyk", Size::new(40, 30), ColorMode::Cmyk, SampleType::U8);
    cmyk.icc_profile = Some(std::sync::Arc::new(Builtin::CoatedCmyk.profile().to_bytes().to_vec()));
    assert_eq!(full_icc(&cmyk), srgb);
    // A gray document's composite repeats its gray samples in R, G and B: the PNG carries its
    // gray curve as an RGB profile.
    let gray = Builtin::SGray.profile().clone();
    let mut g = Document::new("gray", Size::new(40, 30), ColorMode::Grayscale, SampleType::U8);
    g.icc_profile = Some(std::sync::Arc::new(gray.to_bytes().to_vec()));
    let tagged = Profile::parse(&full_icc(&g)).expect("profile");
    assert_eq!(tagged.color_space, ColorSpace::Rgb);
    assert_eq!(tagged.to_bytes(), gray.gray_as_rgb().expect("gray as RGB").to_bytes());
}

#[test]
fn a_canvas_past_the_render_limit_is_saved_without_it() {
    // 40000 x 30000 px is over the compositor's 2^30-pixel limit; the layers stay small.
    let mut d = doc(64, 64, SampleType::U8);
    d.size = Size::new(40_000, 30_000);
    assert!(photocraft_io::pcraft_full_composite(&d).is_none());
    let bytes = export(&d, "a.pcraft", &on()).expect("export").bytes;
    assert_eq!(photocraft_format::read_full_composite(&bytes).expect("manifest"), None);
    assert!(ZipReader::new(&bytes).expect("zip").find("composite/full.png").is_none());
}

#[test]
fn a_sparse_canvas_past_the_composite_memory_budget_is_saved_without_it() {
    // The layer data stays tiny; an 8193 x 8192 RGBA8 composite exceeds 256 MiB.
    let mut d = doc(64, 64, SampleType::U8);
    d.size = Size::new(8193, 8192);
    assert!(photocraft_io::pcraft_full_composite(&d).is_none());
    let bytes = export(&d, "a.pcraft", &on()).expect("export").bytes;
    assert_eq!(photocraft_format::read_full_composite(&bytes).expect("manifest"), None);
    assert!(ZipReader::new(&bytes).expect("zip").find("composite/full.png").is_none());
}
