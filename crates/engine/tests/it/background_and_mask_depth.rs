use photocraft_color::SampleType;
use photocraft_engine::Session;
use serde_json::{Value, json};

const STYLES: [&str; 10] =
    ["dropShadow", "innerShadow", "outerGlow", "innerGlow", "stroke", "colorOverlay", "gradientOverlay", "patternOverlay", "bevelEmboss", "satin"];
const VECTOR_MASKS: [&str; 3] = ["layer.vectorMask.add", "layer.vectorMask.revealAll", "layer.vectorMask.hideAll"];

fn background_only() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s
}

fn has_background(s: &Session) -> bool {
    s.active().unwrap().doc.background().is_some()
}

fn converts_and_undoes(cmd: &str, params: Value) {
    let mut s = background_only();
    s.execute(cmd, params).unwrap_or_else(|e| panic!("{cmd}: {e}"));
    assert!(!has_background(&s), "{cmd} left a Background carrying the change");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(has_background(&s), "{cmd}: one undo restores the Background");
}

#[test]
fn layer_styles_and_vector_masks_turn_the_background_into_a_layer() {
    for kind in STYLES {
        converts_and_undoes(&format!("layer.layerStyle.{kind}"), json!({}));
    }
    for cmd in VECTOR_MASKS {
        converts_and_undoes(cmd, json!({}));
    }
    converts_and_undoes("layer.layerStyle.replace", json!({"effects": [{"kind": "stroke"}]}));
}

#[test]
fn clearing_a_style_keeps_the_background() {
    let mut s = background_only();
    s.execute("layer.layerStyle.replace", json!({"effects": []})).unwrap();
    assert!(has_background(&s));
}

fn mask_sample(s: &Session) -> SampleType {
    let st = s.active().unwrap();
    st.doc.walk().into_iter().find_map(|(_, _, l)| l.mask.as_ref().map(|m| m.surface.format().sample)).expect("a layer mask")
}

fn painted(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[5, 10], [30, 10]], "size": 8, "hardness": 1.0, "color": "#ff0000"})).unwrap();
    s
}

#[test]
fn frame_clips_and_remove_background_masks_match_the_document_depth() {
    for depth in [16, 32] {
        let want = if depth == 16 { SampleType::U16 } else { SampleType::F32 };
        for (cmd, params) in [
            ("layer.new.frameFromLayers", json!({})),
            ("layer.new.frame", json!({"x": 4, "y": 4, "width": 16, "height": 12})),
            ("layer.new.frame", json!({"x": 4, "y": 4, "width": 16, "height": 12, "shape": "ellipse"})),
            ("layer.removeBackground", json!({})),
        ] {
            let mut s = painted(depth);
            s.execute(cmd, params.clone()).unwrap_or_else(|e| panic!("{cmd} {params}: {e}"));
            if cmd == "layer.removeBackground" {
                assert_eq!(mask_sample(&s), want, "{cmd} {params} in a {depth}-bit document");
            } else {
                let st = s.active().unwrap();
                let frame = st.doc.layer(st.active_layer.unwrap()).unwrap();
                assert!(frame.frame.as_ref().is_some_and(|f| f.valid()));
                assert!(frame.vector_mask.is_some() && frame.mask.is_none(), "frames retain vector geometry at every depth");
                let output = photocraft_compose::flatten_to_surface(&st.doc, st.doc.pixel_format(), None);
                assert_eq!(output.format().sample, want);
                if cmd == "layer.new.frameFromLayers" {
                    let image = photocraft_compose::render_layer_in(&st.doc, frame, st.doc.bounds());
                    assert!(image.px.iter().any(|p| p[3] > 0.), "framed image remains visible");
                }
            }
        }
    }
}

fn copied_style_over_background() -> (Session, u64, u64) {
    let mut s = background_only();
    let bg = s.active().unwrap().doc.background().unwrap().id.0;
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
    s.execute("layer.layerStyle.copyLayerStyle", json!({})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap().0;
    (s, bg, layer)
}

#[test]
fn pasting_a_style_onto_the_background_turns_it_into_a_layer() {
    let (mut s, bg, _) = copied_style_over_background();
    s.execute("layer.layerStyle.pasteLayerStyle", json!({"layer": bg})).unwrap();
    assert!(!has_background(&s), "the pasted style landed on a Background");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(has_background(&s), "one undo restores the Background");
}

#[test]
fn pasting_a_style_with_no_layer_skips_the_background() {
    let (mut s, bg, layer) = copied_style_over_background();
    s.execute("layer.select", json!({"layer": bg})).unwrap();
    s.execute("layer.select", json!({"layer": layer, "mode": "add"})).unwrap();
    s.execute("layer.layerStyle.pasteLayerStyle", json!({})).unwrap();
    assert!(has_background(&s));
    assert!(s.active().unwrap().doc.background().unwrap().effects.items.is_empty());
}
