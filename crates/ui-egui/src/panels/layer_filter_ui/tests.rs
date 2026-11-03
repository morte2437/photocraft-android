//! #5048: the Layers panel's filter-type dropdown (Kind, Name, Mode, Artboard).

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_color::BlendMode;
use photocraft_doc::Document;
use serde_json::json;

use super::{Filter, FilterBy};
use crate::PhotocraftApp;
use crate::state::UiState;

/// Background, "Ink" (Multiply) and "Paper" in a group "Sheets", and "ink notes" (type) on top.
fn session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paper"})).unwrap();
    s.execute("layer.groupLayers", json!({"name": "Sheets"})).unwrap();
    let paper = layer(&s.active().unwrap().doc, "Paper");
    s.execute("layer.select", json!({"layer": paper})).unwrap();
    let ink = s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": ink, "blend": "Multiply"})).unwrap();
    let sheets = layer(&s.active().unwrap().doc, "Sheets");
    s.execute("layer.select", json!({"layer": sheets})).unwrap();
    let text = s.execute("type.create", json!({"text": "ink notes", "x": 4, "y": 20, "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": text, "name": "ink notes"})).unwrap();
    // Opened last: an open active group would take the type layer inside it, as in Photoshop.
    s.execute("layer.setExpanded", json!({"all": true, "expanded": true})).unwrap();
    s
}

fn layer(doc: &Document, name: &str) -> u64 {
    doc.walk().into_iter().find(|(_, _, l)| l.name == name).map(|(_, _, l)| l.id.0).unwrap_or_else(|| panic!("no layer {name}"))
}

/// The names `filter` lists, top to bottom.
fn listed(doc: &Document, filter: &Filter) -> Vec<String> {
    crate::layer_tree_ui::display_rows(doc, filter.active()).into_iter().filter(|(_, l)| filter.shows(l)).map(|(_, l)| l.name.clone()).collect()
}

fn filter(by: FilterBy, kinds: &[&str], name: &str, mode: BlendMode) -> Filter {
    Filter::of(&UiState {
        layer_filter_by: by,
        layer_filter: kinds.iter().map(|k| k.to_string()).collect(),
        layer_filter_name: name.into(),
        layer_filter_mode: mode,
        ..UiState::default()
    })
}

#[test]
fn each_filter_type_lists_its_matches() {
    let s = session();
    let doc = &s.active().unwrap().doc;
    let all = listed(doc, &Filter::of(&UiState::default()));
    assert_eq!(all, ["ink notes", "Sheets", "Ink", "Paper", "Background"]);
    // Kind: groups match no kind, as before.
    assert_eq!(listed(doc, &filter(FilterBy::Kind, &["pixel"], "", BlendMode::Normal)), ["Ink", "Paper", "Background"]);
    assert_eq!(listed(doc, &filter(FilterBy::Kind, &["type"], "", BlendMode::Normal)), ["ink notes"]);
    // Name: part of the name, ignoring case and surrounding spaces; groups match too.
    assert_eq!(listed(doc, &filter(FilterBy::Name, &[], " INK ", BlendMode::Normal)), ["ink notes", "Ink"]);
    assert_eq!(listed(doc, &filter(FilterBy::Name, &[], "sheet", BlendMode::Normal)), ["Sheets"]);
    assert_eq!(listed(doc, &filter(FilterBy::Name, &[], "nothing like it", BlendMode::Normal)), Vec::<String>::new());
    // Mode: one blend mode; a group's own Pass Through counts.
    assert_eq!(listed(doc, &filter(FilterBy::Mode, &[], "", BlendMode::Multiply)), ["Ink"]);
    assert_eq!(listed(doc, &filter(FilterBy::Mode, &[], "", BlendMode::PassThrough)), ["Sheets"]);
    assert_eq!(listed(doc, &filter(FilterBy::Mode, &[], "", BlendMode::Normal)), ["ink notes", "Paper", "Background"]);
}

#[test]
fn only_a_criterion_turns_kind_and_name_on_while_mode_always_filters() {
    let kinds_left_on = filter(FilterBy::Name, &["pixel"], "", BlendMode::Multiply);
    assert!(!kinds_left_on.active(), "the other types' criteria don't apply");
    assert!(!filter(FilterBy::Name, &[], "   ", BlendMode::Normal).active(), "spaces are no criterion");
    assert!(!filter(FilterBy::Kind, &[], "ink", BlendMode::Normal).active());
    assert!(filter(FilterBy::Mode, &[], "", BlendMode::Normal).active(), "Mode filters by Normal at first, like Photoshop");
}

#[test]
fn saved_settings_without_the_new_fields_still_load() {
    let mut saved = serde_json::to_value(UiState::default()).unwrap();
    let fields = saved.as_object_mut().unwrap();
    for key in ["layer_filter_by", "layer_filter_name", "layer_filter_mode"] {
        assert!(fields.remove(key).is_some(), "{key} is saved");
    }
    let ui: UiState = serde_json::from_value(saved).unwrap();
    assert_eq!((ui.layer_filter_by, ui.layer_filter_name.as_str(), ui.layer_filter_mode), (FilterBy::Kind, "", BlendMode::Normal));
    let round = serde_json::to_value(UiState { layer_filter_by: FilterBy::Mode, ..UiState::default() }).unwrap();
    assert_eq!(round["layer_filter_by"], "mode");
}

fn harness() -> Harness<'static, PhotocraftApp> {
    let session = session();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session, crate::Services::default())
    });
    h.run_steps(8);
    h
}

/// The names of the rows the Layers panel drew, top to bottom.
fn drawn(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    let doc = &h.state().session.active().unwrap().doc;
    let mut rows = crate::layer_row_ui::recorded(&h.ctx);
    rows.sort_by(|a, b| a.row.top().total_cmp(&b.row.top()));
    rows.iter().filter_map(|r| doc.walk().into_iter().find(|(_, _, l)| l.id.0 == r.layer).map(|(_, _, l)| l.name.clone())).collect()
}

#[test]
fn the_dropdown_switches_the_panel_between_filter_types() {
    let mut h = harness();
    assert_eq!(drawn(&h), ["ink notes", "Sheets", "Ink", "Paper", "Background"]);
    // The type is a real dropdown now: open it and pick Name.
    // A dropdown shows its choice as the combo box's value.
    h.get_by_value("Kind").click();
    h.run_steps(4);
    h.query_all_by_label("Name").last().expect("Name in the open list").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_by, FilterBy::Name);
    assert_eq!(drawn(&h), ["ink notes", "Sheets", "Ink", "Paper", "Background"], "no text yet: every layer");
    h.state_mut().ui.layer_filter_name = "ink".into();
    h.run_steps(4);
    assert_eq!(drawn(&h), ["ink notes", "Ink"]);
    // Mode starts at Normal; the kind toggles left on under Kind don't leak into it.
    h.state_mut().ui.layer_filter = vec!["type".into()];
    h.state_mut().ui.layer_filter_by = FilterBy::Mode;
    h.run_steps(4);
    assert_eq!(drawn(&h), ["ink notes", "Paper", "Background"]);
    h.state_mut().ui.layer_filter_mode = BlendMode::Multiply;
    h.run_steps(4);
    assert_eq!(drawn(&h), ["Ink"]);
    // Back to Kind: its own criterion applies again.
    h.state_mut().ui.layer_filter_by = FilterBy::Kind;
    h.run_steps(4);
    assert_eq!(drawn(&h), ["ink notes"]);
}

/// On a Background: the artboard "Back" holding "Grass", the artboard "Front" holding the group
/// "Trees" with "Oak", and "Loose" in no artboard, on top.
fn boards_session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    // Every layer is made before the artboards, since a new layer goes into the artboard it is on.
    for name in ["Grass", "Oak", "Loose"] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
    }
    let grass = layer(&s.active().unwrap().doc, "Grass");
    s.execute("layer.select", json!({"layer": grass})).unwrap();
    s.execute("layer.new.artboardFromLayers", json!({"name": "Back"})).unwrap();
    let oak = layer(&s.active().unwrap().doc, "Oak");
    s.execute("layer.select", json!({"layer": oak})).unwrap();
    s.execute("layer.groupLayers", json!({"name": "Trees"})).unwrap();
    s.execute("layer.new.artboardFromLayers", json!({"name": "Front"})).unwrap();
    let loose = layer(&s.active().unwrap().doc, "Loose");
    s.execute("layer.select", json!({"layer": loose})).unwrap();
    s.execute("layer.arrange.bringToFront", json!({})).unwrap();
    s.execute("layer.setExpanded", json!({"all": true, "expanded": true})).unwrap();
    s
}

/// The names the Artboard filter lists with the layers `names` selected, every group open.
fn listed_by_artboard(s: &mut photocraft_engine::Session, names: &[&str]) -> Vec<String> {
    s.execute("select.deselectLayers", json!({})).unwrap();
    for (i, name) in names.iter().enumerate() {
        let id = layer(&s.active().unwrap().doc, name);
        s.execute("layer.select", json!({"layer": id, "mode": if i == 0 { "replace" } else { "add" }})).unwrap();
    }
    let d = s.active().unwrap();
    let mut f = filter(FilterBy::Artboard, &[], "", BlendMode::Normal);
    f.board_rows = super::board_rows(&d.doc, &d.selected_layers());
    listed(&d.doc, &f)
}

#[test]
fn the_artboard_filter_lists_the_artboards_that_hold_the_selected_layers() {
    let mut s = boards_session();
    let all = ["Loose", "Front", "Trees", "Oak", "Back", "Grass", "Background"];
    assert_eq!(listed_by_artboard(&mut s, &[]), all, "nothing selected: every layer");
    // A layer at any depth in an artboard, or the artboard itself: that artboard only.
    assert_eq!(listed_by_artboard(&mut s, &["Oak"]), ["Front", "Trees", "Oak"]);
    assert_eq!(listed_by_artboard(&mut s, &["Front"]), ["Front", "Trees", "Oak"]);
    assert_eq!(listed_by_artboard(&mut s, &["Grass"]), ["Back", "Grass"]);
    // Layers in two artboards: both artboards.
    assert_eq!(listed_by_artboard(&mut s, &["Oak", "Grass"]), ["Front", "Trees", "Oak", "Back", "Grass"]);
    // A selected layer in no artboard: every layer again.
    assert_eq!(listed_by_artboard(&mut s, &["Loose"]), all);
    assert_eq!(listed_by_artboard(&mut s, &["Oak", "Loose"]), all);
    let round = serde_json::to_value(UiState { layer_filter_by: FilterBy::Artboard, ..UiState::default() }).unwrap();
    assert_eq!(round["layer_filter_by"], "artboard");
}

#[test]
fn the_artboard_filter_follows_the_selection_in_the_panel() {
    let session = boards_session();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session, crate::Services::default())
    });
    h.run_steps(8);
    let all = ["Loose", "Front", "Trees", "Oak", "Back", "Grass", "Background"];
    h.get_by_value("Kind").click();
    h.run_steps(4);
    h.query_all_by_label("Artboard").last().expect("Artboard in the open list").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_by, FilterBy::Artboard);
    assert_eq!(drawn(&h), all, "Loose, in no artboard, is selected: every layer");
    let oak = layer(&h.state().session.active().unwrap().doc, "Oak");
    h.state_mut().session.execute("layer.select", json!({"layer": oak})).unwrap();
    h.run_steps(4);
    assert_eq!(drawn(&h), ["Front", "Trees", "Oak"]);
    // The artboard's tree as the panel shows it: a closed group keeps its contents hidden.
    let trees = layer(&h.state().session.active().unwrap().doc, "Trees");
    h.state_mut().session.execute("layer.setExpanded", json!({"layer": trees, "expanded": false})).unwrap();
    h.run_steps(4);
    assert_eq!(drawn(&h), ["Front", "Trees"]);
    h.state_mut().session.execute("select.deselectLayers", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(drawn(&h), ["Loose", "Front", "Trees", "Back", "Grass", "Background"], "nothing selected: every row");
}

#[test]
fn the_artboard_filter_walks_deep_groups_without_recursion() {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Artboard, Group, Layer, LayerContent, Size};
    use photocraft_geom::Rect;
    use std::collections::HashSet;

    let mut nested = Layer::new("Leaf", LayerContent::Group(Group { children: vec![], expanded: false, artboard: None, frame_stroke: None }));
    let selected = nested.id;
    let mut expected = HashSet::from([selected]);
    for _ in 0..16_384 {
        nested = Layer::new("Group", LayerContent::Group(Group { children: vec![nested], expanded: false, artboard: None, frame_stroke: None }));
        expected.insert(nested.id);
    }
    let mut doc = Document::new("Deep groups", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let board = Layer::new(
        "Board",
        LayerContent::Group(Group { children: vec![nested], expanded: true, artboard: Some(Artboard::new(Rect::new(0, 0, 8, 8))), frame_stroke: None }),
    );
    expected.insert(board.id);
    doc.layers.push(board);
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            let actual = super::board_rows(&doc, &[selected]);
            let mut pending = std::mem::take(&mut doc.layers);
            while let Some(mut layer) = pending.pop() {
                if let Some(children) = layer.children_mut() {
                    pending.append(children);
                }
            }
            assert_eq!(actual, expected);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn the_artboard_filter_lists_the_outer_artboard_for_a_layer_in_a_nested_artboard() {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Artboard, Group, Layer, LayerContent, Size};
    use photocraft_geom::Rect;
    use std::collections::HashSet;

    fn group(name: &str, children: Vec<Layer>, artboard: Option<Artboard>) -> Layer {
        Layer::new(name, LayerContent::Group(Group { children, expanded: true, artboard, frame_stroke: None }))
    }
    let leaf = group("Leaf", vec![], None);
    let inner = group("Inner", vec![leaf.clone()], Some(Artboard::new(Rect::new(0, 0, 4, 4))));
    let sibling = group("Sibling", vec![], None);
    let outer = group("Outer", vec![inner.clone(), sibling.clone()], Some(Artboard::new(Rect::new(0, 0, 8, 8))));
    let other = group("Other", vec![], Some(Artboard::new(Rect::new(8, 0, 8, 8))));
    let mut doc = Document::new("Nested artboards", Size::new(16, 8), ColorMode::Rgb, SampleType::U8);
    let (outer_id, inner_id, leaf_id, sibling_id, other_id) = (outer.id, inner.id, leaf.id, sibling.id, other.id);
    doc.layers.push(outer);
    doc.layers.push(other);
    let outer_rows = HashSet::from([outer_id, inner_id, leaf_id, sibling_id]);
    assert_eq!(super::board_rows(&doc, &[leaf_id]), outer_rows, "a layer in a nested artboard: the outer artboard's rows");
    assert_eq!(super::board_rows(&doc, &[inner_id]), outer_rows, "the nested artboard itself: the outer artboard's rows");
    let both: HashSet<_> = outer_rows.iter().copied().chain([other_id]).collect();
    assert_eq!(super::board_rows(&doc, &[leaf_id, other_id]), both);
}
