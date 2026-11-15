//! PDF export model mapping for PhotoCraft.
//!
//! This module projects document rasters, vectors, spot channels and print policy into the
//! neutral page records in craft-pdf. PDF object layout and serialization live in craft-pdf.

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::Document;

use crate::{ExportResult, IoError};

mod shapes;
mod text;

/// Neutral page records and PDF syntax are owned by craft-pdf; this module only maps the
/// PhotoCraft document to those records and supplies its product-level cancellation policy.
pub use craft_pdf::write_raster::{ContentOp, Image as PrintImage, Marks, Page as PrintPage, SelectionFont, SpotPlate, VectorContent};

/// A one-page PDF.
pub fn print_pdf(page: &PrintPage) -> Vec<u8> {
    craft_pdf::write_raster::write_pages_with_vector_content(std::slice::from_ref(page), &[VectorContent::default()], &SelectionFont::default(), None)
        .unwrap_or_default()
}

fn print_pdf_pages(
    pages: &[PrintPage],
    vectors: &[VectorContent],
    selection_font: &SelectionFont,
    xmp: Option<&str>,
    ctl: &photocraft_raster::Interrupt,
) -> std::result::Result<Vec<u8>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    craft_pdf::write_raster::write_pages_with_vector_content(pages, vectors, selection_font, xmp)
        .ok_or_else(|| IoError::Unsupported("PDF vector content contains an invalid number".into()))
}

/// The namespace of the spot-plate description written by the shared PDF metadata encoder.
pub const SPOT_PLATES_NS: &str = craft_pdf::write_raster::SPOT_PLATES_NAMESPACE;

/// The channels of `doc` that leave as plates, in channel order (the first prints first,
/// underneath): every channel of a Multichannel document, the spot channels of any other.
fn plate_channels(doc: &Document) -> impl Iterator<Item = &photocraft_doc::AlphaChannel> {
    let multichannel = doc.mode == ColorMode::Multichannel;
    doc.channels.iter().filter(move |ch| multichannel || ch.spot.is_some())
}

/// The spot plates of `doc` over `rect` (the canvas, or one artboard's rect), in channel order.
///
/// In a Multichannel document every channel is a plate: its spot channels at the ink density
/// they store (1 = solid), an alpha-type channel as the transparent black ink the composite shows
/// it as (1 − value, as `photocraft_compose::multichannel` prints it). Any other document keeps
/// its alpha channels as masks and exports only its spot channels. The alternate colour is the
/// ink colour separated through the CMYK profile in use (`Color::in_mode`); it only previews the
/// plate, as Photoshop's does. Solidity is a preview attribute too: it is not part of the plate
/// image or its tint transform, and only the shared XMP description writer carries it.
fn plates_of(doc: &Document, rect: photocraft_geom::Rect) -> Vec<SpotPlate> {
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    plate_channels(doc)
        .map(|ch| {
            let (ink, alpha_type) = match ch.spot {
                Some((ink, _)) => (ink, false),
                None => (photocraft_color::Color::BLACK, true),
            };
            let n = ch.surface.channels().max(1);
            let values = ch.surface.read_region(rect);
            let data: Vec<u8> = values.chunks(n).map(|px| px.first().copied().unwrap_or(0.0)).map(|v| to8(if alpha_type { 1.0 - v } else { v })).collect();
            let srgb = ink.to_rgb();
            SpotPlate {
                name: ch.name.clone(),
                cmyk: ink.in_mode(ColorMode::Cmyk).c,
                lab: photocraft_color::convert::srgb_to_lab(srgb),
                srgb,
                solidity: ch.spot.map_or(0.0, |(_, s)| s.clamp(0.0, 1.0)),
                width: rect.width(),
                height: rect.height(),
                data,
            }
        })
        .collect()
}

/// The vector operators for the contiguous visible top prefix of `doc`, top to bottom, with the
/// indices of the layers they replace, and the selection font covering their text (plus, when a
/// lower candidate is rejected during emission, that layer's unused characters).
/// Hidden and blocked layers contribute no characters, so they cannot exhaust the font's codes.
fn collect_vectors(doc: &Document) -> (Vec<(usize, Vec<ContentOp>)>, SelectionFont) {
    enum Candidate<'a> {
        Ops(Vec<ContentOp>),
        Text(text::PreparedText<'a>),
    }
    let mut candidates = Vec::new();
    if doc.icc_profile.is_none() {
        let mut repertoire = std::collections::BTreeSet::new();
        for (index, layer) in doc.layers.iter().enumerate().rev() {
            if !layer.visible {
                continue;
            }
            if layer.blend != photocraft_color::BlendMode::Normal
                || layer.opacity != 1.0
                || layer.fill_opacity != 1.0
                || layer.clipped
                || layer.mask.is_some()
                || layer.vector_mask.is_some()
                || (layer.effects.enabled && !layer.effects.items.is_empty())
                || layer.excluded_channels != 0
                || layer.blend_if != Default::default()
                || layer.advanced != Default::default()
                || layer.video.is_some()
            {
                break;
            }
            let candidate = match &layer.content {
                photocraft_doc::LayerContent::Shape(shape) => match shapes::shape_content(shape, doc.mode) {
                    Some(path) => Candidate::Ops(vec![ContentOp::Path(path)]),
                    None => break,
                },
                photocraft_doc::LayerContent::Text(text_layer) => {
                    let Some(prepared) = text::prepare(text_layer, doc.mode, doc.resolution_dpi) else { break };
                    for ch in text_layer.text.chars() {
                        repertoire.insert(ch);
                        if repertoire.len() > 255 {
                            break;
                        }
                    }
                    if repertoire.len() > 255 {
                        break;
                    }
                    Candidate::Text(prepared)
                }
                _ => break,
            };
            candidates.push((index, candidate));
        }
    }
    let selection_font = SelectionFont::new(candidates.iter().filter_map(|(_, candidate)| match candidate {
        Candidate::Text(prepared) => Some(prepared.text()),
        Candidate::Ops(_) => None,
    }));
    let mut layers = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates {
        let ops = match candidate {
            Candidate::Ops(ops) => ops,
            Candidate::Text(prepared) => match text::ops(&prepared, doc.mode, &selection_font) {
                Some(ops) => ops,
                // Keep the successful upper vectors; this layer and everything below stay raster.
                None => break,
            },
        };
        layers.push((index, ops));
    }
    (layers, selection_font)
}

/// Pixels in `rect`, saturating on overflow.
fn rect_pixels(rect: photocraft_geom::Rect) -> u64 {
    u64::from(rect.width()).saturating_mul(u64::from(rect.height()))
}

/// File › Save As / Save a Copy › PDF (#3401): the flattened document at its print size
/// (pixels ÷ resolution × 72 pt). One page per artboard when the document has artboards, else one
/// page for the whole document. Layers and transparency are dropped — the warning says so — and
/// the embedded profile travels with the images.
///
/// On a whole-document page the topmost simple shape and text layers stay vectors over the
/// flattened rest, with selectable text ([`collect_vectors`]); artboard pages are raster.
///
/// With `spot_plates`, the document's spot channels follow each page's image and vectors as
/// overprinting `/Separation` plates (FILE-215-17), an artboard page carrying its own region of
/// each, and a Multichannel document, which is nothing but its channels, is written as plates
/// alone. Without it a Multichannel document has nothing to write and is refused.
pub fn export(doc: &Document, spot_plates: bool, ctl: &photocraft_raster::Interrupt) -> std::result::Result<ExportResult, IoError> {
    export_with(doc, spot_plates, doc.metadata.xmp.as_deref(), ctl)
}

/// [`export`] with the XMP packet to embed (`doc_xmp`: the document's, or `None` when the save
/// embeds no metadata). With spot plates the packet gains their description through the shared
/// PDF writer;
/// without plates it is written as it is, and nothing when there is none.
pub fn export_with(doc: &Document, spot_plates: bool, doc_xmp: Option<&str>, ctl: &photocraft_raster::Interrupt) -> std::result::Result<ExportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let (w, h) = (usize::try_from(doc.size.width).unwrap_or(0), usize::try_from(doc.size.height).unwrap_or(0));
    if w == 0 || h == 0 {
        return Err(IoError::Unsupported("PDF needs a document with non-zero width and height; nothing was written".into()));
    }
    // The composite's channels per pixel; None for a Multichannel document, which has no composite.
    let device = match doc.mode {
        ColorMode::Rgb => Some(3),
        ColorMode::Grayscale => Some(1),
        ColorMode::Cmyk => Some(4),
        ColorMode::Multichannel if spot_plates && !doc.channels.is_empty() => None,
        ColorMode::Multichannel => {
            return Err(IoError::Unsupported(
                "a Multichannel PDF is its channels as spot plates; with no channels, or spot plates left out, there is nothing to write".into(),
            ));
        }
        m => {
            return Err(IoError::Unsupported(format!(
                "PDF saves RGB, Grayscale, CMYK and Multichannel documents; {m:?} needs converting to RGB first (Image > Mode)"
            )));
        }
    };
    let fmt = doc.pixel_format();
    let n = fmt.channels().max(1);
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    let paper = if doc.mode == ColorMode::Cmyk { 0.0 } else { 1.0 };
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let icc = doc.icc_profile.as_ref().map(|p| p.to_vec());
    let plate_count = if spot_plates { plate_channels(doc).count() } else { 0 };
    // Straight alpha, composited here over paper: white in RGB/Gray (1 = full light), zero ink
    // in CMYK (0 = paper).
    let image_of = |values: &[f32], rect: photocraft_geom::Rect, device: usize| {
        let mut data = Vec::with_capacity((rect.width() as usize * rect.height() as usize).saturating_mul(device));
        for px in values.chunks(n) {
            let a = px.last().copied().unwrap_or(1.0).clamp(0.0, 1.0);
            for v in px.iter().take(device) {
                data.push(to8(v * a + paper * (1.0 - a)));
            }
        }
        PrintImage { width: rect.width(), height: rect.height(), channels: device, data, icc: icc.clone() }
    };
    let page_of = |image: Option<PrintImage>, rect: photocraft_geom::Rect, page_label: Option<String>| {
        let (pw, ph) = (f64::from(rect.width()) * 72.0 / dpi, f64::from(rect.height()) * 72.0 / dpi);
        let plates = if spot_plates { plates_of(doc, rect) } else { Vec::new() };
        PrintPage { paper: (pw, ph), image, rect: (0.0, 0.0, pw, ph), marks: Marks::default(), description: None, label: None, page_label, plates }
    };
    let mut warnings = Vec::new();
    if device.is_some() {
        warnings.push("PDF was saved flattened: layers, masks-as-editable and transparency are not preserved (#3401)".into());
    }
    let artboards = doc.artboards();
    let (pages, vectors, selection_font) = if !artboards.is_empty() {
        // Every page's encoded pixels are kept until the PDF is assembled; bound that peak.
        const MAX_PDF_EXPORT_BYTES: u64 = 1 << 30;
        let page_pixels: u64 = artboards.iter().fold(0u64, |bytes, (_, _, a)| bytes.saturating_add(rect_pixels(a.rect)));
        // A page keeps `device` bytes a pixel for its composite and one a pixel for each plate.
        let retained = page_pixels.saturating_mul((device.unwrap_or(0) + plate_count) as u64);
        // One page renders at a time: its f32 composite surface plus the region read back from
        // it, both `n` channels at 4 bytes per pixel. The whole-document canvas is never held.
        let largest = artboards.iter().map(|(_, _, a)| rect_pixels(a.rect)).max().unwrap_or(0);
        let temporary = largest.saturating_mul(n as u64).saturating_mul(4).saturating_mul(2);
        let estimate = retained.saturating_add(temporary);
        if estimate > MAX_PDF_EXPORT_BYTES {
            return Err(IoError::Unsupported(format!(
                "PDF export would need about {} MB for {} artboards; export them in smaller batches",
                estimate >> 20,
                artboards.len()
            )));
        }
        // One document shell is reused for every page. Each page renders a document holding only
        // its artboard group (the same isolation as `artboard_document`), so an overlapping board
        // cannot paint on another board's page. Layer pixels are shared with `doc`; this one
        // clone and the per-page group clone hold no new pixel buffers, but they are outside the
        // 1 GiB estimate because they are not retained page data.
        let mut one = doc.clone();
        let mut pages = Vec::with_capacity(artboards.len());
        for (id, name, artboard) in &artboards {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let rect = artboard.rect;
            if rect.width() == 0 || rect.height() == 0 {
                continue;
            }
            // A hidden artboard still gets its page, rendered from its own content rather than
            // left blank by the group's visibility.
            let Some(layer) = doc.layer(*id) else { continue };
            let image = device.map(|device| {
                let mut group = layer.clone();
                group.visible = true;
                one.layers.clear();
                one.layers.push(group);
                // Render only this artboard's rect; the surface never holds the whole document.
                let surface = photocraft_compose::composite_to_surface(&one, rect, PixelFormat::new(fmt.mode, fmt.sample, true), None);
                image_of(&surface.read_region(rect), rect, device)
            });
            pages.push(page_of(image, rect, Some((*name).to_string())));
        }
        if pages.is_empty() {
            return Err(IoError::Unsupported("PDF: the document's artboards have no area".into()));
        }
        warnings.push(format!("PDF exported {} artboards as {} pages; vector/text layers on artboards are rasterised", artboards.len(), pages.len()));
        let vectors = vec![VectorContent::default(); pages.len()];
        (pages, vectors, SelectionFont::default())
    } else {
        let canvas = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
        // Simple topmost shapes and text layers can be drawn over the flattened backdrop without
        // changing compositing. Stop at the first unsupported visible layer, since it may depend
        // on the backdrop (adjustments, blend modes, masks, effects, or clipping). A Multichannel
        // document has no composite for them to sit on: it is its plates alone.
        let (vector_layers, selection_font) = if device.is_some() { collect_vectors(doc) } else { (Vec::new(), SelectionFont::default()) };
        let image = device.map(|device| {
            let mut backdrop = doc.clone();
            for (index, _) in &vector_layers {
                if let Some(layer) = backdrop.layers.get_mut(*index) {
                    layer.visible = false;
                }
            }
            let surface = photocraft_compose::flatten_to_surface(&backdrop, PixelFormat::new(fmt.mode, fmt.sample, true), None);
            image_of(&surface.read_region(canvas), canvas, device)
        });
        // The vector layers, bottom to top, in the document's pixel space (y down), over the
        // image and under the plates.
        let mut operations = Vec::new();
        if !vector_layers.is_empty() {
            let scale = 72.0 / dpi;
            let ph = f64::from(doc.size.height) * scale;
            operations.push(ContentOp::Save);
            operations.push(ContentOp::Concat([scale, 0.0, 0.0, -scale, 0.0, ph]));
            for (_, ops) in vector_layers.iter().rev() {
                operations.extend(ops.iter().cloned());
            }
            operations.push(ContentOp::Restore);
        }
        (vec![page_of(image, canvas, None)], vec![VectorContent { operations }], selection_font)
    };
    // The plates are the document's: every page has the same ones, over its own region.
    let plates = pages.first().map(|p| p.plates.as_slice()).unwrap_or_default();
    if !plates.is_empty() {
        let names: Vec<&str> = plates.iter().map(|p| p.name.as_str()).collect();
        warnings.push(format!(
            "{} spot plate{} written as overprinting Separation colorants ({}), 8-bit; their preview colours are the inks separated to CMYK",
            plates.len(),
            if plates.len() == 1 { "" } else { "s" },
            names.join(", ")
        ));
    } else if doc.channels.iter().any(|ch| ch.spot.is_some()) {
        warnings.push("spot channels were not written: spot plates were left out".into());
    }
    let xmp = craft_pdf::write_raster::xmp_with_spot_plates(doc_xmp, plates);
    if !doc.metadata.text.is_empty() {
        warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
    }
    Ok(ExportResult { bytes: print_pdf_pages(&pages, &vectors, &selection_font, xmp.as_deref(), ctl)?, warnings })
}

#[cfg(test)]
mod tests;
