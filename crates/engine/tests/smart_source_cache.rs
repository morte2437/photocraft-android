//! #10030: the decoded-source cache counts the bytes it holds, and Purge All and closing a
//! document with smart objects release them. One integration-test process keeps the global
//! cache isolated from parallel unit tests; the tests here take turns on it.
use std::sync::Mutex;

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
use photocraft_engine::{Session, file_cmds, smart_cmds};
use serde_json::json;

static TURN: Mutex<()> = Mutex::new(());

const SIDE: u32 = 256;

fn png() -> Vec<u8> {
    let px = (0..SIDE * SIDE).flat_map(|i| [(i % SIDE) as u8, (i / SIDE) as u8, ((i * 7) % 251) as u8, 255]);
    let img = Image::from_raw(SIDE, SIDE, ChannelLayout::Rgba, SampleType::U8, px.collect()).unwrap();
    photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap()
}

/// Every cache `edit.purge.all` clears, emptied.
fn empty_caches() {
    smart_cmds::purge_source_cache();
    smart_cmds::purge_stack_cache();
    photocraft_compose::purge_effect_cache();
    photocraft_compose::masks::purge_cache();
    photocraft_compose::shape_split::purge_cache();
}

fn session_with_smart_object(bytes: &[u8]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": SIDE, "height": SIDE})).unwrap();
    file_cmds::place_bytes(&mut s, "art.png", bytes.to_vec(), None, &json!({})).unwrap();
    s
}

fn decode(s: &Session, bytes: &[u8]) {
    smart_cmds::source_image("art.png", bytes, s.active().unwrap().doc.pixel_format()).unwrap();
}

#[test]
fn the_cache_counts_real_tile_bytes() {
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    empty_caches();
    let bytes = png();
    let s = session_with_smart_object(&bytes);
    decode(&s, &bytes);
    let held = smart_cmds::source_cache_bytes();
    let px = (SIDE * SIDE) as usize;
    assert!(held >= 4 * px && held < 5 * px, "{held} bytes for {px} 8-bit RGBA pixels");
    assert_eq!(smart_cmds::purge_source_cache(), held);
    assert_eq!(smart_cmds::source_cache_bytes(), 0);
}

#[test]
fn purge_all_reports_and_empties_the_source_cache() {
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    empty_caches();
    let bytes = png();
    let mut s = session_with_smart_object(&bytes);
    // Nothing else holds anything to purge: only the source cache enables Purge All.
    s.execute("edit.purge.histories", json!({})).unwrap();
    s.clipboard = None;
    assert!(!s.is_enabled("edit.purge.all"));
    decode(&s, &bytes);
    assert!(smart_cmds::source_cache_bytes() > 0);
    assert!(s.is_enabled("edit.purge.all"));
    let purged = s.execute("edit.purge.all", json!({})).unwrap();
    assert!(purged["purged"].as_array().unwrap().contains(&json!("smart object source cache")), "{purged}");
    assert_eq!(smart_cmds::source_cache_bytes(), 0);
    assert!(!s.is_enabled("edit.purge.all"));
}

#[test]
fn closing_a_document_with_a_smart_object_empties_the_source_cache() {
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    empty_caches();
    let bytes = png();
    let mut s = session_with_smart_object(&bytes);
    decode(&s, &bytes);
    assert!(smart_cmds::source_cache_bytes() > 0);
    s.execute("file.close", json!({})).unwrap();
    assert_eq!(smart_cmds::source_cache_bytes(), 0);

    // A document without smart objects leaves it alone.
    let mut plain = Session::new();
    plain.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let s = session_with_smart_object(&bytes);
    decode(&s, &bytes);
    plain.execute("file.close", json!({})).unwrap();
    assert!(smart_cmds::source_cache_bytes() > 0);
    empty_caches();
}
