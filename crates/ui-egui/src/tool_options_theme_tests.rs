//! Issue #4178: switching from a Pro theme to any other theme (Adwaita, Studio, Classic, Solarized,
//! Breeze) removed the options bar's controls for the Marquee, Lasso, Paint Bucket, Crop,
//! Perspective Crop and Move tools, leaving a one-line hint. A theme changes sizes and colours
//! (`theme::Tokens`), never which options a tool has.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::ThemeKind;

/// The tools whose options bar used to be a hint outside the Pro themes.
const TOOLS: [Tool; 11] = [
    Tool::Crop,
    Tool::PerspectiveCrop,
    Tool::RectMarquee,
    Tool::EllipseMarquee,
    Tool::SingleRowMarquee,
    Tool::SingleColumnMarquee,
    Tool::Lasso,
    Tool::PolygonLasso,
    Tool::MagneticLasso,
    Tool::PaintBucket,
    Tool::Move,
];

/// The options bar for `tool` under `theme`, with a document open, after a few settled frames.
fn options_bar(theme: ThemeKind, tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 60.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h
}

/// Every labelled node in the bar as `role label`, sorted. The Pro themes end option labels with
/// a colon and start the bar with a Home button; both are chrome, not tool options.
fn controls(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    let mut v: Vec<String> = h
        .root()
        .children_recursive()
        .filter_map(|n| {
            let node = n.accesskit_node();
            let label = node.label().or_else(|| node.value())?;
            let label = label.trim_end_matches(':');
            (label != "Home").then(|| format!("{:?} {label}", node.role()))
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn every_theme_shows_the_same_tool_options() {
    for tool in TOOLS {
        let reference = controls(&options_bar(ThemeKind::ProMedium, tool));
        assert!(reference.len() > 2, "{tool:?}: the Pro bar has options: {reference:?}");
        for theme in ThemeKind::ALL {
            let found = controls(&options_bar(theme, tool));
            let missing: Vec<&String> = reference.iter().filter(|c| !found.contains(c)).collect();
            assert!(missing.is_empty(), "{theme:?} {tool:?}: the options bar lacks {missing:?}; it shows {found:?}");
        }
    }
}

#[test]
fn every_theme_shows_the_crop_options() {
    // The issue's example: in Adwaita the Crop tool had no ratio, Clear, overlay or Delete Cropped
    // Pixels controls, only "Drag a crop box · …".
    for theme in ThemeKind::ALL {
        let mut h = options_bar(theme, Tool::Crop);
        assert!(h.query_by_label_contains("Drag a crop box").is_none(), "{theme:?}: the hint replaced the options");
        assert!(h.state().ui.tool_options.crop_delete);
        h.get_by_label("Delete Cropped Pixels").click();
        h.run_steps(2);
        assert!(!h.state().ui.tool_options.crop_delete, "{theme:?}: the checkbox turns Delete Cropped Pixels off");
        h.state_mut().ui.tool_options.crop_ratio = "1:1".into();
        h.run_steps(2);
        h.get_by_label("Clear").click();
        h.run_steps(2);
        assert!(h.state().ui.tool_options.crop_ratio.is_empty(), "{theme:?}: Clear empties the ratio");
    }
}
