use super::*;
use photocraft_doc::Layer;
use photocraft_geom::Rect;

/// A 64 x 48 document with a red layer A and a blue layer B and three comps: both shown, B hidden, A hidden.
fn session() -> (Session, LayerId, LayerId, Vec<u64>) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let (a, b) = s
        .edit("setup", |doc, _| {
            let fmt = doc.pixel_format();
            let mut a = Layer::raster("A", fmt);
            a.surface_mut().unwrap().fill_rect(Rect::new(4, 4, 14, 14), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
            let mut b = Layer::raster("B", fmt);
            b.surface_mut().unwrap().fill_rect(Rect::new(30, 20, 40, 30), &photocraft_raster::from_rgba(&fmt, [0.0, 0.0, 1.0, 1.0]));
            let (ia, ib) = (a.id, b.id);
            doc.layers.push(a);
            doc.layers.push(b);
            Ok((ia, ib))
        })
        .unwrap();
    let mut comps = vec![s.execute("layerComp.new", json!({"name": "Both"})).unwrap()["comp"].as_u64().unwrap()];
    s.edit("hide b", |d, _| {
        d.layer_mut(b).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    comps.push(s.execute("layerComp.new", json!({"name": "NoB"})).unwrap()["comp"].as_u64().unwrap());
    s.edit("show b hide a", |d, _| {
        d.layer_mut(b).unwrap().visible = true;
        d.layer_mut(a).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    comps.push(s.execute("layerComp.new", json!({"name": "NoA"})).unwrap()["comp"].as_u64().unwrap());
    // Leave the document in a state no comp shows (A and B both hidden).
    s.edit("hide both", |d, _| {
        d.layer_mut(b).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    (s, a, b, comps)
}

fn vis(s: &Session, id: LayerId) -> bool {
    s.active().unwrap().doc.layer(id).unwrap().visible
}

fn past(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-comps-hist-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn run_leaves_document_on_last_comp_with_one_state_each() {
    let (mut s, a, b, comps) = session();
    let dir = temp_dir("run");
    let before = past(&s);
    s.execute("file.export.layerCompsToFiles", json!({"dir": dir.to_string_lossy(), "prefix": "doc"})).unwrap();
    assert_eq!(past(&s), before + 3);
    assert_eq!(s.active().unwrap().doc.last_applied_comp, Some(comps[2] as u32));
    assert!(!vis(&s, a));
    assert!(vis(&s, b));
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Apply Layer Comp"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undoing_the_run_steps_back_to_the_state_before_it() {
    let (mut s, a, b, comps) = session();
    let dir = temp_dir("undo");
    let last_before = s.active().unwrap().doc.last_applied_comp;
    s.execute("file.export.layerCompsToFiles", json!({"dir": dir.to_string_lossy()})).unwrap();
    assert!(s.undo());
    // One undo goes back to comp 2 (A shown, B hidden).
    assert_eq!(s.active().unwrap().doc.last_applied_comp, Some(comps[1] as u32));
    assert!(vis(&s, a) && !vis(&s, b));
    assert!(s.undo());
    assert!(s.undo());
    assert!(!vis(&s, a) && !vis(&s, b));
    assert_eq!(s.active().unwrap().doc.last_applied_comp, last_before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failing_before_writing_changes_neither_document_nor_history() {
    let (mut s, a, b, _) = session();
    let dir = temp_dir("fail");
    let d = dir.to_string_lossy().into_owned();
    let before = past(&s);
    let last = s.active().unwrap().doc.last_applied_comp;
    assert!(s.execute("file.export.layerCompsToFiles", json!({"dir": d, "comps": [9999]})).is_err());
    assert!(s.execute("file.export.layerCompsToFiles", json!({"dir": d, "comps": []})).is_err());
    assert!(s.execute("file.export.layerCompsToFiles", json!({"comps": [1]})).is_err());
    assert_eq!(past(&s), before);
    assert_eq!(s.active().unwrap().doc.last_applied_comp, last);
    assert!(!vis(&s, a) && !vis(&s, b));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn written_files_still_each_show_their_own_comp() {
    let (mut s, _, _, _) = session();
    let dir = temp_dir("files");
    let r = s.execute("file.export.layerCompsToFiles", json!({"dir": dir.to_string_lossy(), "prefix": "doc"})).unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(files.len(), 3);
    assert!(files[0].ends_with("doc_0000_Both.png"), "{files:?}");
    assert!(files[2].ends_with("doc_0002_NoA.png"), "{files:?}");
    let px = |path: &str, rect: Rect| {
        let doc = photocraft_io::import("x.png", &std::fs::read(path).unwrap()).unwrap().document;
        photocraft_compose::render(&doc, rect).px[0]
    };
    let a_rect = Rect::new(8, 8, 9, 9);
    let b_rect = Rect::new(35, 25, 36, 26);
    // The background is white: a present red A has green near 0, a present blue B has red near 0.
    let has_a = |path: &str| px(path, a_rect)[1] < 0.1;
    let has_b = |path: &str| px(path, b_rect)[0] < 0.1;
    assert!(has_a(&files[0]) && has_b(&files[0]), "Both");
    assert!(has_a(&files[1]) && !has_b(&files[1]), "NoB");
    assert!(!has_a(&files[2]) && has_b(&files[2]), "NoA");
    let _ = std::fs::remove_dir_all(&dir);
}
