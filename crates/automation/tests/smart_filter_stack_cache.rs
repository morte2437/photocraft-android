//! One integration-test process keeps the global smart filter stage cache isolated from the
//! parallel tests in `tests/it`, so a denied render can be shown to have cached nothing.
use photocraft_automation::workspace::authorize_desktop_engine_step;
use photocraft_engine::{Session, smart_cmds};
use serde_json::json;

#[path = "support/smart_filter_fixture.rs"]
mod fixture;

use fixture::{Fixture, assert_denied};

#[test]
fn an_ambient_map_or_lut_never_renders_unfiltered_pixels_into_the_stack_cache() {
    // The denial path must fail the render before any stage is cached process-wide.
    for lookup in [false, true] {
        let f = Fixture::new();
        let (doc, layer) = f.saved_filter(lookup);
        let mut s = Session::new();
        s.add_document(doc, None);
        s.authorize = Some(authorize_desktop_engine_step);
        let bytes = smart_cmds::stack_cache_bytes();
        assert_denied(s.execute("layer.smartFilter.setVisible", json!({"layer": layer.0, "visible": true})));
        assert_eq!(bytes, smart_cmds::stack_cache_bytes(), "a denied render cached no stage");
    }
}
