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

#[test]
fn sparse_large_preview_counts_coverage_without_a_dense_canvas_mask() {
    let area = Rect::new(0, 0, 100_000, 100_000);
    let mut selection = Surface::new(photocraft_color::PixelFormat::GRAY8);
    selection.write_pixel(99_999, 99_999, &[1.0]);
    selection.write_pixel(2, 3, &[0.5]);
    selection.write_pixel(-1, -1, &[1.0]);
    assert_eq!(selected_pixels(Some(&selection), area), 2);
    assert_eq!(preview_ants(Some(&selection), area, 1.0).unwrap().len(), 8);
    let small = Rect::new(0, 0, 10, 10);
    assert_eq!(selected_pixels(Some(&selection), small), 1);
    assert_eq!(preview_ants(Some(&selection), small, 1.0).unwrap().len(), 4);

    let mut inverted = Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.0]);
    inverted.write_pixel(2, 3, &[0.0]);
    let smaller = Rect::new(0, 0, 1000, 1000);
    assert_eq!(selected_pixels(Some(&inverted), smaller), 999_999);
    assert_eq!(selected_pixels(None, area), 0);
}
