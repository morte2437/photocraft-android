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

/// The bar's controls (not their captions) as `(label, rect)`, left to right.
fn controls_in_order(h: &Harness<'_, PhotocraftApp>) -> Vec<(String, egui::Rect)> {
    let mut v: Vec<(String, egui::Rect)> = h
        .root()
        .children_recursive()
        .filter(|n| n.accesskit_node().role() != egui::accesskit::Role::Label)
        .filter_map(|n| Some((n.accesskit_node().label()?.trim_end_matches(':').to_string(), n.rect())))
        .collect();
    v.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
    v
}

/// #9784: Photoshop's painting tools show Opacity right after Mode, as the Pro themes do. The
/// other themes used to put Hardness between them for the Brush and Eraser.
#[test]
fn every_theme_puts_opacity_right_after_mode() {
    for tool in [Tool::Brush, Tool::Eraser, Tool::Pencil, Tool::CloneStamp] {
        for theme in ThemeKind::ALL {
            let found = controls_in_order(&options_bar(theme, tool));
            let names: Vec<&str> = found.iter().map(|(l, _)| l.as_str()).collect();
            let mode = names.iter().position(|l| *l == "Mode").unwrap_or_else(|| panic!("{theme:?} {tool:?}: no Mode control in {names:?}"));
            assert_eq!(names.get(mode + 1), Some(&"Opacity"), "{theme:?} {tool:?}: Opacity follows Mode in {names:?}");
            assert!(found[mode].1.right() <= found[mode + 1].1.left(), "{theme:?} {tool:?}: Mode and Opacity don't overlap: {found:?}");
            if matches!(tool, Tool::Brush | Tool::Eraser) && !crate::theme::Tokens::for_kind(theme).pro {
                let size = names.iter().position(|l| *l == "Size").unwrap_or_else(|| panic!("{theme:?} {tool:?}: no Size control in {names:?}"));
                assert_eq!(names.get(size + 1), Some(&"Hardness"), "{theme:?} {tool:?}: Hardness stays right after Size: {names:?}");
                assert!(size + 1 < mode, "{theme:?} {tool:?}: Size and Hardness come before Mode: {names:?}");
            }
        }
    }
}

/// #9784 moved the Hardness field beside Size outside the Pro themes. It still follows the
/// Eraser's Mode: a Pencil or Block eraser is always hard (Photoshop), so Hardness is disabled
/// there and enabled in Brush mode.
#[test]
fn the_eraser_hardness_field_follows_its_mode_outside_pro() {
    for theme in ThemeKind::ALL.into_iter().filter(|k| !crate::theme::Tokens::for_kind(*k).pro) {
        for (mode, enabled) in [("brush", true), ("pencil", false), ("block", false)] {
            let mut h = options_bar(theme, Tool::Eraser);
            h.state_mut().ui.tool_options.eraser_mode = mode.into();
            h.run_steps(3);
            let disabled: Vec<bool> = h
                .root()
                .children_recursive()
                .filter(|n| n.accesskit_node().role() != egui::accesskit::Role::Label && n.accesskit_node().label().as_deref() == Some("Hardness"))
                .map(|n| n.accesskit_node().is_disabled())
                .collect();
            assert!(!disabled.is_empty(), "{theme:?} {mode}: the Hardness field is shown");
            assert!(disabled.iter().all(|d| *d != enabled), "{theme:?} {mode}: Hardness enabled = {enabled}, found disabled = {disabled:?}");
        }
    }
}
