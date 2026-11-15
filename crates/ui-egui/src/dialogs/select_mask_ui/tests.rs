use super::*;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};

/// A 160 × 100 document: warm on the left (x < 80), cool on the right, with the top-left
/// 20 × 20 selected.
fn app_with_selection() -> PhotocraftApp {
    app_sized(160, 100)
}

fn app_sized(w: i32, h: i32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": w, "height": h, "depth": 8})).unwrap();
    app.session
        .edit("paint", |doc, _| {
            let s = doc.layers[0].surface_mut().unwrap();
            s.fill_rect(Rect::new(0, 0, w / 2, h), &[0.85, 0.55, 0.25, 1.0]);
            s.fill_rect(Rect::new(w / 2, 0, w, h), &[0.15, 0.35, 0.75, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.session.prefs.edit(|p| {
        p.dialogs.remove(REMEMBERED);
    });
    app.sync_views();
    app
}

fn ws(app: &PhotocraftApp) -> &SelectMaskWorkspace {
    app.distort.select_mask.as_ref().unwrap()
}

fn ui(app: &mut PhotocraftApp, v: Value) -> Value {
    control(app, &v).unwrap()
}

fn selected(app: &mut PhotocraftApp) -> u64 {
    ui(app, json!({"waitPreview": true}))["selectedProxyPixels"].as_u64().unwrap()
}

fn cov(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    app.session.active().unwrap().doc.selection.as_ref().map_or(0.0, |s| s.sample_channel(x, y, 0))
}

/// Photoshop opens on Quick Selection adding, the Onion Skin view, Real-Time Refinement on, and
/// output to the selection.
#[test]
fn opens_like_photoshop() {
    let mut app = app_with_selection();
    crate::menus::invoke(&mut app, &egui::Context::default(), "select.selectAndMask", json!({})).unwrap();
    let d = ws(&app).describe();
    assert_eq!((d["tool"].as_str(), d["mode"].as_str(), d["size"].as_f64()), (Some("quick"), Some("add"), Some(30.0)));
    let s = &d["settings"];
    assert_eq!((s["view"].as_str(), s["output"].as_str()), (Some("onionSkin"), Some("selection")));
    assert_eq!((s["realTimeRefinement"].as_bool(), s["highQualityPreview"].as_bool(), s["remember"].as_bool()), (Some(true), Some(false), Some(false)));
    assert_eq!((s["opacity"]["onionSkin"].as_f64(), s["opacity"]["onBlack"].as_f64()), (Some(50.0), Some(100.0)));
    assert_eq!(selected(&mut app), 400, "the 20 × 20 selection");
    assert!(ws(&app).describe()["previewReady"].as_bool().unwrap());
}

#[test]
fn opening_needs_a_selection_or_a_layer_mask() {
    let mut app = app_with_selection();
    app.session.execute("select.deselect", json!({})).unwrap();
    assert!(open(&mut app).is_err());
    assert!(app.distort.select_mask.is_none());
    // `ui` addresses an open workspace only.
    let e = crate::distort_ui::menu(&mut app, &egui::Context::default(), "select.selectAndMask", &json!({"ui": {}})).unwrap();
    assert!(e.is_err());
}

/// Every tool's stroke is an edit (the Refine Edge Brush's a stroke), each one undoable.
#[test]
fn tools_make_edits_and_undo_and_redo_them() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let before = selected(&mut app);
    // Quick Selection across the cool half grows the selection over it.
    ui(&mut app, json!({"stroke": {"points": [[120.0, 50.0], [130.0, 50.0]]}}));
    let quick = selected(&mut app);
    assert!(quick > before + 3000, "{before} → {quick}");
    // Brush, Lasso and Polygonal Lasso subtract with Alt.
    ui(&mut app, json!({"tool": "brush", "size": 10, "hardness": 50, "stroke": {"points": [[100.0, 10.0], [100.0, 90.0]], "alt": true}}));
    let brushed = selected(&mut app);
    assert!(brushed < quick, "{quick} → {brushed}");
    ui(&mut app, json!({"tool": "lasso", "mode": "subtract", "stroke": {"points": [[140.0, 0.0], [160.0, 0.0], [160.0, 100.0], [140.0, 100.0]]}}));
    ui(&mut app, json!({"tool": "polygonalLasso", "mode": "add", "stroke": {"points": [[30.0, 30.0], [50.0, 30.0], [50.0, 50.0]]}}));
    ui(&mut app, json!({"tool": "refineEdge", "stroke": {"points": [[80.0, 20.0], [80.0, 80.0]]}}));
    let d = ws(&app).describe();
    assert_eq!((d["edits"].as_u64(), d["strokes"].as_u64(), d["undo"].as_u64()), (Some(4), Some(1), Some(5)));
    let e = &ws(&app).edits;
    assert_eq!(
        (e[1]["tool"].as_str(), e[1]["mode"].as_str(), e[1]["hardness"].as_f64(), e[1]["size"].as_f64()),
        (Some("brush"), Some("subtract"), Some(50.0), Some(10.0))
    );
    assert_eq!((e[2]["tool"].as_str(), e[2]["mode"].as_str()), (Some("lasso"), Some("subtract")));
    assert_eq!((e[3]["tool"].as_str(), e[3]["points"].as_array().map(Vec::len)), (Some("lasso"), Some(3)));
    // Undo everything but the Quick Selection stroke, then redo the Brush's.
    for _ in 0..4 {
        ui(&mut app, json!({"undo": true}));
    }
    assert_eq!(selected(&mut app), quick);
    ui(&mut app, json!({"redo": true}));
    assert_eq!(selected(&mut app), brushed);
    // A new edit drops what could be redone.
    ui(&mut app, json!({"invert": true}));
    assert_eq!(ws(&app).describe()["redo"].as_u64(), Some(0));
}

/// Object Selection: a click picks the object under it, a drag the object in the rectangle.
#[test]
fn object_selection_clicks_and_drags() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ws_mut(&mut app).zoom = 1.0;
    ui(&mut app, json!({"tool": "object", "stroke": {"points": [[120.0, 50.0], [121.0, 50.0]]}}));
    ui(&mut app, json!({"stroke": {"points": [[90.0, 10.0], [150.0, 90.0]]}}));
    let e = &ws(&app).edits;
    assert_eq!(e[0]["point"], json!([120.0, 50.0]));
    assert_eq!(e[1]["rect"], json!([90.0, 10.0, 60.0, 80.0]));
}

fn ws_mut(app: &mut PhotocraftApp) -> &mut SelectMaskWorkspace {
    app.distort.select_mask.as_mut().unwrap()
}

/// Select Subject, Clear Selection and Invert are edits too; Reset drops everything at once,
/// undoably.
#[test]
fn buttons_and_reset() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"clearSelection": true}));
    assert_eq!(selected(&mut app), 0);
    ui(&mut app, json!({"invert": true}));
    assert_eq!(selected(&mut app), 160 * 100);
    ui(&mut app, json!({"selectSubject": true}));
    assert_eq!(ws(&app).edits.last().and_then(|e| e["tool"].as_str()), Some("subject"));
    ui(&mut app, json!({"radius": 12, "view": "blackWhite", "remember": true}));
    let d = ui(&mut app, json!({"reset": true}));
    assert_eq!((d["edits"].as_u64(), d["settings"]["radius"].as_f64(), d["settings"]["view"].as_str()), (Some(0), Some(0.0), Some("onionSkin")));
    assert_eq!(d["settings"]["remember"].as_bool(), Some(true), "Reset keeps Remember Settings");
    assert_eq!(selected(&mut app), 400);
    let d = ui(&mut app, json!({"undo": true}));
    assert_eq!((d["edits"].as_u64(), d["settings"]["radius"].as_f64()), (Some(3), Some(12.0)));
}

/// OK runs `select.refineEdge` with the edits, strokes and settings: one history step, the same
/// result as the command with those params, and the workspace closes.
#[test]
fn ok_is_one_step_and_matches_the_command() {
    let mut app = app_with_selection();
    let mut direct = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"radius": 2, "stroke": {"points": [[120.0, 50.0]]}}));
    ui(&mut app, json!({"tool": "brush", "stroke": {"points": [[40.0, 60.0], [60.0, 60.0]]}}));
    let params = commit_params(ws(&app));
    let steps = app.session.active().unwrap().history.past_len();
    let r = ui(&mut app, json!({"commit": true}));
    assert_eq!(r["committed"], json!(true));
    assert!(app.distort.select_mask.is_none(), "OK closes");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Select and Mask"));
    direct.session.execute(CMD, params).unwrap();
    for (x, y) in [(5, 5), (120, 50), (50, 60), (50, 80), (79, 50)] {
        assert_eq!(cov(&app, x, y), cov(&direct, x, y), "({x}, {y})");
    }
    assert!(cov(&app, 120, 50) > 0.5 && cov(&app, 50, 60) > 0.5 && cov(&app, 50, 80) < 0.5);
}

/// Cancel leaves the document as it was.
#[test]
fn cancel_changes_nothing() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"invert": true, "radius": 10}));
    let steps = app.session.active().unwrap().history.past_len();
    let r = ui(&mut app, json!({"cancel": true}));
    assert_eq!(r["cancelled"], json!(true));
    assert!(app.distort.select_mask.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert!(cov(&app, 5, 5) > 0.99 && cov(&app, 100, 50) < 0.01);
}

/// Remember Settings keeps the settings for the next time; without it the workspace opens on
/// Photoshop's defaults.
#[test]
fn remember_settings() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"remember": true, "radius": 7, "view": "overlay", "overlayColor": [0, 0, 255], "commit": true}));
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    open(&mut app).unwrap();
    let s = ws(&app).describe()["settings"].clone();
    assert_eq!((s["radius"].as_f64(), s["view"].as_str(), s["overlayColor"].clone()), (Some(7.0), Some("overlay"), json!([0, 0, 255])));
    ui(&mut app, json!({"remember": false, "commit": true}));
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    open(&mut app).unwrap();
    let s = ws(&app).describe()["settings"].clone();
    assert_eq!((s["radius"].as_f64(), s["view"].as_str()), (Some(0.0), Some("onionSkin")));
}

/// Decontaminate Colors makes new pixels, so it outputs to a new layer (as Photoshop does).
#[test]
fn decontaminate_outputs_to_a_new_layer() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let d = ui(&mut app, json!({"decontaminate": true}));
    assert_eq!(d["settings"]["output"].as_str(), Some("newLayerWithMask"));
    let d = ui(&mut app, json!({"output": "newDocument"}));
    assert_eq!(d["settings"]["output"].as_str(), Some("newDocument"));
    let d = ui(&mut app, json!({"output": "selection"}));
    assert_eq!(d["settings"]["output"].as_str(), Some("newLayerWithMask"));
}

/// A layer mask without a selection (or the targeted mask) is refined and output to the mask.
#[test]
fn a_layer_mask_opens_on_the_mask() {
    let mut app = app_with_selection();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    // Revealing the selection uses it up (#8381), leaving the mask alone.
    app.session.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_none());
    open(&mut app).unwrap();
    assert_eq!(ws(&app).describe()["settings"]["output"].as_str(), Some("layerMask"));
    assert_eq!(selected(&mut app), 400, "the mask is what is shown");
}

/// Bad control params are errors, never a panic, and change nothing.
#[test]
fn hostile_control_params_are_errors() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    for bad in [
        json!({"tool": "wand"}),
        json!({"mode": "intersect"}),
        json!({"view": "sideways"}),
        json!({"output": "printer"}),
        json!({"indicates": "both"}),
        json!({"stroke": {"points": "x"}}),
        json!({"stroke": {"points": [[1.0]]}}),
    ] {
        assert!(control(&mut app, &bad).is_err(), "{bad}");
    }
    for odd in [json!({"size": -5}), json!({"size": 1e12}), json!({"hardness": 500}), json!({"radius": 1e9, "opacity": -3}), json!({"stroke": {"points": []}})]
    {
        control(&mut app, &odd).unwrap();
    }
    let d = ws(&app).describe();
    assert_eq!((d["tool"].as_str(), d["edits"].as_u64()), (Some("quick"), Some(0)));
    assert_eq!((d["size"].as_f64(), d["hardness"].as_f64(), d["settings"]["radius"].as_f64()), (Some(5000.0), Some(100.0), Some(250.0)));
    assert!(control(&mut app, &json!({"waitPreview": true})).is_ok(), "the clamped settings still preview");
}

/// High Quality Preview runs on the document at full size; otherwise on a copy at most 1600 px.
#[test]
fn high_quality_preview_is_full_size() {
    let mut app = app_sized(3300, 60);
    open(&mut app).unwrap();
    assert_eq!(ws(&app).describe()["proxyScale"].as_u64(), Some(3));
    let reduced = selected(&mut app);
    assert!((40..=60).contains(&reduced), "{reduced} reduced px of the 20 × 20 selection");
    assert_eq!(ui(&mut app, json!({"highQualityPreview": true}))["proxyScale"].as_u64(), Some(1));
    assert_eq!(selected(&mut app), 400);
}

/// Edits scale with the reduced copy: points, rectangles and sizes by 1 / k.
#[test]
fn edits_scale_to_the_reduced_copy() {
    let e = json!({"tool": "quick", "points": [[30.0, 60.0]], "size": 30.0, "rect": [3, 6, 9, 12], "point": [15, 3]});
    assert_eq!(scale_edit(&e, 3), json!({"tool": "quick", "points": [[10.0, 20.0]], "size": 10.0, "rect": [1.0, 2.0, 3.0, 4.0], "point": [5.0, 1.0]}));
    assert_eq!(scale_edit(&json!({"size": 2.0}), 4)["size"], json!(1.0), "never below a pixel");
    assert_eq!(scale_edit(&json!("x"), 2), json!("x"));
}

/// With Real-Time Refinement the stroke being drawn is in the preview before it is released.
#[test]
fn real_time_refinement_previews_the_stroke_being_drawn() {
    for real_time in [true, false] {
        let mut app = app_with_selection();
        open(&mut app).unwrap();
        ui(&mut app, json!({"realTimeRefinement": real_time}));
        let before = selected(&mut app);
        crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: 120.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
        crate::distort_ui::pointer(&mut app, ToolEvent::Move { x: 130.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
        let during = selected(&mut app);
        assert_eq!(during > before, real_time, "real time {real_time}: {before} → {during}");
        // The preview knows how much of the stroke it shows: the rest is drawn over it.
        assert_eq!(ws(&app).preview.trail, if real_time { 2 } else { 0 });
        crate::distort_ui::pointer(&mut app, ToolEvent::Up { x: 130.0, y: 50.0 }, Modifiers::NONE);
        assert!(ws(&app).trail.is_some(), "the released stroke stays drawn until its preview comes");
        assert!(selected(&mut app) > before);
        assert_eq!(ws(&app).edits.len(), 1);
        assert!(ws(&app).trail.is_none() && ws(&app).preview.trail == usize::MAX, "then the preview shows all of it");
    }
}

/// A stroke refused at the engine's limits leaves nothing drawn: no trail, and a preview without it.
#[test]
fn a_refused_stroke_leaves_no_trail_or_preview() {
    for tool in ["quick", "refineEdge"] {
        let mut app = app_with_selection();
        open(&mut app).unwrap();
        ui(&mut app, json!({"tool": tool}));
        let w = ws_mut(&mut app);
        if tool == "quick" {
            w.edits = vec![json!({"tool": "invert"}); MAX_EDITS];
        } else {
            w.strokes = vec![json!({"points": []}); MAX_STROKES];
        }
        crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: 120.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
        crate::distort_ui::pointer(&mut app, ToolEvent::Move { x: 130.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
        let want = ws(&app).preview.want;
        crate::distort_ui::pointer(&mut app, ToolEvent::Up { x: 130.0, y: 50.0 }, Modifiers::NONE);
        let w = ws(&app);
        assert!(w.error.as_deref().is_some_and(|e| e.starts_with("at most")), "{tool}: {:?}", w.error);
        assert!(w.trail.is_none(), "{tool}: the refused stroke is not drawn");
        assert!(w.preview.want > want, "{tool}: a new preview drops the refused stroke");
    }
}

/// Opening again while the workspace is open keeps its work and says why.
#[test]
fn opening_again_keeps_the_open_workspace() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"stroke": {"points": [[120.0, 50.0], [130.0, 50.0]]}}));
    let e = crate::menus::invoke(&mut app, &egui::Context::default(), "select.selectAndMask", json!({}));
    assert!(e.is_err_and(|e| e.contains("already open")));
    assert_eq!(ws(&app).edits.len(), 1, "the edit survives");
}

/// The preview keeps the selection after the edits it applied and starts the next from there:
/// after undo it starts over.
#[test]
fn the_preview_reuses_the_edits_it_applied() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"stroke": {"points": [[120.0, 50.0]]}}));
    selected(&mut app);
    assert_eq!(ws(&app).preview.cache.as_ref().map(|c| c.edits.len()), Some(1));
    ui(&mut app, json!({"tool": "brush", "stroke": {"points": [[40.0, 60.0]]}}));
    let both = selected(&mut app);
    assert_eq!(ws(&app).preview.cache.as_ref().map(|c| c.edits.len()), Some(2));
    ui(&mut app, json!({"undo": true}));
    let one = selected(&mut app);
    assert!(one < both);
    assert_eq!(ws(&app).preview.cache.as_ref().map(|c| c.edits.len()), Some(1));
}

/// The selection the edits made on an older document is not reused: a Quick Selection stroke
/// on a repainted image previews what OK would commit.
#[test]
fn the_preview_recomputes_the_edits_after_the_document_changes() {
    let warm = |app: &mut PhotocraftApp| {
        app.session
            .edit("paint", |doc, _| {
                doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 160, 100), &[0.85, 0.55, 0.25, 1.0]);
                Ok(())
            })
            .unwrap();
    };
    let stroke = json!({"stroke": {"points": [[120.0, 50.0], [130.0, 50.0]]}});
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, stroke.clone());
    let two_tone = selected(&mut app);
    // The pixels change under the workspace (a control-channel command) before it redraws.
    warm(&mut app);
    let after = selected(&mut app);
    let mut fresh = app_with_selection();
    warm(&mut fresh);
    open(&mut fresh).unwrap();
    ui(&mut fresh, stroke);
    let expected = selected(&mut fresh);
    assert_ne!(two_tone, expected, "the repaint changes what the stroke selects");
    assert_eq!(after, expected);
    assert_eq!(ws(&app).preview.cache.as_ref().map(|c| c.revision), app.session.active().map(|s| s.revision));
}

/// A request with a bad field is refused whole: the valid fields before it change nothing either.
#[test]
fn a_refused_request_changes_nothing() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let before = ws(&app).describe();
    for bad in [
        json!({"tool": "brush", "mode": "invalid"}),
        json!({"tool": "brush", "mode": "subtract", "size": 80, "radius": 40, "stroke": {"points": "x"}}),
        json!({"hardness": 10, "sampleAllLayers": true, "hideView": true, "stroke": {"points": [[1.0]]}}),
    ] {
        assert!(control(&mut app, &bad).is_err(), "{bad}");
        assert_eq!(ws(&app).describe(), before, "{bad}");
        assert!(!ws(&app).hide_view && !ws(&app).sample_all, "{bad}");
    }
}

/// With nothing selected OK refines nothing, so Show Edge shows no Refine Edge Brush stroke.
#[test]
fn show_edge_shows_no_stroke_without_a_selection() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"clearSelection": true, "showEdge": true}));
    let empty = selected(&mut app);
    ui(&mut app, json!({"tool": "refineEdge", "stroke": {"points": [[40.0, 20.0], [40.0, 80.0]]}}));
    assert_eq!(selected(&mut app), empty);
    ui(&mut app, json!({"commit": true}));
    assert_eq!(cov(&app, 40, 50), 0.0, "and OK selects nothing there");
}

/// A preview of pixels the document no longer has is never shown, even mid-stroke, and the old one
/// goes when the document changes.
#[test]
fn a_preview_of_an_old_revision_is_dropped() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    selected(&mut app);
    assert!(ws(&app).preview.pending.is_some());
    let j = job(&app).unwrap();
    let old = j.revision;
    let late = compute(j, None);
    app.session.execute("select.all", json!({})).unwrap();
    follow_revision(&mut app);
    assert!(ws(&app).preview.pending.is_none() && ws(&app).preview.tex.is_none(), "the old preview is gone");
    let w = ws_mut(&mut app);
    w.pressed = true;
    let generation = w.preview.want;
    take_result(w, generation, old, late);
    assert!(w.preview.pending.is_none() && w.preview.cache.is_none() && w.preview.shown != generation);
}

/// Sends `key` with `modifiers` through the workspace's key handling.
fn press(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
    let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let input = egui::RawInput { events: vec![ev], ..Default::default() };
    ctx.run_ui(input, |ui| keys(app, ui.ctx())).textures_delta.clear();
}

/// With the overlay colour's Color Picker open over the workspace, its keys (tool letters, undo)
/// belong to the picker, not to the tools under it.
#[test]
fn keys_wait_while_the_overlay_colour_picker_is_open() {
    let mut app = app_with_selection();
    let ctx = egui::Context::default();
    open(&mut app).unwrap();
    press(&mut app, &ctx, Key::R, Modifiers::NONE);
    assert_eq!(ws(&app).tool, SmTool::RefineEdge);
    crate::color_picker_ui::request_for_widget(&ctx, egui::Id::unique("overlay-swatch"), "Color Picker (Overlay Color)", [1.0, 0.0, 0.0]);
    crate::color_picker_ui::widget_tick(&mut app, &ctx);
    assert!(crate::color_picker_ui::any_open(&app));
    press(&mut app, &ctx, Key::B, Modifiers::NONE);
    assert_eq!(ws(&app).tool, SmTool::RefineEdge, "B went to the picker");
    app.ui.dialogs.clear();
    press(&mut app, &ctx, Key::B, Modifiers::NONE);
    assert_eq!(ws(&app).tool, SmTool::Brush);
}

/// Photoshop's keys: W, R, B, L, H, Z pick the tools (Shift+W and Shift+L cycle their slots),
/// [ and ] size the brush, the view letters, F, J and P the view, Ctrl+Z undoes.
#[test]
fn keys_like_photoshop() {
    let mut app = app_with_selection();
    let ctx = egui::Context::default();
    open(&mut app).unwrap();
    let tool = |app: &PhotocraftApp| ws(app).tool;
    for (key, mods, want) in [
        (Key::R, Modifiers::NONE, SmTool::RefineEdge),
        (Key::B, Modifiers::NONE, SmTool::Brush),
        (Key::W, Modifiers::NONE, SmTool::Quick),
        (Key::W, Modifiers::SHIFT, SmTool::Object),
        (Key::H, Modifiers::NONE, SmTool::Hand),
        (Key::W, Modifiers::NONE, SmTool::Object),
        (Key::L, Modifiers::NONE, SmTool::Lasso),
        (Key::L, Modifiers::SHIFT, SmTool::PolygonalLasso),
        (Key::Z, Modifiers::NONE, SmTool::Zoom),
        (Key::L, Modifiers::NONE, SmTool::PolygonalLasso),
        (Key::B, Modifiers::NONE, SmTool::Brush),
    ] {
        press(&mut app, &ctx, key, mods);
        assert_eq!(tool(&app), want, "{mods:?}+{key:?}");
    }
    press(&mut app, &ctx, Key::CloseBracket, Modifiers::NONE);
    assert_eq!(ws(&app).brush_size, 33.0);
    press(&mut app, &ctx, Key::OpenBracket, Modifiers::NONE);
    assert_eq!(ws(&app).brush_size, 30.0);
    let view = |app: &PhotocraftApp| ws(app).opts.view;
    for (key, want) in [
        (Key::K, Mode::BlackWhite),
        (Key::A, Mode::OnBlack),
        (Key::T, Mode::OnWhite),
        (Key::V, Mode::Overlay),
        (Key::Y, Mode::OnLayers),
        (Key::M, Mode::MarchingAnts),
        (Key::F, Mode::Overlay),
        (Key::O, Mode::OnionSkin),
    ] {
        press(&mut app, &ctx, key, Modifiers::NONE);
        assert_eq!(view(&app), want, "{key:?}");
    }
    press(&mut app, &ctx, Key::J, Modifiers::NONE);
    press(&mut app, &ctx, Key::P, Modifiers::NONE);
    assert!(ws(&app).opts.show_edge && ws(&app).opts.show_original);
    ui(&mut app, json!({"invert": true}));
    press(&mut app, &ctx, Key::Z, Modifiers::COMMAND);
    assert_eq!(ws(&app).edits.len(), 0);
    press(&mut app, &ctx, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(ws(&app).edits.len(), 1);
}

/// The Polygonal Lasso takes clicks: Enter closes the polygon (not OK), Esc drops it (not Cancel),
/// clicking the first vertex closes it too.
#[test]
fn polygonal_lasso_enter_and_escape() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ws_mut(&mut app).zoom = 1.0;
    ui(&mut app, json!({"tool": "polygonalLasso"}));
    let click = |app: &mut PhotocraftApp, x: f64, y: f64| crate::distort_ui::pointer(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    for (x, y) in [(30.0, 30.0), (60.0, 30.0), (60.0, 60.0)] {
        click(&mut app, x, y);
    }
    assert_eq!(ws(&app).describe()["polygon"].as_u64(), Some(3));
    assert!(enter(&mut app), "Enter closes the polygon");
    assert!(app.distort.select_mask.is_some(), "and leaves the workspace open");
    assert_eq!(ws(&app).edits.len(), 1);
    click(&mut app, 30.0, 30.0);
    click(&mut app, 60.0, 30.0);
    escape(&mut app);
    assert!(app.distort.select_mask.is_some() && ws(&app).poly.is_none(), "Esc drops the polygon only");
    for (x, y) in [(70.0, 70.0), (90.0, 70.0), (90.0, 90.0), (71.0, 71.0)] {
        click(&mut app, x, y);
    }
    assert_eq!(ws(&app).edits.len(), 2, "clicking the first vertex closes it");
    escape(&mut app);
    assert!(app.distort.select_mask.is_none(), "Esc with no polygon cancels");
}

/// The workspace draws: tools on the left, the options bar, the Properties panel with Photoshop's
/// sections and buttons; clicking a tool picks it, OK commits.
#[test]
fn renders_photoshops_layout() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    for label in [
        "Remember Settings",
        "View Mode",
        "Real-Time Refinement",
        "High Quality Preview",
        "Refine Mode",
        "Color Aware",
        "Edge Detection",
        "Smart Radius",
        "Global Refinements",
        "Clear Selection",
        "Invert",
        "Output Settings",
        "Decontaminate Colors",
        "Select Subject",
        "Refine Hair",
        "Sample All Layers",
        "OK",
        "Cancel",
    ] {
        assert!(h.query_by_label_contains(label).is_some(), "{label} is shown");
    }
    assert!(h.get_by_label("Refine Hair").accesskit_node().is_disabled(), "Refine Hair is greyed out");
    h.get_by_label("Brush Tool (B)").click();
    h.run_steps(2);
    assert_eq!(h.state().distort.select_mask.as_ref().map(|w| w.tool), Some(SmTool::Brush));
    assert!(h.query_by_label_contains("Hardness").is_some(), "the Brush's options bar");
    h.get_by_label("Invert").click();
    h.run_steps(2);
    assert_eq!(h.state().distort.select_mask.as_ref().map(|w| w.edits.len()), Some(1));
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(h.state().distort.select_mask.is_none(), "OK closes");
    let app = h.state();
    assert!(cov(app, 5, 5) < 0.01 && cov(app, 100, 50) > 0.99, "the inverted selection");
}

/// Panning shows Photoshop's hand: open while Space is held (or the Hand tool is picked), closed
/// while dragging; and Space doesn't press the panel button clicked last.
#[test]
fn panning_shows_the_hand() {
    use egui::{PointerButton, Pos2};
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"tool": "brush"}));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::distort_ui::keys(app, &ctx);
            keys(app, &ctx);
            show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    // Click Invert, then move over the image.
    h.get_by_label("Invert").click();
    h.run_steps(2);
    assert_eq!(h.state().distort.select_mask.as_ref().map(|w| w.edits.len()), Some(1));
    let over = Pos2::new(LEFT_W + 300.0, BAR_H + TAB_H + 300.0);
    h.event(egui::Event::PointerMoved(over));
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "the brush's own cursor");
    let space = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>, pressed: bool| {
        h.event(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(1);
    };
    space(&mut h, true);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Grab, "Space: the open hand");
    assert_eq!(h.state().distort.select_mask.as_ref().map(|w| w.edits.len()), Some(1), "Space didn't press Invert again");
    let centre = h.state().distort.select_mask.as_ref().map(|w| w.center).unwrap();
    h.event(egui::Event::PointerButton { pos: over, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for dx in [5.0, 20.0, 40.0] {
        h.event(egui::Event::PointerMoved(Pos2::new(over.x + dx, over.y)));
        h.run_steps(1);
    }
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Grabbing, "dragging: the closed hand");
    h.event(egui::Event::PointerButton { pos: Pos2::new(over.x + 40.0, over.y), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    space(&mut h, false);
    let w = h.state().distort.select_mask.as_ref().unwrap();
    assert!(w.center[0] < centre[0], "the view panned: {:?} → {:?}", centre, w.center);
    assert!(w.edits.len() == 1 && w.strokes.is_empty(), "panning painted nothing");
    // The Hand tool shows the open hand without Space.
    h.state_mut().distort.select_mask.as_mut().unwrap().pick(SmTool::Hand);
    h.event(egui::Event::PointerMoved(over));
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Grab);
}

/// The greyed Object Aware and Refine Hair say why they can't be used.
#[test]
fn greyed_controls_explain_themselves() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    for (label, why) in [("Object Aware", "segmentation model"), ("Refine Hair", "hair detection")] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label} is greyed");
        assert!(h.query_by_label_contains(why).is_none(), "{label}: no tooltip before hovering");
        let at = h.get_by_label(label).rect().center();
        h.hover_at(at);
        h.run_steps(8);
        assert!(h.query_by_label_contains(why).is_some(), "{label}: hovering says why");
        h.hover_at(egui::pos2(700.0, 450.0));
        h.run_steps(4);
    }
}

fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let (req, _rx) = crate::control::ControlRequest::new(method, params);
    match crate::control::handle(app, &egui::Context::default(), &req) {
        crate::control::Outcome::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

/// A second document with its own selection, the first one active again.
fn two_documents() -> PhotocraftApp {
    let mut app = app_with_selection();
    app.session.execute("file.new", json!({"width": 160, "height": 100, "depth": 8})).unwrap();
    app.session.execute("select.rect", json!({"x": 100, "y": 50, "width": 30, "height": 30})).unwrap();
    assert!(app.session.set_active(0));
    app.sync_views();
    app
}

fn fingerprint(app: &PhotocraftApp, i: usize) -> (u64, Option<f32>, Option<f32>) {
    let st = &app.session.documents()[i];
    let sel = st.doc.selection.as_ref();
    (st.revision, sel.map(|s| s.sample_channel(10, 10, 0)), sel.map(|s| s.sample_channel(120, 50, 0)))
}

/// The #8896 review: a document switch and OK queued on the control channel, with no frame
/// between them, must not apply the first document's edits to the second.
#[test]
fn ok_after_a_document_switch_changes_neither_document() {
    let mut app = two_documents();
    open(&mut app).unwrap();
    ui(&mut app, json!({"stroke": {"points": [[120.0, 50.0], [130.0, 50.0]]}}));
    let before = (fingerprint(&app, 0), fingerprint(&app, 1));
    let switched = call(&mut app, "engine.execute", json!({"command": "document.activate", "params": {"document": 1}}));
    assert_eq!(switched["ok"], false, "external edits respect the modal gate");
    assert_eq!(app.session.active_index(), Some(0));
    assert!(app.session.set_active(1), "simulate a trusted native target change before the next frame");
    let r = call(&mut app, "ui.menu.invoke", json!({"command": "select.selectAndMask", "params": {"ui": {"commit": true}}}));
    assert!(r.get("error").is_some(), "the stale OK is refused: {r}");
    assert_eq!((fingerprint(&app, 0), fingerprint(&app, 1)), before, "neither document changed");
    assert!(app.distort.select_mask.is_none(), "the stale workspace is closed");
}

/// Every route into the workspace checks its document: control edits, the preview and pointer
/// events on another document are refused, and none of them edits that document.
#[test]
fn control_preview_and_pointer_on_another_document_are_refused() {
    for route in ["control", "preview", "pointer"] {
        let mut app = two_documents();
        open(&mut app).unwrap();
        let before = fingerprint(&app, 1);
        assert!(app.session.set_active(1));
        match route {
            "control" => assert!(control(&mut app, &json!({"stroke": {"points": [[120.0, 50.0], [130.0, 50.0]]}})).is_err()),
            "preview" => assert!(control(&mut app, &json!({"waitPreview": true})).is_err()),
            _ => {
                let ev = ToolEvent::Down { x: 120.0, y: 50.0, pressure: 1.0 };
                crate::distort_ui::pointer(&mut app, ev, egui::Modifiers::NONE);
            }
        }
        assert!(app.distort.select_mask.is_none(), "{route}: the stale workspace is closed");
        assert_eq!(fingerprint(&app, 1), before, "{route}: the other document is untouched");
    }
}

/// Selecting another layer in the same document (a shortcut, or the control channel between
/// requests) must not send a layer-mask OK to that layer.
#[test]
fn ok_after_a_layer_switch_leaves_the_new_layer_alone() {
    let mut app = app_with_selection();
    let first = app.session.active().unwrap().active_layer.unwrap();
    let other = app.run("layer.new.layer", json!({"name": "Other"})).unwrap()["layer"].as_u64().map(LayerId).unwrap();
    app.run("layer.select", json!({"layer": first.0})).unwrap();
    open(&mut app).unwrap();
    ui(&mut app, json!({"output": "layerMask"}));
    let revision = app.session.active().unwrap().revision;
    let blocked = call(&mut app, "engine.execute", json!({"command": "layer.select", "params": {"layer": other.0}}));
    assert_eq!(blocked["ok"], false, "external edits respect the modal gate");
    app.session.execute("layer.select", json!({"layer":other.0})).unwrap();
    let revision_after_select = app.session.active().unwrap().revision;
    let r = call(&mut app, "ui.menu.invoke", json!({"command": "select.selectAndMask", "params": {"ui": {"commit": true}}}));
    assert!(r.get("error").is_some(), "the stale OK is refused: {r}");
    let st = app.session.active().unwrap();
    assert!(st.doc.layer(other).unwrap().mask.is_none(), "no mask on the newly selected layer");
    assert!(st.doc.layer(first).unwrap().mask.is_none(), "nor on the first");
    assert!(st.doc.selection.is_some(), "the selection stands");
    assert!(revision_after_select >= revision && st.revision == revision_after_select, "nothing was edited after the switch");
    assert!(app.distort.select_mask.is_none());
}

/// A stale workspace is closed with a reason that is shown in the status bar and returned to the
/// control channel (#8896).
#[test]
fn the_reason_a_stale_workspace_closed_is_reported() {
    let document = "Select and Mask was closed: its document is no longer the active one";
    let layer = "Select and Mask was closed: another layer was selected";
    for (reason, english) in [("document", document), ("layer", layer)] {
        let mut app = if reason == "document" { two_documents() } else { app_with_selection() };
        let first = app.session.active().unwrap().active_layer.unwrap();
        let other = (reason == "layer").then(|| app.run("layer.new.layer", json!({"name": "Other"})).unwrap()["layer"].as_u64().map(LayerId).unwrap());
        app.run("layer.select", json!({"layer": first.0})).unwrap();
        open(&mut app).unwrap();
        match other {
            Some(other) => drop(app.session.execute("layer.select", json!({"layer": other.0}))),
            None => assert!(app.session.set_active(1)),
        }
        let err = commit(&mut app).expect_err(&format!("{reason}: the stale OK is refused"));
        assert_eq!(err, english, "{reason}: the error");
        assert_eq!(app.ui.status, english, "{reason}: the status");
    }
}

/// The ants of `selection` over the reduced copy `area`, `k` document px per copy px.
fn shown_ants(selection: Surface, area: Rect, k: u32) -> Ants {
    let bounds = trace_bounds(Some(&selection), area);
    Ants { selection, bounds, k, outline: None }
}

fn ants_mut(app: &mut PhotocraftApp) -> &mut Ants {
    ws_mut(app).preview.ants.as_mut().unwrap()
}

#[test]
fn sparse_large_preview_keeps_its_ants() {
    let area = Rect::new(0, 0, 100_000, 100_000);
    let mut selection = Surface::new(photocraft_color::PixelFormat::GRAY8);
    selection.write_pixel(99_999, 99_999, &[1.0]);
    selection.write_pixel(2, 3, &[0.5]);
    selection.write_pixel(-1, -1, &[1.0]);
    assert_eq!(selected_pixels(Some(&selection), area), 2);
    let small = Rect::new(0, 0, 10, 10);
    assert_eq!(selected_pixels(Some(&selection), small), 1);
    // At 100 % around the far corner: the one pixel there, outlined by four lines. The old ants
    // were unit edges of a canvas-sized mask, dropped past 4 MP.
    let mut ants = shown_ants(selection, area, 1);
    let segs = ant_outline(&mut ants, (1.0, [99_500.0, 99_500.0, 100_500.0, 100_500.0]), false);
    assert_eq!(segs.len(), 4, "{segs:?}");
    assert!(segs.iter().all(|(a, b)| [a, b].iter().all(|p| (99_999..=100_000).contains(&p[0]) && (99_999..=100_000).contains(&p[1]))));

    let mut inverted = Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.0]);
    inverted.write_pixel(2, 3, &[0.0]);
    let smaller = Rect::new(0, 0, 1000, 1000);
    assert_eq!(selected_pixels(Some(&inverted), smaller), 999_999);
    assert_eq!(selected_pixels(None, area), 0);
}

/// The workspace in a harness on `tool`, and a point over the image.
fn resize_harness(tool: &str) -> (egui_kittest::Harness<'static, PhotocraftApp>, egui::Pos2) {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"tool": tool}));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::distort_ui::keys(app, &ctx);
            keys(app, &ctx);
            show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    (h, egui::Pos2::new(LEFT_W + 300.0, BAR_H + TAB_H + 300.0))
}

/// Presses `button` at `at` with `modifiers` held.
fn pointer_down(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, at: egui::Pos2, button: egui::PointerButton, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button, pressed: true, modifiers });
    h.run_steps(1);
}

/// Moves the held pointer from `from` by `by`, in four steps.
fn pointer_move(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, from: egui::Pos2, by: egui::Vec2) {
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(from + by * (k as f32 / 4.0)));
        h.run_steps(1);
    }
}

fn tap_key(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, key: Key, pressed: bool) {
    h.event(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// A whole drag of `button` from `at` by `by` with `modifiers` held.
fn pointer_drag(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, at: egui::Pos2, button: egui::PointerButton, modifiers: Modifiers, by: egui::Vec2) {
    pointer_down(h, at, button, modifiers);
    pointer_move(h, at, by);
    h.event(egui::Event::PointerButton { pos: at + by, button, pressed: false, modifiers });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

/// Alt+right-drag (and Control+Alt-drag) on the image resizes the current brush tool instead of
/// selecting, as on the main canvas (#10497): only that tool's size changes, the Brush's hardness
/// follows vertical motion, nothing is painted, and the next Alt-drag still subtracts.
#[test]
fn alt_right_drag_resizes_the_workspace_brush() {
    use egui::PointerButton;
    let (mut h, over) = resize_harness("brush");
    let zoom = h.state().distort.select_mask.as_ref().unwrap().zoom;
    pointer_drag(&mut h, over, PointerButton::Secondary, Modifiers::ALT, vec2(40.0, -60.0));
    let w = h.state().distort.select_mask.as_ref().unwrap();
    let want = (30.0 + 2.0 * f64::from(40.0 / zoom)).round();
    assert_eq!(w.brush_size, want, "horizontal motion sets the diameter (zoom {zoom})");
    let soft = (100.0 - 60.0 * 100.0 / crate::brush_resize::HARDNESS_TRAVEL).round();
    assert_eq!(w.hardness, soft, "upward motion softens the brush, at the canvas HUD's rate");
    assert_eq!((w.quick_size, w.refine_size), (30.0, 30.0), "the other tools keep their sizes");
    assert!(w.strokes.is_empty() && w.edits.is_empty() && w.undo.is_empty(), "resizing selected nothing");
    assert!(w.resize.is_none(), "the gesture ends with the button");
    // Alt with the left button is still the subtracting stroke.
    pointer_drag(&mut h, over, PointerButton::Primary, Modifiers::ALT, vec2(30.0, 0.0));
    let w = h.state().distort.select_mask.as_ref().unwrap();
    // A Brush stroke is an edit (Quick Selection's are strokes).
    assert_eq!(w.edits.len(), 1, "Alt+left-drag selects");
    assert_eq!(w.edits[0]["mode"], json!("subtract"));
    assert_eq!(w.brush_size, want, "and doesn't resize");
    // Control+Alt-drag (the macOS gesture) resizes Quick Selection on its own.
    h.state_mut().distort.select_mask.as_mut().unwrap().pick(SmTool::Quick);
    pointer_drag(&mut h, over, PointerButton::Primary, Modifiers::CTRL | Modifiers::ALT, vec2(-10.0, 0.0));
    let w = h.state().distort.select_mask.as_ref().unwrap();
    assert_eq!(w.quick_size, (30.0 - 2.0 * f64::from(10.0 / zoom)).round().max(1.0));
    assert_eq!((w.brush_size, w.hardness), (want, soft), "the Brush keeps its size and hardness");
    assert!(w.strokes.is_empty() && w.edits.len() == 1, "resizing Quick Selection selected nothing");
}

/// Space pressed mid-resize pans, and the button released before Space still ends the resize
/// (CodeRabbit on #10559): a Space-drag that goes on after Space comes up paints, as any
/// Space-drag does, instead of resizing the brush from the old anchor.
#[test]
fn a_resize_released_while_space_pans_lets_the_next_drag_paint() {
    use egui::PointerButton;
    let (mut h, over) = resize_harness("brush");
    let zoom = h.state().distort.select_mask.as_ref().unwrap().zoom;
    pointer_down(&mut h, over, PointerButton::Secondary, Modifiers::ALT);
    pointer_move(&mut h, over, vec2(40.0, 0.0));
    tap_key(&mut h, Key::Space, true);
    h.event(egui::Event::PointerButton { pos: over + vec2(40.0, 0.0), button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::ALT });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    let want = (30.0 + 2.0 * f64::from(40.0 / zoom)).round();
    assert_eq!(h.state().distort.select_mask.as_ref().unwrap().brush_size, want);
    // Space+drag pans; Space comes up mid-drag and the drag goes on.
    pointer_down(&mut h, over, PointerButton::Primary, Modifiers::NONE);
    pointer_move(&mut h, over, vec2(0.0, 20.0));
    tap_key(&mut h, Key::Space, false);
    pointer_move(&mut h, over + vec2(0.0, 20.0), vec2(30.0, 0.0));
    h.event(egui::Event::PointerButton { pos: over + vec2(30.0, 20.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let w = h.state().distort.select_mask.as_ref().unwrap();
    assert_eq!(w.brush_size, want, "the drag doesn't resize the brush");
    assert_eq!(w.edits.len(), 1, "the drag's rest paints");
    assert_eq!(w.edits[0]["mode"], json!("add"));
    assert!(w.resize.is_none());
}

/// A resize belongs to the tool it began on (CodeRabbit on #10559): if another tool is picked
/// mid-drag, the rest of the drag neither resizes that tool nor selects with it.
#[test]
fn picking_another_tool_mid_resize_neither_resizes_nor_selects() {
    use egui::PointerButton;
    let (mut h, over) = resize_harness("brush");
    let zoom = h.state().distort.select_mask.as_ref().unwrap().zoom;
    let want = (30.0 + 2.0 * f64::from(40.0 / zoom)).round();
    // Alt+right-drag the Brush larger, let go of Alt, press W (Quick Selection) and drag on.
    pointer_down(&mut h, over, PointerButton::Secondary, Modifiers::ALT);
    pointer_move(&mut h, over, vec2(40.0, 0.0));
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    tap_key(&mut h, Key::W, true);
    tap_key(&mut h, Key::W, false);
    assert_eq!(h.state().distort.select_mask.as_ref().unwrap().tool, SmTool::Quick);
    pointer_move(&mut h, over + vec2(40.0, 0.0), vec2(40.0, 0.0));
    h.event(egui::Event::PointerButton { pos: over + vec2(80.0, 0.0), button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let w = h.state().distort.select_mask.as_ref().unwrap();
    assert_eq!((w.brush_size, w.quick_size), (want, 30.0), "Quick Selection keeps its size");
    assert!(w.strokes.is_empty() && w.edits.is_empty() && w.undo.is_empty(), "nothing was selected");
    // The same with the Lasso (not a brush) picked mid-drag: the rest of the drag draws no lasso.
    tap_key(&mut h, Key::B, true);
    tap_key(&mut h, Key::B, false);
    pointer_down(&mut h, over, PointerButton::Secondary, Modifiers::ALT);
    pointer_move(&mut h, over, vec2(20.0, 0.0));
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    tap_key(&mut h, Key::L, true);
    tap_key(&mut h, Key::L, false);
    assert_eq!(h.state().distort.select_mask.as_ref().unwrap().tool, SmTool::Lasso);
    pointer_move(&mut h, over + vec2(20.0, 0.0), vec2(0.0, 60.0));
    pointer_move(&mut h, over + vec2(20.0, 60.0), vec2(-60.0, 0.0));
    h.event(egui::Event::PointerButton { pos: over + vec2(-40.0, 60.0), button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let w = h.state().distort.select_mask.as_ref().unwrap();
    assert!(w.strokes.is_empty() && w.edits.is_empty() && w.undo.is_empty(), "the Lasso selected nothing");
    assert!(w.resize.is_none(), "the gesture ends with the button");
}

/// A resize ends when its own button comes up, even while the middle button pans (CodeRabbit on
/// #10559): a left-drag begun before the middle button is let go doesn't resize the brush from
/// the old anchor when the pan ends.
#[test]
fn a_resize_released_while_the_middle_button_pans_ends_with_its_button() {
    use egui::PointerButton;
    let (mut h, over) = resize_harness("brush");
    let zoom = h.state().distort.select_mask.as_ref().unwrap().zoom;
    let want = (30.0 + 2.0 * f64::from(40.0 / zoom)).round();
    pointer_down(&mut h, over, PointerButton::Secondary, Modifiers::ALT);
    pointer_move(&mut h, over, vec2(40.0, 0.0));
    let at = over + vec2(40.0, 0.0);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Middle, pressed: true, modifiers: Modifiers::ALT });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::ALT });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    assert_eq!(h.state().distort.select_mask.as_ref().unwrap().brush_size, want);
    // The left button goes down while the middle one still pans, then the middle one comes up
    // and the left drag goes on.
    pointer_move(&mut h, at, vec2(0.0, 20.0));
    let at = at + vec2(0.0, 20.0);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    pointer_move(&mut h, at, vec2(30.0, -20.0));
    let at = at + vec2(30.0, -20.0);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Middle, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    pointer_move(&mut h, at, vec2(60.0, -40.0));
    h.event(egui::Event::PointerButton { pos: at + vec2(60.0, -40.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let w = h.state().distort.select_mask.as_ref().unwrap();
    assert_eq!((w.brush_size, w.hardness), (want, 100.0), "the later drag doesn't resize the brush");
    assert!(w.resize.is_none(), "the gesture ended with its button");
}

/// Esc mid-resize puts the brush back and ends only the resize, as in Photoshop (FredApps on
/// #10559): the workspace stays open, the rest of that drag neither resizes nor selects, and the
/// next drag selects as usual.
#[test]
fn escape_mid_resize_restores_the_brush_and_keeps_the_workspace() {
    use egui::PointerButton;
    let (mut h, over) = resize_harness("brush");
    pointer_down(&mut h, over, PointerButton::Secondary, Modifiers::ALT);
    pointer_move(&mut h, over, vec2(40.0, -60.0));
    let w = h.state().distort.select_mask.as_ref().unwrap();
    assert!(w.resize.is_some() && w.brush_size > 30.0 && w.hardness < 100.0, "the drag resizes");
    // Esc with Alt and the right button still held.
    h.event(egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::ALT });
    h.run_steps(1);
    h.event(egui::Event::Key { key: Key::Escape, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::ALT });
    h.run_steps(1);
    let w = h.state().distort.select_mask.as_ref().expect("Esc ends the resize, not the workspace");
    assert_eq!((w.brush_size, w.hardness), (30.0, 100.0), "the brush is as it was");
    assert!(w.resize.is_none());
    // The drag goes on with Alt and the right button held: it does nothing.
    pointer_move(&mut h, over + vec2(40.0, -60.0), vec2(40.0, 40.0));
    h.event(egui::Event::PointerButton { pos: over + vec2(80.0, -20.0), button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::ALT });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    let w = h.state().distort.select_mask.as_ref().unwrap();
    assert_eq!((w.brush_size, w.hardness), (30.0, 100.0), "the rest of the drag doesn't resize");
    assert!(w.strokes.is_empty() && w.edits.is_empty() && w.undo.is_empty(), "nor select");
    assert!(w.resize.is_none());
    // The next drag selects with the brush as it was.
    pointer_drag(&mut h, over, PointerButton::Primary, Modifiers::NONE, vec2(30.0, 0.0));
    let w = h.state().distort.select_mask.as_ref().unwrap();
    assert_eq!((w.brush_size, w.hardness), (30.0, 100.0), "the next drag doesn't resize");
    assert_eq!(w.edits.len(), 1, "it selects");
    assert_eq!(w.edits[0]["mode"], json!("add"));
    // Control+Alt-drag (the macOS gesture) on Quick Selection, Esc with both still held.
    let ctrl_alt = Modifiers::CTRL | Modifiers::ALT;
    h.state_mut().distort.select_mask.as_mut().unwrap().pick(SmTool::Quick);
    pointer_down(&mut h, over, PointerButton::Primary, ctrl_alt);
    pointer_move(&mut h, over, vec2(30.0, 0.0));
    assert!(h.state().distort.select_mask.as_ref().unwrap().quick_size > 30.0, "the drag resizes");
    h.event(egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: ctrl_alt });
    h.run_steps(1);
    let w = h.state().distort.select_mask.as_ref().expect("the workspace stays open");
    assert_eq!(w.quick_size, 30.0, "Quick Selection's size is as it was");
    assert!(w.resize.is_none());
    pointer_move(&mut h, over + vec2(30.0, 0.0), vec2(30.0, 0.0));
    h.event(egui::Event::PointerButton { pos: over + vec2(60.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: ctrl_alt });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    let w = h.state().distort.select_mask.as_ref().unwrap();
    assert_eq!(w.quick_size, 30.0, "the rest of the drag doesn't resize");
    assert!(w.strokes.is_empty() && w.edits.len() == 1, "nor select");
}

/// The brush circles drawn last frame (the black outline's centre and radius).
fn brush_circles(h: &egui_kittest::Harness<'_, PhotocraftApp>) -> Vec<(egui::Pos2, f32)> {
    h.output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Circle(s) if s.stroke == Stroke::new(1.5, Color32::BLACK) => Some((s.center, s.radius)),
            _ => None,
        })
        .collect()
}

/// While the brush is resized its circle stays where the drag began, at the new size, and the
/// Diameter/Hardness readout follows the pointer, as on the main canvas (FredApps on #10559).
#[test]
fn the_resize_circle_stays_put_while_the_readout_follows_the_pointer() {
    use egui::PointerButton;
    let (mut h, over) = resize_harness("brush");
    let zoom = h.state().distort.select_mask.as_ref().unwrap().zoom;
    let readout = || egui::Id::unique("select-mask-brush-resize-readout");
    h.event(egui::Event::PointerMoved(over));
    h.run_steps(2);
    assert_eq!(brush_circles(&h), [(over, 15.0 * zoom)], "hovering, the circle is at the pointer");
    pointer_down(&mut h, over, PointerButton::Secondary, Modifiers::ALT);
    for by in [vec2(40.0, -60.0), vec2(60.0, -60.0)] {
        pointer_move(&mut h, over, by);
        // The cursor is drawn before the frame's input, so it shows the new size a frame later.
        h.run_steps(1);
        let at = over + by;
        let w = h.state().distort.select_mask.as_ref().unwrap();
        let (size, hardness) = (w.brush_size, w.hardness);
        assert_eq!(brush_circles(&h), [(over, size as f32 * 0.5 * zoom)], "the circle stays at the anchor at the new size ({by:?})");
        let rect = h.ctx.memory(|m| m.area_rect(readout())).expect("the readout shows");
        assert_eq!(rect.min, at + vec2(16.0, 18.0), "the readout is beside the pointer");
        assert!(h.query_by_label(&format!("{} px", size.round() as i64)).is_some(), "it shows the diameter");
        assert!(h.query_by_label(&format!("{}%", hardness.round() as i64)).is_some(), "and the hardness");
    }
    h.event(egui::Event::PointerButton { pos: over + vec2(60.0, -60.0), button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::ALT });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    let size = h.state().distort.select_mask.as_ref().unwrap().brush_size;
    assert_eq!(brush_circles(&h), [(over + vec2(60.0, -60.0), size as f32 * 0.5 * zoom)], "after the drag the circle follows the pointer again");
    assert!(h.query_by_label("Diameter:").is_none(), "and the readout is gone");
}

/// A selection of the rectangle `r` (reduced-copy px).
fn rect_selection(r: Rect) -> Surface {
    let mut s = Surface::new(photocraft_color::PixelFormat::GRAY8);
    s.fill_rect(r, &[1.0]);
    s
}

/// The Marching Ants view outlines the refined selection as the canvas does: merged runs (4 lines
/// for a rectangle, not one per pixel edge), on the document's pixel edges at the copy's scale.
#[test]
fn marching_ants_trace_merged_runs_like_the_canvas() {
    let area = Rect::new(0, 0, 100, 60);
    let sel = rect_selection(Rect::new(5, 5, 25, 15));
    let whole = crate::outline::outline(&sel, sel.content_bounds());
    let mut ants = shown_ants(sel, area, 2);
    let segs = ant_outline(&mut ants, (1.0, [0.0, 0.0, 200.0, 120.0]), false).to_vec();
    assert_eq!(segs, whole);
    assert_eq!(segs.len(), 4, "{segs:?}");
    for (a, b) in segs {
        for p in [a, b].map(|p| [p[0] * 2, p[1] * 2]) {
            let on_x = (p[0] == 10 || p[0] == 50) && (10..=30).contains(&p[1]);
            let on_y = (p[1] == 10 || p[1] == 30) && (10..=50).contains(&p[0]);
            assert!(on_x || on_y, "{p:?} lies on the document rectangle (10, 10)–(50, 30)");
        }
    }
}

/// Frames reuse the trace while the view stays within its quantised region; a zoom traces again.
#[test]
fn ant_outline_is_cached_per_view() {
    let mut ants = shown_ants(rect_selection(Rect::new(10, 10, 90, 50)), Rect::new(0, 0, 100, 60), 1);
    let first = ant_outline(&mut ants, (1.0, [0.0, 0.0, 100.0, 60.0]), false).as_ptr();
    let key = ants.outline.as_ref().unwrap().0;
    let panned = ant_outline(&mut ants, (1.0, [3.0, 2.0, 103.0, 62.0]), false).as_ptr();
    assert_eq!(first, panned, "a small pan keeps the trace");
    assert_eq!(ants.outline.as_ref().unwrap().0, key);
    let zoomed = ant_outline(&mut ants, (0.25, [0.0, 0.0, 400.0, 240.0]), false).len();
    assert_eq!(ants.outline.as_ref().unwrap().0.step, 4, "zoomed out to 25 %: one sample per 4 px");
    assert_eq!(zoomed, 4);
}

/// The preview job traces the outline for the view it was asked for, so a new preview (one per
/// move with Real-Time Refinement) costs the UI thread no trace.
#[test]
fn the_preview_job_traces_the_outline_for_the_view() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let area = ERect::from_min_size(pos2(50.0, 60.0), vec2(800.0, 500.0));
    let w = ws_mut(&mut app);
    (w.view_area, w.zoom, w.center) = (Some((area, 1.5)), 2.0, [80.0, 50.0]);
    // The other views keep no refined selection.
    ui(&mut app, json!({"waitPreview": true}));
    upload(&mut app, &egui::Context::default());
    assert!(ws(&app).preview.ants.is_none(), "Onion Skin");
    ui(&mut app, json!({"view": "marchingAnts", "waitPreview": true}));
    upload(&mut app, &egui::Context::default());
    let w = ws_mut(&mut app);
    let view = ant_view(View { area, zoom: w.zoom, center: w.center }, 1.5);
    let ants = w.preview.ants.as_mut().unwrap();
    let traced = ants.outline.as_ref().map(|(key, segs)| (*key, segs.as_ptr())).expect("traced in the job");
    assert_eq!(traced.0, outline_key(ants.k, ants.bounds, view));
    assert_eq!(ant_outline(ants, view, false).as_ptr(), traced.1, "drawn without tracing again");
    assert_eq!(ants.outline.as_ref().unwrap().1.len(), 4, "the 20 × 20 selection");
}

/// Until the preview at the new scale arrives, the ants keep the scale of the preview they outline.
#[test]
fn the_outline_keeps_its_previews_scale_while_high_quality_is_toggled() {
    let mut app = app_sized(3300, 60);
    app.session.execute("select.rect", json!({"x": 30, "y": 15, "width": 30, "height": 30})).unwrap();
    open(&mut app).unwrap();
    ui(&mut app, json!({"view": "marchingAnts", "waitPreview": true}));
    upload(&mut app, &egui::Context::default());
    ui(&mut app, json!({"highQualityPreview": true}));
    assert_eq!(ws(&app).proxy_k(), 1);
    let ants = ants_mut(&mut app);
    assert_eq!(ants.k, 3);
    let k = ants.k as i32;
    let segs = ant_outline(ants, (1.0, [0.0, 0.0, 3300.0, 60.0]), false);
    let mut doc: Vec<_> = segs.iter().map(|(a, b)| ([a[0] * k, a[1] * k], [b[0] * k, b[1] * k])).collect();
    doc.sort();
    assert_eq!(doc, vec![([30, 15], [30, 45]), ([30, 15], [60, 15]), ([30, 45], [60, 45]), ([60, 15], [60, 45])]);
}

/// An inverted selection (selected by default) is outlined along the canvas border too, on a
/// canvas past the old ants' 4 MP limit.
#[test]
fn an_inverted_selection_outlines_the_canvas_border() {
    let area = Rect::new(0, 0, 3000, 2000);
    let mut inverted = Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.0]);
    inverted.fill_rect(Rect::new(1000, 800, 1400, 1200), &[0.0]);
    let mut ants = shown_ants(inverted, area, 1);
    assert_eq!(ants.bounds, area);
    let segs = ant_outline(&mut ants, (0.25, [0.0, 0.0, 3000.0, 2000.0]), false);
    assert_eq!(segs.len(), 8, "{segs:?}");
    for border in [([0, 0], [3000, 0]), ([0, 2000], [3000, 2000]), ([0, 0], [0, 2000]), ([3000, 0], [3000, 2000])] {
        assert!(segs.contains(&border), "{border:?} in {segs:?}");
    }
}

/// The reduced copy covers whole blocks of `k` px, past the canvas when its size isn't a multiple
/// of `k`; the ants stay on the canvas border, not on the pasteboard beyond it.
#[test]
fn the_border_ants_stay_on_the_canvas_of_a_reduced_copy() {
    let mut app = app_sized(3301, 60);
    open(&mut app).unwrap();
    ui(&mut app, json!({"invert": true, "view": "marchingAnts", "waitPreview": true}));
    upload(&mut app, &egui::Context::default());
    let canvas = ws(&app).canvas;
    let ants = ants_mut(&mut app);
    assert_eq!(ants.k, 3);
    let k = ants.k;
    let segs = ant_outline(ants, (0.5, [0.0, 0.0, 3301.0, 60.0]), false);
    let points: Vec<[f64; 2]> = segs.iter().flat_map(|(a, b)| [*a, *b]).map(|p| ant_point(p, k, canvas)).collect();
    assert!(points.iter().all(|p| (0.0..=3301.0).contains(&p[0]) && (0.0..=60.0).contains(&p[1])), "{points:?}");
    assert!(points.iter().any(|p| p[0] == 3301.0) && points.iter().any(|p| p[1] == 60.0), "the right and bottom border: {points:?}");
}

/// A document edit while the workspace is open drops the old preview's ants with its image.
#[test]
fn a_document_edit_clears_the_old_ants() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"view": "marchingAnts", "waitPreview": true}));
    upload(&mut app, &egui::Context::default());
    assert!(!ant_outline(ants_mut(&mut app), (1.0, [0.0, 0.0, 160.0, 100.0]), false).is_empty());
    app.session
        .edit("paint", |doc, _| {
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 4, 4), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    follow_revision(&mut app);
    assert!(ws(&app).preview.ants.is_none());
}

/// With nothing selected the Marching Ants view draws no ants, and drawing it doesn't panic.
#[test]
fn marching_ants_without_a_selection_draw_nothing() {
    let empty = Surface::new(photocraft_color::PixelFormat::GRAY8);
    assert!(ant_outline(&mut shown_ants(empty, Rect::new(0, 0, 100, 100), 1), (1.0, [0.0, 0.0, 100.0, 100.0]), true).is_empty());
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"clearSelection": true, "view": "marchingAnts", "waitPreview": true}));
    assert_eq!(ws(&app).describe()["selectedProxyPixels"].as_u64(), Some(0));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                show(app, &ctx);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let w = h.state().distort.select_mask.as_ref().unwrap();
    let outline = w.preview.ants.as_ref().and_then(|a| a.outline.as_ref());
    assert!(outline.is_some_and(|(_, segs)| segs.is_empty()), "traced, and nothing to outline");
}

type GpuHarness = egui_kittest::Harness<'static, PhotocraftApp>;

/// A rendered frame: RGBA8, device pixels.
struct Frame {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Frame {
    fn luma(&self, x: i64, y: i64) -> Option<u8> {
        if x < 0 || y < 0 || x >= i64::from(self.w) || y >= i64::from(self.h) {
            return None;
        }
        let i = (y as usize * self.w as usize + x as usize) * 4;
        let p = self.px.get(i..i + 3)?;
        Some(((u16::from(p[0]) * 3 + u16::from(p[1]) * 6 + u16::from(p[2])) / 10) as u8)
    }
}

/// The workspace open on a white 400 × 300 document with (100, 80)–(300, 220) selected, in the
/// Marching Ants view at 100 %, on an offscreen wgpu harness, and the frame time the workspace
/// opened at (after three frames); `None` without a GPU adapter.
fn ants_harness(ppp: f32, step_dt: f32) -> Option<(GpuHarness, f64)> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 80, "width": 200, "height": 140})).unwrap();
    app.sync_views();
    // `wgpu()` creates the render state and panics when no adapter is found.
    let builder = std::panic::catch_unwind(|| {
        egui_kittest::Harness::<PhotocraftApp>::builder().with_size(vec2(1000.0, 700.0)).with_pixels_per_point(ppp).with_step_dt(step_dt).wgpu()
    })
    .ok()?;
    let mut h = builder.build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                show(app, &ctx);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    // The canvas's clock runs a few frames before the dialog opens, so a clock of the dialog's
    // own, started when it opens, would be out of phase with it.
    h.run_steps(3);
    let opened = h.ctx.input(|i| i.time);
    open(h.state_mut()).unwrap();
    ui(h.state_mut(), json!({"view": "marchingAnts", "waitPreview": true}));
    h.run_steps(4);
    let w = h.state_mut().distort.select_mask.as_mut().unwrap();
    (w.zoom, w.center) = (1.0, [200.0, 150.0]);
    h.run_steps(2);
    Some((h, opened))
}

fn frame(h: &mut GpuHarness) -> Frame {
    let img = h.render().unwrap();
    Frame { w: img.width(), h: img.height(), px: img.into_raw() }
}

/// Device pixel of document point `p`.
fn device(h: &GpuHarness, ppp: f32, p: [f64; 2]) -> (i64, i64) {
    let w = h.state().distort.select_mask.as_ref().unwrap();
    let (area, _) = w.view_area.unwrap();
    let s = View { area, zoom: w.zoom, center: w.center }.to_screen(p) * ppp;
    (s.x.round() as i64, s.y.round() as i64)
}

/// Along the edge from device pixel `a` to `b` (12 px in from each end): the offsets across it
/// (±6) of the dark pixels at each step, and how many pixels of the band are grey.
fn scan(img: &Frame, a: (i64, i64), b: (i64, i64)) -> (Vec<Vec<i64>>, usize) {
    let vertical = a.0 == b.0;
    let (lo, hi) = if vertical { (a.1.min(b.1), a.1.max(b.1)) } else { (a.0.min(b.0), a.0.max(b.0)) };
    let mut across = Vec::new();
    let mut greys = 0;
    for t in lo + 12..hi - 12 {
        let mut dark = Vec::new();
        for o in -6..=6 {
            let (x, y) = if vertical { (a.0 + o, t) } else { (t, a.1 + o) };
            let l = img.luma(x, y).unwrap_or(255);
            if l < 64 {
                dark.push(o);
            }
            if (24..232).contains(&l) {
                greys += 1;
            }
        }
        across.push(dark);
    }
    (across, greys)
}

/// #10506: the Marching Ants view draws the canvas's ants: every edge `w` device px thick, crisp
/// black and white, dashes of 4·w px. They were 1-point antialiased lines, grey and uneven.
#[test]
fn select_and_mask_ants_match_the_canvas() {
    let _gpu = photocraft_testkit::gpu_lock();
    for ppp in [1.0f32, 1.25] {
        let Some((mut h, _)) = ants_harness(ppp, 1.0 / 60.0) else { return };
        let img = frame(&mut h);
        let w = ppp.ceil() as usize;
        let [tl, tr, br, bl] = [[100.0, 80.0], [300.0, 80.0], [300.0, 220.0], [100.0, 220.0]].map(|p| device(&h, ppp, p));
        for (name, (a, b)) in [("top", (tl, tr)), ("right", (tr, br)), ("bottom", (bl, br)), ("left", (tl, bl))] {
            let (across, greys) = scan(&img, a, b);
            assert_eq!(greys, 0, "@{ppp}x {name}: crisp black and white");
            let max = across.iter().map(Vec::len).max().unwrap_or(0);
            assert_eq!(max, w, "@{ppp}x {name}: {w} px thick");
            let dark: usize = across.iter().map(Vec::len).sum();
            let frac = dark as f32 / (across.len() * w) as f32;
            assert!((0.45..=0.55).contains(&frac), "@{ppp}x {name}: {frac:.2} of the line is black");
            let rows: std::collections::BTreeSet<i64> = across.iter().flatten().copied().collect();
            for row in rows {
                let mut runs = vec![0usize];
                for d in &across {
                    match (d.contains(&row), runs.last_mut()) {
                        (true, Some(n)) => *n += 1,
                        (false, Some(0)) => {}
                        _ => runs.push(0),
                    }
                }
                let inner = runs.get(1..runs.len().saturating_sub(1)).unwrap_or(&[]);
                assert!(inner.iter().all(|&n| n == 4 * w), "@{ppp}x {name} row {row}: dashes {runs:?}, want {} px", 4 * w);
            }
        }
    }
}

/// #10506: the ants march along the edge. They used to swap black and white 8 times a second.
#[test]
fn select_and_mask_ants_march_rather_than_blink() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some((mut h, _)) = ants_harness(1.0, 0.125) else { return };
    let row = |h: &mut GpuHarness| {
        let img = frame(h);
        let ((x0, y), (x1, _)) = (device(h, 1.0, [100.0, 80.0]), device(h, 1.0, [300.0, 80.0]));
        // The darkest row of the band along the top edge (grey counts as dark here, so an
        // antialiased line is still read).
        let line = |dy: i64| (x0 + 12..x1 - 12).map(|x| img.luma(x, y + dy).is_some_and(|l| l < 160)).collect::<Vec<bool>>();
        let best = (-2..=2).max_by_key(|dy| line(*dy).iter().filter(|b| **b).count()).unwrap();
        (line(best), h.ctx.input(|i| i.time))
    };
    let (a, ta) = row(&mut h);
    h.step();
    let (b, tb) = row(&mut h);
    assert!((tb - ta - 0.125).abs() < 1e-6, "frames {ta} and {tb}");
    // How far the canvas's ants march between the two frames.
    let ((pa, period), (pb, _)) = (crate::ants::Style::new(1.0, ta).march(), crate::ants::Style::new(1.0, tb).march());
    let shift = (pb - pa).rem_euclid(period) as usize;
    assert!(shift > 0, "the pattern moves");
    let dark = a.iter().filter(|d| **d).count();
    assert!((a.len() * 2..=a.len() * 8).contains(&(dark * 10)), "the edge has dashes: {dark} of {} px dark", a.len());
    let n = b.len() - shift;
    let shifted = (0..n).filter(|&i| b[i] != a[i + shift]).count();
    let inverted = (0..n).filter(|&i| b[i] != a[i]).count();
    assert!(shifted * 10 < n, "the dashes shift by {shift} px: {shifted} of {n} pixels don't");
    assert!(inverted * 10 < n * 9, "black and white don't swap: {inverted} of {n} pixels changed");
}

/// #10506: at 100 % and 200 % the dialog's ants are the canvas's ants at the frame's time: every
/// pixel of the top edge's band is black exactly where `ants::Style::new(ppp, time)` puts a dash.
/// The dialog reads the same frame clock as the canvas, so the pattern doesn't restart when it
/// opens; and it is fixed to the screen, so the two line up.
#[test]
fn select_and_mask_ants_have_the_canvas_phase() {
    let _gpu = photocraft_testkit::gpu_lock();
    for ppp in [1.0f32, 2.0] {
        // 0.31 s steps, so the march has a phase that isn't zero or a whole dash at the frame, nor
        // zero when the dialog opens.
        let Some((mut h, opened)) = ants_harness(ppp, 0.31) else { return };
        // A clock restarted when the dialog opened would be off by the canvas's phase then.
        let (at_open, period) = crate::ants::Style::new(ppp, opened).march();
        assert!(at_open.rem_euclid(period) != 0, "@{ppp}x: phase {at_open} when the dialog opened at {opened} s");
        let img = frame(&mut h);
        let time = h.ctx.input(|i| i.time);
        let (phase, period) = crate::ants::Style::new(ppp, time).march();
        let (dash, w) = (period / 2, period / 8);
        assert_eq!(w, ppp.ceil() as i64);
        assert!(phase % dash != 0, "@{ppp}x: phase {phase} at {time} s");
        let ((x0, ay), (x1, _)) = (device(&h, ppp, [100.0, 80.0]), device(&h, ppp, [300.0, 80.0]));
        let mut wrong = Vec::new();
        for y in ay - w / 2..ay - w / 2 + w {
            for x in x0 + 12..x1 - 12 {
                let black = (x + w * y + phase).rem_euclid(period) < dash;
                let l = img.luma(x, y).unwrap_or(128);
                if (l < 64) != black || (24..232).contains(&l) {
                    wrong.push((x, y, l));
                }
            }
        }
        assert!(wrong.is_empty(), "@{ppp}x phase {phase}: {} pixels differ from the canvas's ants, first {:?}", wrong.len(), wrong.first());
    }
}
