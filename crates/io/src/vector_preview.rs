//! PhotoCraft's private PSD display-preview block. The linked-layer payload is never replaced.
//!
//! `pcVP` carries a versioned header, BLAKE3 of the native AI/PDF/EPS source, the SVG byte
//! length and a bounded display-only SVG. Other readers may ignore it and use PSD cached pixels.

use std::sync::Arc;

use photocraft_doc::VectorPreview;
use photocraft_geom::Size;

pub const PSD_KEY: [u8; 4] = *b"pcVP";
const HEADER: &[u8; 8] = b"PCVP\0\0\0\x01";
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_PIXELS: u64 = 24_000_000;
const PREFIX: usize = 44;

/// Recognize native-vector payloads by content; names never authorize a preview.
pub fn is_native_source(source: &[u8]) -> bool {
    source.starts_with(b"%PDF-") || source.starts_with(b"%!PS-Adobe-") || source.starts_with(&[0xc5, 0xd0, 0xd3, 0xc6])
}

#[cfg(test)]
thread_local! { static HASHED_SOURCE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// Hash only a supported, bounded source. PSD import caches this result per source UUID.
pub(crate) fn source_hash(source: &[u8]) -> Option<[u8; 32]> {
    if source.is_empty() || source.len() > MAX_BYTES || !is_native_source(source) {
        return None;
    }
    #[cfg(test)]
    HASHED_SOURCE_BYTES.with(|count| count.set(count.get().saturating_add(source.len())));
    Some(*blake3::hash(source).as_bytes())
}

/// Verify the source binding and render limits before retaining a preview from a file.
pub fn validate(preview: &VectorPreview, source: &[u8]) -> Option<Size> {
    document(preview, source).map(|document| document.size)
}

/// Recover only the unitless source-pixel frame of a normalized preview, without drawing it.
/// The caller must compute `expected_source_hash` from the actual immutable native source;
/// the preview's claimed digest alone never establishes that binding. Rendering callers must
/// still use [`validate`] before drawing. Unsupported metadata leaves the PSD frame available.
pub fn frame_metadata(preview: &VectorPreview, expected_source_hash: [u8; 32]) -> Option<(f64, f64)> {
    if preview.source_hash != expected_source_hash || preview.svg.is_empty() || preview.svg.len() > MAX_BYTES || !crate::svg::is_svg(&preview.svg) {
        return None;
    }
    let text = std::str::from_utf8(&preview.svg).ok()?;
    let parsed = roxmltree::Document::parse(text).ok()?;
    let root = parsed.root_element();
    if root.tag_name().name() != "svg" {
        return None;
    }
    let dimension = |key| root.attribute(key)?.parse::<f64>().ok();
    let (width, height) = dimension("width").zip(dimension("height"))?;
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 && width <= 16384.0 && height <= 16384.0 && width * height <= MAX_PIXELS as f64)
        .then_some((width, height))
}

/// Exact source-pixel frame of a normalized preview. Native PSD dimensions can be fractional;
/// the integer display canvas alone must not replace their placement coordinate system.
pub fn frame(preview: &VectorPreview, source: &[u8]) -> Option<(f64, f64)> {
    frame_for_svg(&preview.svg, validate(preview, source)?)
}

/// The source-pixel frame written by a preview's root `width`/`height`, accepted only when it
/// rounds to the validated display canvas `size`. Without unitless dimensions the display canvas
/// itself is the frame; non-finite, oversized or mismatched dimensions give `None`.
pub(crate) fn frame_for_svg(svg: &[u8], size: Size) -> Option<(f64, f64)> {
    let text = std::str::from_utf8(svg).ok()?;
    let parsed = roxmltree::Document::parse(text).ok()?;
    let root = parsed.root_element();
    let dimension = |key| root.attribute(key).and_then(|value| value.parse::<f64>().ok());
    let Some((width, height)) = dimension("width").zip(dimension("height")) else {
        // A general SVG may spell lengths in physical units. Its validated display canvas is
        // the fallback; our normalized native previews always write unitless source pixels.
        return Some((f64::from(size.width), f64::from(size.height)));
    };
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 || width > 16384.0 || height > 16384.0 || width * height > MAX_PIXELS as f64 {
        return None;
    }
    // Match svg::canvas's f32 conversion, minimum size and rounding exactly. A mismatched
    // viewport cannot supply trusted source geometry even when the SVG itself can render.
    let rounded = |value: f64| ((value as f32).max(1.0).round() as u32).clamp(1, 16384);
    (rounded(width) == size.width && rounded(height) == size.height).then_some((width, height))
}

/// Bounded display document used for PSD smart-filter source caches, never as embedded contents.
pub(crate) fn document(preview: &VectorPreview, source: &[u8]) -> Option<photocraft_doc::Document> {
    if preview.source_hash != source_hash(source)? {
        return None;
    }
    display_document(&preview.svg)
}

/// Bounded preview parsing after the caller has checked the original-source binding.
fn display_document(svg: &[u8]) -> Option<photocraft_doc::Document> {
    display_document_with_budget(svg, 4 * MAX_PIXELS)
}

#[cfg(test)]
thread_local! { static RENDERED_PREVIEWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// Keep callers that need only a display document separate from aggregate-work accounting.
fn display_document_with_budget(svg: &[u8], max_pixels: u64) -> Option<photocraft_doc::Document> {
    display_document_with_work(svg, max_pixels).map(|(document, _)| document)
}

/// Validate a display-only SVG and report the decode/render allowance it consumed.
fn display_document_with_work(svg: &[u8], max_pixels: u64) -> Option<(photocraft_doc::Document, u64)> {
    if max_pixels == 0 || svg.is_empty() || svg.len() > MAX_BYTES || !crate::svg::is_svg(svg) {
        return None;
    }
    #[cfg(test)]
    RENDERED_PREVIEWS.with(|count| count.set(count.get().saturating_add(1)));
    crate::svg::import_svg_preview_with_work("Preview.svg", svg, max_pixels).ok().map(|(imported, work)| (imported.document, work))
}

/// Serialize only a validated preview of the same native source; the blob is four-byte padded.
pub fn encode(preview: &VectorPreview, source: &[u8]) -> Option<Vec<u8>> {
    validate(preview, source)?;
    block_bytes(preview)
}

/// The `pcVP` blob of a preview whose source binding and SVG the caller has validated.
fn block_bytes(preview: &VectorPreview) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(PREFIX + preview.svg.len() + 3);
    out.extend_from_slice(HEADER);
    out.extend_from_slice(&preview.source_hash);
    out.extend_from_slice(&u32::try_from(preview.svg.len()).ok()?.to_be_bytes());
    out.extend_from_slice(&preview.svg);
    out.resize(out.len().next_multiple_of(4), 0);
    Some(out)
}

/// Reject unknown versions, stale hashes, malformed SVG, oversize payloads and trailing data.
pub fn decode(block: &[u8], source: &[u8]) -> Option<VectorPreview> {
    decode_with_hash(block, source_hash(source)?)
}

/// PSD imports reuse the hash of their immutable, bounded embedded source. This function
/// never hashes the large source again and still verifies every block's source binding.
pub(crate) fn decode_with_hash(block: &[u8], expected_hash: [u8; 32]) -> Option<VectorPreview> {
    decode_with_hash_budget(block, expected_hash, 4 * MAX_PIXELS).map(|(preview, _)| preview)
}

/// Check the bounded private block before rendering, retaining its actual validation charge.
fn decode_with_hash_budget(block: &[u8], expected_hash: [u8; 32], max_pixels: u64) -> Option<(VectorPreview, u64)> {
    if block.len() > PREFIX + MAX_BYTES + 3 || block.get(..8)? != HEADER {
        return None;
    }
    let source_hash: [u8; 32] = block.get(8..40)?.try_into().ok()?;
    let length = usize::try_from(u32::from_be_bytes(block.get(40..44)?.try_into().ok()?)).ok()?;
    if length == 0 || length > MAX_BYTES {
        return None;
    }
    let end = PREFIX.checked_add(length)?;
    if block.len() != end.next_multiple_of(4) || block.get(end..)?.iter().any(|byte| *byte != 0) {
        return None;
    }
    // Reject mismatched native contents before allocating or parsing any SVG data.
    if source_hash != expected_hash {
        return None;
    }
    let svg = block.get(PREFIX..end)?;
    let (_, work) = display_document_with_work(svg, max_pixels)?;
    Some((VectorPreview { source_hash, svg: Arc::new(svg.to_vec()) }, work))
}

/// One PSD import's source/preview identities and aggregate validation budget. Cached instances
/// share the preview allocation and do not spend the work budget again.
pub(crate) struct ImportCache {
    source_hashes: std::collections::HashMap<String, Option<[u8; 32]>>,
    previews: std::collections::HashMap<([u8; 32], [u8; 32]), Option<VectorPreview>>,
    remaining_pixels: u64,
}

impl Default for ImportCache {
    /// One file import's cache with the standard 96 MP aggregate preview-work allowance.
    fn default() -> Self {
        Self::new(96_000_000)
    }
}

impl ImportCache {
    /// Start one file operation with an independent aggregate preview-work allowance.
    fn new(max_pixels: u64) -> Self {
        Self { source_hashes: Default::default(), previews: Default::default(), remaining_pixels: max_pixels }
    }

    /// The source callback borrows the first matching immutable linked-layer payload. No disk
    /// paths are followed, and an exhausted import does not hash additional unknown sources.
    pub(crate) fn decode<'a>(&mut self, id: &str, block: &[u8], source: impl FnOnce() -> Option<&'a [u8]>) -> Option<VectorPreview> {
        if block.len() > PREFIX + MAX_BYTES + 3 || block.get(..8)? != HEADER {
            return None;
        }
        if self.remaining_pixels == 0 && !self.source_hashes.contains_key(id) {
            return None;
        }
        let hash = (*self.source_hashes.entry(id.to_string()).or_insert_with(|| source_hash(source()?)))?;
        let key = (hash, *blake3::hash(block).as_bytes());
        if let Some(cached) = self.previews.get(&key) {
            return cached.clone();
        }
        let limit = self.remaining_pixels;
        let decoded = decode_with_hash_budget(block, hash, limit);
        let preview = match decoded {
            Some((preview, work)) => {
                self.remaining_pixels = self.remaining_pixels.saturating_sub(work);
                Some(preview)
            }
            None => {
                // A failed parse can already have decoded images. Reserve its full attempted
                // allowance, even when no display document was returned.
                self.remaining_pixels = self.remaining_pixels.saturating_sub(limit);
                None
            }
        };
        self.previews.insert(key, preview.clone());
        preview
    }
}

/// A native preview validated once per PSD export: its bounded display document (the smart-filter
/// source cache), the exact source-pixel frame (`None` when the SVG's own size does not supply
/// one) and the encoded `pcVP` block.
pub(crate) struct Exported {
    pub(crate) document: photocraft_doc::Document,
    pub(crate) frame: Option<(f64, f64)>,
    pub(crate) block: Vec<u8>,
}

type SourceIdentity = (usize, usize, usize);
type SourceEntry = (Arc<Vec<u8>>, Option<[u8; 32]>);
type PreviewIdentity = ([u8; 32], [u8; 32]);

/// One PSD export's source/preview identities and aggregate validation budget, mirroring
/// [`ImportCache`]. Instances sharing one immutable source and preview allocation hash and render
/// once; distinct previews share the import's 96 MP budget.
pub(crate) struct ExportCache {
    /// Hash of the actual source bytes by (owning allocation, offset, length). Each entry keeps
    /// its owner alive, so an address can never be reused for other bytes during the export.
    sources: std::collections::HashMap<SourceIdentity, SourceEntry>,
    /// Content hash of each preview SVG allocation, kept alive in the same way.
    svgs: std::collections::HashMap<usize, (Arc<Vec<u8>>, [u8; 32])>,
    previews: std::collections::HashMap<PreviewIdentity, Option<Arc<Exported>>>,
    remaining_pixels: u64,
}

impl Default for ExportCache {
    /// One PSD export's cache with the standard 96 MP aggregate preview-work allowance.
    fn default() -> Self {
        Self::new(96_000_000)
    }
}

impl ExportCache {
    /// Start one file operation with an independent aggregate preview-work allowance.
    fn new(max_pixels: u64) -> Self {
        Self { sources: Default::default(), svgs: Default::default(), previews: Default::default(), remaining_pixels: max_pixels }
    }

    /// The validated preview of `source`, a byte range of the immutable `owner`. The preview's
    /// claimed hash is compared with the hash of those actual bytes on every call; only that
    /// hash and the render are reused.
    pub(crate) fn get(&mut self, preview: &VectorPreview, owner: &Arc<Vec<u8>>, source: &[u8]) -> Option<Arc<Exported>> {
        let base = owner.as_ptr().addr();
        let offset = source.as_ptr().addr().checked_sub(base)?;
        // Only a range of the pinned owner may be identified by its address.
        if offset.checked_add(source.len())? > owner.len() || preview.svg.is_empty() || preview.svg.len() > MAX_BYTES {
            return None;
        }
        let key = (base, offset, source.len());
        let svg_key = Arc::as_ptr(&preview.svg).addr();
        // An exhausted export does not hash additional unknown sources or previews.
        if self.remaining_pixels == 0 && !(self.sources.contains_key(&key) && self.svgs.contains_key(&svg_key)) {
            return None;
        }
        let hash = self.sources.entry(key).or_insert_with(|| (Arc::clone(owner), source_hash(source))).1?;
        if preview.source_hash != hash {
            return None;
        }
        let svg_hash = self.svgs.entry(svg_key).or_insert_with(|| (Arc::clone(&preview.svg), *blake3::hash(&preview.svg).as_bytes())).1;
        if let Some(cached) = self.previews.get(&(hash, svg_hash)) {
            return cached.clone();
        }
        let limit = self.remaining_pixels;
        let exported = match display_document_with_work(&preview.svg, limit) {
            Some((document, work)) => {
                self.remaining_pixels = self.remaining_pixels.saturating_sub(work);
                let frame = frame_for_svg(&preview.svg, document.size);
                block_bytes(preview).map(|block| Arc::new(Exported { document, frame, block }))
            }
            None => {
                // Failed validation may already have spent its whole decode allowance.
                self.remaining_pixels = self.remaining_pixels.saturating_sub(limit);
                None
            }
        };
        self.previews.insert((hash, svg_hash), exported.clone());
        exported
    }
}

#[cfg(test)]
mod tests {

    /// Frame recovery parses bounded geometry only; display validation remains a separate gate.
    #[test]
    fn metadata_frame_is_source_bound_and_does_not_decode_old_preview_images() {
        let source = b"%PDF-1.7\noriginal native vector";
        let hash = *blake3::hash(source).as_bytes();
        let mut preview = photocraft_doc::VectorPreview {
            source_hash: hash,
            svg: std::sync::Arc::new(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="8.3" height="6.2"><image href="data:image/png;base64,AAAA" width="8" height="6"/></svg>"#
                    .to_vec(),
            ),
        };
        assert_eq!(super::frame_metadata(&preview, hash), Some((8.3, 6.2)));
        assert!(super::validate(&preview, source).is_none(), "malformed image data is still rejected by actual display validation");
        assert!(super::frame_metadata(&preview, [0; 32]).is_none());
        for (width, height) in [("8pt", "6"), ("NaN", "6"), ("0", "6"), ("16384", "16384")] {
            preview.svg = std::sync::Arc::new(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"/>"#).into_bytes());
            assert!(super::frame_metadata(&preview, hash).is_none(), "unsupported frame {width} by {height}");
        }
    }
    use super::*;

    const SOURCE: &[u8] = b"%PDF-1.7\nprivate native source";
    const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"><rect width="8" height="6" fill="red"/></svg>"#;

    fn preview() -> VectorPreview {
        VectorPreview { source_hash: *blake3::hash(SOURCE).as_bytes(), svg: Arc::new(SVG.to_vec()) }
    }

    #[test]
    fn block_roundtrip_binds_preview_to_exact_native_contents() {
        let preview = preview();
        let block = encode(&preview, SOURCE).unwrap();
        assert_eq!(decode(&block, SOURCE), Some(preview));
        assert!(decode(&block, b"%PDF-1.7\nchanged source").is_none());
        assert!(decode(&block, SVG).is_none());
    }

    #[test]
    fn malformed_preview_blocks_are_ignored_without_losing_the_native_source() {
        let good = encode(&preview(), SOURCE).unwrap();
        for truncated in [0, 7, 39, 43, good.len() - 1] {
            assert!(decode(&good[..truncated], SOURCE).is_none());
        }
        let mut bad = good.clone();
        bad[7] = 2;
        assert!(decode(&bad, SOURCE).is_none());
        bad = good.clone();
        bad[40..44].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode(&bad, SOURCE).is_none());
        bad = good;
        bad.extend_from_slice(&[0; 4]);
        assert!(decode(&bad, SOURCE).is_none());
        let mut preview = preview();
        preview.svg = Arc::new(b"not SVG".to_vec());
        assert!(encode(&preview, SOURCE).is_none());
        preview.svg = Arc::new(br#"<svg xmlns="http://www.w3.org/2000/svg" width="16384" height="16384"/>"#.to_vec());
        assert!(encode(&preview, SOURCE).is_none());
    }

    #[test]
    fn many_psd_instances_hash_the_native_payload_once_and_share_the_validated_preview() {
        use photocraft_color::{ColorMode, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::Affine;
        use photocraft_raster::Surface;

        let mut source = vec![b' '; MAX_BYTES];
        source[..SOURCE.len()].copy_from_slice(SOURCE);
        let preview = VectorPreview { source_hash: *blake3::hash(&source).as_bytes(), svg: Arc::new(SVG.to_vec()) };
        let mut doc = Document::new("shared preview", Size::new(8, 6), ColorMode::Rgb, SampleType::U8);
        let mut smart = SmartObject::new(
            SmartSource::Embedded { file_name: "Artwork.ai".into(), bytes: Arc::new(source.clone()) },
            Affine::IDENTITY,
            Some(Surface::new(doc.pixel_format())),
        );
        smart.vector_preview = Some(preview.clone());
        doc.layers.push(Layer::new("native", LayerContent::Smart(smart)));
        let out = crate::export(&doc, "psd", &Default::default()).unwrap();
        let mut file = photocraft_psd::PsdFile::from_bytes(&out.bytes).unwrap();
        let record = file.layers()[0].clone();
        *file.layers_mut() = vec![record; 100];
        let before = HASHED_SOURCE_BYTES.with(std::cell::Cell::get);
        let (back, _) = crate::psd_to_document(&file);
        let hashed = HASHED_SOURCE_BYTES.with(std::cell::Cell::get) - before;
        assert_eq!(hashed, source.len(), "a shared 16 MiB payload must not be hashed per layer");
        let previews: Vec<_> = back
            .layers
            .iter()
            .map(|layer| {
                let LayerContent::Smart(smart) = &layer.content else { panic!("smart object lost") };
                smart.vector_preview.as_ref().expect("preview lost")
            })
            .collect();
        assert_eq!(previews.len(), 100);
        assert!(previews.iter().all(|actual| **actual == preview));
        assert!(previews.iter().all(|actual| Arc::ptr_eq(&actual.svg, &previews[0].svg)), "shared layers retain one SVG allocation");
    }

    #[test]
    fn psd_export_hashes_and_renders_a_shared_native_preview_once_for_all_instances() {
        use photocraft_color::{ColorMode, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::Affine;
        use photocraft_raster::Surface;

        let counters = || {
            (
                HASHED_SOURCE_BYTES.with(std::cell::Cell::get),
                RENDERED_PREVIEWS.with(std::cell::Cell::get),
                crate::psd_export::EMBED_HASHED_SOURCE_BYTES.with(std::cell::Cell::get),
            )
        };
        let mut source = vec![b' '; 256 * 1024];
        source[..SOURCE.len()].copy_from_slice(SOURCE);
        let preview = VectorPreview { source_hash: *blake3::hash(&source).as_bytes(), svg: Arc::new(SVG.to_vec()) };
        let mut doc = Document::new("shared preview", Size::new(8, 6), ColorMode::Rgb, SampleType::U8);
        let mut smart = SmartObject::new(
            SmartSource::Embedded { file_name: "Artwork.ai".into(), bytes: Arc::new(source.clone()) },
            Affine::IDENTITY,
            Some(Surface::new(doc.pixel_format())),
        );
        smart.vector_preview = Some(preview.clone());
        for _ in 0..100 {
            doc.layers.push(Layer::new("native", LayerContent::Smart(smart.clone())));
        }
        let check = |bytes: &[u8]| {
            let back = crate::import("shared.psd", bytes).unwrap().document;
            assert_eq!(back.layers.len(), 100);
            for layer in &back.layers {
                let LayerContent::Smart(sm) = &layer.content else { panic!("smart object lost") };
                assert_eq!(sm.vector_preview.as_ref(), Some(&preview), "valid preview lost");
                let SmartSource::Linked { path } = &sm.source else { panic!("expected the PSD embedded-file table") };
                assert_eq!(crate::linked::embedded_payload(&back.metadata, path).unwrap().1, source.as_slice(), "original native bytes changed");
            }
            back
        };

        // Embedded instances share one source allocation.
        let before = counters();
        let out = crate::export(&doc, "psd", &Default::default()).unwrap();
        let after = counters();
        assert_eq!(after.0 - before.0, source.len(), "the shared native source must not be hashed per layer or per use");
        assert_eq!(after.1 - before.1, 1, "the shared preview must not be rendered per layer or per use");
        assert_eq!(after.2 - before.2, source.len(), "the conversion cache must not re-hash the shared Embedded owner per layer");
        let file = photocraft_psd::PsdFile::from_bytes(&out.bytes).unwrap();
        assert!(file.layers().iter().all(|layer| layer.block(&PSD_KEY).is_some()));
        let back = check(&out.bytes);

        // Reopened instances link to one preserved embedded file of the document.
        let before = counters();
        let again = crate::export(&back, "psd", &Default::default()).unwrap();
        let after = counters();
        assert_eq!(after.0 - before.0, source.len(), "the preserved linked payload must not be hashed per layer or per use");
        assert_eq!(after.1 - before.1, 1, "the reopened shared preview must not be rendered per layer or per use");
        assert_eq!(after.2 - before.2, 0, "preserved linked-table sources need no Embedded conversion hash");
        check(&again.bytes);
    }

    #[test]
    fn export_cache_checks_actual_source_bytes_and_bounds_distinct_previews() {
        let owner = Arc::new(SOURCE.to_vec());
        let red = preview();
        let mut blue = red.clone();
        blue.svg = Arc::new(String::from_utf8(SVG.to_vec()).unwrap().replace("red", "blue").into_bytes());
        // A claimed hash of other bytes is never trusted, even after the real source was cached.
        let mut stale = red.clone();
        stale.source_hash = *blake3::hash(b"%PDF-1.7\nother").as_bytes();
        let changed = Arc::new(b"%PDF-1.7\nprivate native sourcf".to_vec());
        let mut cache = ExportCache::new(95);
        assert!(cache.get(&stale, &owner, owner.as_slice()).is_none());
        let first = cache.get(&red, &owner, owner.as_slice()).unwrap();
        assert_eq!(first.block, encode(&red, SOURCE).unwrap());
        assert_eq!(first.frame, Some((8.0, 6.0)));
        assert!(cache.get(&red, &changed, changed.as_slice()).is_none(), "equal-length changed bytes must not reuse the cached hash");
        assert!(cache.get(&stale, &owner, owner.as_slice()).is_none());
        // A range outside the owner cannot be keyed by its address.
        assert!(cache.get(&red, &owner, SOURCE).is_none());
        let remaining = cache.remaining_pixels;
        assert_eq!(remaining, 47);
        for _ in 0..100 {
            assert!(Arc::ptr_eq(&cache.get(&red, &owner, owner.as_slice()).unwrap(), &first));
        }
        assert_eq!(cache.remaining_pixels, remaining, "cached instances are free");
        // Two 8×6 previews take 96 pixels; a 95-pixel export may validate only one.
        assert!(cache.get(&blue, &owner, owner.as_slice()).is_none());
        assert_eq!(cache.remaining_pixels, 0);
        let other = Arc::new(SOURCE.to_vec());
        assert!(cache.get(&red, &other, other.as_slice()).is_none(), "an exhausted export hashes no further sources");
        assert!(cache.get(&red, &owner, owner.as_slice()).is_some(), "a valid cache hit spends no work");
    }

    /// Distinct tiny previews are cheap; a tiny canvas cannot disguise a large raster decode.
    #[test]
    fn preview_caches_charge_decode_work_and_retain_many_normal_tiny_previews() {
        let owner = Arc::new(SOURCE.to_vec());
        let mut imports = ImportCache::new(10_000);
        let mut exports = ExportCache::new(10_000);
        let mut tiny = Vec::new();
        for n in 0..12 {
            let mut drawing = preview();
            drawing.svg = Arc::new(String::from_utf8(SVG.to_vec()).unwrap().replace("red", &format!("#{n:06x}")).into_bytes());
            let block = block_bytes(&drawing).unwrap();
            assert!(imports.decode("native", &block, || Some(SOURCE)).is_some());
            assert!(exports.get(&drawing, &owner, owner.as_slice()).is_some());
            tiny.push((drawing, block));
        }
        assert_eq!(imports.remaining_pixels, 10_000 - 12 * 48);
        assert_eq!(exports.remaining_pixels, imports.remaining_pixels);
        let image = photocraft_codecs::Image::from_u8(64, 64, photocraft_codecs::ChannelLayout::Rgb, vec![255; 64 * 64 * 3]).unwrap();
        let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let data = png.iter().map(|byte| format!("%{byte:02X}")).collect::<String>();
        let mut drawing = preview();
        drawing.svg = Arc::new(
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><image width="1" height="1" href="data:image/png,{data}"/></svg>"#)
                .into_bytes(),
        );
        let block = block_bytes(&drawing).unwrap();
        let before = imports.remaining_pixels;
        assert!(imports.decode("native", &block, || None).is_some());
        assert!(exports.get(&drawing, &owner, owner.as_slice()).is_some());
        assert!(before - imports.remaining_pixels >= 4097);
        assert_eq!(exports.remaining_pixels, imports.remaining_pixels);
        // An invalid new preview may already spend work; its attempted allowance is charged.
        let mut invalid = preview();
        invalid.svg = Arc::new(br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><image href="data:image/png,invalid"/></svg>"#.to_vec());
        assert!(imports.decode("native", &block_bytes(&invalid).unwrap(), || None).is_none());
        assert!(exports.get(&invalid, &owner, owner.as_slice()).is_none());
        assert_eq!(imports.remaining_pixels, 0);
        assert_eq!(exports.remaining_pixels, 0);
        assert!(imports.decode("native", &tiny[0].1, || None).is_some(), "valid import cache hits remain available");
        assert!(exports.get(&tiny[0].0, &owner, owner.as_slice()).is_some(), "valid export cache hits remain available");
    }

    #[test]
    fn distinct_import_previews_share_a_work_budget_and_cached_instances_are_free() {
        let red = preview();
        let mut blue = red.clone();
        blue.svg = Arc::new(String::from_utf8(SVG.to_vec()).unwrap().replace("red", "blue").into_bytes());
        let red_block = encode(&red, SOURCE).unwrap();
        let blue_block = encode(&blue, SOURCE).unwrap();
        // Two 8×6 previews would take 96 pixels; a 95-pixel import may validate only one.
        // This small budget exercises the production 96 MP policy without allocating 96 MP.
        let mut cache = ImportCache::new(95);
        assert!(cache.decode("shared", &red_block, || Some(SOURCE)).is_some());
        assert_eq!(cache.remaining_pixels, 47);
        for _ in 0..100 {
            assert!(cache.decode("shared", &red_block, || panic!("cached source was read again")).is_some());
        }
        assert_eq!(cache.remaining_pixels, 47);
        assert!(cache.decode("shared", &blue_block, || panic!("cached source was read again")).is_none());
        assert_eq!(cache.remaining_pixels, 0);
        assert!(cache.decode("new source", &blue_block, || panic!("exhausted import read another source")).is_none());
        assert!(cache.decode("shared", &red_block, || panic!("valid cache hit spent work")).is_some());
        let mut cache = ImportCache::new(96);
        assert!(cache.decode("shared", &red_block, || Some(SOURCE)).is_some());
        assert!(cache.decode("shared", &blue_block, || None).is_some());
        assert_eq!(cache.remaining_pixels, 0);
    }
}
