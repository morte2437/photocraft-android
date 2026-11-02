use photocraft_engine::{Session, paint::tile::b64_encode};
use photocraft_psd::atn;
use serde_json::json;

/// Legacy descriptor syntax must reach the same commands and parameters as version 16.
#[test]
fn legacy_v12_commands_import_like_v16() {
    let bytes = include_bytes!("../../../psd/tests/fixtures/atn/v12-commands.atn");
    let modern = atn::write(&atn::parse(bytes).unwrap()).unwrap();
    let expected = vec![
        ("filter.blur.gaussianBlur".into(), json!({"radius": 2.5})),
        ("image.adjustments.levels".into(), json!({"inBlack": 10, "inWhite": 240, "gamma": 1.2, "outBlack": 0, "outWhite": 250})),
        ("select.all".into(), json!({})),
        ("layer.new.layer".into(), json!({"name": "Legacy layer"})),
    ];
    for input in [bytes.as_slice(), modern.as_slice()] {
        let mut s = Session::new();
        let result = s.execute("actions.load", json!({"data": b64_encode(input)})).unwrap();
        assert_eq!(result["unsupported"], json!([]));
        assert_eq!(s.actions.list[0].steps, expected);
    }
}
