//! #6541: the Layers panel marks an adjustment layer PhotoCraft keeps as imported but cannot
//! render, so a picture that differs from Photoshop's has a visible reason.

use egui::vec2;
use egui_kittest::Harness;
use photocraft_doc::{Adjustment, Layer, LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;
use crate::layer_row_ui::{Indicator, RowRects, recorded};

fn row(h: &Harness<'_, PhotocraftApp>, id: LayerId) -> RowRects {
    recorded(&h.ctx).into_iter().find(|r| r.layer == id.0).unwrap()
}

#[test]
fn an_unsupported_adjustment_row_carries_a_warning_marker() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let plain = LayerId(s.execute("layer.new.layer", json!({"name": "plain"})).unwrap()["layer"].as_u64().unwrap());
    let mystery = Layer::new("Mystery", LayerContent::Adjustment(Adjustment::Unsupported { psd_key: "xyzw".into(), raw: vec![1, 2, 3].into() }));
    let mystery_id = mystery.id;
    s.edit("unsupported fixture", |doc, active| {
        doc.layers.push(mystery);
        *active = Some(mystery_id);
        Ok(())
    })
    .unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(8);
    let marked = row(&h, mystery_id);
    assert!(marked.indicators.iter().any(|(k, _)| *k == Indicator::Unsupported), "{:?}", marked.indicators);
    assert!(!row(&h, plain).indicators.iter().any(|(k, _)| *k == Indicator::Unsupported));
    // The marker takes room from the name, never overlaps it.
    let (_, marker) = marked.indicators.iter().find(|(k, _)| *k == Indicator::Unsupported).copied().unwrap();
    if let Some(name) = marked.name {
        assert!(name.right() <= marker.left(), "name {name:?} runs under the marker {marker:?}");
    }
}
