use super::*;

fn roots(name: &str) -> (PathBuf, PathBuf, AuthorizedWorkspace) {
    let base = std::env::temp_dir().join(format!("photocraft-workspace-{}-{name}", std::process::id()));
    let inside = base.join("inside");
    let outside = base.join("outside");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let workspace = AuthorizedWorkspace::new(Some(&inside), Some(&inside)).unwrap();
    (inside, outside, workspace)
}

#[test]
fn export_cleanup_owns_only_created_files_even_during_unwind() {
    let (inside, _, workspace) = roots("export-cleanup");
    std::fs::create_dir(inside.join("frames")).unwrap();
    std::fs::write(inside.join("frames/clip_0009.png"), b"old").unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut export = workspace.export_sequence("frames").unwrap();
        export.write("clip_0000.png", b"new").unwrap();
        assert!(export.write("clip_0009.png", b"overwrite").is_err());
        assert!(export.write("../escape.png", b"escape").is_err());
        panic!("rendering panic");
    }));
    assert!(panic.is_err());
    assert_eq!(std::fs::read_dir(inside.join("frames")).unwrap().count(), 1);
    assert_eq!(std::fs::read(inside.join("frames/clip_0009.png")).unwrap(), b"old");
    assert!(AuthorizedWorkspace::default().export_sequence("frames").is_err());
}

#[cfg(unix)]
#[test]
fn export_directory_symlinks_cannot_escape_the_granted_root() {
    let (inside, outside, workspace) = roots("export-symlink");
    std::os::unix::fs::symlink(&outside, inside.join("escape")).unwrap();
    assert!(workspace.export_sequence("escape").is_err());
    assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn valid_read_and_nonexistent_output_write_stay_in_root() {
    let (inside, _, workspace) = roots("valid");
    std::fs::write(inside.join("read.txt"), b"canary").unwrap();
    assert_eq!(workspace.read("read.txt").unwrap(), b"canary");
    workspace.write("new-output.txt", b"created").unwrap();
    assert_eq!(std::fs::read(inside.join("new-output.txt")).unwrap(), b"created");
}

#[test]
fn large_observed_length_does_not_trigger_a_fixed_limit_or_upfront_reservation() {
    let mut shortened = std::io::Cursor::new(b"psb");
    assert_eq!(read_to_observed_length(&mut shortened, (1 << 30) + 1).unwrap(), b"psb");
}

#[test]
fn read_growth_is_rejected_at_the_observed_length() {
    let mut growing = std::io::repeat(b'x').take(65);
    let error = read_to_observed_length(&mut growing, 64).unwrap_err().to_string();
    assert!(error.contains("grew beyond its observed length of 64 bytes"), "{error}");
    let mut allowed = std::io::repeat(b'x').take(64);
    assert_eq!(read_to_observed_length(&mut allowed, 64).unwrap().len(), 64);
    let mut empty = std::io::Cursor::new(b"x");
    assert!(read_to_observed_length(&mut empty, 0).is_err());
}

#[test]
fn open_pcraft_loads_a_directory_below_the_read_root() {
    let (inside, _, workspace) = roots("pcraft-dir");
    let doc = photocraft_doc::Document::new("Native", photocraft_doc::Size::new(16, 12), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    photocraft_format::PcraftWriter::new().save_dir(&doc, &inside.join("native.pcraft"), &photocraft_format::SaveOptions::default()).unwrap();
    let loaded = workspace.open_pcraft("native.pcraft").unwrap().expect("the directory form loads");
    assert_eq!(loaded.name, doc.name);
    assert_eq!((loaded.size.width, loaded.size.height), (16, 12));
}

#[test]
fn open_pcraft_reports_zip_files_and_missing_paths_clearly() {
    let (inside, _, workspace) = roots("pcraft-errors");
    let doc = photocraft_doc::Document::new("Zipped", photocraft_doc::Size::new(4, 4), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let zip = inside.join("zipped.pcraft");
    photocraft_format::PcraftWriter::new().save_path(&doc, &zip, &photocraft_format::SaveOptions::default()).unwrap();
    assert!(zip.is_file());
    assert!(workspace.open_pcraft("zipped.pcraft").unwrap().is_none(), "the zip form falls back to the bytes path");
    let error = workspace.open_pcraft("missing.pcraft").unwrap_err().to_string();
    assert!(error.contains("failed"), "a missing path propagates its io error: {error}");
    assert!(AuthorizedWorkspace::default().open_pcraft("x.pcraft").is_err(), "no read authority fails closed");
}

#[cfg(unix)]
#[test]
fn open_pcraft_rejects_non_regular_entries_and_escaping_symlinks() {
    let (inside, _, workspace) = roots("pcraft-nonregular");
    let bundle = inside.join("native.pcraft");
    std::fs::create_dir_all(bundle.join("manifest.json")).unwrap();
    let error = workspace.open_pcraft("native.pcraft").unwrap_err().to_string();
    assert!(error.contains("not a regular file"), "a directory entry is refused, not streamed: {error}");

    let (inside2, _, workspace2) = roots("pcraft-symlink");
    let bundle2 = inside2.join("native.pcraft");
    std::fs::create_dir(&bundle2).unwrap();
    std::os::unix::fs::symlink("/dev/zero", bundle2.join("manifest.json")).unwrap();
    assert!(workspace2.open_pcraft("native.pcraft").is_err(), "an escaping symlink entry is refused");
}

fn temp_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".tmp")).collect()
}

#[test]
fn write_replaces_atomically_and_leaves_no_temp_file() {
    let (inside, _, workspace) = roots("atomic");
    std::fs::create_dir_all(inside.join("sub")).unwrap();
    workspace.write("sub/out.psd", b"original").unwrap();
    workspace.write("sub/out.psd", b"replacement").unwrap();
    assert_eq!(std::fs::read(inside.join("sub/out.psd")).unwrap(), b"replacement");
    assert!(temp_files(&inside.join("sub")).is_empty());
    // Renaming over a directory fails: nothing is left behind and the directory survives.
    std::fs::create_dir_all(inside.join("adir")).unwrap();
    assert!(workspace.write("adir", b"x").is_err());
    assert!(inside.join("adir").is_dir());
    assert!(temp_files(&inside).is_empty());
}

/// #7095: replacing a file through the automation path keeps the old file's extended
/// attributes, the same as a save through `atomic_write` does.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
#[test]
fn write_keeps_the_replaced_files_extended_attributes() {
    #[cfg(target_os = "macos")]
    const TAG: &str = "com.apple.metadata:_kMDItemUserTags";
    #[cfg(not(target_os = "macos"))]
    const TAG: &str = "user.photocraft.test";

    let (inside, _, workspace) = roots("xattr");
    let path = inside.join("doc.psd");
    workspace.write("doc.psd", b"original").unwrap();
    if rustix::fs::setxattr(&path, TAG, b"tagged", rustix::fs::XattrFlags::empty()).is_err() {
        return; // a file system without extended attributes: nothing to assert
    }

    workspace.write("doc.psd", b"replacement, longer").unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"replacement, longer");
    let mut buf = [0u8; 64];
    let len = rustix::fs::getxattr(&path, TAG, buf.as_mut_slice()).expect("the attribute was dropped");
    assert_eq!(&buf[..len], b"tagged");
    assert!(temp_files(&inside).is_empty());
}

#[cfg(unix)]
#[test]
fn write_into_read_only_folder_keeps_the_original() {
    use std::os::unix::fs::PermissionsExt;
    let (inside, _, workspace) = roots("readonly");
    let ro = inside.join("ro");
    std::fs::create_dir_all(&ro).unwrap();
    std::fs::write(ro.join("doc.psd"), b"original").unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
    let root_user = std::fs::File::create(ro.join("probe")).is_ok();
    if !root_user {
        assert!(workspace.write("ro/doc.psd", b"new").is_err());
        assert_eq!(std::fs::read(ro.join("doc.psd")).unwrap(), b"original");
        assert!(temp_files(&ro).is_empty());
    }
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn rejects_absolute_traversal_mixed_prefix_and_malformed_paths_without_panicking() {
    let (_, _, workspace) = roots("reject");
    for path in
        ["", "/absolute", "../escape", "a/../escape", "a\\..\\escape", "a\\b", "C:/escape", "a//b", "./a", "name:stream", "NUL", "con.txt", "folder/COM1.log"]
    {
        assert!(workspace.read(path).is_err(), "read accepted {path:?}");
        assert!(workspace.write(path, b"x").is_err(), "write accepted {path:?}");
    }
}

#[test]
fn read_and_write_authority_are_separate() {
    let (inside, _, _) = roots("separate");
    std::fs::write(inside.join("read.txt"), b"canary").unwrap();
    let read_only = AuthorizedWorkspace::new(Some(&inside), None).unwrap();
    assert_eq!(read_only.read("read.txt").unwrap(), b"canary");
    assert!(read_only.write("blocked.txt", b"x").is_err());
    let write_only = AuthorizedWorkspace::new(None, Some(&inside)).unwrap();
    assert!(write_only.read("read.txt").is_err());
    write_only.write("written.txt", b"ok").unwrap();
}

#[test]
fn missing_parent_is_a_stable_error_before_any_file_is_created() {
    let (inside, _, workspace) = roots("missing-parent");
    let error = workspace.write("missing/output.txt", b"x").unwrap_err().to_string();
    assert!(error.contains("automation write"), "{error}");
    assert!(!inside.join("missing").exists());
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected() {
    use std::os::unix::fs::symlink;

    let (inside, outside, workspace) = roots("symlink");
    std::fs::write(outside.join("secret.txt"), b"outside").unwrap();
    symlink(&outside, inside.join("link")).unwrap();
    assert!(workspace.read("link/secret.txt").is_err());
    assert!(workspace.write("link/new.txt", b"blocked").is_err());
    assert!(!outside.join("new.txt").exists());
}

#[cfg(windows)]
#[test]
fn windows_symlink_escape_is_rejected_when_supported() {
    use std::os::windows::fs::symlink_dir;

    let (inside, outside, workspace) = roots("windows-link");
    std::fs::write(outside.join("secret.txt"), b"outside").unwrap();
    if symlink_dir(&outside, inside.join("link")).is_err() {
        return;
    }
    assert!(workspace.read("link/secret.txt").is_err());
    assert!(workspace.write("link/new.txt", b"blocked").is_err());
    assert!(!outside.join("new.txt").exists());
}

#[cfg(windows)]
#[test]
fn windows_junction_escape_is_rejected_when_supported() {
    use std::process::Command;

    let (inside, outside, workspace) = roots("windows-junction");
    std::fs::write(outside.join("secret.txt"), b"outside").unwrap();
    let link = inside.join("junction");
    let status = Command::new("cmd.exe").args(["/D", "/C", "mklink", "/J"]).arg(&link).arg(&outside).status();
    if !status.is_ok_and(|status| status.success()) {
        return;
    }
    assert!(workspace.read("junction/secret.txt").is_err());
    assert!(workspace.write("junction/new.txt", b"blocked").is_err());
    assert!(!outside.join("new.txt").exists());
    let _ = std::fs::remove_dir(link);
}

#[test]
fn filesystem_commands_fail_closed() {
    for id in [
        "file.open",
        "file.openAs",
        "file.save",
        "file.saveAs",
        "file.saveACopy",
        "file.export.saveForWebLegacy",
        "pattern.import",
        "layer.smartObjects.exportContents",
        "layer.smartObjects.replaceContents",
        "layer.smartObjects.relinkToFile",
        "layer.smartObjects.convertToLinked",
        "measurementLog.export",
        "layer.videoLayers.reloadFrame",
        "edit.colorSettings",
    ] {
        assert!(authorize_engine_command(id, &serde_json::json!({})).is_err());
    }
    assert!(authorize_engine_command("file.new", &serde_json::json!({})).is_ok());
    assert!(authorize_engine_command("file.automate.cropAndStraightenPhotos", &serde_json::json!({})).is_ok());
    assert!(authorize_desktop_engine_command("file.automate.cropAndStraightenPhotos", &serde_json::json!({})).is_ok());
    for id in ["layer.smartObjects.editContents", "layer.smartObjects.convertToLayers", "layer.smartObjects.saveContents"] {
        assert!(authorize_desktop_engine_command(id, &serde_json::json!({"layer": 19})).is_ok(), "{id}");
    }
    for id in ["layer.smartObjects.editContents", "layer.smartObjects.convertToLayers"] {
        assert!(authorize_engine_command(id, &serde_json::json!({"path": "/outside/source.psb"})).is_err(), "{id}");
    }
    // The UI-level examples in docs/control-protocol.md.
    for id in ["view.zoomIn", "window.theme.pro", "edit.search"] {
        assert!(authorize_desktop_engine_command(id, &serde_json::json!({})).is_ok(), "{id}");
    }
    assert!(authorize_engine_command("image.mode.cmyk", &serde_json::json!({})).is_ok());
    assert!(authorize_desktop_engine_command("image.mode.cmyk", &serde_json::json!({})).is_err());
    assert!(authorize_engine_command("image.mode.rgb", &serde_json::json!({"profile": "/outside/profile.icc"})).is_err());
    assert!(authorize_engine_command("image.mode.rgb", &serde_json::json!({"profile": "srgb"})).is_ok());
    assert!(authorize_engine_command("file.new", &serde_json::json!({"profile": "/outside/profile.icc"})).is_err());
    assert!(authorize_engine_command("file.new", &serde_json::json!({"profile": "adobe-rgb-compat"})).is_ok());
    assert!(authorize_engine_command("file.new", &serde_json::json!({"profile": "working"})).is_ok());
    assert!(authorize_engine_command("file.new", &serde_json::json!({"profile": "none"})).is_ok());
    assert!(authorize_engine_command("file.new", &serde_json::json!({"profile": "default"})).is_err());
    assert!(authorize_engine_command("file.new", &serde_json::json!({"profile": "working-cmyk"})).is_err());
    // Shortcut sets: text in and out is fine; a path reads or writes the filesystem.
    let ks = |p: serde_json::Value| authorize_engine_command("edit.keyboardShortcuts", &p);
    assert!(ks(serde_json::json!({"exportKys": true, "summarize": true, "importKys": "<photoshop-keyboard-shortcuts/>"})).is_ok());
    for key in ["importKys", "exportKys", "summarize"] {
        assert!(ks(serde_json::json!({ key: {"path": "/outside/set.kys"} })).is_err(), "{key}");
    }
    assert!(authorize_engine_command("filter.distort.displace", &serde_json::json!({"mapPath": "outside.png"})).is_err());
    assert!(authorize_engine_command("layer.setAdjustment", &serde_json::json!({"file": "outside.cube"})).is_err());
    assert!(authorize_engine_command("prefs.set", &serde_json::json!({"path": "colorSettings.workingRgb", "value": "outside.icc"})).is_err());
    assert!(authorize_engine_step("file.open", &serde_json::json!({})).is_err());
    assert!(authorize_engine_step("actions.play", &serde_json::json!({})).is_ok());
    // An allowed command can't reach a denied one by running it on its own behalf.
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", serde_json::json!({"width": 4, "height": 4})).unwrap();
    session.authorize = Some(authorize_desktop_engine_step);
    let params = serde_json::json!({"to": "grayscale"});
    assert!(authorize_desktop_engine_command("file.automate.conditionalModeChange", &params).is_ok());
    assert!(session.execute("file.automate.conditionalModeChange", params).is_err());
    assert_eq!(session.active().unwrap().doc.mode, photocraft_engine::doc::ColorMode::Rgb);
    assert!(authorize_desktop_engine_step("file.saveACopy", &serde_json::json!({})).is_err());
}

// Regression test for #3763: `edit.installedProfiles` scans the OS and user ICC folders but
// takes no path parameter, so automation let it through until it was listed as ambient.
#[test]
fn installed_profiles_listing_is_refused_for_automation() {
    for params in [serde_json::json!({}), serde_json::json!({"class": "output", "colorSpace": "cmyk"})] {
        assert!(authorize_engine_command("edit.installedProfiles", &params).is_err(), "authorize_engine_command allowed it with {params}");
        assert!(authorize_desktop_engine_command("edit.installedProfiles", &params).is_err(), "authorize_desktop_engine_command allowed it with {params}");
        assert!(authorize_engine_step("edit.installedProfiles", &params).is_err(), "authorize_engine_step allowed it with {params}");
    }
    // The sandboxed session refuses it before the engine scans any folder.
    let mut headless = crate::Headless::new();
    let refused = headless.command_run("edit.installedProfiles", serde_json::json!({})).unwrap_err();
    assert!(refused.to_string().contains("ambient filesystem paths"), "{refused}");
}

#[test]
fn apply_data_set_is_judged_by_the_values_it_applies() {
    let mut headless = crate::Headless::new();
    headless.command_run("file.new", serde_json::json!({"width": 8, "height": 8})).unwrap();
    let layer = headless.command_run("layer.new.layer", serde_json::json!({})).unwrap()["layer"].clone();
    let defs = [
        serde_json::json!({"name": "shown", "layer": layer, "type": "visibility"}),
        serde_json::json!({"name": "photo", "layer": layer, "type": "pixelReplacement"}),
    ];
    headless.command_run("image.variables.define", serde_json::json!({"defs": defs})).unwrap();
    let sets = serde_json::json!({"dataSets": [
        {"name": "hidden", "values": [{"variable": "shown", "kind": "visibility", "value": false}]},
        {"name": "swap", "values": [
            {"variable": "shown", "kind": "visibility", "value": false},
            {"variable": "photo", "kind": "pixels", "value": "/outside/photo.png"},
        ]},
    ]});
    headless.command_run("image.variables.dataSets", sets).unwrap();
    let visible = |headless: &crate::Headless| headless.session.active().unwrap().doc.layers.iter().all(|l| l.visible);
    let refused = headless.command_run("image.applyDataSet", serde_json::json!({"name": "swap"})).unwrap_err();
    assert!(refused.to_string().contains("ambient filesystem paths"), "{refused}");
    assert!(visible(&headless), "a refused data set applies none of its values");
    assert!(headless.command_run("image.applyDataSet", serde_json::json!({"name": "hidden"})).is_ok());
    assert!(!visible(&headless));
}

#[test]
fn automation_rejects_ambient_path_commands_and_preferences() {
    for (id, params) in [
        ("brush.presets.importAbr", serde_json::json!({"path": "/outside/set.abr"})),
        ("brush.presets.importImage", serde_json::json!({"path": "/outside/tip.png"})),
        ("gradient.presets.importGrd", serde_json::json!({"path": "/outside/set.grd"})),
        ("plugin.install", serde_json::json!({"path": "/outside/plugin.wasm"})),
        ("plugin.reload", serde_json::json!({"path": "/outside/plugins"})),
        ("plugin.reload", serde_json::json!({})),
        ("plugin.install", serde_json::json!({"path": " "})),
        ("swatches.import", serde_json::json!({"path": "/outside/set.aco"})),
        ("swatches.export", serde_json::json!({"path": "/outside/set.ase"})),
        ("actions.load", serde_json::json!({"path": "/outside/set.atn"})),
        ("actions.save", serde_json::json!({"path": "/outside/set.atn"})),
        ("style.presets.importAsl", serde_json::json!({"path": "/outside/set.asl"})),
        ("style.presets.exportAsl", serde_json::json!({"path": "/outside/set.asl"})),
        ("style.presets.exportAsl", serde_json::json!({"path": 5})),
        ("layer.layerStyle.contours.load", serde_json::json!({"path": "/outside/set.shc"})),
        ("layer.layerStyle.contours.save", serde_json::json!({"path": "/outside/set.shc"})),
    ] {
        assert!(authorize_engine_command(id, &params).is_err(), "{id}: {params}");
    }
    for (id, params) in [
        ("brush.presets.importAbr", serde_json::json!({"data": "QUJD"})),
        ("brush.presets.importImage", serde_json::json!({"data": "QUJD"})),
        ("gradient.presets.importGrd", serde_json::json!({"data": "QUJD"})),
        ("plugin.install", serde_json::json!({"data": "QUJD"})),
        ("swatches.import", serde_json::json!({"data": "QUJD"})),
        ("swatches.export", serde_json::json!({"format": "ase"})),
        ("actions.load", serde_json::json!({"data": "QUJD"})),
        ("actions.save", serde_json::json!({"set": "S"})),
        ("style.presets.importAsl", serde_json::json!({"data": "QUJD"})),
        ("style.presets.exportAsl", serde_json::json!({"group": "Basics"})),
        ("layer.layerStyle.contours.load", serde_json::json!({"data": "QUJD"})),
        ("layer.layerStyle.contours.save", serde_json::json!({"names": ["a"]})),
    ] {
        assert!(authorize_engine_command(id, &params).is_ok(), "{id}: {params}");
    }
    for path in [
        "",
        "colorSettings",
        "colorSettings.workingRgb",
        "scriptEvents",
        "scriptEvents.enabled",
        ".scriptEvents",
        ".scriptEvents.enabled",
        "historyLog.filePath",
        ".historyLog.filePath",
        "plugIns.additionalPluginsFolder",
        "fileHandling.lutFolder",
        "fileHandling",
        "scratchDisks.disks",
        ".",
        "..historyLog.filePath",
    ] {
        assert!(authorize_engine_command("prefs.set", &serde_json::json!({"path": path, "value": {}})).is_err(), "{path}");
    }
    assert!(
        authorize_engine_command("prefs.set", &serde_json::json!({"values": {"interface.language": "fr", "historyLog.filePath": "/outside/log"}})).is_err()
    );
    assert!(authorize_engine_command("prefs.set", &serde_json::json!({"path": "interface.language", "value": "fr"})).is_ok());
    assert!(authorize_engine_command("prefs.set", &serde_json::json!({"path": "fileHandling.autosave", "value": false})).is_ok());
}

#[test]
fn registry_filesystem_path_params_are_classified() {
    let probe = serde_json::json!({
        "path": "/outside/photocraft-probe",
        "file": "/outside/photocraft-probe.icc",
        "mapPath": "/outside/photocraft-probe.png",
        "profile": "/outside/photocraft-probe.icc",
        "workingRgb": "/outside/photocraft-probe.icc",
        "workingCmyk": "/outside/photocraft-probe.icc",
        "workingGray": "/outside/photocraft-probe.icc",
        "input": "/outside/photocraft-input",
        "output": "/outside/photocraft-output",
        "paths": ["/outside/photocraft-probe.psd"],
    });
    let preference_probe = serde_json::json!({"path": ".scriptEvents", "value": {}});
    let unclassified: Vec<_> = photocraft_engine::command_specs()
        .iter()
        .filter(|spec| documents_filesystem_path_params(spec.id, spec.params))
        .filter(|spec| {
            let params = if spec.id == "prefs.set" { &preference_probe } else { &probe };
            authorize_engine_command(spec.id, params).is_ok()
        })
        .map(|spec| spec.id)
        .collect();
    assert!(unclassified.is_empty(), "filesystem path parameters in the command registry need an automation policy: {unclassified:?}");
}

fn documents_filesystem_path_params(id: &str, params: &str) -> bool {
    // These commands return settings or installed-profile paths; they do not accept a host
    // filesystem path as an automation input.
    if matches!(id, "prefs.get" | "prefs.reset" | "edit.installedProfiles") {
        return false;
    }
    // These registry descriptions refer to document vector paths, not host files.
    if matches!(
        id,
        "filter.blurGallery.pathBlur"
            | "filter.render.flame"
            | "shape.create"
            | "shape.edit"
            | "shape.info"
            | "shape.presets.list"
            | "shape.presets.new"
            | "path.list"
            | "path.info"
            | "path.set"
            | "path.transform"
            | "path.moveAnchors"
            | "path.moveHandle"
            | "path.bendSegment"
            | "path.convertPoint"
            | "path.addAnchor"
            | "path.deleteAnchor"
            | "path.clippingPath.set"
            | "path.rename"
            | "path.stroke"
            | "type.attachToPath"
            | "type.fillPath"
            | "type.detachFromPath"
            | "select.toWorkPath"
            | "layer.vectorMask.add"
            | "layer.vectorMask.edit"
            | "layer.vectorMask.info"
            | "paint.symmetryFromPath"
            | "edit.defineCustomShape"
            | "layer.combineShapes.unite"
            | "layer.combineShapes.subtractFrontShape"
            | "layer.combineShapes.intersectShapeAreas"
            | "layer.combineShapes.excludeOverlappingShapes"
            | "layer.combineShapes.mergeShapeComponents"
    ) || id.starts_with("layer.newFillLayer.")
        || id.starts_with("layer.newAdjustmentLayer.")
    {
        // New fill and adjustment layers take a vector path as their vector mask (#1419).
        return false;
    }
    params.to_ascii_lowercase().contains("path")
}

#[test]
fn whole_preferences_update_cannot_enable_automation_script_events() {
    let mut headless = crate::headless::Headless::new();
    let result = headless.command_run(
        "prefs.set",
        serde_json::json!({
            "path": "",
            "value": {
                "scriptEvents": {
                    "enabled": true,
                    "bindings": [{
                        "event": "newDocument",
                        "steps": [["file.saveACopy", {"path": "/outside/canary.psd"}]]
                    }]
                }
            }
        }),
    );
    assert!(result.is_err());

    for (id, params) in [
        ("brush.presets.importAbr", serde_json::json!({"path": "/outside/set.abr"})),
        ("gradient.presets.importGrd", serde_json::json!({"path": "/outside/set.grd"})),
        ("plugin.install", serde_json::json!({"path": "/outside/plugin.wasm"})),
        ("plugin.reload", serde_json::json!({"path": "/outside/plugins"})),
        ("style.presets.importAsl", serde_json::json!({"path": "/outside/set.asl"})),
        ("style.presets.exportAsl", serde_json::json!({"path": "/outside/set.asl"})),
        ("prefs.set", serde_json::json!({"path": "historyLog.filePath", "value": "/outside/log"})),
        ("prefs.set", serde_json::json!({"values": {"interface.language": "fr", "historyLog.filePath": "/outside/log"}})),
    ] {
        assert!(headless.command_run(id, params).is_err(), "{id}");
    }
    headless.command_run("file.new", serde_json::json!({"width": 5, "height": 5})).unwrap();
    assert!(headless.session.file_menu.event_log.is_empty());
}

/// #2176: an absolute path beneath the root names the same file as its relative path; every
/// other absolute path is refused before any file effect.
#[test]
fn absolute_paths_beneath_the_root_are_translated_and_others_refused() {
    let (inside, outside, workspace) = roots("absolute-paths");
    let abs = |p: &Path| p.to_string_lossy().into_owned();

    workspace.write(&abs(&inside.join("a.bin")), b"one").unwrap();
    assert_eq!(std::fs::read(inside.join("a.bin")).unwrap(), b"one");
    assert_eq!(workspace.read(&abs(&inside.join("a.bin"))).unwrap(), b"one");
    // Forward slashes too (on Windows the native spelling uses `\`).
    assert_eq!(workspace.read(&abs(&inside.join("a.bin")).replace('\\', "/")).unwrap(), b"one");
    std::fs::create_dir(inside.join("sub")).unwrap();
    workspace.write(&abs(&inside.join("sub").join("b.bin")), b"two").unwrap();
    assert_eq!(std::fs::read(inside.join("sub/b.bin")).unwrap(), b"two");
    assert_eq!(workspace.read("sub/b.bin").unwrap(), b"two", "relative paths still work");

    let refused = |path: String| {
        let error = workspace.write(&path, b"x").unwrap_err().to_string();
        assert!(error.contains("automation path rejected"), "{path}: {error}");
    };
    refused(abs(&outside.join("escape.bin")));
    refused(abs(&inside.join("..").join("outside").join("traversal.bin")));
    refused(abs(&inside));
    refused(abs(&inside.join("CON")));
    refused(format!("{}:hidden", abs(&inside.join("a.bin"))));
    // A sibling whose name merely starts with the root's name is not beneath it.
    let sibling = inside.with_file_name("inside-sibling");
    std::fs::create_dir_all(&sibling).unwrap();
    refused(abs(&sibling.join("lookalike.bin")));
    refused(format!("{}-sibling/lookalike.bin", abs(&inside)));
    // Climbing back out through a subdirectory is refused too.
    refused(abs(&inside.join("sub").join("..").join("..").join("outside").join("climb.bin")));
    assert!(!sibling.join("lookalike.bin").exists());
    assert!(!outside.join("climb.bin").exists());
    assert!(!outside.join("escape.bin").exists());
    assert!(!outside.join("traversal.bin").exists());
    assert_eq!(std::fs::read(inside.join("a.bin")).unwrap(), b"one");
}

/// Each authority is checked against its own root, and none granted refuses absolute paths.
#[test]
fn absolute_paths_are_checked_against_the_authority_s_own_root() {
    let (inside, outside, _) = roots("absolute-authority");
    let workspace = AuthorizedWorkspace::new(Some(&outside), Some(&inside)).unwrap();
    let target = inside.join("written.bin");
    workspace.write(&target.to_string_lossy(), b"w").unwrap();
    assert!(workspace.read(&target.to_string_lossy()).is_err(), "not under the read root");
    let none = AuthorizedWorkspace::default();
    assert!(none.read(&target.to_string_lossy()).is_err());
}
