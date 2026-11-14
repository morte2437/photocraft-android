//! #10723: Layer › New › Artboard and Layer › New › Artboard from Layers, recorded as a Make of
//! class `artboardSection` (`docs/atn.md`, "Recorded through the menus").

use super::*;
use photocraft_doc::{Layer, LayerId};
use photocraft_geom::Rect;

const NEW: &str = "layer.new.artboard";
const FROM_LAYERS: &str = "layer.new.artboardFromLayers";
const SOURCE: &str = "_atnArtboardSectionDescriptor";

/// `artboardRect` with its edges in the recorded order, built here independently of the
/// converter.
fn board_rect(cls: &str, [top, left, bottom, right]: [f64; 4]) -> D {
    D::Descriptor(
        Descriptor::new(cls).with("Top ", D::Double(top)).with("Left", D::Double(left)).with("Btom", D::Double(bottom)).with("Rght", D::Double(right)),
    )
}

/// The `null` reference: the class `artboardSection`.
fn section_class() -> D {
    D::Reference(vec![ReferenceItem::Class(class("artboardSection"))])
}

/// The measured Layer › New › Artboard step, session ids included.
fn new_board(name: &str, edges: [f64; 4]) -> Descriptor {
    Descriptor::new("null")
        .with("null", section_class())
        .with("layerSectionStart", D::Integer(7))
        .with("layerSectionEnd", D::Integer(8))
        .with("Nm  ", D::Text(UnicodeString::new_nul(name)))
        .with("artboardRect", board_rect("classFloatRect", edges))
}

/// The measured Layer › New › Artboard from Layers step, session ids included.
fn from_layers(name: &str) -> Descriptor {
    let target = D::Reference(vec![ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") }]);
    Descriptor::new("null")
        .with("null", section_class())
        .with("From", target)
        .with("layerSectionStart", D::Integer(12))
        .with("layerSectionEnd", D::Integer(13))
        .with("Nm  ", D::Text(UnicodeString::new_nul(name)))
}

/// `d` with the value of its first `key` replaced by `v`.
fn replaced(mut d: Descriptor, key: &str, v: D) -> Descriptor {
    if let Some(slot) = d.items.iter_mut().find(|(k, _)| k.is(key)) {
        slot.1 = v;
    }
    d
}

/// A Make step holding `d`.
fn make_item(d: Descriptor) -> AtnItem {
    AtnItem { event: AtnEvent::Code(*b"Mk  "), dictionary: b"Make".to_vec(), descriptor: Some(d), ..Default::default() }
}

/// Load `items` as action "A" through `actions.load`.
fn load(items: Vec<AtnItem>) -> (Session, Value) {
    let file = AtnFile {
        name: UnicodeString::new_nul("Boards"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        ..Default::default()
    };
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&atn::write(&file).unwrap())})).unwrap();
    (s, r)
}

/// Action "A" written by `actions.save` and parsed back.
fn saved(s: &mut Session) -> Vec<AtnItem> {
    let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
    assert_eq!(r["skipped"], json!([]), "{r}");
    atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap().actions.remove(0).items
}

/// The descriptor `id` with `p` saves as, if it saves.
fn exported(id: &str, p: &Value) -> Option<Descriptor> {
    export_item(id, p, "S").and_then(|item| item.descriptor)
}

/// The document's artboards as (name, rect).
fn boards(s: &Session) -> Vec<(String, Rect)> {
    s.active().unwrap().doc.artboards().into_iter().map(|(_, name, board)| (name.to_string(), board.rect)).collect()
}

/// Recorded steps load as their command with the exact rectangle and name, and save back byte for
/// byte: with session ids, in another key order, with another rectangle class, with an empty name
/// and in the canonical form.
#[test]
fn recorded_steps_load_and_save_back_unchanged() {
    let mut reordered = new_board("Wide", [-20.0, 300.0, 380.0, 940.0]);
    reordered.items.reverse();
    let mut no_ids = from_layers("Cards");
    no_ids.items.retain(|(k, _)| !k.is("layerSectionStart") && !k.is("layerSectionEnd"));
    let cases = [
        (new_board("Hero", [20.0, 10.0, 70.0, 110.0]), NEW, json!({"rect": [10, 20, 100, 50], "name": "Hero"})),
        (reordered, NEW, json!({"rect": [300, -20, 640, 400], "name": "Wide"})),
        (
            replaced(new_board("", [0.0, 0.0, 80.0, 100.0]), "artboardRect", board_rect("Rctn", [0.0, 0.0, 80.0, 100.0])),
            NEW,
            json!({"rect": [0, 0, 100, 80], "name": ""}),
        ),
        // Edges in the 32-bit range, a width past it.
        (new_board("Span", [0.0, -2e9, 10.0, 2e9]), NEW, json!({"rect": [-2_000_000_000_i64, 0, 4_000_000_000_i64, 10], "name": "Span"})),
        // The canonical form but for an edge of -0.0, which only the bytes tell apart from 0.0.
        (
            {
                let mut d = new_board("Zero", [-0.0, 10.0, 70.0, 110.0]);
                d.items.retain(|(k, _)| !k.is("layerSectionStart") && !k.is("layerSectionEnd"));
                d
            },
            NEW,
            json!({"rect": [10, 0, 100, 70], "name": "Zero"}),
        ),
        (from_layers("Cards"), FROM_LAYERS, json!({"name": "Cards"})),
        (no_ids, FROM_LAYERS, json!({"name": "Cards"})),
    ];
    for (d, id, params) in cases {
        let (mut s, r) = load(vec![make_item(d.clone())]);
        assert_eq!(r["unsupported"], json!([]), "{r}");
        let (got, p) = &s.actions.list[0].steps[0];
        let mut p = p.clone();
        p.as_object_mut().unwrap().remove(SOURCE);
        assert_eq!((got.as_str(), &p), (id, &params));
        let back = saved(&mut s);
        assert_eq!((&back[0].event, back[0].descriptor.as_ref().map(Descriptor::to_bytes)), (&AtnEvent::Code(*b"Mk  "), Some(d.to_bytes())));
    }
}

/// A loaded New Artboard step makes the recorded board, as one undoable edit.
#[test]
fn a_loaded_artboard_step_plays_and_undoes() {
    let (mut s, _) = load(vec![make_item(new_board("Hero", [20.0, 10.0, 70.0, 110.0]))]);
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert_eq!(played["ran"], 1, "{played}");
    assert_eq!(boards(&s), vec![("Hero".to_string(), Rect::new(10, 20, 110, 70))]);
    s.execute("history.undo", json!({})).unwrap();
    assert_eq!(boards(&s), vec![]);
}

/// A loaded Artboard from Layers step puts the selected layer into a board around its pixels, as
/// one undoable edit.
#[test]
fn a_loaded_artboard_from_layers_step_plays_and_undoes() {
    let (mut s, _) = load(vec![make_item(from_layers("Cards"))]);
    s.execute("file.new", json!({"width": 100, "height": 80})).unwrap();
    let square = s
        .edit("square", |doc, active| {
            let fmt = doc.pixel_format();
            let mut l = Layer::raster("Square", fmt);
            l.surface_mut().unwrap().fill_rect(Rect::new(10, 15, 30, 45), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
            let id = doc.insert_above(None, l);
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert_eq!(played["ran"], 1, "{played}");
    assert_eq!(boards(&s), vec![("Cards".to_string(), Rect::new(10, 15, 30, 45))]);
    let doc = &s.active().unwrap().doc;
    let board: LayerId = doc.artboards()[0].0;
    assert_eq!(doc.artboard_of(square), Some(board));
    s.execute("history.undo", json!({})).unwrap();
    assert_eq!(boards(&s), vec![]);
    assert_eq!(s.active().unwrap().doc.artboard_of(square), None);
}

/// A step recorded in PhotoCraft writes the measured keys without the session ids, `Nm  ` only
/// for a name of its own, and loads back to the same settings.
#[test]
fn native_steps_save_the_measured_keys() {
    let no_ids = |mut d: Descriptor| {
        d.items.retain(|(k, _)| !k.is("layerSectionStart") && !k.is("layerSectionEnd"));
        d
    };
    let mut unnamed = no_ids(new_board("", [20.0, 10.0, 70.0, 110.0]));
    unnamed.items.retain(|(k, _)| !k.is("Nm  "));
    let mut unnamed_from = no_ids(from_layers(""));
    unnamed_from.items.retain(|(k, _)| !k.is("Nm  "));
    let cases = [
        (NEW, json!({"rect": [10, 20, 100, 50], "name": "Hero"}), no_ids(new_board("Hero", [20.0, 10.0, 70.0, 110.0]))),
        (
            NEW,
            json!({"x": 10, "y": 20, "width": 100, "height": 50, "name": "Hero", "preset": "", "background": "white", "takeBackground": false}),
            no_ids(new_board("Hero", [20.0, 10.0, 70.0, 110.0])),
        ),
        (NEW, json!({"rect": [10.0, 20, 100, 50], "name": ""}), unnamed.clone()),
        (NEW, json!({"rect": [10, 20, 100, 50]}), unnamed),
        (FROM_LAYERS, json!({"name": "Cards"}), no_ids(from_layers("Cards"))),
        (FROM_LAYERS, json!({"name": "", "width": 0, "height": 0, "background": "white"}), unnamed_from.clone()),
        (FROM_LAYERS, json!({}), unnamed_from),
        (NEW, json!({"rect": [-2_000_000_000_i64, 0, 4_000_000_000_i64, 10], "name": "Span"}), no_ids(new_board("Span", [0.0, -2e9, 10.0, 2e9]))),
    ];
    for (id, p, d) in cases {
        assert_eq!(exported(id, &p).as_ref(), Some(&d), "{id} {p}");
        let (s, _) = load(vec![make_item(d)]);
        let (got, back) = &s.actions.list[0].steps[0];
        assert_eq!(got, id);
        assert!(back.get(SOURCE).is_none(), "{back}");
        assert_eq!(exported(id, back).map(|d| d.to_bytes()), exported(id, &p).map(|d| d.to_bytes()), "{back}");
    }
}

/// A native step the event can't hold is refused: a board without a position (it lands beside
/// the last one), a fractional or empty size, an edge past the 32-bit range, a preset, another
/// background or colour, the Background taken into the first board, a size of its own for
/// Artboard from Layers, a layer target, or a name that isn't text.
#[test]
fn settings_the_event_cannot_hold_are_refused() {
    for (id, p) in [
        (NEW, json!({})),
        (NEW, json!({"width": 100, "height": 50, "name": "", "preset": "", "background": "white"})),
        (NEW, json!({"x": 10, "y": 20, "width": 100})),
        (NEW, json!({"rect": [10, 20, 100]})),
        (NEW, json!({"rect": [10, 20, 100.5, 50]})),
        (NEW, json!({"rect": [10, 20, 0, 50]})),
        (NEW, json!({"rect": [10, 20, 100, -50]})),
        (NEW, json!({"rect": [2_147_483_600, 0, 100, 50]})),
        (NEW, json!({"rect": [-2_147_483_649_i64, 0, 100, 50]})),
        (NEW, json!({"rect": [0, 0, 4_294_967_296_i64, 50]})),
        (NEW, json!({"rect": [10, 20, 100, 50], "x": 5})),
        (NEW, json!({"rect": [10, 20, 100, 50], "preset": "iPhone 14"})),
        (NEW, json!({"rect": [10, 20, 100, 50], "background": "black"})),
        (NEW, json!({"rect": [10, 20, 100, 50], "background": "custom", "color": "#00ff00"})),
        (NEW, json!({"rect": [10, 20, 100, 50], "color": "#ffffff"})),
        (NEW, json!({"rect": [10, 20, 100, 50], "takeBackground": true})),
        (NEW, json!({"rect": [10, 20, 100, 50], "name": 3})),
        (FROM_LAYERS, json!({"width": 1080, "height": 0})),
        (FROM_LAYERS, json!({"height": 1350})),
        (FROM_LAYERS, json!({"background": "transparent"})),
        (FROM_LAYERS, json!({"layer": "Square"})),
        (FROM_LAYERS, json!({"rect": [10, 20, 100, 50]})),
    ] {
        assert!(exported(id, &p).is_none(), "{id} {p}");
    }
}

/// Any other form keeps the reference step: both or neither of `artboardRect` and `From`, another
/// `From` target, a repeated or extra key, values of another type, a fractional or empty
/// rectangle, or another `null` class.
#[test]
fn other_forms_keep_the_photoshop_step() {
    let mut no_rect = new_board("Hero", [20.0, 10.0, 70.0, 110.0]);
    no_rect.items.pop();
    let three_edges = Descriptor::new("classFloatRect").with("Top ", D::Double(0.0)).with("Left", D::Double(0.0)).with("Btom", D::Double(10.0));
    let board = new_board("Hero", [20.0, 10.0, 70.0, 110.0]);
    let named_layer = D::Reference(vec![ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Square") }]);
    let forms = [
        no_rect,
        board.clone().with("From", from_layers("Cards").get("From").unwrap().clone()),
        replaced(from_layers("Cards"), "From", named_layer),
        board.clone().with("Nm  ", D::Text(UnicodeString::new_nul("Again"))),
        board.clone().with("artboardBackgroundType", D::Integer(1)),
        from_layers("Cards").with("Wdth", D::Double(1080.0)),
        replaced(board.clone(), "layerSectionStart", D::Double(7.0)),
        replaced(board.clone(), "Nm  ", D::Integer(1)),
        replaced(board.clone(), "null", D::Reference(vec![ReferenceItem::Class(class("artboard"))])),
        replaced(board.clone(), "artboardRect", D::Integer(0)),
        new_board("Hero", [20.0, 10.5, 70.0, 110.0]),
        new_board("Hero", [20.0, 10.0, 20.0, 110.0]),
        new_board("Hero", [20.0, 110.0, 70.0, 10.0]),
        new_board("Hero", [20.0, 10.0, 70.0, 1e12]),
        replaced(board.clone(), "artboardRect", board_rect("classFloatRect", [20.0, 10.0, 70.0, f64::NAN])),
        replaced(board.clone(), "artboardRect", D::Descriptor(three_edges.clone())),
        replaced(board.clone(), "artboardRect", D::Descriptor(three_edges.clone().with("Rght", D::UnitFloat { unit: PX, value: 110.0 }))),
        replaced(board.clone(), "artboardRect", D::Descriptor(three_edges.clone().with("Rght", D::Double(110.0)).with("Rght", D::Double(110.0)))),
    ];
    for d in forms {
        let (mut s, r) = load(vec![make_item(d.clone())]);
        assert_eq!(r["unsupported"].as_array().map(Vec::len), Some(1), "{d:?}");
        assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&mut s)[0].descriptor.as_ref().map(Descriptor::to_bytes), Some(d.to_bytes()));
    }
}

/// A loaded step whose board was changed saves the new settings, not the stale recording; one
/// whose recording is damaged still saves its settings.
#[test]
fn changed_or_damaged_steps_save_their_settings() {
    let (s, _) = load(vec![make_item(new_board("Hero", [20.0, 10.0, 70.0, 110.0])), make_item(from_layers("Cards"))]);
    let steps = &s.actions.list[0].steps;
    let (mut board, mut cards) = (steps[0].1.clone(), steps[1].1.clone());
    assert!(board.get(SOURCE).is_some() && cards.get(SOURCE).is_some(), "{board} {cards}");
    board["rect"] = json!([0, 0, 10, 10]);
    cards["name"] = json!("");
    let mut unnamed = from_layers("");
    unnamed.items.retain(|(k, _)| k.is("null") || k.is("From"));
    let mut small = new_board("Hero", [0.0, 0.0, 10.0, 10.0]);
    small.items.retain(|(k, _)| !k.is("layerSectionStart") && !k.is("layerSectionEnd"));
    assert_eq!(exported(NEW, &board), Some(small.clone()));
    assert_eq!(exported(FROM_LAYERS, &cards), Some(unnamed));
    board[SOURCE] = json!("not base64!");
    assert_eq!(exported(NEW, &board), Some(small));
}

/// A recording too large to keep whole stays a kept step, and a stored recording past that size
/// is never decoded on save: the step's settings are written instead.
#[test]
fn oversized_recordings_are_not_kept_or_parsed() {
    let long = "N".repeat(40_000);
    let huge = new_board(&long, [20.0, 10.0, 70.0, 110.0]);
    let (mut s, r) = load(vec![make_item(huge.clone())]);
    assert_eq!(r["unsupported"].as_array().map(Vec::len), Some(1), "{r}");
    assert_eq!(saved(&mut s)[0].descriptor.as_ref().map(Descriptor::to_bytes), Some(huge.to_bytes()));
    // The same recording stored by hand: it matches the settings, but it is past the bound.
    let p = json!({"rect": [10, 20, 100, 50], "name": long, SOURCE: photocraft_paint::tile::b64_encode(&huge.to_bytes())});
    let mut canonical = huge;
    canonical.items.retain(|(k, _)| !k.is("layerSectionStart") && !k.is("layerSectionEnd"));
    assert_eq!(exported(NEW, &p), Some(canonical));
}
