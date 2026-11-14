//! #126: group disclosure triangles in the real Layers panel (whole app, real pointer input).

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Document, LayerContent, LayerId};
use serde_json::json;

use super::display_rows;
use crate::PhotocraftApp;

/// Background, then `outer { in-outer, inner { deep } }` with a pixel layer above it all.
fn layered() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "deep"})).unwrap();
    let inner = s.execute("layer.groupLayers", json!({"name": "inner"})).unwrap()["layer"].as_u64().unwrap();
    let outer = s.execute("layer.groupLayers", json!({"layer": inner, "name": "outer"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": inner})).unwrap();
    // `layer.new.layer` lands above the active (closed) group `inner`, inside `outer`.
    s.execute("layer.new.layer", json!({"name": "in-outer"})).unwrap();
    s.execute("layer.select", json!({"layer": outer})).unwrap();
    s.execute("layer.new.layer", json!({"name": "top"})).unwrap();
    // Group Layers creates collapsed groups (#5571); these tests start with every group open.
    s.execute("layer.setExpanded", json!({"all": true, "expanded": true})).unwrap();
    s
}

fn harness_with(session: photocraft_engine::Session, ppp: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_pixels_per_point(ppp).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session, crate::Services::default())
    });
    h.run_steps(8);
    h
}

fn group(h: &Harness<'_, PhotocraftApp>, name: &str) -> (LayerId, bool) {
    let doc = &h.state().session.active().unwrap().doc;
    doc.walk()
        .into_iter()
        .find_map(|(_, _, l)| match &l.content {
            LayerContent::Group(g) if l.name == name => Some((l.id, g.expanded)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no group {name}"))
}

/// The on-screen centre of a group's triangle, scrolled into view in the Layers panel first (the
/// dock gives Layers a fixed height, so rows can sit below its fold). `None` when the row isn't
/// listed (inside a closed group).
fn triangle(h: &mut Harness<'_, PhotocraftApp>, name: &str) -> Option<Pos2> {
    let verb = if group(h, name).1 { "Collapse" } else { "Expand" };
    let label = format!("{verb} group {name}");
    h.query_by_label(&label)?.scroll_to_me();
    h.run_steps(4);
    h.query_by_label(&label).map(|n| n.rect().center())
}

/// A real click (press and release on separate frames), modifiers held throughout.
fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(1);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

#[test]
fn rows_list_groups_above_their_contents_and_hide_closed_ones() {
    let s = layered();
    let doc: &Document = &s.active().unwrap().doc;
    let names = |doc: &Document, all| display_rows(doc, all).iter().map(|(d, l)| format!("{d}:{}", l.name)).collect::<Vec<_>>();
    assert_eq!(names(doc, false), ["0:top", "0:outer", "1:in-outer", "1:inner", "2:deep", "0:Background"]);
    let mut closed = doc.clone();
    let id = closed.walk().into_iter().find(|(_, _, l)| l.name == "inner").unwrap().2.id;
    if let Some(LayerContent::Group(g)) = closed.layer_mut(id).map(|l| &mut l.content) {
        g.expanded = false;
    }
    assert_eq!(names(&closed, false), ["0:top", "0:outer", "1:in-outer", "1:inner", "0:Background"]);
    assert_eq!(names(&closed, true).len(), 6, "a kind filter still finds layers in closed groups");
}

#[test]
fn clicking_a_triangle_toggles_its_group_at_every_ui_scale() {
    for ppp in [1.0, 1.5, 2.0] {
        let mut h = harness_with(layered(), ppp);
        let active = h.state().session.active().unwrap().active_layer;
        let undo = h.state().session.active().unwrap().history.entries().len();
        let p = triangle(&mut h, "inner").expect("inner's triangle shows");
        click(&mut h, p, Modifiers::NONE);
        assert!(!group(&h, "inner").1, "@{ppp}x: click collapses");
        assert!(group(&h, "outer").1, "@{ppp}x: only that group");
        // The click opened/closed the group; it didn't select it or start a drag-reorder.
        assert_eq!(h.state().session.active().unwrap().active_layer, active, "@{ppp}x: selection unchanged");
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), undo, "@{ppp}x: not an undo step");
        let p = triangle(&mut h, "inner").expect("still shows, now pointing right");
        click(&mut h, p, Modifiers::NONE);
        assert!(group(&h, "inner").1, "@{ppp}x: click expands again");
        // Nested: closing the outer group hides the inner group's row (and so its triangle).
        let p = triangle(&mut h, "outer").unwrap();
        click(&mut h, p, Modifiers::NONE);
        assert!(!group(&h, "outer").1);
        assert!(triangle(&mut h, "inner").is_none(), "@{ppp}x: inner row hidden in a closed group");
    }
}

#[test]
fn alt_click_toggles_every_group() {
    let mut h = harness_with(layered(), 1.0);
    let p = triangle(&mut h, "outer").unwrap();
    click(&mut h, p, Modifiers::ALT);
    assert!(!group(&h, "outer").1 && !group(&h, "inner").1, "⌥-click closes all groups");
    let p = triangle(&mut h, "outer").unwrap();
    click(&mut h, p, Modifiers::ALT);
    assert!(group(&h, "outer").1 && group(&h, "inner").1, "⌥-click opens all groups");
    assert!(triangle(&mut h, "inner").is_some());
}

#[test]
fn state_survives_psd_export_and_import_and_imported_groups_toggle() {
    let mut h = harness_with(layered(), 2.0);
    let p = triangle(&mut h, "inner").unwrap();
    click(&mut h, p, Modifiers::NONE);
    let doc = (*h.state().session.active().unwrap().doc).clone();
    let bytes = photocraft_io::export(&doc, "groups.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("groups.psd", &bytes).unwrap().document;
    let mut s = photocraft_engine::Session::new();
    s.add_document(back, None);
    let mut h = harness_with(s, 2.0);
    assert!(group(&h, "outer").1 && !group(&h, "inner").1, "open/closed state round-trips through PSD");
    // A PSD-imported group toggles like any other.
    let p = triangle(&mut h, "inner").expect("imported group has a triangle");
    click(&mut h, p, Modifiers::NONE);
    assert!(group(&h, "inner").1);
    let p = triangle(&mut h, "outer").unwrap();
    click(&mut h, p, Modifiers::NONE);
    assert!(!group(&h, "outer").1);
}

/// Background, a collapsed group "Low Group" at the bottom of the list, 40 fillers above it (so it
/// sits below the Layers panel's fold) and a collapsed "High Group" on top (#13117).
fn long_list_with_groups() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "low child"})).unwrap();
    s.execute("layer.groupLayers", json!({"name": "Low Group"})).unwrap();
    for i in 0..40 {
        s.execute("layer.new.layer", json!({"name": format!("Filler {i}")})).unwrap();
    }
    s.execute("layer.new.layer", json!({"name": "high child"})).unwrap();
    s.execute("layer.groupLayers", json!({"name": "High Group"})).unwrap();
    s
}

/// Every text painted in the harness's last frame, with its screen position.
fn painted_texts(h: &Harness<'_, PhotocraftApp>) -> Vec<(String, Pos2)> {
    fn walk(s: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos)),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.output().shapes.iter().for_each(|c| walk(&c.shape, &mut out));
    out
}

/// #13117: a group row scrolled out of view paints nothing, like any other off-screen row; only
/// its disclosure widget stays (the next test).
#[test]
fn off_screen_group_paints_no_row() {
    let h = harness_with(long_list_with_groups(), 1.0);
    let window = h.ctx.content_rect();
    let low = h.get_by_label("Low Group").rect();
    let high = h.get_by_label("High Group").rect();
    assert!(low.top() >= window.bottom(), "Low Group sits below the fold: {low:?} vs {window:?}");
    assert!(window.contains_rect(high), "High Group shows: {high:?}");
    let named_in = |name: &str, row: egui::Rect| painted_texts(&h).iter().any(|(t, p)| t == name && p.y >= row.top() && p.y <= row.bottom());
    assert!(named_in("High Group", high), "the on-screen group's name is painted");
    assert!(!named_in("Low Group", low), "the off-screen group's row paints no name");
    // Nor any other text: no text shape lands in the off-screen row at all.
    let in_low: Vec<_> = painted_texts(&h).into_iter().filter(|(_, p)| low.contains(*p)).collect();
    assert!(in_low.is_empty(), "the off-screen group's row adds no text shapes: {in_low:?}");
}

/// #13117: skipping the off-screen group's painting keeps its triangle a widget: it is listed for
/// accessibility, and scrolled to, it still toggles the group.
#[test]
fn off_screen_group_disclosure_stays_reachable() {
    let mut h = harness_with(long_list_with_groups(), 1.0);
    assert!(h.query_by_label("Expand group Low Group").is_some(), "the off-screen group keeps its disclosure widget");
    assert!(!group(&h, "Low Group").1);
    let p = triangle(&mut h, "Low Group").expect("scrolled to the off-screen group's triangle");
    click(&mut h, p, Modifiers::NONE);
    assert!(group(&h, "Low Group").1, "clicking it expands the group");
}

/// #13127 review: a group row whose eye-column colour menu is open stays whole when it scrolls
/// out of view. The menu is the eye's own popup, which egui closes if a frame doesn't show it.
#[test]
fn eye_menu_survives_its_group_scrolling_off_screen() {
    let mut h = harness_with(long_list_with_groups(), 1.0);
    let window = h.ctx.content_rect();
    let high = h.get_by_label("High Group").rect();
    assert!(window.contains_rect(high), "High Group shows: {high:?}");
    // Right-click High Group's eye column (the row's left 30 points): its colour menu opens.
    let eye = egui::pos2(high.left() + 15.0, high.center().y);
    h.hover_at(eye);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: eye, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: eye, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!(egui::Popup::is_any_open(&h.ctx), "the eye column's colour menu opened");
    // Scroll the list to its bottom group, taking High Group out of view, without a click.
    h.query_by_label("Expand group Low Group").expect("Low Group's disclosure").scroll_to_me();
    h.run_steps(6);
    let high = h.get_by_label("High Group").rect();
    let low = h.get_by_label("Low Group").rect();
    // Low Group is in view, and High Group sits more than a window's height above it, so it is
    // outside the panel's (shorter) list.
    assert!(window.contains_rect(low), "scrolled to Low Group: {low:?}");
    assert!(high.bottom() < low.top() - window.height(), "High Group is out of view: {high:?} vs {low:?}");
    assert!(egui::Popup::is_any_open(&h.ctx), "the colour menu stays open while its row is out of view");
}
