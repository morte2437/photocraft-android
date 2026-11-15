//! PDF import end-to-end through craft-pdf (pure Rust, no shared library).
//!
//! A six-page synthetic PDF (plain, CropBox offset, rotations 90/180/270 and an oversized
//! CropBox) imports at 72 and 300 DPI as one artboard per page, with the page's print size kept,
//! a blue rectangle sampled from the raster, and the renderer's side/pixel limits respected by
//! lowering the effective DPI on a huge page.

use std::sync::atomic::{AtomicBool, Ordering};

use lopdf::{Document as PdfDocument, Object, Stream, dictionary};
use photocraft_doc::LayerContent;
use photocraft_geom::Rect;
use photocraft_io::{ImportOptions, IoError, import_with_options_and_interrupt};
use photocraft_raster::Interrupt;

const PAGE_W: f64 = 200.0;
const PAGE_H: f64 = 100.0;

/// Six one-page variants: plain, CropBox offset to (10, 20, 210, 120), rotations of 90/180/270
/// degrees and an oversized CropBox (-50, -50, 250, 150) around the 200 x 100 MediaBox. Each page
/// draws a 40 x 40 blue rectangle at (20, 30) in PDF space.
fn variant_pdf() -> Vec<u8> {
    let pages: [(Option<i64>, Option<[f32; 4]>); 6] = [
        (None, None),
        (None, Some([10.0, 20.0, 210.0, 120.0])),
        (Some(90), None),
        (Some(180), None),
        (Some(270), None),
        (None, Some([-50.0, -50.0, 250.0, 150.0])),
    ];
    let mut pdf = PdfDocument::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let mut kids = Vec::new();
    for (rotate, crop) in pages {
        let content = b"0 0 1 rg 20 30 40 40 re f";
        let content_id = pdf.add_object(Stream::new(dictionary! {}, content.to_vec()));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => Object::Array(vec![Object::Real(0.0), Object::Real(0.0), Object::Real(PAGE_W as f32), Object::Real(PAGE_H as f32)]),
            "Contents" => content_id,
            "Resources" => dictionary! {},
        };
        if let Some(rotate) = rotate {
            page.set("Rotate", rotate);
        }
        if let Some([left, bottom, right, top]) = crop {
            page.set("CropBox", Object::Array(vec![Object::Real(left), Object::Real(bottom), Object::Real(right), Object::Real(top)]));
        }
        kids.push(Object::Reference(pdf.add_object(page)));
    }
    let count = kids.len() as i64;
    pdf.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => Object::Array(kids),
            "Count" => count,
        }),
    );
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).expect("writing the variant PDF");
    bytes
}

/// One page of `width` x `height` points with the same blue rectangle.
fn one_page_pdf(width: f32, height: f32) -> Vec<u8> {
    let mut pdf = PdfDocument::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let content_id = pdf.add_object(Stream::new(dictionary! {}, b"0 0 1 rg 20 30 40 40 re f".to_vec()));
    let page_id = pdf.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => Object::Array(vec![Object::Real(0.0), Object::Real(0.0), Object::Real(width), Object::Real(height)]),
        "Contents" => content_id,
        "Resources" => dictionary! {},
    });
    pdf.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => Object::Array(vec![Object::Reference(page_id)]),
            "Count" => 1,
        }),
    );
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).expect("writing the page PDF");
    bytes
}

/// The first artboard rectangle of `document`.
fn first_artboard(document: &photocraft_doc::Document) -> Rect {
    let layer = document.layers.first().expect("an imported page group");
    let group = match &layer.content {
        LayerContent::Group(group) => group,
        _ => panic!("{} is not a group", layer.name),
    };
    group.artboard.as_ref().expect("page artboard").rect
}

/// Each page becomes its own artboard at 72 and 300 DPI, with the print size preserved.
#[test]
fn imports_every_page_as_an_artboard_with_its_print_size() {
    let pdf = variant_pdf();
    for dpi in [72.0f32, 300.0] {
        let options = ImportOptions { pdf_dpi: dpi, ..Default::default() };
        let result = import_with_options_and_interrupt("variants.pdf", &pdf, &options, &Interrupt::NONE).expect("PDF import");
        assert_eq!(result.document.layers.len(), 6, "one artboard per page at {dpi} dpi");
        let mut top_left = 0i32;
        for (index, layer) in result.document.layers.iter().enumerate() {
            let group = match &layer.content {
                LayerContent::Group(group) => group,
                _ => panic!("{} is not a page group", layer.name),
            };
            let artboard = group.artboard.as_ref().expect("page artboard").rect;
            let expected = match index {
                // CropBox [10, 20, 210, 120] intersected with the MediaBox is 190 x 80 points.
                1 => (190.0, 80.0),
                // Rotations 90 and 270 swap the axes.
                2 | 4 => (100.0, 200.0),
                _ => (200.0, 100.0),
            };
            let print_width = f64::from(artboard.width()) * 72.0 / f64::from(dpi);
            let print_height = f64::from(artboard.height()) * 72.0 / f64::from(dpi);
            assert!((print_width - expected.0).abs() < 1.0, "page {} width is {print_width:.1} pt, expected {} at {dpi} dpi", index + 1, expected.0);
            assert!((print_height - expected.1).abs() < 1.0, "page {} height is {print_height:.1} pt, expected {} at {dpi} dpi", index + 1, expected.1);
            assert_eq!(artboard.x0, top_left, "pages lay out side by side from x=0");
            top_left = artboard.x1 + 80;
            let surface = group.children.first().and_then(|layer| layer.surface()).expect("page raster layer");
            if index == 0 {
                // The rectangle's centre in PDF space is (40, 50); at the document's top-left
                // origin that is y = PAGE_H - 50.
                let scale = f64::from(dpi) / 72.0;
                let x = artboard.x0 + (40.0 * scale).round() as i32;
                let y = ((PAGE_H - 50.0) * scale).round() as i32;
                let mut pixel = [[0u8; 4]; 1];
                surface.read_rgba8_into(Rect::from_xywh(x, y, 1, 1), &mut pixel);
                assert!(pixel[0][2] > 120 && pixel[0][0] < 120, "page 1 should be blue inside the rectangle, got {:?}", pixel[0]);
                let mut background = [[0u8; 4]; 1];
                surface.read_rgba8_into(Rect::from_xywh(artboard.x0 + 2, 2, 1, 1), &mut background);
                assert!(background[0][0] > 240 && background[0][1] > 240 && background[0][2] > 240, "page background should be white, got {:?}", background[0]);
            }
        }
    }
}

/// A page over the renderer's side limit lowers the effective DPI instead of failing.
#[test]
fn huge_pages_lower_the_effective_dpi_within_the_renderer_bounds() {
    // 3000 pt at 300 dpi would be 12500 px wide, over craft-pdf's 8192 px side limit.
    let pdf = one_page_pdf(3000.0, 100.0);
    let options = ImportOptions { pdf_dpi: 300.0, ..Default::default() };
    let result = import_with_options_and_interrupt("wide.pdf", &pdf, &options, &Interrupt::NONE).expect("PDF import");
    assert!(result.warnings.iter().any(|warning| warning.contains("rendered at")), "warnings: {:?}", result.warnings);
    let artboard = first_artboard(&result.document);
    assert!(artboard.width() <= 8192, "width {} is over the renderer's side limit", artboard.width());
    assert!(result.document.resolution_dpi < 300.0, "the effective DPI should be lowered");
    let print_width = f64::from(artboard.width()) * 72.0 / f64::from(result.document.resolution_dpi);
    assert!((print_width - 3000.0).abs() < 1.0, "print width {print_width:.1} pt should stay 3000 pt");
}

/// Cancelling at the public boundary or between pages returns Cancelled, never a document.
///
/// The renderer's watcher polls the interrupt while a page is being interpreted, so the number
/// of checks is not fixed any more; these triggers do not depend on counting them.
#[test]
fn cancelling_before_and_between_pages_is_reported() {
    let pdf = variant_pdf();
    let options = ImportOptions { pdf_dpi: 72.0, ..Default::default() };

    // The boundary check before dispatch: already cancelled.
    let always = || true;
    let ctl = Interrupt::cancel_only(&always);
    match import_with_options_and_interrupt("variants.pdf", &pdf, &options, &ctl) {
        Err(IoError::Cancelled) => {}
        other => panic!("an already-cancelled import returned {:?}", other.map(|_| ())),
    }

    // The per-page loop's check: the first page's progress cancels the next iteration.
    let cancelled = AtomicBool::new(false);
    let cancel = || cancelled.load(Ordering::SeqCst);
    let progress = |fraction: f32| {
        if fraction > 0.0 {
            cancelled.store(true, Ordering::SeqCst);
        }
    };
    let ctl = Interrupt::new(&cancel, &progress);
    match import_with_options_and_interrupt("variants.pdf", &pdf, &options, &ctl) {
        Err(IoError::Cancelled) => {}
        other => panic!("cancelling after the first page returned {:?}", other.map(|_| ())),
    }
}
