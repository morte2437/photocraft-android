//! Copy placement must use source-space bounds before destination cache clipping.
use photocraft_doc::LayerContent;
use photocraft_engine::Session;
use serde_json::json;

/// Cover source clipping, groups, missing caches, destination depth, placement and Undo/Redo.
#[test]
fn copying_shapes_to_smaller_documents_preserves_requested_center() {
    for x in [90, 700] {
        for grouped in [false, true] {
            for missing_cache in [false, true] {
                for depth in [8, 16] {
                    for at in [None, Some([40, 60])] {
                        let mut s = Session::new();
                        s.execute("file.new", json!({"width":800,"height":600,"background":"transparent"})).unwrap();
                        let mut ids = Vec::new();
                        for offset in if grouped { vec![0, 30] } else { vec![0] } {
                            s.execute("shape.create", json!({"kind":"rect","rect":[x+offset,20,20,20],"fill":"#ff0000"})).unwrap();
                            s.execute("layer.layerStyle.dropShadow", json!({"size":4,"distance":3})).unwrap();
                            ids.push(s.active().unwrap().active_layer.unwrap());
                        }
                        if grouped {
                            s.active_mut().unwrap().selected_layers = ids.clone();
                            s.execute("layer.groupLayers", json!({})).unwrap();
                        }
                        if missing_cache {
                            s.edit("drop cached shapes", |doc, _| {
                                for id in &ids {
                                    let LayerContent::Shape(shape) = &mut doc.layer_mut(*id).unwrap().content else { panic!("shape") };
                                    shape.cache = None;
                                }
                                Ok(())
                            })
                            .unwrap();
                        }
                        s.execute("file.new", json!({"width":100,"height":100,"depth":depth,"background":"transparent"})).unwrap();
                        let before = s.active().unwrap().doc.clone();
                        let mut params = json!({"source":0,"document":1,"center":true});
                        if let Some(at) = at {
                            params["center"] = json!(false);
                            params["at"] = json!(at);
                        }
                        let result = s.execute("layer.copyToDocument", params).unwrap();
                        let target = at.unwrap_or([50, 50]);
                        let dx = target[0] - (x + if grouped { 25 } else { 10 });
                        let dy = target[1] - 30;
                        assert_eq!(result["offset"], json!([dx, dy]), "x={x}, grouped={grouped}, cache missing={missing_cache}");
                        let st = s.active().unwrap();
                        let shapes: Vec<_> = st
                            .doc
                            .walk()
                            .into_iter()
                            .filter_map(|(_, _, layer)| if let LayerContent::Shape(shape) = &layer.content { Some(shape) } else { None })
                            .collect();
                        assert_eq!(shapes.len(), if grouped { 2 } else { 1 });
                        for shape in shapes {
                            let fresh = photocraft_vector::render_shape(shape, st.doc.pixel_format(), st.doc.bounds());
                            assert!(
                                shape.cache.as_ref() == Some(&fresh),
                                "cache mismatch: x={x}, grouped={grouped}, cache missing={missing_cache}, depth={depth}, at={at:?}"
                            );
                            assert!(!fresh.content_bounds().is_empty());
                        }
                        let after = st.doc.clone();
                        assert!(s.undo());
                        assert_eq!(s.active().unwrap().doc, before);
                        assert!(s.redo());
                        assert_eq!(s.active().unwrap().doc, after);
                    }
                }
            }
        }
    }
}
