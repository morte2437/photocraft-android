//! #2735: a dropdown takes one press-drag-release, like the menus (#775): press the box, drag onto
//! an option, release to choose it.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn app_with_a_layer() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        // No tool options bar Mode dropdown showing "Normal" too.
        app.ui.tool = Tool::Move;
        app
    });
    h.run_steps(6);
    h
}

fn blend(h: &Harness<'_, PhotocraftApp>) -> BlendMode {
    let st = h.state().session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().blend
}

/// The Layers panel's Blend Mode dropdown (showing `value`).
fn blend_box(h: &Harness<'_, PhotocraftApp>, value: &str) -> egui::Rect {
    let named = |n: &egui_kittest::Node<'_>| {
        let a = n.accesskit_node();
        a.value().as_deref() == Some(value) || a.label().as_deref() == Some(value)
    };
    h.query_all_by_role(Role::ComboBox).find(|n| named(n)).expect("the Layers panel's Blend Mode dropdown").rect()
}

#[test]
fn press_drag_release_picks_a_blend_mode() {
    let mut h = app_with_a_layer();
    assert_eq!(blend(&h), BlendMode::Normal);
    let at = blend_box(&h, "Normal").center();
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(3);
    let multiply = h.query_by_label("Multiply").expect("the list opened on the press").rect().center();
    for k in 1..=6 {
        h.hover_at(at + (multiply - at) * (k as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(multiply);
    h.run_steps(3);
    assert_eq!(blend(&h), BlendMode::Multiply, "releasing on Multiply picked it");
    assert!(h.query_by_label("Color Burn").is_none(), "and closed the list");
}

#[test]
fn a_click_still_opens_the_list_for_a_second_click() {
    let mut h = app_with_a_layer();
    let at = blend_box(&h, "Normal").center();
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
    assert_eq!(blend(&h), BlendMode::Normal, "the click chose nothing");
    let screen = h.query_by_label("Screen").expect("the list stays open after a click");
    screen.click();
    h.run_steps(3);
    assert_eq!(blend(&h), BlendMode::Screen, "a second click picks");
}

/// #7113: on macOS a trackpad tap-to-click delivers `Pressed` and `Released` in the same frame,
/// which raises both the modal dialog (`pointer_pressed_on_area`) and the reopened ComboBox popup
/// (`!visible_last_frame`) in the same pass. When the popup's static LayerId already sits before
/// the dialog's in `Areas::order` (from an earlier open or `ui.ron`), egui's stable sort left the
/// dropdown menu behind the dialog window.
#[test]
fn single_frame_trackpad_tap_keeps_dialog_dropdown_above_modal_and_selects_option() {
    let mut h = app_with_a_layer();
    let tap = |h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2| {
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.input_mut().events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(3);
    };

    // First dialog: open the Palette dropdown once so its static popup LayerId enters `Areas::order`,
    // then close the dialog.
    let d1 = crate::filter_dialog::open(h.state_mut(), "image.mode.indexedColor").unwrap();
    h.run_steps(3);
    let palette_box = blend_box(&h, "Selective").center();
    tap(&mut h, palette_box);
    assert!(egui::Popup::is_any_open(&h.ctx));
    crate::dialogs::cancel(h.state_mut(), d1).unwrap();
    h.run_steps(2);

    // Second dialog gets a newer `("dialog", d2)` LayerId placed after the popup's static LayerId
    // in `Areas::order` (the same ordering `ui.ron` restores across launches).
    let d2 = crate::filter_dialog::open(h.state_mut(), "image.mode.indexedColor").unwrap();
    h.run_steps(3);
    let palette_box = blend_box(&h, "Selective").center();
    let dialog_layer = h.ctx.layer_id_at(palette_box).expect("dialog layer at ComboBox");

    // 1-frame trackpad tap on the Palette dropdown: the list must open IN FRONT OF the dialog.
    tap(&mut h, palette_box);
    assert!(egui::Popup::is_any_open(&h.ctx), "tap opens the dropdown");
    let perceptual_pos = h.get_by_label("Perceptual").rect().center();
    let top_at_item = h.ctx.layer_id_at(perceptual_pos);
    assert!(
        top_at_item.is_some() && top_at_item != Some(dialog_layer),
        "#7113: dropdown popup layer ({top_at_item:?}) must be above the modal dialog ({dialog_layer:?})"
    );

    // Second 1-frame trackpad tap on "Perceptual" selects it and closes the list.
    tap(&mut h, perceptual_pos);
    assert!(!egui::Popup::is_any_open(&h.ctx), "tap on option closes the dropdown");
    assert_eq!(
        h.state().ui.dialogs.iter().find(|d| d.id == d2).and_then(|d| d.fields.get("palette")).and_then(|v| v.as_str()),
        Some("perceptual"),
        "tap on Perceptual selected it"
    );
}
