//! #12673, #13033, #12656, #12967: a press with the Burn, Sponge, Blur or Smudge tool in an Indexed
//! Color or Bitmap document opens the reference application's colour-mode alert with OK, once, and
//! paints nothing; in RGB the same press still paints.

use serde_json::json;

use super::brush_refusal_alert_tests::{alerts, app, refused, steps, stroke};
use super::*;

const TOOLS: [(Tool, &str); 4] = [(Tool::Burn, "burn tool"), (Tool::Sponge, "sponge tool"), (Tool::Blur, "blur tool"), (Tool::Smudge, "smudge tool")];

#[test]
fn indexed_color_documents_get_the_mode_alert() {
    for (tool, name) in TOOLS {
        let mut app = app(tool);
        app.run("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
        refused(&mut app, &format!("Could not use the {name} because it does not work with Indexed Color images (convert image to RGB Color mode to edit)."));
    }
}

#[test]
fn bitmap_documents_get_the_mode_alert() {
    for (tool, name) in TOOLS {
        let mut app = app(tool);
        app.run("image.mode.grayscale", json!({})).unwrap();
        app.run("image.mode.bitmap", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.mode, photocraft_doc::ColorMode::Bitmap);
        refused(&mut app, &format!("Could not use the {name} because it does not work with Bitmap images (convert image to Grayscale mode to edit)."));
    }
}

#[test]
fn rgb_documents_keep_painting() {
    for (tool, _) in TOOLS {
        let mut app = app(tool);
        let before = steps(&app);
        stroke(&mut app);
        assert!(alerts(&app).is_empty(), "{tool:?}: no alert");
        assert_eq!(steps(&app), before + 1, "{tool:?}: the stroke was applied");
    }
}
