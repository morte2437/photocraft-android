//! Krita import. `craft-kra` owns the archive, XML and tile formats; this module maps its
//! neutral document representation into PhotoCraft's layer model.

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use craft_kra::tiles::{Space, TileFile};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask};
use photocraft_geom::{Size, TILE_SIZE};
use photocraft_raster::{Interrupt, Surface};

use crate::ora::{MAX_NODES, MAX_PIXEL_BYTES, Warnings, krita_blend};
use crate::{ImportResult, IoError};

type Result<T> = std::result::Result<T, IoError>;

/// The `mimetype` entry of a Krita archive.
#[cfg(test)]
const MIMETYPE: &[u8] = craft_kra::MIMETYPE;
pub(crate) const EXTENSIONS: [&str; 2] = craft_kra::EXTENSIONS;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Kra(message.into())
}

fn map_error(error: craft_kra::Error) -> IoError {
    match error {
        craft_kra::Error::Invalid(message) => invalid(message),
        craft_kra::Error::Unsupported(message) => IoError::Unsupported(message),
    }
}

/// `true` for a Krita archive (by its `mimetype` entry).
#[cfg(test)]
pub(crate) fn is_kra(bytes: &[u8]) -> bool {
    craft_kra::is_kra(bytes)
}

fn model(space: Space) -> (ColorMode, SampleType) {
    match space {
        Space::Rgba8 => (ColorMode::Rgb, SampleType::U8),
        Space::Rgba16 => (ColorMode::Rgb, SampleType::U16),
        Space::Gray8 | Space::Alpha8 => (ColorMode::Grayscale, SampleType::U8),
        Space::Gray16 => (ColorMode::Grayscale, SampleType::U16),
    }
}

struct Reader<'a, 'c> {
    archive: craft_kra::Archive<'a>,
    ctl: &'c Interrupt<'c>,
    fmt: PixelFormat,
    nodes: usize,
    budget: usize,
    warnings: Warnings,
}

impl Reader<'_, '_> {
    /// Reserve for decoded pixels and the raster backing tiles they occupy.
    fn decoded_bytes(file: &TileFile<'_>, bpp: usize, dx: i32, dy: i32) -> Result<usize> {
        let overflow = || invalid("tile size overflow");
        let logical =
            file.tiles().len().checked_mul(file.width).and_then(|n| n.checked_mul(file.height)).and_then(|n| n.checked_mul(bpp)).ok_or_else(overflow)?;
        let (w, h) = (i32::try_from(file.width).map_err(|_| overflow())?, i32::try_from(file.height).map_err(|_| overflow())?);
        let mut backing = HashSet::new();
        let side = TILE_SIZE;
        for tile in file.tiles() {
            let (x, y) = (tile.x.checked_add(dx).ok_or_else(overflow)?, tile.y.checked_add(dy).ok_or_else(overflow)?);
            let (x1, y1) = (x.checked_add(w - 1).ok_or_else(overflow)?, y.checked_add(h - 1).ok_or_else(overflow)?);
            for col in x.div_euclid(side)..=x1.div_euclid(side) {
                for row in y.div_euclid(side)..=y1.div_euclid(side) {
                    backing.insert((col, row));
                }
            }
        }
        let tile_bytes = (side as usize).checked_mul(side as usize).and_then(|n| n.checked_mul(bpp)).ok_or_else(overflow)?;
        Ok(logical.max(backing.len().checked_mul(tile_bytes).ok_or_else(overflow)?))
    }

    fn surface(&mut self, name: &str, space: Space, fmt: PixelFormat, dx: i32, dy: i32) -> Result<Option<Surface>> {
        let Some(bytes) = self.archive.layer_file(name).map_err(map_error)? else { return Ok(None) };
        let file = TileFile::parse(&bytes).map_err(map_error)?;
        if file.pixel_size != space.pixel_size() {
            return Err(invalid(format!("{name}: pixel size {} does not match its colour space", file.pixel_size)));
        }
        self.budget = self
            .budget
            .checked_sub(Self::decoded_bytes(&file, fmt.bytes_per_pixel(), dx, dy)?)
            .ok_or_else(|| invalid("decoded layers exceed the 1 GiB limit"))?;

        let mut surface = match self.archive.default_pixel(name, space).map_err(map_error)? {
            Some(rgba) if rgba.iter().any(|v| *v != 0.0) => Surface::with_default(fmt, &photocraft_raster::from_rgba(&fmt, rgba)),
            _ => Surface::new(fmt),
        };
        let n = file.width.checked_mul(file.height).ok_or_else(|| invalid("tile size overflow"))?;
        let expected = n.checked_mul(file.pixel_size).ok_or_else(|| invalid("tile size overflow"))?;
        let mut source = Vec::new();
        let mut converted = Vec::new();
        let native = PixelFormat::new(model(space).0, model(space).1, true);
        for tile in file.tiles() {
            self.ctl.check().map_err(|_| IoError::Cancelled)?;
            file.decode(tile, &mut source).map_err(map_error)?;
            if source.len() != expected {
                return Err(invalid("decoded tile has an unexpected size"));
            }
            if fmt == native {
                space.samples(&source, &mut converted);
            } else {
                converted.clear();
                let mut encoded = vec![0; fmt.bytes_per_pixel()];
                for px in source.chunks_exact(space.pixel_size()) {
                    let rgba = space.rgba(px);
                    let samples = photocraft_raster::from_rgba(&fmt, rgba);
                    photocraft_raster::encode_pixel(&fmt, &samples, &mut encoded);
                    converted.extend_from_slice(&encoded);
                }
            }
            let (x, y) = (
                tile.x.checked_add(dx).ok_or_else(|| invalid("tile offset overflow"))?,
                tile.y.checked_add(dy).ok_or_else(|| invalid("tile offset overflow"))?,
            );
            let rect = photocraft_geom::Rect::from_xywh(x, y, file.width as u32, file.height as u32);
            if converted.len() != n * fmt.bytes_per_pixel() || rect.width() as usize != file.width || rect.height() as usize != file.height {
                return Err(invalid("tile does not fit its position"));
            }
            surface.write_interleaved(rect, &converted);
        }
        surface.prune();
        Ok(Some(surface))
    }

    fn props(&mut self, node: &craft_kra::Node, layer: &mut Layer) {
        layer.name = node.name.as_deref().filter(|n| !n.is_empty()).map_or_else(|| format!("Layer {}", self.nodes), str::to_string);
        layer.visible = node.visible;
        layer.opacity = (node.opacity / 255.0).clamp(0.0, 1.0) as f32;
        if node.locked {
            layer.locks.all = true;
        }
        let op = node.composite_op.as_deref().unwrap_or("normal");
        layer.blend = match krita_blend(op) {
            Some((mode, approx)) => {
                if approx {
                    self.warnings.note(&format!("Krita's {op} blend mode is approximated by {}", mode.label()));
                }
                mode
            }
            None => {
                self.warnings.layer(&layer.name, &format!("Krita's {op} blend mode is not supported; it opens as Normal"));
                BlendMode::Normal
            }
        };
        if node.channel_flags.as_deref().is_some_and(|f| f.contains('0')) {
            self.warnings.layer(&layer.name, "channel locks (Krita's per-channel blending) are ignored");
        }
        if node.inherit_alpha {
            self.warnings.layer(&layer.name, "Inherit Alpha is not supported; these layers blend normally");
        }
        if node.keyframes.as_deref().is_some_and(|k| !k.is_empty()) {
            self.warnings.layer(&layer.name, "animated layers open with the frame Krita saved in the layer");
        }
    }

    fn masks(&mut self, node: &craft_kra::Node, layer: &mut Layer, dx: i32, dy: i32) -> Result<()> {
        for mask in &node.masks {
            self.ctl.check().map_err(|_| IoError::Cancelled)?;
            match mask.node_type.as_str() {
                "transparencymask" => {}
                "selectionmask" => continue,
                other => {
                    let what = match other {
                        "filtermask" => "filter masks",
                        "transformmask" => "transform masks",
                        "colorizemask" => "colorize masks",
                        _ => "unknown masks",
                    };
                    self.warnings.layer(&layer.name, &format!("{what} are not supported and were left out"));
                    continue;
                }
            }
            if layer.mask.is_some() {
                self.warnings.layer(&layer.name, "only the first transparency mask of a layer is kept");
                continue;
            }
            let Some(file) = mask.filename.as_deref() else { continue };
            let (mx, my) = (dx.saturating_add(mask.x), dy.saturating_add(mask.y));
            match self.surface(&format!("{file}.pixelselection"), Space::Alpha8, PixelFormat::GRAY8, mx, my)? {
                Some(surface) => layer.mask = Some(LayerMask { surface, enabled: mask.visible, ..LayerMask::reveal_all() }),
                None => self.warnings.layer(&layer.name, "a transparency mask's pixels are missing from the file"),
            }
        }
        Ok(())
    }

    fn layers(&mut self, nodes: &[craft_kra::Node], depth: usize, dx: i32, dy: i32) -> Result<Vec<Layer>> {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(IoError::Unsupported(format!("layer groups nested deeper than {}", photocraft_doc::MAX_GROUP_DEPTH)));
        }
        let mut out = Vec::new();
        for child in nodes {
            self.ctl.check().map_err(|_| IoError::Cancelled)?;
            self.nodes += 1;
            if self.nodes > MAX_NODES {
                return Err(invalid(format!("more than {MAX_NODES} layers")));
            }
            let (x, y) = (dx.saturating_add(child.x), dy.saturating_add(child.y));
            let mut layer = match child.node_type.as_str() {
                "paintlayer" => {
                    let mut layer = Layer::raster("", self.fmt);
                    self.props(child, &mut layer);
                    let space = child.colorspace_name.as_deref().and_then(Space::from_name);
                    match (space, child.filename.as_deref()) {
                        (Some(space), Some(file)) => match self.surface(file, space, self.fmt, x, y)? {
                            Some(surface) => layer.content = LayerContent::Raster(surface),
                            None => self.warnings.layer(&layer.name, "the layer's pixels are missing from the file; it opens empty"),
                        },
                        (None, _) => {
                            let cs = child.colorspace_name.as_deref().unwrap_or("?");
                            self.warnings.layer(&layer.name, &format!("layers in Krita's {cs} colour space are not supported and were left out"));
                            continue;
                        }
                        (_, None) => self.warnings.layer(&layer.name, "the layer names no pixel file; it opens empty"),
                    }
                    layer
                }
                "grouplayer" => {
                    let mut layer = Layer::group("", Vec::new());
                    self.props(child, &mut layer);
                    if child.passthrough {
                        if layer.blend != BlendMode::Normal {
                            self.warnings.layer(&layer.name, "a pass-through group's blend mode is ignored, as in Krita");
                        }
                        layer.blend = BlendMode::PassThrough;
                    }
                    let kids = self.layers(&child.layers, depth + 1, dx, dy)?;
                    if let LayerContent::Group(group) = &mut layer.content {
                        group.children = kids;
                        group.expanded = !child.collapsed;
                    }
                    layer
                }
                other => {
                    let name = child.name.as_deref().unwrap_or(other);
                    let what = match other {
                        "shapelayer" => "vector layers",
                        "adjustmentlayer" => "filter layers",
                        "generatorlayer" => "fill layers",
                        "filelayer" => "file layers",
                        "clonelayer" => "clone layers",
                        _ => "unknown layer kinds",
                    };
                    self.warnings.layer(name, &format!("Krita {what} are not supported yet and were left out"));
                    continue;
                }
            };
            self.masks(child, &mut layer, x, y)?;
            out.push(layer);
        }
        out.reverse();
        Ok(out)
    }
}

/// Krita's merged image as a one-layer stand-in, for documents we can't read layer by layer.
fn preview(name: &str, archive: &mut craft_kra::Archive<'_>, why: String) -> Result<ImportResult> {
    let png = archive.merged_image().map_err(map_error)?.ok_or_else(|| invalid(format!("{why}, and the merged image is missing")))?;
    let mut result = crate::flat::import_flat(name, &png)?;
    result.document.name = name.to_string();
    result.warnings.insert(0, format!("{why}; the file opens as Krita's merged image, without its layers"));
    result.source_read_only = true;
    result.preview_only = true;
    Ok(result)
}

pub(crate) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult> {
    let mut archive = craft_kra::Archive::open(bytes).map_err(map_error)?;
    let image = archive.image().clone();
    let Some(space) = Space::from_name(&image.colorspace_name) else {
        return preview(name, &mut archive, format!("Krita's {} colour space is not supported yet", image.colorspace_name));
    };
    let (mode, depth) = model(space);
    let mut reader = Reader { archive, ctl, fmt: PixelFormat::new(mode, depth, true), nodes: 0, budget: MAX_PIXEL_BYTES, warnings: Warnings::default() };
    let layers = reader.layers(&image.layers, 1, 0, 0)?;
    ctl.progress(0.9);
    let mut document = Document::new(name, Size::new(image.width, image.height), mode, depth);
    document.layers = layers;
    if let Some(profile) = reader.archive.icc_profile().map_err(map_error)? {
        document.icc_profile = Some(std::sync::Arc::new(profile));
    }
    if image.x_res > 0.0 && image.x_res < 1.0e6 {
        document.resolution_dpi = image.x_res as f32;
        if let Some(warning) = crate::unequal_resolution_warning(image.x_res, image.y_res) {
            reader.warnings.note(&warning);
        }
    }
    if document.layers.is_empty() {
        reader.warnings.note("no layers of the Krita document could be imported");
    }
    if image.assistants || image.reference_images {
        reader.warnings.note("drawing assistants and reference images were left out");
    }
    Ok(ImportResult { document, warnings: reader.warnings.finish(), source_read_only: true, preview_only: false })
}
