use super::*;
use photocraft_doc::Layer;

const PACKET: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:app="http://example.com/app/1.0/"><app:arc>120</app:arc></rdf:Description></rdf:RDF></x:xmpmeta>"#;

fn fixture(depth: u8) -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "depth": depth})).unwrap();
    let (a, b) = s
        .edit("Fixture", |doc, _| {
            let a = Layer::raster("A", doc.pixel_format());
            let b = Layer::group("B", vec![Layer::raster("Child", doc.pixel_format())]);
            let ids = (a.id, b.id);
            doc.layers = vec![a, b];
            Ok(ids)
        })
        .unwrap();
    s.select_layer(a).unwrap();
    (s, a, b)
}

fn xmp(s: &Session, id: LayerId) -> Option<String> {
    s.active().unwrap().doc.layer(id).unwrap().xmp.as_deref().map(str::to_string)
}

#[test]
fn sets_reads_and_removes_as_one_undo_step_each() {
    for depth in [8, 16, 32] {
        let (mut s, a, b) = fixture(depth);
        assert_eq!(s.execute(ID, json!({})).unwrap(), json!({"layer": a.0, "rawData": null, "changed": false}));
        let past = s.active().unwrap().history.past_len();
        let before = s.active().unwrap().doc.clone();

        // The active layer by default; a group by id.
        assert_eq!(s.execute(ID, json!({"rawData": PACKET})).unwrap(), json!({"layer": a.0, "rawData": PACKET, "changed": true}));
        assert_eq!(s.execute(ID, json!({"layer": b.0, "rawData": PACKET})).unwrap()["changed"], true);
        assert_eq!(s.active().unwrap().history.past_len(), past + 2);
        assert_eq!(s.active().unwrap().last_damage, Some(photocraft_geom::Rect::EMPTY));
        assert_eq!(s.execute(ID, Value::Null).unwrap()["rawData"], PACKET);
        assert_eq!(crate::inspect::layer(s.active().unwrap().doc.layer(b).unwrap())["hasXmp"], true);

        // Only the packets changed.
        let mut expected = (*before).clone();
        for id in [a, b] {
            expected.layer_mut(id).unwrap().xmp = Some(PACKET.into());
        }
        assert_eq!(*s.active().unwrap().doc, expected, "depth {depth}");

        // The same packet again is not an edit.
        assert_eq!(s.execute(ID, json!({"rawData": PACKET})).unwrap()["changed"], false);
        assert_eq!(s.active().unwrap().history.past_len(), past + 2);

        assert_eq!(s.execute(ID, json!({"rawData": null})).unwrap(), json!({"layer": a.0, "rawData": null, "changed": true}));
        assert_eq!(xmp(&s, a), None);
        assert!(s.undo());
        assert_eq!(xmp(&s, a).as_deref(), Some(PACKET));
        assert!(s.undo() && s.undo());
        assert_eq!(s.active().unwrap().doc, before);
        assert!(s.redo());
        assert_eq!(xmp(&s, a).as_deref(), Some(PACKET));
    }
}

#[test]
fn duplicating_a_layer_copies_its_packet() {
    let (mut s, a, _) = fixture(8);
    s.execute(ID, json!({"rawData": PACKET})).unwrap();
    s.execute("layer.duplicate", json!({})).unwrap();
    let copy = s.active().unwrap().active_layer.unwrap();
    assert_ne!(copy, a);
    assert_eq!(xmp(&s, copy).as_deref(), Some(PACKET));
}

#[test]
fn refuses_what_is_not_an_xmp_packet() {
    let (mut s, a, _) = fixture(8);
    let refused = |s: &mut Session, p: Value| matches!(s.execute(ID, p), Err(EngineError::BadParams { .. }));
    for p in [
        json!([]),
        json!({"layer": "A"}),
        json!({"rawData": 7}),
        json!({"rawData": ""}),
        json!({"rawData": "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">"}),
        json!({"rawData": "<html/>"}),
        json!({"rawData": "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\u{0}</x:xmpmeta>"}),
        json!({"rawData": "<!DOCTYPE x [<!ENTITY a \"aaaa\">]><x:xmpmeta xmlns:x=\"adobe:ns:meta/\">&a;</x:xmpmeta>"}),
        json!({"rawData": format!("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">{}</x:xmpmeta>", "a".repeat(MAX_LAYER_XMP))}),
    ] {
        assert!(refused(&mut s, p.clone()), "{p:.80}");
    }
    let r = s.execute(ID, json!({"layer": 999_999_999}));
    assert!(matches!(r, Err(EngineError::UnknownLayer { .. })), "{r:?}");
    assert_eq!(xmp(&s, a), None);
    // A bare RDF root and a packet wrapper are packets.
    let rdf = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"/>"#;
    assert_eq!(s.execute(ID, json!({"rawData": rdf})).unwrap()["changed"], true);
    let wrapped = format!("<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>{PACKET}<?xpacket end=\"w\"?>");
    assert_eq!(s.execute(ID, json!({"rawData": wrapped})).unwrap()["changed"], true);
}

#[test]
fn needs_a_document_and_a_layer() {
    let mut s = Session::new();
    assert!(matches!(s.execute(ID, json!({})), Err(EngineError::Disabled(..))));
    assert!(enabled(&s).is_err());
}
