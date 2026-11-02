//! The command registry. Ids follow Photoshop's menu structure (see the parity checklist).

use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Fill, Layer, LayerContent, LayerId, LayerMask, Size};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::Surface;
use serde_json::{Map, Value, json};

use crate::{EngineError, Result, Session, inspect, pixels};

type Run = fn(&mut Session, &Value) -> Result<Value>;
type Enabled = fn(&Session) -> std::result::Result<(), String>;

/// Metadata + implementation for one command.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu placement, e.g. `["Layer", "New"]`. Empty = not in menus.
    pub menu: &'static [&'static str],
    /// Default shortcut in egui-style notation (`Cmd+Shift+N`), mapped per platform by the UI.
    pub shortcut: Option<&'static str>,
    /// Human/agent-readable parameter description (JSON-ish).
    pub params: &'static str,
    pub enabled: Enabled,
    pub run: Run,
    /// Record in the session journal (false for queries).
    pub journal: bool,
}

/// Image › Image Rotation by a right angle or a flip (pixels, vectors, guides, … all move).
fn turn(s: &mut Session, label: &str, t: crate::canvas_geom::Turn) -> Result<Value> {
    if let Some(d) = s.active() {
        let a = t.affine(f64::from(d.doc.size.width), f64::from(d.doc.size.height));
        // Sampler points are pixel indices, not pixel edges: map the pixel's centre, so the point
        // lands on the same pixel `Turn::pixel_map` moves the pixels to (`w - 1 - x`).
        let half = photocraft_geom::Affine::translate(0.5, 0.5);
        s.set_canvas_map(a, half.then(&a).then(&photocraft_geom::Affine::translate(-0.5, -0.5)));
    }
    s.edit(label, |doc, _| {
        crate::canvas_geom::turn_canvas(doc, t);
        Ok(())
    })?;
    Ok(Value::Null)
}

macro_rules! cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub mod actions_cmds;
pub mod actions_edit_cmds;
pub mod adjust_cmds;
mod adjust_params;
pub mod align_cmds;
pub mod analysis_cmds;
pub mod artboard_cmds;
pub mod atn_cmds;
pub mod automate_cmds;
pub mod background_cmds;
pub mod brush_cmds;
pub mod brush_key_cmds;
pub mod brush_preset_cmds;
pub mod channel_cmds;
pub mod color_cmds;
pub mod color_to_alpha_cmds;
pub mod comps_cmds;
pub mod contour_lib_cmds;
pub mod cutout_cmds;
pub mod distort_cmds;
pub mod document_preset_cmds;
pub mod edit_cmds;
pub mod edit_menu_cmds;
mod enabled;
pub mod eraser_cmds;
pub mod export_preset_cmds;
pub mod exr_cmds;
pub mod extra_cmds;
pub mod face_cmds;
pub mod file_cmds;
pub mod fill_cmds;
pub mod fill_key_cmds;
pub mod float_cmds;
pub mod font_library_cmds;
pub(super) mod frame_cmds;
pub mod fx_view_cmds;
pub mod fx_visibility_cmds;
pub mod gallery_cmds;
pub mod gradient_fill_cmds;
pub mod group_view_cmds;
pub mod history_cmds;
pub mod history_menu_cmds;
pub mod history_options_cmds;
pub mod history_select_cmds;
pub mod history_source_cmds;
pub mod image_cmds;
pub mod layer_copy_cmds;
pub mod layer_label_cmds;
pub mod layer_list_cmds;
pub mod layer_mask_props_cmds;
pub mod layer_menu_cmds;
pub mod layer_multi_cmds;
pub mod layer_nav_cmds;
mod layer_params;
pub mod layer_xmp_cmds;
pub mod lens_cmds;
pub mod magnetic_cmds;
pub mod mask_view_cmds;
pub(super) mod migrate_cmds;
pub mod mode_cmds;
pub mod multichannel_cmds;
pub mod notes_cmds;
mod ops;
pub mod paint_cmds;
mod params;
pub(super) mod path_edit_cmds;
pub mod pattern_cmds;
pub mod pattern_fill_cmds;
pub mod perspective_crop_cmds;
pub mod photo_cmds;
pub mod pick_cmds;
pub mod plugin_cmds;
pub mod preset_import_cmds;
pub mod print_cmds;
pub mod rasterize_style_cmds;
pub mod redeye_cmds;
pub mod render_cmds;
pub mod retouch_cmds;
pub mod sample_cmds;
pub mod sampler_cmds;
pub mod select_extra_cmds;
pub mod selection_cmds;
pub mod slice_cmds;
pub mod smart_cmds;
pub mod smart_svg_cmds;
pub mod smartselect_cmds;
pub mod solid_fill_cmds;
pub mod stamp_cmds;
pub mod style_asl_cmds;
pub mod swatch_cmds;
pub mod symmetry_cmds;
pub(super) mod timeline_cmds;
pub mod transform_cmds;
pub(super) mod trap_cmds;
pub mod type_area_cmds;
pub mod type_caret_cmds;
pub mod type_cmds;
pub mod type_extra_cmds;
pub mod type_mask_cmds;
pub mod type_spell_cmds;
pub mod type_styles_cmds;
pub mod updates_cmds;
pub(super) mod variables_cmds;
pub mod vector_cmds;
pub(super) mod video_cmds;
pub mod vp_cmds;
pub mod warp_cmds;
pub mod web_cmds;
pub(super) mod wia_cmds;

pub(crate) use adjust_params::inject_kind;
use adjust_params::{adjust_mode, destructive_adjust, doc_mode, gray_tone_params, new_adjustment};
pub use adjust_params::{adjustment_from_params, adjustment_kind};
pub(crate) use enabled::{always, check_all_unlocked, check_pixels_unlocked, has_paintable, is_mask_target, paint_surface};
use enabled::{can_redo, can_undo, has_doc, has_layer, has_pixel_layer, has_pixel_or_channel, has_selection};
pub use layer_params::blend_from_str;
pub(crate) use layer_params::{NEW_LAYER_PATH, explicit_layer, layer_listing, layer_panel_option, layer_param, mask_at_depth, mask_new_layer, selection_mask};
use ops::{arrange, merge_group, set_mask, set_mask_and_deselect};
pub(crate) use ops::{layer_copy, translate_layer};
use params::{bad, set_clipped};
pub(crate) use params::{check_f32_values, cmyk_param, color_param, int, int_i32, parse_hex, u32_id, u32_id_param};

pub fn command_specs() -> &'static [CommandSpec] {
    &registry().specs
}

/// One `command.list` / `engine.commands` row for `c`, given why it can't run now (`None` = it can).
/// `disabledReason` is that message; `needsDocument` is true when the reason is that no document is
/// open (#3664), so an agent can tell "open or create a document first" from a precondition that a
/// document alone won't meet (a selection, an empty clipboard).
pub fn list_entry(s: &Session, c: &CommandSpec, reason: Option<String>) -> Value {
    let needs_document =
        s.active().is_none() && reason.as_deref().is_some_and(|r| ["no document", "no active document", "no open document"].iter().any(|p| r.starts_with(p)));
    let mut entry = json!({
        "id": c.id, "label": c.label, "menu": c.menu, "shortcut": c.shortcut, "params": c.params,
        "enabled": reason.is_none(), "disabledReason": reason, "needsDocument": needs_document,
    });
    if let (Some(k), Some(o)) = (search_keywords(c.id), entry.as_object_mut()) {
        o.insert("keywords".into(), json!(k));
    }
    entry
}

/// Other names a command is looked for by, beyond its id and label: the Photoshop tools and
/// options it stands for (#5060). `select.rect` is also the Elliptical and Single Row/Column
/// Marquee, which an agent searches for as "ellipse" or "marquee".
pub(crate) const SEARCH_KEYWORDS: &[(&str, &str)] =
    &[("select.rect", "Rectangular Marquee, Elliptical Marquee (\"ellipse\":true), ellipse, oval, circle, Single Row/Column Marquee (\"line\")")];

/// [`SEARCH_KEYWORDS`] of command `id`, if it has any.
pub fn search_keywords(id: &str) -> Option<&'static str> {
    SEARCH_KEYWORDS.iter().find(|(k, _)| *k == id).map(|(_, words)| *words)
}

/// Does a [`list_entry`] row match a command-list filter (`commands --filter`, `engine.commands`,
/// MCP `command_list`)? Its id, label or keywords contain `needle`, ignoring case.
pub fn list_entry_matches(entry: &Value, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    ["id", "label", "keywords"].iter().any(|k| entry.get(*k).and_then(Value::as_str).is_some_and(|v| v.to_lowercase().contains(&needle)))
}

pub fn find(id: &str) -> Option<&'static CommandSpec> {
    let registry = registry();
    registry.by_id.get(id).and_then(|&index| registry.specs.get(index))
}

struct Registry {
    specs: Vec<CommandSpec>,
    by_id: std::collections::HashMap<&'static str, usize>,
}

impl Registry {
    fn new(specs: Vec<CommandSpec>) -> Self {
        let mut by_id = std::collections::HashMap::with_capacity(specs.len());
        for (index, spec) in specs.iter().enumerate() {
            // Preserve the linear lookup's first-match behavior if an id is duplicated.
            by_id.entry(spec.id).or_insert(index);
        }
        Self { specs, by_id }
    }
}

fn registry() -> &'static Registry {
    static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| Registry::new(build()))
}

// ---------- the table ----------

#[allow(clippy::too_many_lines)]
fn build() -> Vec<CommandSpec> {
    let mut v = vec![
        // File
        cmd!(
            "file.new",
            "New…",
            ["File"],
            Some("Cmd+N"),
            r##"{"width":1..300000=1920,"height":1..300000=1080 (pixel counts are rounded),"mode":"rgb|gray|grayscale|cmyk|lab"="rgb","depth":8|16|32=8,"background":"white|black|backgroundColor|transparent|#rrggbb"="white","backgroundColor":[r,g,b]? (0..1; defaults to toolbox colour),"resolution":1..30000=72 (ppi),"name":str,"artboard":bool=false (one artboard over the canvas, its background the chosen one, holding an empty Layer 1; no Background layer),"profile":"working"|built-in id|path to an .icc file="working" (the colour profile it is tagged with; it must suit the mode)}"##,
            always,
            |s, p| {
                use crate::document_preset_cmds::{MAX_DIMENSION, MAX_RESOLUTION, background_color, color_mode, sample_type};
                crate::document_preset_cmds::validate_new_params(s, p)?;
                // A size given as a float (`512.0`, as JSON from a UI field) is still that size (#254).
                let px = |k: &str, d: u32| match p.get(k) {
                    Some(v) => v
                        .as_u64()
                        .map(|n| n.clamp(1, MAX_DIMENSION as u64) as u32)
                        .or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f.round().clamp(1.0, MAX_DIMENSION as f64) as u32))
                        .unwrap_or(d),
                    None => d,
                };
                let (w, h) = (px("width", 1920), px("height", 1080));
                let mode = color_mode(p.get("mode").and_then(Value::as_str).unwrap_or("rgb")).unwrap_or(ColorMode::Rgb);
                let depth = sample_type(p.get("depth").and_then(Value::as_u64).unwrap_or(8)).unwrap_or(SampleType::U8);
                let name = p.get("name").and_then(Value::as_str).unwrap_or("Untitled").to_string();
                let res = p.get("resolution").and_then(Value::as_f64).unwrap_or(72.0).clamp(1.0, MAX_RESOLUTION) as f32;
                // The background colour; `None` for Transparent.
                let background = match p.get("background").and_then(Value::as_str).unwrap_or("white") {
                    "backgroundColor" => {
                        let bgc = background_color(p, s.tools.background, "file.new")?;
                        Some(Color::rgba(bgc[0], bgc[1], bgc[2], 1.0))
                    }
                    "transparent" => None,
                    "black" => Some(Color::BLACK),
                    other => {
                        let c = parse_hex(other).unwrap_or([1.0; 4]);
                        Some(Color::rgba(c[0], c[1], c[2], c[3]))
                    }
                };
                let mut doc = if p.get("artboard").and_then(Value::as_bool).unwrap_or(false) {
                    crate::artboard_cmds::new_artboard_document(name, Size::new(w, h), mode, depth, background)
                } else if let Some(c) = background {
                    Document::with_background(name, Size::new(w, h), mode, depth, c)
                } else {
                    let mut d = Document::new(name, Size::new(w, h), mode, depth);
                    d.layers.push(Layer::raster("Layer 1", d.pixel_format()));
                    d
                };
                doc.resolution_dpi = res;
                // Photoshop's New Document › Color Profile: tag the document with another profile
                // than the working space (#321). Checked before the document opens.
                if let Some(spec) = p.get("profile").and_then(Value::as_str).filter(|spec| !matches!(*spec, "working" | "none")) {
                    let prof = s.color.resolve(spec, Some(&doc), None)?;
                    if prof.color_space != crate::color_cmds::mode_space(doc.mode) {
                        return Err(EngineError::BadParams {
                            cmd: "file.new".into(),
                            msg: format!("profile `{}` is {:?}; the document is {:?}", prof.description, prof.color_space, doc.mode),
                        });
                    }
                    doc.icc_profile = Some(prof.to_bytes());
                }
                let i = s.add_document(doc, None);
                Ok(json!({ "document": i }))
            }
        ),
        cmd!("file.close", "Close", ["File"], Some("Cmd+W"), r##"{"document":index?}"##, has_doc, |s, p| {
            let i = p.get("document").and_then(Value::as_u64).map(|v| v as usize).or(s.active_index()).ok_or(EngineError::NoDocument)?;
            s.close(i).ok_or(EngineError::NoDocument)?;
            Ok(Value::Null)
        }),
        // Edit
        cmd!("edit.undo", "Undo", ["Edit"], Some("Cmd+Z"), "{}", can_undo, |s, _| Ok(json!(s.undo()))),
        cmd!("edit.redo", "Redo", ["Edit"], Some("Cmd+Shift+Z"), "{}", can_redo, |s, _| Ok(json!(s.redo()))),
        cmd!(
            "edit.fill",
            "Fill…",
            ["Edit"],
            Some("Shift+F5"),
            r##"{"contents":"foreground|background|color|contentAware|pattern|history|black|gray|white"="color","color":"#rrggbb|[r,g,b,a]"=foreground (contents=color),"pattern":id|name (contents=pattern),"scale":%=100,"angle":deg,"state":row?,"snapshot":id? (contents=history: a History panel row as history.setSource, redo rows last, or a snapshot; default the History Brush source),"colorAdaptation":bool=true (contents=contentAware),"mode":"normal|multiply|…"="normal","opacity":0..100=100,"preserveTransparency":bool=false,"target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}?}"##,
            has_pixel_or_channel,
            crate::fill_cmds::fill
        ),
        cmd!(
            "edit.clear",
            "Clear",
            ["Edit"],
            Some("Delete"),
            r##"{"target":"mask"|{"color":i}|{"colors":[i,…]}?=Channels panel target,"layer":id|name?=active} (a targeted mask, channel or Quick Mask is cleared alone)"##,
            has_pixel_or_channel,
            |s, p| {
                let bg = s.tools.background;
                // A targeted alpha channel or Quick Mask is filled instead (#2734).
                let channel = if crate::channel_cmds::is_channel_target(p) {
                    Some(p.clone())
                } else if p.get("target").is_none() {
                    crate::channel_cmds::targeted_channel(s)
                } else {
                    None
                };
                if let Some(t) = channel {
                    s.edit("Clear", |doc, _| crate::channel_cmds::clear_channel(doc, &t, bg))?;
                    return Ok(Value::Null);
                }
                let id = layer_param(s, p)?;
                if is_mask_target(p) {
                    s.edit("Clear Mask", |doc, _| crate::edit_cmds::clear_mask(doc, id, bg, p))?;
                    return Ok(Value::Null);
                }
                // A targeted colour channel clears that channel only (#2720).
                if let Some(colors) = crate::color_channels::target(s, p)? {
                    s.edit("Clear", |doc, _| {
                        for k in colors.indices() {
                            crate::channel_cmds::clear_color_channel(doc, id, k, bg)?;
                        }
                        Ok(())
                    })?;
                    return Ok(Value::Null);
                }
                s.edit("Clear", |doc, _| {
                    let sel = doc.selection.clone();
                    let area = sel.as_ref().map(|m| m.content_bounds()).unwrap_or(doc.bounds());
                    crate::edit_cmds::clear_area(doc, id, area, sel.as_ref(), bg)
                })?;
                Ok(Value::Null)
            }
        ),
        // Select
        cmd!("select.all", "All", ["Select"], Some("Cmd+A"), "{}", has_doc, |s, _| {
            s.edit("Select All", |doc, _| {
                let mut m = Surface::new(photocraft_color::PixelFormat::GRAY8);
                m.fill_rect(doc.bounds(), &[1.0]);
                doc.selection = Some(m);
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!("select.deselect", "Deselect", ["Select"], Some("Cmd+D"), "{}", has_selection, |s, _| {
            // Nothing selected: nothing to do and no history step, as in Photoshop (#6744).
            if s.active().is_none_or(|d| d.doc.selection.is_none()) {
                return Ok(Value::Null);
            }
            s.edit("Deselect", |doc, _| {
                doc.selection = None;
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!("select.inverse", "Inverse", ["Select"], Some("Cmd+Shift+I"), "{}", has_selection, |s, _| {
            s.edit("Inverse", |doc, _| {
                let old = doc.selection.take().unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8));
                doc.selection = Some(photocraft_algo::selection::inverse_surface(&old, doc.bounds()));
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!(
            "select.rect",
            "Rectangular Selection",
            [],
            None,
            r##"{"x":i32,"y":i32,"width":u32,"height":u32,"mode":"replace|add|subtract|intersect"="replace","ellipse":bool=false,"line":"row|column"?,"antiAlias":bool=true,"feather":px=0}"##,
            has_doc,
            |s, p| {
                let get = |k: &str| int_i32("select.rect", p, k).and_then(|v| v.ok_or_else(|| bad("select.rect", format!("missing `{k}`"))));
                let r = Rect::from_xywh(get("x")?, get("y")?, get("width")?.max(0) as u32, get("height")?.max(0) as u32);
                let mode = p.get("mode").and_then(Value::as_str).unwrap_or("replace").to_string();
                let ellipse = p.get("ellipse").and_then(Value::as_bool).unwrap_or(false);
                // The History step is named after the tool: the Single Row / Single Column
                // Marquee send `line` (Photoshop).
                let step = match p.get("line").and_then(Value::as_str) {
                    Some("row") => "Single Row Marquee",
                    Some("column") => "Single Column Marquee",
                    Some(other) => return Err(bad("select.rect", format!("unknown `line` {other:?} (row|column)"))),
                    None if ellipse => "Elliptical Marquee",
                    None => "Rectangular Marquee",
                };
                // Options bar: anti-aliased ellipse edges (4x4 supersampled) and Feather (applied to the new shape only).
                let aa = p.get("antiAlias").and_then(Value::as_bool).unwrap_or(true);
                let feather = p.get("feather").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 1000.0) as f32;
                s.edit(step, |doc, _| {
                    let area = doc.bounds();
                    // A marquee dragged past the canvas stops at its edge (Photoshop); the ellipse
                    // keeps the dragged shape and is only cut there.
                    use photocraft_algo::selection as sel;
                    let shape = if ellipse {
                        sel::ellipse_surface(r, area, aa)
                    } else {
                        let mut s = Surface::new(photocraft_color::PixelFormat::GRAY8);
                        s.fill_rect(r.intersect(&area), &[1.0]);
                        s
                    };
                    if feather > 0.0 {
                        let window = selection_cmds::feather_window(r, area, feather);
                        let m = sel::feather(&sel::mask_from_surface(Some(&shape), window), window.width() as usize, window.height() as usize, feather);
                        doc.selection = selection_cmds::combine_band(doc.selection.as_ref(), &m, window, sel::SelectionMode::parse(&mode));
                        return Ok(());
                    }
                    let old = doc.selection.take();
                    let combined = match (mode.as_str(), old) {
                        ("add", Some(o)) => sel::combine_sparse(&o, &shape, area, |a, b| a.max(b)),
                        ("subtract", Some(o)) => sel::combine_sparse(&o, &shape, area, |a, b| a * (1.0 - b)),
                        ("intersect", Some(o)) => sel::combine_surfaces(&o, &shape, area, |a, b| a.min(b)),
                        // With no existing selection, neither operation can select new pixels.
                        ("subtract" | "intersect", None) => Surface::new(photocraft_color::PixelFormat::GRAY8),
                        _ => shape,
                    };
                    // Sparse Add/Subtract keeps untouched pixels outside the canvas. Only pixels
                    // inside the document count when deciding whether a selection remains.
                    let has_on_canvas_pixels = (combined.default_bytes().first().copied().unwrap_or(0) != 0 && {
                        let canvas_tiles = u64::from(area.width().div_ceil(TILE_SIZE as u32)) * u64::from(area.height().div_ceil(TILE_SIZE as u32));
                        let stored = combined.tiles().filter(|(coord, _)| !coord.rect().intersect(&area).is_empty()).count() as u64;
                        stored < canvas_tiles
                    }) || combined.tiles().any(|(coord, tile)| {
                        let tile_rect = coord.rect();
                        let overlap = tile_rect.intersect(&area);
                        if overlap.is_empty() {
                            return false;
                        }
                        let x0 = (overlap.x0 - tile_rect.x0) as usize;
                        let x1 = (overlap.x1 - tile_rect.x0) as usize;
                        (overlap.y0 - tile_rect.y0..overlap.y1 - tile_rect.y0).any(|y| {
                            let row_start = y as usize * TILE_SIZE as usize;
                            let start = row_start + x0;
                            tile.bytes().get(start..row_start + x1).is_some_and(|row| row.iter().any(|&v| v != 0))
                        })
                    });
                    doc.selection = has_on_canvas_pixels.then_some(combined);
                    Ok(())
                })?;
                Ok(Value::Null)
            }
        ),
        // Layer
        cmd!(
            "layer.new.layer",
            "Layer…",
            ["Layer", "New"],
            Some("Cmd+Shift+N"),
            r##"{"name":str?,"clipToPrevious":bool=false (Use Previous Layer to Create Clipping Mask; inside a clip group the layer joins it anyway),"color":"none|red|orange|yellow|green|seafoam|blue|indigo|magenta|fuchsia|violet|gray"="none","mode":"Normal|Multiply|…"="Normal","opacity":0..1=1,"fillNeutral":bool=false (fill with the mode's neutral color: modes without one are an error),"position":"above|below"="above" (below: the Layers panel's Cmd/Ctrl-click; never under the Background)} (above the active layer, or the top layer inside it when it is an open group)"##,
            has_doc,
            crate::new_layer::run
        ),
        cmd!(
            "layer.new.group",
            "Group…",
            ["Layer", "New"],
            None,
            r##"{"name":text?,"color":"none|red|orange|yellow|green|seafoam|blue|indigo|magenta|fuchsia|violet|gray"="none","mode":str="passThrough" (blend mode),"opacity":0..100=100 (percent),"position":"above|below"="above" (below: the Layers panel's Cmd/Ctrl-click; never under the Background)} (above the active layer, or inside it when it is an open group; clipped when it lands inside a clipping mask)"##,
            has_doc,
            |s, p| {
                let opts = crate::group_options::GroupOptions::parse("layer.new.group", p)?;
                let below = crate::group_options::below("layer.new.group", p)?;
                let place = if below { crate::layer_place::Place::Below } else { crate::layer_place::Place::Auto };
                let id = s.edit("New Group", |doc, active| {
                    let name = opts.name.clone().unwrap_or_else(|| doc.next_layer_name("Group"));
                    let mut group = Layer::group(name, vec![]);
                    opts.apply(&mut group);
                    let bg = crate::background_cmds::background_id(doc);
                    let id = crate::layer_place::insert_new(doc, *active, group, place);
                    crate::background_cmds::keep_background_at_bottom(bg, doc)?;
                    *active = Some(id);
                    Ok(id)
                })?;
                Ok(json!({ "layer": id.0 }))
            }
        ),
        cmd!(
            "layer.groupLayers",
            "Group Layers",
            ["Layer"],
            Some("Cmd+G"),
            r##"{"layer":id?,"name":text?,"color":"none|red|orange|yellow|green|seafoam|blue|indigo|magenta|fuchsia|violet|gray"="none","mode":str="passThrough" (blend mode),"opacity":0..100=100 (percent)} (no layer: every selected layer; the new group starts collapsed)"##,
            has_layer,
            crate::layer_multi_cmds::group_layers
        ),
        cmd!(
            "layer.duplicate",
            "Duplicate Layer…",
            ["Layer"],
            None,
            r##"{"layer":id?,"inPlace":bool=false (artboards: keep the copy on the original instead of beside it),"name":str? (one layer: the copy's name),"document":index|"new"=same document (Photoshop's Destination: "new" makes a new document the size of this canvas holding just the copies),"documentName":str? (the new document's name)} (no layer: every selected layer)"##,
            has_layer,
            |s, p| {
                if let Some(d) = p.get("document") {
                    if d.as_str() == Some("new") {
                        return crate::layer_copy_cmds::duplicate_to_new_document(s, p);
                    }
                    let same = d.as_u64().is_some_and(|i| s.active_index().is_some_and(|a| i == a as u64));
                    if !same {
                        return Err(EngineError::BadParams {
                            cmd: "layer.duplicate".into(),
                            msg: if d.as_u64().is_some() {
                                "the destination is another open document (use layer.copyToDocument)".into()
                            } else {
                                "`document` must be a document index or \"new\"".into()
                            },
                        });
                    }
                }
                let in_place = p.get("inPlace").and_then(Value::as_bool).unwrap_or(false);
                let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string);
                if crate::layer_multi_cmds::multi(s, p) {
                    return crate::layer_multi_cmds::duplicate_selected(s, in_place, "Duplicate Layers");
                }
                let id = layer_param(s, p)?;
                let add_copy = layer_panel_option(s, "addCopyToCopiedLayersAndGroups", true);
                let nid = s.edit("Duplicate Layer", |doc, active| {
                    let mut dup = layer_copy(doc, id, add_copy)?;
                    if let Some(name) = &name {
                        dup.name.clone_from(name);
                    }
                    let nid = doc.insert_above(Some(id), dup);
                    if !in_place {
                        crate::artboard_cmds::place_copy(doc, nid)?;
                    }
                    *active = Some(nid);
                    Ok(nid)
                })?;
                Ok(json!({ "layer": nid.0 }))
            }
        ),
        cmd!("layer.delete", "Delete Layer", ["Layer", "Delete"], None, r##"{"layer":id?} (no layer: every selected layer)"##, has_layer, |s, p| {
            if crate::layer_multi_cmds::multi(s, p) {
                return crate::layer_multi_cmds::delete_selected(s);
            }
            let id = layer_param(s, p)?;
            s.edit("Delete Layer", |doc, active| {
                let neighbours = crate::layer_multi_cmds::deletion_neighbours(doc, id);
                let before = crate::clip_group::Before::read(doc, &[id]);
                doc.remove(id).ok_or(EngineError::NoLayer(id))?;
                crate::clip_group::settle(doc, &[], &before, false);
                if doc.layers.is_empty() {
                    return Err(EngineError::Other("a document must keep at least one layer".into()));
                }
                if active.is_some_and(|current| doc.layer(current).is_none()) {
                    *active = neighbours.into_iter().find(|candidate| doc.layer(*candidate).is_some());
                }
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!(
            "layer.select",
            "Select Layer",
            [],
            None,
            r##"{"layer":id|str | "name":str,"occurrence":index?,"mode":"replace|toggle|range|add|remove"="replace"} (name occurrence: zero-based document walk order; required for duplicate names; toggle = ⌘-click, range = ⇧-click, remove = deselect it, keeping at least one)"##,
            has_doc,
            crate::layer_multi_cmds::select
        ),
        cmd!(
            "layer.setProps",
            "Layer Properties",
            [],
            None,
            r##"{"layer":id?,"layers":[id,…]? (the same change to each, one step: the Layers panel's controls with several layers selected),"name":str? (one layer only),"visible":bool?,"opacity":0..1?,"fill":0..1?,"blend":"Multiply|…"?,"clipped":bool?,"locked":bool?,"locks":{"transparency","pixels","position","artboard","all":bool}?,"channels":[bool,…]? (Advanced Blending: which colour channels blend, R G B / C M Y K / L a b)}"##,
            has_layer,
            |s, p| {
                // Several layers at once (the Layers panel with several selected, as in Photoshop),
                // else the given or active one.
                let ids: Vec<LayerId> = match p.get("layers") {
                    None | Some(Value::Null) => vec![layer_param(s, p)?],
                    Some(Value::Array(a)) if !a.is_empty() && a.len() <= 10_000 => {
                        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
                        let mut ids = Vec::with_capacity(a.len());
                        for v in a {
                            let id = v.as_u64().map(LayerId).ok_or_else(|| bad("layer.setProps", format!("`layers` holds layer ids, got {v}")))?;
                            if doc.layer(id).is_none() {
                                return Err(EngineError::NoLayer(id));
                            }
                            if !ids.contains(&id) {
                                ids.push(id);
                            }
                        }
                        ids
                    }
                    Some(v) => return Err(bad("layer.setProps", format!("`layers` is a non-empty list of layer ids, got {v}"))),
                };
                if ids.len() > 1 && p.get("name").is_some() {
                    return Err(bad("layer.setProps", "`name` renames one layer, not several"));
                }
                // Opacity and Fill are fractions here, while the Layer Style commands take
                // percentages: a value outside 0..1 is refused, never clamped to 100 % (#5242).
                let unit = |key: &str| -> Result<Option<f32>> {
                    let Some(v) = p.get(key) else { return Ok(None) };
                    match v.as_f64() {
                        Some(x) if (0.0..=1.0).contains(&x) => Ok(Some(x as f32)),
                        _ => Err(bad("layer.setProps", format!("{key} must be a number in 0..1 (1 = 100 %), got {v}"))),
                    }
                };
                let (opacity, fill) = (unit("opacity")?, unit("fill")?);
                // Photoshop's History names for a single kind of change; a mix is "Layer Properties".
                let changed: Vec<&str> = p
                    .as_object()
                    .map(|o| o.keys().map(String::as_str).filter(|k| !matches!(*k, "layer" | "layers" | "coalesce")).collect())
                    .unwrap_or_default();
                let label = match changed.as_slice() {
                    ["visible"] => "Layer Visibility",
                    ["opacity"] => "Opacity Change",
                    ["fill"] => "Fill Opacity Change",
                    ["blend"] => "Blending Change",
                    ["name"] => "Rename Layer",
                    _ => "Layer Properties",
                };
                // The eye: a history state only with History Options › Make Layer Visibility
                // Changes Undoable.
                let only_visibility = changed.as_slice() == ["visible"];
                let record = !only_visibility || s.visibility_undoable();
                s.edit_with(label, record, |doc, _| {
                    // The Background has no opacity, fill or blend mode: like Photoshop, setting
                    // one that changes its look turns it into a normal layer ("Layer 0") first.
                    let num = |k: &str| p.get(k).and_then(Value::as_f64).is_some_and(|v| v < 1.0);
                    let blends = p.get("blend").and_then(Value::as_str).and_then(blend_from_str).is_some_and(|b| b != photocraft_color::BlendMode::Normal);
                    for &id in &ids {
                        if num("opacity") || num("fill") || blends {
                            crate::extra_cmds::background_to_layer(doc, id);
                        }
                        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
                        if let Some(v) = p.get("name").and_then(Value::as_str) {
                            l.name = v.to_string();
                        }
                        if let Some(v) = p.get("visible").and_then(Value::as_bool) {
                            l.visible = v;
                        }
                        if let Some(v) = opacity {
                            l.opacity = v;
                        }
                        if let Some(v) = fill {
                            l.fill_opacity = v;
                        }
                        if let Some(v) = p.get("blend").and_then(Value::as_str) {
                            let b = blend_from_str(v).ok_or_else(|| bad("layer.setProps", format!("unknown blend mode `{v}`")))?;
                            // Pass Through is a group's mode only, as in Photoshop.
                            if b == photocraft_color::BlendMode::PassThrough && !l.is_group() {
                                return Err(bad("layer.setProps", format!("Pass Through applies to groups only; layer {} isn't one", l.id.0)));
                            }
                            l.blend = b;
                        }
                        if let Some(v) = p.get("clipped").and_then(Value::as_bool) {
                            l.clipped = v;
                        }
                        if let Some(v) = p.get("locked").and_then(Value::as_bool) {
                            l.locks.all = v;
                        }
                        if let Some(Value::Array(ch)) = p.get("channels") {
                            // Blending Options › Advanced Blending › Channels: unchecked = left out.
                            l.excluded_channels = ch.iter().take(32).enumerate().filter(|(_, v)| v.as_bool() == Some(false)).fold(0, |m, (i, _)| m | 1 << i);
                        }
                        if let Some(Value::Object(m)) = p.get("locks") {
                            for (k, v) in m {
                                let v = v.as_bool().unwrap_or(false);
                                if !l.locks.set(k, v) {
                                    return Err(bad("layer.setProps", format!("unknown lock `{k}`")));
                                }
                            }
                        }
                    }
                    Ok(())
                })?;
                Ok(Value::Null)
            }
        ),
        cmd!("layer.arrange.bringForward", "Bring Forward", ["Layer", "Arrange"], Some("Cmd+]"), r##"{"layer":id?}"##, has_layer, |s, p| arrange(s, p, 1)),
        cmd!("layer.arrange.sendBackward", "Send Backward", ["Layer", "Arrange"], Some("Cmd+["), r##"{"layer":id?}"##, has_layer, |s, p| arrange(s, p, -1)),
        cmd!("layer.arrange.bringToFront", "Bring to Front", ["Layer", "Arrange"], Some("Cmd+Shift+]"), r##"{"layer":id?}"##, has_layer, |s, p| arrange(
            s,
            p,
            i32::MAX
        )),
        cmd!("layer.arrange.sendToBack", "Send to Back", ["Layer", "Arrange"], Some("Cmd+Shift+["), r##"{"layer":id?}"##, has_layer, |s, p| arrange(
            s,
            p,
            i32::MIN
        )),
        cmd!(
            "layer.createClippingMask",
            "Create Clipping Mask",
            ["Layer"],
            Some("Cmd+Alt+G"),
            r##"{"layer":id?} (no layer: every selected layer but the lowest clips to the layer below it)"##,
            has_layer,
            |s, p| set_clipped(s, p, true)
        ),
        cmd!(
            "layer.releaseClippingMask",
            "Release Clipping Mask",
            ["Layer"],
            None,
            r##"{"layer":id?} (no layer: every selected layer)"##,
            has_layer,
            |s, p| set_clipped(s, p, false)
        ),
        cmd!(
            "layer.layerMask.revealAll",
            "Reveal All",
            ["Layer", "Layer Mask"],
            None,
            r##"{"layer":id?} (no layer: every selected layer that can take a mask)"##,
            has_layer,
            |s, p| set_mask(s, p, "Add Layer Mask", Some(LayerMask::reveal_all()))
        ),
        cmd!(
            "layer.layerMask.hideAll",
            "Hide All",
            ["Layer", "Layer Mask"],
            None,
            r##"{"layer":id?} (no layer: every selected layer that can take a mask)"##,
            has_layer,
            |s, p| set_mask(s, p, "Add Layer Mask", Some(LayerMask::hide_all()))
        ),
        cmd!(
            "layer.layerMask.revealSelection",
            "Reveal Selection",
            ["Layer", "Layer Mask"],
            None,
            r##"{"layer":id?} (no layer: every selected layer that can take a mask)"##,
            has_selection,
            |s, p| {
                let sel = s.active().and_then(|d| d.doc.selection.clone()).ok_or(EngineError::Other("no selection".into()))?;
                let mask = LayerMask { surface: sel, ..LayerMask::reveal_all() };
                // The selection becomes the mask, so Photoshop drops it (as Hide Selection does): a
                // following Invert (⌘I) on the targeted mask then inverts all of it, not just the
                // part inside the old selection. Same undo step as the mask.
                set_mask_and_deselect(s, p, "Add Layer Mask", mask)
            }
        ),
        cmd!("layer.layerMask.delete", "Delete", ["Layer", "Layer Mask"], None, r##"{"layer":id?}"##, has_layer, |s, p| set_mask(
            s,
            p,
            "Delete Layer Mask",
            None
        )),
        cmd!("layer.mergeDown", "Merge Down", ["Layer"], None, r##"{"layer":id?}"##, has_layer, |s, p| {
            let id = layer_param(s, p)?;
            s.edit("Merge Down", |doc, active| {
                let path = doc.path_of(id).ok_or(EngineError::NoLayer(id))?;
                let Some((&idx, parent)) = path.split_last() else { return Err(EngineError::NoLayer(id)) };
                if idx == 0 {
                    return Err(EngineError::Other("no layer below to merge into".into()));
                }
                let mut below = parent.to_vec();
                below.push(idx - 1);
                let lower = doc.layer_at(&below).ok_or(EngineError::NoLayer(id))?.clone();
                let upper = doc.layer(id).ok_or(EngineError::NoLayer(id))?.clone();
                let merged = pixels::merge_down(doc, &lower, &upper);
                doc.remove(id);
                *doc.layer_at_mut(&below).ok_or(EngineError::NoLayer(lower.id))? = merged;
                *active = Some(lower.id);
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!("layer.mergeGroup", "Merge Group", ["Layer"], None, r##"{"layer":id?}"##, has_layer, |s, p| {
            let id = layer_param(s, p)?;
            if !s.active().and_then(|d| d.doc.layer(id)).is_some_and(|l| matches!(l.content, LayerContent::Group(_))) {
                return Err(EngineError::Other("Merge Group needs a group".into()));
            }
            merge_group(s, id)
        }),
        cmd!("layer.flattenImage", "Flatten Image", ["Layer"], None, "{}", has_doc, |s, _| {
            s.edit("Flatten Image", |doc, active| {
                let fmt = doc.pixel_format();
                let mut bg = Layer::raster("Background", fmt);
                bg.locks.transparency = true;
                bg.locks.position = true;
                *crate::pixels_mut(&mut bg)? = photocraft_compose::flatten_to_surface(doc, fmt, Some([1.0, 1.0, 1.0]));
                *active = Some(bg.id);
                doc.layers = vec![bg];
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!(
            "layer.newFillLayer.solidColor",
            "Solid Color…",
            ["Layer", "New Fill Layer"],
            None,
            r##"{"color":"#rrggbb"=foreground,"cmyk":[c,m,y,k]? (inks 0..1 the colour was picked as: a CMYK document's fill holds exactly these; default the foreground's CMYK pick),"path":"work"|saved path name|{…path}? (the active path: becomes the vector mask, making a shape; else the selection is the layer mask)}"##,
            has_doc,
            |s, p| {
                let c = color_param("layer.newFillLayer.solidColor", p, "color", s.tools.foreground)?;
                // The inks the colour was picked as, so CMYK 50/50/50/100 stays 50/50/50/100 instead
                // of what its sRGB stand-in converts back to through the document profile (#8568).
                let inks = match cmyk_param("layer.newFillLayer.solidColor", p, "cmyk")? {
                    Some(k) => Some(k),
                    None => s.tools.picked_cmyk([c[0], c[1], c[2]]),
                };
                let default_mask = layer_panel_option(s, "useDefaultMasksOnFillLayers", true);
                let id = s.edit("New Color Fill Layer", |doc, active| {
                    let fill = match inks {
                        Some(k) if doc.mode == ColorMode::Cmyk => Fill::Solid(Color { mode: ColorMode::Cmyk, c: k, alpha: c[3] }),
                        _ => Fill::Solid(Color::rgba(c[0], c[1], c[2], c[3])).in_mode(doc.mode),
                    };
                    let mut l = Layer::new(doc.next_layer_name("Color Fill"), LayerContent::Fill(fill));
                    mask_new_layer(doc, &mut l, p, "layer.newFillLayer.solidColor", default_mask)?;
                    let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::Auto);
                    *active = Some(id);
                    Ok(id)
                })?;
                Ok(json!({ "layer": id.0 }))
            }
        ),
        cmd!(
            "layer.newFillLayer.gradient",
            "Gradient…",
            ["Layer", "New Fill Layer"],
            None,
            r##"{"from":"#rrggbb","to":"#rrggbb","angle":deg=90,"style":"linear|radial|angle|reflected|diamond","reverse":bool,"path":"work"|saved path name|{…path}? (the active path: becomes the vector mask; else the selection is the layer mask)}"##,
            has_doc,
            |s, p| {
                let a = color_param("layer.newFillLayer.gradient", p, "from", s.tools.foreground)?;
                let b = color_param("layer.newFillLayer.gradient", p, "to", s.tools.background)?;
                let angle = crate::gradient_fill_cmds::angle_param("layer.newFillLayer.gradient", p, 90.0)?;
                let style = crate::layer_style::gradient_style(p.get("style").and_then(Value::as_str).unwrap_or("linear"));
                let reverse = p.get("reverse").and_then(Value::as_bool).unwrap_or(false);
                let default_mask = layer_panel_option(s, "useDefaultMasksOnFillLayers", true);
                let id = s.edit("New Gradient Fill Layer", |doc, active| {
                    let fill = Fill::gradient(
                        vec![(0.0, Color::rgba(a[0], a[1], a[2], a[3])), (1.0, Color::rgba(b[0], b[1], b[2], b[3]))],
                        angle,
                        1.0,
                        style,
                        reverse,
                    )
                    .in_mode(doc.mode);
                    let mut l = Layer::new(doc.next_layer_name("Gradient Fill"), LayerContent::Fill(fill));
                    mask_new_layer(doc, &mut l, p, "layer.newFillLayer.gradient", default_mask)?;
                    let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::Auto);
                    *active = Some(id);
                    Ok(id)
                })?;
                Ok(json!({ "layer": id.0 }))
            }
        ),
        cmd!(
            "layer.setAdjustment",
            "Adjustment Properties",
            [],
            None,
            r##"{"layer":id?, …params of that adjustment kind} (Levels, Curves: "eyedropper":{"point":"black|gray|white","at":[x,y],"size":1..101=1} or {"point",…,"color":[r,g,b] 0..1} = the Properties panel's Set Black/Neutral Gray/White Point, sampling the image below the layer, RGB and Grayscale (no gray) only)"##,
            has_layer,
            |s, p| {
                let id = layer_param(s, p)?;
                let eyedropped = crate::adjust_params::layer_eyedropper(s, id, p)?;
                s.edit("Modify Adjustment", |doc, _| {
                    let mode = doc.mode;
                    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
                    let LayerContent::Adjustment(adj) = &mut l.content else { return Err(EngineError::Other("not an adjustment layer".into())) };
                    if matches!(adj, Adjustment::Unsupported { .. }) {
                        return Err(EngineError::Other("this adjustment can't be edited (kept as imported)".into()));
                    }
                    let kind = adjustment_kind(adj);
                    // No params resets to the defaults; otherwise params merge over the current values.
                    *adj = if let Some(eyedropped) = eyedropped {
                        eyedropped
                    } else if crate::adjust_params::user_keys(p).is_empty() {
                        crate::adjust_params::default_for(kind, mode)?
                    } else {
                        crate::adjust_params::from_params(kind, p, Some(adj), mode)?
                    };
                    Ok(())
                })?;
                Ok(Value::Null)
            }
        ),
        cmd!(
            "layer.moveTo",
            "Reorder Layer",
            [],
            None,
            r##"{"layer":id?| "layers":[id,…]?, "target":id|name,"targetOccurrence":n? (which of several layers named `target`, in document order),"position":"above|below|into"="above","copy":bool=false} (layers: one undoable move, document order preserved; copy: place duplicates there and leave the layers, as ⌥-dragging a Layers panel row; a recorded action keeps the target by name)"##,
            has_layer,
            crate::layer_multi_cmds::move_to
        ),
        cmd!(
            "layer.translate",
            "Move Layer",
            [],
            None,
            r##"{"layer":id?,"dx":i32,"dy":i32,"units":"px"|"pt"="px" (pt: points, whole pixels at the document's resolution),"target":"pixels"|"mask"="pixels","selection":bool=false} (no layer: every selected layer; linked layers follow; target mask: a layer whose mask is unlinked moves that mask alone, a linked mask moves with the layer; selection: the selection moves too, in the same step)"##,
            has_layer,
            crate::layer_multi_cmds::translate
        ),
        // Image
        cmd!("image.imageRotation.flipCanvasHorizontal", "Flip Canvas Horizontal", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Flip Canvas Horizontal", crate::canvas_geom::Turn::FlipHorizontal)
        }),
        cmd!("image.imageRotation.flipCanvasVertical", "Flip Canvas Vertical", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Flip Canvas Vertical", crate::canvas_geom::Turn::FlipVertical)
        }),
        cmd!("image.imageRotation.180", "180°", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Rotate Canvas", crate::canvas_geom::Turn::Rotate180)
        }),
        cmd!("image.imageRotation.90cw", "90° Clockwise", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Rotate Canvas", crate::canvas_geom::Turn::Cw90)
        }),
        cmd!("image.imageRotation.90ccw", "90° Counter Clockwise", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Rotate Canvas", crate::canvas_geom::Turn::Ccw90)
        }),
        cmd!(
            "image.adjustments.desaturate",
            "Desaturate",
            ["Image", "Adjustments"],
            Some("Cmd+Shift+U"),
            r##"{"target":"pixels"|"mask"|{"color":i}|{"colors":[i,…]}?=Channels panel target,"layer":id|name?=active}"##,
            has_pixel_layer,
            |s, p| {
                destructive_adjust(
                    s,
                    "Desaturate",
                    Adjustment::HueSaturation {
                        hue: 0.0,
                        saturation: -100.0,
                        lightness: 0.0,
                        colorize: false,
                        ranges: photocraft_doc::adjust::HueRange::defaults(),
                    },
                    p,
                )
            }
        ),
        // Paint
        cmd!(
            "paint.stroke",
            "Brush Stroke",
            [],
            None,
            r##"{"points":[[x,y,pressure?,tiltX?,tiltY?,rotation?,timeMs?,wheel?],…],"brush":{…BrushSettings}?,"preset":name?,"size":0.5..5000 px?,"hardness":0..1?,"opacity":0..1?,"flow":0..1?,"fill":bool=false (fill the region the closed `points` enclose with the brush colour, its edge anti-aliased and softened by hardness, instead of stamping the outline; a closed gesture paints a filled dab, not a ring),"spacing":0..10?,"color":"#rrggbb"?=foreground,"mode":"normal|multiply|screen|…|clear (erases with this brush)"="normal","erase":bool?,"smoothing":0..1?,"zoom":number=1,"seed":u64?,"target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}=Channels panel target}"##,
            has_paintable,
            crate::brush_cmds::paint_stroke
        ),
        cmd!(
            "tools.setColors",
            "Set Colors",
            [],
            None,
            r##"{"foreground":"#rrggbb"|[r,g,b,a?]?,"background":same?,"foregroundCmyk":[c,m,y,k]? (inks 0..1 the foreground was picked as: a CMYK document is filled with exactly these),"backgroundCmyk":same?}"##,
            always,
            |s, p| {
                let cmyk = |key: &str| cmyk_param("tools.setColors", p, key);
                let (fg_cmyk, bg_cmyk) = (cmyk("foregroundCmyk")?, cmyk("backgroundCmyk")?);
                let foreground = color_param("tools.setColors", p, "foreground", s.tools.foreground)?;
                let background = color_param("tools.setColors", p, "background", s.tools.background)?;
                s.tools.foreground = foreground;
                s.tools.background = background;
                if let Some(k) = fg_cmyk {
                    let [r, g, b, _] = s.tools.foreground;
                    s.tools.remember_cmyk([r, g, b], k);
                }
                if let Some(k) = bg_cmyk {
                    let [r, g, b, _] = s.tools.background;
                    s.tools.remember_cmyk([r, g, b], k);
                }
                Ok(Value::Null)
            }
        ),
        cmd!("tools.swapColors", "Switch Foreground and Background Colors", [], Some("X"), "{}", always, |s, _| {
            std::mem::swap(&mut s.tools.foreground, &mut s.tools.background);
            Ok(Value::Null)
        }),
        cmd!("tools.defaultColors", "Default Foreground and Background Colors", [], Some("D"), "{}", always, |s, _| {
            if s.tools.mask_targeted {
                s.tools.foreground = crate::ToolState::MASK_COLORS[0];
                s.tools.background = crate::ToolState::MASK_COLORS[1];
            } else {
                s.tools.foreground = [0.0, 0.0, 0.0, 1.0];
                s.tools.background = [1.0, 1.0, 1.0, 1.0];
            }
            Ok(Value::Null)
        }),
        // Queries (not journaled)
        CommandSpec {
            id: "session.inspect",
            label: "Inspect Session",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| Ok(inspect::session(s)),
            journal: false,
        },
        CommandSpec {
            id: "document.inspect",
            label: "Inspect Document",
            menu: &[],
            shortcut: None,
            params: r##"{"document":index?}"##,
            enabled: has_doc,
            run: |s, p| {
                let d = match p.get("document") {
                    Some(v) => {
                        let i = v.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("document.inspect", "`document` must be an index"))?;
                        s.documents().get(i).ok_or_else(|| EngineError::Other(format!("no document at index {i}")))?
                    }
                    None => s.active().ok_or(EngineError::NoDocument)?,
                };
                Ok(inspect::document(d))
            },
            journal: false,
        },
        CommandSpec {
            id: "document.activate",
            label: "Activate Document",
            menu: &[],
            shortcut: None,
            params: r##"{"document":index} or {"offset":int} (the document that many places after the active one in opening order, before it when negative: a Photoshop action's switch to another document) → {document}"##,
            enabled: has_doc,
            run: |s, p| {
                // A null key counts as omitted.
                let given = |k: &str| p.get(k).filter(|v| !v.is_null());
                let idx = match (given("document"), given("offset")) {
                    (Some(_), Some(_)) => return Err(bad("document.activate", "give `document` or `offset`, not both")),
                    (None, None) => return Err(bad("document.activate", "needs `document` (a tab index) or `offset`")),
                    // Refuse offsets past the opening-order ends rather than wrap, so an action that
                    // copies from one document never silently pastes into the wrong one (#7103).
                    (None, Some(o)) => {
                        let o = o.as_i64().ok_or_else(|| bad("document.activate", "`offset` must be an integer"))?;
                        s.document_at_offset(o)?
                    }
                    (Some(d), None) => {
                        let i = d.as_u64().ok_or_else(|| bad("document.activate", "`document` must be a tab index"))?;
                        usize::try_from(i).map_err(|_| bad("document.activate", "`document` out of range"))?
                    }
                };
                if s.set_active(idx) { Ok(json!({"document": idx})) } else { Err(EngineError::Other(format!("no document at index {idx}"))) }
            },
            journal: false,
        },
        CommandSpec {
            id: "document.move",
            label: "Move Document",
            menu: &[],
            shortcut: None,
            params: r##"{"document":index?,"to":index}"##,
            enabled: has_doc,
            run: |s, p| {
                let index = |key: &str| {
                    p.get(key)
                        .map(|v| v.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("document.move", format!("`{key}` must be a tab index"))))
                };
                let from = match index("document") {
                    Some(i) => i?,
                    None => s.active_index().ok_or(EngineError::NoDocument)?,
                };
                let to = index("to").ok_or_else(|| bad("document.move", "missing `to`"))??;
                let to = s.move_document(from, to).ok_or(EngineError::NoDocument)?;
                Ok(json!({"document": to}))
            },
            journal: false,
        },
        CommandSpec {
            id: "command.list",
            label: "List Commands",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| Ok(Value::Array(command_specs().iter().map(|c| list_entry(s, c, (c.enabled)(s).err())).collect())),
            journal: false,
        },
        CommandSpec {
            id: "document.pixel",
            label: "Read Composite Pixel",
            menu: &[],
            shortcut: None,
            params: r##"{"x":i32,"y":i32}"##,
            enabled: has_doc,
            run: |s, p| {
                let coord = |k: &str| i32::try_from(int(p, k).unwrap_or(0)).map_err(|_| bad("document.pixel", format!("{k} must fit in 32 bits")));
                let (x, y) = (coord("x")?, coord("y")?);
                let d = s.active().ok_or(EngineError::NoDocument)?;
                // At i32::MAX the 1x1 rect saturates to empty: a pixel that far out is transparent.
                let px = photocraft_compose::render(&d.doc, Rect::from_xywh(x, y, 1, 1)).px.first().copied().unwrap_or_default();
                Ok(json!(px))
            },
            journal: false,
        },
    ];

    // Adjustment layers + destructive adjustments, generated from one list.
    // Levels/Curves channel keys follow the document: red/green/blue (RGB), gray (Grayscale),
    // cyan/magenta/yellow/black (CMYK), lightness/a/b (Lab); see `adjust_params`.
    const ADJ: &[(&str, &str, &str)] = &[
        ("brightnessContrast", "Brightness/Contrast…", r##"{"brightness":-150..150=0,"contrast":-50..100=0,"legacy":bool=false}"##),
        (
            "levels",
            "Levels…",
            r##"{"inBlack":0..253=0,"gamma":0.01..9.99=1,"inWhite":2..255=255,"outBlack":0..255=0,"outWhite":0..255=255,"red":json,"green":json,"blue":json,"eyedropper":json} (top level = composite; per channel {"inBlack","gamma","inWhite","outBlack","outWhite"} under red/green/blue, gray, cyan/magenta/yellow/black or lightness/a/b; eyedropper = the dialog's Set Black/Neutral Gray/White Point picker {"point":"black|gray|white","at":[x,y],"size":1..101=1} or {"point",…,"color":[r,g,b] 0..1}: black/white set each channel's input black/white to the sample, gray each channel's gamma so the sample turns neutral at its luma; RGB and Grayscale (no gray) only, applied over the given levels)"##,
        ),
        (
            "curves",
            "Curves…",
            r##"{"points":json,"red":json,"green":json,"blue":json,"pencil":bool=false,"smooth":0..10=0,"eyedropper":json} (curves as [[in,out],…] in 0..255, 2..19 points: points = composite; red/green/blue, gray, cyan/magenta/yellow/black or lightness/a/b per channel; pencil = drawn curves (Draw to modify the curve): each curve is 256 output levels in 0..255 (point curves given are drawn in, and turning pencil off fits points to the drawing); smooth = Smooth passes over the drawn curves; eyedropper = the dialog's Set Black/Neutral Gray/White Point picker {"point":"black|gray|white","at":[x,y],"size":1..101=1} or {"point",…,"color":[r,g,b] 0..1}, RGB and Grayscale (no gray) only, applied over the given curves)"##,
        ),
        ("exposure", "Exposure…", r##"{"exposure":-20..20=0,"offset":-0.5..0.5=0,"gamma":0.01..9.99=1}"##),
        ("vibrance", "Vibrance…", r##"{"vibrance":-100..100=0,"saturation":-100..100=0}"##),
        (
            "hueSaturation",
            "Hue/Saturation…",
            r##"{"hue":-180..180=0,"saturation":-100..100=0,"lightness":-100..100=0,"colorize":bool=false,"reds":json,"yellows":json,"greens":json,"cyans":json,"blues":json,"magentas":json} (per range {"hue","saturation","lightness","range":[4 hue degrees]}; colorize: hue 0..360, saturation 0..100)"##,
        ),
        (
            "colorBalance",
            "Color Balance…",
            r##"{"shadows":json,"midtones":json,"highlights":json,"preserveLuminosity":bool=true} (each tone [cyan-red, magenta-green, yellow-blue] in -100..100)"##,
        ),
        (
            "blackWhite",
            "Black & White…",
            r##"{"reds":-200..300=40,"yellows":-200..300=60,"greens":-200..300=40,"cyans":-200..300=60,"blues":-200..300=20,"magentas":-200..300=80,"tint":bool=false,"tintColor":"#rrggbb"}"##,
        ),
        (
            "photoFilter",
            "Photo Filter…",
            r##"{"filter":"warming85|warmingLBA|warming81|cooling80|coolingLBB|cooling82|red|orange|yellow|green|cyan|blue|violet|magenta|sepia|deepRed|deepBlue|deepEmerald|deepYellow|underwater","color":"#rrggbb","density":0..100=25,"preserveLuminosity":bool=true}"##,
        ),
        (
            "channelMixer",
            "Channel Mixer…",
            r##"{"red":json,"green":json,"blue":json,"gray":json,"monochrome":bool=false} (per output channel [red %, green %, blue %, constant %] in -200..200; gray = the monochrome mix)"##,
        ),
        ("invert", "Invert", "{}"),
        ("posterize", "Posterize…", r##"{"levels":2..255=4}"##),
        ("threshold", "Threshold…", r##"{"level":1..255=128}"##),
        ("gradientMap", "Gradient Map…", r##"{"stops":json,"reverse":bool=false,"dither":bool=false} (stops [[location 0..1, "#rrggbb"], …], 2..64)"##),
        (
            "selectiveColor",
            "Selective Color…",
            r##"{"method":"relative|absolute"="relative","colors":"reds|yellows|greens|cyans|blues|magentas|whites|neutrals|blacks"="reds","cyan":-100..100=0,"magenta":-100..100=0,"yellow":-100..100=0,"black":-100..100=0,"reds":json} (per-range [c,m,y,k] arrays: reds yellows greens cyans blues magentas whites neutrals blacks)"##,
        ),
        (
            "colorLookup",
            "Color Lookup…",
            r##"{"lut":"none|warm|cool|tealOrange|bleachBypass|fadedFilm|dayForNight|monoContrast|crossProcess"="none","file":text,"interpolation":"trilinear|tetrahedral"="trilinear","dither":bool=false,"data":json} (file: .cube/.3dl/.look path; data: file text + "fileName")"##,
        ),
    ];
    for &(kind, label, params) in ADJ {
        let layer_id: &'static str = Box::leak(format!("layer.newAdjustmentLayer.{kind}").into_boxed_str());
        let image_id: &'static str = Box::leak(format!("image.adjustments.{kind}").into_boxed_str());
        v.push(CommandSpec {
            id: layer_id,
            label,
            menu: &["Layer", "New Adjustment Layer"],
            shortcut: None,
            params: Box::leak(format!("{params} + {{{NEW_LAYER_PATH}}}").into_boxed_str()),
            enabled: has_doc,
            run: |s, p| {
                let kind = p.get("__kind").and_then(Value::as_str).unwrap_or("invert").to_string();
                let eyedropped = if matches!(kind.as_str(), "levels" | "curves") { crate::adjust_params::eyedropper_from_params(s, &kind, p)? } else { None };
                let adj = match eyedropped {
                    Some(adj) => adj,
                    None => crate::adjust_params::from_params(&kind, p, None, doc_mode(s))?,
                };
                new_adjustment(s, adj, p, &format!("layer.newAdjustmentLayer.{kind}"))
            },
            journal: true,
        });
        v.push(CommandSpec {
            id: image_id,
            label,
            menu: &["Image", "Adjustments"],
            shortcut: match kind {
                "levels" => Some("Cmd+L"),
                "curves" => Some("Cmd+M"),
                "hueSaturation" => Some("Cmd+U"),
                "colorBalance" => Some("Cmd+B"),
                "invert" => Some("Cmd+I"),
                "blackWhite" => Some("Cmd+Alt+Shift+B"),
                _ => None,
            },
            params,
            enabled: crate::adjust_cmds::has_adjustable,
            run: |s, p| {
                let kind = p.get("__kind").and_then(Value::as_str).unwrap_or("invert").to_string();
                let eyedropped = if matches!(kind.as_str(), "levels" | "curves") { crate::adjust_params::eyedropper_from_params(s, &kind, p)? } else { None };
                // A smart filter re-runs its params, so it records what an eyedropper resolved
                // to, not the sample (#3127).
                let recorded = eyedropped.as_ref().map(|adj| {
                    let mut q = p.clone();
                    if let (Value::Object(q), Value::Object(resolved)) = (&mut q, crate::adjust_params::to_params(adj)) {
                        q.remove("eyedropper");
                        q.extend(resolved);
                    }
                    q
                });
                let adj = match eyedropped {
                    Some(adj) => adj,
                    None => {
                        // A gray plane target keeps the neutral tone space even over params
                        // carrying Lab or CMYK records (#5335).
                        let tone = if is_mask_target(p) || crate::channel_cmds::is_channel_target(p) {
                            if matches!(kind.as_str(), "levels" | "curves") { gray_tone_params(p) } else { p.clone() }
                        } else {
                            p.clone()
                        };
                        crate::adjust_params::from_params(&kind, &tone, None, adjust_mode(s, p))?
                    }
                };
                let adj = if matches!(kind.as_str(), "levels" | "curves")
                    && let Some(colors) = crate::color_channels::target(s, p)?
                {
                    crate::color_channels::tone_adjustment(&kind, p, adj, doc_mode(s), colors)?
                } else {
                    adj
                };
                let label = adj.label().to_string();
                // A smart object records the adjustment as a smart filter (Photoshop), which
                // re-runs from the source instead of baking into a copy of the pixels.
                if crate::adjust_cmds::adjust_as_smart_filter(s, &kind, &label, recorded.as_ref().unwrap_or(p))? {
                    return Ok(Value::Null);
                }
                destructive_adjust(s, &label, adj, p)
            },
            journal: true,
        });
    }
    v.extend(crate::layer_style::specs());
    v.extend(crate::contour_lib_cmds::specs());
    v.extend(crate::layer_list_cmds::specs());
    v.extend(crate::filters::specs());
    v.extend(crate::filters_ext::specs());
    v.extend(crate::color_to_alpha_cmds::specs());
    v.extend(crate::gallery_cmds::specs());
    v.extend(crate::gradient_fill_cmds::specs());
    v.extend(crate::solid_fill_cmds::specs());
    v.extend(crate::pattern_fill_cmds::specs());
    v.extend(crate::type_cmds::specs());
    v.extend(crate::type_mask_cmds::specs());
    v.extend(crate::transform_cmds::specs());
    v.extend(crate::float_cmds::specs());
    v.extend(crate::vector_cmds::specs());
    v.extend(crate::path_edit_cmds::specs());
    v.extend(crate::smartselect_cmds::specs());
    v.extend(crate::cutout_cmds::specs());
    v.extend(crate::symmetry_cmds::specs());
    v.extend(crate::edit_cmds::specs());
    v.extend(crate::smart_svg_cmds::specs());
    v.extend(crate::color_cmds::specs());
    v.extend(crate::brush_cmds::specs());
    v.extend(crate::brush_preset_cmds::specs());
    v.extend(crate::eraser_cmds::specs());
    v.extend(crate::preset_import_cmds::specs());
    v.extend(crate::style_asl_cmds::specs());
    v.extend(crate::atn_cmds::specs());
    v.extend(crate::actions_edit_cmds::specs());
    v.extend(crate::swatch_cmds::specs());
    v.extend(crate::retouch_cmds::specs());
    v.extend(crate::redeye_cmds::specs());
    v.extend(crate::image_cmds::specs());
    v.extend(crate::perspective_crop_cmds::specs());
    v.extend(crate::selection_cmds::specs());
    v.extend(crate::magnetic_cmds::specs());
    v.extend(crate::select_extra_cmds::specs());
    v.extend(crate::paint_cmds::specs());
    v.extend(crate::extra_cmds::specs());
    v.extend(crate::background_cmds::specs());
    v.extend(crate::file_cmds::specs());
    v.extend(crate::exr_cmds::specs());
    v.extend(crate::type_extra_cmds::specs());
    v.extend(crate::type_area_cmds::specs());
    v.extend(crate::type_styles_cmds::specs());
    v.extend(crate::type_spell_cmds::specs());
    v.extend(crate::type_caret_cmds::specs());
    v.extend(crate::smart_cmds::specs());
    v.extend(crate::layer_multi_cmds::specs());
    v.extend(crate::layer_copy_cmds::specs());
    v.extend(crate::prefs::specs());
    v.extend(crate::updates_cmds::specs());
    v.extend(crate::edit_menu_cmds::specs());
    v.extend(crate::fill_key_cmds::specs());
    v.extend(crate::sample_cmds::specs());
    v.extend(crate::face_cmds::specs());
    v.extend(crate::sampler_cmds::specs());
    v.extend(crate::brush_key_cmds::specs());
    v.extend(crate::stamp_cmds::specs());
    v.extend(crate::align_cmds::specs());
    v.extend(crate::photo_cmds::specs());
    v.extend(crate::lens_cmds::specs());
    v.extend(crate::vp_cmds::specs());
    v.extend(crate::channel_cmds::specs());
    v.extend(crate::adjust_cmds::specs());
    v.extend(crate::layer_menu_cmds::specs());
    v.extend(crate::layer_label_cmds::specs());
    v.extend(crate::layer_xmp_cmds::specs());
    v.extend(crate::mode_cmds::specs());
    v.extend(crate::multichannel_cmds::specs());
    v.extend(crate::pattern_cmds::specs());
    v.extend(crate::warp_cmds::specs());
    v.extend(crate::comps_cmds::specs());
    v.extend(crate::artboard_cmds::specs());
    v.extend(crate::distort_cmds::specs());
    v.extend(crate::analysis_cmds::specs());
    v.extend(crate::notes_cmds::specs());
    v.extend(crate::font_library_cmds::specs());
    v.extend(crate::history_cmds::specs());
    v.extend(crate::history_options_cmds::specs());
    v.extend(crate::history_menu_cmds::specs());
    v.extend(crate::history_source_cmds::specs());
    v.extend(crate::history_select_cmds::specs());
    v.extend(crate::proof_sim::specs());
    v.extend(crate::presets::specs());
    v.extend(crate::document_preset_cmds::specs());
    v.extend(crate::export_preset_cmds::specs());
    v.extend(crate::render_cmds::specs());
    v.extend(crate::slice_cmds::specs());
    v.extend(crate::web_cmds::specs());
    v.extend(crate::automate_cmds::specs());
    v.extend(crate::print_cmds::specs());
    v.extend(crate::installed_profiles::specs());
    v.extend(crate::pick_cmds::specs());
    v.extend(crate::layer_nav_cmds::specs());
    v.extend(crate::frame_cmds::specs());
    v.extend(crate::migrate_cmds::specs());
    v.extend(crate::trap_cmds::specs());
    v.extend(crate::timeline_cmds::specs());
    v.extend(crate::video_cmds::specs());
    v.extend(crate::jobs::specs());
    v.extend(crate::wia_cmds::specs());
    v.extend(crate::rasterize_style_cmds::specs());
    v.extend(crate::variables_cmds::specs());
    v.extend(crate::plugin_cmds::specs());
    v.extend(crate::group_view_cmds::specs());
    v.extend(crate::fx_view_cmds::specs());
    v.extend(crate::fx_visibility_cmds::specs());
    v.extend(crate::mask_view_cmds::specs());
    v.extend(crate::layer_mask_props_cmds::specs());
    v.extend(crate::actions_cmds::specs());
    // The dispatch guard's unit test needs a command that panics on purpose (REL-3).
    #[cfg(test)]
    v.push(test_panic_command());
    v
}

/// A command whose `run` panics on purpose, so the last-resort guard in `jobs::dispatch`
/// can be tested (REL-3). Test builds only: `commands::build` pushes it under `cfg(test)`.
#[cfg(test)]
fn test_panic_command() -> CommandSpec {
    #[allow(clippy::panic)]
    fn run(_: &mut Session, _: &Value) -> Result<Value> {
        panic!("test: this command panicked on purpose")
    }
    CommandSpec { id: "test.panic", label: "Test Panic", menu: &[], shortcut: None, params: r##"{}"##, enabled: |_: &Session| Ok(()), run, journal: false }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn duplicate_ids_keep_the_first_spec_and_listing_order() {
        let mut built = build().into_iter();
        let first = built.next().unwrap();
        let mut second = built.next().unwrap();
        let id = first.id;
        second.id = id;
        let registry = Registry::new(vec![first, second]);
        assert_eq!(registry.by_id.get(id), Some(&0));
        assert_eq!(registry.specs.len(), 2);
    }
}

#[cfg(test)]
mod set_props_tests {
    use super::*;

    #[test]
    fn opacity_and_fill_outside_zero_to_one_are_refused_not_clamped() {
        // #5242: `55` (a percentage, as the Layer Style commands take) used to become 100 %.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.setProps", json!({"opacity": 0.55, "fill": 0.4})).unwrap();
        let layer = |s: &Session| {
            let d = s.active().unwrap();
            let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
            (l.opacity, l.fill_opacity)
        };
        assert_eq!(layer(&s), (0.55, 0.4));
        for bad in [json!({"opacity": 55}), json!({"opacity": -0.1}), json!({"fill": 1.5}), json!({"opacity": "50%"}), json!({"opacity": 0.5, "fill": 40})] {
            let e = s.execute("layer.setProps", bad.clone()).unwrap_err().to_string();
            assert!(e.contains("0..1"), "{bad}: {e}");
            assert_eq!(layer(&s), (0.55, 0.4), "{bad} left the layer alone");
        }
        s.execute("layer.setProps", json!({"opacity": 0, "fill": 1})).unwrap();
        assert_eq!(layer(&s), (0.0, 1.0));
    }
}

#[cfg(test)]
mod parse_hex_tests {
    use super::parse_hex;

    #[test]
    fn signs_and_other_non_hex_characters_are_rejected() {
        for bad in ["#+f+f+f", "+f+f+f+f", "#-f0000", "#gg0000", "#0f0f+f"] {
            assert_eq!(parse_hex(bad), None, "{bad:?} is not hex digits");
        }
        let g = 15.0 / 255.0;
        assert_eq!(parse_hex("#0f0f0f"), Some([g, g, g, 1.0]));
        assert_eq!(parse_hex("#0f0f0fff"), Some([g, g, g, 1.0]));
    }
}
