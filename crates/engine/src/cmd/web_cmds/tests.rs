use super::*;
use photocraft_doc::{Layer, LayerContent};

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-web-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// A 64×48 document: white background, a red square with soft (half-transparent) edges on a
/// transparent layer, and a gradient band.
fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth, "name": "web.psd", "background": "transparent"})).unwrap();
    s.edit("paint", |doc, active| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster("Logo", fmt);
        let surf = l.surface_mut().unwrap();
        surf.fill_rect(Rect::new(8, 8, 32, 32), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        surf.fill_rect(Rect::new(32, 8, 34, 32), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 0.4]));
        for x in 0..64 {
            surf.fill_rect(Rect::new(x, 36, x + 1, 48), &photocraft_raster::from_rgba(&fmt, [x as f32 / 63.0, 0.5, 1.0 - x as f32 / 63.0, 1.0]));
        }
        let id = doc.insert_above(None, l);
        *active = Some(id);
        Ok(())
    })
    .unwrap();
    s
}

fn decode(path: &str) -> photocraft_codecs::Image {
    photocraft_codecs::decode(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn estimates_preserve_last_successful_export_settings() {
    let dir = tmp("estimate_settings");
    let mut s = session(8);
    let saved = json!({"format": "png24", "path": format!("{dir}/saved.png"), "width": 32, "height": 24});
    s.execute("file.export.saveForWebLegacy", saved.clone()).unwrap();
    assert_eq!(s.file_menu.last_web.as_ref(), Some(&saved));

    let estimate = s.execute("file.export.saveForWebLegacy", json!({"format": "jpeg", "quality": 10, "width": 16, "height": 12})).unwrap();
    assert_eq!((estimate["width"].as_u64(), estimate["height"].as_u64()), (Some(16), Some(12)));
    assert_eq!(s.file_menu.last_web.as_ref(), Some(&saved), "preview estimates must not replace the settings from the last successful export");
}

/// The next launch's dialog seeds from the preferences: `file_menu` is not saved, so a complete
/// export remembers its settings in `prefs.dialogs` too, as Fill and the filter dialogs do.
#[test]
fn a_complete_export_remembers_its_settings_in_the_preferences() {
    let dir = tmp("remember_settings");
    let mut s = session(8);
    s.execute("file.export.saveForWebLegacy", json!({"format": "jpeg", "quality": 42, "path": format!("{dir}/out.jpg")})).unwrap();
    let saved = s.prefs().dialogs.get("file.export.saveForWebLegacy").cloned().expect("remembered");
    assert_eq!(saved["format"], "jpeg");
    assert_eq!(saved["quality"], 42);
    assert!(saved.get("path").is_none(), "the destination is per export, not remembered");

    // Preview estimates remember nothing, like they don't replace `file_menu`.
    s.execute("file.export.saveForWebLegacy", json!({"format": "png24", "width": 16, "height": 12})).unwrap();
    assert_eq!(s.prefs().dialogs.get("file.export.saveForWebLegacy").unwrap()["format"], "jpeg");
}

#[test]
fn platform_writer_receives_resized_image_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let params = json!({"format": "png24", "path": "download.png", "width": 32, "height": 24});
        let mut files = Vec::new();
        let r = save_for_web_with_writer(&mut s, &params, &mut |path, bytes| {
            files.push((path.to_string(), bytes.to_vec()));
            Ok(())
        })
        .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "download.png");
        assert_eq!(r["files"], json!(["download.png"]));
        assert_eq!(r["bytes"].as_u64(), Some(files[0].1.len() as u64));
        let img = photocraft_codecs::decode(&files[0].1).unwrap().convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
        assert_eq!(img.dimensions(), (32, 24), "{depth}-bit document");
        let i = (8 * 32 + 8) * 4;
        assert_eq!(&img.data()[i..i + 4], &[255, 0, 0, 255]);
        assert_eq!(s.file_menu.last_web.as_ref(), Some(&params));
    }
}

#[test]
fn platform_writer_checks_authorization_before_dropping_floating_pixels() {
    let mut s = session(8);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
    s.execute("select.float", json!({"dx": 32})).unwrap();
    assert!(crate::float_cmds::floating(s.active().unwrap()).is_some());
    s.authorize = Some(|cmd, params| {
        assert_eq!(cmd, "file.export.saveForWebLegacy");
        assert_eq!(params["path"], "moved.png");
        Err(other("export denied"))
    });
    let params = json!({"format": "png24", "path": "moved.png"});
    let mut output = Vec::new();
    let denied = save_for_web_with_writer(&mut s, &params, &mut |_, bytes| {
        output.extend_from_slice(bytes);
        Ok(())
    });
    assert!(denied.unwrap_err().to_string().contains("export denied"));
    assert!(output.is_empty());
    assert!(s.file_menu.last_web.is_none());
    assert!(crate::float_cmds::floating(s.active().unwrap()).is_some(), "denied export must not commit a floating move");

    s.authorize = None;
    save_for_web_with_writer(&mut s, &params, &mut |_, bytes| {
        output.extend_from_slice(bytes);
        Ok(())
    })
    .unwrap();
    assert!(crate::float_cmds::floating(s.active().unwrap()).is_none());
    let img = photocraft_codecs::decode(&output).unwrap().convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
    let old = (10 * 64 + 10) * 4;
    let moved = (10 * 64 + 42) * 4;
    assert_eq!(img.data()[old + 3], 0, "the selected pixels left their original position");
    assert_eq!(&img.data()[moved..moved + 4], &[255, 0, 0, 255], "the download contains the moved pixels");
}

#[test]
fn platform_writer_receives_slices_spacer_and_html_with_sibling_references() {
    let mut s = session(8);
    s.execute("slice.new", json!({"rect": [8, 8, 24, 24], "name": "logo", "alt": "Logo"})).unwrap();
    s.execute("slice.new", json!({"rect": [40, 0, 24, 20], "kind": "noImage", "cellText": "Hello & bye"})).unwrap();
    let params = json!({"format": "png24", "dir": "downloads", "imagesFolder": "", "html": true});
    let mut files = std::collections::BTreeMap::new();
    let r = save_for_web_with_writer(&mut s, &params, &mut |path, bytes| {
        assert!(files.insert(path.to_string(), bytes.to_vec()).is_none(), "each file is written once");
        Ok(())
    })
    .unwrap();
    let page = std::str::from_utf8(files.get(r["html"].as_str().unwrap()).unwrap()).unwrap();
    let images = r["files"].as_array().unwrap();
    assert_eq!(files.len(), images.len() + 2, "image slices, spacer and HTML all reach the writer");
    for path in images {
        let path = path.as_str().unwrap();
        let filename = path.strip_prefix("downloads/").unwrap();
        assert!(page.contains(&format!("src=\"{filename}\"")), "the HTML refers to its sibling {filename}");
        assert!(photocraft_codecs::decode(files.get(path).unwrap()).is_ok());
    }
    assert!(page.contains("src=\"spacer.gif\""));
    assert!(page.contains("Hello &amp; bye"));
    assert_eq!(photocraft_codecs::decode(files.get("downloads/spacer.gif").unwrap()).unwrap().dimensions(), (1, 1));
    assert_eq!(photocraft_codecs::decode(files.get("downloads/logo.png").unwrap()).unwrap().dimensions(), (24, 24));
    assert_eq!(s.file_menu.last_web.as_ref(), Some(&params));
}

#[test]
fn reserved_document_and_slice_names_export_with_portable_names() {
    let dir = tmp("reserved-slice-names");
    let mut s = session(8);
    s.edit("rename", |doc, _| {
        doc.name = "aux.psd".into();
        Ok(())
    })
    .unwrap();
    s.execute("slice.new", json!({"rect": [0, 0, 16, 16], "name": "con"})).unwrap();
    let result = s.execute("file.export.saveForWebLegacy", json!({"dir": dir, "format": "png24", "html": true})).unwrap();
    assert!(result["html"].as_str().unwrap().ends_with("/aux_.html"));
    assert!(result["files"].as_array().unwrap().iter().any(|f| f.as_str().unwrap().ends_with("/con_.png")));
    assert!(std::path::Path::new(result["html"].as_str().unwrap()).is_file());
}

#[test]
fn a_dotted_device_file_name_exports_without_the_device_stem() {
    // `nul.v2_` would still be the NUL device on Windows; `nul_.v2` is a file.
    let dir = tmp("reserved-dotted-name");
    let mut s = session(8);
    s.execute("slice.new", json!({"rect": [0, 0, 16, 16], "name": "logo"})).unwrap();
    let path = format!("{dir}/nul.v2.png");
    let result = s.execute("file.export.saveForWebLegacy", json!({"path": path, "slices": "all", "format": "png24", "html": true})).unwrap();
    let html = result["html"].as_str().unwrap();
    assert!(html.ends_with("/nul_.v2.html"), "{html}");
    assert!(std::path::Path::new(html).is_file());
}

#[test]
fn platform_writer_failure_stops_export_and_preserves_saved_settings() {
    let mut s = session(8);
    let saved = json!({"format": "png24", "path": "saved.png"});
    save_for_web_with_writer(&mut s, &saved, &mut |_, _| Ok(())).unwrap();
    s.execute("slice.new", json!({"rect": [8, 8, 24, 24], "name": "logo"})).unwrap();
    let mut attempts = 0;
    let failed = save_for_web_with_writer(&mut s, &json!({"format": "gif", "dir": "downloads", "html": true}), &mut |_, _| {
        attempts += 1;
        if attempts == 2 { Err(other("download failed")) } else { Ok(()) }
    });
    assert!(failed.unwrap_err().to_string().contains("download failed"));
    assert_eq!(attempts, 2, "no later slice, spacer or HTML should be written after failure");
    assert_eq!(s.file_menu.last_web.as_ref(), Some(&saved));
}

#[test]
fn estimates_every_format_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let mut sizes = Vec::new();
        for f in ["gif", "png8", "png24", "jpeg", "wbmp", "webp"] {
            let r = s.execute("file.export.saveForWebLegacy", json!({"format": f, "colors": 16})).unwrap();
            assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(64), Some(48)), "{f}");
            assert!(r["bytes"].as_u64().unwrap() > 20, "{f}");
            if f == "gif" || f == "png8" {
                assert!(r["colors"].as_u64().unwrap() <= 16);
            }
            sizes.push(r["bytes"].as_u64().unwrap());
        }
        assert_eq!(sizes[4], 4 + 8 * 48, "WBMP: header plus 1 bit per pixel");
    }
    let mut s = session(8);
    assert!(s.execute("file.export.saveForWebLegacy", json!({"format": "tiff"})).is_err());
    let r = s.execute("file.export.saveForWebLegacy", json!({"preset": "JPEG Low", "percent": 50})).unwrap();
    assert_eq!(r["format"], "jpg");
    assert_eq!(r["width"], 32);
}

#[test]
fn gif_and_png8_keep_transparency_and_palette() {
    let dir = tmp("gif");
    let mut s = session(8);
    let gif = format!("{dir}/a.gif");
    let r = s.execute("file.export.saveForWebLegacy", json!({"format": "gif", "colors": 8, "dither": "none", "path": gif})).unwrap();
    assert_eq!(r["files"][0], json!(gif));
    let img = decode(&gif).convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
    let px = |x: usize, y: usize| img.data()[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4].to_vec();
    assert_eq!(px(0, 0)[3], 0, "transparent background");
    assert_eq!(px(16, 16), vec![255, 0, 0, 255]);
    // The 40 % edge is under ½: transparent, as Photoshop's hard GIF transparency.
    assert_eq!(px(33, 16)[3], 0);
    let mut colors: Vec<Vec<u8>> = (0..64 * 48).map(|i| img.data()[i * 4..i * 4 + 4].to_vec()).filter(|p| p[3] > 0).collect();
    colors.sort();
    colors.dedup();
    assert!(colors.len() <= 7, "8 colours including the transparent one, got {}", colors.len());
    // Transparency off: everything over the matte.
    let png8 = format!("{dir}/a.png");
    s.execute("file.export.saveForWebLegacy", json!({"format": "png8", "transparency": false, "matte": "#00ff00", "path": png8})).unwrap();
    let img = decode(&png8).convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
    assert_eq!(&img.data()[..4], &[0, 255, 0, 255]);
}

/// #4115: Photoshop's scripted Save for Web has no matte unless one is asked for, so a soft black
/// shadow exported to PNG-8 stays black (alpha only 0 or 255) instead of turning grey. A preset
/// keeps its white matte, and a matte asked for still applies.
#[test]
fn png8_has_no_matte_by_default() {
    let dir = tmp("matte");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 256, "height": 8, "background": "transparent"})).unwrap();
    s.edit("paint", |doc, active| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster("Shadow", fmt);
        let surf = l.surface_mut().unwrap();
        for x in 0..256 {
            surf.fill_rect(Rect::new(x, 0, x + 1, 8), &photocraft_raster::from_rgba(&fmt, [0.0, 0.0, 0.0, 1.0 - x as f32 / 255.0]));
        }
        *active = Some(doc.insert_above(None, l));
        Ok(())
    })
    .unwrap();
    let export = |s: &mut Session, extra: Value| {
        let path = format!("{dir}/{}.png", s.journal.len());
        let mut p = json!({"format": "png8", "path": path});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        s.execute("file.export.saveForWebLegacy", p).unwrap();
        let img = decode(&path).convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
        (0..256).map(|x| img.data()[x * 4..x * 4 + 4].to_vec()).collect::<Vec<_>>()
    };
    let plain = export(&mut s, json!({}));
    assert!(plain.iter().all(|p| p == &[0, 0, 0, 255] || p[3] == 0), "black or transparent: {:?}", &plain[60..70]);
    assert_eq!(plain[64], [0, 0, 0, 255], "alpha 191 stays black");
    assert_eq!(plain[200][3], 0);
    // A matte asked for, or a preset's, lightens the soft pixels as before.
    let white = export(&mut s, json!({"matte": "#ffffff"}));
    assert!(white[64][0] > 40 && white[64][3] == 255, "{:?}", white[64]);
    let preset = WebSettings::from_params(&json!({"preset": "PNG-8 128 Dithered"}), "x").unwrap();
    assert_eq!(preset.matte, Some([1.0, 1.0, 1.0]));
}

#[test]
fn jpeg_quality_progressive_and_metadata() {
    let dir = tmp("jpeg");
    let mut s = session(16);
    s.execute("file.fileInfo", json!({"copyright": "© Example", "author": "A. Person", "description": "secret notes"})).unwrap();
    let lo = format!("{dir}/lo.jpg");
    let hi = format!("{dir}/hi.jpg");
    s.execute("file.export.saveForWebLegacy", json!({"format": "jpeg", "quality": 10, "path": lo, "metadata": "none"})).unwrap();
    s.execute("file.export.saveForWebLegacy", json!({"format": "jpeg", "quality": 95, "progressive": true, "path": hi, "metadata": "copyright"})).unwrap();
    let (a, b) = (std::fs::read(&lo).unwrap(), std::fs::read(&hi).unwrap());
    assert!(a.len() < b.len());
    assert!(b.windows(2).any(|w| w == [0xFF, 0xC2]), "progressive");
    let xmp = decode(&hi).meta.xmp.unwrap_or_default();
    assert!(xmp.contains("Example"), "copyright kept");
    assert!(!xmp.contains("secret"), "description dropped");
    assert!(decode(&lo).meta.xmp.is_none());
}

/// WebP (#2415, #6391): lossy at the chosen quality or lossless, keeping translucency unless
/// transparency is off, with the chosen metadata and an optional colour profile.
#[test]
fn webp_lossy_lossless_transparency_and_metadata() {
    let dir = tmp("webp");
    let mut s = session(16);
    s.execute("file.fileInfo", json!({"copyright": "© Example", "description": "secret notes"})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "srgb"})).unwrap();
    let path = |name: &str| format!("{dir}/{name}");
    let mut export = |name: &str, p: Value| {
        let mut p = p;
        p["format"] = json!("webp");
        p["path"] = json!(path(name));
        s.execute("file.export.saveForWebLegacy", p).unwrap();
        std::fs::read(path(name)).unwrap()
    };
    let lo = export("lo.webp", json!({"quality": 5}));
    let hi = export("hi.webp", json!({"quality": 95, "embedIcc": true}));
    let lossless = export("lossless.webp", json!({"lossless": true, "metadata": "none"}));
    let opaque = export("opaque.webp", json!({"transparency": false, "matte": "#00ff00"}));
    for bytes in [&lo, &hi, &lossless, &opaque] {
        assert_eq!((&bytes[0..4], &bytes[8..12]), (&b"RIFF"[..], &b"WEBP"[..]));
    }
    assert!(lo.len() < hi.len(), "quality drives the size: {} vs {}", lo.len(), hi.len());
    let has = |b: &[u8], fourcc: &[u8]| b.windows(4).any(|w| w == fourcc);
    assert!(has(&hi, b"VP8 ") && has(&hi, b"ALPH"), "lossy keeps translucency in an alpha chunk");
    assert!(has(&lossless, b"VP8L") && !has(&lossless, b"VP8 "));
    assert!(!has(&opaque, b"ALPH"), "transparency off composites over the matte");

    // Lossless is the PNG-24 export's pixels exactly.
    s.execute("file.export.saveForWebLegacy", json!({"format": "png24", "path": path("same.png")})).unwrap();
    let (a, b) = (decode(&path("lossless.webp")), decode(&path("same.png")));
    let rgba = |i: &photocraft_codecs::Image| i.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8).data().to_vec();
    assert_eq!(rgba(&a), rgba(&b));
    // The transparent corner stays transparent; with transparency off it is the matte.
    assert_eq!(rgba(&a)[3], 0);
    let o = rgba(&decode(&path("opaque.webp")));
    assert!(o[3] == 255 && o[1] > 200 && o[0] < 60, "{:?}", &o[..4]);

    let meta = decode(&path("hi.webp"));
    let xmp = meta.meta.xmp.unwrap_or_default();
    assert!(xmp.contains("Example") && !xmp.contains("secret"), "copyright only");
    assert!(meta.icc.is_some(), "Embed Color Profile");
    assert!(decode(&path("lo.webp")).icc.is_none());
    assert!(a.meta.xmp.is_none(), "metadata none");
}

/// Sizes WebP cannot hold are an error, not a crash.
#[test]
fn webp_too_large_is_an_error() {
    let mut s = session(8);
    for lossless in [false, true] {
        let r = s.execute("file.export.saveForWebLegacy", json!({"format": "webp", "lossless": lossless, "width": 20000, "height": 2}));
        assert!(r.is_err(), "lossless {lossless}: {r:?}");
    }
}

#[test]
fn platform_writer_checks_settings_before_dropping_floating_pixels() {
    let mut s = session(8);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
    s.execute("select.float", json!({"dx": 32})).unwrap();
    let rev = s.active().unwrap().revision;
    let params = json!({"format": "webp", "path": "moved.webp", "lossless": "yes"});
    let mut output = Vec::new();
    let refused = save_for_web_with_writer(&mut s, &params, &mut |_, bytes| {
        output.extend_from_slice(bytes);
        Ok(())
    });
    assert!(refused.unwrap_err().to_string().contains("lossless"));
    assert!(output.is_empty());
    assert!(s.file_menu.last_web.is_none());
    assert_eq!(s.active().unwrap().revision, rev, "the document is untouched");
    assert!(crate::float_cmds::floating(s.active().unwrap()).is_some(), "a refused export must not commit a floating move");
}

/// An on/off option that isn't true or false is refused (`"lossless": "true"` would otherwise
/// write lossy WebP), and the refused export writes and remembers nothing.
#[test]
fn non_boolean_flags_are_refused_without_changing_anything() {
    let dir = tmp("flags");
    let mut s = session(8);
    let out = format!("{dir}/out.webp");
    let flags = ["transparency", "interlaced", "progressive", "lossless", "optimized", "embedIcc", "convertToSrgb"];
    for k in flags {
        for v in [json!("true"), json!(1), json!([true])] {
            let rev = s.active().unwrap().revision;
            let mut p = json!({"format": "webp", "path": out});
            p[k] = v.clone();
            let err = s.execute("file.export.saveForWebLegacy", p).unwrap_err().to_string();
            assert!(err.contains(k), "{k}: {v}: {err}");
            assert_eq!(s.active().unwrap().revision, rev, "{k}: the document is untouched");
            assert!(!std::path::Path::new(&out).exists(), "{k}: nothing written");
            assert!(s.file_menu.last_web.is_none(), "{k}: nothing remembered");
        }
    }
    // null is the same as leaving it out.
    s.execute("file.export.saveForWebLegacy", json!({"format": "webp", "path": out, "lossless": null})).unwrap();
    assert!(std::path::Path::new(&out).exists());
}

#[test]
fn slices_export_with_html_table() {
    let dir = tmp("slices");
    let mut s = session(8);
    s.execute("slice.new", json!({"rect": [8, 8, 24, 24], "name": "logo", "url": "https://example.org/", "alt": "Logo"})).unwrap();
    s.execute("slice.new", json!({"rect": [40, 0, 24, 20], "kind": "noImage", "cellText": "Hello & bye"})).unwrap();
    let r = s.execute("file.export.saveForWebLegacy", json!({"format": "png24", "dir": dir, "html": true})).unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let n = photocraft_doc::slices::resolve(&s.active().unwrap().doc).len();
    assert_eq!(files.len(), n - 1, "every slice but the no-image one");
    let logo = files.iter().find(|f| f.ends_with("images/logo.png")).unwrap();
    assert_eq!(decode(logo).dimensions(), (24, 24));
    let html = std::fs::read_to_string(r["html"].as_str().unwrap()).unwrap();
    assert!(html.contains("<a href=\"https://example.org/\">"));
    assert!(html.contains("alt=\"Logo\""));
    assert!(html.contains("Hello &amp; bye"));
    assert!(html.contains("images/spacer.gif"));
    assert!(std::path::Path::new(&format!("{dir}/images/spacer.gif")).is_file());
    // Total area of the exported slices plus the text cell is the canvas.
    let area: u64 = files
        .iter()
        .map(|f| {
            let (w, h) = decode(f).dimensions();
            u64::from(w) * u64::from(h)
        })
        .sum();
    assert_eq!(area + 24 * 20, 64 * 48);
    // Scaled export scales slices.
    let dir2 = tmp("slices2");
    let r = s.execute("file.export.saveForWebLegacy", json!({"format": "gif", "dir": dir2, "percent": 50, "slices": "user"})).unwrap();
    assert_eq!(r["files"].as_array().unwrap().len(), 1);
    assert_eq!(decode(r["files"][0].as_str().unwrap()).dimensions(), (12, 12));
}

#[test]
fn slice_html_escapes_imported_markup_by_default_and_requires_trusted_opt_in() {
    let dir = tmp("slice-html-safety");
    let mut s = session(8);
    let slice = s
        .execute(
            "slice.new",
            json!({"rect": [0, 0, 24, 20], "kind": "noImage", "name": "text", "cellTextIsHtml": true,
            "cellText": "<script>alert(1)</script><b>Trusted label</b>"}),
        )
        .unwrap();
    let default = s.execute("file.export.saveForWebLegacy", json!({"format": "png24", "dir": dir, "html": true})).unwrap();
    let page = std::fs::read_to_string(default["html"].as_str().unwrap()).unwrap();
    assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!page.contains("<script>"));
    assert!(page.contains("&lt;b&gt;Trusted label&lt;/b&gt;"));

    let trusted_dir = tmp("slice-html-trusted");
    let trusted = s.execute("file.export.saveForWebLegacy", json!({"format": "png24", "dir": trusted_dir, "html": true, "trustedSliceHtml": true})).unwrap();
    let page = std::fs::read_to_string(trusted["html"].as_str().unwrap()).unwrap();
    assert!(page.contains("<script>alert(1)</script><b>Trusted label</b>"));
    assert!(s.file_menu.last_web.as_ref().unwrap().get("trustedSliceHtml").is_none());

    let plain_dir = tmp("slice-html-plain");
    s.execute("slice.set", json!({"slice": slice["slice"], "cellTextIsHtml": false})).unwrap();
    let plain = s.execute("file.export.saveForWebLegacy", json!({"format": "png24", "dir": plain_dir, "html": true, "trustedSliceHtml": true})).unwrap();
    let page = std::fs::read_to_string(plain["html"].as_str().unwrap()).unwrap();
    assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
}

#[test]
fn slice_links_allow_normal_urls_and_drop_active_schemes() {
    let dir = tmp("slice-link-safety");
    let mut s = session(8);
    for (x, url) in [
        (0, "javascript:alert(1)"),
        (10, "data:text/html,<script>alert(1)</script>"),
        (20, "https://example.org/page?a=1&b=2"),
        (30, "../relative/page.html#part"),
        (40, "mailto:hello@example.org"),
        (50, "JaVa\tscript:alert(1)"),
    ] {
        s.execute("slice.new", json!({"rect": [x, 0, 10, 10], "name": format!("link{x}"), "url": url})).unwrap();
    }
    let r = s.execute("file.export.saveForWebLegacy", json!({"format": "png24", "dir": dir, "html": true, "slices": "user"})).unwrap();
    let page = std::fs::read_to_string(r["html"].as_str().unwrap()).unwrap();
    assert!(!page.contains("href=\"javascript:"));
    assert!(!page.contains("href=\"data:"));
    assert!(!page.contains("href=\"JaVa"));
    assert!(page.contains("href=\"https://example.org/page?a=1&amp;b=2\""));
    assert!(page.contains("href=\"../relative/page.html#part\""));
    assert!(page.contains("href=\"mailto:hello@example.org\""));
    assert_eq!(page.matches("<a href=").count(), 3);
}

#[test]
fn images_folder_stays_inside_export_directory_and_is_escaped_in_html() {
    let root = tmp("slice-image-folder");
    let dir = format!("{root}/output");
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = session(8);
    s.execute(
        "slice.new",
        json!({"rect": [0, 0, 20, 20], "name": "logo", "url": "https://example.org", "target": "x\" onclick=\"alert(1)", "alt": "x\" onerror=\"alert(1)"}),
    )
    .unwrap();
    for images in ["../escaped", "..\\escaped", "/absolute", "C:\\absolute", "nested/../../escaped", "/"] {
        assert!(s.execute("file.export.saveForWebLegacy", json!({"format": "gif", "dir": dir, "html": true, "imagesFolder": images})).is_err(), "{images}");
    }
    assert!(!std::path::Path::new(&format!("{root}/escaped")).exists());
    // Windows forbids quotes in path components; an ampersand still exercises HTML escaping.
    #[cfg(windows)]
    let (folder, escaped_folder) = ("nested/im&files", "nested/im&amp;files");
    #[cfg(not(windows))]
    let (folder, escaped_folder) = ("nested/im\" onerror=\"alert(1)&", "nested/im&quot; onerror=&quot;alert(1)&amp;");
    let r = s.execute("file.export.saveForWebLegacy", json!({"format": "gif", "dir": dir, "html": true, "imagesFolder": folder})).unwrap();
    let page = std::fs::read_to_string(r["html"].as_str().unwrap()).unwrap();
    assert!(page.contains(&format!("src=\"{escaped_folder}/logo.gif\"")));
    assert!(page.contains(&format!("src=\"{escaped_folder}/spacer.gif\"")));
    assert!(page.contains("alt=\"x&quot; onerror=&quot;alert(1)\""));
    assert!(!page.contains("onerror=\"alert(1)\""));
    assert!(!page.contains("onclick=\"alert(1)\""));
    assert!(std::path::Path::new(&dir).join(folder).join("spacer.gif").is_file());
}

#[cfg(unix)]
#[test]
fn images_folder_rejects_existing_symlink_outside_export_directory() {
    let dir = tmp("slice-image-symlink");
    let outside = tmp("slice-image-outside");
    std::os::unix::fs::symlink(&outside, format!("{dir}/linked")).unwrap();
    let mut s = session(8);
    s.execute("slice.new", json!({"rect": [0, 0, 20, 20], "name": "logo"})).unwrap();
    assert!(s.execute("file.export.saveForWebLegacy", json!({"format": "gif", "dir": dir, "html": true, "imagesFolder": "linked"})).is_err());
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
}

#[test]
fn export_preferences_drive_quick_export() {
    let dir = tmp("quick");
    let mut s = session(8);
    let r = s.execute("file.export.exportPreferences", json!({"quickExportFormat": "jpg", "jpegQuality": 40, "quickExportLocation": "sameFolder"})).unwrap();
    assert_eq!(r["values"]["quickExportFormat"], "jpg");
    assert!(s.execute("file.export.exportPreferences", json!({"quickExportFormat": "bmp"})).is_err());
    // Unsaved document: a path is needed.
    assert!(s.execute("file.export.quickExport", json!({})).is_err());
    s.active_mut().unwrap().path = Some(format!("{dir}/web.psd"));
    let r = s.execute("file.export.quickExport", json!({})).unwrap();
    assert_eq!(r["path"], json!(format!("{dir}/web.jpg")));
    assert_eq!(decode(&format!("{dir}/web.jpg")).dimensions(), (64, 48));
    s.execute("file.export.exportPreferences", json!({"quickExportFormat": "gif"})).unwrap();
    let r = s.execute("file.export.quickExport", json!({"path": format!("{dir}/x.gif")})).unwrap();
    assert_eq!(r["format"], "gif");
    assert!(std::fs::read(format!("{dir}/x.gif")).unwrap().starts_with(b"GIF89a"));
    s.execute("prefs.reset", json!({"path": "export"})).unwrap();
}

/// #6599: with Location = Same Folder, a document opened from `photo.png` would be exported over
/// `photo.png` itself. Without a `path` that is refused and the file is left alone; named
/// explicitly (the save dialog's answer) it is written.
#[test]
fn quick_export_never_replaces_the_documents_own_file_unasked() {
    let dir = tmp("quick_own_file");
    let mut s = session(8);
    s.execute("file.export.exportPreferences", json!({"quickExportLocation": "sameFolder", "quickExportFormat": "png"})).unwrap();
    let own = format!("{dir}/photo.png");
    std::fs::write(&own, b"original").unwrap();
    s.active_mut().unwrap().path = Some(own.clone());
    assert_eq!(quick_export_target(&s), Some(own.clone()));
    assert!(quick_export_target_is_own_file(&s));
    let e = s.execute("file.export.quickExport", json!({})).unwrap_err().to_string();
    assert!(e.contains("own file"), "{e}");
    assert_eq!(std::fs::read(&own).unwrap(), b"original", "the source is untouched");
    s.execute("file.export.quickExport", json!({"path": own})).unwrap();
    assert_eq!(decode(&own).dimensions(), (64, 48));
    // Another format, or a PSD's earlier export beside it, is still written without asking.
    s.execute("file.export.exportPreferences", json!({"quickExportFormat": "jpg"})).unwrap();
    assert!(!quick_export_target_is_own_file(&s));
    assert_eq!(s.execute("file.export.quickExport", json!({})).unwrap()["path"], json!(format!("{dir}/photo.jpg")));
    s.execute("file.export.exportPreferences", json!({"quickExportFormat": "png"})).unwrap();
    s.active_mut().unwrap().path = Some(format!("{dir}/photo.psd"));
    assert!(!quick_export_target_is_own_file(&s));
    assert_eq!(s.execute("file.export.quickExport", json!({})).unwrap()["path"], json!(own));
    s.execute("prefs.reset", json!({"path": "export"})).unwrap();
}

/// #8519: Quick Export as PNG renamed to `photo.jpeg` in the save dialog wrote PNG bytes under a
/// JPEG name, which upload checks refuse. A Quick Export extension picks the format; another
/// image extension is refused; no extension keeps Export Preferences' format.
#[test]
fn quick_export_writes_the_format_its_path_names() {
    let dir = tmp("quick_ext");
    let mut s = session(8);
    s.execute("file.export.exportPreferences", json!({"quickExportFormat": "png"})).unwrap();
    for (name, format, magic) in [
        ("a.jpeg", "jpg", &b"\xFF\xD8\xFF"[..]),
        ("b.JPG", "jpg", b"\xFF\xD8\xFF"),
        ("c.gif", "gif", b"GIF8"),
        ("d.webp", "webp", b"RIFF"),
        ("e", "png", b"\x89PNG"),
    ] {
        let path = format!("{dir}/{name}");
        let r = s.execute("file.export.quickExport", json!({"path": path})).unwrap();
        assert_eq!(r["format"], json!(format), "{name}");
        assert!(std::fs::read(&path).unwrap().starts_with(magic), "{name} holds {format} data");
    }
    let tif = format!("{dir}/f.tif");
    let e = s.execute("file.export.quickExport", json!({"path": tif})).unwrap_err().to_string();
    assert!(e.contains("Export As"), "{e}");
    assert!(!std::path::Path::new(&tif).exists());
    s.execute("prefs.reset", json!({"path": "export"})).unwrap();
}

#[test]
fn same_file_sees_through_other_names_for_one_file() {
    let dir = tmp("same_file");
    let (a, b, c) = (format!("{dir}/a.png"), format!("{dir}/b.png"), format!("{dir}/c.png"));
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(&c, b"c").unwrap();
    assert!(crate::file_cmds::same_file(&a, &a));
    assert!(crate::file_cmds::same_file(&a, format!("{dir}/./a.png")));
    assert!(!crate::file_cmds::same_file(&a, &c));
    assert!(!crate::file_cmds::same_file(&a, &b), "a missing file is not the same file");
    std::fs::hard_link(&a, &b).unwrap();
    assert!(crate::file_cmds::same_file(&a, &b), "a hard link is the same file");
    #[cfg(windows)]
    assert!(crate::file_cmds::same_file(&a, format!("{dir}/A.PNG")), "NTFS names ignore case");
}

#[test]
fn webp_export_preferences_select_lossless_or_lossy_output() {
    let dir = tmp("quick_webp_quality");
    let mut s = session(8);
    let defaults = s.prefs().export.clone();
    assert!(defaults.webp_lossless, "existing Quick Export defaults to lossless WebP");
    assert_eq!(defaults.webp_quality, 85);

    // Simulate an older stored preference section without the newly added WebP keys.
    let mut old = s.prefs().to_json();
    old["export"].as_object_mut().unwrap().remove("webpLossless");
    old["export"].as_object_mut().unwrap().remove("webpQuality");
    let restored: crate::prefs::Preferences = serde_json::from_value(old).unwrap();
    assert!(restored.export.webp_lossless);
    assert_eq!(restored.export.webp_quality, 85);

    let r = s.execute("file.export.exportPreferences", json!({"quickExportFormat": "webp", "webpLossless": true, "webpQuality": 40})).unwrap();
    assert_eq!(r["values"]["webpLossless"], true);
    assert_eq!(r["values"]["webpQuality"], 40);
    let lossless_path = format!("{dir}/lossless.webp");
    s.execute("file.export.quickExport", json!({"path": lossless_path})).unwrap();
    let lossless = std::fs::read(&lossless_path).unwrap();
    assert!(lossless.starts_with(b"RIFF") && &lossless[8..12] == b"WEBP");
    assert!(lossless.windows(4).any(|w| w == b"VP8L"), "lossless WebP should use VP8L");
    assert_eq!(decode(&lossless_path).dimensions(), (64, 48));

    s.execute("file.export.exportPreferences", json!({"webpLossless": false})).unwrap();
    let low_path = format!("{dir}/lossy40.webp");
    s.execute("file.export.quickExport", json!({"path": low_path})).unwrap();
    let low = std::fs::read(&low_path).unwrap();
    assert!(low.windows(4).any(|w| w == b"VP8 "), "lossy WebP should contain a VP8 frame");
    assert_eq!(decode(&low_path).dimensions(), (64, 48));

    s.execute("file.export.exportPreferences", json!({"webpQuality": 90})).unwrap();
    let high_path = format!("{dir}/lossy90.webp");
    s.execute("file.export.quickExport", json!({"path": high_path})).unwrap();
    let high = std::fs::read(&high_path).unwrap();
    assert!(high.windows(4).any(|w| w == b"VP8 "));
    assert_eq!(decode(&high_path).dimensions(), (64, 48));
    assert_ne!(low, high, "changing WebP quality should change the encoded image");

    let before = s.prefs().export.webp_quality;
    for quality in [0, 101] {
        assert!(s.execute("file.export.exportPreferences", json!({"webpQuality": quality})).is_err());
        assert_eq!(s.prefs().export.webp_quality, before);
    }
}

#[test]
fn generator_spaced_size_keeps_only_the_file_name() {
    for (name, w, h, file) in [
        ("2 x 2 2 icon.png", 2.0, 2.0, "2 icon.png"),
        ("1in x 1in in.png", 72.0, 72.0, "in.png"),
        ("2px x 1px x.png", 2.0, 1.0, "x.png"),
        ("2in  x  1in   label name.png", 144.0, 72.0, "label name.png"),
    ] {
        let a = parse_asset_name(name, 72.0);
        assert_eq!(a.len(), 1, "{name}");
        assert_eq!((a[0].width, a[0].height, a[0].file.as_str()), (Some(w), Some(h), file), "{name}");
    }
}

#[test]
fn generator_naming_grammar() {
    let a = parse_asset_name("200% foo@2x.png, 48x48 icons/bar.png8 + photo.jpg8", 72.0);
    assert_eq!(a.len(), 3);
    assert_eq!((a[0].file.as_str(), a[0].format.as_str(), a[0].scale), ("foo@2x.png", "png32", Some(2.0)));
    assert_eq!((a[1].file.as_str(), a[1].format.as_str(), a[1].width, a[1].height), ("icons/bar.png", "png8", Some(48.0), Some(48.0)));
    assert_eq!((a[2].file.as_str(), a[2].quality), ("photo.jpg", Some(80)));
    let b = parse_asset_name("?x100 thumb.jpg50%", 72.0);
    assert_eq!((b[0].width, b[0].height, b[0].quality), (None, Some(100.0), Some(50)));
    let c = parse_asset_name("1in x 2cm label.gif", 300.0);
    assert_eq!(c[0].width, Some(300.0));
    assert!((c[0].height.unwrap() - 236.22).abs() < 0.01);
    assert!(parse_asset_name("Layer 1", 72.0).is_empty());
    assert!(parse_asset_name("notes.txt", 72.0).is_empty());
    assert!(parse_asset_name("x.jpgq", 72.0).is_empty());
    let d = parse_defaults("default 50% low/ + 200% @2x", 72.0).unwrap();
    assert_eq!(
        d,
        vec![
            AssetDefault { scale: Some(0.5), folder: "low/".into(), ..Default::default() },
            AssetDefault { scale: Some(2.0), suffix: "@2x".into(), ..Default::default() }
        ]
    );
    assert!(parse_defaults("defaults are nice", 72.0).is_none());
}

#[test]
fn image_assets_needs_the_generator_preference() {
    let dir = tmp("generator_pref");
    let mut s = session(8);
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.renameLayer", json!({"layer": id.0, "name": "logo.png"})).unwrap();
    assert!(!s.prefs().plug_ins.generator_enabled, "off by default, as in Photoshop");
    assert!(!s.is_enabled("file.generate.imageAssets"), "unavailable until it is enabled");
    assert!(s.execute("file.generate.imageAssets", json!({"dir": dir})).is_err());

    s.execute("prefs.set", json!({"path": "plugIns.generatorEnabled", "value": true})).unwrap();
    assert!(s.is_enabled("file.generate.imageAssets"));
    assert_eq!(s.execute("file.generate.imageAssets", json!({"dir": dir})).unwrap()["enabled"], true);
    s.active_mut().unwrap().path = Some(format!("{dir}/doc.psd"));
    assert!(crate::automate_cmds::document_saved(&mut s, 0).is_some(), "assets are written on save");

    // Turning it off again takes the menu item away and stops the export, even for a document
    // left switched on.
    s.execute("prefs.set", json!({"path": "plugIns.generatorEnabled", "value": false})).unwrap();
    assert!(!s.is_enabled("file.generate.imageAssets"));
    assert!(crate::automate_cmds::document_saved(&mut s, 0).is_none(), "no assets while Generator is off");
}

#[test]
fn image_assets_are_generated_now_and_on_save() {
    let dir = tmp("assets");
    let mut s = session(8);
    // Preferences ▸ Plug-ins ▸ Enable Generator: the feature is opt-in.
    s.execute("prefs.set", json!({"path": "plugIns.generatorEnabled", "value": true})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.renameLayer", json!({"layer": id.0, "name": "logo.png, 50% small/logo.gif, logo.jpg80%"})).unwrap();
    let r = s.execute("file.generate.imageAssets", json!({"dir": dir})).unwrap();
    assert_eq!(r["enabled"], true);
    assert_eq!(r["files"].as_array().unwrap().len(), 3, "{r}");
    // Trimmed to the layer's pixels (x 0..64, y 8..48).
    assert_eq!(decode(&format!("{dir}/logo.png")).dimensions(), (64, 40));
    assert_eq!(decode(&format!("{dir}/small/logo.gif")).dimensions(), (32, 20));
    assert_eq!(decode(&format!("{dir}/logo.jpg")).dimensions(), (64, 40));
    // On save: written to <name>-assets next to the file.
    let saved = format!("{dir}/doc.psd");
    s.active_mut().unwrap().path = Some(saved.clone());
    let r = crate::automate_cmds::document_saved(&mut s, 0).unwrap();
    assert_eq!(r["dir"], json!(format!("{dir}/doc-assets")));
    assert!(std::path::Path::new(&format!("{dir}/doc-assets/logo.png")).is_file());
    // Off: nothing on save.
    assert_eq!(s.execute("file.generate.imageAssets", json!({})).unwrap()["enabled"], false);
    assert!(crate::automate_cmds::document_saved(&mut s, 0).is_none());
    // A default layer adds variants.
    let s2 = session(8);
    let doc = s2.active().unwrap().doc.clone();
    let mut d = (*doc).clone();
    d.layers.push(Layer::new(
        "default 200% @2x",
        LayerContent::Group(photocraft_doc::Group { children: vec![], expanded: false, artboard: None, frame_stroke: None }),
    ));
    let lid = d.layers[1].id;
    d.layer_mut(lid).unwrap().name = "icon.png".into();
    let (files, errors) = generate_assets(&d, &dir);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(files.iter().map(|p| std::path::PathBuf::from(p.as_str())).collect::<Vec<_>>(), vec![std::path::Path::new(&dir).join("icon@2x.png")]);
    assert_eq!(decode(&files[0]).dimensions(), (128, 80));
}

/// Photoshop's Generator exports an artboard named like an asset at the board's size, transparent
/// edges included (generator-assets renderer: the canvas bounds are the artboard's), with no
/// preference. Layers inside the board are still trimmed to their pixels.
#[test]
fn image_assets_keep_full_artboards() {
    let dir = tmp("asset-artboards");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "name": "boards.psd", "background": "transparent"})).unwrap();
    s.edit("board", |doc, _| {
        let fmt = doc.pixel_format();
        let mut logo = Layer::raster("logo.png", fmt);
        logo.surface_mut().unwrap().fill_rect(Rect::new(12, 10, 20, 16), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        let board =
            photocraft_doc::Artboard { rect: Rect::new(4, 6, 44, 36), background: photocraft_doc::ArtboardBackground::Transparent, preset: String::new() };
        let group = photocraft_doc::Group { children: vec![logo], expanded: true, artboard: Some(board), frame_stroke: None };
        doc.layers = vec![Layer::new("board.png", LayerContent::Group(group))];
        Ok(())
    })
    .unwrap();

    s.execute("prefs.set", json!({"path": "plugIns.generatorEnabled", "value": true})).unwrap();
    let r = s.execute("file.generate.imageAssets", json!({"on": true, "dir": dir})).unwrap();
    assert!(r["errors"].as_array().unwrap().is_empty(), "{r}");
    let board = decode(&format!("{dir}/board.png"));
    assert_eq!(board.dimensions(), (40, 30), "the whole board, not its trimmed pixels");
    let px = board.to_rgba8();
    let at = |x: usize, y: usize| &px[(y * 40 + x) * 4..(y * 40 + x) * 4 + 4];
    assert_eq!(at(0, 0)[3], 0, "the board's empty corner stays transparent");
    assert_eq!(at(39, 29)[3], 0);
    // The logo keeps its place on the board: document (12, 10) is board (8, 4).
    assert!(at(8, 4)[0] > 250 && at(8, 4)[3] == 255, "{:?}", at(8, 4));
    assert_eq!(at(7, 4)[3], 0);
    assert_eq!(decode(&format!("{dir}/logo.png")).dimensions(), (8, 6), "a layer on the board is still trimmed");
}

/// An artboard asset with nothing on the board (empty, or its contents all outside it) is
/// refused like an empty layer, not written as a blank board: Photoshop's Generator refuses a
/// zero-bounds pixmap before it pads to the board.
#[test]
fn image_assets_refuse_empty_artboards() {
    let dir = tmp("asset-empty-artboards");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "name": "boards.psd", "background": "transparent"})).unwrap();
    s.edit("boards", |doc, _| {
        let fmt = doc.pixel_format();
        let mut outside = Layer::raster("outside", fmt);
        outside.surface_mut().unwrap().fill_rect(Rect::new(50, 40, 60, 46), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        let board = |x0| photocraft_doc::Artboard {
            rect: Rect::new(x0, 0, x0 + 20, 20),
            background: photocraft_doc::ArtboardBackground::Transparent,
            preset: String::new(),
        };
        let empty = photocraft_doc::Group { children: vec![], expanded: true, artboard: Some(board(0)), frame_stroke: None };
        let off = photocraft_doc::Group { children: vec![outside], expanded: true, artboard: Some(board(24)), frame_stroke: None };
        doc.layers = vec![Layer::new("empty.png", LayerContent::Group(empty)), Layer::new("off.png", LayerContent::Group(off))];
        Ok(())
    })
    .unwrap();
    s.execute("prefs.set", json!({"path": "plugIns.generatorEnabled", "value": true})).unwrap();
    let r = s.execute("file.generate.imageAssets", json!({"on": true, "dir": dir})).unwrap();
    assert_eq!(r["files"], json!([]), "{r}");
    let errors = r["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2, "{r}");
    assert!(errors.iter().all(|e| e["error"].as_str().unwrap().contains("the layer is empty")), "{r}");
    assert!(!std::path::Path::new(&format!("{dir}/empty.png")).exists());
    assert!(!std::path::Path::new(&format!("{dir}/off.png")).exists());
}

/// #6614: two layers, or two default variants, naming one file never overwrite each other: the
/// layer higher in the Layers panel keeps the file, whatever the case of its name, and the
/// others are reported.
#[test]
fn image_assets_never_write_one_file_twice() {
    let dir = tmp("asset-collisions");
    let mut s = session(8);
    s.edit("paint", |doc, active| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster("Top", fmt);
        l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 4, 4), &photocraft_raster::from_rgba(&fmt, [0.0, 0.0, 1.0, 1.0]));
        let id = doc.insert_above(None, l);
        *active = Some(id);
        Ok(())
    })
    .unwrap();
    let mut d = (*s.active().unwrap().doc).clone();
    let id = |d: &photocraft_doc::Document, name: &str| d.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap().2.id;
    let (top, logo) = (id(&d, "Top"), id(&d, "Logo"));
    d.layer_mut(top).unwrap().name = "Logo.PNG".into();
    d.layer_mut(logo).unwrap().name = "logo.png, logo.png".into();
    let (files, errors) = generate_assets(&d, &dir);
    assert_eq!(files.iter().map(|p| std::path::PathBuf::from(p.as_str())).collect::<Vec<_>>(), vec![std::path::Path::new(&dir).join("Logo.png")]);
    assert_eq!(decode(&files[0]).dimensions(), (4, 4), "the top layer keeps the file");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors.iter().all(|e| e["error"].as_str().unwrap().contains("another layer already writes this file")), "{errors:?}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "one file, not a second one under another case");

    // Two default variants that land on one name: the first keeps it.
    let dir = tmp("asset-variant-collisions");
    d.layer_mut(top).unwrap().name = "icon.png".into();
    d.layer_mut(logo).unwrap().name = "Logo".into();
    d.layers.push(Layer::new(
        "default 50% + 200% + 200% @2x",
        LayerContent::Group(photocraft_doc::Group { children: vec![], expanded: false, artboard: None, frame_stroke: None }),
    ));
    let (files, errors) = generate_assets(&d, &dir);
    let names: Vec<_> = files.iter().map(|p| std::path::Path::new(p).file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["icon.png", "icon@2x.png"]);
    assert_eq!(decode(&files[0]).dimensions(), (2, 2), "the 50% variant keeps icon.png");
    assert_eq!(errors.len(), 1, "{errors:?}");
}

#[test]
fn image_assets_reject_traversal_from_names_and_defaults() {
    let base = tmp("asset-path-guard");
    let dir = format!("{base}/output");
    std::fs::create_dir_all(&dir).unwrap();
    let outside = tmp("asset-path-outside");
    let mut d = (*session(8).active().unwrap().doc).clone();
    let id = d.layers[1].id;
    d.layer_mut(id).unwrap().name =
        format!("nested/ok.png, ../escaped.png, ..\\backslash.png, {outside}/absolute.png, C:\\drive\\escape.png, CON.png, aux/escape.png, foo./trailing.png");
    let (files, errors) = generate_assets(&d, &dir);
    assert_eq!(files.iter().map(|p| std::path::PathBuf::from(p.as_str())).collect::<Vec<_>>(), vec![std::path::Path::new(&dir).join("nested").join("ok.png")]);
    assert_eq!(errors.len(), 7, "{errors:?}");
    assert!(errors.iter().all(|e| e["error"].as_str().unwrap().contains("unsafe component")), "{errors:?}");
    assert!(!std::path::Path::new(&format!("{base}/escaped.png")).exists());
    assert!(!std::path::Path::new(&format!("{outside}/absolute.png")).exists());

    d.layer_mut(id).unwrap().name = "safe.png".into();
    d.layers.push(Layer::new(
        "default ../ + /outside/ + ..\\bad/ + C:\\drive/ + 50% nested/",
        LayerContent::Group(photocraft_doc::Group { children: vec![], expanded: false, artboard: None, frame_stroke: None }),
    ));
    let (files, errors) = generate_assets(&d, &dir);
    assert_eq!(
        files.iter().map(|p| std::path::PathBuf::from(p.as_str())).collect::<Vec<_>>(),
        vec![std::path::Path::new(&dir).join("nested").join("safe.png")]
    );
    assert_eq!(errors.len(), 4, "{errors:?}");
    assert!(errors.iter().all(|e| e["error"].as_str().unwrap().contains("unsafe component")), "{errors:?}");
}

#[cfg(unix)]
#[test]
fn image_assets_reject_existing_symlinks_in_folders_and_files() {
    let dir = tmp("asset-symlink-guard");
    let outside = tmp("asset-symlink-outside");
    let sentinel = format!("{outside}/sentinel.png");
    std::fs::write(&sentinel, b"unchanged").unwrap();
    let webp_sentinel = format!("{outside}/sentinel.webp");
    std::fs::write(&webp_sentinel, b"also unchanged").unwrap();
    std::os::unix::fs::symlink(&outside, format!("{dir}/linked")).unwrap();
    std::os::unix::fs::symlink(&sentinel, format!("{dir}/leaf.png")).unwrap();
    std::os::unix::fs::symlink(&webp_sentinel, format!("{dir}/leaf.webp")).unwrap();
    let mut d = (*session(8).active().unwrap().doc).clone();
    let id = d.layers[1].id;
    d.layer_mut(id).unwrap().name = "linked/new/sub.png, leaf.png, leaf.webp, good/safe.png".into();
    let (files, errors) = generate_assets(&d, &dir);
    assert_eq!(files, vec![format!("{dir}/good/safe.png")]);
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(errors.iter().all(|e| e["error"].as_str().unwrap().contains("symlink")), "{errors:?}");
    assert!(!std::path::Path::new(&format!("{outside}/new")).exists());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"unchanged");
    assert_eq!(std::fs::read(&webp_sentinel).unwrap(), b"also unchanged");

    let linked_root = format!("{dir}/rootlink");
    std::os::unix::fs::symlink(&outside, &linked_root).unwrap();
    d.layer_mut(id).unwrap().name = "root.png".into();
    let (files, errors) = generate_assets(&d, &format!("{linked_root}/"));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(files, vec![format!("{linked_root}/root.png")]);
    assert!(std::path::Path::new(&format!("{outside}/root.png")).is_file());

    let mut s = session(8);
    let exported = s.execute("file.export.saveForWebLegacy", json!({"dir": linked_root, "format": "png24", "html": true})).unwrap();
    assert!(exported["html"].as_str().unwrap().ends_with("/rootlink/web.html"));
    assert!(std::path::Path::new(&format!("{outside}/web.html")).is_file());
}

#[test]
fn web_settings_from_params() {
    let st = WebSettings::from_params(&json!({"preset": "GIF 32 No Dither", "matte": "none", "webSnap": 50}), "x").unwrap();
    assert_eq!((st.format, st.colors, st.dither, st.matte), (WebFormat::Gif, 32, Dither::None, None));
    assert!((st.web_snap - 0.5).abs() < 1e-6);
    assert!(WebSettings::from_params(&json!({"preset": "Nope"}), "x").is_err());
    assert!(WebSettings::from_params(&json!({"palette": "weird"}), "x").is_err());
    let webp = WebSettings::from_params(&json!({"format": "webp", "lossless": true, "quality": 70}), "x").unwrap();
    assert_eq!((webp.format, webp.lossless, webp.quality), (WebFormat::Webp, true, 70));
    assert_eq!(WebSettings::from_params(&webp.to_params(), "x").unwrap(), webp);
    for p in PRESETS {
        let st = preset_settings(p).unwrap();
        assert_eq!(WebSettings::from_params(&st.to_params(), "x").unwrap(), st, "{p} round-trips through params");
    }
}

#[test]
fn previews_show_the_optimised_pixels() {
    let s = session(8);
    let doc = s.active().unwrap().doc.clone();
    let (wd, _, _) = web_document(&doc, &json!({}), &WebSettings::default()).unwrap();
    let buf = photocraft_compose::flatten(&wd);
    for f in ["jpeg", "gif", "png8", "png24", "wbmp", "webp"] {
        let st = WebSettings::from_params(&json!({"format": f, "quality": 90, "transparency": false}), "x").unwrap();
        let o = optimize(&buf.px, 64, wd.bounds(), &st, None, None, None, 72.0, true).unwrap();
        assert_eq!(o.preview.len(), 64 * 48 * 4, "{f}");
        // Red square centre stays red (WBMP: dark).
        let i = (16 * 64 + 16) * 4;
        let px = &o.preview[i..i + 4];
        if f == "wbmp" {
            assert_eq!(px[0], 0);
        } else {
            assert!(px[0] > 200 && px[1] < 60 && px[2] < 60, "{f}: {px:?}");
        }
        let none = optimize(&buf.px, 64, wd.bounds(), &st, None, None, None, 72.0, false).unwrap();
        assert!(none.preview.is_empty());
        assert_eq!(none.bytes, o.bytes);
    }
}

#[test]
fn jpeg_preview_at_odd_sizes() {
    for (w, h, q) in [(640usize, 431usize, 90), (640, 430, 60), (333, 222, 30)] {
        let px: Vec<[f32; 4]> = (0..w * h).map(|i| [((i % w) as f32 / w as f32), 0.3, ((i / w) as f32 / h as f32), 1.0]).collect();
        let st = WebSettings::from_params(&json!({"format": "jpeg", "quality": q}), "x").unwrap();
        let o = optimize(&px, w, Rect::new(0, 0, w as i32, h as i32), &st, None, None, None, 72.0, true).unwrap();
        for &(x, y) in &[(10usize, 10usize), (w - 10, h / 2), (w / 2, h - 5)] {
            let i = (y * w + x) * 4;
            let want = [(x as f32 / w as f32 * 255.0) as i32, 76, (y as f32 / h as f32 * 255.0) as i32];
            for c in 0..3 {
                assert!((i32::from(o.preview[i + c]) - want[c]).abs() < 12, "{w}x{h} at {x},{y}: {:?} vs {want:?}", &o.preview[i..i + 4]);
            }
        }
    }
}

#[test]
fn slices_with_the_same_name_get_distinct_files() {
    // #908: two slices named `tile` (and names that sanitize alike) used to overwrite each other.
    let dir = tmp("dupes");
    let mut s = session(8);
    s.execute("slice.new", json!({"rect": [0, 0, 10, 10], "name": "tile"})).unwrap();
    s.execute("slice.new", json!({"rect": [20, 0, 10, 10], "name": "tile"})).unwrap();
    s.execute("slice.new", json!({"rect": [40, 0, 10, 10], "name": "a/b"})).unwrap();
    s.execute("slice.new", json!({"rect": [0, 20, 10, 10], "name": "a?b"})).unwrap();
    s.execute("slice.new", json!({"rect": [20, 20, 10, 10], "name": "spacer"})).unwrap();
    let r = s.execute("file.export.saveForWebLegacy", json!({"format": "gif", "dir": dir, "slices": "user", "html": true})).unwrap();
    let mut files: Vec<String> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().rsplit('/').next().unwrap().to_string()).collect();
    files.sort();
    assert_eq!(files, ["a_b.gif", "a_b_2.gif", "spacer_2.gif", "tile.gif", "tile_2.gif"]);
    let html = std::fs::read_to_string(r["html"].as_str().unwrap()).unwrap();
    assert!(html.contains("images/tile.gif") && html.contains("images/tile_2.gif"));
}

#[test]
fn overlapping_slices_export_each_pixel_once() {
    // #7770: without HTML too, a slice covered by a later one is written as its visible pieces.
    let dir = tmp("overlap_images");
    let mut s = session(8);
    s.execute("slice.new", json!({"rect": [0, 0, 40, 30], "name": "under"})).unwrap();
    s.execute("slice.new", json!({"rect": [20, 15, 40, 30], "name": "over"})).unwrap();
    let r = s.execute("file.export.saveForWebLegacy", json!({"format": "png24", "dir": dir, "slices": "user"})).unwrap();
    let mut files: Vec<String> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let dims: Vec<(u32, u32)> = files.iter().map(|f| decode(f).dimensions()).collect();
    files.iter_mut().for_each(|f| *f = f.rsplit('/').next().unwrap().to_string());
    assert_eq!(files, ["under.png", "under-03.png", "over.png"]);
    assert_eq!(dims, [(40, 15), (20, 15), (40, 30)]);
}

#[test]
fn overlapping_slices_keep_the_later_slice_in_html() {
    // #909: the later slice is on top; the earlier one shows only where it isn't covered.
    let dir = tmp("overlap");
    let mut s = session(8);
    s.execute("slice.new", json!({"rect": [0, 0, 40, 30], "name": "under", "url": "https://example.org/under"})).unwrap();
    s.execute("slice.new", json!({"rect": [20, 15, 40, 30], "name": "over", "url": "https://example.org/over"})).unwrap();
    let r = s.execute("file.export.saveForWebLegacy", json!({"format": "png24", "dir": dir, "html": true})).unwrap();
    let html = std::fs::read_to_string(r["html"].as_str().unwrap()).unwrap();
    assert!(html.contains("https://example.org/over"), "the later slice's cell is in the table");
    assert!(html.contains("https://example.org/under"));
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let over = files.iter().find(|f| f.ends_with("images/over.png")).unwrap();
    assert_eq!(decode(over).dimensions(), (40, 30));
    // The visible pieces of every image tile the canvas exactly once.
    let area: u64 = files
        .iter()
        .filter(|f| !f.ends_with("spacer.gif"))
        .map(|f| {
            let (w, h) = decode(f).dimensions();
            u64::from(w) * u64::from(h)
        })
        .sum();
    assert_eq!(area, 64 * 48);
    // As Photoshop names subslices: the first piece keeps the name, the others add their number.
    let under = files.iter().find(|f| f.ends_with("images/under.png")).unwrap();
    assert_eq!(decode(under).dimensions(), (40, 15), "the covered slice is cut into pieces: {files:?}");
    assert!(files.iter().any(|f| f.ends_with("images/under-03.png")), "{files:?}");
}

#[path = "color_table_tests.rs"]
mod color_table_tests;

/// A little-endian camera EXIF: IFD0 with Make, Orientation 6, 300 ppi and pointers to an Exif
/// IFD (DateTimeOriginal) and a GPS IFD (GPSLatitudeRef); IFD0 links to an IFD1 thumbnail.
fn camera_exif() -> Vec<u8> {
    let entry = |v: &mut Vec<u8>, tag: u16, ty: u16, count: u32, value: u32| {
        v.extend_from_slice(&tag.to_le_bytes());
        v.extend_from_slice(&ty.to_le_bytes());
        v.extend_from_slice(&count.to_le_bytes());
        v.extend_from_slice(&value.to_le_bytes());
    };
    let (ifd0, data0) = (8u32, 8 + 2 + 7 * 12 + 4);
    let (make, xres, yres) = (data0, data0 + 10, data0 + 18);
    let exif_ifd = data0 + 26;
    let date = exif_ifd + 2 + 12 + 4;
    let gps_ifd = date + 20;
    let ifd1 = gps_ifd + 2 + 12 + 4;
    let mut v = b"II*\0".to_vec();
    v.extend_from_slice(&ifd0.to_le_bytes());
    v.extend_from_slice(&7u16.to_le_bytes());
    entry(&mut v, 0x010F, 2, 9, make);
    entry(&mut v, 0x0112, 3, 1, 6);
    entry(&mut v, 282, 5, 1, xres);
    entry(&mut v, 283, 5, 1, yres);
    entry(&mut v, 296, 3, 1, 2);
    entry(&mut v, 0x8769, 4, 1, exif_ifd);
    entry(&mut v, 0x8825, 4, 1, gps_ifd);
    v.extend_from_slice(&ifd1.to_le_bytes());
    v.extend_from_slice(b"TestMake\0\0");
    for _ in 0..2 {
        v.extend_from_slice(&300u32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
    }
    v.extend_from_slice(&1u16.to_le_bytes());
    entry(&mut v, 0x9003, 2, 20, date);
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(b"2024:05:01 10:30:00\0");
    v.extend_from_slice(&1u16.to_le_bytes());
    entry(&mut v, 0x0001, 2, 2, u32::from_le_bytes(*b"N\0\0\0"));
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    entry(&mut v, 0x0103, 3, 1, 6);
    v.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(v.len(), ifd1 as usize + 18);
    v
}

/// The payload (after `Exif\0\0`) of the file's EXIF APP1 segment, read from the markers.
fn exif_app1(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut i = 2;
    while i + 4 <= bytes.len() && bytes[i] == 0xFF && bytes[i + 1] != 0xDA {
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let body = &bytes[i + 4..i + 2 + len];
        if bytes[i + 1] == 0xE1 && body.starts_with(b"Exif\0\0") {
            return Some(body[6..].to_vec());
        }
        i += 2 + len;
    }
    None
}

/// IFD0's entries (tag → inline value) and its link to the next IFD, little-endian only.
fn ifd0(tiff: &[u8]) -> (std::collections::HashMap<u16, u32>, u32) {
    let u16_at = |o: usize| u16::from_le_bytes([tiff[o], tiff[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([tiff[o], tiff[o + 1], tiff[o + 2], tiff[o + 3]]);
    assert_eq!(&tiff[..4], b"II*\0");
    let at = u32_at(4) as usize;
    let n = u16_at(at) as usize;
    let tags = (0..n)
        .map(|k| {
            let e = at + 2 + k * 12;
            let value = if u16_at(e + 2) == 3 { u32::from(u16_at(e + 8)) } else { u32_at(e + 8) };
            (u16_at(e), value)
        })
        .collect();
    (tags, u32_at(at + 2 + n * 12))
}

fn photo_session(exif: Vec<u8>) -> Session {
    let mut s = session(8);
    s.execute("file.fileInfo", json!({"copyright": "© Example"})).unwrap();
    s.edit("exif", |doc, _| {
        doc.metadata.exif = Some(std::sync::Arc::new(exif));
        Ok(())
    })
    .unwrap();
    s
}

/// #7436: Save for Web's "All" writes the photo's EXIF, as Save a Copy does: upright
/// (Orientation 1), without the source's thumbnail and at the exported resolution. Every other
/// choice, "All Except Camera Info" included, leaves the camera data (and GPS) out.
#[test]
fn save_for_web_all_writes_the_photos_exif() {
    let dir = tmp("exif");
    let mut s = photo_session(camera_exif());
    let export = |s: &mut Session, metadata: &str| {
        let path = format!("{dir}/{metadata}.jpg");
        let r = s.execute("file.export.saveForWebLegacy", json!({"format": "jpeg", "metadata": metadata, "path": path})).unwrap();
        (std::fs::read(&path).unwrap(), r)
    };
    let (all, r) = export(&mut s, "all");
    let exif = exif_app1(&all).expect("an EXIF APP1 segment");
    let camera = photocraft_algo::exif::read(&exif);
    assert_eq!(camera.make.as_deref(), Some("TestMake"));
    assert_eq!(camera.date_taken.as_deref(), Some("2024:05:01 10:30:00"));
    let (tags, next) = ifd0(&exif);
    assert_eq!(tags.get(&0x0112), Some(&1), "the exported pixels are already upright");
    assert!(tags.contains_key(&0x8825), "GPS kept");
    assert_eq!(next, 0, "the source's thumbnail is not linked");
    assert_eq!(photocraft_codecs::exif_resolution(&exif), Some((72.0, 72.0)), "the document's resolution, not the source's 300");
    assert!(decode(&format!("{dir}/all.jpg")).meta.xmp.unwrap_or_default().contains("Example"));
    for m in ["none", "copyright", "copyrightAndContact", "allExceptCameraInfo"] {
        let (bytes, r) = export(&mut s, m);
        assert_eq!(exif_app1(&bytes), None, "{m}");
        assert_eq!(r["warnings"], json!([]), "{m}");
    }
    assert_eq!(r["warnings"], json!([]), "it fits");
    let xmp = decode(&format!("{dir}/allExceptCameraInfo.jpg")).meta.xmp.unwrap_or_default();
    assert!(xmp.contains("Example"), "all the XMP");
    assert_eq!(WebSettings::from_params(&json!({"metadata": "allExceptCameraInfo"}), "x").unwrap().to_params()["metadata"], "allExceptCameraInfo");
}

/// WebP holds EXIF in both modes: "All" keeps an EXIF-only photo's camera data (no XMP needed to
/// trigger the metadata), every other choice leaves it out.
#[test]
fn save_for_web_webp_all_keeps_an_exif_only_photos_exif() {
    let dir = tmp("webp_exif");
    let mut s = session(8);
    s.edit("exif", |doc, _| {
        doc.metadata.exif = Some(std::sync::Arc::new(camera_exif()));
        Ok(())
    })
    .unwrap();
    for lossless in [false, true] {
        for metadata in ["all", "none", "copyright", "allExceptCameraInfo"] {
            let path = format!("{dir}/{metadata}-{lossless}.webp");
            s.execute("file.export.saveForWebLegacy", json!({"format": "webp", "lossless": lossless, "metadata": metadata, "path": path})).unwrap();
            let meta = decode(&path).meta;
            if metadata == "all" {
                let exif = meta.exif.unwrap_or_else(|| panic!("EXIF kept (lossless {lossless})"));
                assert_eq!(photocraft_algo::exif::read(&exif).make.as_deref(), Some("TestMake"), "lossless {lossless}");
                assert!(meta.xmp.is_none(), "the photo had no XMP");
            } else {
                assert!(meta.exif.is_none(), "{metadata} leaves the camera data out (lossless {lossless})");
            }
        }
    }
}

#[test]
fn quick_export_all_writes_the_photos_exif() {
    let dir = tmp("quick_exif");
    let mut s = photo_session(camera_exif());
    let export = |s: &mut Session, metadata: &str| {
        s.execute("file.export.exportPreferences", json!({"quickExportFormat": "jpg", "metadata": metadata})).unwrap();
        let path = format!("{dir}/{metadata}.jpg");
        let r = s.execute("file.export.quickExport", json!({"path": path})).unwrap();
        (std::fs::read(&path).unwrap(), r)
    };
    let (all, r) = export(&mut s, "all");
    assert_eq!(photocraft_algo::exif::read(&exif_app1(&all).expect("EXIF")).make.as_deref(), Some("TestMake"));
    assert_eq!(r["warnings"], json!([]));
    assert_eq!(exif_app1(&export(&mut s, "copyright").0), None);
    assert_eq!(exif_app1(&export(&mut s, "none").0), None);
    s.execute("prefs.reset", json!({"path": "export"})).unwrap();
}

/// EXIF too large for its one segment is left out and said so; a truncated one doesn't panic.
#[test]
fn web_exif_that_does_not_fit_is_reported_and_bad_exif_is_harmless() {
    let dir = tmp("exif_bad");
    let mut big = camera_exif();
    big.resize(70_000, 0);
    let mut s = photo_session(big);
    let path = format!("{dir}/big.jpg");
    let r = s.execute("file.export.saveForWebLegacy", json!({"format": "jpeg", "metadata": "all", "path": path})).unwrap();
    assert_eq!(exif_app1(&std::fs::read(&path).unwrap()), None);
    assert!(r["warnings"][0].as_str().is_some_and(|w| w.contains("EXIF (70000 bytes)")), "{r}");
    s.execute("file.export.exportPreferences", json!({"quickExportFormat": "jpg", "metadata": "all"})).unwrap();
    let r = s.execute("file.export.quickExport", json!({"path": format!("{dir}/quick.jpg")})).unwrap();
    assert!(r["warnings"][0].as_str().is_some_and(|w| w.contains("EXIF")), "{r}");
    s.execute("prefs.reset", json!({"path": "export"})).unwrap();

    for cut in [3, 9, 40, 100] {
        let mut s = photo_session(camera_exif()[..cut].to_vec());
        let path = format!("{dir}/cut{cut}.jpg");
        s.execute("file.export.saveForWebLegacy", json!({"format": "jpeg", "metadata": "all", "path": path})).unwrap();
        assert_eq!(decode(&path).dimensions(), (64, 48));
    }
}

#[test]
fn quick_export_exr_preserves_linear_hdr_and_alpha() {
    use photocraft_codecs::{ChannelLayout, Format, Image, SampleType};
    let dir = tmp("quick_exr_hdr");
    let samples = [-0.25, 2.0, 0.125, 1.0, 4.0, 0.25, 0.5, 0.5];
    let image = Image::from_f32(2, 1, ChannelLayout::Rgba, &samples).unwrap();
    let bytes = photocraft_codecs::encode(&image, Format::OpenExr, &Default::default()).unwrap();
    let doc = photocraft_io::import("hdr.exr", &bytes).unwrap().document;
    let mut s = Session::new();
    s.add_document(doc, Some(format!("{dir}/source.psd")));
    for convert in [true, false] {
        s.execute("file.export.exportPreferences", json!({"quickExportFormat": "exr", "quickExportLocation": "sameFolder", "convertToSrgb": convert})).unwrap();
        let result = s.execute("file.export.quickExport", json!({})).unwrap();
        assert_eq!(result["format"], "exr");
        let path = format!("{dir}/source.exr");
        assert_eq!(result["path"], path);
        assert_eq!(result["bytes"].as_u64(), Some(std::fs::metadata(&path).unwrap().len()));
        let decoded = decode(&path);
        assert_eq!(decoded.sample_type(), SampleType::F32);
        let actual = decoded.convert(ChannelLayout::Rgba, SampleType::F32).to_f32_samples().unwrap();
        for (got, expected) in actual.iter().zip(samples) {
            assert!((got - expected).abs() < 0.0001, "EXR preserves scene values: {actual:?}");
        }
        assert_eq!(actual.len(), samples.len());
    }
    // The format preference must never silently overwrite the source EXR.
    let own = format!("{dir}/source.exr");
    s.active_mut().unwrap().path = Some(own.clone());
    let before = std::fs::read(&own).unwrap();
    assert!(s.execute("file.export.quickExport", json!({})).is_err());
    assert_eq!(std::fs::read(&own).unwrap(), before);
    s.execute("file.export.quickExport", json!({"path": format!("{dir}/explicit.exr")})).unwrap();
    assert_eq!(decode(&format!("{dir}/explicit.exr")).dimensions(), (2, 1));
}

/// #9325: an untagged document is treated as sRGB, and with Embed Color Profile on the file
/// says so, as Photoshop's Save for Web does; off, it still carries none.
#[test]
fn embed_color_profile_tags_an_untagged_document_srgb() {
    let mut s = session(8);
    assert!(s.active().unwrap().doc.icc_profile.is_none(), "a new document is untagged");
    let dir = tmp("assumed-icc");
    let profile = |path: &str| decode(path).icc.map(|b| photocraft_cms::Profile::parse(&b).unwrap().description);
    for (format, ext) in [("jpeg", "jpg"), ("png24", "png")] {
        let on = format!("{dir}/on.{ext}");
        s.execute("file.export.saveForWebLegacy", json!({"format": format, "embedIcc": true, "path": on})).unwrap();
        assert_eq!(profile(&on).as_deref(), Some(photocraft_cms::Builtin::Srgb.description()), "{format}");
        let off = format!("{dir}/off.{ext}");
        s.execute("file.export.saveForWebLegacy", json!({"format": format, "embedIcc": false, "path": off})).unwrap();
        assert_eq!(profile(&off), None, "{format}");
    }
}
