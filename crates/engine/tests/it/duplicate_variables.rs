//! Variable definitions must continue targeting their layers after a copied document gets
//! fresh ids (#4969), and a merged-only copy must not keep bindings to discarded layers.

use photocraft_doc::{Document, Layer, LayerContent, LayerId, Size, TextLayer};
use photocraft_engine::Session;
use serde_json::json;

fn scene() -> (Session, LayerId, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let ids = s
        .edit("setup", |doc, _| {
            let fmt = doc.pixel_format();
            let mut photo = Layer::raster("photo", fmt);
            photo.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 64, 48), &photocraft_raster::from_rgba(&fmt, [0.5, 0.5, 0.5, 1.0]));
            let badge = Layer::raster("badge", fmt);
            let title = Layer::new("title", LayerContent::Text(TextLayer { text: "Old".into(), ..Default::default() }));
            let (p, b, t) = (photo.id, badge.id, title.id);
            doc.layers.push(photo);
            doc.layers.push(badge);
            doc.layers.push(title);
            Ok((p, b, t))
        })
        .unwrap();
    (s, ids.0, ids.1, ids.2)
}

fn red_png(dir: &str) -> String {
    let doc = Document::with_background(
        "red",
        Size::new(4, 4),
        photocraft_doc::ColorMode::Rgb,
        photocraft_doc::SampleType::U8,
        photocraft_doc::Color::rgb(1.0, 0.0, 0.0),
    );
    let bytes = photocraft_io::export(&doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    let path = format!("{dir}/replacement.png");
    std::fs::write(&path, bytes).unwrap();
    path
}

fn copied_layer(s: &Session, name: &str) -> LayerId {
    s.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap().2.id
}

fn define_all(s: &mut Session, photo: LayerId, badge: LayerId, title: LayerId, png: &str) {
    s.execute(
        "image.variables.define",
        json!({"defs": [
            {"name": "showBadge", "layer": badge.0, "type": "visibility"},
            {"name": "headline", "layer": title.0, "type": "textReplacement"},
            {"name": "photo", "layer": photo.0, "type": "pixelReplacement", "method": "conform"},
        ]}),
    )
    .unwrap();
    s.execute(
        "image.variables.dataSets",
        json!({"dataSets": [
            {"name": "A", "values": [
                {"variable": "showBadge", "kind": "visibility", "value": false},
                {"variable": "headline", "kind": "text", "value": "Hello"},
                {"variable": "photo", "kind": "pixels", "value": png},
            ]},
        ]}),
    )
    .unwrap();
}

#[test]
fn duplicated_documents_keep_variables_targeting_the_copied_layers() {
    let dir = std::env::temp_dir().join(format!("pc-dup-vars-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = red_png(&dir.to_string_lossy());

    for command in ["image.duplicate", "history.newDocument"] {
        let (mut s, photo, badge, title) = scene();
        define_all(&mut s, photo, badge, title, &png);
        s.execute(command, json!({})).unwrap();

        let doc = &s.active().unwrap().doc;
        let new_photo = copied_layer(&s, "photo");
        let new_badge = copied_layer(&s, "badge");
        let new_title = copied_layer(&s, "title");
        assert_ne!(new_photo, photo, "{command}: layers must get new identities");
        let targets: Vec<LayerId> = doc.variables.defs.iter().map(|d| d.layer).collect();
        assert_eq!(targets, vec![new_badge, new_title, new_photo], "{command}: variable defs follow their layers");

        let r = s.execute("image.applyDataSet", json!({"name": "A"})).unwrap();
        assert_eq!(r["applied"], "A");
        let doc = &s.active().unwrap().doc;
        assert!(!doc.layer(new_badge).unwrap().visible, "{command}: visibility drives the copy");
        match &doc.layer(new_title).unwrap().content {
            LayerContent::Text(t) => assert_eq!(t.text, "Hello", "{command}: text drives the copy"),
            _ => panic!("not a text layer"),
        }
        let px = doc.layer(new_photo).unwrap().surface().unwrap().pixel(0, 0);
        assert!(px[0] > 0.9 && px[1] < 0.1, "{command}: pixel replacement drives the copy: {px:?}");

        // The source document still targets its own layers and is untouched.
        s.set_active(0);
        let src = &s.active().unwrap().doc;
        let src_targets: Vec<LayerId> = src.variables.defs.iter().map(|d| d.layer).collect();
        assert_eq!(src_targets, vec![badge, title, photo], "{command}: source defs untouched");
        assert!(src.layer(badge).unwrap().visible);
        assert!(src.layer(photo).unwrap().surface().unwrap().pixel(0, 0)[0] < 0.9);
    }
}

#[test]
fn dangling_variable_targets_stay_dangling_after_duplication() {
    let (mut s, photo, badge, title) = scene();
    define_all(&mut s, photo, badge, title, "unused.png");
    // A def may already refer to a deleted layer before duplication.
    let missing = LayerId(u64::MAX);
    s.edit("Break one def", |doc, _| {
        doc.variables.defs.iter_mut().find(|d| d.name == "showBadge").unwrap().layer = missing;
        Ok(())
    })
    .unwrap();
    s.execute("image.duplicate", json!({})).unwrap();

    let doc = &s.active().unwrap().doc;
    let def = doc.variables.defs.iter().find(|d| d.name == "showBadge").unwrap();
    assert_eq!(def.layer, missing, "a deleted target must not be rewritten to an unrelated new id");
    let photo_def = doc.variables.defs.iter().find(|d| d.name == "photo").unwrap();
    assert_eq!(photo_def.layer, copied_layer(&s, "photo"), "live targets still remap");
}

#[test]
fn merged_only_duplicate_does_not_carry_variables_for_removed_layers() {
    let dir = std::env::temp_dir().join(format!("pc-dup-vars-merged-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = red_png(&dir.to_string_lossy());
    let (mut s, photo, badge, title) = scene();
    define_all(&mut s, photo, badge, title, &png);

    s.execute("image.duplicate", json!({"mergedOnly": true})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!(d.layers.len(), 1, "merged-only copy is a single composite layer");
    assert!(d.variables.defs.is_empty(), "old variables cannot address discarded layers");
    assert!(d.variables.data_sets.is_empty());
    assert_eq!(d.variables.active, None);

    s.set_active(0);
    assert_eq!(s.active().unwrap().doc.variables.defs.len(), 3, "original variables are preserved");
}
