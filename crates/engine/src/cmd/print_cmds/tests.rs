use super::*;
use photocraft_doc::{Knot, NamedPath, Path, Subpath};

/// Compares signed-offset layout with actual PDF image operators and whole-page mirroring
/// across scale/fit/orientation cases, while checking the source document stays unchanged.
#[test]
fn negative_print_positions_match_pdf_without_changing_the_document() {
    let dir = tmp("negative-position");
    for (scale, fit, landscape, mirrored) in
        [(100.0, false, false, false), (175.0, false, true, false), (50.0, true, false, false), (100.0, false, false, true)]
    {
        let mut s = session("rgb", 8);
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let path = format!("{dir}/{scale}-{fit}-{landscape}-{mirrored}.pdf");
        let p = json!({"paper": "a4", "center": false, "left": -0.5, "top": -0.25,
            "scale": scale, "scaleToFit": fit, "orientation": if landscape {"landscape"} else {"portrait"},
            "emulsionDown": mirrored, "output": path, "send": false, "dryRun": true});
        let preview = layout(&before, &p, "file.print").unwrap();
        let (x, y, w, h) = preview.rect;
        assert_eq!(x, -36.0);
        assert!((preview.paper.1 - y - h + 18.0).abs() < 1e-9);
        assert!((w / h - 2.0).abs() < 1e-9);
        if !fit {
            assert!((w - 300.0 * scale / 100.0).abs() < 1e-9);
        } else {
            assert!((w - (preview.paper.0 - 36.0)).abs() < 1e-9);
        }
        let result = s.execute("file.print", p).unwrap();
        assert_eq!(result["sent"], false);
        assert_eq!(result["imageRect"], json!([x, y, w, h]));
        let pdf = std::fs::read(path).unwrap();
        let content = pdf_content(&pdf);
        assert!(content.contains(&format!("q {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Im0 Do Q")));
        assert_eq!(content.contains(&format!("q -1 0 0 1 {:.2} 0 cm", preview.paper.0)), mirrored);
        let after = s.active().unwrap();
        assert_eq!(after.revision, revision);
        assert_eq!(after.doc.size, before.size);
        assert_eq!(after.doc.resolution_dpi, before.resolution_dpi);
        assert_eq!(photocraft_compose::thumbnail(&before, 32).pixels, photocraft_compose::thumbnail(&after.doc, 32).pixels);
    }
}

/// Checks inclusive signed inch limits, malformed manual offsets and overflow-prone scale
/// or paper inputs at the shared layout boundary, without invoking a printer.
#[test]
fn print_position_and_geometry_bounds_reject_malformed_inputs() {
    let s = session("rgb", 8);
    let doc = &s.active().unwrap().doc;
    for key in ["top", "left"] {
        for value in [json!(-10000.0), json!(10000.0), json!(-0.125)] {
            let mut p = json!({"center": false});
            p[key] = value;
            assert!(layout(doc, &p, "file.print").is_ok());
        }
        for value in [json!(1e308), json!(-10001.0), json!("NaN"), Value::Null] {
            let mut p = json!({"center": false});
            p[key] = value;
            assert!(layout(doc, &p, "file.print").is_err());
        }
    }
    assert!(layout(doc, &json!({"scale": 1e308}), "file.print").is_err());
    assert!(layout(doc, &json!({"paper": [1e308, 792]}), "file.print").is_err());
}

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-print-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    // Spelt the way command results spell paths, so an expectation built with `/` matches on
    // Windows too. Win32 takes `/` as a separator, so the directory is still usable as one.
    d.to_string_lossy().replace('\\', "/")
}

fn session(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 150, "mode": mode, "depth": depth, "name": "Print Me.psd", "background": "#3366cc"})).unwrap();
    s
}

fn find(h: &[u8], n: &[u8], from: usize) -> usize {
    from + h[from..].windows(n.len()).position(|w| w == n).unwrap()
}

/// The (inflated) image stream of a print PDF: (width, height, colour space, samples).
fn pdf_image(pdf: &[u8]) -> (u32, u32, String, Vec<u8>) {
    let i = find(pdf, b"/Subtype /Image", 0);
    let end = find(pdf, b">>\nstream\n", i);
    let dict = String::from_utf8_lossy(&pdf[i..end]).into_owned();
    let num = |k: &str| dict.split(k).nth(1).unwrap().split_whitespace().next().unwrap().parse::<u32>().unwrap();
    let cs = dict.split("/ColorSpace ").nth(1).unwrap().split(" /BitsPerComponent").next().unwrap().to_string();
    let len = num("/Length ") as usize;
    let start = end + 10;
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&pdf[start..start + len]), &mut out).unwrap();
    (num("/Width "), num("/Height "), cs, out)
}

#[test]
fn print_dry_run_renders_a_pdf_and_reports_the_platform_backend() {
    let dir = tmp("print");
    for depth in [8, 16, 32] {
        let mut s = session("rgb", depth);
        let out = format!("{dir}/rgb-{depth}.pdf");
        let r = s
            .execute(
                "file.print",
                json!({"output": out, "send": true, "dryRun": true, "printer": "Office", "copies": 2, "paper": "a4", "cornerCropMarks": true, "registrationMarks": true, "labels": true}),
            )
            .unwrap();
        let cmd: Vec<String> = serde_json::from_value(r["command"].clone()).unwrap();
        #[cfg(unix)]
        {
            assert_eq!(&cmd[..5], &["lp", "-d", "Office", "-n", "2"]);
            assert!(cmd.contains(&"fit-to-page=false".into()));
            assert!(!cmd.contains(&out), "PDF bytes are streamed instead of passing a path");
        }
        #[cfg(windows)]
        assert_eq!(cmd.first().map(String::as_str), Some("xps-print"));
        assert_eq!(r["sent"], false);
        assert_eq!(r["pdf"], out);
        let pdf = std::fs::read(&out).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 595.28 841.89]"));
        let (w, h, cs, data) = pdf_image(&pdf);
        assert_eq!((w, h), (300, 150));
        assert_eq!(cs, "/DeviceRGB");
        assert_eq!(&data[..3], &[0x33, 0x66, 0xcc]);
        // 300 px at 72 ppi = 300 pt, centred.
        let rect: Vec<f64> = serde_json::from_value(r["imageRect"].clone()).unwrap();
        assert!((rect[2] - 300.0).abs() < 1e-6 && (rect[0] - (595.28 - 300.0) / 2.0).abs() < 1e-6);
    }
    // Print to PDF: no spooling by default.
    let mut s = session("gray", 8);
    let out = format!("{dir}/out.pdf");
    let r = s.execute("file.print", json!({"output": out, "scaleToFit": true, "orientation": "landscape"})).unwrap();
    assert_eq!(r["sent"], false);
    assert!(r["command"].is_null());
    let (_, _, cs, _) = pdf_image(&std::fs::read(&out).unwrap());
    assert_eq!(cs, "/DeviceGray");
    let scale = r["scale"].as_f64().unwrap();
    assert!(scale > 200.0, "fit to a landscape letter page: {scale}");
    assert!(s.execute("file.print", json!({"paper": "napkin", "dryRun": true})).is_err());
    assert!(s.execute("file.print", json!({"colorHandling": "photocraftManages", "dryRun": true})).is_err(), "needs a printer profile");
}

#[test]
fn photocraft_manages_colors_converts_to_the_printer_profile() {
    let mut s = session("rgb", 8);
    let out = format!("{}/managed.pdf", tmp("managed"));
    let r = s
        .execute(
            "file.print",
            json!({"output": out, "dryRun": true, "colorHandling": "photocraftManages", "printerProfile": "coated-cmyk", "intent": "perceptual", "bpc": false}),
        )
        .unwrap();
    assert_eq!(r["color"]["intent"], "perceptual");
    let pdf = std::fs::read(&out).unwrap();
    let (_, _, cs, data) = pdf_image(&pdf);
    assert_eq!(cs, "[/ICCBased 7 0 R]");
    assert!(String::from_utf8_lossy(&pdf).contains("/N 4 /Alternate /DeviceCMYK"));
    assert_eq!(data.len(), 300 * 150 * 4);
    // Blue: lots of cyan, little yellow.
    assert!(data[0] > 150 && data[2] < 60, "{:?}", &data[..4]);
    // The document itself is untouched.
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Rgb);
}

/// The inflated page content stream (object 4).
fn pdf_content(pdf: &[u8]) -> String {
    let i = find(pdf, b"4 0 obj\n", 0);
    let len_at = find(pdf, b"/Length ", i) + 8;
    let len: usize = String::from_utf8_lossy(&pdf[len_at..len_at + 12]).split_whitespace().next().unwrap().parse().unwrap();
    let start = find(pdf, b"stream\n", i) + 7;
    let mut out = String::new();
    std::io::Read::read_to_string(&mut flate2::read::ZlibDecoder::new(&pdf[start..start + len]), &mut out).unwrap();
    out
}

#[test]
fn emulsion_down_mirrors_the_whole_page() {
    let mut s = session("rgb", 8);
    let dir = tmp("emulsion");
    let plain_path = format!("{dir}/plain.pdf");
    let mirrored_path = format!("{dir}/mirrored.pdf");
    let again_path = format!("{dir}/again.pdf");
    let plain = s
        .execute("file.print", json!({"output": plain_path, "dryRun": true, "paper": "a4", "center": false, "left": 1.0, "top": 1.0, "labels": true}))
        .unwrap();
    let mirrored = s
        .execute(
            "file.print",
            json!({"output": mirrored_path, "dryRun": true, "paper": "a4", "center": false, "left": 1.0, "top": 1.0, "labels": true, "emulsionDown": true}),
        )
        .unwrap();
    assert_eq!(plain["emulsionDown"], false);
    assert_eq!(mirrored["emulsionDown"], true);
    // Same layout; only the page is flipped.
    assert_eq!(plain["imageRect"], mirrored["imageRect"]);
    let (p, m) = (std::fs::read(plain["pdf"].as_str().unwrap()).unwrap(), std::fs::read(mirrored["pdf"].as_str().unwrap()).unwrap());
    let (pc, mc) = (pdf_content(&p), pdf_content(&m));
    assert!(!pc.contains("-1 0 0 1"));
    // The flip wraps the image and the label, after the white paper, and is closed again.
    let flip = mc.find("q -1 0 0 1 595.28 0 cm").expect("mirror transform");
    assert!(flip > mc.find("re f Q").unwrap() && flip < mc.find("/Im0 Do").unwrap() && flip < mc.find("Tj").unwrap());
    assert_eq!(mc.matches("q ").count() + mc.matches("q\n").count(), mc.matches("Q\n").count(), "balanced q/Q: {mc}");
    // The pixels themselves are not touched (the transform mirrors them).
    assert_eq!(pdf_image(&p).3, pdf_image(&m).3);
    // Print One Copy keeps it.
    let again = s.execute("file.printOneCopy", json!({"output": again_path, "dryRun": true})).unwrap();
    assert_eq!(again["emulsionDown"], true);
    for r in [plain, mirrored, again] {
        let _ = std::fs::remove_file(r["pdf"].as_str().unwrap());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn page_of(channels: usize, data: Vec<u8>, emulsion_down: bool) -> PrintPage {
    PrintPage {
        paper: (600.0, 800.0),
        image: PrintImage { width: 2, height: 1, channels, data, icc: None },
        rect: (100.0, 200.0, 144.0, 72.0),
        marks: Marks::default(),
        description: None,
        label: None,
        emulsion_down,
    }
}

#[test]
fn windows_print_image_is_rgb_mirrored_for_emulsion_down() {
    assert_eq!(page_rgb(&page_of(3, vec![1, 2, 3, 4, 5, 6], false)).unwrap(), [1, 2, 3, 4, 5, 6]);
    assert_eq!(page_rgb(&page_of(3, vec![1, 2, 3, 4, 5, 6], true)).unwrap(), [4, 5, 6, 1, 2, 3]);
    assert_eq!(page_rgb(&page_of(1, vec![7, 9], true)).unwrap(), [9, 9, 9, 7, 7, 7]);
    assert!(page_rgb(&page_of(4, vec![0; 8], false)).is_err(), "CMYK can't go to a GDI printer");
    assert!(page_rgb(&page_of(3, vec![0; 5], false)).is_err(), "short data");
    // GDI's origin is the sheet's top-left; Emulsion Down moves the box to the mirrored side.
    assert_eq!(page_rect_top_left(&page_of(3, vec![0; 6], false)), (100.0, 528.0, 144.0, 72.0));
    assert_eq!(page_rect_top_left(&page_of(3, vec![0; 6], true)), (356.0, 528.0, 144.0, 72.0));
}

#[test]
fn native_rgb_route_preserves_pdf_only_color_and_page_content() {
    let mut page = page_of(3, vec![0; 6], false);
    assert!(native_rgb_compatible(&page, "printerManages"));
    page.label = Some("Print Me".into());
    assert!(!native_rgb_compatible(&page, "printerManages"));
    page.label = None;
    page.description = Some("Proof description".into());
    assert!(!native_rgb_compatible(&page, "printerManages"));
    page.description = Some(String::new());
    assert!(native_rgb_compatible(&page, "printerManages"));
    page.image.icc = Some(vec![1, 2, 3]);
    assert!(!native_rgb_compatible(&page, "printerManages"), "preserve embedded source ICC in the PDF route");
    assert!(native_rgb_compatible(&page, "photocraftManages"));
    assert!(native_rgb_compatible(&page, "photoshopManages"));
    assert!(!native_rgb_compatible(&page_of(4, vec![0; 8], false), "photocraftManages"));
    assert!(native_rgb_compatible(&page_of(1, vec![0; 2], false), "noColorManagement"));
}

/// Two printers, the second the default, in place of the system spooler's.
fn test_printers() -> PrinterSource {
    std::sync::Arc::new(|| {
        vec![
            craft_print::Printer { name: "Studio".into(), default: false, two_sided: None, description: String::new() },
            craft_print::Printer { name: "Office".into(), default: true, two_sided: Some(true), description: String::new() },
        ]
    })
}

#[test]
fn printer_commands_answer_without_a_document_and_reject_bad_settings() {
    let mut s = Session::new();
    s.printer_source = Some(test_printers());
    let r = s.execute("file.print.printers", json!({})).unwrap();
    assert_eq!(r["supported"], cfg!(windows));
    let listed = if cfg!(windows) { json!([{"name": "Studio", "default": false}, {"name": "Office", "default": true}]) } else { json!([]) };
    assert_eq!(r["printers"], listed);
    assert!(s.execute("file.print.page", json!({"printer": "Anything", "printerSettings": "zz"})).is_err());
    let mut s = session("rgb", 8);
    assert!(s.execute("file.print", json!({"printer": "Anything", "printerSettings": "0102", "send": true})).is_err());
}

/// The GDI route lists a stand-in's printers only on Windows. Elsewhere it answers `Unsupported`
/// as the real route does, so the Print dialog shows the system's printers and paper controls.
#[test]
fn stand_in_printers_reach_the_gdi_route_on_windows_only() {
    let source = test_printers();
    assert_eq!(stand_in_rgb_printers(&source, false), Err(craft_print::rgb::PrintError::Unsupported));
    let listed = stand_in_rgb_printers(&source, true).unwrap();
    assert_eq!(listed.iter().map(|p| (p.name.as_str(), p.default)).collect::<Vec<_>>(), [("Studio", false), ("Office", true)]);
}

/// `file.printers` lists a stand-in's printers everywhere but the browser. In the browser it lists
/// none, as the real command does there.
#[test]
fn stand_in_printers_reach_file_printers_outside_the_browser_only() {
    let source = test_printers();
    assert!(stand_in_printers(Some(&source), true).is_none());
    let listed = stand_in_printers(Some(&source), false).map(|source| source());
    assert_eq!(listed.unwrap_or_default().iter().map(|p| (p.name.as_str(), p.default)).collect::<Vec<_>>(), [("Studio", false), ("Office", true)]);
    assert!(stand_in_printers(None, false).is_none());
}

/// Synthetic driver settings: a DEVMODEW of `size` bytes plus `extra` private driver bytes, every
/// byte a pattern so a stray write shows, with `dmFields` saying only the paper size is set.
fn devmode(size: u16, extra: u16) -> Vec<u8> {
    let mut b: Vec<u8> = (0..usize::from(size) + usize::from(extra)).map(|i| (i % 251) as u8).collect();
    b[68..70].copy_from_slice(&size.to_le_bytes());
    b[70..72].copy_from_slice(&extra.to_le_bytes());
    b[72..76].copy_from_slice(&0x2u32.to_le_bytes()); // DM_PAPERSIZE
    b
}

/// Photoshop's Layout buttons: driver settings turned to landscape or portrait set `dmOrientation`
/// and its `dmFields` bit, and keep every other byte, the driver's private settings included.
#[test]
fn turned_settings_set_the_orientation_and_keep_the_rest() {
    for (size, extra) in [(220u16, 0u16), (220, 48)] {
        let before = devmode(size, extra);
        let dm = craft_print::rgb::DevMode::from_bytes(before.clone()).unwrap();
        for (landscape, orient) in [(true, 2i16), (false, 1i16)] {
            let after = turned_settings(&dm, landscape).unwrap().as_bytes().to_vec();
            assert_eq!(after.len(), before.len());
            assert_eq!(i16::from_le_bytes([after[76], after[77]]), orient, "{size}+{extra} landscape={landscape}");
            assert_eq!(u32::from_le_bytes([after[72], after[73], after[74], after[75]]), 0x3, "DM_ORIENTATION added, DM_PAPERSIZE kept");
            for (i, (a, b)) in after.iter().zip(&before).enumerate() {
                if !(72..76).contains(&i) && !(76..78).contains(&i) {
                    assert_eq!(a, b, "byte {i} changed");
                }
            }
        }
    }
}

/// The driver's paper is turned only when it is the layout's paper the other way round (within
/// the 2 pt a driver's paper may differ by): not for the same paper, another paper or a square one.
#[test]
fn only_the_same_paper_the_other_way_round_is_turned() {
    let (a3, a3_wide) = ((842.29, 1191.14), (1190.55, 841.89));
    assert!(turned_paper(a3, a3_wide), "portrait driver, landscape layout");
    assert!(turned_paper((1191.14, 842.29), (841.89, 1190.55)), "landscape driver, portrait layout");
    assert!(!turned_paper(a3, (841.89, 1190.55)), "same orientation");
    assert!(!turned_paper((595.32, 841.92), a3_wide), "an A4 driver for an A3 layout");
    assert!(!turned_paper((612.0, 612.0), (612.0, 612.0)), "square");
}

/// `file.print.page` takes an orientation and refuses any other value before asking a printer, so
/// the error is the same with or without one.
#[test]
fn printer_page_refuses_an_unknown_orientation() {
    let mut s = Session::new();
    for bad in [json!("sideways"), json!(2), json!(true)] {
        let e = s.execute("file.print.page", json!({"printer": "Anything", "orientation": bad})).unwrap_err().to_string();
        assert!(e.contains("orientation"), "{bad}: {e}");
    }
}

/// A real driver turns: Windows' "Microsoft Print to PDF" on its default (portrait) settings, for a
/// landscape A4 layout, gets settings turned to landscape and a wide page; a portrait layout keeps
/// the driver's own settings. Needs that printer, so it runs by hand:
/// `cargo test -p photocraft-engine --lib a_portrait_driver -- --ignored`.
#[cfg(windows)]
#[test]
#[ignore]
fn a_portrait_driver_turns_for_a_landscape_layout() {
    let printer = "Microsoft Print to PDF";
    let defaults = craft_print::rgb::default_settings(printer).unwrap();
    let a4 = craft_print::rgb::page(printer, Some(&defaults)).unwrap().paper;
    let (settings, page) = settings_for_layout(printer, None, (a4.1, a4.0)).unwrap();
    assert!(settings.is_some(), "turned settings come back");
    assert!((page.paper.0 - a4.1).abs() <= 2.0 && (page.paper.1 - a4.0).abs() <= 2.0, "wide: {:?}", page.paper);
    let (settings, page) = settings_for_layout(printer, None, a4).unwrap();
    assert!(settings.is_none(), "nothing to turn");
    assert!((page.paper.0 - a4.0).abs() <= 2.0, "{:?}", page.paper);
}

#[test]
fn printer_profile_can_be_an_installed_icc_file() {
    let dir = tmp("icc");
    let icc = format!("{dir}/My Paper.icc");
    let output = format!("{dir}/managed.pdf");
    std::fs::write(&icc, photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes().as_slice()).unwrap();
    let mut s = session("rgb", 16);
    let r = s
        .execute("file.print", json!({"output": output, "dryRun": true, "colorHandling": "photocraftManages", "printerProfile": icc, "emulsionDown": true}))
        .unwrap();
    assert_eq!(r["color"]["printerProfile"], json!(icc));
    let (_, _, cs, data) = pdf_image(&std::fs::read(r["pdf"].as_str().unwrap()).unwrap());
    assert_eq!(cs, "[/ICCBased 7 0 R]");
    assert_eq!(data.len(), 300 * 150 * 4);
    assert!(s.execute("file.print", json!({"dryRun": true, "colorHandling": "photocraftManages", "printerProfile": format!("{dir}/missing.icc")})).is_err());
    let _ = std::fs::remove_file(r["pdf"].as_str().unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

/// #7410: the settings of the last print survive a relaunch, as Photoshop's do. They are kept in
/// the preferences (without the output, dry-run and send choices), and a new session with those
/// preferences, before printing anything, repeats them with Print One Copy.
#[test]
fn the_last_print_settings_survive_a_relaunch() {
    let mut s = session("rgb", 8);
    assert!(last_print(&s).is_none());
    s.execute("file.print", json!({"dryRun": true, "copies": 3, "paper": "a5", "scale": 50, "intent": "perceptual"})).unwrap();
    let kept = s.prefs().dialogs.get(REMEMBERED).cloned().expect("kept in the preferences");
    assert_eq!((&kept["paper"], &kept["scale"], &kept["intent"]), (&json!("a5"), &json!(50), &json!("perceptual")));
    assert!(kept.get("dryRun").is_none() && kept.get("output").is_none() && kept.get("send").is_none(), "{kept}");
    // A relaunch: a new session with these preferences, nothing printed in it yet.
    let mut t = session("rgb", 8);
    t.prefs.edit(|p| {
        p.dialogs.insert(REMEMBERED.into(), kept.clone());
    });
    assert_eq!(last_print(&t), Some(kept));
    let r = t.execute("file.printOneCopy", json!({"dryRun": true})).unwrap();
    assert_eq!(r["copies"], 1);
    assert_eq!(r["paper"], json!([419.53, 595.28]));
    assert_eq!(r["scale"], 50.0);
}

/// #7410: a printer remembered from an earlier session that the print server no longer lists is
/// dropped before Print One Copy repeats the settings (with its driver settings), so the default
/// printer takes the job; a listed one (in any case) stays, and an empty list can't tell.
#[test]
fn a_vanished_remembered_printer_falls_back_to_the_default() {
    let settings = || json!({"printer": "Office", "printerSettings": "00ff", "paper": "a5"});
    let mut q = settings();
    drop_vanished_printer(&mut q, &["Studio", "Lab"]);
    assert_eq!(q, json!({"paper": "a5"}), "gone: the default printer takes it");
    for listed in [&["Studio", "office"][..], &[][..]] {
        let mut q = settings();
        drop_vanished_printer(&mut q, listed);
        assert_eq!(q, settings(), "{listed:?}");
    }
    let mut q = json!({"printer": "", "paper": "a5"});
    drop_vanished_printer(&mut q, &["Studio"]);
    assert_eq!(q, json!({"printer": "", "paper": "a5"}), "the default printer stays the default");
}

#[test]
fn print_one_copy_repeats_the_last_settings() {
    let mut s = session("rgb", 8);
    let first = s.execute("file.print", json!({"dryRun": true, "copies": 3, "paper": "a5", "scale": 50})).unwrap();
    assert!(first["pdf"].is_null());
    let r = s.execute("file.printOneCopy", json!({"dryRun": true})).unwrap();
    assert_eq!(r["copies"], 1);
    assert_eq!(r["paper"], json!([419.53, 595.28]));
    assert_eq!(r["scale"], 50.0);
    let cmd: Vec<String> = serde_json::from_value(r["command"].clone()).unwrap();
    #[cfg(unix)]
    assert_eq!(&cmd[..3], &["lp", "-n", "1"]);
    #[cfg(windows)]
    assert_eq!(cmd.first().map(String::as_str), Some("xps-print"));
    assert!(r["pdf"].is_null());
}

#[test]
fn package_copies_links_and_relinks() {
    let dir = tmp("package");
    let link = format!("{dir}/art.png");
    let mut src = Session::new();
    src.execute("file.new", json!({"width": 20, "height": 20, "background": "#ff0000"})).unwrap();
    crate::file_cmds::save_doc(&src.active().unwrap().doc, &link, None).unwrap();
    let mut s = session("rgb", 8);
    s.execute("file.placeLinked", json!({"path": link})).unwrap();
    let r = s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap();
    let copied = format!("{dir}/pkg/Print Me/Links/art.png");
    assert_eq!(r["links"], json!([copied]));
    assert!(std::path::Path::new(&copied).is_file());
    // Reported paths are `/`-spelt on every platform, not the native separator up to `dir` and
    // `/` after it: a client hands them back to the control channel, which takes only `/`.
    let docp = r["document"].as_str().unwrap();
    for p in [r["folder"].as_str().unwrap(), docp, r["links"][0].as_str().unwrap()] {
        assert!(!p.contains('\\'), "{p}");
    }
    assert!(docp.ends_with("Print Me/Print Me.pcraft"));
    let back = photocraft_io::import("Print Me.pcraft", &std::fs::read(docp).unwrap()).unwrap().document;
    let linked: Vec<String> = back
        .walk()
        .iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Smart(so) => match &so.source {
                SmartSource::Linked { path } => Some(path.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(linked.len(), 1);
    assert!(linked[0].ends_with("Links/art.png"), "{linked:?}");
    // The open document still points at the original.
    let open = &s.active().unwrap().doc;
    assert!(
        open.walk()
            .iter()
            .any(|(_, _, l)| matches!(&l.content, LayerContent::Smart(so) if matches!(&so.source, SmartSource::Linked { path } if *path == link)))
    );
}

/// Links whose names collide get distinct copies, even when a renamed copy's name is already
/// another link's name (`b/2-x.png` copied before the third `x.png` would be renamed to it).
#[test]
fn package_never_overwrites_one_link_copy_with_another() {
    let dir = tmp("package-collide");
    let mut s = session("rgb", 8);
    let mut links = Vec::new();
    for (sub, name, colour) in [("a", "x.png", "#ff0000"), ("b", "2-x.png", "#00ff00"), ("c", "x.png", "#0000ff")] {
        std::fs::create_dir_all(format!("{dir}/{sub}")).unwrap();
        let link = format!("{dir}/{sub}/{name}");
        let mut src = Session::new();
        src.execute("file.new", json!({"width": 4, "height": 4, "background": colour})).unwrap();
        crate::file_cmds::save_doc(&src.active().unwrap().doc, &link, None).unwrap();
        s.execute("file.placeLinked", json!({"path": link})).unwrap();
        links.push(std::fs::read(&link).unwrap());
    }
    let r = s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap();
    let copies: Vec<&str> = r["links"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(copies.len(), 3);
    let unique: std::collections::HashSet<&&str> = copies.iter().collect();
    assert_eq!(unique.len(), 3, "{copies:?}");
    for (copy, bytes) in copies.iter().zip(&links) {
        assert_eq!(&std::fs::read(copy).unwrap(), bytes, "{copy}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A second package into the same folder copies its links beside the first one's, never over them.
#[test]
fn package_keeps_files_already_in_links() {
    let dir = tmp("package-again");
    let link = format!("{dir}/art.png");
    let mut src = Session::new();
    src.execute("file.new", json!({"width": 4, "height": 4, "background": "#ff0000"})).unwrap();
    crate::file_cmds::save_doc(&src.active().unwrap().doc, &link, None).unwrap();
    let mut s = session("rgb", 8);
    s.execute("file.placeLinked", json!({"path": link})).unwrap();
    let first = s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap();
    let first = first["links"][0].as_str().unwrap().to_string();
    let kept = std::fs::read(&first).unwrap();
    let second = s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap();
    let second = second["links"][0].as_str().unwrap();
    assert_ne!(second, first);
    assert!(second.ends_with("Links/0-art.png"), "{second}");
    assert_eq!(std::fs::read(&first).unwrap(), kept);
    assert_eq!(std::fs::read(second).unwrap(), kept);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A missing link placed twice is reported once.
#[test]
fn package_lists_a_missing_link_once() {
    let dir = tmp("package-missing-once");
    let link = format!("{dir}/gone.png");
    let mut src = Session::new();
    src.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    crate::file_cmds::save_doc(&src.active().unwrap().doc, &link, None).unwrap();
    let mut s = session("rgb", 8);
    s.execute("file.placeLinked", json!({"path": link})).unwrap();
    s.execute("file.placeLinked", json!({"path": link})).unwrap();
    std::fs::remove_file(&link).unwrap();
    let r = s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap();
    assert_eq!(r["missing"], json!([link]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn paths_export_as_illustrator_postscript() {
    let mut s = session("rgb", 8);
    assert!(!s.is_enabled("file.export.pathsToIllustrator"));
    s.edit("paths", |d, _| {
        let tri = Subpath::polygon(&[(10.0, 10.0), (100.0, 10.0), (50.0, 90.0)]);
        let mut curve = Subpath::polyline(&[(0.0, 0.0), (300.0, 150.0)]);
        curve.knots[0] = Knot::smooth(photocraft_geom::Point::new(0.0, 0.0), photocraft_geom::Point::new(0.0, 0.0), photocraft_geom::Point::new(50.0, 0.0));
        d.paths.push(NamedPath { name: "Shapes".into(), path: Path::new(vec![tri, curve]), psd_raw: None });
        Ok(())
    })
    .unwrap();
    let r = s.execute("file.export.pathsToIllustrator", json!({})).unwrap();
    let ai = r["ai"].as_str().unwrap();
    assert!(ai.starts_with("%!PS-Adobe-2.0 EPSF-1.2"));
    assert!(ai.contains("%%BoundingBox: 0 0 300 150"));
    assert!(ai.contains("*u\n10.0000 140.0000 m\n100.0000 140.0000 L\n"), "{ai}");
    assert!(ai.contains("\nn\n") && ai.contains("\nN\n*U\n"));
    assert!(ai.contains(" C\n"), "curve segment");
    let dir = tmp("ai");
    let r = s.execute("file.export.pathsToIllustrator", json!({"path": format!("{dir}/p.ai"), "paths": "Shapes"})).unwrap();
    assert_eq!(r["paths"], 1);
    assert!(s.execute("file.export.pathsToIllustrator", json!({"paths": "Nope"})).is_err());
}

#[test]
fn print_dry_run_without_output_retains_no_pdf() {
    let mut s = session("rgb", 8);
    let result = s.execute("file.print", json!({"dryRun": true})).unwrap();
    assert!(result["pdf"].is_null());
    assert_eq!(result["sent"], false);
    assert!(result["bytes"].as_u64().unwrap() > 100);
    assert!(result["command"].is_array());
    assert!(s.file_menu.last_print.as_ref().unwrap().get("dryRun").is_none());
}

#[test]
fn print_without_sending_requires_output_unless_dry_run_and_forgets_send_override() {
    let mut s = session("rgb", 8);
    let before = s.file_menu.last_print.clone();
    let err = s.execute("file.print", json!({"send": false})).unwrap_err();
    assert!(err.to_string().contains("needs an output path"), "{err}");
    assert_eq!(s.file_menu.last_print, before);
    let result = s.execute("file.print", json!({"send": false, "dryRun": true})).unwrap();
    assert!(result["pdf"].is_null());
    assert!(result["command"].is_null());
    assert!(s.file_menu.last_print.as_ref().unwrap().get("send").is_none());
    let out = format!("{}/export.pdf", tmp("non-sending-export"));
    let result = s.execute("file.print", json!({"output": out, "send": false})).unwrap();
    assert_eq!(result["sent"], false);
    assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF-1.4"));
    assert!(s.file_menu.last_print.as_ref().unwrap().get("send").is_none());
    s.execute("file.print", json!({"output": out, "send": true, "dryRun": true})).unwrap();
    assert!(s.file_menu.last_print.as_ref().unwrap().get("send").is_none());
}
/// parallel tests are untouched, and the real spool boundary still consumes the PDF bytes.
#[cfg(unix)]
#[test]
fn spool_transport_streams_pdf_and_propagates_refusal() {
    const CHILD: &str = "PHOTOCRAFT_TEST_PRINT_TRANSPORT";
    if let Ok(dir) = std::env::var(CHILD) {
        let mut s = session("rgb", 8);
        let source = s.active().unwrap().doc.id;
        s.execute("file.scripts.scriptEventsManager", json!({"add": {"event": "print", "steps": [["image.imageRotation.90cw", {}]]}})).unwrap();
        let crate::jobs::Started::Job(job) = s.start("file.print", json!({"printer": "Office", "copies": 3, "paper": "a4", "scale": 50})).unwrap() else {
            panic!("desktop printing must start on a worker");
        };
        assert!(!s.cancel_job(job), "queue side effects must not be reported as cancelled");
        assert_eq!(s.execute("jobs.cancel", json!({})).unwrap()["cancelled"], 0);
        assert!(s.execute("jobs.cancel", json!({"job": job.0})).unwrap_err().to_string().contains("system print queue"));
        s.execute("file.new", json!({"width": 40, "height": 10})).unwrap();
        let viewed = s.active().unwrap().doc.id;
        let journal_before_completion = s.journal.len();
        let out = s.wait_job(job).unwrap();
        assert_eq!(s.journal.len(), journal_before_completion + 1, "print hooks must not become duplicate recorded action steps");
        assert_eq!(s.journal.last().unwrap().0, "file.print");
        assert_eq!(s.active().unwrap().doc.id, viewed, "completion keeps the user's tab");
        assert_eq!(s.active().unwrap().doc.size.width, 40, "print hook must not act on the newly viewed document");
        let printed = &s.documents().iter().find(|st| st.doc.id == source).unwrap().doc;
        assert_eq!((printed.size.width, printed.size.height), (150, 300), "print hook ran on the source document");
        assert_eq!(out["sent"], true);
        assert_eq!((out["spooler"].as_str(), out["backend"].as_str()), (Some("request id is Office-42"), Some("lp")));
        assert!(out["pdf"].is_null(), "spool-only printing leaves no diagnostic PDF");
        let pdf = std::fs::read(format!("{dir}/received.pdf")).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 595.28 841.89]"));
        let args = std::fs::read_to_string(format!("{dir}/args")).unwrap();
        assert!(args.contains("fit-to-page=false"));
        assert!(args.contains("-n\n3\n"));
        assert!(!args.contains(".pdf"));
        let remembered = s.file_menu.last_print.clone();
        let explicit = format!("{dir}/saved.pdf");
        let err = s.execute("file.print", json!({"printer": "Refuse", "output": explicit, "send": true})).unwrap_err();
        assert!(err.to_string().contains("queue refused"), "{err}");
        assert!(std::fs::read(&explicit).unwrap().starts_with(b"%PDF-1.4"), "explicit export survives a spool refusal");
        assert_eq!(s.file_menu.last_print, remembered, "failed printing does not replace repeat settings");
        // Export-only choices must not disable submission when Print One Copy repeats settings.
        let export = format!("{dir}/export-only.pdf");
        s.execute("file.print", json!({"printer": "Office", "output": export, "send": false})).unwrap();
        let remembered = s.file_menu.last_print.as_ref().unwrap();
        assert!(remembered.get("send").is_none() && remembered.get("output").is_none());
        let repeat = s.execute("file.printOneCopy", json!({})).unwrap();
        assert_eq!(repeat["sent"], true);
        assert_eq!(repeat["copies"], 1);
        assert!(repeat["pdf"].is_null());
        assert!(!std::fs::read_to_string(format!("{dir}/args")).unwrap().contains(".pdf"));
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("lp");
    std::fs::write(
        &script,
        b"#!/bin/sh\nif [ \"$2\" = Refuse ]; then echo 'queue refused' >&2; exit 1; fi\nprintf '%s\\n' \"$@\" > \"$PHOTOCRAFT_TEST_PRINT_TRANSPORT/args\"\n/bin/cat > \"$PHOTOCRAFT_TEST_PRINT_TRANSPORT/received.pdf\"\necho 'request id is Office-42 (0 file(s))'\n",
    ).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cmd::print_cmds::tests::spool_transport_streams_pdf_and_propagates_refusal", "--nocapture"])
        .env(CHILD, dir.path())
        .env("PATH", dir.path())
        // The child spools into the fake `lp` above, so a shell that blocks real printing
        // (`PHOTOCRAFT_NO_PRINT`, see `disabled_printing_never_spools`) must not stop it.
        .env_remove("PHOTOCRAFT_NO_PRINT")
        .output()
        .unwrap();
    assert!(result.status.success(), "{}\n{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"), "child must select the transport test");
}

#[test]
fn disabled_printing_never_spools() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    super::disable_printing();
    assert!(super::printing_disabled());
    let e = s.execute("file.print", json!({})).unwrap_err().to_string();
    assert!(e.contains("printing is disabled"), "{e}");
    assert!(s.execute("file.printOneCopy", json!({})).is_err());
    // A dry run still renders and reports the command it would have run.
    let r = s.execute("file.print", json!({"dryRun": true})).unwrap();
    assert_eq!(r["sent"], false);
    assert!(r["command"].is_array(), "{r}");
}

#[test]
fn printers_lists_the_spooler_destinations_without_a_document() {
    let mut s = Session::new();
    s.printer_source = Some(test_printers());
    let r = s.execute("file.printers", json!({})).unwrap();
    assert_eq!(r["printers"], json!([{"name": "Studio", "default": false}, {"name": "Office", "default": true}]));
    assert_eq!(r["default"], "Office");
    s.printer_source = Some(std::sync::Arc::new(Vec::new));
    let r = s.execute("file.printers", json!({})).unwrap();
    assert_eq!((r["printers"].clone(), r["default"].clone()), (json!([]), Value::Null));
}

/// Threads that list printers at the same time enter the spooler one at a time: the stand-in
/// printer list records how many threads were inside it at once.
#[test]
fn printer_listings_enter_the_spooler_one_thread_at_a_time() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    const THREADS: usize = 8;
    let inside = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    let start = Arc::new(Barrier::new(THREADS));
    let threads: Vec<_> = (0..THREADS)
        .map(|i| {
            let (inside, most, start) = (Arc::clone(&inside), Arc::clone(&most), Arc::clone(&start));
            std::thread::spawn(move || {
                let mut s = Session::new();
                s.printer_source = Some(Arc::new(move || {
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    most.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    inside.fetch_sub(1, Ordering::SeqCst);
                    vec![craft_print::Printer { name: "Office".into(), default: true, two_sided: None, description: String::new() }]
                }));
                start.wait();
                for _ in 0..3 {
                    // `file.print.printers` lists the stand-in on Windows only.
                    let id = if i % 2 == 0 || !cfg!(windows) { "file.printers" } else { "file.print.printers" };
                    assert_eq!(s.execute(id, json!({})).unwrap()["printers"][0]["name"], "Office");
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(most.load(Ordering::SeqCst), 1, "two threads were in the printer listing at once");
}

/// A caller that panics inside the spooler leaves it usable for later callers.
#[test]
fn a_panic_inside_the_spooler_does_not_block_later_callers() {
    let panicked = std::thread::spawn(|| with_spooler::<()>(|| panic!("a driver call panicked"))).join();
    assert!(panicked.is_err());
    assert_eq!(with_spooler(|| 7), 7);
    let mut s = Session::new();
    s.printer_source = Some(test_printers());
    assert_eq!(s.execute("file.printers", json!({})).unwrap()["default"], "Office");
}

/// A driver call made while this thread already holds the spooler (a command run from the Print
/// dialog's `try_with_spooler`) runs at once.
#[test]
fn a_thread_holding_the_spooler_does_not_wait_for_itself() {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut s = Session::new();
        s.printer_source = Some(test_printers());
        // Inside one outer hold, so no other test takes the lock between the calls.
        let answers = with_spooler(|| {
            let nested = with_spooler(|| with_spooler(|| 7));
            let tried = try_with_spooler(|| with_spooler(|| try_with_spooler(|| 8)));
            let listed = try_with_spooler(|| s.execute("file.printers", json!({})).unwrap()["default"].clone());
            (nested, tried, listed)
        });
        done.send(answers).unwrap();
    });
    // Bounded, so a thread that waits for itself fails the test rather than hanging it.
    let answers = finished.recv_timeout(std::time::Duration::from_secs(30)).expect("a thread holding the spooler waited for itself");
    assert_eq!(answers, (7, Some(Some(8)), Some(json!("Office"))));
}

/// While another thread is in the spooler (a print job being handed over, Print Settings…), the
/// Print dialog's listings return at once with the last lists this process got.
#[test]
fn listings_without_waiting_return_the_last_lists_while_the_spooler_is_busy() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&asked);
    let mut s = Session::new();
    s.printer_source = Some(Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        vec![craft_print::Printer { name: "Office".into(), default: true, two_sided: None, description: String::new() }]
    }));
    let (held_tx, held) = mpsc::channel();
    let (release, released_rx) = mpsc::channel::<()>();
    let released = Arc::new(AtomicBool::new(false));
    let holder = {
        let released = Arc::clone(&released);
        let mut lister = Session::new();
        lister.printer_source = s.printer_source.clone();
        std::thread::spawn(move || {
            with_spooler(|| {
                // Lists this stand-in's printers once with the lock held, so another test's listing
                // cannot replace the last lists before the listings below.
                held_tx.send((list_system_printers(&lister), list_rgb_printers(&lister))).unwrap();
                // Bounded, so a listing that waits fails the test rather than hanging it.
                let _ = released_rx.recv_timeout(std::time::Duration::from_secs(5));
                released.store(true, Ordering::SeqCst);
            });
        })
    };
    let (kept, kept_rgb) = held.recv().unwrap();
    let before = asked.load(Ordering::SeqCst);
    let listed = printers_without_waiting(&mut s);
    let listed_rgb = rgb_printers_without_waiting(&s);
    assert!(!released.load(Ordering::SeqCst), "the listing waited for the spooler");
    release.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(asked.load(Ordering::SeqCst), before, "a printer list was asked for while the spooler was busy");
    assert_eq!(listed, printers_answer(kept));
    assert_eq!(listed_rgb, kept_rgb);
}

#[test]
fn listings_without_waiting_skip_the_lists_of_another_stand_in() {
    uncached_listings_do_not_wait(true);
}

#[test]
fn listings_without_waiting_return_empty_when_no_list_was_cached() {
    uncached_listings_do_not_wait(false);
}

fn uncached_listings_do_not_wait(cache_other: bool) {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&asked);
    let mut s = Session::new();
    s.printer_source = Some(Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        vec![craft_print::Printer { name: "Proofing".into(), default: true, two_sided: None, description: String::new() }]
    }));
    let (held_tx, held) = mpsc::channel();
    let (release, released_rx) = mpsc::channel();
    let released = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&released);
    let holder = std::thread::spawn(move || {
        with_spooler(|| {
            if cache_other {
                let mut other = Session::new();
                other.printer_source = Some(test_printers());
                list_system_printers(&other);
                let _ = list_rgb_printers(&other);
            } else {
                *LAST_PRINTERS.lock().unwrap() = None;
                *LAST_RGB_PRINTERS.lock().unwrap() = None;
            }
            held_tx.send(()).unwrap();
            let _ = released_rx.recv_timeout(std::time::Duration::from_secs(5));
            done.store(true, Ordering::SeqCst);
        });
    });
    held.recv().unwrap();
    let listed = printers_without_waiting(&mut s);
    let listed_rgb = rgb_printers_without_waiting(&s);
    let waited = released.load(Ordering::SeqCst);
    let _ = release.send(());
    holder.join().unwrap();
    assert!(!waited, "an uncached listing waited for the spooler");
    assert_eq!(asked.load(Ordering::SeqCst), 0);
    assert_eq!(listed, printers_answer(Vec::new()));
    assert_eq!(listed_rgb, Err(craft_print::rgb::PrintError::Unsupported));
    with_spooler(|| {
        let listed = printers_without_waiting(&mut s);
        let listed_rgb = rgb_printers_without_waiting(&s);
        assert_eq!(listed["printers"], json!([{"name": "Proofing", "default": true}]));
        let names = listed_rgb.map(|list| list.into_iter().map(|p| p.name).collect::<Vec<_>>());
        if cfg!(windows) {
            assert_eq!(names, Ok(vec!["Proofing".to_string()]));
        } else {
            assert_eq!(names, Err(craft_print::rgb::PrintError::Unsupported));
        }
    });
}

/// Spooling tests run in a child process: `disable_printing` is process-wide and a test above
/// sets it, and CI sets `PHOTOCRAFT_NO_PRINT` for the whole shell. True in the child, which runs
/// the test body; the parent starts the child and checks that it passed.
fn in_child(test: &str, envs: &[(&str, &str)]) -> bool {
    const CHILD: &str = "PHOTOCRAFT_TEST_PRINT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", test, "--nocapture"]).env(CHILD, "1").env_remove("PHOTOCRAFT_NO_PRINT");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    // Bounded: a child stuck in a printer dialog or a driver must not hang the suite.
    let mut child = cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let out = child.wait_with_output().unwrap();
    assert!(
        status.is_some_and(|s| s.success()),
        "{}\nstdout: {}\nstderr: {}",
        if status.is_none() { "the child did not finish within 120 s and was killed" } else { "the child failed" },
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

/// A spooler that keeps every job it gets.
fn recording_spooler(preferred: bool, seen: &std::sync::Arc<std::sync::Mutex<Vec<PrintJob>>>) -> Spooler {
    let seen = std::sync::Arc::clone(seen);
    Spooler {
        name: "test",
        preferred,
        run: std::sync::Arc::new(move |job: &PrintJob| {
            seen.lock().unwrap().push(job.clone());
            Ok("queued".into())
        }),
    }
}

#[test]
fn a_preferred_app_spooler_takes_the_job() {
    if !in_child("cmd::print_cmds::tests::a_preferred_app_spooler_takes_the_job", &[]) {
        return;
    }
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut s = session("rgb", 8);
    s.print_spooler = Some(recording_spooler(true, &seen));
    let r = s.execute("file.print", json!({"copies": 2, "paper": "a4", "orientation": "landscape"})).unwrap();
    assert_eq!((r["sent"].as_bool(), r["backend"].as_str(), r["spooler"].as_str()), (Some(true), Some("test"), Some("queued")));
    assert!(r["pdf"].is_null(), "{r}");
    let jobs = seen.lock().unwrap();
    let job = &jobs[0];
    assert!(job.pdf.starts_with(b"%PDF-1.4"));
    assert_eq!((job.copies, job.title.as_str(), job.printer.as_deref(), job.landscape), (2, "Print Me.psd", None, true));
    assert!((job.paper.0 - 841.89).abs() < 0.01 && (job.paper.1 - 595.28).abs() < 0.01);
    drop(jobs);
    // A named print server asks for the IPP route, preferred spooler or not.
    let e = s.execute("file.print", json!({"printServer": "127.0.0.1:1"})).unwrap_err().to_string();
    assert!(e.contains("no print server at 127.0.0.1:1") || e.contains("not available on Windows"), "{e}");
    assert_eq!(seen.lock().unwrap().len(), 1);
    // "output" keeps the file and does not spool unless asked.
    let out = format!("{}/out.pdf", tmp("output"));
    let r = s.execute("file.print", json!({"output": out})).unwrap();
    assert_eq!((r["sent"].as_bool(), r["pdf"].as_str()), (Some(false), Some(out.as_str())));
    assert!(std::path::Path::new(&out).exists());
}

#[cfg(not(target_os = "windows"))]
mod cups {
    use super::*;
    use photocraft_ipp::{
        Attribute, OP_CUPS_GET_CLASSES, OP_CUPS_GET_DEFAULT, OP_CUPS_GET_PRINTERS, OP_PRINT_JOB, Printer, Request, TAG_JOB_GROUP, TAG_NAME, TAG_PRINTER_GROUP,
    };
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::mpsc;

    fn printer(name: &str, is_default: bool) -> Printer {
        Printer { name: name.into(), info: String::new(), state: 3, accepting_jobs: true, is_default, is_class: false }
    }

    #[test]
    fn the_printer_is_the_requested_one_else_the_default_else_the_only_one() {
        let two = [printer("Office", false), printer("Lab", true)];
        assert_eq!(resolve_printer(None, Some("Lab"), &two).ok(), Some("Lab".into()));
        assert_eq!(resolve_printer(Some("office"), Some("Lab"), &two).ok(), Some("Office".into()));
        assert_eq!(resolve_printer(None, None, &two[..1]).ok(), Some("Office".into()));
        match resolve_printer(None, None, &two) {
            Err(SpoolError::Failed(e)) => assert!(e.contains("no default printer") && e.contains("Office, Lab"), "{e}"),
            _ => panic!("several printers and no default must fail"),
        }
        match resolve_printer(Some("Nope"), Some("Lab"), &two) {
            Err(SpoolError::Failed(e)) => assert!(e.contains("`Nope`") && e.contains("Office, Lab"), "{e}"),
            _ => panic!("an unknown printer must fail"),
        }
        match resolve_printer(None, None, &[]) {
            Err(SpoolError::NoRoute(e)) => assert!(e.contains("no printer is set up"), "{e}"),
            _ => panic!("no printers is no route"),
        }
        // A printer asked for by name on a server without any is a failure, not a missing route.
        match resolve_printer(Some("Office"), None, &[]) {
            Err(SpoolError::Failed(e)) => assert!(e.contains("`Office`") && e.contains("none"), "{e}"),
            _ => panic!("a named printer never falls through"),
        }
        // A stale default that the server no longer lists falls through to the only printer.
        assert_eq!(resolve_printer(None, Some("Gone"), &two[..1]).ok(), Some("Office".into()));
    }

    /// What the fake received per request: op, attributes, document.
    type Seen = (u16, Vec<Attribute>, Vec<u8>);

    /// A fake CUPS on a TCP port: lists `printers` and `classes`, names `default`, accepts jobs
    /// as id 42. Answers for as long as the test process lives.
    fn fake_cups(printers: &'static [&'static str], classes: &'static [&'static str], default: Option<&'static str>) -> (String, mpsc::Receiver<Seen>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let server = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let mut reader = BufReader::new(stream);
                let (mut first, mut line, mut len) = (String::new(), String::new(), 0usize);
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if first.is_empty() {
                        first = line.clone();
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                }
                if first.is_empty() {
                    continue; // a reachability probe
                }
                let mut body = vec![0u8; len];
                reader.read_exact(&mut body).unwrap();
                let (op, attrs, doc) = photocraft_ipp::parse_request(&body).unwrap();
                let list = |names: &[&str]| {
                    if names.is_empty() {
                        return Request::new(0x0406, 1).end();
                    }
                    let mut r = Request::new(0, 1);
                    for p in names {
                        r.group(TAG_PRINTER_GROUP);
                        r.attr(TAG_NAME, "printer-name", p.as_bytes());
                    }
                    r.end()
                };
                let reply = match op {
                    OP_CUPS_GET_PRINTERS => list(printers),
                    OP_CUPS_GET_CLASSES => list(classes),
                    OP_CUPS_GET_DEFAULT => match default {
                        Some(d) => {
                            let mut r = Request::new(0, 1);
                            r.group(TAG_PRINTER_GROUP);
                            r.attr(TAG_NAME, "printer-name", d.as_bytes());
                            r.end()
                        }
                        None => Request::new(0x0406, 1).end(),
                    },
                    OP_PRINT_JOB => {
                        let mut r = Request::new(0, 1);
                        r.group(TAG_JOB_GROUP);
                        r.integer("job-id", 42);
                        r.end()
                    }
                    _ => Request::new(0x0501, 1).end(),
                };
                let _ = tx.send((op, attrs, doc));
                let mut out = reader.into_inner();
                let _ = out.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n", reply.len()).as_bytes());
                let _ = out.write_all(&reply);
                let _ = out.flush();
            }
        });
        (server, rx)
    }

    #[test]
    fn a_named_server_is_printed_to_over_ipp() {
        if !in_child("cmd::print_cmds::tests::cups::a_named_server_is_printed_to_over_ipp", &[]) {
            return;
        }
        let (server, rx) = fake_cups(&["Office"], &["Lab"], Some("Lab"));
        let mut s = session("rgb", 8);
        let r = s.execute("file.print", json!({"printServer": server, "copies": 3})).unwrap();
        assert_eq!((r["sent"].as_bool(), r["backend"].as_str(), r["spooler"].as_str()), (Some(true), Some("ipp"), Some("job 42 on Lab")));
        assert!(r["pdf"].is_null());
        let job = std::iter::from_fn(|| rx.try_recv().ok()).find(|(op, _, _)| *op == OP_PRINT_JOB).expect("a Print-Job request");
        let attr = |name: &str| job.1.iter().find(|a| a.name == name).unwrap_or_else(|| panic!("{name}"));
        assert!(attr("printer-uri").text().ends_with("/classes/Lab"), "{}", attr("printer-uri").text());
        assert_eq!(
            (attr("copies").int(), attr("job-name").text().as_str(), attr("document-format").text().as_str()),
            (Some(3), "Print Me.psd", "application/pdf")
        );
        assert!(job.2.starts_with(b"%PDF-1.4"));
        // The requested printer, in any case; an unknown one names the choices.
        let r = s.execute("file.print", json!({"printServer": server, "printer": "office"})).unwrap();
        assert_eq!(r["spooler"], "job 42 on Office");
        let job = std::iter::from_fn(|| rx.try_recv().ok()).filter(|(op, _, _)| *op == OP_PRINT_JOB).last().expect("the second Print-Job");
        assert!(job.1.iter().any(|a| a.name == "printer-uri" && a.text().ends_with("/printers/Office")));
        let e = s.execute("file.print", json!({"printServer": server, "printer": "Nope"})).unwrap_err().to_string();
        assert!(e.contains("`Nope`") && e.contains("Office, Lab"), "{e}");
        // Printers can be listed.
        let r = s.execute("file.printers", json!({"printServer": server})).unwrap();
        let names: Vec<&str> = r["printers"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
        assert_eq!((names, r["default"].as_str()), (vec!["Office", "Lab"], Some("Lab")));
        assert_eq!(
            (r["printers"][1]["default"].as_bool(), r["printers"][1]["class"].as_bool(), r["printers"][0]["class"].as_bool()),
            (Some(true), Some(true), Some(false))
        );
    }

    #[test]
    fn without_a_printer_the_app_spooler_takes_over() {
        if !in_child("cmd::print_cmds::tests::cups::without_a_printer_the_app_spooler_takes_over", &[]) {
            return;
        }
        let (server, _rx) = fake_cups(&[], &[], None);
        let mut s = session("gray", 8);
        let e = s.execute("file.print", json!({"printServer": server})).unwrap_err().to_string();
        assert!(e.contains("no printer is set up"), "{e}");
        // Asking for a printer by name never falls through to another route.
        let e = s.execute("file.print", json!({"printServer": server, "printer": "Office"})).unwrap_err().to_string();
        assert!(e.contains("`Office`"), "{e}");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        s.print_spooler = Some(recording_spooler(false, &seen));
        let r = s.execute("file.print", json!({"printServer": server})).unwrap();
        assert_eq!((r["backend"].as_str(), r["spooler"].as_str()), (Some("test"), Some("queued")));
        assert_eq!(seen.lock().unwrap().len(), 1);
        // An unreachable named server is a failure, not a missing route.
        let e = s.execute("file.print", json!({"printServer": "127.0.0.1:1"})).unwrap_err().to_string();
        assert!(e.contains("no print server at"), "{e}");
        let r = s.execute("file.printers", json!({"printServer": "127.0.0.1:1"})).unwrap();
        assert!(r["printers"].as_array().unwrap().is_empty() && r["note"].as_str().unwrap().contains("no print server"));
    }

    /// The Flatpak case: no `lp` on the path, so the job goes to the system's CUPS over IPP
    /// (`CUPS_SERVER` names it here), and with no print server at all to the app's spooler.
    /// Runs in a child process, since the path is process-wide.
    #[test]
    fn without_lp_the_job_goes_over_ipp_then_to_the_app_spooler() {
        const CHILD: &str = "PHOTOCRAFT_TEST_PRINT_NO_LP";
        if std::env::var_os(CHILD).is_some() {
            let mut s = session("rgb", 8);
            let r = s.execute("file.print", json!({"copies": 2})).unwrap();
            assert_eq!((r["sent"].as_bool(), r["backend"].as_str(), r["spooler"].as_str()), (Some(true), Some("ipp"), Some("job 42 on Office")));
            let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            s.print_spooler = Some(recording_spooler(false, &seen));
            // The IPP route first, then the app's spooler only when there is no server.
            let r = s.execute("file.print", json!({})).unwrap();
            assert_eq!(r["backend"], "ipp");
            assert!(seen.lock().unwrap().is_empty());
            // (CUPS_SERVER is read per job; an unreachable one is no route.)
            let r = s.execute("file.print", json!({"printServer": "127.0.0.1:1"})).unwrap_err().to_string();
            assert!(r.contains("no print server at"), "{r}");
            println!("child ok");
            return;
        }
        let (server, rx) = fake_cups(&["Office"], &[], Some("Office"));
        let no_lp = tempfile::tempdir().unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "cmd::print_cmds::tests::cups::without_lp_the_job_goes_over_ipp_then_to_the_app_spooler", "--nocapture"])
            .env(CHILD, "1")
            .env("PATH", no_lp.path())
            .env("CUPS_SERVER", &server)
            .env_remove("PHOTOCRAFT_NO_PRINT")
            .output()
            .unwrap();
        let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && stdout.contains("child ok"), "stdout: {stdout}\nstderr: {stderr}");
        let jobs = std::iter::from_fn(|| rx.try_recv().ok()).filter(|(op, _, _)| *op == OP_PRINT_JOB).count();
        assert_eq!(jobs, 2, "both prints reached the fake CUPS");
    }
}

#[cfg(target_os = "windows")]
#[test]
fn windows_pdf_component_is_detected_by_loading_its_dll() {
    // #5884: a Windows image without a loadable Windows.Data.Pdf.dll reports an error instead of
    // aborting at the delay-load stub.
    let dir = tempfile::tempdir().unwrap();
    let dll = windows_pdf_dll(dir.path());
    assert!(!windows_pdf_loads(&dll));
    // Present but not loadable counts as missing.
    std::fs::create_dir(dir.path().join("System32")).unwrap();
    std::fs::write(&dll, b"MZ").unwrap();
    assert!(!windows_pdf_loads(&dll));
    let root = std::env::var_os("SystemRoot").unwrap();
    // The real DLL may be missing or unloadable on a stripped image; the check follows the loader.
    let system = windows_pdf_dll(std::path::Path::new(&root));
    assert_eq!(require_windows_pdf().is_ok(), windows_pdf_loads(&system));
}

#[cfg(not(target_os = "windows"))]
#[test]
fn the_windows_pdf_check_is_a_no_op_elsewhere() {
    assert!(require_windows_pdf().is_ok());
}

/// A 500 × 1000 pt document (72 ppi) on A3 (841.89 × 1190.55 pt): its height sets the fit.
fn a3_layout(extra: Value) -> Result<PrintLayout> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 500, "height": 1000, "resolution": 72})).unwrap();
    let mut p = json!({"paper": "a3", "scaleToFit": true});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    layout(&s.active().unwrap().doc, &p, "file.print")
}

#[test]
fn scale_to_fit_fills_a_borderless_printable_area() {
    let (pw, ph) = (841.89, 1190.55);
    // Without a printable area, the fixed 18 pt margin stays (CUPS, PDF).
    let l = a3_layout(json!({})).unwrap();
    assert!((l.scale - (ph - 36.0) / 1000.0).abs() < 1e-9, "{l:?}");
    // Borderless: the driver reports the whole sheet (#7782: a Canon PRO-1000 reports its A3 as
    // 842.29 × 1191.14 pt), and the image reaches the paper's edges.
    let l = a3_layout(json!({"printable": [0.0, 0.0, 842.29, 1191.14]})).unwrap();
    assert!((l.scale - ph / 1000.0).abs() < 1e-9, "{l:?}");
    let (x, y, w, h) = l.rect;
    assert!(y.abs() < 1e-9 && (h - ph).abs() < 1e-9 && (x - (pw - w) / 2.0).abs() < 1e-9, "{l:?}");
}

#[test]
fn scale_to_fit_stays_inside_an_uneven_printable_area() {
    let (pw, ph) = (841.89, 1190.55);
    // 17 pt left and right, 14 pt at the top, 48 pt at the bottom (from the top-left, as
    // `file.print.page` reports it). The layout's rect has its origin at the bottom-left.
    let l = a3_layout(json!({"printable": [17.0, 14.0, pw - 34.0, ph - 62.0]})).unwrap();
    assert!((l.scale - (ph - 62.0) / 1000.0).abs() < 1e-9, "{l:?}");
    let (x, y, w, h) = l.rect;
    assert!((y - 48.0).abs() < 1e-9 && (y + h - (ph - 14.0)).abs() < 1e-9, "fills the printable height: {l:?}");
    assert!((x - (pw - w) / 2.0).abs() < 1e-9, "centred between equal side margins: {l:?}");
    // Crop and registration marks keep their 36 pt inset, inside the printable area.
    let l = a3_layout(json!({"printable": [17.0, 14.0, pw - 34.0, ph - 62.0], "cornerCropMarks": true})).unwrap();
    assert!((l.scale - (ph - 62.0 - 72.0) / 1000.0).abs() < 1e-9, "{l:?}");
    assert!((l.rect.1 - 84.0).abs() < 1e-9, "{l:?}");
    // Scale to fit off: the printable area changes nothing.
    let l = a3_layout(json!({"printable": [17.0, 14.0, pw - 34.0, ph - 62.0], "scaleToFit": false})).unwrap();
    assert_eq!(l, a3_layout(json!({"scaleToFit": false})).unwrap());
}

#[test]
fn a_bad_printable_area_is_refused() {
    let portrait = json!([0.0, 0.0, 842.29, 1191.14]);
    for bad in [
        json!([0, 0, 100]),
        json!([0, 0, 0, 100]),
        json!([0, 0, 100, -5]),
        json!("whole"),
        json!([2000, 0, 100, 100]),
        json!([0, "oops", 0, 842.29, 1191.14]),
        json!([0, 0, 842.29, null]),
    ] {
        assert!(a3_layout(json!({ "printable": bad })).is_err(), "{bad}");
    }
    // A portrait sheet's area on a landscape layout is another paper, not a rounding error.
    assert!(a3_layout(json!({"printable": portrait, "orientation": "landscape"})).is_err());
}

#[test]
fn emulsion_down_fits_the_mirrored_printable_area() {
    let (pw, ph) = (841.89, 1190.55);
    // 10 pt on the left, 40 pt on the right of the sheet as the driver sees it. The page is
    // mirrored as it prints, so the image box (before mirroring) must fit the mirror image.
    // A wide image, so the printable width sets the fit.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1000, "height": 500, "resolution": 72})).unwrap();
    let p = json!({"paper": "a3", "scaleToFit": true, "printable": [10.0, 0.0, pw - 50.0, ph], "emulsionDown": true});
    let l = layout(&s.active().unwrap().doc, &p, "file.print").unwrap();
    let (x, _, w, _) = l.rect;
    let printed = (pw - x - w, pw - x);
    assert!(printed.0 >= 10.0 - 1e-9 && printed.1 <= pw - 40.0 + 1e-9, "printed at {printed:?}: {l:?}");
}

/// A native print fits only the driver's area: one the caller passed is dropped when the
/// driver can't give one for this paper (no such paper here, or no driver at all).
#[test]
fn a_native_print_drops_a_printable_area_the_driver_did_not_give() {
    let p = with_driver_printable(&json!({"paper": [500.0, 500.0], "printable": [0.0, 0.0, 100.0, 100.0], "scaleToFit": true}));
    assert!(p.get("printable").is_none(), "{p}");
    assert_eq!(p["paper"], json!([500.0, 500.0]));
}

#[test]
fn a_print_uses_the_printable_area_but_does_not_remember_it() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 500, "height": 1000, "resolution": 72})).unwrap();
    let r = s.execute("file.print", json!({"paper": "a3", "scaleToFit": true, "printable": [0.0, 0.0, 842.29, 1191.14], "dryRun": true})).unwrap();
    assert!((r["scale"].as_f64().unwrap() - 119.055).abs() < 1e-6, "{r}");
    // The next print (Print One Copy, or the dialog on another printer or to PDF) reads its own.
    let last = last_print(&s).unwrap();
    assert!(last.get("printable").is_none() && last["scaleToFit"] == true, "{last}");
}

#[test]
fn a_rejected_printable_area_fails_the_print_and_changes_nothing() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 500, "height": 1000, "resolution": 72})).unwrap();
    s.execute("file.print", json!({"paper": "a3", "scale": 50, "dryRun": true})).unwrap();
    let before = (last_print(&s), s.prefs.get().dialogs.get(REMEMBERED).cloned(), s.active().unwrap().doc.clone());
    for printable in [json!([2000, 0, 100, 100]), json!([0, "oops", 0, 842.29, 1191.14])] {
        let r = s.execute("file.print", json!({"paper": "a3", "scaleToFit": true, "printable": printable, "dryRun": true}));
        assert!(r.is_err(), "{printable}: {r:?}");
    }
    assert!(before == (last_print(&s), s.prefs.get().dialogs.get(REMEMBERED).cloned(), s.active().unwrap().doc.clone()));
    // A print at a manual scale doesn't read the area, so a stale one doesn't fail it.
    let r = s.execute("file.print", json!({"paper": "a3", "scale": 50, "printable": [0.0, 0.0, 842.29, 1191.14], "orientation": "landscape", "dryRun": true}));
    assert!(r.is_ok(), "{r:?}");
    // Marks need room only to fit: a small manual scale in a 60 pt-wide area prints.
    let r = s.execute("file.print", json!({"paper": "a3", "scale": 5, "printable": [0, 0, 60, 1191.14], "cornerCropMarks": true, "dryRun": true}));
    assert!(r.is_ok(), "{r:?}");
    let r = s.execute("file.print", json!({"paper": "a3", "scaleToFit": true, "printable": [0, 0, 60, 1191.14], "cornerCropMarks": true, "dryRun": true}));
    assert!(r.is_err(), "{r:?}");
}

#[test]
fn package_of_a_document_named_after_a_windows_device_is_written() {
    // On Windows a folder named `nul` is the NUL device: the save failed with "path not found".
    let dir = tmp("package-device-name");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "nul.psd"})).unwrap();
    let r = s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap();
    assert_eq!(r["folder"].as_str().unwrap(), format!("{dir}/pkg/nul_"));
    let document = r["document"].as_str().unwrap();
    assert!(document.ends_with("pkg/nul_/nul_.pcraft"), "{document}");
    assert!(std::path::Path::new(document).is_file());
}

/// A dry-run print to `name`.pdf with `extra` params; returns the result and the PDF content stream.
fn print_with(s: &mut Session, dir: &str, name: &str, extra: Value) -> (Value, Vec<u8>) {
    let mut p = json!({"output": format!("{dir}/{name}.pdf"), "dryRun": true, "paper": "a4", "center": false, "left": 1.0, "top": 1.0});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    let r = s.execute("file.print", p).unwrap();
    let pdf = std::fs::read(r["pdf"].as_str().unwrap()).unwrap();
    (r, pdf)
}

#[test]
fn zero_bleed_prints_exactly_as_before() {
    let mut s = session("rgb", 8);
    let dir = tmp("bleed-zero");
    let (plain, a) = print_with(&mut s, &dir, "plain", json!({"cornerCropMarks": true}));
    let (zero, b) = print_with(&mut s, &dir, "zero", json!({"cornerCropMarks": true, "bleed": 0.0}));
    assert_eq!(a, b, "bleed 0 must not change a single byte");
    assert_eq!(plain["bleed"], 0.0);
    assert_eq!(zero["bleed"], 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bleed_puts_corner_crop_marks_on_the_trim_line_outside_the_image() {
    let mut s = session("rgb", 8);
    let dir = tmp("bleed");
    let (r, pdf) = print_with(&mut s, &dir, "bleed", json!({"cornerCropMarks": true, "centerCropMarks": true, "bleed": 0.125}));
    assert_eq!(r["bleed"], 0.125);
    let rect: Vec<f64> = r["imageRect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    let (x, y, w, h) = (rect[0], rect[1], rect[2], rect[3]);
    let c = pdf_content(&pdf);
    // Bottom-left corner (PDF is bottom-up): the horizontal mark runs left from 6 pt outside the
    // image, 9 pt (0.125 in) above the image's bottom edge; the vertical one 9 pt right of its left edge.
    assert!(c.contains(&format!("{:.2} {:.2} m {:.2} {:.2} l S", x - 6.0, y + 9.0, x - 24.0, y + 9.0)), "{c}");
    assert!(c.contains(&format!("{:.2} {:.2} m {:.2} {:.2} l S", x + 9.0, y - 6.0, x + 9.0, y - 24.0)), "{c}");
    // Top-right corner, mirrored.
    assert!(c.contains(&format!("{:.2} {:.2} m {:.2} {:.2} l S", x + w + 6.0, y + h - 9.0, x + w + 24.0, y + h - 9.0)), "{c}");
    assert!(c.contains(&format!("{:.2} {:.2} m {:.2} {:.2} l S", x + w - 9.0, y + h + 6.0, x + w - 9.0, y + h + 24.0)), "{c}");
    // Centre crop marks are on the midlines, which a symmetric bleed does not move.
    assert!(c.contains(&format!("{:.2} {:.2} m {:.2} {:.2} l S", x + w / 2.0, y - 6.0, x + w / 2.0, y - 24.0)), "{c}");
    // Print One Copy repeats the last print, bleed included.
    let again = s.execute("file.printOneCopy", json!({"output": format!("{dir}/again.pdf"), "dryRun": true})).unwrap();
    assert_eq!(again["bleed"], 0.125);
    // The image box itself is unchanged: bleed moves marks, not pixels.
    let (plain, _) = print_with(&mut s, &dir, "plain", json!({"cornerCropMarks": true}));
    assert_eq!(plain["imageRect"], r["imageRect"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bleed_without_corner_crop_marks_changes_nothing() {
    let mut s = session("rgb", 8);
    let dir = tmp("bleed-nomarks");
    let (_, a) = print_with(&mut s, &dir, "plain", json!({"registrationMarks": true}));
    let (r, b) = print_with(&mut s, &dir, "bleed", json!({"registrationMarks": true, "bleed": 0.1}));
    assert_eq!(a, b);
    assert_eq!(r["bleed"], 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_bleed_is_an_error_not_a_panic() {
    let mut s = session("rgb", 8);
    // The test document prints 300 × 150 pt, so 1.05 in (75.6 pt) leaves no trim box.
    for bleed in [json!(-0.1), json!("0.1"), json!(true), json!([0.1]), json!(1.05), json!(1e308)] {
        let r = s.execute("file.print", json!({"dryRun": true, "send": false, "output": "", "cornerCropMarks": true, "bleed": bleed}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "bleed {bleed}: {r:?}");
    }
}

#[test]
fn print_strings_keep_winansi_accents_and_punctuation() {
    assert_eq!(pdf_string("Café – résumé"), r"(Caf\351 \226 r\351sum\351)");
    assert_eq!(pdf_string("€ ‘quote’ “double” • Œœ Šš Žž Ÿ"), r"(\200 \221quote\222 \223double\224 \225 \214\234 \212\232 \216\236 \237)");
    assert_eq!(pdf_string("é7"), r"(\3517)", "octal escapes must have three digits before a digit");
}

#[test]
fn print_strings_keep_pdf_delimiters_safe_and_fallback_bounded() {
    assert_eq!(pdf_string(r"a(b)\c"), r"(a\(b\)\\c)");
    assert_eq!(pdf_string("\n\r\t\0\u{7f}\u{81}中😀"), "(????????)");
    assert_eq!(pdf_string(""), "()");
}

#[test]
fn printed_pdf_uses_winansi_for_labels_and_descriptions() {
    let mut page = page_of(3, vec![255; 6], false);
    page.label = Some("Café – résumé.psd".into());
    page.description = Some("Price: €20 (proof)".into());
    let pdf = print_pdf(&page);
    assert!(String::from_utf8_lossy(&pdf).contains("/Encoding /WinAnsiEncoding"));
    let content = pdf_content(&pdf);
    assert!(content.contains(r"(Caf\351 \226 r\351sum\351.psd) Tj"), "{content}");
    assert!(content.contains(r"(Price: \20020 \(proof\)) Tj"), "{content}");
    assert!(content.is_ascii(), "encoded bytes are escaped, never UTF-8 text operands");
}
