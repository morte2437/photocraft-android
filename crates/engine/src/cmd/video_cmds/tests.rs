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
    let px = photocraft_compose::flatten(&photocraft_io::import("t.gif", &gif).unwrap().document).px;
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
    let frames =
        load_frames(&dir, photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8, true)).unwrap().frames;
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
