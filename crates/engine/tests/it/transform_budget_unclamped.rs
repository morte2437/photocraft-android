//! #6430: integration tests use the production budget (Memory Usage), not the unit tests' reduced
//! one.
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::{Value, json};

fn painted() -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 24, "height": 16})).unwrap();
    session.execute("layer.new.layer", json!({})).unwrap();
    session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 2, 11, 10), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    session
}

/// `command` with `params` is refused as too large, and the document, its revision and History
/// are as they were.
fn assert_refused_without_changes(session: &mut Session, command: &str, params: Value) {
    let state = session.active().unwrap();
    let (document, revision, history) = (state.doc.clone(), state.revision, state.history.past_len());
    let error = session.execute(command, params).unwrap_err().to_string();
    assert!(error.starts_with("too large to transform"), "{error}");
    let state = session.active().unwrap();
    assert!(state.doc == document, "refusal changed the document");
    assert_eq!((state.revision, state.history.past_len()), (revision, history));
}

#[test]
fn extreme_layer_scales_are_refused_before_a_clipped_strip_is_warped() {
    let mut session = painted();
    session.execute("filter.render.clouds", json!({})).unwrap();
    for scale in [1e9, 1e30, 1e308, -1e308] {
        for axis in ["scaleX", "scaleY"] {
            assert_refused_without_changes(&mut session, "edit.transform", json!({(axis): scale}));
        }
    }
}

/// Transform Selection refuses the same extreme scales and keeps the selection.
#[test]
fn extreme_selection_scales_are_refused_without_losing_the_selection() {
    let mut session = painted();
    session.execute("select.rect", json!({"x": 10, "y": 2, "width": 1, "height": 8})).unwrap();
    for scale in [1e30, 1e308] {
        for axis in ["scaleX", "scaleY"] {
            assert_refused_without_changes(&mut session, "select.transformSelection", json!({(axis): scale}));
        }
    }
}

/// #7807: what one transform's result may take is Memory Usage (Preferences › Performance), so
/// raising it lets a refused transform through.
#[test]
fn memory_usage_is_what_a_transform_may_take() {
    let mut session = painted();
    // The 1 x 8 px column stretched 40000 times about its centre: 40003 x 10 px of 8-bit RGBA, 1.6 MB.
    let stretch = json!({"scaleX": 4e6, "interpolation": "nearest"});
    session.edit_prefs(|p| p.performance.memory_usage_mb = 1);
    let state = session.active().unwrap();
    let (document, history) = (state.doc.clone(), state.history.past_len());
    let error = session.execute("edit.transform", stretch.clone()).unwrap_err().to_string();
    assert!(error.contains("would be 40003 x 10 px (2 MB), more than the 1 MB Memory Usage allows (Preferences › Performance)"), "{error}");
    let state = session.active().unwrap();
    assert!(state.doc == document && state.history.past_len() == history, "refusal changed the document");
    session.edit_prefs(|p| p.performance.memory_usage_mb = 2);
    session.execute("edit.transform", stretch).unwrap();
    let state = session.active().unwrap();
    let bounds = state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert_eq!((bounds.width(), bounds.height()), (40_000, 8), "stretched, nothing clipped");
}
