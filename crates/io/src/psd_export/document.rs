use super::PSD_MAX_FILE_SIZE;
use super::estimate::{MERGED_RENDERED, MERGED_WEIGHT, fit_psd_size, psd_version, sign_global_blocks};
use super::merged::{cmyk_of, merged_planes, placeholder_planes};
use super::resources::{global_block, guides_resource, keep_imported_resource, legacy_name, pascal_names_resource, path_resources, unicode_names_resource};
use super::smart::{SmartOut, link_group_resource};
use super::{
    ColorMode, Compression, Document, Ex, GlobalLayerMask, Header, ImageData, ImageResource, Interrupt, LayerContent, LayerInfo, LayerInfoPlacement,
    MaskParameters, PixelFormat, PsdExportOptions, PsdFile, PsdMode, ResolutionInfo, SampleType, TaggedBlock, deinterleave, ids, psd_background, psd_depth,
    psd_mode, q255, version_info_resource,
};

/// Converts a document to a PSD file model (see [`document_to_psd_with`]).
pub fn document_to_psd(doc: &Document) -> PsdFile {
    document_to_psd_with(doc, &PsdExportOptions::default()).0
}

/// Converts a document to a PSD/PSB file model, returning warnings about
/// anything that could not be represented. The merged composite is rendered
/// with `photocraft_compose::flatten`.
pub fn document_to_psd_with(doc: &Document, opts: &PsdExportOptions) -> (PsdFile, Vec<String>) {
    document_to_psd_sized(doc, opts, 0, &Interrupt::NONE, PSD_MAX_FILE_SIZE)
}

/// [`document_to_psd_with`] for a background save (#3389): reports progress through `ctl` after
/// each layer record and each band of the merged composite. Once `ctl` is cancelled it stops
/// early and returns an **incomplete** model: callers check `ctl` and discard it.
pub(crate) fn document_to_psd_interruptible(doc: &Document, opts: &PsdExportOptions, ctl: &Interrupt) -> (PsdFile, Vec<String>) {
    document_to_psd_sized(doc, opts, 0, ctl, PSD_MAX_FILE_SIZE)
}

/// [`document_to_psd_with`] for a document embedded `depth` smart objects deep.
pub(super) fn document_to_psd_nested(doc: &Document, opts: &PsdExportOptions, depth: u32, ctl: &Interrupt) -> (PsdFile, Vec<String>) {
    document_to_psd_sized(doc, opts, depth, ctl, PSD_MAX_FILE_SIZE)
}

/// Export with a caller-selected PSD size limit; the test suite lowers it to exercise promotion.
pub(super) fn document_to_psd_sized(doc: &Document, opts: &PsdExportOptions, depth: u32, ctl: &Interrupt, size_limit: u64) -> (PsdFile, Vec<String>) {
    let layerless_limit = opts.size_limit_psb.then_some(size_limit);
    if doc.mode == ColorMode::Multichannel {
        return crate::multichannel_map::document_to_psd(doc, opts.force_psb, layerless_limit);
    }
    if let Some(file) = crate::palette_map::document_to_psd(doc, opts.force_psb, layerless_limit) {
        return file;
    }
    let fmt = doc.pixel_format();
    export_supported_document(doc, opts, depth, ctl, fmt, size_limit)
}

fn export_supported_document(
    doc: &Document,
    opts: &PsdExportOptions,
    depth: u32,
    ctl: &Interrupt,
    fmt: PixelFormat,
    size_limit: u64,
) -> (PsdFile, Vec<String>) {
    // A Duotone is written over its grey plate with its inks: the block it was imported with,
    // verbatim, while the inks are unedited, else one made from them (`duotone_map`). Photoshop
    // has only 8-bit Duotone: it refuses to open a 16-bit one.
    let duotone_inks: Option<Vec<u8>> = doc
        .duotone
        .as_ref()
        .filter(|d| !d.inks.is_empty() && doc.mode == ColorMode::Duotone && fmt.mode == ColorMode::Grayscale && fmt.sample == SampleType::U8)
        .map(|d| d.psd_raw.as_deref().map_or_else(|| crate::duotone_map::write(d), <[u8]>::to_vec));
    let sample = fmt.sample;
    let cc = fmt.mode.color_channels();
    let big = doc.size.width > 30_000 || doc.size.height > 30_000;
    let version = psd_version(doc, opts.force_psb);
    let mut ex = Ex {
        fmt,
        dpi: doc.resolution_dpi,
        document: opts.document_path.clone(),
        read_linked_files: opts.read_linked_files,
        text_styles: doc.text_styles.clone(),
        text_index: Default::default(),
        mask_fmt: PixelFormat::new(ColorMode::Grayscale, sample, false),
        cc,
        cmyk: fmt.mode == ColorMode::Cmyk,
        version,
        next_id: 0,
        used_ids: Default::default(),
        layer_ids: Default::default(),
        canvas: doc.bounds(),
        records: Vec::new(),
        warnings: Vec::new(),
        guides: doc.guides.clone(),
        comps: (!crate::comps_map::comps_unchanged(doc)).then(|| (doc.layer_comps.clone(), doc.last_document_state.clone())),
        smart: SmartOut::new(doc, depth),
        patterns: doc.patterns.clone(),
        background: psd_background(doc),
    };
    add_format_warnings(doc, opts, fmt, big, duotone_inks.is_some(), &mut ex.warnings);
    index_layer_ids_and_text(doc, &mut ex);
    // Progress: one unit per layer record, then the merged composite (#3389).
    let records = doc.walk().len() as f32;
    let units = records + MERGED_WEIGHT;
    let layers_share = records / units;
    let mut done = 0.0f32;
    let mut tick = || {
        done += 1.0;
        ctl.progress(done / units);
        !ctl.cancelled()
    };
    let space = photocraft_compose::cmyk_space(doc);
    photocraft_color::convert::with_cmyk_space(space.as_ref(), || ex.emit(&doc.layers, &mut tick));
    drop(tick);
    ctl.progress(layers_share);

    let merged = prepare_merged_planes(doc, &mut ex, opts, ctl, fmt, duotone_inks.is_some(), layers_share);
    let MergedExport { header, image_data, extra, has_alpha, quick_mask_id, real_merged } = merged;

    // Resources.
    let mut resources = vec![
        // hRes/vRes are always pixels per inch. The display unit written here is inches.
        ImageResource::new(ids::RESOLUTION_INFO, ResolutionInfo::from_dpi(f64::from(doc.resolution_dpi)).to_bytes()),
        ImageResource::new(ids::GLOBAL_ANGLE, (doc.global_light.angle.round() as i32).to_be_bytes().to_vec()),
        ImageResource::new(ids::GLOBAL_ALTITUDE, (doc.global_light.altitude.round() as i32).to_be_bytes().to_vec()),
    ];
    if let Some(icc) = &doc.icc_profile {
        resources.push(ImageResource::new(ids::ICC_PROFILE, icc.to_vec()));
    }
    if !doc.guides.horizontal.is_empty() || !doc.guides.vertical.is_empty() {
        resources.push(ImageResource::new(1032, guides_resource(doc)));
    }
    if !extra.is_empty() {
        // One entry per extra channel, the merged transparency included (as in every file
        // Photoshop opens; one short, Photoshop refuses the file with "unexpected end-of-file").
        let names: Vec<&str> = has_alpha.then_some("Transparency").into_iter().chain(extra.iter().map(|a| a.name.as_str())).collect();
        resources.push(ImageResource::new(1006, pascal_names_resource(&names)));
        resources.push(ImageResource::new(1045, unicode_names_resource(&names)));
        resources.push(ImageResource::new(crate::channel_map::DISPLAY_INFO, crate::channel_map::display_info(&extra, has_alpha)));
        // Photoshop writes 1053 in every file with an extra channel. Its absence is how a file from
        // an earlier PhotoCraft is told apart (`psd_import::earlier_photocraft_spot_planes`).
        resources.push(ImageResource::new(crate::channel_map::ALPHA_IDENTIFIERS, crate::channel_map::alpha_identifiers(has_alpha, extra.len())));
    }
    if let Some(id) = quick_mask_id {
        let mut data = id.to_be_bytes().to_vec();
        data.push(0);
        resources.push(ImageResource::new(crate::channel_map::QUICK_MASK_INFO, data));
    }
    resources.extend(path_resources(doc, &mut ex.warnings));
    if let Some(groups) = link_group_resource(&doc.layers) {
        resources.push(ImageResource::new(ids::LAYER_GROUP_INFO, groups));
    }
    // The pixels are saved as they are shown: never let a reader rotate them again. The XMP and
    // EXIF resolution follow the ResolutionInfo resource, so no copy contradicts it (#1691).
    let ppi = Some((doc.resolution_dpi, doc.resolution_dpi)).filter(|d| d.0 > 0.0);
    if let Some(x) = &doc.metadata.xmp {
        resources.push(ImageResource::new(ids::XMP, photocraft_codecs::export_xmp(x, ppi).as_bytes().to_vec()));
    }
    if let Some(e) = &doc.metadata.exif {
        resources.push(ImageResource::new(ids::EXIF, photocraft_codecs::export_exif(e, ppi).into_owned()));
    }
    let mut global_blocks = Vec::new();
    let preserved = ex.smart.finish(crate::annotations_map::export_blocks(doc, crate::pattern_map::export_global_blocks(doc)));
    // The type layers' `Txt2` is regenerated (the auto-kern modes live there, and a preserved
    // block goes stale as text is edited): one object per `TextIndex`, at the slot each layer's
    // `TySh` names (unused slots stay empty; see `photocraft_text::psd::build_txt2`). The block
    // it replaces is handed over so a kept object keeps everything the file held beyond its text
    // and style runs (Photoshop's `/21 /1` glyph pen positions, …).
    let text_layers: Vec<(i32, &photocraft_doc::TextLayer)> = doc
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Text(t) => Some((ex.text_index.get(&l.id).copied().unwrap_or(0), t)),
            _ => None,
        })
        .collect();
    let previous = (!text_layers.is_empty()).then(|| preserved.iter().find(|(_, key, _)| key == b"Txt2").map(|(_, _, data)| data.to_vec())).flatten();
    // A fresh document of ordinary (Metrics) text has nothing to put in `Txt2`, and Photoshop never
    // writes its empty `<< /0 << >> >>` stub — readers such as Photopea reject it (#3514). Keep the
    // block when the file had one (round-trip its pen positions/extras) or a layer is Optical (the
    // one mode EngineData can't carry; #1348).
    let txt2 = (previous.is_some() || photocraft_text::psd::needs_txt2(&text_layers))
        .then(|| photocraft_text::psd::build_txt2_at(&text_layers, previous.as_deref(), ex.dpi));
    for (sig, key, data) in preserved {
        if txt2.is_some() && key == *b"Txt2" {
            continue;
        }
        // A kept layer block (one the import could not read, #2700) describes layers the
        // document doesn't have. The document's own layers are written below, and a file holds
        // one layer list: a second `Lr16`/`Lr32` would contradict it.
        if !ex.records.is_empty() && matches!(&key, b"Lr16" | b"Lr32" | b"Layr") {
            continue;
        }
        let block = global_block(sig, key, &data);
        // A kept merged-transparency marker describes the opened file's composite. Over an opaque
        // one it would make the first alpha channel read back as the transparency, and vanish.
        if !has_alpha && block.is_merged_transparency_marker() {
            continue;
        }
        global_blocks.push(block);
    }
    if let Some(txt2) = txt2 {
        let mut tb = TaggedBlock::new(*b"Txt2", txt2.clone());
        tb.signature = *b"8BIM";
        tb.padding = Some(vec![0; (4 - txt2.len() % 4) % 4]);
        global_blocks.push(tb);
    }
    let comps_resource = ex.comps.is_some().then(|| crate::comps_map::write_comps_resource(doc));
    let (slices_resource, slices_warning) = crate::slices_map::export_resource(doc, &ex.layer_ids);
    if let Some(warning) = slices_warning {
        ex.warnings.push(warning);
    }
    for (id, name, data) in &doc.metadata.psd_resources {
        // Layer comps: the preserved list while unchanged, else regenerated (or dropped) below.
        if *id == crate::comps_map::LAYER_COMPS && comps_resource.is_some() {
            continue;
        }
        // Slices: the preserved resource while unchanged, else regenerated (or dropped) below.
        if *id == crate::slices_map::SLICES && slices_resource.is_some() {
            continue;
        }
        if !keep_imported_resource(doc, *id, data) {
            continue;
        }
        // 1053 is written afresh with the extra channels. A kept copy (from a Multichannel file, or
        // a document saved by an earlier version) would be a second one, or list channels that
        // are gone.
        if *id == crate::channel_map::ALPHA_IDENTIFIERS {
            continue;
        }
        let mut r = ImageResource::new(*id, data.to_vec());
        r.name = legacy_name(name);
        resources.push(r);
    }
    if let Some(data) = crate::annotations_map::fresh_scale_resource(doc) {
        resources.push(ImageResource::new(crate::annotations_map::MEASUREMENT_SCALE, data));
    }
    if let Some(Some(data)) = comps_resource {
        resources.push(ImageResource::new(crate::comps_map::LAYER_COMPS, data));
    }
    if let Some(Some(data)) = slices_resource {
        resources.push(ImageResource::new(crate::slices_map::SLICES, data));
    }
    resources.push(version_info_resource(real_merged));

    let has_layers = !ex.records.is_empty();
    // Photoshop writes a merged-transparency marker (`Mtrn`, `Mt16`, `Mt32`; `MTrn` in a TIFF)
    // before the layer block, and won't open a file whose `Lr32` comes first (#5068): the layer
    // block goes right after the last kept marker, else first.
    let index = global_blocks.iter().rposition(TaggedBlock::is_merged_transparency_marker).map_or(0, |i| i + 1);
    let placement = match (has_layers, sample) {
        (true, SampleType::U16) => LayerInfoPlacement::GlobalBlock { index, signature: *b"8BIM", key: *b"Lr16", padding: None },
        (true, SampleType::F32) => LayerInfoPlacement::GlobalBlock { index, signature: *b"8BIM", key: *b"Lr32", padding: None },
        _ => LayerInfoPlacement::Section,
    };
    let records = std::mem::take(&mut ex.records);
    let mut layer_info = has_layers.then_some(LayerInfo { merged_alpha: has_alpha, layers: records, padding: None });
    if let Some(info) = &mut layer_info {
        // Photoshop pads the layer info to 4 bytes (an even pad misaligns readers after a global
        // `Lr16`/`Lr32` block, #200). A length that cannot be computed keeps the even default.
        // A global block's length is the exact data length and its padding to 4 follows the block:
        // readers that check that length exactly (Unity's PSD Importer) reject it otherwise (#4654).
        if matches!(placement, LayerInfoPlacement::GlobalBlock { .. }) {
            info.padding = Some(Vec::new());
        } else if info.pad_to(version, 4).is_err() {
            info.padding = None;
        }
    }
    // 32-bit files carry Photoshop's HDR toning records as Color Mode Data; Photoshop will not
    // open a 32-bit file without them (#291).
    let color_mode_data = match duotone_inks {
        Some(raw) => raw,
        None => photocraft_psd::hdr::color_mode_data_for_depth(header.depth),
    };
    let mut file = PsdFile {
        header,
        color_mode_data,
        resources,
        layer_info,
        layer_info_placement: placement,
        global_layer_mask: (has_layers || !global_blocks.is_empty()).then(GlobalLayerMask::default),
        global_blocks,
        layer_mask_trailing: Vec::new(),
        image_data,
        write_as_psb: false,
    };
    if opts.size_limit_psb
        && let Some(warning) = fit_psd_size(&mut file, size_limit)
    {
        ex.warnings.push(warning);
    }
    sign_global_blocks(&mut file);
    (file, ex.warnings)
}

fn add_format_warnings(doc: &Document, opts: &PsdExportOptions, fmt: PixelFormat, big: bool, has_duotone: bool, warnings: &mut Vec<String>) {
    if big && !opts.force_psb {
        warnings.push("document exceeds 30000 px; written as PSB".into());
    }
    if fmt.mode != doc.mode && !has_duotone {
        warnings.push(format!("{:?} document written as {:?}", doc.mode, fmt.mode));
    }
}

struct MergedExport<'a> {
    header: Header,
    image_data: ImageData,
    extra: Vec<&'a photocraft_doc::AlphaChannel>,
    has_alpha: bool,
    quick_mask_id: Option<u16>,
    real_merged: bool,
}

fn prepare_merged_planes<'a>(
    doc: &'a Document,
    ex: &mut Ex,
    opts: &PsdExportOptions,
    ctl: &Interrupt,
    fmt: PixelFormat,
    duotone: bool,
    layers_share: f32,
) -> MergedExport<'a> {
    let sample = fmt.sample;
    let cc = fmt.mode.color_channels();
    let real_merged = opts.maximize_compatibility || ex.records.is_empty() || duotone;
    let (mut planes, has_alpha) = merged_image(doc, opts, ctl, &fmt, real_merged, layers_share);
    if !has_alpha {
        planes.truncate(cc * doc.size.area() as usize * sample.bytes());
    }
    let (extra, quick_mask_id) = append_extra_channels(doc, ex, &mut planes, has_alpha, sample);
    let channels = (cc + usize::from(has_alpha) + extra.len()) as u16;
    let mode = if duotone { PsdMode::Duotone } else { psd_mode(fmt.mode) };
    let header = Header::new(ex.version, doc.size.width, doc.size.height, channels, psd_depth(sample), mode);
    let compression = if sample == SampleType::F32 { Compression::Raw } else { opts.merged_compression };
    let image_data = ImageData::encode(compression, &planes, &header).or_else(|_| ImageData::encode(Compression::Raw, &planes, &header)).unwrap_or(ImageData {
        compression: Compression::Raw,
        data: planes,
        stored: None,
    });
    ctl.progress(1.0);
    MergedExport { header, image_data, extra, has_alpha, quick_mask_id, real_merged }
}

fn append_extra_channels<'a>(
    doc: &'a Document,
    ex: &mut Ex,
    planes: &mut Vec<u8>,
    has_alpha: bool,
    sample: SampleType,
) -> (Vec<&'a photocraft_doc::AlphaChannel>, Option<u16>) {
    let cc = ex.cc;
    let max_extra = 56 - cc - usize::from(has_alpha);
    if doc.channels.len() > max_extra {
        ex.warnings.push(format!("only {max_extra} alpha channels fit in PSD; the rest were dropped"));
    }
    let mut extra: Vec<_> = doc.channels.iter().take(max_extra).collect();
    let quick_mask_id = match &doc.quick_mask {
        Some(q) if extra.len() < max_extra => {
            extra.push(q);
            Some((cc + usize::from(has_alpha) + extra.len() - 1) as u16)
        }
        Some(_) => {
            ex.warnings.push("Quick Mask was dropped because the PSD channel limit was reached".into());
            None
        }
        None => None,
    };
    let canvas = doc.bounds();
    for channel in &extra {
        let surface = if channel.surface.format() != ex.mask_fmt { channel.surface.convert(ex.mask_fmt) } else { channel.surface.clone() };
        let mut bytes = surface.to_interleaved(canvas);
        if channel.spot.is_some() {
            crate::channel_map::photoshop_spot_plane(&mut bytes, sample);
        }
        planes.extend(deinterleave(&bytes, 1, sample, &[false]).remove(0));
    }
    (extra, quick_mask_id)
}

fn merged_image(doc: &Document, opts: &PsdExportOptions, ctl: &Interrupt, fmt: &PixelFormat, real_merged: bool, layers_share: f32) -> (Vec<u8>, bool) {
    if !real_merged {
        return (placeholder_planes(doc, fmt), false);
    }
    // Merged composite, rendered and encoded in bands (no full-size float composite). Matting
    // against white only changes pixels with alpha < 1; if some are slightly translucent but
    // all round to opaque (so no alpha channel is written), encode once more without the matte.
    let merged_end = layers_share + (1.0 - layers_share) * MERGED_RENDERED;
    let (mut planes, has_alpha, translucent) =
        crate::with_span(ctl, layers_share, merged_end, |ctl| merged_planes(doc, fmt, cmyk_of(fmt), opts.merged_matte, ctl));
    if !has_alpha && translucent && opts.merged_matte {
        planes = merged_planes(doc, fmt, cmyk_of(fmt), false, &Interrupt::cancel_only(&|| ctl.cancelled())).0;
    }
    (planes, has_alpha)
}

fn index_layer_ids_and_text(doc: &Document, ex: &mut Ex) {
    // Assign layer ids up front (stable across round trips): keep unique
    // `psd_id`s, number the rest after them; dividers come after all.
    {
        let walk = doc.walk();
        let mut used = std::collections::HashSet::new();
        for (_, _, l) in &walk {
            if let Some(id) = l.psd_id
                && used.insert(id)
            {
                ex.layer_ids.insert(l.id, id);
            }
        }
        let mut next = 1u32;
        for (_, _, l) in &walk {
            if let std::collections::hash_map::Entry::Vacant(e) = ex.layer_ids.entry(l.id) {
                while used.contains(&next) {
                    next += 1;
                }
                used.insert(next);
                e.insert(next);
            }
        }
        ex.next_id = used.iter().copied().max().unwrap_or(0);
        ex.used_ids = used;
        // Text objects' number in the generated `Txt2` (#1348): a preserved `TySh` keeps the
        // number it already names — its `Txt2` object must sit at that slot for `apply_txt2` —
        // and new layers get numbers past every kept one and every object in the file's `Txt2`.
        // Kept numbers are claimed first: a layer added below others must not take a number a
        // layer above it keeps, or each of those would be renumbered onto its neighbour's object
        // and take its style (#7773); nor may it land on a deleted layer's object.
        let mut used_txt = std::collections::HashSet::new();
        ex.text_index = Default::default();
        let mut fresh = Vec::new();
        for (_, _, l) in &walk {
            let LayerContent::Text(t) = &l.content else {
                continue;
            };
            match t.psd_raw.as_deref().and_then(|d| photocraft_text::psd::text_index(d)) {
                Some(i) if used_txt.insert(i) => {
                    ex.text_index.insert(l.id, i);
                }
                _ => fresh.push(l.id),
            }
        }
        let objects =
            doc.metadata.psd_global_blocks.iter().find(|(_, key, _)| key == b"Txt2").map_or(0, |(_, _, d)| photocraft_text::psd::txt2_object_count(d));
        let mut next_txt = used_txt.iter().max().map_or(0, |m| m + 1).max(i32::try_from(objects).unwrap_or(i32::MAX));
        if next_txt.saturating_add(fresh.len() as i32) > photocraft_text::psd::MAX_TEXT_INDEX + 1 {
            next_txt = 0;
        }
        for id in fresh {
            while used_txt.contains(&next_txt) {
                next_txt += 1;
            }
            used_txt.insert(next_txt);
            ex.text_index.insert(id, next_txt);
        }
    }
}

/// PSD mask parameters for a user mask's and a vector mask's (density, feather); `None` when
/// every value is the default (density 1, feather 0).
pub(super) fn mask_parameters(user: Option<(f32, f32)>, vector: Option<(f32, f32)>) -> Option<MaskParameters> {
    let density = |v: Option<(f32, f32)>| v.and_then(|(d, _)| (d < 1.0).then(|| q255(d)));
    let feather = |v: Option<(f32, f32)>| v.and_then(|(_, f)| (f.is_finite() && f > 0.0).then_some(f64::from(f)));
    let p =
        MaskParameters { flags: 0, user_density: density(user), user_feather: feather(user), vector_density: density(vector), vector_feather: feather(vector) };
    let flags = u8::from(p.user_density.is_some())
        | u8::from(p.user_feather.is_some()) << 1
        | u8::from(p.vector_density.is_some()) << 2
        | u8::from(p.vector_feather.is_some()) << 3;
    (flags != 0).then_some(MaskParameters { flags, ..p })
}
