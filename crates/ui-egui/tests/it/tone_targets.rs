//! Levels/Curves must use the same grayscale target in their controls, pickers and commands.
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, Layer, LayerContent, LayerMask};
use photocraft_engine::{Session, channel_cmds::ChannelTarget};
use photocraft_geom::Size;
use photocraft_raster::Surface;
use photocraft_ui_egui::{PhotocraftApp, adjust_dialog, adjust_preview, dialogs, filter_dialog};
use serde_json::{Value, json};

const MODES: [ColorMode; 3] = [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab];
const GRAY_TARGET_MODES: [ColorMode; 4] = [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Grayscale];
const DEPTHS: [SampleType; 3] = [SampleType::U8, SampleType::U16, SampleType::F32];
const TARGETS: [ChannelTarget; 3] = [ChannelTarget::Alpha(0), ChannelTarget::LayerMask, ChannelTarget::Composite];

/// Build a native-depth document with distinct colour pixels, a named alpha, a layer mask and optional Quick Mask.
fn fixture(mode: ColorMode, depth: SampleType, target: ChannelTarget) -> PhotocraftApp {
    let mut doc = Document::new("Tone targets", Size::new(32, 32), mode, depth);
    let mut pixels = Surface::new(PixelFormat::new(mode, depth, true));
    pixels.fill_rect(
        doc.bounds(),
        &match mode {
            ColorMode::Cmyk => vec![0.1, 0.2, 0.3, 0.4, 1.0],
            ColorMode::Lab => vec![0.5, 0.5, 0.5, 1.0],
            ColorMode::Grayscale => vec![0.25, 1.0],
            _ => vec![0.0, 0.0, 0.0, 1.0],
        },
    );
    let mut gray = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
    gray.fill_rect(doc.bounds(), &[0.25]);
    let mut layer = Layer::new("Named layer", LayerContent::Raster(pixels));
    layer.mask = Some(LayerMask { surface: gray.clone(), ..LayerMask::reveal_all() });
    let id = layer.id;
    doc.layers.push(layer);
    doc.channels.push(AlphaChannel::new("Saved sky", gray.clone()));
    if target == ChannelTarget::Composite {
        doc.quick_mask = Some(AlphaChannel::new("Quick Mask", gray));
    }
    let mut session = Session::new();
    session.add_document(doc, None);
    session.select_layer(id).unwrap();
    session.active_mut().unwrap().channel_view.target = target;
    let mut app = PhotocraftApp::new(session, Default::default());
    app.background_jobs = false;
    app
}

/// Read a representative sample from the fixture’s selected scalar plane.
fn value(app: &PhotocraftApp, target: ChannelTarget) -> f32 {
    let doc = &app.session.active().unwrap().doc;
    match target {
        ChannelTarget::Alpha(i) => doc.channels[i].surface.sample_channel(8, 8, 0),
        ChannelTarget::LayerMask => doc.layers[0].mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
        ChannelTarget::Composite => doc.quick_mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
        _ => panic!("not a gray target"),
    }
}

/// Render real modal controls with accessibility enabled for input-driven assertions.
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

/// Open a neutral tone dialog and settle its layout before interacting with controls.
fn open(app: PhotocraftApp, kind: &str) -> Harness<'static, PhotocraftApp> {
    let mut h = harness(app);
    adjust_dialog::open(h.state_mut(), &format!("image.adjustments.{kind}")).unwrap();
    h.run_steps(4);
    h
}

/// Extract only the public command parameters from the top modal.
fn params(h: &Harness<'static, PhotocraftApp>) -> Value {
    filter_dialog::params_of(&h.state().ui.dialogs.last().unwrap().fields)
}

/// Read the visible selected labels of the dialog’s channel dropdowns.
fn names(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.query_all_by_role(egui::accesskit::Role::ComboBox).filter_map(|n| n.value()).collect()
}

/// Type into the actual Levels gamma control and blur it to commit the value.
fn edit_gamma(h: &mut Harness<'static, PhotocraftApp>, gamma: &str) {
    h.get_by_label("Gamma").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Text(gamma.into()));
    h.run_steps(1);
    h.get_by_label("Preview").click();
    h.run_steps(2);
}

/// Real Levels gamma edits affect the named scalar target and agree with preview, commit and undo.
#[test]
fn levels_gray_target_controls_change_the_target_in_every_mode_and_depth() {
    for mode in GRAY_TARGET_MODES {
        for depth in DEPTHS {
            for target in TARGETS {
                let mut h = open(fixture(mode, depth, target), "levels");
                let name = match target {
                    ChannelTarget::Alpha(_) => "Saved sky",
                    ChannelTarget::LayerMask => "Named layer Mask",
                    _ => "Quick Mask",
                };
                assert_eq!(names(&h), [name], "{mode:?} {depth:?} {target:?}");
                // Type into the actual visible gamma field and blur it without confirming.
                edit_gamma(&mut h, "2.0");
                assert_eq!(params(&h)["gamma"], 2.0);
                let st = h.state().session.active().unwrap();
                let pixels = st.doc.layers[0].content.clone();
                let history = st.history.past_len();
                let before = value(h.state(), target);
                // The standalone preview helper starts from a composite session. Supply the
                // plane that the live preview request captures from Channels/Quick Mask.
                let mut preview_params = params(&h);
                preview_params["target"] = match target {
                    ChannelTarget::Alpha(i) => json!({"channel":i}),
                    ChannelTarget::LayerMask => json!("mask"),
                    _ => json!("quickMask"),
                };
                let preview = filter_dialog::preview_document(&st.doc, st.active_layer, "image.adjustments.levels", &preview_params, 1).unwrap();
                let preview_value = match target {
                    ChannelTarget::Alpha(i) => preview.channels[i].surface.sample_channel(8, 8, 0),
                    ChannelTarget::LayerMask => preview.layers[0].mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
                    _ => preview.quick_mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
                };
                let id = h.state().ui.dialogs.last().unwrap().id;
                dialogs::confirm(h.state_mut(), id).unwrap();
                let after = value(h.state(), target);
                assert_eq!(preview_value, after, "the gamma preview matches committed native samples");
                assert!(after > before + 0.1, "{mode:?} {depth:?} {target:?}: {before} -> {after}");
                assert_eq!(h.state().session.active().unwrap().doc.layers[0].content, pixels);
                assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
                h.state_mut().run("edit.undo", json!({})).unwrap();
                assert_eq!(value(h.state(), target), before);
                h.state_mut().run("edit.redo", json!({})).unwrap();
                assert_eq!(value(h.state(), target), after);
            }
        }
    }
}

/// Each RGB/CMYK/Lab component exposes only its own Levels record and preserves other components.
#[test]
fn levels_isolates_each_colour_channel_and_edits_its_matching_record() {
    for mode in MODES {
        let labels: &[&str] = match mode {
            ColorMode::Rgb => &["Red", "Green", "Blue"],
            ColorMode::Cmyk => &["Cyan", "Magenta", "Yellow", "Black"],
            ColorMode::Lab => &["Lightness", "a", "b"],
            _ => unreachable!(),
        };
        let keys = photocraft_engine::adjust_params::channel_keys(photocraft_engine::adjust_params::default_space(mode));
        for depth in DEPTHS {
            for (k, label) in labels.iter().enumerate() {
                let mut app = fixture(mode, depth, ChannelTarget::Color(k));
                // Nonzero samples let gamma change each RGB component as well as Lab/CMYK.
                app.session
                    .edit("Colour pattern", |doc, _| {
                        let bounds = doc.bounds();
                        let n = mode.color_channels();
                        let mut values = vec![0.25; n + 1];
                        values[n] = 0.6;
                        doc.layers[0].surface_mut().unwrap().fill_rect(bounds, &values);
                        Ok(())
                    })
                    .unwrap();
                let mut h = open(app, "levels");
                assert_eq!(names(&h), [*label], "{mode:?} {depth:?} {k}");
                let before = params(&h);
                let original = h.state().session.active().unwrap().doc.clone();
                edit_gamma(&mut h, "2.0");
                let after = params(&h);
                assert_eq!(after[keys[k]]["gamma"], 2.0);
                for (i, key) in keys.iter().enumerate().filter(|(i, _)| *i != k) {
                    assert_eq!(after[key], before[key], "untargeted {i}");
                }
                let id = h.state().ui.dialogs.last().unwrap().id;
                dialogs::confirm(h.state_mut(), id).unwrap();
                let doc = &h.state().session.active().unwrap().doc;
                let old = original.layers[0].surface().unwrap().pixel(8, 8);
                let new = doc.layers[0].surface().unwrap().pixel(8, 8);
                assert!((new[k] - old[k]).abs() > 0.01, "{mode:?} {depth:?} {k}: {old:?} -> {new:?}");
                for i in (0..old.len()).filter(|i| *i != k) {
                    assert_eq!(new[i], old[i], "untargeted sample/transparency {i}");
                }
                assert_eq!(doc.channels, original.channels);
                assert_eq!(doc.layers[0].mask, original.layers[0].mask);
            }
        }
    }
}

/// Spot names stay literal and Layers-panel mask targeting exposes the same named gray controls.
#[test]
fn levels_uses_literal_spot_name_and_layers_panel_mask_name() {
    for mode in GRAY_TARGET_MODES {
        let mut app = fixture(mode, SampleType::U8, ChannelTarget::Alpha(0));
        app.session
            .edit("Named spot", |doc, _| {
                doc.channels[0].name = "Sky α — ink 1".into();
                doc.channels[0].spot = Some((photocraft_color::Color::rgb(0.2, 0.4, 0.6), 0.8));
                Ok(())
            })
            .unwrap();
        assert_eq!(names(&open(app, "levels")), ["Sky α — ink 1"]);

        let mut app = fixture(mode, SampleType::U8, ChannelTarget::LayerMask);
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
        app.ui.mask_target = true;
        let mut h = open(app, "levels");
        assert_eq!(names(&h), ["Named layer Mask"]);
        assert_eq!(params(&h)["target"], "mask");
        let id = h.state().ui.dialogs.last().unwrap().id;
        let mut fields = h.state().ui.dialogs.last().unwrap().fields.clone();
        adjust_dialog::reset(h.state(), &h.ctx, &mut fields);
        h.state_mut().ui.dialog_mut(id).unwrap().fields = fields;
        h.run_steps(2);
        assert_eq!(names(&h), ["Named layer Mask"]);
        assert_eq!(params(&h)["target"], "mask");
    }
}

/// Levels Auto derives endpoints from the selected component’s histogram rather than the composite.
#[test]
fn levels_auto_uses_the_selected_colour_channels_histogram() {
    let mut app = fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Color(1));
    app.session
        .edit("Distinct colour components", |doc, _| {
            let bounds = doc.bounds();
            doc.layers[0].mask = None;
            doc.layers[0].surface_mut().unwrap().fill_rect(bounds, &[0.1, 0.4, 0.8, 1.0]);
            Ok(())
        })
        .unwrap();
    let mut h = open(app, "levels");
    assert_eq!(names(&h), ["Green"]);
    let before = params(&h);
    h.get_by_label("Auto").click();
    h.run_steps(2);
    let after = params(&h);
    assert_eq!(after["green"]["inBlack"], 102.0);
    assert_eq!(after["green"]["inWhite"], 104.0);
    for key in ["red", "blue", "inBlack", "inWhite"] {
        assert_eq!(after[key], before[key]);
    }
}

/// Black/white picker controls sample the gray plane and update the visible neutral curve.
#[test]
fn grayscale_picker_controls_sample_the_plane_and_change_the_visible_curve() {
    for kind in ["curves", "levels"] {
        for mode in GRAY_TARGET_MODES {
            for depth in DEPTHS {
                for target in TARGETS {
                    let mut h = open(fixture(mode, depth, target), kind);
                    assert!(h.get_by_label("Set Neutral Gray Point").accesskit_node().is_disabled());
                    assert!(!h.get_by_label("Set White Point").accesskit_node().is_disabled());
                    h.get_by_label("Set White Point").click();
                    h.run_steps(2);
                    assert!(adjust_dialog::picker_armed(h.state()));
                    let before = params(&h);
                    let original = h.state().session.active().unwrap().doc.clone();
                    assert!(adjust_dialog::sample_at(h.state_mut(), 8.0, 8.0));
                    let after = params(&h);
                    let key = if kind == "curves" { "points" } else { "inWhite" };
                    assert_ne!(before[key], after[key], "{kind} {mode:?} {depth:?} {target:?}");
                    let sampled_level = if kind == "curves" {
                        after["points"].as_array().unwrap().last().unwrap()[0].as_f64().unwrap()
                    } else {
                        after["inWhite"].as_f64().unwrap()
                    };
                    assert!((sampled_level - 63.75).abs() < 0.3, "sample the plane's 25% gray, not the colour layer: {after}");
                    for key in ["red", "green", "blue"] {
                        assert_eq!(before[key], after[key], "hidden {key} must not change");
                    }
                    assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
                    let id = h.state().ui.dialogs.last().unwrap().id;
                    dialogs::confirm(h.state_mut(), id).unwrap();
                    assert!(value(h.state(), target) > 0.99, "{kind} {mode:?} {depth:?} {target:?}");
                    assert_eq!(h.state().session.active().unwrap().doc.layers[0].content, original.layers[0].content);
                }
            }
        }
    }
}

/// Replay a recorded tone step with its dialog enabled while a different channel is selected.
fn action_dialog(mode: ColorMode, target: Value, panel: ChannelTarget, kind: &str) -> Harness<'static, PhotocraftApp> {
    let mut app = fixture(mode, SampleType::U8, panel);
    app.run("actions.record", json!({"name": "Tone step"})).unwrap();
    // Imported/serialized steps can carry document-space defaults alongside their explicit
    // grayscale target. Those records must not leak back into the dialog after normalization.
    let mut recorded = photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for(kind, mode).unwrap());
    recorded["target"] = target;
    for key in ["red", "green", "blue"] {
        recorded[key] = if kind == "levels" { json!({"gamma": 4}) } else { json!([[0, 255], [255, 255]]) };
    }
    app.run(&format!("image.adjustments.{kind}"), recorded).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    app.run("actions.dialog", json!({"action": "Tone step", "step": 0, "enabled": true})).unwrap();
    let mut h = harness(app);
    let result = h.state_mut().run("actions.play", json!({"action": "Tone step"})).unwrap();
    assert!(result.get("paused").is_some(), "{result:?}");
    h.run_steps(4);
    h
}

/// A recorded alpha target overrides a currently selected colour component in controls and commit.
#[test]
fn action_dialog_uses_recorded_alpha_instead_of_current_red_channel() {
    for mode in MODES {
        for kind in ["curves", "levels"] {
            let mut h = action_dialog(mode, json!({"channel": 0}), ChannelTarget::Color(0), kind);
            assert_eq!(names(&h), ["Saved sky"]);
            assert_eq!(params(&h)["target"], json!({"channel": 0}));
            let neutral = photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for(kind, ColorMode::Grayscale).unwrap());
            for key in ["red", "green", "blue"] {
                assert_eq!(params(&h)[key], neutral[key], "hidden {key} starts neutral");
            }
            for key in ["cyan", "black", "lightness", "a", "b"] {
                assert!(params(&h).get(key).is_none());
            }
            let id = h.state().ui.dialogs.last().unwrap().id;
            let expected = if kind == "curves" {
                h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("points".into(), json!([[0, 255], [255, 0]]));
                0.75
            } else {
                edit_gamma(&mut h, "2.0");
                0.5
            };
            let before = h.state().session.active().unwrap().doc.layers[0].content.clone();
            dialogs::confirm(h.state_mut(), id).unwrap();
            assert!((value(h.state(), ChannelTarget::Alpha(0)) - expected).abs() < 0.01);
            assert_eq!(h.state().session.active().unwrap().doc.layers[0].content, before);
        }
    }
}

/// Reset, pencil conversion and sampling retain an Action’s explicit mask target.
#[test]
fn action_mask_target_survives_reset_pencil_and_picker() {
    for kind in ["curves", "levels"] {
        let mut h = action_dialog(ColorMode::Lab, json!("mask"), ChannelTarget::Alpha(0), kind);
        assert_eq!(names(&h), ["Named layer Mask"]);
        let id = h.state().ui.dialogs.last().unwrap().id;
        let mut fields = h.state().ui.dialogs.last().unwrap().fields.clone();
        adjust_dialog::reset(h.state(), &h.ctx, &mut fields);
        h.state_mut().ui.dialog_mut(id).unwrap().fields = fields;
        h.run_steps(2);
        assert_eq!(params(&h)["target"], "mask");
        if kind == "curves" {
            h.get_by_label("Draw to modify the curve").click();
            h.run_steps(2);
            assert_eq!(params(&h)["target"], "mask");
            assert_eq!(params(&h)["pencil"], true);
        }
        h.get_by_label("Set White Point").click();
        h.run_steps(2);
        assert!(adjust_dialog::sample_at(h.state_mut(), 8.0, 8.0));
        assert_eq!(params(&h)["target"], "mask");
        let alpha = h.state().session.active().unwrap().doc.channels[0].clone();
        dialogs::confirm(h.state_mut(), id).unwrap();
        assert!(value(h.state(), ChannelTarget::LayerMask) > 0.99);
        assert_eq!(h.state().session.active().unwrap().doc.channels[0], alpha);
    }
}

/// Explicit scalar Action parameters use the channel preview path independently of panel state.
#[test]
fn explicit_channel_action_uses_channel_preview_even_with_composite_selected() {
    let mut h = action_dialog(ColorMode::Rgb, json!({"channel":0}), ChannelTarget::Color(0), "curves");
    h.state_mut().session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
    assert!(!adjust_preview::on_layer(h.state_mut(), 0));
    assert_eq!(names(&h), ["Saved sky"]);
    let id = h.state().ui.dialogs.last().unwrap().id;
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("points".into(), json!([[0, 255], [255, 0]]));
    let st = h.state().session.active().unwrap();
    let preview = filter_dialog::preview_document(&st.doc, st.active_layer, "image.adjustments.curves", &params(&h), 1).unwrap();
    dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(preview.channels, h.state().session.active().unwrap().doc.channels);
    assert_eq!(preview.layers, h.state().session.active().unwrap().doc.layers);
}

/// An explicit pixels Action keeps colour controls and edits pixels while an alpha is selected.
#[test]
fn explicit_pixels_action_bypasses_current_alpha_target() {
    let mut app = fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Alpha(0));
    let mut raw = json!({"target":"pixels","points":[[0,0],[255,255]]});
    for key in ["red", "green", "blue"] {
        raw[key] = json!([[0, 255], [255, 255]]);
    }
    let mut action = photocraft_engine::actions_cmds::Action::new("Pixel edit", vec![("image.adjustments.curves".into(), raw)]);
    action.dialogs.insert(0);
    app.session.actions.list.push(action);
    let mut h = harness(app);
    h.state_mut().run("actions.play", json!({"action":"Pixel edit"})).unwrap();
    h.run_steps(4);
    assert_eq!(names(&h), ["RGB"]);
    for key in ["red", "green", "blue"] {
        assert_eq!(params(&h)[key], json!([[0, 255], [255, 255]]), "pixel targets retain their recorded {key} curve");
    }
    h.get_by_label("Set White Point").click();
    h.run_steps(2);
    let base = params(&h);
    let expected = photocraft_engine::adjust_params::adjustment_eyedropper(
        "curves",
        &base,
        photocraft_engine::adjust_params::CurvesEyedropper::White,
        [0.0; 3],
        ColorMode::Rgb,
    )
    .unwrap();
    assert!(adjust_dialog::sample_at(h.state_mut(), 8.0, 8.0));
    let sampled = photocraft_engine::adjust_params::from_params("curves", &params(&h), None, ColorMode::Rgb).unwrap();
    assert_eq!(sampled, expected, "the picker writes the curve for the committed black pixel");
    assert_ne!(params(&h)["points"], base["points"], "the sample cannot silently do nothing");
    assert_eq!(params(&h)["target"], "pixels");
    let before = h.state().session.active().unwrap().doc.clone();
    let id = h.state().ui.dialogs.last().unwrap().id;
    // A known non-neutral replay result must actually reach the pixels, while the selected
    // alpha remains untouched. A silently dropped adjustment must fail this assertion.
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("points".into(), json!([[0, 64], [255, 255]]));
    for key in ["red", "green", "blue"] {
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(key.into(), json!([[0, 0], [255, 255]]));
    }
    dialogs::confirm(h.state_mut(), id).unwrap();
    let after = &h.state().session.active().unwrap().doc;
    assert_eq!(after.channels, before.channels);
    let old = before.layers[0].surface().unwrap().pixel(8, 8);
    let new = after.layers[0].surface().unwrap().pixel(8, 8);
    assert!(old[..3].iter().all(|v| *v == 0.0));
    assert!(new[..3].iter().all(|v| (*v - 64.0 / 255.0).abs() < 0.005));
    assert_eq!(new[3], old[3]);
}

/// Rejected samples and Cancel leave all committed document data and history unchanged.
#[test]
fn invalid_grayscale_sample_and_cancel_preserve_the_document() {
    let mut h = open(fixture(ColorMode::Lab, SampleType::F32, ChannelTarget::Alpha(0)), "curves");
    h.get_by_label("Set White Point").click();
    h.run_steps(2);
    let before = params(&h);
    let original = h.state().session.active().unwrap().doc.clone();
    for (x, y) in [(-1.0, 8.0), (99.0, 8.0), (f64::NAN, 8.0)] {
        assert!(adjust_dialog::sample_at(h.state_mut(), x, y));
        assert_eq!(params(&h), before);
    }
    let id = h.state().ui.dialogs.last().unwrap().id;
    // Automation can supply malformed private dialog fields too.
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("__curvePickerBase".into(), json!([]));
    assert!(adjust_dialog::sample_at(h.state_mut(), 8.0, 8.0));
    assert_eq!(params(&h), before);
    dialogs::cancel(h.state_mut(), id).unwrap();
    assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
}

/// Reset discards the armed picker’s saved curve so the next sample uses neutral settings.
#[test]
fn resetting_an_armed_picker_does_not_restore_its_old_curve() {
    let mut h = open(fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Alpha(0)), "curves");
    let id = h.state().ui.dialogs.last().unwrap().id;
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("points".into(), json!([[0, 64], [255, 255]]));
    h.get_by_label("Set White Point").click();
    h.run_steps(2);
    assert!(adjust_dialog::sample_at(h.state_mut(), 8.0, 8.0));
    let mut fields = h.state().ui.dialogs.last().unwrap().fields.clone();
    adjust_dialog::reset(h.state(), &h.ctx, &mut fields);
    h.state_mut().ui.dialog_mut(id).unwrap().fields = fields;
    assert!(adjust_dialog::sample_at(h.state_mut(), 8.0, 8.0));
    assert_eq!(params(&h)["points"][0][1], 0.0);
}

/// A deleted explicit channel fails preview and commit instead of falling back to colour pixels.
#[test]
fn missing_explicit_channel_cannot_preview_as_a_colour_layer() {
    let mut h = open(fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Color(0)), "curves");
    h.state_mut().session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
    let id = h.state().ui.dialogs.last().unwrap().id;
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("target".into(), json!({"channel":999}));
    assert!(!adjust_preview::on_layer(h.state_mut(), 0));
    let original = h.state().session.active().unwrap().doc.clone();
    assert!(dialogs::confirm(h.state_mut(), id).is_err());
    assert_eq!(h.state().session.active().unwrap().doc.channels, original.channels);
    assert_eq!(h.state().session.active().unwrap().doc.layers, original.layers);
}

/// A recorded `gray` alias must seed editable composite controls, rather than override them invisibly.
#[test]
fn scalar_action_gray_alias_seeds_the_visible_controls() {
    for kind in ["levels", "curves"] {
        let mut app = fixture(ColorMode::Lab, SampleType::U8, ChannelTarget::Color(0));
        let gray = if kind == "levels" { json!({"gamma":2}) } else { json!([[0, 255], [255, 0]]) };
        let mut action = photocraft_engine::actions_cmds::Action::new(
            "Gray alias",
            vec![(format!("image.adjustments.{kind}"), json!({"target":{"channel":0},"gray":gray}))],
        );
        action.dialogs.insert(0);
        app.session.actions.list.push(action);
        let mut h = harness(app);
        h.state_mut().run("actions.play", json!({"action":"Gray alias"})).unwrap();
        h.run_steps(4);
        assert_eq!(names(&h), ["Saved sky"]);
        let id = h.state().ui.dialogs.last().unwrap().id;
        if kind == "levels" {
            assert_eq!(params(&h)["gamma"].as_f64(), Some(2.0), "the visible gamma agrees with the recorded gray gamma");
            edit_gamma(&mut h, "1.0");
        } else {
            assert_eq!(params(&h)["points"], json!([[0.0, 255.0], [255.0, 0.0]]), "the visible curve is the recorded gray curve");
            h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("points".into(), json!([[0, 0], [255, 255]]));
        }
        assert!(params(&h).get("gray").is_none(), "a hidden alias must not override subsequent control edits");
        let before = h.state().session.active().unwrap().doc.clone();
        dialogs::confirm(h.state_mut(), id).unwrap();
        assert!((value(h.state(), ChannelTarget::Alpha(0)) - 0.25).abs() < 0.005, "editing the visible controls back to neutral is effective");
        assert_eq!(h.state().session.active().unwrap().doc.layers[0].content, before.layers[0].content);
    }
}

/// Malformed aliases remain command errors; neutral dialog defaults must not conceal them.
#[test]
fn invalid_scalar_action_gray_alias_cannot_commit() {
    for (kind, gray) in [("levels", json!({"gamma":"invalid"})), ("curves", json!("invalid")), ("curves", json!([[0, 0]]))] {
        let mut app = fixture(ColorMode::Lab, SampleType::U8, ChannelTarget::Color(0));
        let command = format!("image.adjustments.{kind}");
        let mut action = photocraft_engine::actions_cmds::Action::new("Invalid alias", vec![(command.clone(), json!({"target":{"channel":0},"gray":gray}))]);
        action.dialogs.insert(0);
        app.session.actions.list.push(action);
        let mut h = harness(app);
        h.state_mut().run("actions.play", json!({"action":"Invalid alias"})).unwrap();
        h.run_steps(4);
        let original = h.state().session.active().unwrap().doc.clone();
        let layer = h.state().session.active().unwrap().active_layer;
        assert!(filter_dialog::preview_document(&original, layer, &command, &params(&h), 1).is_none());
        let id = h.state().ui.dialogs.last().unwrap().id;
        assert!(dialogs::confirm(h.state_mut(), id).is_err());
        assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
        assert!(h.state().ui.dialogs.is_empty());
    }
}

/// Play an actual recorded modal step while a different scalar channel is selected.
fn colour_set_action(mode: ColorMode, depth: SampleType, kind: &str, recorded: Value) -> Harness<'static, PhotocraftApp> {
    colour_set_action_with_selection(mode, depth, kind, recorded, false)
}

/// Set up selection before playback: a paused Action owns its captured document/history.
fn colour_set_action_with_selection(mode: ColorMode, depth: SampleType, kind: &str, recorded: Value, feathered: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = fixture(mode, depth, ChannelTarget::Alpha(0));
    app.session
        .edit("Colour pattern", |doc, _| {
            let bounds = doc.bounds();
            let n = mode.color_channels();
            let mut samples = Vec::new();
            for y in 0..32 {
                for x in 0..32 {
                    for k in 0..n {
                        samples.push(((x * 7 + y * 11 + k * 37) % 221 + 15) as f32 / 255.0);
                    }
                    samples.push(0.65);
                }
            }
            doc.layers[0].surface_mut().unwrap().write_region(bounds, &samples);
            if feathered {
                let mut selection = Surface::new(PixelFormat::GRAY8);
                selection.fill_rect(photocraft_geom::Rect::new(4, 4, 24, 24), &[0.5]);
                doc.selection = Some(selection);
            }
            Ok(())
        })
        .unwrap();
    let mut action = photocraft_engine::actions_cmds::Action::new("Colour set alias", vec![(format!("image.adjustments.{kind}"), recorded)]);
    action.dialogs.insert(0);
    app.session.actions.list.push(action);
    let mut h = harness(app);
    h.state_mut().run("actions.play", json!({"action":"Colour set alias"})).unwrap();
    h.run_steps(4);
    h
}

/// Build the original serialized Action records, independently of dialog normalization.
fn colour_set_alias_params(mode: ColorMode, indices: &[usize], kind: &str, pencil: bool, native: bool) -> Value {
    let keys = photocraft_engine::adjust_params::channel_keys(photocraft_engine::adjust_params::default_space(mode));
    let mut raw = if kind == "levels" { json!({"gamma":1.25,"gray":{"gamma":2}}) } else { json!({"gray":[[0,255],[100,30],[255,0]],"pencil":pencil}) };
    raw["target"] = json!({"colors":indices});
    if pencil {
        raw["smooth"] = json!(2);
    }
    if native {
        for key in keys {
            raw[*key] = if kind == "levels" { json!({"gamma":1.5}) } else { json!([[0, 0], [100, 180], [255, 255]]) };
        }
    }
    raw
}

/// Legacy colour-set aliases seed the real controls and preserve native records, including
/// Lab's separate combined/Lightness records and once-only pencil conversion and smoothing.
#[test]
fn colour_set_action_gray_alias_matches_direct_command_preview_commit_and_undo() {
    for mode in MODES {
        let sets: [(&[usize], &str); 2] = match mode {
            ColorMode::Rgb => [(&[0, 2], "RB"), (&[1, 2], "GB")],
            ColorMode::Cmyk => [(&[0, 2], "CY"), (&[0, 3], "CB")],
            ColorMode::Lab => [(&[0, 2], "Lb"), (&[1, 2], "ab")],
            _ => unreachable!(),
        };
        for depth in DEPTHS {
            for (indices, label) in sets {
                for (kind, pencil) in [("levels", false), ("curves", false), ("curves", true)] {
                    for native in [false, true] {
                        let raw = colour_set_alias_params(mode, indices, kind, pencil, native);
                        let mut h = colour_set_action_with_selection(mode, depth, kind, raw.clone(), native);
                        let context = format!("{mode:?} {depth:?} {label} {kind} pencil={pencil} native={native}");
                        assert_eq!(names(&h), [label], "{context}");
                        let fields = params(&h);
                        assert!(fields.get("gray").is_none(), "{context}: alias must not override visible controls: {fields}");
                        assert!(fields.get("smooth").is_none(), "{context}: smoothing has already been applied");
                        if kind == "levels" {
                            assert_eq!(fields["gamma"], 2.0, "{context}");
                        }
                        if pencil {
                            assert_eq!(fields["points"].as_array().unwrap().len(), 256, "{context}");
                        }
                        let st = h.state().session.active().unwrap();
                        let original = st.doc.clone();
                        let layer = st.active_layer;
                        let mut oracle = Session::new();
                        oracle.add_document((*original).clone(), None);
                        oracle.select_layer(layer.unwrap()).unwrap();
                        let command = format!("image.adjustments.{kind}");
                        oracle.execute(&command, raw.clone()).unwrap();
                        let expected = &oracle.active().unwrap().doc;
                        let preview = filter_dialog::preview_document(&original, layer, &command, &fields, 1).unwrap();
                        assert_eq!(preview.layers, expected.layers, "{context}: preview must match the recorded command");
                        assert_eq!(preview.channels, original.channels, "{context}");
                        assert_eq!(preview.selection, original.selection, "{context}");
                        assert_eq!(preview.layers[0].mask, original.layers[0].mask, "{context}");
                        let before = original.layers[0].surface().unwrap().pixel(8, 8);
                        let after = preview.layers[0].surface().unwrap().pixel(8, 8);
                        for k in (0..before.len()).filter(|k| !indices.contains(k)) {
                            assert_eq!(after[k].to_bits(), before[k].to_bits(), "{context}: untouched component/alpha {k}");
                        }
                        let id = h.state().ui.dialogs.last().unwrap().id;
                        dialogs::confirm(h.state_mut(), id).unwrap();
                        assert_eq!(h.state().session.active().unwrap().doc.layers, expected.layers, "{context}: commit");
                        assert_eq!(h.state().session.active().unwrap().channel_view.target, ChannelTarget::Alpha(0), "{context}: panel target is view state");
                        assert_eq!(h.state().session.actions.list[0].steps[0].1, raw, "dialog normalization must not rewrite the stored Action");
                        h.state_mut().run("edit.undo", json!({})).unwrap();
                        assert_eq!(h.state().session.active().unwrap().doc.layers, original.layers, "{context}: undo");
                        h.state_mut().run("edit.redo", json!({})).unwrap();
                        assert_eq!(h.state().session.active().unwrap().doc.layers, expected.layers, "{context}: redo");
                    }
                }
            }
        }
    }
}

/// Editing the visible combined record back to neutral removes the recorded alias's effect.
#[test]
fn colour_set_action_gray_alias_does_not_override_visible_edits() {
    for mode in MODES {
        for (kind, pencil) in [("levels", false), ("curves", false), ("curves", true)] {
            let mut raw = if kind == "levels" { json!({"gray":{"gamma":2}}) } else { json!({"gray":[[0,255],[255,0]],"pencil":pencil}) };
            raw["target"] = json!({"colors":[0,2]});
            let mut h = colour_set_action(mode, SampleType::U16, kind, raw);
            let original = h.state().session.active().unwrap().doc.clone();
            let id = h.state().ui.dialogs.last().unwrap().id;
            if kind == "levels" {
                edit_gamma(&mut h, "1.0");
            } else {
                let identity = if pencil { json!((0..256).collect::<Vec<_>>()) } else { json!([[0, 0], [255, 255]]) };
                h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("points".into(), identity);
            }
            assert!(params(&h).get("gray").is_none());
            dialogs::confirm(h.state_mut(), id).unwrap();
            assert_eq!(h.state().session.active().unwrap().doc.layers, original.layers, "{mode:?} {kind} pencil={pencil}");
            assert_eq!(h.state().session.active().unwrap().doc.channels, original.channels);
        }
    }
}

/// Invalid aliases cannot be hidden by seeded identity controls or mutate document pixels.
#[test]
fn invalid_colour_set_action_gray_alias_cannot_preview_or_commit() {
    for mode in MODES {
        for (kind, gray) in [("levels", json!({"gamma":"invalid"})), ("curves", json!("invalid")), ("curves", json!([[0, 0]]))] {
            let mut h = colour_set_action(mode, SampleType::F32, kind, json!({"target":{"colors":[0,2]},"gray":gray}));
            let st = h.state().session.active().unwrap();
            let original = st.doc.clone();
            let command = format!("image.adjustments.{kind}");
            assert!(filter_dialog::preview_document(&original, st.active_layer, &command, &params(&h), 1).is_none(), "{mode:?} {kind}");
            let id = h.state().ui.dialogs.last().unwrap().id;
            assert!(dialogs::confirm(h.state_mut(), id).is_err());
            assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
        }
    }
}
/// Alt+adjustment recalls settings for the current scalar plane without resurrecting colour records or old addressing.
#[test]
fn last_tone_settings_follow_the_current_scalar_target() {
    for mode in GRAY_TARGET_MODES {
        for target in TARGETS {
            for kind in ["levels", "curves"] {
                let mut app = fixture(mode, SampleType::U8, target);
                let command = format!("image.adjustments.{kind}");
                let mut last = if kind == "levels" { json!({"gamma":2.0}) } else { json!({"points":[[0,255],[255,0]]}) };
                let colour = if kind == "levels" { json!({"gamma":3.0}) } else { json!([[0, 255], [255, 0]]) };
                for key in ["red", "green", "blue", "cyan", "magenta", "yellow", "black", "lightness", "a", "b"] {
                    last[key] = colour.clone();
                }
                // Preferences from an earlier explicit-target dialog must never restore that address.
                last["target"] = json!({"channel":999});
                last["layer"] = json!(999);
                last["document"] = json!(999);
                app.session.edit_prefs(|p| {
                    p.dialogs.insert(command.clone(), last);
                });
                let mut h = harness(app);
                let id = adjust_dialog::open_last(h.state_mut(), &command).unwrap();
                h.run_steps(4);
                let fields = params(&h);
                for key in ["red", "green", "blue"] {
                    let neutral = if kind == "levels" { fields[key]["gamma"] == 1.0 } else { fields[key] == json!([[0.0, 0.0], [255.0, 255.0]]) };
                    assert!(neutral, "{mode:?} {target:?} {kind}: hidden {key} must stay neutral: {fields}");
                }
                assert!(fields.get("layer").is_none() && fields.get("document").is_none());
                if target == ChannelTarget::LayerMask {
                    assert_eq!(fields["target"], "mask", "retain the current mask target");
                } else {
                    assert!(fields.get("target").is_none(), "do not restore the old explicit target");
                }
                let before = h.state().session.active().unwrap().doc.layers[0].content.clone();
                dialogs::confirm(h.state_mut(), id).unwrap();
                assert!(value(h.state(), target) > 0.4, "recalled settings must affect {mode:?} {target:?} {kind}");
                assert_eq!(h.state().session.active().unwrap().doc.layers[0].content, before);
                let remembered = &h.state().session.prefs().dialogs[&command];
                for key in ["target", "layer", "document"] {
                    assert!(remembered.get(key).is_none(), "last-used settings must not persist {key}");
                }
            }
        }
    }
}

/// Last-used colour records remain editable in their own space; incompatible spaces and malformed settings fall back to neutral.
#[test]
fn last_tone_settings_preserve_compatible_colour_controls_and_reject_bad_settings() {
    use photocraft_engine::adjust_params::{default_for, from_params, to_params};
    for mode in MODES {
        for kind in ["levels", "curves"] {
            let command = format!("image.adjustments.{kind}");
            let key = photocraft_engine::adjust_params::channel_keys(photocraft_engine::adjust_params::default_space(mode))[0];
            let record = if kind == "levels" { json!({"gamma":2.0}) } else { json!([[0, 255], [255, 0]]) };
            let mut supplied = json!({});
            supplied[key] = record;
            let compatible = to_params(&from_params(kind, &supplied, None, mode).unwrap());
            let wrong_mode = if mode == ColorMode::Rgb { ColorMode::Lab } else { ColorMode::Rgb };
            let incompatible = to_params(&default_for(kind, wrong_mode).unwrap());
            let invalid = if kind == "levels" { json!({"gamma":"invalid"}) } else { json!({"gray":[[0,0]]}) };
            for (last, expected) in [
                (compatible.clone(), compatible),
                (incompatible, to_params(&default_for(kind, mode).unwrap())),
                (invalid, to_params(&default_for(kind, mode).unwrap())),
            ] {
                let mut app = fixture(mode, SampleType::U8, ChannelTarget::Color(0));
                app.session.edit_prefs(|p| {
                    p.dialogs.insert(command.clone(), last.clone());
                });
                let original = app.session.active().unwrap().doc.clone();
                let mut h = harness(app);
                let id = adjust_dialog::open_last(h.state_mut(), &command).unwrap();
                h.run_steps(4);
                // Main captures the selected native component in each modal. Recall preserves
                // that address alongside the settings, while preferences remain address-free.
                let mut expected = expected;
                expected["target"] = json!({"colors":[0]});
                assert_eq!(params(&h), expected, "{mode:?} {kind}: {last}");
                dialogs::cancel(h.state_mut(), id).unwrap();
                assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
                assert_eq!(h.state().session.prefs().dialogs[&command], last, "Cancel preserves remembered settings");
            }
        }
    }
}

/// Grayscale layer pixels expose one curve too, so recalling an RGB dialog must not introduce hidden channel edits.
#[test]
fn last_tone_settings_on_grayscale_pixels_drop_hidden_colour_records() {
    for kind in ["levels", "curves"] {
        let mut app = fixture(ColorMode::Grayscale, SampleType::U8, ChannelTarget::Color(0));
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
        let command = format!("image.adjustments.{kind}");
        let last = if kind == "levels" { json!({"gamma":2.0,"red":{"gamma":3.0}}) } else { json!({"points":[[0,255],[255,0]],"red":[[0,255],[255,0]]}) };
        app.session.edit_prefs(|p| {
            p.dialogs.insert(command.clone(), last);
        });
        let mut h = harness(app);
        let id = adjust_dialog::open_last(h.state_mut(), &command).unwrap();
        let fields = params(&h);
        if kind == "levels" {
            assert_eq!(fields["red"]["gamma"], 1.0);
            assert_eq!(fields["gamma"], 2.0);
        } else {
            assert_eq!(fields["red"], json!([[0.0, 0.0], [255.0, 255.0]]));
            assert_eq!(fields["points"], json!([[0.0, 255.0], [255.0, 0.0]]));
        }
        dialogs::cancel(h.state_mut(), id).unwrap();
    }
}

/// Build a filtered smart object with a selected filter mask, optionally still the implicit reveal-all mask.
fn filter_mask_fixture(mode: ColorMode, depth: SampleType, stored: bool) -> PhotocraftApp {
    let mut app = fixture(mode, depth, ChannelTarget::Color(0));
    app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius":1})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    if stored {
        app.run("edit.fill", json!({"target":"filterMask","contents":"color","color":"#404040"})).unwrap();
    }
    app.ui.filter_mask_target = Some(id.0);
    app
}

/// New upstream filter masks use the same scalar controls, picker and preserved targeting in every document space.
#[test]
fn filter_mask_tone_dialogs_preserve_the_target_and_sample_its_own_levels() {
    for mode in GRAY_TARGET_MODES {
        for depth in DEPTHS {
            for kind in ["levels", "curves"] {
                for stored in [false, true] {
                    let app = filter_mask_fixture(mode, depth, stored);
                    let st = app.session.active().unwrap();
                    let layer = st.active_layer.unwrap();
                    let expected_name = format!("{} Filter Mask", st.doc.layer(layer).unwrap().name);
                    let before = st.doc.clone();
                    let mut h = open(app, kind);
                    assert_eq!(names(&h), [expected_name], "{mode:?} {depth:?} {kind}");
                    assert_eq!(params(&h)["target"], "filterMask");
                    let id = h.state().ui.dialogs.last().unwrap().id;
                    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("__curvePicker".into(), json!("white"));
                    assert!(adjust_dialog::sample_at(h.state_mut(), 8.0, 8.0));
                    assert!(
                        h.state().ui.dialogs.last().unwrap().fields.contains_key("__curvePickerBase"),
                        "the picker succeeds on {mode:?} {depth:?} stored={stored}: {}",
                        h.state().ui.status
                    );
                    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.active().unwrap().doc), "sampling is read-only");
                    assert_eq!(params(&h)["target"], "filterMask");
                    if kind == "levels" {
                        let expected = if stored { 64.0 } else { 255.0 };
                        assert!((params(&h)["inWhite"].as_f64().unwrap() - expected).abs() < 1.0);
                    }
                    dialogs::confirm(h.state_mut(), id).unwrap();
                    let after = &h.state().session.active().unwrap().doc;
                    let LayerContent::Smart(old) = &before.layer(layer).unwrap().content else { panic!("smart fixture") };
                    let LayerContent::Smart(new) = &after.layer(layer).unwrap().content else { panic!("smart result") };
                    assert_eq!(new.smart_filters, old.smart_filters, "the adjustment edits the mask rather than adding a filter");
                    assert_eq!(new.source, old.source);
                    assert_eq!(after.channels, before.channels);
                    assert_eq!(after.layer(layer).unwrap().mask, before.layer(layer).unwrap().mask);
                    assert!(new.filter_mask.as_ref().unwrap().surface.sample_channel(8, 8, 0) > 0.99);
                }
            }
        }
    }
}

/// A recorded alpha address takes precedence over the currently selected filter mask.
#[test]
fn action_alpha_target_overrides_the_filter_mask_panel_target() {
    for mode in MODES {
        for kind in ["levels", "curves"] {
            let mut app = filter_mask_fixture(mode, SampleType::U8, false);
            let before = app.session.active().unwrap().doc.clone();
            let values = if kind == "levels" { json!({"target":{"channel":0},"gamma":2}) } else { json!({"target":{"channel":0},"points":[[0,255],[255,0]]}) };
            let mut action = photocraft_engine::actions_cmds::Action::new("Alpha over filter mask", vec![(format!("image.adjustments.{kind}"), values)]);
            action.dialogs.insert(0);
            app.session.actions.list.push(action);
            let mut h = harness(app);
            h.state_mut().run("actions.play", json!({"action":"Alpha over filter mask"})).unwrap();
            h.run_steps(4);
            assert_eq!(names(&h), ["Saved sky"]);
            assert_eq!(params(&h)["target"], json!({"channel":0}));
            let id = h.state().ui.dialogs.last().unwrap().id;
            dialogs::confirm(h.state_mut(), id).unwrap();
            assert!(value(h.state(), ChannelTarget::Alpha(0)) > 0.4);
            assert_eq!(h.state().session.active().unwrap().doc.layers, before.layers);
        }
    }
}

/// Explicit Curves points beat the legacy alias, while malformed pencil maps stay errors.
/// Canceling a valid alias dialog retains the original document and recorded Action verbatim.
#[test]
fn colour_set_action_gray_alias_precedence_and_cancel() {
    for mode in MODES {
        let raw = json!({"target":{"colors":[0,2]},"points":[[0,20],[255,240]],"gray":"ignored"});
        let mut h = colour_set_action(mode, SampleType::U8, "curves", raw.clone());
        assert!(params(&h).get("gray").is_none());
        assert_eq!(params(&h)["points"], json!([[0.0, 20.0], [255.0, 240.0]]));
        let original = h.state().session.active().unwrap().doc.clone();
        let id = h.state().ui.dialogs.last().unwrap().id;
        dialogs::cancel(h.state_mut(), id).unwrap();
        assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
        assert_eq!(h.state().session.actions.list[0].steps[0].1, raw);
        let mut h = colour_set_action(mode, SampleType::U8, "curves", json!({"target":{"colors":[0,2]},"gray":[0,127,255],"pencil":true}));
        let original = h.state().session.active().unwrap().doc.clone();
        let id = h.state().ui.dialogs.last().unwrap().id;
        assert!(dialogs::confirm(h.state_mut(), id).is_err());
        assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
    }
}

/// A resolved Lab Action stores the combined and native Lightness settings independently when recalled.
#[test]
fn colour_set_action_gray_alias_lab_recall_keeps_both_records() {
    for kind in ["levels", "curves"] {
        let raw = if kind == "levels" {
            json!({"target":{"colors":[0,2]},"gray":{"gamma":2},"lightness":{"gamma":1.543217}})
        } else {
            json!({"target":{"colors":[0,2]},"gray":[[0,255],[255,0]],"lightness":[[0,30.123456],[255,200.654321]]})
        };
        let mut h = colour_set_action(ColorMode::Lab, SampleType::U8, kind, raw);
        let expected = params(&h);
        let id = h.state().ui.dialogs.last().unwrap().id;
        dialogs::confirm(h.state_mut(), id).unwrap();
        h.state_mut().session.execute("channel.target", json!({"colors":[0,2]})).unwrap();
        adjust_dialog::open_last(h.state_mut(), &format!("image.adjustments.{kind}")).unwrap();
        h.run_steps(4);
        let recalled = params(&h);
        for key in if kind == "levels" { &["gamma", "lightness", "a", "b"][..] } else { &["points", "lightness", "a", "b"][..] } {
            assert_eq!(recalled[*key], expected[*key], "{kind}: {key}");
        }
    }
}

/// Scalar alias normalization must consume smoothing exactly once, just like colour sets.
#[test]
fn scalar_action_gray_alias_does_not_repeat_pencil_smoothing() {
    let mut app = fixture(ColorMode::Lab, SampleType::F32, ChannelTarget::Alpha(0));
    let raw = json!({"target":{"channel":0},"gray":[[0,255],[100,30],[255,0]],"pencil":true,"smooth":2});
    let mut action = photocraft_engine::actions_cmds::Action::new("Smoothed scalar", vec![("image.adjustments.curves".into(), raw.clone())]);
    action.dialogs.insert(0);
    app.session.actions.list.push(action);
    let mut h = harness(app);
    h.state_mut().run("actions.play", json!({"action":"Smoothed scalar"})).unwrap();
    h.run_steps(4);
    assert!(params(&h).get("smooth").is_none());
    let st = h.state().session.active().unwrap();
    let mut oracle = Session::new();
    oracle.add_document((*st.doc).clone(), None);
    oracle.select_layer(st.active_layer.unwrap()).unwrap();
    oracle.execute("image.adjustments.curves", raw).unwrap();
    let id = h.state().ui.dialogs.last().unwrap().id;
    dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(h.state().session.active().unwrap().doc.channels, oracle.active().unwrap().doc.channels);
}

/// Missing and malformed recorded targets must not display document-colour controls or
/// histograms. The original address remains available for errors and Cancel leaves pixels intact.
#[test]
fn review_unresolved_action_targets_do_not_fall_back_to_colour_controls() {
    for target in [json!({"channel":999}), json!({"color":0,"channel":0}), json!({"colors":[999]}), json!("quickMask")] {
        for kind in ["levels", "curves"] {
            let mut app = fixture(ColorMode::Lab, SampleType::U16, ChannelTarget::Color(0));
            let command = format!("image.adjustments.{kind}");
            let mut action = photocraft_engine::actions_cmds::Action::new("Missing target", vec![(command, json!({"target":target}))]);
            action.dialogs.insert(0);
            app.session.actions.list.push(action);
            let mut h = harness(app);
            h.state_mut().run("actions.play", json!({"action":"Missing target"})).unwrap();
            h.run_steps(4);
            assert!(names(&h).is_empty(), "invalid target must not show a colour dropdown: {target}");
            assert_eq!(params(&h)["target"], target);
            assert!(h.query_by_label("The adjustment target is missing or unavailable.").is_some());
            assert!(params(&h).get("red").is_none() && params(&h).get("lightness").is_none());
            let original = h.state().session.active().unwrap().doc.clone();
            let id = h.state().ui.dialogs.last().unwrap().id;
            assert!(dialogs::confirm(h.state_mut(), id).is_err());
            assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
        }
    }
}

/// Malformed Action step parameter shapes fail before opening an editable neutral dialog.
#[test]
fn review_malformed_action_params_cannot_open_a_neutral_adjustment() {
    for value in [json!([]), json!([1]), json!(42), json!("bad"), json!(true)] {
        let mut app = fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Alpha(0));
        let before = app.session.active().unwrap().doc.clone();
        let mut action = photocraft_engine::actions_cmds::Action::new("Bad params", vec![("image.adjustments.levels".into(), value)]);
        action.dialogs.insert(0);
        app.session.actions.list.push(action);
        let result = app.run("actions.play", json!({"action":"Bad params"})).unwrap();
        assert!(app.ui.dialogs.is_empty(), "bad params must fail the step instead of becoming defaults");
        assert!(result.get("failed").is_some() || result.get("error").is_some(), "{result}");
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }
    for value in [Value::Null, json!({})] {
        let mut app = fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Alpha(0));
        let mut action = photocraft_engine::actions_cmds::Action::new("Default params", vec![("image.adjustments.levels".into(), value)]);
        action.dialogs.insert(0);
        app.session.actions.list.push(action);
        let result = app.run("actions.play", json!({"action":"Default params"})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(app.ui.dialogs.len(), 1);
    }
}

/// Recalling a precise Action's accepted settings must retain their parsed tone values and
/// produce the same native plane as directly applying those settings a second time.
#[test]
fn review_precise_action_settings_survive_last_used_recall() {
    for mode in MODES {
        for kind in ["levels", "curves"] {
            let mut raw = if kind == "levels" {
                json!({"gray":{"gamma":1.234567,"inBlack":8.123456}})
            } else {
                json!({"gray":[[0,0],[100.123456,200.654321],[255,255]]})
            };
            raw["target"] = json!({"channel":0});
            let mut h = colour_set_action(mode, SampleType::F32, kind, raw);
            let expected = params(&h);
            let id = h.state().ui.dialogs.last().unwrap().id;
            dialogs::confirm(h.state_mut(), id).unwrap();
            let st = h.state().session.active().unwrap();
            let mut oracle = Session::new();
            oracle.add_document((*st.doc).clone(), None);
            oracle.select_layer(st.active_layer.unwrap()).unwrap();
            oracle.execute(&format!("image.adjustments.{kind}"), expected.clone()).unwrap();
            let id = adjust_dialog::open_last(h.state_mut(), &format!("image.adjustments.{kind}")).unwrap();
            h.run_steps(4);
            let parse = |p: &Value| photocraft_engine::adjust_params::from_params(kind, p, None, ColorMode::Grayscale).unwrap();
            assert_eq!(parse(&params(&h)), parse(&expected), "{mode:?} {kind}");
            dialogs::confirm(h.state_mut(), id).unwrap();
            assert_eq!(h.state().session.active().unwrap().doc.channels, oracle.active().unwrap().doc.channels);
        }
    }
}

/// Build distinct active and addressed masks, retaining the active layer and panel target.
fn addressed_mask_app(mode: ColorMode, depth: SampleType, filter: bool) -> (PhotocraftApp, photocraft_doc::LayerId) {
    let mut app = if filter { filter_mask_fixture(mode, depth, true) } else { fixture(mode, depth, ChannelTarget::Alpha(0)) };
    let mut other = app.session.active().unwrap().doc.layers[0].clone();
    other.id = Layer::new("id", other.content.clone()).id;
    other.name = "Receiver".into();
    let mut mask = LayerMask::reveal_all();
    mask.surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
    mask.surface.fill_rect(app.session.active().unwrap().doc.bounds(), &[0.75]);
    if filter {
        let LayerContent::Smart(sm) = &mut other.content else { panic!("smart") };
        sm.filter_mask = Some(mask);
    } else {
        other.mask = Some(mask);
    }
    let id = other.id;
    app.session
        .edit("Other mask", |doc, _| {
            doc.layers.push(other);
            Ok(())
        })
        .unwrap();
    (app, id)
}

/// Named/id mask Actions show, sample, preview and edit the addressed layer's own mask without
/// switching the selected layer. The matrix includes native raster and Smart Filter masks.
#[test]
fn review_mask_action_context_matches_the_addressed_layer() {
    for mode in MODES {
        for depth in DEPTHS {
            for kind in ["levels", "curves"] {
                for filter in [false, true] {
                    let (app, layer) = addressed_mask_app(mode, depth, filter);
                    for selector in [json!(layer.0), json!("Receiver"), json!({"name":"Receiver"})] {
                        let mut app = fixture_for_addressed_replay(&app);
                        let command = format!("image.adjustments.{kind}");
                        let target = if filter { "filterMask" } else { "mask" };
                        let raw = json!({"layer":selector,"target":target});
                        let mut action = photocraft_engine::actions_cmds::Action::new("Other mask", vec![(command.clone(), raw)]);
                        action.dialogs.insert(0);
                        app.session.actions.list.push(action);
                        let original = app.session.active().unwrap().doc.clone();
                        let active = app.session.active().unwrap().active_layer;
                        let expected =
                            photocraft_engine::mask_view_cmds::viewed_mask(original.layer(layer).unwrap(), filter).unwrap().surface.sample_channel(8, 8, 0);
                        let mut h = harness(app);
                        h.state_mut().run("actions.play", json!({"action":"Other mask"})).unwrap();
                        h.run_steps(4);
                        assert_eq!(names(&h), [if filter { "Receiver Filter Mask" } else { "Receiver Mask" }]);
                        let id = h.state().ui.dialogs.last().unwrap().id;
                        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("__curvePicker".into(), json!("white"));
                        assert!(adjust_dialog::sample_at(h.state_mut(), 8.0, 8.0));
                        let p = params(&h);
                        let want = photocraft_engine::adjust_params::adjustment_eyedropper(
                            kind,
                            &json!({}),
                            photocraft_engine::adjust_params::CurvesEyedropper::White,
                            [expected; 3],
                            ColorMode::Grayscale,
                        )
                        .unwrap();
                        let got = photocraft_engine::adjust_params::from_params(kind, &p, None, ColorMode::Grayscale).unwrap();
                        assert_eq!(got, want, "{mode:?} {depth:?} {kind} {target}");
                        let preview = filter_dialog::preview_document(&original, active, &command, &p, 1).unwrap();
                        dialogs::confirm(h.state_mut(), id).unwrap();
                        let doc = &h.state().session.active().unwrap().doc;
                        assert_eq!(doc.layers, preview.layers);
                        assert_eq!(doc.layers[0], original.layers[0]);
                        assert_eq!(doc.channels, original.channels);
                        assert!(
                            photocraft_engine::mask_view_cmds::viewed_mask(doc.layer(layer).unwrap(), filter).unwrap().surface.sample_channel(8, 8, 0) > 0.99
                        );
                        assert_eq!(h.state().session.active().unwrap().active_layer, active);
                    }
                }
            }
        }
    }
}

/// Copy the source document for independent Action cases without carrying modal/UI history.
fn fixture_for_addressed_replay(app: &PhotocraftApp) -> PhotocraftApp {
    let st = app.session.active().unwrap();
    let mut s = Session::new();
    s.add_document((*st.doc).clone(), None);
    s.select_layer(st.active_layer.unwrap()).unwrap();
    s.active_mut().unwrap().channel_view = st.channel_view.clone();
    let mut result = PhotocraftApp::new(s, Default::default());
    result.background_jobs = false;
    result
}

/// Repairing a missing Action address re-seeds its recorded alias without losing modal context.
#[test]
fn review_repaired_action_target_seeds_recorded_controls() {
    for (kind, raw) in [
        ("levels", json!({"target":{"channel":999},"gray":{"gamma":2}})),
        ("curves", json!({"target":{"channel":999},"gray":[[0,255],[255,0]],"lightness":[[0,0],[255,255]]})),
    ] {
        let mut h = colour_set_action(ColorMode::Lab, SampleType::U16, kind, raw.clone());
        assert!(names(&h).is_empty());
        assert_eq!(params(&h)["gray"], raw["gray"], "unresolved aliases retain their original meaning");
        let id = h.state().ui.dialogs.last().unwrap().id;
        let before = h.state().session.active().unwrap().doc.clone();
        let fields = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
        let resume = fields.get("__resumeAction").cloned();
        assert!(resume.is_some());
        fields.insert("target".into(), json!({"channel":0}));
        h.run_steps(4);
        assert_eq!(names(&h), ["Saved sky"]);
        assert!(params(&h).get("gray").is_none());
        assert_eq!(h.state().ui.dialogs.last().unwrap().fields.get("__resumeAction").cloned(), resume);
        if kind == "levels" {
            assert_eq!(params(&h)["gamma"], 2.0);
            edit_gamma(&mut h, "1.0");
            dialogs::confirm(h.state_mut(), id).unwrap();
            assert_eq!(h.state().session.active().unwrap().doc.channels, before.channels);
        } else {
            assert_eq!(params(&h)["points"], json!([[0.0, 255.0], [255.0, 0.0]]));
            let mut oracle = Session::new();
            oracle.add_document((*before).clone(), None);
            let mut repaired = raw;
            repaired["target"] = json!({"channel":0});
            oracle.execute("image.adjustments.curves", repaired).unwrap();
            dialogs::confirm(h.state_mut(), id).unwrap();
            assert_eq!(h.state().session.active().unwrap().doc.channels, oracle.active().unwrap().doc.channels);
        }
    }
}

/// Record Again resolves a recorded scalar target before seeding defaults from another panel target.
#[test]
fn record_again_uses_recorded_alpha_settings_and_matches_its_preview() {
    for mode in MODES {
        for kind in ["levels", "curves"] {
            let mut app = fixture(mode, SampleType::U16, ChannelTarget::Color(0));
            let mut recorded = photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for(kind, mode).unwrap());
            for key in ["red", "green", "blue", "cyan", "magenta", "yellow", "black", "lightness", "a", "b"] {
                if recorded.get(key).is_some() {
                    recorded[key] = if kind == "levels" { json!({"gamma":4}) } else { json!([[0, 255], [255, 255]]) };
                }
            }
            recorded["target"] = json!({"channel": 0});
            if kind == "levels" {
                recorded["gamma"] = json!(2);
            } else {
                recorded["points"] = json!([[0, 255], [255, 0]]);
            }
            app.session
                .actions
                .list
                .push(photocraft_engine::actions_cmds::Action::new("Recorded alpha", vec![(format!("image.adjustments.{kind}"), recorded)]));
            app.ui.actions.selected = Some(0);
            app.ui.actions.selected_step = Some(0);
            let original = app.session.active().unwrap().doc.clone();
            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
                |ui, app| {
                    photocraft_ui_egui::actions::panel_menu(app, ui);
                    dialogs::show(app, ui.ctx());
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, Default::default());
            h.run_steps(3);
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Record Again").click();
            h.run_steps(3);
            assert_eq!(names(&h), ["Saved sky"], "{mode:?} {kind}");
            let values = params(&h);
            assert_eq!(values["target"], json!({"channel": 0}));
            let neutral = photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for(kind, ColorMode::Grayscale).unwrap());
            for key in ["red", "green", "blue", "cyan", "magenta", "yellow", "black", "lightness", "a", "b"] {
                assert_eq!(values.get(key), neutral.get(key), "{mode:?} {kind}: hidden colour record {key}");
            }
            let id = h.state().ui.dialogs.last().unwrap().id;
            let state = h.state().session.active().unwrap();
            let preview = filter_dialog::preview_document(&state.doc, state.active_layer, &format!("image.adjustments.{kind}"), &values, 1).unwrap();
            assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
            dialogs::confirm(h.state_mut(), id).unwrap();
            let after = &h.state().session.active().unwrap().doc;
            assert_eq!(preview.channels[0].surface, after.channels[0].surface);
            let expected = if kind == "levels" { 0.5 } else { 0.75 };
            assert!((value(h.state(), ChannelTarget::Alpha(0)) - expected).abs() < 0.01, "{mode:?} {kind}");
            assert_eq!(after.layers, original.layers);
            assert_eq!(after.quick_mask, original.quick_mask);
            assert_eq!(h.state().session.actions.list[0].steps[0].1["target"], json!({"channel": 0}));
        }
    }
}
