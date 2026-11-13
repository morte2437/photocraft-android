use super::*;
use egui::accesskit::{Role, Toggled};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_cms::Builtin;
use photocraft_doc::{Color, SampleType, Size};
use std::{cell::RefCell, path::Path, rc::Rc};

const CONVERT: &str = "Convert to sRGB";
const EMBED: &str = "Embed Color Profile";
type Writes = Rc<RefCell<Vec<Vec<u8>>>>;

fn harness(gpu: bool) -> (Harness<'static, PhotocraftApp>, Writes) {
    let writes = Writes::default();
    let captured = writes.clone();
    let builder = Harness::builder().with_size(egui::vec2(1100.0, 800.0)).with_max_steps(64);
    let builder = if gpu { builder.wgpu() } else { builder };
    let h = builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let (picker, _) = crate::file_dialog::fake(vec![
            Some(crate::FileDialogAnswer::SaveTo("converted.png".into())),
            Some(crate::FileDialogAnswer::SaveTo("original.png".into())),
        ]);
        let services = crate::Services {
            file_dialog: Some(picker),
            export: Some(Box::new(|doc, path, settings| {
                photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(move |_, bytes| {
                captured.borrow_mut().push(bytes.to_vec());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let mut doc = Document::with_background("Wide gamut", Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let blue = [30.0 / 255.0, 30.0 / 255.0, 200.0 / 255.0, 1.0];
        let surface = doc.layers[0].surface_mut().unwrap();
        surface.fill_rect(photocraft_geom::Rect::new(50, 25, 150, 125), &photocraft_raster::from_rgba(&surface.format(), blue));
        doc.icc_profile = Some(Builtin::AdobeRgbCompat.profile().to_bytes());
        app.session.add_document(doc, None);
        app
    });
    (h, writes)
}

fn checkbox(h: &Harness<'_, PhotocraftApp>, label: &str) -> Toggled {
    h.get_by_role_and_label(Role::CheckBox, label).accesskit_node().toggled().unwrap()
}

fn click(h: &mut Harness<'_, PhotocraftApp>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    h.run_steps(6);
}

fn open_dialog(h: &mut Harness<'_, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(6);
}

fn snapshot(h: &mut Harness<'_, PhotocraftApp>, dir: Option<&Path>, file: &str) {
    if let Some(dir) = dir {
        h.render().unwrap().save(dir.join(file)).unwrap();
    }
}

fn verify_png(bytes: &[u8], want: [u8; 3], tolerance: u8, embedded: bool) {
    let doc = photocraft_io::import("out.png", bytes).unwrap().document;
    let pixel = doc.layers[0].surface().unwrap().rgba(100, 75);
    for (channel, (got, want)) in pixel.into_iter().zip(want).enumerate() {
        let got = (got * 255.0).round() as u8;
        assert!(got.abs_diff(want) <= tolerance, "channel {channel}: expected {want} ± {tolerance}, got {got}");
    }
    assert_eq!(doc.icc_profile.is_some(), embedded);
    if let Some(icc) = doc.icc_profile.as_deref() {
        assert!(Profile::parse(icc).unwrap().same_colors(Builtin::AdobeRgbCompat.profile()));
    }
}

fn export_both_ways(snapshots: Option<&Path>) {
    let (mut h, writes) = harness(snapshots.is_some());
    let source = h.state().session.active().unwrap().doc.clone();
    open_dialog(&mut h);
    h.get_by_label("Color Space");
    assert_eq!(checkbox(&h, CONVERT), Toggled::True);
    assert_eq!(checkbox(&h, EMBED), Toggled::True);
    snapshot(&mut h, snapshots, "export-as-default.png");

    click(&mut h, Role::CheckBox, EMBED);
    click(&mut h, Role::Button, "Export");
    verify_png(&writes.borrow()[0], [24, 24, 205], 1, false);

    open_dialog(&mut h);
    assert_eq!(checkbox(&h, CONVERT), Toggled::True);
    assert_eq!(checkbox(&h, EMBED), Toggled::False);
    click(&mut h, Role::CheckBox, CONVERT);
    click(&mut h, Role::CheckBox, EMBED);
    snapshot(&mut h, snapshots, "export-as-document-profile.png");
    click(&mut h, Role::Button, "Export");
    verify_png(&writes.borrow()[1], [30, 30, 200], 0, true);

    assert!(Arc::ptr_eq(&source, &h.state().session.active().unwrap().doc));
    open_dialog(&mut h);
    assert_eq!(checkbox(&h, CONVERT), Toggled::False, "remembered document-profile choice");
    assert_eq!(checkbox(&h, EMBED), Toggled::True);
}

#[test]
fn checkbox_defaults_on_and_controls_png_pixels_and_embedding() {
    export_both_ways(None);
}

#[test]
fn existing_document_profile_preset_and_recorded_export_keep_source_colors() {
    let (mut h, writes) = harness(false);
    h.state_mut().run("file.export.presets.save", json!({"name": "Original colors", "settings": {"profile": "document", "format": "jpg"}})).unwrap();
    open_dialog(&mut h);
    click(&mut h, Role::ComboBox, "Preset");
    h.get_by_label("Original colors").click();
    h.run_steps(4);
    assert_eq!(checkbox(&h, CONVERT), Toggled::False, "existing JPEG preset");
    assert_eq!(checkbox(&h, EMBED), Toggled::True);

    replay(h.state_mut(), &json!({"path": "recorded.png", "format": "png", "profile": "document", "embedProfile": true})).unwrap();
    verify_png(&writes.borrow()[0], [30, 30, 200], 0, true);
}

#[test]
fn export_as_color_space_gpu_snapshots() {
    let Some(dir) = std::env::var_os("PHOTOCRAFT_SRGB_SNAPSHOTS") else { return };
    let dir = std::path::PathBuf::from(dir);
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&dir).unwrap();
    export_both_ways(Some(&dir));
}
