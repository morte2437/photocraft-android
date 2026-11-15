//! #12041: committing Type-tool text in an Indexed Color or a Bitmap document.
//!
//! The reference application still lets you click and type there and shows the text while editing;
//! the commit leaves **no type layer** and a selection in the shape of the glyphs, as one history
//! state named "New Type Layer", and paints nothing. [`super::type_cmds::commits_as_selection`] is
//! that mode check and [`super::type_cmds::name_selection_commit`] the step's name; the conversion
//! is the Type Mask tools' `type.createSelection` with `removeLayer`, coalesced into the typing
//! session, which is what [`commit`] below runs and what `ui-egui`'s `commit_here` now does for the
//! Type tool in these modes.
//!
//! `type.create` itself stays mode-agnostic on purpose, and
//! [`the_type_layer_command_keeps_making_the_tools_working_layer`] pins that: the tools make the
//! layer they edit and show on canvas with that command, on the first click already with text in it
//! when Preferences > Type > "Fill new type layers with placeholder text" is on (the default), so a
//! mode branch there would take away the layer the session types into.
//!
//! The issue measures the horizontal point text you click and type, and lists paragraph text and
//! vertical type under *Not measured*, where its rule is that today's behaviour stays:
//! [`only_the_measured_horizontal_point_text_converts_in_these_modes`] pins that half of the check,
//! and [`an_uncoalesced_edit_between_typing_and_the_commit_splits_the_states`] what happens to the
//! states when an edit outside the session's coalesce group comes between the typing and the commit.

use super::type_cmds::{commits_as_selection, name_selection_commit, new_text_layer};
use super::*;
use photocraft_doc::text::{Orientation, TextShape};

/// The typing session's coalesce key, as `ui-egui`'s `session_key` gives one.
const SESSION: &str = "type-session-12041";

/// The text the tool commits, at a baseline inside every document used here. `coalesce` for the
/// tool's own calls (one step for the session), without it for a script's or the pin's.
fn typed(coalesce: bool) -> Value {
    let mut p = json!({ "text": "Hi", "x": 20, "y": 80, "size": 48 });
    if coalesce {
        p["coalesce"] = json!(SESSION);
    }
    p
}

/// A blank RGB document, to be converted to the mode under test.
fn blank() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 240, "height": 120, "background": "transparent"})).unwrap();
    s
}

fn indexed() -> Session {
    let mut s = blank();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
    assert_eq!(mode(&s), ColorMode::Indexed, "the setup made an Indexed Color document");
    s
}

fn bitmap() -> Session {
    let mut s = blank();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.execute("image.mode.bitmap", json!({"method": "threshold", "frequency": 12, "shape": "diamond"})).unwrap();
    assert_eq!(mode(&s), ColorMode::Bitmap, "the setup made a Bitmap document");
    s
}

fn mode(s: &Session) -> ColorMode {
    s.active().unwrap().doc.mode
}

/// The Type tool's commit of the session's working layer in a document [`commits_as_selection`]
/// names: the Type Mask tools' conversion, in the session's one step, which keeps the name the step
/// was opened with.
fn commit(s: &mut Session, layer: u64) {
    s.execute("type.createSelection", json!({"layer": layer, "removeLayer": true, "operation": "new", "coalesce": SESSION})).unwrap();
    name_selection_commit(s);
}

fn text_layers(s: &Session) -> usize {
    s.active().unwrap().doc.walk().into_iter().filter(|(_, _, l)| matches!(l.content, LayerContent::Text(_))).count()
}

fn layers(s: &Session) -> usize {
    s.active().unwrap().doc.walk().len()
}

/// The selection's coverage over the whole canvas, in document pixels.
fn coverage(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap().doc.clone();
    let b = d.bounds();
    let mut out = Vec::new();
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            out.push(d.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0)));
        }
    }
    out
}

/// The selection's bounds, as its corners.
fn selection_corners(s: &Session) -> Option<[i32; 4]> {
    s.active().unwrap().doc.selection.as_ref().map(|m| m.content_bounds()).map(|r| [r.x0, r.y0, r.x1, r.y1])
}

/// Every layer's pixels, quantised: what the commit must leave exactly as it was.
fn pixels(s: &Session) -> Vec<u8> {
    let d = s.active().unwrap().doc.clone();
    let b = d.bounds();
    let mut out = Vec::new();
    for (_, _, l) in d.walk() {
        let Some(surface) = l.surface() else { continue };
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                out.extend(surface.rgba(x, y).map(|v| (v * 255.0).round() as u8));
            }
        }
    }
    out
}

fn history(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

#[test]
fn committing_in_indexed_color_or_bitmap_leaves_the_glyphs_selected_and_no_layer() {
    for (name, make) in [("Indexed Color", indexed as fn() -> Session), ("Bitmap", bitmap as fn() -> Session)] {
        let mut s = make();
        let (steps, layers_before, pixels_before) = (history(&s).len(), layers(&s), pixels(&s));

        // The tool's session: a type layer to edit and show while typing, as measured.
        let layer = s.execute("type.create", typed(true)).unwrap()["layer"].as_u64().unwrap();
        assert_eq!(text_layers(&s), 1, "{name}: typing happens on a layer the tool can show");

        commit(&mut s, layer);

        assert_eq!(text_layers(&s), 0, "{name}: committing leaves no type layer");
        assert_eq!(layers(&s), layers_before, "{name}: the working layer went with it");
        assert!(s.active().unwrap().doc.selection.is_some(), "{name}: the glyphs are selected");
        let selected = s.active().unwrap().doc.selection.as_ref().unwrap();
        assert!(!selected.content_bounds().is_empty(), "{name}: the selection has a shape");
        assert!(coverage(&s).iter().any(|v| *v > 0.0), "{name}: the selected pixels are the glyphs'");
        assert_eq!(pixels(&s), pixels_before, "{name}: the commit paints nothing");

        // One step for the whole session, named as Photoshop names it.
        let h = history(&s);
        assert_eq!(h.len(), steps + 1, "{name}: the session is one history state: {h:?}");
        assert_eq!(h.last().map(String::as_str), Some("New Type Layer"), "{name}: its name");

        // And undoing that one state gives the document back as it was.
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none(), "{name}: undo takes the selection");
        assert_eq!(layers(&s), layers_before, "{name}: undo takes the whole session");
        assert_eq!(pixels(&s), pixels_before, "{name}: nothing else was changed");
    }
}

#[test]
fn the_selection_is_the_glyph_shape_the_type_mask_of_the_same_text_makes() {
    for (name, make) in [("Indexed Color", indexed as fn() -> Session), ("Bitmap", bitmap as fn() -> Session)] {
        let mut s = make();
        let layer = s.execute("type.create", typed(true)).unwrap()["layer"].as_u64().unwrap();
        commit(&mut s, layer);
        let (got, got_bounds) = (coverage(&s), selection_corners(&s));

        // The pin: the same text straight through the Type Mask command.
        let mut pin = make();
        pin.execute("type.createSelection", typed(false)).unwrap();
        assert_eq!(got_bounds, selection_corners(&pin), "{name}: same selection bounds");
        assert_eq!(got, coverage(&pin), "{name}: same selection coverage");
    }
}

#[test]
fn rgb_grayscale_cmyk_and_lab_keep_making_type_layers_at_every_depth() {
    for (id, want) in [("rgb", ColorMode::Rgb), ("gray", ColorMode::Grayscale), ("cmyk", ColorMode::Cmyk), ("lab", ColorMode::Lab)] {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 240, "height": 120, "mode": id, "depth": depth, "background": "transparent"})).unwrap();
            assert_eq!(mode(&s), want, "{id} at {depth} bit: the document really is that mode");
            let point = new_text_layer(&s, &typed(false), "type.create").unwrap();
            assert!(!commits_as_selection(want, &point), "{id} keeps type layers");

            let layer = s.execute("type.create", typed(true)).unwrap()["layer"].as_u64().unwrap();
            // Today's commit: rename the session's layer in the same step, select nothing.
            let name = super::type_cmds::layer_name("Hi");
            s.execute("type.edit", json!({"layer": layer, "name": name, "coalesce": SESSION})).unwrap();
            assert_eq!(text_layers(&s), 1, "{id} at {depth} bit keeps its type layer");
            assert!(s.active().unwrap().doc.selection.is_none(), "{id} at {depth} bit selects nothing");
            let h = history(&s);
            assert_eq!(h.len(), 2, "{id} at {depth} bit: open plus one step for the session: {h:?}");
            assert_eq!(h.last().map(String::as_str), Some("New Type Layer"), "{id} at {depth} bit keeps the step's name");
        }
    }
}

#[test]
fn the_type_layer_command_keeps_making_the_tools_working_layer_in_these_modes() {
    // `type.create` is the primitive the Type tools create the layer they edit with (and scripts and
    // actions their text layers with), so the colour-mode branch is the commit's, not its own.
    for (name, make) in [("Indexed Color", indexed as fn() -> Session), ("Bitmap", bitmap as fn() -> Session)] {
        let mut s = make();
        let id = s.execute("type.create", typed(false)).unwrap()["layer"].as_u64().unwrap();
        assert!(matches!(s.active().unwrap().doc.layer(LayerId(id)).unwrap().content, LayerContent::Text(_)), "{name}: the command still makes the layer");
        assert!(s.active().unwrap().doc.selection.is_none(), "{name}: and selects nothing itself");
        assert_eq!(history(&s).last().map(String::as_str), Some("New Type Layer"), "{name}: its own step name");
    }
}

#[test]
fn a_refused_conversion_leaves_the_typed_text_and_the_document_alone() {
    // The commit's fallback in `ui-egui` keeps today's type layer when the conversion is refused,
    // so the typed text is not lost with a selection that was never made. Here the document would
    // have to keep no layer at all, which is what `type.createSelection` with `removeLayer` refuses.
    for (name, make) in [("Indexed Color", indexed as fn() -> Session), ("Bitmap", bitmap as fn() -> Session)] {
        let mut s = make();
        let only = s.active().unwrap().doc.layers[0].id.0;
        let layer = s.execute("type.create", typed(true)).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.delete", json!({"layer": only})).unwrap();
        let (pixels_before, steps, label) = (pixels(&s), history(&s).len(), history(&s).last().cloned());

        assert!(
            s.execute("type.createSelection", json!({"layer": layer, "removeLayer": true, "operation": "new", "coalesce": SESSION})).is_err(),
            "{name}: the conversion is refused"
        );
        // `commit_here` stops there and keeps the layer, so the step is never renamed either.
        assert_eq!(text_layers(&s), 1, "{name}: the typed text stays as a type layer");
        assert!(s.active().unwrap().doc.selection.is_none(), "{name}: nothing was selected");
        assert_eq!(pixels(&s), pixels_before, "{name}: the refused step painted nothing");
        assert_eq!(history(&s).len(), steps, "{name}: the refused step is no new state");
        assert_eq!(history(&s).last().cloned(), label, "{name}: the refused conversion renames nothing");
    }
}

/// The measurement covers horizontal point text. Paragraph (box) text and vertical type are on the
/// issue's *Not measured* list, where its rule is that today's behaviour stays - and a coverage mask
/// could not give a reflowing box or a stacked column back anyway, so their editable layer stays.
#[test]
fn only_the_measured_horizontal_point_text_converts_in_these_modes() {
    let s = indexed();
    let point = new_text_layer(&s, &typed(false), "type.create").unwrap();
    assert!(matches!(point.shape, TextShape::Point) && point.orientation == Orientation::Horizontal, "the fixture is horizontal point text");

    let mut box_params = typed(false);
    box_params["box"] = json!([20.0, 20.0, 180.0, 80.0]);
    let paragraph = new_text_layer(&s, &box_params, "type.create").unwrap();
    assert!(matches!(paragraph.shape, TextShape::Box { .. }), "the fixture is paragraph text");

    let mut vertical_params = typed(false);
    vertical_params["orientation"] = json!("vertical");
    let vertical = new_text_layer(&s, &vertical_params, "type.create").unwrap();
    assert_eq!(vertical.orientation, Orientation::Vertical, "the fixture is vertical text");

    for mode in [ColorMode::Indexed, ColorMode::Bitmap] {
        assert!(commits_as_selection(mode, &point), "{mode:?}: the measured text becomes the selection");
        assert!(!commits_as_selection(mode, &paragraph), "{mode:?}: paragraph text keeps its type layer");
        assert!(!commits_as_selection(mode, &vertical), "{mode:?}: vertical text keeps its type layer");
    }
}

/// An edit that does not carry the session's `coalesce` key ends its coalesce group - in the tool
/// that is the deliberately per-press Alt+arrow kerning step. The commit then makes a state of its
/// own, which is what the Type Mask tools' commit and the plain layer's rename do in the same
/// situation today; nothing is mis-named and nothing else moves.
#[test]
fn an_uncoalesced_edit_between_typing_and_the_commit_splits_the_states() {
    for (name, make) in [("Indexed Color", indexed as fn() -> Session), ("Bitmap", bitmap as fn() -> Session)] {
        let mut s = make();
        let layer = s.execute("type.create", typed(true)).unwrap()["layer"].as_u64().unwrap();
        // The stand-in for that kerning press: a real edit under no coalesce key.
        s.execute("type.edit", json!({"layer": layer, "antialias": "sharp"})).unwrap();
        let before = history(&s);
        assert_eq!(before.last().map(String::as_str), Some("Edit Type"), "{name}: the unkeyed edit is a state of its own");

        commit(&mut s, layer);

        let h = history(&s);
        assert_eq!(h.len(), before.len() + 1, "{name}: the commit is a further state once the group ended: {h:?}");
        assert_eq!(h.last().map(String::as_str), Some("New Type Layer"), "{name}: named for the type layer either way");
        assert_eq!(h[h.len() - 2].as_str(), "Edit Type", "{name}: the edit between them keeps its own name");
        assert_eq!(text_layers(&s), 0, "{name}: still no type layer is left");
        assert!(s.active().unwrap().doc.selection.is_some(), "{name}: still the glyphs are selected");

        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none(), "{name}: one undo takes the conversion");
        assert_eq!(text_layers(&s), 1, "{name}: which brings the working layer back, as the mask tools' undo does");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(text_layers(&s), 1, "{name}: the next undo takes the unkeyed edit, not the session");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(text_layers(&s), 0, "{name}: and the third one takes the typing session");
    }
}
