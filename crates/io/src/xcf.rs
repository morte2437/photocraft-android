//! GIMP XCF import. `craft-xcf` owns XCF parsing, decompression and pixel decoding; this module
//! maps its neutral document representation into PhotoCraft's model.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use craft_xcf::{BaseType, Orientation, Pixels, TreeNode};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, LabelColor, Layer, LayerContent, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

use crate::ora::Warnings;
use crate::{ImportResult, IoError};

type Result<T> = std::result::Result<T, IoError>;

#[cfg(test)]
pub(crate) const MAX_SIDE: u32 = craft_xcf::MAX_SIDE;
pub(crate) const MAX_PIXEL_BYTES: usize = craft_xcf::MAX_PIXEL_BYTES;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Xcf(message.into())
}

fn map_error(error: craft_xcf::Error) -> IoError {
    match error {
        craft_xcf::Error::Invalid(message) => invalid(message),
        craft_xcf::Error::Cancelled => IoError::Cancelled,
    }
}

/// `true` for a gzip stream named `.xcf.gz` / `.xcfgz`.
pub(crate) fn is_gzipped_xcf(name: &str, bytes: &[u8]) -> bool {
    craft_xcf::is_gzipped_xcf(name, bytes)
}

/// Inflates a gzipped XCF with craft-xcf's size bound.
pub(crate) fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    craft_xcf::gunzip(bytes).map_err(map_error)
}

fn blend_mode(mode: u32) -> (BlendMode, Option<&'static str>) {
    use BlendMode as B;
    match mode {
        0 | 28 => (B::Normal, None),
        1 => (B::Dissolve, None),
        2 | 29 => (B::Normal, Some("Behind")),
        3 | 30 => (B::Multiply, None),
        4 | 31 => (B::Screen, None),
        5 | 19 => (B::SoftLight, Some("GIMP's legacy Soft light")),
        45 => (B::SoftLight, Some("GIMP's Soft light")),
        23 => (B::Overlay, None),
        6 | 32 => (B::Difference, None),
        7 | 33 => (B::LinearDodge, None),
        8 | 34 => (B::Subtract, None),
        9 | 35 => (B::Darken, None),
        10 | 36 => (B::Lighten, None),
        11 | 37 => (B::Hue, Some("HSV Hue")),
        12 | 38 => (B::Saturation, Some("HSV Saturation")),
        13 | 39 => (B::Color, Some("HSL Color")),
        14 | 40 => (B::Luminosity, Some("HSV Value")),
        15 | 41 => (B::Divide, None),
        16 | 42 => (B::ColorDodge, None),
        17 | 43 => (B::ColorBurn, None),
        18 | 44 => (B::HardLight, None),
        20 | 46 => (B::Normal, Some("Grain extract")),
        21 | 47 => (B::Normal, Some("Grain merge")),
        22 | 57 => (B::Normal, Some("Color erase")),
        24 => (B::Hue, Some("LCH Hue")),
        25 => (B::Saturation, Some("LCH Chroma")),
        26 => (B::Color, Some("LCH Color")),
        27 => (B::Luminosity, Some("LCH Lightness")),
        48 => (B::VividLight, None),
        49 => (B::PinLight, None),
        50 => (B::LinearLight, None),
        51 => (B::HardMix, None),
        52 => (B::Exclusion, None),
        53 => (B::LinearBurn, None),
        54 => (B::DarkerColor, Some("Luma darken only")),
        55 => (B::LighterColor, Some("Luma lighten only")),
        56 => (B::Luminosity, Some("Luminance")),
        58 => (B::Normal, Some("Erase")),
        59 => (B::Normal, Some("Merge")),
        60 => (B::Normal, Some("Split")),
        61 => (B::PassThrough, None),
        _ => (B::Normal, Some("an unknown mode")),
    }
}

fn label(tag: u32) -> LabelColor {
    match tag {
        1 => LabelColor::Blue,
        2 => LabelColor::Green,
        3 => LabelColor::Yellow,
        4 => LabelColor::Orange,
        6 => LabelColor::Red,
        7 => LabelColor::Violet,
        8 => LabelColor::Gray,
        _ => LabelColor::None,
    }
}

fn write_pixels(surface: &mut Surface, pixels: &Pixels, x: i32, y: i32, budget: &mut usize) -> Result<()> {
    let (w, h) = (
        i32::try_from(pixels.width).map_err(|_| invalid("drawable width is out of range"))?,
        i32::try_from(pixels.height).map_err(|_| invalid("drawable height is out of range"))?,
    );
    let rect = Rect::new(x, y, x.checked_add(w).ok_or_else(|| invalid("offset overflow"))?, y.checked_add(h).ok_or_else(|| invalid("offset overflow"))?);
    if surface.channels() != pixels.channels || pixels.data.len() != pixels.width as usize * pixels.height as usize * pixels.channels {
        return Err(invalid("pixel data does not match the drawable"));
    }
    let bytes = usize::try_from(photocraft_raster::tile_storage_bytes(rect, &surface.format())).unwrap_or(usize::MAX);
    if bytes > *budget {
        return Err(invalid("the layers exceed the 1 GiB pixel limit"));
    }
    let before = surface.tile_count();
    surface.write_region(rect, &pixels.data);
    surface.prune();
    let added = surface.tile_count().saturating_sub(before) as u64;
    *budget = budget.saturating_sub(usize::try_from(photocraft_raster::stored_tiles_bytes(added, &surface.format())).unwrap_or(usize::MAX));
    Ok(())
}

struct Mapper<'a, 'c> {
    xcf: &'a craft_xcf::Xcf<'a>,
    ctl: &'c Interrupt<'c>,
    fmt: PixelFormat,
    mask_fmt: PixelFormat,
    budget: usize,
    clipped_to_backdrop: usize,
    linear_compositing: bool,
    warnings: Warnings,
}

impl Mapper<'_, '_> {
    fn pixels(&mut self, index: usize) -> Result<Option<Pixels>> {
        self.xcf.layer_pixels(index, &mut self.budget, &|| !self.ctl.cancelled()).map_err(map_error)
    }

    fn layer(&mut self, index: usize) -> Result<(Layer, bool)> {
        self.ctl.check().map_err(|_| IoError::Cancelled)?;
        let image = self.xcf.image();
        let source = image.layers.get(index).ok_or_else(|| invalid("tree references a missing layer"))?;
        let name = source.name.as_str();
        if source.floating_selection {
            self.warnings.layer(name, "is a floating selection and was not imported (anchor it in GIMP first)");
        }
        let (blend, note) = blend_mode(source.mode);
        if let Some(note) = note {
            self.warnings.layer(
                name,
                &format!(
                    "uses GIMP's {note} mode, which opens as {}",
                    if blend == BlendMode::Normal { "Normal".to_string() } else { format!("Photoshop's {blend:?}") }
                ),
            );
        }
        match source.composite_mode.unsigned_abs() {
            0 | 1 => {}
            2 => self.clipped_to_backdrop += 1,
            3 => self.warnings.layer(name, "composites with Clip to layer, which is not reproduced"),
            4 => self.warnings.layer(name, "composites with Intersection, which is not reproduced"),
            _ => {}
        }
        if source.text {
            self.warnings.layer(name, "is a text layer and opens as pixels");
        }
        if source.vector {
            self.warnings.layer(name, "is a vector layer and opens as pixels");
        }
        if source.link {
            self.warnings.layer(name, "is a link layer and opens as pixels");
        }
        if source.effects > 0 {
            self.warnings.layer(name, &format!("has {} layer effect(s) (non-destructive filters) that are not applied", source.effects));
        }

        let mut layer = if source.group {
            let mut group = Layer::group(name, Vec::new());
            if let LayerContent::Group(content) = &mut group.content {
                content.expanded = source.expanded;
            }
            group.blend = blend;
            group
        } else {
            let mut raster = Layer::raster(name, self.fmt);
            raster.blend = if blend == BlendMode::PassThrough { BlendMode::Normal } else { blend };
            raster
        };
        layer.opacity = source.opacity;
        layer.visible = source.visible;
        layer.locks.transparency = source.lock_alpha;
        layer.locks.pixels = source.lock_content;
        layer.locks.position = source.lock_position;
        layer.label = label(source.color_tag);
        layer.clipped = source.clipped;

        if !source.group && source.has_pixels() {
            let mut pixels = self.pixels(index)?.ok_or_else(|| invalid("layer pixels are missing"))?;
            if !source.has_alpha() {
                let n = pixels.channels;
                let mut data = Vec::with_capacity(pixels.data.len() / n * (n + 1));
                for p in pixels.data.chunks_exact(n) {
                    data.extend_from_slice(p);
                    data.push(1.0);
                }
                pixels = Pixels { width: pixels.width, height: pixels.height, channels: n + 1, data };
                layer.locks.transparency = true;
            }
            let mut surface = Surface::new(self.fmt);
            write_pixels(&mut surface, &pixels, source.x, source.y, &mut self.budget)?;
            layer.content = LayerContent::Raster(surface);
        }
        if let Some((_, pixels)) = self.xcf.layer_mask(index, &mut self.budget, &|| !self.ctl.cancelled()).map_err(map_error)? {
            let mut surface = Surface::new(self.mask_fmt);
            write_pixels(&mut surface, &pixels, source.x, source.y, &mut self.budget)?;
            layer.mask = Some(LayerMask { surface, enabled: source.apply_mask, linked: true, density: 1.0, feather: 0.0 });
        }
        let linear = source.composite_space.unsigned_abs() == 1 || source.blend_space.unsigned_abs() == 1;
        self.linear_compositing |= linear;
        Ok((layer, linear))
    }

    fn build(&mut self, tree: &[TreeNode]) -> Result<Vec<Layer>> {
        let mut out = Vec::with_capacity(tree.len());
        for node in tree.iter().rev() {
            let (mut layer, _) = self.layer(node.layer)?;
            if !node.children.is_empty() {
                let children = self.build(&node.children)?;
                if let LayerContent::Group(group) = &mut layer.content {
                    group.children = children;
                }
            }
            out.push(layer);
        }
        Ok(out)
    }
}

pub(crate) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult> {
    let xcf = craft_xcf::Xcf::parse_with(bytes, &|| !ctl.cancelled()).map_err(map_error)?;
    let image = xcf.image();
    let mode = if image.base_type == BaseType::Gray { ColorMode::Grayscale } else { ColorMode::Rgb };
    let mode = if image.base_type == BaseType::Indexed { ColorMode::Rgb } else { mode };
    let depth = match (image.precision.float, image.precision.bytes) {
        (false, 1) => SampleType::U8,
        (false, 4) => SampleType::U16,
        (false, _) => SampleType::U16,
        (true, _) => SampleType::F32,
    };
    let mut warnings = Warnings::default();
    if image.version > craft_xcf::KNOWN_VERSION {
        warnings.note(&format!("XCF version {} is newer than this reader (version {}); unknown parts were skipped", image.version, craft_xcf::KNOWN_VERSION));
    }
    if image.base_type == BaseType::Indexed {
        warnings.note("the indexed image opens as RGB");
    }
    if !image.precision.float && image.precision.bytes == 4 {
        warnings.note("32-bit integer samples open as 16-bit");
    }
    for source in &image.layers {
        if source.floating_selection {
            warnings.layer(&source.name, "is a floating selection and was not imported (anchor it in GIMP first)");
        }
    }
    let fmt = PixelFormat::new(mode, depth, true);
    let mask_fmt = PixelFormat::new(ColorMode::Grayscale, depth, false);
    let mut mapper = Mapper { xcf: &xcf, ctl, fmt, mask_fmt, budget: MAX_PIXEL_BYTES, clipped_to_backdrop: 0, linear_compositing: false, warnings };
    let tree = image.tree();
    for &index in &tree.too_deep {
        if let Some(layer) = image.layers.get(index) {
            mapper.warnings.layer(&layer.name, "is nested deeper than the reader allows and was moved to the top level");
        }
    }
    for &index in &tree.misplaced {
        if let Some(layer) = image.layers.get(index) {
            mapper.warnings.layer(&layer.name, "has an item path that leads to no group; opened at the top level");
        }
    }
    let layers = mapper.build(&tree.roots)?;

    let mut channels = Vec::new();
    let mut selection = None;
    for index in 0..image.channel_count {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let (channel, pixels) = xcf.channel(index, &mut mapper.budget, &|| !ctl.cancelled()).map_err(map_error)?;
        let mut surface = Surface::new(mask_fmt);
        write_pixels(&mut surface, &pixels, 0, 0, &mut mapper.budget)?;
        if channel.selection {
            selection = Some(surface);
        } else {
            channels.push(AlphaChannel::new(channel.name, surface));
        }
    }

    let mut document = Document::new(name, Size::new(image.width, image.height), mode, depth);
    document.layers = layers;
    document.channels = channels;
    document.selection = selection;
    if let Some((x, y)) = image.resolution.filter(|(x, _)| *x > 0.0 && *x < 1.0e6) {
        document.resolution_dpi = x;
        if let Some(warning) = crate::unequal_resolution_warning(f64::from(x), f64::from(y)) {
            mapper.warnings.note(&warning);
        }
    }
    for guide in &image.guides {
        if guide.position < 0 && image.version < 15 {
            continue;
        }
        match guide.orientation {
            Orientation::Horizontal => document.guides.horizontal.push(f64::from(guide.position)),
            Orientation::Vertical => document.guides.vertical.push(f64::from(guide.position)),
        }
    }
    document.icc_profile = match (image.precision.linear, image.icc_profile.clone(), mode) {
        (true, icc, ColorMode::Grayscale) => {
            if icc.is_some() {
                mapper.warnings.note("the linear-light image's own profile was replaced by linear gray");
            }
            Some(craft_cms::builtin::linear_gray().to_bytes())
        }
        (true, icc, _) => {
            if icc.is_some() {
                mapper.warnings.note("the linear-light image's own profile was replaced by linear sRGB");
            }
            Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes())
        }
        (false, Some(icc), _) => Some(Arc::new(icc)),
        (false, None, _) => None,
    };
    if let Some(comment) = image.comment.as_deref().filter(|c| !c.is_empty()) {
        document.metadata.text.push(("Comment".into(), comment.to_string()));
    }
    if image.path_count > 0 {
        mapper.warnings.note(&format!("{} path(s) were not imported", image.path_count));
    }
    if mapper.clipped_to_backdrop > 0 && document.layers.first().is_some_and(|l| !l.locks.transparency || l.is_group()) {
        mapper.warnings.note(&format!(
            "{} layer(s) use a mode GIMP clips to the layers beneath; where those are transparent GIMP shows nothing and PhotoCraft shows the layer",
            mapper.clipped_to_backdrop
        ));
    }
    if !image.precision.linear && mapper.linear_compositing {
        mapper
            .warnings
            .note("GIMP composites this image in linear light; PhotoCraft composites in gamma-encoded RGB, so blends and soft edges differ slightly");
    }
    if document.layers.is_empty() {
        mapper.warnings.note("the XCF file has no layers");
    }
    ctl.progress(1.0);
    Ok(ImportResult { document, warnings: mapper.warnings.finish(), source_read_only: true, preview_only: false })
}
