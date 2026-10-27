//! Vector commands: shape layers (`shape.*`), document paths (`path.*`, Paths panel), selection ⇄
//! path, and layer vector masks (`layer.vectorMask.*`).
//!
//! Coordinates are document pixels. Paths in parameters and results use a compact JSON form:
//!
//! ```json
//! {"subpaths":[{"closed":true,"op":"combine|subtract|intersect|exclude",
//!               "knots":[[x,y], {"anchor":[x,y],"in":[x,y],"out":[x,y],"smooth":true}, …]}],
//!  "fillRule":"nonzero|evenodd", "inverted":false}
//! ```
//!
//! A knot given as `[x, y]` is a corner with retracted handles; `in`/`out` default to the
//! anchor. The serde form of `photocraft_doc::Path` is accepted too. Colours are `"#rrggbb"`,
//! `"#rrggbbaa"` or `[r,g,b,a]` (0..1); opacities are percentages (0..100).
//!
//! Every edit re-renders the shape layer's cache with `photocraft-vector` in one undoable step.
//! PSD blocks stay byte-identical while they still decode to the model (see `photocraft-io`).

use photocraft_algo::selection::{self as sel, SelectionMode};
use photocraft_color::{BlendMode, Color};
use photocraft_doc::{
    ClippingPath, Document, Fill, FillRule, GradientStyle, Knot, Layer, LayerContent, LayerId, LineCap, LineJoin, LiveShape, NamedPath, Path, PathOp,
    ShapeLayer, ShapeStroke, StrokeAlign, Subpath, VectorMask,
};
use photocraft_geom::{Affine, Point, Rect};
use photocraft_vector as vector;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, blend_from_str};
use crate::{EngineError, Result, Session};

/// Keys PSD uses for a shape layer's vector data; dropped when a shape is rasterized.
const SHAPE_BLOCKS: [&[u8; 4]; 8] = [b"vmsk", b"vsms", b"vogk", b"vstk", b"vscg", b"SoCo", b"GdFl", b"PtFl"];

pub(crate) fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

pub(crate) fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.filter(|id| d.doc.layer(*id).is_some()).map(|_| ()).ok_or_else(|| "no active layer".into())
}

fn has_selection(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.doc.selection.is_some()).map(|_| ()).ok_or_else(|| "no selection".into())
}

// ---------------------------------------------------------------------------
// Specs
// ---------------------------------------------------------------------------

/// Params of the Layer › Combine Shapes operations.
const COMBINE_PARAMS: &str = r##"{"layer":id?,"subpath":index? (default all but the first)} (no layer/subpath and two or more shape layers selected: merges them into the top-most, whose components above the bottom layer's take the operation)"##;

pub(crate) const PATH_FORM: &str = r##"path: {"subpaths":[{"closed":bool=true,"op":"combine|subtract|intersect|exclude","knots":[[x,y] | {"anchor":[x,y],"in":[x,y],"out":[x,y],"smooth":bool}]}],"fillRule":"nonzero|evenodd","inverted":bool}"##;

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:expr, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

mod params;
mod paths;
mod shape;
mod vector_mask;

pub(crate) use params::{layer_id, nums, pt};
pub use params::{parse_path, parse_stroke, path_json};
pub use paths::STROKE_PATH_TOOLS;
use paths::{
    clipping_path_clear, clipping_path_set, path_delete, path_fill, path_rename, path_set, path_stroke, path_to_selection, path_transform, paths_list,
    resolve_path, select_to_work_path,
};
pub(crate) use paths::{is_work, path_mut};
use shape::{
    can_paste_shape_fill, can_paste_shape_stroke, combine_shapes, copy_shape_fill, copy_shape_stroke, has_shape_fill, has_shape_stroke, merge_components,
    paste_shape_fill, paste_shape_stroke, selection_to_shape, shape_create, shape_edit, shape_info, shape_rasterize,
};
pub(crate) use shape::{map_path, map_shape, shift_shape, with_shape};
pub use shape::{preview_shape_stroke, refresh_shape, shape_path, transform_shape, translate_vectors};
use vector_mask::{toggle_vector_mask, vector_mask_add, vector_mask_delete, vector_mask_edit, vector_mask_info, vector_mask_rasterize, with};

pub(crate) fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

pub fn specs() -> Vec<CommandSpec> {
    let stroke = r##""stroke":{"width":px,"color":"#rrggbb"|fill,"opacity":0..100,"align":"inside|center|outside","cap":"butt|round|square","join":"miter|round|bevel","miterLimit":n,"dashes":[multiples of width],"dashOffset":n}|null"##;
    let fill = r##""fill":"#rrggbb"|[r,g,b,a]|{"gradient":{"stops":[[t,"#hex"|[r,g,b,a]|stored Color]],"angle":deg,"scale":%,"style":"linear|radial|angle|reflected|diamond","reverse":bool,"opacityStops":[[t,0..1]]?,"midpoints":[0..1]?,"offset":[x,y]?,"dither":bool?,"align":bool?}}|{"pattern":name}|null (no fill)"##;
    vec![
        spec!(
            "shape.create",
            "New Shape Layer",
            ["Layer", "New"],
            leak(format!(
                r##"{{"kind":"rect|roundedRect|ellipse|polygon|star|line|path"="rect","rect":[x,y,w,h] (rect/roundedRect/ellipse/polygon/star),"radii":[tl,tr,br,bl]|r (roundedRect=10),"sides":3..100=5,"starRatio":0..1 (star=0.5),"cornerRadius":px=0 (polygon/star: rounds every vertex; the Triangle tool is a 3-sided polygon),"from":[x,y],"to":[x,y],"weight":px=1 (line),"path":{{…}} (kind path),{fill}=foreground,{stroke}=none,"name":str?,"addTo":layerId? + "op":"combine|subtract|intersect|exclude" (append to an existing shape layer)}} → shape.info. {PATH_FORM}"##
            )),
            has_doc,
            shape_create
        ),
        spec!(
            "shape.edit",
            "Edit Shape",
            [],
            leak(format!(
                r##"{{"layer":id?,"path":{{…}}? (replaces; drops live shape),"kind":str?,"rect":[x,y,w,h]?,"radii":[tl,tr,br,bl]|r?,"sides":n?,"starRatio":r?,"cornerRadius":px?,"from":[x,y]?,"to":[x,y]?,"weight":px? (live-shape params regenerate the path),"move":[dx,dy]?,"transform":[a,b,c,d,e,f]?,"op":"combine|…"? + "subpath":index? (default all but the first),"fillRule":"nonzero|evenodd"?,{fill}?,{stroke}? (merged into the current stroke),"name":str?}} → shape.info"##
            )),
            has_layer,
            shape_edit
        ),
        CommandSpec {
            id: "shape.info",
            label: "Shape Layer Info",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?} → {layer,name,kind,path,fill,stroke,live,bounds:[x,y,w,h]}"##,
            enabled: has_layer,
            run: |s, p| {
                let id = layer_id(s, p)?;
                shape_info(s, id)
            },
            journal: false,
        },
        spec!("shape.rasterize", "Rasterize Shape", ["Layer", "Rasterize"], r##"{"layer":id?}"##, has_layer, shape_rasterize),
        CommandSpec {
            id: "path.list",
            label: "List Paths",
            menu: &[],
            shortcut: None,
            params: r##"{} → {paths:[{name,knots,subpaths}],workPath,clippingPath,layerPath (active layer's shape path / vector mask)}"##,
            enabled: has_doc,
            run: |s, _| paths_list(s),
            journal: false,
        },
        CommandSpec {
            id: "path.info",
            label: "Path Info",
            menu: &[],
            shortcut: None,
            params: r##"{"name":str|"work"|"layer"="work"} → {name,path}"##,
            enabled: has_doc,
            run: |s, p| {
                let name = p.get("name").and_then(Value::as_str);
                let path = resolve_path(s, name)?;
                Ok(json!({ "name": name.unwrap_or("work"), "path": path_json(&path) }))
            },
            journal: false,
        },
        spec!(
            "path.set",
            "Set Path",
            [],
            leak(format!(
                r##"{{"name":str|"work"="work","path":{{…}},"op":"combine|subtract|intersect|exclude"? (append to the existing path with this op instead of replacing)}}. {PATH_FORM}"##
            )),
            has_doc,
            path_set
        ),
        spec!("path.delete", "Delete Path", [], r##"{"name":str|"work"="work"}"##, has_doc, path_delete),
        spec!(
            "path.transform",
            "Free Transform Path",
            [],
            r##"{"name":str|"work"|"layer"="work","layer":id? (with layer target),"matrix":[a,b,c,d,e,f] | "quad":[[x,y]×4] (Distort/Perspective: where the frame's corners go, clockwise from top-left) | "style"|"mesh"|"warp":… (Warp, as edit.transform.warp) | "translateX":px?,"translateY":px?,"scaleX":factor?,"scaleY":factor?,"angle":degrees?,"rect":[x0,y0,x1,y1]? (the quad's or warp's frame; default = path control bounds)} (numeric transforms pivot on path bounds center)"##,
            has_doc,
            path_transform
        ),
        spec!(
            "path.clippingPath.set",
            "Clipping Path",
            [],
            r##"{"name":savedPathName,"flatness":0..100=0} (PSD export clipping path)"##,
            has_doc,
            clipping_path_set
        ),
        spec!("path.clippingPath.clear", "Clear Clipping Path", [], r##"{}"##, has_doc, clipping_path_clear),
        spec!("path.style.copyFill", "Copy Fill", [], r##"{} (copies active shape layer fill)"##, has_shape_fill, copy_shape_fill),
        spec!("path.style.copyStroke", "Copy Complete Stroke", [], r##"{} (copies active shape layer stroke)"##, has_shape_stroke, copy_shape_stroke),
        spec!(
            "path.style.pasteFill",
            "Paste Fill",
            [],
            r##"{"layer":id?} (pastes copied fill onto active shape layer)"##,
            can_paste_shape_fill,
            paste_shape_fill
        ),
        spec!(
            "path.style.pasteStroke",
            "Paste Complete Stroke",
            [],
            r##"{"layer":id?} (pastes copied stroke onto active shape layer)"##,
            can_paste_shape_stroke,
            paste_shape_stroke
        ),
        spec!("path.rename", "Rename Path", [], r##"{"name":str|"work"="work","to":str} (renaming the work path saves it)"##, has_doc, path_rename),
        // ⌘↩ / Ctrl+Enter, as in Photoshop (the UI loads the path selected in the Paths panel, #306).
        CommandSpec {
            id: "path.toSelection",
            label: "Make Selection from Path",
            menu: &[],
            shortcut: Some("Cmd+Enter"),
            params: r##"{"name":str|"work"|"layer"="work","feather":px=0,"antiAlias":bool=true,"mode":"replace|add|subtract|intersect"="replace"}"##,
            enabled: has_doc,
            run: path_to_selection,
            journal: true,
        },
        spec!("select.toWorkPath", "Make Work Path", [], r##"{"tolerance":0.5..10 px=2} → {subpaths,knots}"##, has_selection, select_to_work_path),
        spec!(
            "path.fill",
            "Fill Path",
            [],
            r##"{"name":str|"work"|"layer"="work","layer":id? (pixel layer, default active),"color":"#rrggbb"=foreground,"opacity":0..100=100,"mode":"normal|multiply|…"="normal","feather":px=0,"antiAlias":bool=true}"##,
            has_layer,
            path_fill
        ),
        spec!(
            "path.stroke",
            "Stroke Path",
            [],
            r##"{"name":str|"work"|"layer"="work","layer":id? (pixel layer),"tool":"brush|pencil|eraser|backgroundEraser|cloneStamp|patternStamp|healingBrush|spotHealingBrush|historyBrush|blur|sharpen|smudge|dodge|burn|sponge|colorReplacement|mixerBrush"="brush","simulatePressure":bool=false (taper both ends when the brush follows pen pressure),"size":0.5..5000 px?,"hardness":0..1?,"opacity":0..100?,"color":"#rrggbb"=foreground} (current brush settings otherwise; tools past the Eraser run their paint.* command along the path and take its params)"##,
            has_layer,
            path_stroke
        ),
        spec!(
            "layer.vectorMask.add",
            "Add Vector Mask",
            ["Layer", "Vector Mask"],
            r##"{"layer":id?,"path":{…}? | "name":str|"work"? (copy a document path),"hide":bool=false (Hide All = inverted)} → vector mask info. Without a path the mask reveals all."##,
            has_layer,
            |s, p| vector_mask_add(s, p, false)
        ),
        spec!(
            "layer.vectorMask.fromPath",
            "Vector Mask from Current Path",
            ["Layer", "Vector Mask"],
            r##"{"layer":id?,"name":str|"work"="work"}"##,
            has_layer,
            |s, p| vector_mask_add(s, p, true)
        ),
        spec!(
            "layer.vectorMask.edit",
            "Edit Vector Mask",
            [],
            leak(format!(
                r##"{{"layer":id?,"path":{{…}}?,"enabled":bool?,"linked":bool?,"density":0..100?,"feather":0..1000? (pixels),"invert":true?}}. On a shape layer (whose path is its vector mask) only density and feather apply: they soften its fill, which shows beyond the path at 100 − density %. {PATH_FORM}"##
            )),
            has_layer,
            vector_mask_edit
        ),
        spec!("layer.vectorMask.delete", "Delete Vector Mask", ["Layer", "Vector Mask"], r##"{"layer":id?}"##, has_layer, vector_mask_delete),
        spec!(
            "layer.rasterize.vectorMask",
            "Rasterize Vector Mask",
            [],
            r##"{"layer":id?} (multiplied into the layer mask)"##,
            has_layer,
            vector_mask_rasterize
        ),
        // Menu ids of Photoshop's Layer › Vector Mask / Rasterize / Combine Shapes.
        spec!("layer.vectorMask.revealAll", "Vector Mask: Reveal All", [], r##"{"layer":id?}"##, has_layer, |s, p| vector_mask_add(
            s,
            &with(p, "hide", json!(false)),
            false
        )),
        spec!("layer.vectorMask.hideAll", "Vector Mask: Hide All", [], r##"{"layer":id?}"##, has_layer, |s, p| vector_mask_add(
            s,
            &with(p, "hide", json!(true)),
            false
        )),
        spec!("layer.vectorMask.currentPath", "Vector Mask: Current Path", [], r##"{"layer":id?,"name":str|"work"="work"}"##, has_layer, |s, p| {
            vector_mask_add(s, p, true)
        }),
        spec!("layer.vectorMask.enabled", "Enable Vector Mask", [], r##"{"layer":id?,"enabled":bool? (default: toggle)}"##, has_layer, |s, p| {
            toggle_vector_mask(s, p, "enabled")
        }),
        spec!("layer.vectorMask.linked", "Link Vector Mask", [], r##"{"layer":id?,"linked":bool? (default: toggle)}"##, has_layer, |s, p| toggle_vector_mask(
            s, p, "linked"
        )),
        // Without `layer`, every selected shape layer (the shared Rasterize logic calls back here with one).
        spec!("layer.rasterize.shape", "Rasterize Shape", [], r##"{"layer":id?} (no layer: every selected shape layer)"##, has_layer, |s, p| {
            if p.get("layer").is_some() { shape_rasterize(s, p) } else { crate::extra_cmds::rasterize(s, p, Some("shape")) }
        }),
        // With two or more shape layers selected (and no `layer`/`subpath`), these merge them into
        // the top-most one, as in Photoshop; with one, they set the operation of its components.
        spec!("layer.combineShapes.unite", "Unite Shapes", [], COMBINE_PARAMS, has_layer, |s, p| combine_shapes(s, p, "combine", "Unite Shapes")),
        spec!("layer.combineShapes.subtractFrontShape", "Subtract Front Shape", [], COMBINE_PARAMS, has_layer, |s, p| combine_shapes(
            s,
            p,
            "subtract",
            "Subtract Front Shape"
        )),
        spec!("layer.combineShapes.intersectShapeAreas", "Intersect Shape Areas", [], COMBINE_PARAMS, has_layer, |s, p| combine_shapes(
            s,
            p,
            "intersect",
            "Intersect Shape Areas"
        )),
        spec!("layer.combineShapes.excludeOverlappingShapes", "Exclude Overlapping Shapes", [], COMBINE_PARAMS, has_layer, |s, p| {
            combine_shapes(s, p, "exclude", "Exclude Overlapping Shapes")
        }),
        spec!(
            "layer.combineShapes.mergeShapeComponents",
            "Merge Shape Components",
            [],
            r##"{"layer":id?,"tolerance":px=0.1} (bakes the path operations into plain combined outlines, traced from the exact coverage; no layer and several shape layers selected: merges them into the top-most first)"##,
            has_layer,
            merge_components
        ),
        spec!("select.convertToShape", "Convert to Shape", [], r##"{"tolerance":0.5..10 px=2} → shape.info"##, has_selection, selection_to_shape),
        CommandSpec {
            id: "layer.vectorMask.info",
            label: "Vector Mask Info",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?} → {layer,path,enabled,linked,density,feather}"##,
            enabled: has_layer,
            run: |s, p| {
                let id = layer_id(s, p)?;
                vector_mask_info(s, id)
            },
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
