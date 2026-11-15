//! #11102: Move Auto-Select, Layer, measured selected-ID sets (author comment 6616635118).
//! Photoshop's scripting activeLayer can stay nonempty when its target-layer list is empty.

use egui::Modifiers;
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};

struct Fixture {
    app: PhotocraftApp,
    background: LayerId,
    layers: [LayerId; 3],
}

impl Fixture {
    fn new() -> Self {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
        app.sync_views();
        let background = app.session.active().unwrap().doc.background().unwrap().id;
        let layers = [("A", 8), ("B", 32), ("C", 56)].map(|(name, x)| {
            let id = LayerId(app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap());
            app.run("select.rect", json!({"x": x, "y": 8, "width": 12, "height": 12})).unwrap();
            app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
            id
        });
        app.run("select.deselect", json!({})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_auto_select = true;
        app.ui.tool_options.move_target = "layer".into();
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        Self { app, background, layers }
    }

    fn select(&mut self, ids: &[LayerId]) {
        self.app.run("select.deselectLayers", json!({})).unwrap();
        for id in ids {
            self.app.run("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
        }
    }

    fn assert_selected(&self, expected: &[LayerId]) {
        let mut actual = self.app.session.active().unwrap().selected_layers();
        let mut expected = expected.to_vec();
        actual.sort_unstable_by_key(|id| id.0);
        expected.sort_unstable_by_key(|id| id.0);
        assert_eq!(actual, expected);
    }

    fn bounds(&self) -> Vec<Rect> {
        let doc = &self.app.session.active().unwrap().doc;
        std::iter::once(self.background).chain(self.layers).map(|id| doc.layer(id).unwrap().surface().unwrap().content_bounds()).collect()
    }

    /// Send a click or drag through the tool event path. Equal endpoints send no move event.
    /// The release must end any marquee without raising a move-lock error.
    fn gesture(&mut self, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
        tool_event(&mut self.app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
        if from != to {
            tool_event(&mut self.app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
        }
        tool_event(&mut self.app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
        assert!(self.app.layer_box.is_none(), "release ends the marquee");
        assert!(self.app.ui.dialogs.iter().all(|d| d.kind != DialogKind::Error), "no move-lock dialog");
        assert!(!self.app.ui.status_error, "{}", self.app.ui.status);
    }

    fn convert_background(&mut self) {
        self.select(&[self.background]);
        self.app.run("layer.new.layerFromBackground", json!({})).unwrap();
        assert!(!self.app.session.active().unwrap().doc.is_background(self.background));
    }
}

#[test]
fn point_clicks_toggle_members_but_keep_the_singleton_and_plain_multi_selection() {
    let mut f = Fixture::new();
    let [a, b, c] = f.layers;
    let before = f.bounds();
    f.select(&[b]);
    f.gesture([14.0, 14.0], [14.0, 14.0], Modifiers::NONE);
    f.assert_selected(&[a]);
    f.gesture([38.0, 14.0], [38.0, 14.0], Modifiers::SHIFT);
    f.assert_selected(&[a, b]);
    f.gesture([62.0, 14.0], [62.0, 14.0], Modifiers::SHIFT);
    f.assert_selected(&[a, b, c]);
    f.gesture([38.0, 14.0], [38.0, 14.0], Modifiers::SHIFT);
    f.assert_selected(&[a, c]);

    f.select(&[a]);
    f.gesture([14.0, 14.0], [14.0, 14.0], Modifiers::SHIFT);
    f.assert_selected(&[a]);
    f.select(&[a, b, c]);
    f.gesture([14.0, 14.0], [14.0, 14.0], Modifiers::NONE);
    f.assert_selected(&[a, b, c]);

    f.select(&[b]);
    f.gesture([14.0, 14.0], [14.0, 14.0], Modifiers::COMMAND);
    f.assert_selected(&[b]);
    assert_eq!(f.bounds(), before, "selection-only gestures move no pixels");
}

/// Shift-press changes the selected set immediately, and only that set moves on drag (#11102).
#[test]
fn shift_drag_moves_the_layers_selected_after_the_press() {
    for (x, moving) in [(14.0, &[1_usize][..]), (38.0, &[0][..]), (62.0, &[0, 1, 2][..])] {
        let mut f = Fixture::new();
        let [a, b, _] = f.layers;
        f.select(&[a, b]);
        let selected: Vec<_> = moving.iter().map(|&index| f.layers[index]).collect();
        let before = f.bounds();
        let shift = Modifiers::SHIFT;

        tool_event(&mut f.app, ToolEvent::Down { x, y: 14.0, pressure: 1.0 }, shift);
        f.assert_selected(&selected);
        assert_eq!(f.bounds(), before, "the press only selects");
        let x = x + 6.0;
        tool_event(&mut f.app, ToolEvent::Move { x, y: 14.0, pressure: 1.0 }, shift);
        tool_event(&mut f.app, ToolEvent::Up { x, y: 14.0 }, shift);

        f.assert_selected(&selected);
        let mut expected = before;
        for &index in moving {
            expected[index + 1] = expected[index + 1].translate(6, 0);
        }
        assert_eq!(f.bounds(), expected, "only the layers selected after the press moved");
    }
}

/// A plain click on locked-only pixels clears the selection. Shift keeps the base for an
/// additive marquee, and neither gesture moves the locked content.
#[test]
fn locked_only_click_clears_each_measured_selection_and_shift_keeps_its_base() {
    let mut f = Fixture::new();
    let [a, b, c] = f.layers;
    let before = f.bounds();
    let ordinary = f.app.run("layer.pickAt", json!({"x": 74, "y": 40, "select": false})).unwrap();
    assert_eq!(ordinary["layer"], f.background.0, "ordinary point picks still find locked Background");
    for selected in [vec![a], vec![a, b, c], vec![f.background]] {
        f.select(&selected);
        f.gesture([74.0, 40.0], [74.0, 40.0], Modifiers::NONE);
        f.assert_selected(&[]);
    }
    f.select(&[a, b]);
    f.gesture([74.0, 40.0], [74.0, 40.0], Modifiers::SHIFT);
    f.assert_selected(&[a, b]);
    assert_eq!(f.bounds(), before, "the Background never moves");

    // B's pixels are above the still-locked Background, with no other movable layer there.
    f.app.run("layer.setProps", json!({"layer": b.0, "locks": {"all": true}})).unwrap();
    f.select(&[a]);
    f.gesture([38.0, 14.0], [38.0, 14.0], Modifiers::NONE);
    f.assert_selected(&[]);
    assert_eq!(f.bounds(), before, "the all-locked layer and Background stay put");
}

/// Once Background becomes a normal layer, Position and All locks prevent Auto-Select.
/// An Image Pixels lock still permits selection, and the unlocked layer can be dragged.
#[test]
fn converted_background_point_pick_obeys_position_and_all_but_not_image_pixels_lock() {
    for (locks, selectable) in [(json!({}), true), (json!({"all": true}), false), (json!({"position": true}), false), (json!({"pixels": true}), true)] {
        let mut f = Fixture::new();
        f.convert_background();
        f.app.run("layer.setProps", json!({"layer": f.background.0, "locks": locks})).unwrap();
        f.select(&[f.layers[0]]);
        let before = f.bounds();
        f.gesture([74.0, 40.0], [74.0, 40.0], Modifiers::NONE);
        f.assert_selected(if selectable { std::slice::from_ref(&f.background) } else { &[] });
        assert_eq!(f.bounds(), before);
    }

    let mut f = Fixture::new();
    f.convert_background();
    f.select(&[f.layers[0]]);
    f.gesture([74.0, 40.0], [77.0, 42.0], Modifiers::NONE);
    f.assert_selected(&[f.background]);
    assert_eq!(f.bounds()[0], Rect::new(3, 2, 83, 62), "an unlocked converted Background can move");
}

/// Move Auto-Select picks the movable layer under a Position- or All-locked hit. The context
/// menu still lists the locked hit first so the user can choose it explicitly.
#[test]
fn locked_top_layer_is_clicked_through_while_the_context_menu_still_lists_it() {
    for locks in [json!({"position": true}), json!({"all": true})] {
        let mut f = Fixture::new();
        f.convert_background();
        let a = f.layers[0];
        f.app.run("layer.setProps", json!({"layer": a.0, "locks": locks})).unwrap();
        f.select(&[f.layers[1]]);
        let before = f.bounds();
        let list = f.app.run("layer.pickAt", json!({"x": 14, "y": 14, "list": true})).unwrap();
        assert_eq!(list["layers"][0]["layer"], a.0);
        f.gesture([14.0, 14.0], [14.0, 14.0], Modifiers::NONE);
        f.assert_selected(&[f.background]);
        assert_eq!(f.bounds(), before);
    }
}

/// A Shift marquee adds either the touched child or its enclosing group.
#[test]
fn grouped_marquee_respects_explicit_layer_and_group_targets() {
    for target in ["layer", "group"] {
        let mut f = Fixture::new();
        let [a, _, c] = f.layers;
        f.select(&[a]);
        let group = LayerId(f.app.run("layer.new.groupFromLayers", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap());
        f.app.ui.tool_options.move_target = target.into();
        f.select(&[c]);
        let before = f.bounds();
        f.gesture([4.0, 4.0], [22.0, 24.0], Modifiers::SHIFT);
        let hit = if target == "layer" { a } else { group };
        f.assert_selected(&[c, hit]);
        assert_eq!(f.bounds(), before);
    }
}

/// A marquee starting on Background adds hits with Shift and replaces the set without it.
/// Empty boxes preserve the additive base and clear a replacement, with no pixel movement.
#[test]
fn background_marquee_adds_with_shift_and_replaces_without_it_including_empty_hits() {
    let mut f = Fixture::new();
    let [a, b, c] = f.layers;
    let before = f.bounds();
    for (initial, from, to, mods, expected) in [
        (vec![a], [28.0, 4.0], [70.0, 24.0], Modifiers::SHIFT, vec![a, b, c]),
        (vec![a, b], [28.0, 4.0], [70.0, 24.0], Modifiers::SHIFT, vec![a, b, c]),
        (vec![a], [4.0, 4.0], [22.0, 24.0], Modifiers::SHIFT, vec![a]),
        (vec![a], [4.0, 32.0], [70.0, 48.0], Modifiers::SHIFT, vec![a]),
        (vec![a], [28.0, 4.0], [70.0, 24.0], Modifiers::NONE, vec![b, c]),
        (vec![a], [4.0, 32.0], [70.0, 48.0], Modifiers::NONE, vec![]),
        (vec![], [-4.0, -4.0], [70.0, 24.0], Modifiers::NONE, vec![a, b, c]),
    ] {
        f.select(&initial);
        f.gesture(from, to, mods);
        f.assert_selected(&expected);
        assert_eq!(f.bounds(), before, "marquees select without moving pixels");
    }
}

/// A Background marquee crosses A and B while C starts selected. Only movable hits join the
/// selection with Shift, or replace it without Shift (#11102).
#[test]
fn background_marquee_skips_position_and_all_locked_layers() {
    for (locks, mods, expected) in [
        (json!({"position": true}), Modifiers::SHIFT, &[1_usize, 2][..]),
        (json!({"position": true}), Modifiers::NONE, &[1][..]),
        (json!({"all": true}), Modifiers::SHIFT, &[1, 2][..]),
        (json!({}), Modifiers::SHIFT, &[0, 1, 2][..]),
    ] {
        let mut f = Fixture::new();
        let [a, _, c] = f.layers;
        f.app.run("layer.setProps", json!({"layer": a.0, "locks": locks})).unwrap();
        f.select(&[c]);
        let before = f.bounds();
        f.gesture([4.0, 26.0], [49.0, 13.0], mods);
        f.assert_selected(&expected.iter().map(|&index| f.layers[index]).collect::<Vec<_>>());
        assert_eq!(f.bounds(), before, "marquees select without moving pixels");
    }
}

/// A nonzero marquee wholly on the pasteboard keeps the layer targets when it hits nothing.
#[test]
fn pasteboard_only_empty_marquee_preserves_selection() {
    let mut f = Fixture::new();
    let [a, b, _] = f.layers;
    f.select(&[a, b]);
    let before = f.bounds();
    let active = f.app.session.active().unwrap().active_layer;
    f.gesture([-24.0, 8.0], [-8.0, 26.0], Modifiers::NONE);
    f.assert_selected(&[a, b]);
    assert_eq!(f.app.session.active().unwrap().active_layer, active);
    assert_eq!(f.bounds(), before, "an empty pasteboard marquee moves no pixels");
}

/// An empty transparent start retains the prior no-hit behavior.
#[test]
fn transparent_canvas_empty_marquee_preserves_selection() {
    let mut f = Fixture::new();
    f.convert_background();
    f.app.run("layer.setProps", json!({"layer": f.background.0, "visible": false})).unwrap();
    let a = f.layers[0];
    f.select(&[a]);
    let before = f.bounds();
    f.gesture([28.0, 32.0], [68.0, 48.0], Modifiers::NONE);
    f.assert_selected(&[a]);
    assert_eq!(f.bounds(), before);
}
