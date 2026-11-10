use super::*;
use photocraft_doc::LayerId;
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    s
}

fn frames(s: &Session, id: LayerId) -> usize {
    s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().frames.len()
}

#[test]
fn new_blank_insert_duplicate_delete_frames() {
    let mut s = session();
    let r = s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert!(s.active().unwrap().doc.layer(id).unwrap().video.is_some());
    assert!(s.active().unwrap().doc.timeline.is_some());
    let start = frames(&s, id);
    s.execute("layer.videoLayers.insertBlankFrame", json!({})).unwrap();
    assert_eq!(frames(&s, id), start + 1);
    s.execute("layer.videoLayers.duplicateFrame", json!({})).unwrap();
    assert_eq!(frames(&s, id), start + 2);
    s.execute("layer.videoLayers.deleteFrame", json!({})).unwrap();
    assert_eq!(frames(&s, id), start + 1);
    assert!(s.active().unwrap().doc.timeline.as_ref().unwrap().duration >= frames(&s, id));
}

/// Check the shared growth limit without constructing a million-frame document.
#[test]
fn frame_growth_stops_at_the_limit_of_either_stack() {
    let limit = crate::timeline_cmds::MAX_DURATION;
    assert!(check_frame_growth(limit - 1, None).is_ok());
    assert!(check_frame_growth(limit - 1, Some(limit - 1)).is_ok());
    assert!(check_frame_growth(limit, None).is_err());
    assert!(check_frame_growth(limit - 1, Some(limit)).is_err());
    assert!(check_frame_growth(limit + 1, None).is_err());
}

#[test]
fn rasterize_drops_the_stack() {
    let mut s = session();
    s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
    let r = s.execute("layer.videoLayers.rasterize", json!({})).unwrap();
    assert_eq!(r["rasterized"], true);
    let id = s.active().unwrap().active_layer.unwrap();
    assert!(s.active().unwrap().doc.layer(id).unwrap().video.is_none());
    assert!(matches!(s.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Raster(_)));
}

#[test]
fn frame_ops_need_a_video_layer() {
    let mut s = session();
    assert!(s.execute("layer.videoLayers.insertBlankFrame", json!({})).is_err());
}

#[test]
fn scrub_keeps_content_a_raster() {
    let mut s = session();
    let r = s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    s.execute("layer.videoLayers.insertBlankFrame", json!({})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    assert!(matches!(s.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Raster(_)));
}

fn write_png(path: &str, w: u32, h: u32, color: &str) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": color})).unwrap();
    let bytes = photocraft_io::export(&s.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    std::fs::write(path, bytes).unwrap();
}

fn tmpdir(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("pc-video-{}-{name}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d.to_string_lossy().into_owned()
}

/// A classic little-endian TIFF with one 8-bit gray page per `(w, h, level, NewSubfileType)`,
/// chained in order (NewSubfileType 1 marks a reduced-resolution copy, such as a thumbnail).
fn write_tiff(path: &str, pages: &[(u32, u32, u8, u32)]) {
    let mut f = b"II".to_vec();
    f.extend_from_slice(&42u16.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    let mut next_field = 4;
    for &(w, h, level, kind) in pages {
        let pixels_at = f.len() as u32;
        f.extend(std::iter::repeat_n(level, (w * h) as usize));
        if f.len() % 2 == 1 {
            f.push(0);
        }
        let ifd_at = f.len() as u32;
        f[next_field..next_field + 4].copy_from_slice(&ifd_at.to_le_bytes());
        let entries: [(u16, u16, u32); 10] =
            [(254, 4, kind), (256, 4, w), (257, 4, h), (258, 3, 8), (259, 3, 1), (262, 3, 1), (273, 4, pixels_at), (277, 3, 1), (278, 4, h), (279, 4, w * h)];
        f.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, ty, v) in entries {
            f.extend_from_slice(&tag.to_le_bytes());
            f.extend_from_slice(&ty.to_le_bytes());
            f.extend_from_slice(&1u32.to_le_bytes());
            f.extend_from_slice(&v.to_le_bytes());
        }
        next_field = f.len();
        f.extend_from_slice(&0u32.to_le_bytes());
    }
    std::fs::write(path, f).unwrap();
}

/// A real layered TIFF from the exporter: two 8×6 raster layers and Photoshop layer data in tag
/// 37724, in one full-resolution page.
fn layered_tiff_bytes() -> Vec<u8> {
    let mut doc = Document::new("layered", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let fmt = doc.pixel_format();
    for name in ["bottom", "top"] {
        let mut surface = Surface::new(fmt);
        surface.write_interleaved(photocraft_geom::Rect::new(0, 0, 8, 6), &vec![96u8; 8 * 6 * fmt.bytes_per_pixel()]);
        doc.layers.push(Layer::new(name, LayerContent::Raster(surface)));
    }
    let bytes =
        photocraft_io::export(&doc, "layered.tif", &photocraft_io::ExportOptions { tiff_layers: true, ..Default::default() }).expect("a layered TIFF").bytes;
    assert_eq!(bytes.get(0..2), Some(&b"II"[..]), "the TIFF exporter writes little-endian files here");
    assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]), 42, "a classic TIFF");
    bytes
}

/// [`layered_tiff_bytes`] extended to two pages by appending a copy of its first directory as the
/// next one. The copied directory keeps the original tag and pixel offsets, so both pages carry
/// genuine layer data without hand-written Photoshop blocks; the file stays small, and the second
/// page is never decoded.
fn write_layered_multipage_tiff(path: &str) {
    let bytes = layered_tiff_bytes();
    let u16_at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
    let first_ifd = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let count = u16_at(first_ifd) as usize;
    let dir_len = 2 + count * 12 + 4;
    let mut f = bytes.clone();
    if f.len() % 2 == 1 {
        f.push(0);
    }
    let second_ifd = f.len();
    f.extend_from_slice(&bytes[first_ifd..first_ifd + dir_len]);
    let second_next = second_ifd + 2 + count * 12;
    f[second_next..second_next + 4].copy_from_slice(&0u32.to_le_bytes());
    let first_next = first_ifd + 2 + count * 12;
    f[first_next..first_next + 4].copy_from_slice(&u32::try_from(second_ifd).expect("a small TIFF").to_le_bytes());
    std::fs::write(path, f).expect("write the two-page TIFF");
}

/// #6379: Video Frames to Layers takes every page of a multi-page TIFF, one layer each, in
/// order, as it takes a GIF's frames; a thumbnail is not a page. A single-page TIFF is one frame.
#[test]
fn video_frames_to_layers_takes_every_tiff_page() {
    let dir = tmpdir("tiff-pages");
    let path = format!("{dir}/pages.tif");
    write_tiff(&path, &[(2, 2, 9, 1), (6, 4, 51, 0), (3, 5, 204, 0)]);
    let mut s = session();
    let r = s.execute("file.import.videoFramesToLayers", json!({"path": path})).unwrap();
    assert_eq!(r["layers"], 2, "two pages, the thumbnail left out");
    let doc = &s.active().unwrap().doc;
    for (name, (w, h), level) in [("Frame 1", (6, 4), 51.0), ("Frame 2", (3, 5), 204.0)] {
        let l = doc.layers.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("{name}"));
        let surf = l.surface().unwrap();
        let b = surf.content_bounds();
        assert_eq!((b.width(), b.height()), (w, h), "{name}");
        let px = surf.read_region(photocraft_geom::Rect::new(b.x0, b.y0, b.x0 + 1, b.y0 + 1));
        assert!((px[0] * 255.0 - level).abs() < 0.6 && px[3] == 1.0, "{name}: {px:?}");
    }
    let single = format!("{dir}/single.tif");
    write_tiff(&single, &[(4, 4, 120, 0)]);
    let mut s = session();
    assert_eq!(s.execute("file.import.videoFramesToLayers", json!({"path": single})).unwrap()["layers"], 1);
}

/// #6379 review: a TIFF whose page chain loops back (or is cut short, or runs past the directory
/// limit) lists only its first pages. The import is refused, and the document is left as it was,
/// instead of reporting part of the file as all of it.
#[test]
fn video_frames_to_layers_refuses_a_tiff_with_an_incomplete_page_list() {
    let dir = tmpdir("tiff-loop");
    let path = format!("{dir}/loop.tif");
    write_tiff(&path, &[(2, 2, 51, 0), (2, 2, 204, 0)]);
    // The last directory's next-IFD offset points back to the first directory.
    let mut f = std::fs::read(&path).unwrap();
    let first: [u8; 4] = f[4..8].try_into().unwrap();
    let end = f.len();
    f[end - 4..].copy_from_slice(&first);
    std::fs::write(&path, f).unwrap();
    let mut s = session();
    let before = s.active().unwrap().doc.layers.len();
    let err = s.execute("file.import.videoFramesToLayers", json!({"path": path})).unwrap_err().to_string();
    assert!(err.contains("only the first 2 pages"), "{err}");
    assert_eq!(s.active().unwrap().doc.layers.len(), before, "nothing imported");
}

/// #10897 review: a page refused by the PSD memory admission must reach the caller typed, so the
/// UI, control channel and agents can offer approval or explain the hard limit; a multi-page TIFF
/// is not a way to get a text-only refusal. Every other page failure keeps its file and page text.
#[test]
fn tiff_page_errors_keep_memory_refusals_typed_and_other_context() {
    let display = std::path::Path::new("/tmp/footage/pages.tif");
    let name = "/tmp/footage/pages.tif";

    let refusal = photocraft_io::IoError::PsdApprovalRequired { required: 9_000, soft_limit: 8_000, hard_limit: 16_000, available_ram: Some(4_000) };
    match tiff_page_error(display, name, 1, refusal) {
        EngineError::PsdApprovalRequired { name: named, required, soft_limit, hard_limit, available_ram } => {
            assert_eq!(named, name);
            assert_eq!((required, soft_limit, hard_limit, available_ram), (9_000, 8_000, 16_000, Some(4_000)));
        }
        other => panic!("page 2 approval refusal lost its fields: {other:?}"),
    }

    let hard = photocraft_io::IoError::PsdImportBudget { required: 9_000, limit: 8_000, free_memory: None };
    match tiff_page_error(display, name, 2, hard) {
        EngineError::PsdHardBudget { name: named, required, limit, .. } => {
            assert_eq!(named, name);
            assert_eq!((required, limit), (9_000, 8_000));
        }
        other => panic!("page 3 hard refusal lost its fields: {other:?}"),
    }

    let broken = photocraft_io::IoError::Unsupported("corrupt page".into());
    let err = tiff_page_error(display, name, 0, broken);
    assert!(matches!(err, EngineError::Other(_)), "{err:?}");
    let msg = err.to_string();
    assert!(msg.contains("pages.tif") && msg.contains("page 1") && msg.contains("corrupt page"), "{msg}");
}

/// #10897 review: a multi-page TIFF page with Photoshop layer data is admitted like a PSD. A tiny
/// explicit caller limit must reach the caller as the typed `PsdHardBudget`, with the file name,
/// required bytes and limit intact, and the document must be left untouched. A helper-only test
/// would still pass if the page loop turned that refusal into text.
#[test]
fn video_frames_to_layers_keeps_a_layered_tiff_page_refusal_typed() {
    let dir = tmpdir("tiff-typed");
    let path = format!("{dir}/layered.tif");
    write_layered_multipage_tiff(&path);
    let mut s = session();
    let before_layers = s.active().unwrap().doc.layers.len();
    let before_active = s.active().unwrap().active_layer;
    let _limit = super::limits::Scoped::psd_max_pixel_bytes(1);
    let err = s.execute("file.import.videoFramesToLayers", json!({"path": path})).unwrap_err();
    match err {
        EngineError::PsdHardBudget { name, required, limit, .. } => {
            assert_eq!(name, path);
            assert_eq!(limit, 1);
            assert!(required > limit, "the page estimate must be past the caller's limit: {required} > {limit}");
        }
        other => panic!("the layered page's refusal lost its typed fields: {other:?}"),
    }
    assert_eq!(s.active().unwrap().doc.layers.len(), before_layers, "no partial frame layers");
    assert_eq!(s.active().unwrap().active_layer, before_active, "the active layer is unchanged");
}

/// #10897 review: every page of a multi-page TIFF goes through the footage `keep` budget, as a
/// GIF's frames do. Before, the pages were pushed straight into `frames`, so many large pages could
/// keep more than the cap without an error. A thread-local test scope lowers the cap, so the
/// regression needs a few tiles of memory rather than gibibytes.
#[test]
fn video_frames_to_layers_enforces_the_footage_limit_on_every_tiff_page() {
    let dir = tmpdir("tiff-budget");
    let path = format!("{dir}/pages.tif");
    let mut s = session();
    let fmt = s.active().unwrap().doc.pixel_format();
    let tile = u64::from(photocraft_geom::TILE_SIZE.unsigned_abs()).pow(2) * fmt.bytes_per_pixel() as u64;
    let budget = 1u64 << 20;
    assert!(tile < budget, "one page must fit in the lowered budget");
    // One more page than fits in the budget: only the accumulation can refuse it.
    let pages = usize::try_from(budget / tile + 1).expect("a small page count");
    write_tiff(&path, &vec![(16, 16, 40, 0); pages]);
    let before = s.active().unwrap().doc.layers.len();
    let _limit = super::limits::Scoped::max_footage_bytes(budget);
    let err = s.execute("file.import.videoFramesToLayers", json!({"path": path})).unwrap_err().to_string();
    assert!(err.contains("pages.tif") && err.contains("more than 1 MiB"), "{err}");
    assert_eq!(s.active().unwrap().doc.layers.len(), before, "nothing imported");
}

/// #10897 review: a single-page layered TIFF does not take the multi-page branch, so it falls
/// through to the ordinary single-file import. That import must carry the footage import settings,
/// including the PSD/layered-TIFF memory limit the caller lowered; before, it called the default
/// import and ignored the limit, so this refusal was never typed (and a caller limit could be
/// bypassed). The same file imports as one frame when the limit is not lowered.
#[test]
fn video_frames_to_layers_uses_the_footage_limit_for_a_single_page_layered_tiff() {
    let dir = tmpdir("tiff-single-typed");
    let path = format!("{dir}/layered.tif");
    std::fs::write(&path, layered_tiff_bytes()).expect("write the layered TIFF");
    let mut s = session();
    let before_layers = s.active().unwrap().doc.layers.len();
    let before_active = s.active().unwrap().active_layer;
    {
        let _limit = super::limits::Scoped::psd_max_pixel_bytes(1);
        let err = s.execute("file.import.videoFramesToLayers", json!({"path": path})).unwrap_err();
        match err {
            EngineError::PsdHardBudget { name, required, limit, .. } => {
                assert_eq!(name, path);
                assert_eq!(limit, 1);
                assert!(required > limit, "the page estimate must be past the caller's limit: {required} > {limit}");
            }
            other => panic!("the single-page layered TIFF's refusal lost its typed fields: {other:?}"),
        }
    }
    assert_eq!(s.active().unwrap().doc.layers.len(), before_layers, "no partial frame layers");
    assert_eq!(s.active().unwrap().active_layer, before_active, "the active layer is unchanged");
    // The same page imports normally under the budget the footage command always applied.
    let r = s.execute("file.import.videoFramesToLayers", json!({"path": path})).unwrap();
    assert_eq!(r["layers"], 1, "one page is one frame");
    assert_eq!(s.active().unwrap().doc.layers.len(), before_layers + 1);
}

#[test]
fn new_video_layer_from_image_sequence() {
    let dir = tmpdir("seq");
    write_png(&format!("{dir}/f01.png"), 16, 12, "#ff0000");
    write_png(&format!("{dir}/f02.png"), 16, 12, "#00ff00");
    write_png(&format!("{dir}/f03.png"), 16, 12, "#0000ff");
    let mut s = session();
    let r = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    assert_eq!(r["frames"], 3);
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(frames(&s, id), 3);
    assert!(s.active().unwrap().doc.timeline.as_ref().unwrap().duration >= 3);
}

/// CodeRabbit on #9775: footage that isn't on the active artboard stays out of it, as a placed
/// image does; the board would clip it. Before, the layer was placed while still empty.
#[test]
fn footage_off_the_active_artboard_stays_out_of_it() {
    let dir = tmpdir("board");
    write_png(&format!("{dir}/f01.png"), 16, 12, "#ff0000");
    write_png(&format!("{dir}/f02.png"), 16, 12, "#00ff00");
    let mut s = session();
    let board = s.execute("layer.new.artboard", json!({"rect": [0, 0, 4, 4]})).unwrap();
    assert!(board["layer"].as_u64().is_some(), "{board}");
    let r = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(s.active().unwrap().doc.artboard_of(id), None, "the footage is centred off the board");
}

#[test]
fn frames_to_layers_and_render_video() {
    let dir = tmpdir("ftl");
    write_png(&format!("{dir}/a.png"), 10, 8, "#112233");
    write_png(&format!("{dir}/b.png"), 10, 8, "#445566");
    let mut s = session();
    let r = s.execute("file.import.videoFramesToLayers", json!({"path": dir})).unwrap();
    assert_eq!(r["layers"], 2);
    // Build a 2-frame video layer and render.
    let v = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    assert_eq!(v["frames"], 2);
    let out = tmpdir("render");
    let rr = s.execute("file.export.renderVideo", json!({"dir": out, "format": "png"})).unwrap();
    assert!(rr["frames"].as_u64().unwrap() >= 2);
    assert!(std::path::Path::new(&out).exists());
}

#[test]
fn render_video_to_animated_gif() {
    let dir = tmpdir("gifsrc");
    write_png(&format!("{dir}/a.png"), 10, 8, "#ff0000");
    write_png(&format!("{dir}/b.png"), 10, 8, "#00ff00");
    write_png(&format!("{dir}/c.png"), 10, 8, "#0000ff");
    let mut s = session();
    s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    let out = tmpdir("gifout");
    let r = s.execute("file.export.renderVideo", json!({"dir": out, "format": "gif"})).unwrap();
    assert_eq!(r["frames"], 3);
    let gif = std::fs::read(r["file"].as_str().unwrap()).unwrap();
    assert_eq!(&gif[..6], b"GIF89a");
    assert_eq!(gif.iter().filter(|&&b| b == 0x2C).count(), 3); // 3 frames
}

#[test]
fn animated_gif_keeps_transparency() {
    // #3061: a transparent document rendered as an animated GIF came out opaque black.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
    s.execute("timeline.create", json!({"duration": 2, "fps": 10})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let mut gif = Vec::new();
    render_video_with(
        &s,
        &json!({"format": "gif"}),
        |_, bytes| {
            gif = bytes.to_vec();
            Ok(())
        },
        |_, _| true,
    )
    .unwrap();
    // Each frame has a transparent index and is cleared before the next (disposal 2).
    let packed: Vec<u8> = gif.windows(4).filter(|w| w[..3] == [0x21, 0xF9, 0x04]).map(|w| w[3]).collect();
    assert_eq!(packed, [0x09, 0x09]);
    let px = photocraft_compose::flatten(&photocraft_io::import("t.gif", &gif).unwrap().document).into_px();
    assert_eq!(px[0][3], 0.0, "the corner stays transparent");
    assert_eq!(px[3 * 8 + 3], [1.0, 0.0, 0.0, 1.0], "the square stays red");
}

#[test]
fn a_fully_transparent_animated_gif_frame_encodes() {
    let px = vec![[0.0f32; 4]; 4];
    let f = gif_frame(px, 2, 5).unwrap();
    assert_eq!(f.transparent, Some((f.palette.len() - 1) as u8));
    assert!(f.indices.iter().all(|&i| Some(i) == f.transparent));
}

/// The GIF frame delays (centiseconds, from each Graphic Control Extension) of a Render Video.
fn rendered_gif_delays(s: &Session) -> Vec<u16> {
    let mut gif = Vec::new();
    render_video_with(
        s,
        &json!({"format": "gif"}),
        |_, bytes| {
            gif = bytes.to_vec();
            Ok(())
        },
        |_, _| true,
    )
    .unwrap();
    gif.windows(6).filter(|w| w[..3] == [0x21, 0xF9, 0x04]).map(|w| u16::from_le_bytes([w[4], w[5]])).collect()
}

#[test]
fn animated_gif_delay_follows_timelines_under_one_fps() {
    // 0.5 fps is a valid timeline rate; its frames last 2 s, not the 1 s a 1 fps floor gave.
    let mut s = session();
    s.execute("timeline.create", json!({"duration": 2, "fps": 0.5})).unwrap();
    assert_eq!(rendered_gif_delays(&s), [200, 200]);
    // A loaded timeline with an unusable rate falls back to 30 fps (3 cs), as with no timeline.
    let mut doc = (*s.active().unwrap().doc).clone();
    doc.timeline = Some(Timeline::new(2, f32::NAN));
    {
        let st = s.active_mut().unwrap();
        st.doc = std::sync::Arc::new(doc);
        st.revision += 1;
    }
    assert_eq!(rendered_gif_delays(&s), [3, 3]);
}

#[test]
fn inspect_reports_video_layer() {
    let dir = tmpdir("vinsp");
    write_png(&format!("{dir}/a.png"), 6, 4, "#ff0000");
    write_png(&format!("{dir}/b.png"), 6, 4, "#00ff00");
    let mut s = session();
    s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    let r = s.execute("document.inspect", json!({})).unwrap();
    // Find any layer node carrying a "video" block.
    fn find_video(v: &serde_json::Value) -> Option<serde_json::Value> {
        if let Some(vid) = v.get("video") {
            return Some(vid.clone());
        }
        v.get("children").and_then(|c| c.as_array()).and_then(|a| a.iter().find_map(find_video))
    }
    let layers = r["layers"].as_array().expect("layers array");
    let vid = layers.iter().find_map(find_video).expect("a video layer is reported");
    assert_eq!(vid["frames"], 2);
    assert_eq!(vid["source"]["file"].as_str(), Some(dir.as_str()));
}

fn layer_px(s: &Session, id: LayerId) -> Vec<f32> {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Raster(r) => r.pixel(2, 2),
        _ => panic!("not a raster"),
    }
}

/// #1288: a fill on a video frame survives scrubbing away and back, and reaches Render Video.
#[test]
fn edits_to_a_frame_survive_scrubbing_and_render() {
    let mut s = session();
    let r = s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    s.execute("layer.videoLayers.insertBlankFrame", json!({})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 0})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    assert!(layer_px(&s, id)[3] < 0.01, "frame 1 stays blank");
    s.execute("timeline.setFrame", json!({"frame": 0})).unwrap();
    let p = layer_px(&s, id);
    assert!(p[0] > 0.99 && p[3] > 0.99, "frame 0 keeps its fill: {p:?}");
    // Duplicate Frame copies the edited frame.
    s.execute("layer.videoLayers.duplicateFrame", json!({})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    assert!(layer_px(&s, id)[0] > 0.99);

    // Render Video picks up an edit made at the playhead without scrubbing first.
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    let out = tmpdir("edited");
    s.execute("file.export.renderVideo", json!({"dir": out, "format": "png"})).unwrap();
    let stem = s.active().unwrap().doc.name.rsplit_once('.').map_or(s.active().unwrap().doc.name.clone(), |(a, _)| a.to_string());
    let f1 = std::fs::read(format!("{out}/{stem}_0001.png")).unwrap();
    let doc = photocraft_io::import("f1.png", &f1).unwrap().document;
    let flat = crate::file_cmds::flattened(&doc, doc.pixel_format());
    let q = flat.pixel(2, 2);
    assert!(q[2] > 0.99 && q[0] < 0.01, "rendered frame 1 is blue: {q:?}");
}

#[test]
fn render_writer_progress_and_cancellation_leave_session_unchanged() {
    let mut s = session();
    s.execute("timeline.create", json!({"duration": 4, "fps": 4})).unwrap();
    let before = s.active().unwrap().doc.clone();
    for format in ["png", "gif"] {
        let mut names = Vec::new();
        let mut progress = Vec::new();
        let r = render_video_with(
            &s,
            &json!({"format":format}),
            |name, bytes| {
                assert!(!bytes.is_empty());
                names.push(name.to_owned());
                Ok(())
            },
            |done, total| {
                progress.push((done, total));
                done < 2
            },
        );
        assert!(r.unwrap_err().to_string().contains("cancelled"));
        assert!(names.len() < 4);
        assert_eq!(progress.first(), Some(&(0, 4)));
        assert_eq!(progress.last(), Some(&(2, 4)));
        assert_eq!(s.active().unwrap().doc.timeline, before.timeline);
    }
}

/// A 16×12 32-bit Targa: red, alpha 255 on the left half and 0 on the right.
fn write_tga(path: &str) {
    let mut b = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16, 0, 12, 0, 32, 8];
    for _y in 0..12 {
        for x in 0..16 {
            b.extend_from_slice(&[0, 0, 255, if x < 8 { 255 } else { 0 }]);
        }
    }
    std::fs::write(path, b).unwrap();
}

/// #2225: a Targa opens with its alpha as an "Alpha 1" channel (as in Photoshop), but an image
/// sequence treats a frame's alpha as transparency, as video footage does, so TGA sequences
/// keep their transparent areas.
#[test]
fn tga_sequence_frames_keep_their_transparency() {
    let dir = tmpdir("tga");
    write_tga(&format!("{dir}/f01.tga"));
    write_tga(&format!("{dir}/f02.tga"));
    let frames = load_frames(
        &dir,
        photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8, true),
        &photocraft_io::ImportOptions::default(),
    )
    .unwrap()
    .frames;
    assert_eq!(frames.len(), 2);
    for f in &frames {
        assert_eq!(f.pixel(2, 6), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(f.pixel(12, 6)[3], 0.0, "the right half stays transparent");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ animated GIF footage

/// An animated GIF of solid `w`x`h` frames: one `(colour, delay in centiseconds)` each.
fn write_gif(path: &str, w: u32, h: u32, frames: &[([u8; 3], u16)]) {
    use photocraft_codecs::web::{GifFrame, encode_gif_animated};
    let list: Vec<GifFrame> =
        frames.iter().map(|&(c, delay_cs)| GifFrame { indices: vec![0; (w * h) as usize], palette: vec![c], transparent: None, delay_cs }).collect();
    std::fs::write(path, encode_gif_animated(w, h, &list, true).unwrap()).unwrap();
}

fn frame_rgb(s: &Session, id: LayerId, frame: usize) -> Vec<f32> {
    let v = s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap();
    v.frames[frame].pixel(1, 1)[..3].to_vec()
}

#[test]
fn an_animated_gif_becomes_a_video_layer_with_every_frame() {
    let dir = tmpdir("gifin");
    let path = format!("{dir}/anim.gif");
    write_gif(&path, 16, 12, &[([255, 0, 0], 4), ([0, 255, 0], 4), ([0, 0, 255], 4)]);
    let mut s = session();
    let r = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": path})).unwrap();
    assert_eq!(r["frames"], 3);
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(frames(&s, id), 3);
    assert_eq!(frame_rgb(&s, id, 0), [1.0, 0.0, 0.0]);
    assert_eq!(frame_rgb(&s, id, 1), [0.0, 1.0, 0.0]);
    assert_eq!(frame_rgb(&s, id, 2), [0.0, 0.0, 1.0]);
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.timeline.as_ref().unwrap().duration, 3);
    // 4 cs = 40 ms per frame: 25 fps, on the timeline and on the layer.
    assert_eq!(doc.timeline.as_ref().unwrap().fps, 25.0);
    assert_eq!(doc.layer(id).unwrap().video.as_ref().unwrap().fps, 25.0);
    // One history step: undo takes the layer back.
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layer(id).is_none());
}

#[test]
fn an_explicit_fps_beats_the_gif_delays() {
    let dir = tmpdir("gifin-fps");
    let path = format!("{dir}/anim.gif");
    write_gif(&path, 8, 8, &[([255, 0, 0], 10), ([0, 255, 0], 10)]);
    let mut s = session();
    let r = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": path, "fps": 12.0})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().fps, 12.0);
}

#[test]
fn a_still_gif_is_one_frame_at_the_default_rate() {
    let dir = tmpdir("gifin-still");
    let path = format!("{dir}/still.gif");
    write_gif(&path, 8, 8, &[([255, 0, 0], 0)]);
    let mut s = session();
    let r = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": path})).unwrap();
    assert_eq!(r["frames"], 1);
    // A 0 delay is shown at 100 ms, so 10 fps.
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().fps, 10.0);
}

/// Interpret Footage and New Video Layer from File reject an fps that isn't a finite number in
/// `(0, MAX_FPS]` and keep the layer as it was (#5657). 1e39 used to become an infinite rate the
/// next `.pcraft` save refused.
#[test]
fn an_invalid_frame_rate_is_an_error_and_changes_nothing() {
    let mut s = session();
    let id = LayerId(s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap()["layer"].as_u64().unwrap());
    let fps = |s: &Session| s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().fps;
    assert_eq!(fps(&s), 30.0);
    for bad in [json!(0), json!(-1), json!(1e39), json!("x")] {
        assert!(s.execute("layer.videoLayers.interpretFootage", json!({"fps": bad})).is_err(), "fps {bad}");
        assert_eq!(fps(&s), 30.0, "fps {bad}");
    }
    s.execute("layer.videoLayers.interpretFootage", json!({"fps": 24})).unwrap();
    assert_eq!(fps(&s), 24.0);

    let dir = tmpdir("gifin-badfps");
    let path = format!("{dir}/anim.gif");
    write_gif(&path, 8, 8, &[([255, 0, 0], 10), ([0, 255, 0], 10)]);
    let layers = s.active().unwrap().doc.layer_count();
    for bad in [json!(0), json!(-1), json!(1e39), json!("x")] {
        assert!(s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": path, "fps": bad})).is_err(), "fps {bad}");
        assert_eq!(s.active().unwrap().doc.layer_count(), layers, "fps {bad}");
    }
}

#[test]
fn video_frames_to_layers_takes_every_gif_frame() {
    let dir = tmpdir("gifin-ftl");
    let path = format!("{dir}/anim.gif");
    write_gif(&path, 8, 8, &[([255, 0, 0], 5), ([0, 255, 0], 5), ([0, 0, 255], 5), ([9, 9, 9], 5)]);
    let mut s = session();
    let r = s.execute("file.import.videoFramesToLayers", json!({"path": path})).unwrap();
    assert_eq!(r["layers"], 4);
}

// ------------------------------------------------------------------ animated WebP footage (#1198)

/// An animated WebP of solid `w`x`h` frames: one `(colour, duration in ms)` each, every frame a
/// lossless still in an ANMF chunk.
fn write_webp(path: &str, w: u32, h: u32, frames: &[([u8; 3], u32)]) {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, encode};
    let chunk = |id: &[u8; 4], data: &[u8]| {
        let mut c = id.to_vec();
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(data);
        if data.len() % 2 == 1 {
            c.push(0);
        }
        c
    };
    let u24 = |v: u32| v.to_le_bytes()[..3].to_vec();
    let mut body = b"WEBP".to_vec();
    body.extend(chunk(b"VP8X", &[[0x02, 0, 0, 0].as_slice(), &u24(w - 1), &u24(h - 1)].concat()));
    body.extend(chunk(b"ANIM", &[0, 0, 0, 0, 0, 0]));
    for &(c, ms) in frames {
        let img = Image::from_u8(w, h, ChannelLayout::Rgb, c.repeat((w * h) as usize)).unwrap();
        let still = encode(&img, Format::WebP, &EncodeOptions::default()).unwrap();
        let at = still.windows(4).position(|w| w == b"VP8L").unwrap();
        let len = u32::from_le_bytes(still[at + 4..at + 8].try_into().unwrap()) as usize;
        let anmf = [[0; 6].as_slice(), &u24(w - 1), &u24(h - 1), &u24(ms), &[0x02], &chunk(b"VP8L", &still[at + 8..at + 8 + len])].concat();
        body.extend(chunk(b"ANMF", &anmf));
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    std::fs::write(path, out).unwrap();
}

#[test]
fn an_animated_webp_becomes_a_video_layer_with_every_frame() {
    let dir = tmpdir("webpin");
    let path = format!("{dir}/anim.webp");
    write_webp(&path, 16, 12, &[([255, 0, 0], 50), ([0, 255, 0], 50), ([0, 0, 255], 80)]);
    let mut s = session();
    let r = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": path})).unwrap();
    assert_eq!(r["frames"], 3);
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(frame_rgb(&s, id, 0), [1.0, 0.0, 0.0]);
    assert_eq!(frame_rgb(&s, id, 1), [0.0, 1.0, 0.0]);
    assert_eq!(frame_rgb(&s, id, 2), [0.0, 0.0, 1.0]);
    // The most common duration, 50 ms: 20 fps.
    assert_eq!(s.active().unwrap().doc.timeline.as_ref().unwrap().fps, 20.0);
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layer(id).is_none());
}

#[test]
fn video_frames_to_layers_takes_every_webp_frame() {
    let dir = tmpdir("webpin-ftl");
    let path = format!("{dir}/anim.webp");
    write_webp(&path, 8, 8, &[([255, 0, 0], 40), ([0, 255, 0], 40), ([0, 0, 255], 40)]);
    let mut s = session();
    let r = s.execute("file.import.videoFramesToLayers", json!({"path": path})).unwrap();
    assert_eq!(r["layers"], 3);
}

#[test]
fn a_still_webp_is_one_frame_at_the_default_30_fps() {
    let dir = tmpdir("webpin-still");
    let path = format!("{dir}/still.webp");
    let img = photocraft_codecs::Image::from_u8(8, 8, photocraft_codecs::ChannelLayout::Rgb, [255, 0, 0].repeat(64)).unwrap();
    std::fs::write(&path, photocraft_codecs::encode(&img, photocraft_codecs::Format::WebP, &Default::default()).unwrap()).unwrap();
    let mut s = session();
    let r = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": path})).unwrap();
    assert_eq!(r["frames"], 1);
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().fps, 30.0);
}

#[test]
fn a_gif_rendered_by_render_video_loads_back_frame_for_frame() {
    let src = tmpdir("gifrt-src");
    write_png(&format!("{src}/a.png"), 10, 8, "#ff0000");
    write_png(&format!("{src}/b.png"), 10, 8, "#0000ff");
    let mut s = session();
    s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": src, "fps": 20.0})).unwrap();
    let out = tmpdir("gifrt-out");
    let r = s.execute("file.export.renderVideo", json!({"dir": out, "format": "gif"})).unwrap();
    let mut t = session();
    let back = t.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": r["file"]})).unwrap();
    assert_eq!(back["frames"], 2);
    let id = LayerId(back["layer"].as_u64().unwrap());
    assert_eq!(t.active().unwrap().doc.timeline.as_ref().unwrap().fps, 20.0);
    // GIF palettes are 8-bit: the colours come back to within a quantization step.
    let (a, b) = (frame_rgb(&t, id, 0), frame_rgb(&t, id, 1));
    assert!(a[0] > 0.9 && a[2] < 0.1, "{a:?}");
    assert!(b[2] > 0.9 && b[0] < 0.1, "{b:?}");
}

#[test]
fn a_broken_gif_is_an_error_not_a_panic() {
    let dir = tmpdir("gifin-bad");
    let path = format!("{dir}/bad.gif");
    std::fs::write(&path, b"GIF89a\x01\x00").unwrap();
    let mut s = session();
    assert!(s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": path})).is_err());
}

#[test]
fn gif_delays_map_to_one_frame_rate() {
    // The most common delay wins; the first seen on a tie.
    assert_eq!(fps_from_delays(&[40, 40, 100]), Some(25.0));
    assert_eq!(fps_from_delays(&[100, 40]), Some(10.0));
    // Delays under 20 ms (0 and 10 are common) play at 100 ms, as in browsers.
    assert_eq!(fps_from_delays(&[0, 10, 0]), Some(10.0));
    assert_eq!(fps_from_delays(&[]), None);
}

#[test]
fn imported_video_restore_frame_and_all_keep_source_pixels() {
    let dir = tmpdir("original-footage");
    write_png(&format!("{dir}/f0.png"), 8, 8, "#ff0000");
    write_png(&format!("{dir}/f1.png"), 8, 8, "#00ff00");
    let mut s = session();
    let id = LayerId(s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(layer_px(&s, id), vec![1.0, 0.0, 0.0, 1.0]);

    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.videoLayers.restoreFrame", json!({})).unwrap();
    assert_eq!(layer_px(&s, id), vec![1.0, 0.0, 0.0, 1.0], "current frame restores the red original");

    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.videoLayers.restoreAllFrames", json!({})).unwrap();
    assert_eq!(layer_px(&s, id), vec![0.0, 1.0, 0.0, 1.0], "frame 1 restores green");
    s.execute("timeline.setFrame", json!({"frame": 0})).unwrap();
    assert_eq!(layer_px(&s, id), vec![1.0, 0.0, 0.0, 1.0], "frame 0 restores red");
    assert!(s.undo(), "restoration stays in history");
}

#[test]
fn show_altered_video_switches_between_original_and_edited_frames() {
    let dir = tmpdir("show-altered-original");
    write_png(&format!("{dir}/f0.png"), 8, 8, "#ff0000");
    write_png(&format!("{dir}/f1.png"), 8, 8, "#00ff00");
    let mut s = session();
    let id = LayerId(s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap()["layer"].as_u64().unwrap());
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    assert_eq!(layer_px(&s, id), vec![0.0, 0.0, 1.0, 1.0]);

    s.execute("layer.videoLayers.showAlteredVideo", json!({})).unwrap();
    assert!(!s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().show_altered);
    assert_eq!(layer_px(&s, id), vec![1.0, 0.0, 0.0, 1.0], "hidden altered footage shows red original");
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    assert_eq!(layer_px(&s, id), vec![0.0, 1.0, 0.0, 1.0], "scrubbing while hidden shows green original");
    s.execute("timeline.setFrame", json!({"frame": 0})).unwrap();
    s.execute("layer.videoLayers.showAlteredVideo", json!({})).unwrap();
    assert_eq!(layer_px(&s, id), vec![0.0, 0.0, 1.0, 1.0], "switching back reveals the blue edit");
}

#[test]
fn restoring_blank_video_frames_still_produces_blank_frames() {
    let mut s = session();
    let id = LayerId(s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.videoLayers.restoreFrame", json!({})).unwrap();
    assert_eq!(layer_px(&s, id)[3], 0.0);
}

fn work_area(s: &Session) -> (usize, usize, usize) {
    let t = s.active().unwrap().doc.timeline.as_ref().unwrap();
    (t.work_start, t.work_end, t.duration)
}

#[test]
fn video_edits_keep_a_narrowed_work_area() {
    // #4170: a video edit pushed the work-area end to the full duration.
    for (cmd, params) in [
        ("layer.videoLayers.showAlteredVideo", json!({})),
        ("layer.videoLayers.restoreFrame", json!({})),
        ("layer.videoLayers.interpretFootage", json!({"fps": 24})),
    ] {
        let mut s = session();
        s.execute("timeline.create", json!({"duration": 30, "fps": 30})).unwrap();
        s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
        s.execute("timeline.setProps", json!({"workStart": 2, "workEnd": 10})).unwrap();
        assert_eq!(work_area(&s), (2, 10, 30));
        s.execute(cmd, params).unwrap();
        assert_eq!(work_area(&s), (2, 10, 30), "{cmd} moved the work area");
    }
}

#[test]
fn a_full_work_area_grows_with_the_duration() {
    let mut s = session();
    s.execute("timeline.create", json!({"duration": 30, "fps": 30})).unwrap();
    s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
    assert_eq!(work_area(&s), (0, 30, 30));
    s.execute("layer.videoLayers.insertBlankFrame", json!({})).unwrap();
    assert_eq!(work_area(&s), (0, 31, 31));
    // A narrowed work area stays put when the duration grows.
    s.execute("timeline.setProps", json!({"workStart": 2, "workEnd": 10})).unwrap();
    s.execute("layer.videoLayers.insertBlankFrame", json!({})).unwrap();
    assert_eq!(work_area(&s), (2, 10, 32));
}

#[test]
fn longer_footage_grows_only_a_full_work_area() {
    let dir = tmpdir("workarea");
    for i in 0..5 {
        write_png(&format!("{dir}/f{i:02}.png"), 16, 12, "#808080");
    }
    // A work area covering everything follows the longer footage.
    let mut s = session();
    s.execute("timeline.create", json!({"duration": 3, "fps": 30})).unwrap();
    s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    assert_eq!(work_area(&s), (0, 5, 5));
    // A narrowed one stays put.
    let mut s = session();
    s.execute("timeline.create", json!({"duration": 3, "fps": 30})).unwrap();
    s.execute("timeline.setProps", json!({"workStart": 1, "workEnd": 2})).unwrap();
    s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    assert_eq!(work_area(&s), (1, 2, 5));
}

#[test]
fn grow_duration_never_shrinks_and_keeps_the_work_area_valid() {
    let mut t = Timeline::new(10, 30.0);
    grow_duration(&mut t, 4);
    assert_eq!((t.duration, t.work_end), (10, 10));
    grow_duration(&mut t, 0);
    assert_eq!((t.duration, t.work_end), (10, 10));
    t.work_end = 6;
    grow_duration(&mut t, 12);
    assert_eq!((t.work_start, t.work_end, t.duration), (0, 6, 12));
}

#[test]
fn render_video_rejects_a_loaded_timeline_past_the_frame_cap() {
    let mut s = session();
    let mut doc = (*s.active().unwrap().doc).clone();
    doc.timeline = Some(Timeline::new(usize::MAX, 30.0));
    {
        let st = s.active_mut().unwrap();
        st.doc = std::sync::Arc::new(doc);
        st.revision += 1;
    }
    let err = s.execute("file.export.renderVideo", json!({"dir": tmpdir("overlong"), "format": "gif"})).unwrap_err();
    assert!(err.to_string().contains("limited to"), "{err}");
}

#[test]
fn render_video_rejects_gif_frames_too_large_to_stage() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 9000, "height": 8000})).unwrap();
    let err = s.execute("file.export.renderVideo", json!({"dir": tmpdir("large-gif"), "format": "gif"})).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
}

#[test]
fn render_video_rejects_a_gif_whose_frames_together_are_too_large() {
    // Each 4000 x 4000 frame is under the per-frame cap, but 100,000 of them keep ~1.6 TB staged.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4000, "height": 4000})).unwrap();
    s.execute("timeline.create", json!({"duration": 100_000, "fps": 30})).unwrap();
    let err = s.execute("file.export.renderVideo", json!({"dir": tmpdir("many-gif"), "format": "gif"})).unwrap_err();
    assert!(err.to_string().contains("staged in total"), "{err}");
}

#[test]
fn gif_budget_boundaries() {
    // Sane work is untouched: a minute of 1080p at 30 fps, and the existing small cases.
    assert!(check_gif_budget(1920, 1080, 1000).is_ok());
    assert!(check_gif_budget(16, 12, 1).is_ok());
    // Exactly at the limit passes, one frame more fails.
    let per_frame = 1000 * 1000 + 768;
    let n = MAX_GIF_STAGED_BYTES / per_frame;
    assert!(check_gif_budget(1000, 1000, n).is_ok());
    assert!(check_gif_budget(1000, 1000, n + 1).is_err());
    // The per-frame cap and the 16-bit GIF side still apply, and nothing overflows.
    assert!(check_gif_budget(9000, 8000, 1).is_err());
    assert!(check_gif_budget(65_536, 1, 1).is_err());
    assert!(check_gif_budget(65_535, 65_535, u64::MAX).is_err());
    assert!(check_gif_budget(1, 1, u64::MAX).is_err());
}

#[test]
fn fps_from_delays_picks_most_common_and_breaks_ties_by_first_seen() {
    // Most common delay wins: 100 ms appears twice, 40 ms once -> 10 fps.
    assert_eq!(fps_from_delays(&[100, 40, 100]), Some(10.0));
    // Tie between 40 ms (25 fps) and 50 ms (20 fps): the first seen (40) wins.
    assert_eq!(fps_from_delays(&[40, 50]), Some(25.0));
    // Delays under 20 ms are treated as 100 ms.
    assert_eq!(fps_from_delays(&[0, 10, 5]), Some(10.0));
    assert_eq!(fps_from_delays(&[]), None);
    // Many distinct delays used to be quadratic; this must stay fast and correct. Delay 20
    // (the one appearing twice) wins at 50 fps.
    let mut many: Vec<u32> = (20..5_020).collect();
    many.push(20);
    assert_eq!(fps_from_delays(&many), Some(50.0));
}

#[test]
fn render_video_names_frames_after_the_leaf_of_the_document_name() {
    assert_eq!(safe_stem("clip.mov"), "clip");
    assert_eq!(safe_stem("../../outside/clip.png"), "clip");
    assert_eq!(safe_stem("C:\\Users\\x\\clip.v2.gif"), "clip.v2");
    for hostile in ["..", "../..", "", "/", "a/..", ".hidden."] {
        let stem = safe_stem(hostile);
        assert!(!stem.contains(['/', '\\']) && !stem.is_empty() && stem.chars().any(|c| c != '.'), "{hostile:?} -> {stem:?}");
    }
}
