//! The PDF writer (ISO 32000-1 basics): one raster page per file.
//!
//! A small, self-contained writer used by two paths: File › Save a Copy / Save As › PDF (#3401)
//! through [`export`], and File › Print through [`print_pdf`] (called by the engine, which adds
//! paper geometry, printer marks and labels around the image). One page holds a
//! Flate-compressed image XObject, optional printer marks as vector operators and optional
//! Helvetica text; an embedded ICC profile rides in an ICCBased colour space. Multi-page PDF,
//! layer-preserving PDF and PDF import stay open (FILE-215-10).

use std::io::Write as _;

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::Document;

use crate::{ExportResult, IoError};

mod selection;
mod shapes;
mod text;

/// The image placed on a page: 8-bit interleaved samples, 1 (gray), 3 (RGB) or 4 (CMYK,
/// 1 = full ink), with the profile it was authored in if any.
pub struct PrintImage {
    pub width: u32,
    pub height: u32,
    pub channels: usize,
    pub data: Vec<u8>,
    pub icc: Option<Vec<u8>>,
}

/// Printer marks around the image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Marks {
    pub corner_crop: bool,
    pub center_crop: bool,
    pub registration: bool,
}

/// One page: paper (pt), image box (pt, origin bottom-left), marks and texts.
pub struct PrintPage {
    pub paper: (f64, f64),
    pub image: PrintImage,
    pub rect: (f64, f64, f64, f64),
    pub marks: Marks,
    pub description: Option<String>,
    pub label: Option<String>,
    /// The page's name in the PDF `/PageLabels` tree (Unicode-capable; not drawn on the page).
    pub page_label: Option<String>,
}

pub(super) fn pdf_string(s: &str) -> String {
    let mut o = String::from("(");
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                o.push('\\');
                o.push(c);
            }
            c if c.is_ascii() && !c.is_ascii_control() => o.push(c),
            _ => o.push('?'),
        }
    }
    o.push(')');
    o
}

/// A PDF text string with UTF-16BE hex (the Unicode-capable form, byte-order mark included).
fn pdf_unicode_string(s: &str) -> String {
    let mut o = String::from("<FEFF");
    for u in s.encode_utf16() {
        o.push_str(&format!("{u:04X}"));
    }
    o.push('>');
    o
}

fn deflate(data: &[u8]) -> Vec<u8> {
    // Fast: print PDFs are transient and large. Level 3, not 1: zlib-rs level 1 writes static
    // Huffman blocks only (13-30% bigger on photos); level 3 is the lowest that is never bigger
    // than miniz's level 1 was, at about twice its time.
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(3));
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// Vector marks around `(x, y, w, h)`.
fn marks_ops(m: Marks, (x, y, w, h): (f64, f64, f64, f64)) -> String {
    let mut s = String::from("q 0 G 0.3 w\n");
    let (gap, len) = (6.0, 18.0);
    let line = |s: &mut String, x0: f64, y0: f64, x1: f64, y1: f64| s.push_str(&format!("{x0:.2} {y0:.2} m {x1:.2} {y1:.2} l S\n"));
    if m.corner_crop {
        for (cx, cy, dx, dy) in [(x, y, -1.0, -1.0), (x + w, y, 1.0, -1.0), (x, y + h, -1.0, 1.0), (x + w, y + h, 1.0, 1.0)] {
            line(&mut s, cx + dx * gap, cy, cx + dx * (gap + len), cy);
            line(&mut s, cx, cy + dy * gap, cx, cy + dy * (gap + len));
        }
    }
    if m.center_crop {
        let (mx, my) = (x + w / 2.0, y + h / 2.0);
        line(&mut s, mx, y - gap, mx, y - gap - len);
        line(&mut s, mx, y + h + gap, mx, y + h + gap + len);
        line(&mut s, x - gap, my, x - gap - len, my);
        line(&mut s, x + w + gap, my, x + w + gap + len, my);
    }
    if m.registration {
        let r = 5.0;
        let off = gap + len / 2.0;
        for (cx, cy) in [(x + w / 2.0, y - off), (x + w / 2.0, y + h + off), (x - off, y + h / 2.0), (x + w + off, y + h / 2.0)] {
            // Circle (four Béziers) plus crosshair.
            let k = 0.5523 * r;
            s.push_str(&format!(
                "{:.2} {cy:.2} m {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c S\n",
                cx + r, cx + r, cy + k, cx + k, cy + r, cy + r, cx - k, cy + r, cx - r, cy + k, cx - r, cx - r, cy - k, cx - k, cy - r, cy - r, cx + k, cy - r, cx + r, cy - k, cx + r
            ));
            line(&mut s, cx - r * 1.6, cy, cx + r * 1.6, cy);
            line(&mut s, cx, cy - r * 1.6, cx, cy + r * 1.6);
        }
    }
    s.push_str("Q\n");
    s
}

/// A one-page PDF.
pub fn print_pdf(page: &PrintPage) -> Vec<u8> {
    print_pdf_pages(std::slice::from_ref(page), &[String::new()], &selection::SelectionFont::default(), &photocraft_raster::Interrupt::NONE).unwrap_or_default()
}

/// A PDF with one page per entry; `vectors[i]` (when present) is drawn over page `i`.
fn print_pdf_pages(
    pages: &[PrintPage],
    vectors: &[String],
    selection_font: &selection::SelectionFont,
    ctl: &photocraft_raster::Interrupt,
) -> std::result::Result<Vec<u8>, IoError> {
    if ctl.check().is_err() {
        return Err(IoError::Cancelled);
    }
    let Some(first) = pages.first() else { return Ok(Vec::new()) };
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    let n = pages.len();
    // A label tree is written only for a non-empty name: an all-empty tree would be `/Nums []`,
    // and filtering the first empty name away would start the tree at page index 1, not 0.
    let labels_on = pages.iter().any(|p| p.page_label.as_deref().is_some_and(|label| !label.is_empty()));
    // 1 catalog, 2 pages, then per page: page, contents, image; then the shared font/ICC/selection.
    let font_obj = 3 + n * 3;
    let icc = first.image.icc.as_ref();
    let icc_obj = icc.map(|_| font_obj + 1);
    let selection_base = font_obj + if icc.is_some() { 2 } else { 1 };
    let selection_objects = selection_font.objects(selection_base);
    let labels_obj = labels_on.then_some((selection_base + selection_objects.len()) as u32);
    obj(&mut out, format!("<< /Type /Catalog /Pages 2 0 R{} >>", labels_obj.map(|o| format!(" /PageLabels {o} 0 R")).unwrap_or_default()).as_bytes());
    let kids = (0..n).map(|i| format!("{} 0 R", 3 + i * 3)).collect::<Vec<_>>().join(" ");
    obj(&mut out, format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>").as_bytes());
    for (i, page) in pages.iter().enumerate() {
        let page_obj = 3 + i * 3;
        let content_obj = page_obj + 1;
        let image_obj = page_obj + 2;
        let (pw, ph) = page.paper;
        let (x, y, w, h) = page.rect;
        obj(
            &mut out,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {image_obj} 0 R >> /Font << /F1 {font_obj} 0 R /FSelect {selection_base} 0 R >> >> /Contents {content_obj} 0 R >>"
            )
            .as_bytes(),
        );
        // An explicit white page, so every viewer (and rasteriser) shows paper, not transparency.
        let mut content = format!("q 1 g 0 0 {pw:.2} {ph:.2} re f Q\nq {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Im0 Do Q\n");
        if let Some(v) = vectors.get(i) {
            content.push_str(v);
        }
        content.push_str(&marks_ops(page.marks, page.rect));
        let any_marks = page.marks.corner_crop || page.marks.center_crop || page.marks.registration;
        let pad = if any_marks { 30.0 } else { 6.0 };
        if let Some(l) = page.label.as_deref().filter(|l| !l.is_empty()) {
            content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y + h + pad, pdf_string(l)));
        }
        if let Some(d) = page.description.as_deref().filter(|d| !d.is_empty()) {
            content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y - pad - 8.0, pdf_string(d)));
        }
        let c = deflate(content.as_bytes());
        let mut body = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", c.len()).into_bytes();
        body.extend_from_slice(&c);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
        let img = &page.image;
        let device = match img.channels {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        let cs = match icc_obj {
            Some(obj) => format!("[/ICCBased {obj} 0 R]"),
            None => device.to_string(),
        };
        let z = deflate(&img.data);
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {cs} /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            img.width,
            img.height,
            z.len()
        )
        .into_bytes();
        body.extend_from_slice(&z);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
    }
    obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    if let Some(icc) = icc {
        let device = match first.image.channels {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        let z = deflate(icc);
        let mut body = format!("<< /N {} /Alternate {device} /Filter /FlateDecode /Length {} >>\nstream\n", first.image.channels, z.len()).into_bytes();
        body.extend_from_slice(&z);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
    }
    for body in &selection_objects {
        obj(&mut out, body);
    }
    if labels_obj.is_some() {
        // Every page gets an entry: a number-tree range runs until the next key, so omitting an
        // empty name would make that page inherit the previous page's label. An empty
        // dictionary gives the page the default (decimal) label instead.
        let nums = pages
            .iter()
            .enumerate()
            .map(|(i, p)| match p.page_label.as_deref().filter(|label| !label.is_empty()) {
                Some(label) => format!("{i} << /P {} >>", pdf_unicode_string(label)),
                None => format!("{i} << >>"),
            })
            .collect::<Vec<_>>()
            .join(" ");
        obj(&mut out, format!("<< /Nums [{nums}] >>").as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    Ok(out)
}

/// The vector operators for the contiguous visible top prefix of `doc`, top to bottom, with the
/// indices of the layers they replace, and the selection font covering their text (plus, when a
/// lower candidate is rejected during emission, that layer's unused characters).
/// Hidden and blocked layers contribute no characters, so they cannot exhaust the font's codes.
fn collect_vectors(doc: &Document) -> (Vec<(usize, String)>, selection::SelectionFont) {
    enum Candidate<'a> {
        Ops(String),
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
                photocraft_doc::LayerContent::Shape(shape) => match shapes::shape_path(shape, doc.mode) {
                    Some(ops) => Candidate::Ops(ops),
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
    let selection_font = selection::SelectionFont::new(candidates.iter().filter_map(|(_, candidate)| match candidate {
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
pub fn export(doc: &Document, ctl: &photocraft_raster::Interrupt) -> std::result::Result<ExportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let (w, h) = (usize::try_from(doc.size.width).unwrap_or(0), usize::try_from(doc.size.height).unwrap_or(0));
    if w == 0 || h == 0 {
        return Err(IoError::Unsupported("PDF needs a document with non-zero width and height; nothing was written".into()));
    }
    let device = match doc.mode {
        ColorMode::Rgb => 3,
        ColorMode::Grayscale => 1,
        ColorMode::Cmyk => 4,
        m => return Err(IoError::Unsupported(format!("PDF saves RGB, Grayscale and CMYK documents; {m:?} needs converting to RGB first (Image > Mode)"))),
    };
    let fmt = doc.pixel_format();
    let n = fmt.channels().max(1);
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    let paper = if doc.mode == ColorMode::Cmyk { 0.0 } else { 1.0 };
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let icc = doc.icc_profile.as_ref().map(|p| p.to_vec());
    let artboards = doc.artboards();
    if !artboards.is_empty() {
        // Every page's encoded pixels are kept until the PDF is assembled; bound that peak.
        const MAX_PDF_EXPORT_BYTES: u64 = 1 << 30;
        let page_pixels: u64 = artboards.iter().fold(0u64, |bytes, (_, _, a)| bytes.saturating_add(rect_pixels(a.rect)));
        let retained = page_pixels.saturating_mul(device as u64);
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
            let (aw, ah) = (rect.width(), rect.height());
            if aw == 0 || ah == 0 {
                continue;
            }
            // A hidden artboard still gets its page, rendered from its own content rather than
            // left blank by the group's visibility.
            let Some(layer) = doc.layer(*id) else { continue };
            let mut group = layer.clone();
            group.visible = true;
            one.layers.clear();
            one.layers.push(group);
            // Render only this artboard's rect; the surface never holds the whole document.
            let surface = photocraft_compose::composite_to_surface(&one, rect, PixelFormat::new(fmt.mode, fmt.sample, true), None);
            let values = surface.read_region(rect);
            let mut data = Vec::with_capacity((aw as usize * ah as usize).saturating_mul(device));
            for px in values.chunks(n) {
                let a = px.last().copied().unwrap_or(1.0).clamp(0.0, 1.0);
                for v in px.iter().take(device) {
                    data.push(to8(v * a + paper * (1.0 - a)));
                }
            }
            let image = PrintImage { width: aw, height: ah, channels: device, data, icc: icc.clone() };
            let (pw, ph) = (f64::from(aw) * 72.0 / dpi, f64::from(ah) * 72.0 / dpi);
            pages.push(PrintPage {
                paper: (pw, ph),
                image,
                rect: (0.0, 0.0, pw, ph),
                marks: Marks::default(),
                description: None,
                label: None,
                page_label: Some((*name).to_string()),
            });
        }
        if pages.is_empty() {
            return Err(IoError::Unsupported("PDF: the document's artboards have no area".into()));
        }
        let mut warnings = vec!["PDF was saved flattened: layers, masks-as-editable and transparency are not preserved (#3401)".into()];
        warnings.push(format!("PDF exported {} artboards as {} pages; vector/text layers on artboards are rasterised", artboards.len(), pages.len()));
        if !doc.metadata.text.is_empty() {
            warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
        }
        let vectors = vec![String::new(); pages.len()];
        return Ok(ExportResult { bytes: print_pdf_pages(&pages, &vectors, &selection::SelectionFont::default(), ctl)?, warnings });
    }
    let canvas = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
    // Simple topmost shapes and text layers can be drawn over the flattened backdrop without changing
    // compositing. Stop at the first unsupported visible layer, since it may depend on the
    // backdrop (adjustments, blend modes, masks, effects, or clipping).
    let (vector_layers, selection_font) = collect_vectors(doc);
    let mut backdrop = doc.clone();
    for (index, _) in &vector_layers {
        if let Some(layer) = backdrop.layers.get_mut(*index) {
            layer.visible = false;
        }
    }
    let surface = photocraft_compose::flatten_to_surface(&backdrop, PixelFormat::new(fmt.mode, fmt.sample, true), None);
    let values = surface.read_region(canvas);
    // Straight alpha, composited here over paper: white in RGB/Gray (1 = full light), zero ink
    // in CMYK (0 = paper).
    let mut data = Vec::with_capacity((w * h).saturating_mul(device));
    for px in values.chunks(n) {
        let a = px.last().copied().unwrap_or(1.0).clamp(0.0, 1.0);
        for v in px.iter().take(device) {
            data.push(to8(v * a + paper * (1.0 - a)));
        }
    }
    let image = PrintImage { width: doc.size.width, height: doc.size.height, channels: device, data, icc: icc.clone() };
    let (pw, ph) = (f64::from(doc.size.width) * 72.0 / dpi, f64::from(doc.size.height) * 72.0 / dpi);
    let page = PrintPage { paper: (pw, ph), image, rect: (0.0, 0.0, pw, ph), marks: Marks::default(), description: None, label: None, page_label: None };
    let mut warnings = vec!["PDF was saved flattened: layers, masks-as-editable and transparency are not preserved (#3401)".into()];
    if !doc.metadata.text.is_empty() {
        warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
    }
    let scale = 72.0 / dpi;
    let mut vectors = format!("q {scale} 0 0 -{scale} 0 {ph} cm\n");
    for (_, ops) in vector_layers.iter().rev() {
        vectors.push_str(ops);
    }
    vectors.push_str("Q\n");
    Ok(ExportResult { bytes: print_pdf_pages(&[page], &[vectors], &selection_font, ctl)?, warnings })
}

#[cfg(test)]
mod tests;
