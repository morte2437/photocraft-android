//! `import_gif_frames`: each GIF frame as a single-layer document, in order, with its delay,
//! and any decoder warning that survived a salvaged (truncated) import (#11113).

use photocraft_codecs::web::{GifFrame, encode_gif_animated};
use photocraft_io::{IoError, import_gif_frames};

fn gif(colours: &[([u8; 3], u16)]) -> Vec<u8> {
    let frames: Vec<GifFrame> = colours.iter().map(|&(c, delay_cs)| GifFrame { indices: vec![0; 6], palette: vec![c], transparent: None, delay_cs }).collect();
    encode_gif_animated(3, 2, &frames, true).unwrap()
}

#[test]
fn frames_arrive_in_order_as_documents_with_their_delays() {
    let bytes = gif(&[([255, 0, 0], 3), ([0, 0, 255], 7)]);
    let mut seen = Vec::new();
    let r = import_gif_frames("a.gif", &bytes, &mut |doc, delay_ms| {
        seen.push((doc.layers.len(), doc.bounds().width(), doc.bounds().height(), delay_ms));
        Ok(())
    })
    .unwrap();
    assert_eq!(r.frames, 2);
    assert_eq!(r.warnings, Vec::<String>::new(), "a complete GIF imports quietly");
    assert_eq!(seen, [(1, 3, 2, 30), (1, 3, 2, 70)]);
}

#[test]
fn a_truncated_gif_keeps_its_frames_and_reports_the_truncation() {
    // #11113: the codecs deliberately salvage the frames before the damage; the import must
    // carry the decoder's warning so the caller can disclose the shortened animation.
    let mut bytes = gif(&[([255, 0, 0], 3), ([0, 0, 255], 7), ([0, 255, 0], 7)]);
    bytes.truncate(bytes.len() - 4);
    let mut count = 0;
    let r = import_gif_frames("a.gif", &bytes, &mut |_, _| {
        count += 1;
        Ok(())
    })
    .unwrap();
    assert!(count < 3, "the damaged last frame is not imported ({count} arrived)");
    assert_eq!(r.frames, count);
    assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
    assert!(r.warnings[0].contains("ends early"), "{:?}", r.warnings);
}

#[test]
fn an_error_from_the_callback_stops_the_import() {
    let bytes = gif(&[([255, 0, 0], 3), ([0, 0, 255], 7), ([0, 255, 0], 7)]);
    let mut calls = 0;
    let r = import_gif_frames("a.gif", &bytes, &mut |_, _| {
        calls += 1;
        Err(IoError::Unsupported("stop".into()))
    });
    assert!(matches!(r, Err(IoError::Unsupported(_))));
    assert_eq!(calls, 1);
}

#[test]
fn garbage_is_an_error() {
    assert!(import_gif_frames("a.gif", b"not a gif", &mut |_, _| Ok(())).is_err());
}
