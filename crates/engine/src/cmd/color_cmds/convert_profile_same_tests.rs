//! #14261: converting a document to the profile it already has changes nothing: no history
//! state, no change to the document, its pixels or its profile, and the reply says so.

use super::*;

/// A 16×8 sRGB-tagged document with a red background.
fn tagged() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 8, "background": "#c81e1e"})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "srgb"})).unwrap();
    assert!(s.active().unwrap().doc.icc_profile.is_some(), "tagged");
    s
}

/// History length, dirty flag, embedded profile and composite pixels.
type State = (usize, bool, Option<std::sync::Arc<Vec<u8>>>, Vec<[f32; 4]>);

fn state(s: &Session) -> State {
    let d = s.active().unwrap();
    (d.history.entries().len(), d.is_dirty(), d.doc.icc_profile.clone(), photocraft_compose::flatten(&d.doc).px.to_vec())
}

#[test]
fn the_same_profile_converts_nothing() {
    let mut s = tagged();
    let before = state(&s);
    let r = s.execute("edit.convertToProfile", json!({"profile": "srgb"})).unwrap();
    assert_eq!(r["converted"], false, "{r}");
    assert_eq!(state(&s), before, "no history state, not dirtied, the same pixels and profile");
}

#[test]
fn a_different_profile_still_converts() {
    let mut s = tagged();
    let entries = s.active().unwrap().history.entries().len();
    let r = s.execute("edit.convertToProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
    assert_eq!(r["converted"], true, "{r}");
    let history = s.active().unwrap().history.entries();
    assert_eq!(history.len(), entries + 1);
    assert_eq!(history.last().map(String::as_str), Some("Convert to Profile"));
}

#[test]
fn flatten_with_the_same_profile_runs_as_before() {
    let mut s = tagged();
    let entries = s.active().unwrap().history.entries().len();
    let r = s.execute("edit.convertToProfile", json!({"profile": "srgb", "flatten": true})).unwrap();
    assert_eq!(r["converted"], true, "{r}");
    assert_eq!(s.active().unwrap().history.entries().len(), entries + 1);
}

/// An Indexed document tagged with the destination's profile still converts as before (to RGB):
/// the same-profile shortcut is only for the modes measured.
#[test]
fn an_indexed_document_with_the_same_profile_converts_as_before() {
    let mut s = tagged();
    s.execute("image.mode.indexedColor", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Indexed);
    s.execute("edit.assignProfile", json!({"profile": "srgb"})).unwrap();
    let r = s.execute("edit.convertToProfile", json!({"profile": "srgb"})).unwrap();
    assert_eq!(r["converted"], true, "{r}");
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Rgb);
}

/// An embedded profile that can't be read is not taken for the destination's colours: the
/// document is converted, and the profile replaced.
#[test]
fn an_unreadable_embedded_profile_is_still_replaced() {
    let mut s = tagged();
    s.edit("break the tag", |doc, _| {
        doc.icc_profile = Some(std::sync::Arc::new(b"not an ICC profile".to_vec()));
        Ok(())
    })
    .unwrap();
    let r = s.execute("edit.convertToProfile", json!({"profile": "srgb"})).unwrap();
    assert_eq!(r["converted"], true, "{r}");
    assert_ne!(s.active().unwrap().doc.icc_profile.as_deref().map(Vec::as_slice), Some(&b"not an ICC profile"[..]));
}

/// CodeRabbit on #14825: with Flatten on, an embedded profile of the same colours but other bytes
/// (here another creation date in its header) is replaced by the destination, as the conversion
/// would have, so the reply's profile is the document's.
#[test]
fn flatten_with_the_same_colours_embeds_the_destination_profile() {
    let mut s = tagged();
    let srgb = Builtin::Srgb.profile().to_bytes();
    let mut other = srgb.to_vec();
    // The header's creation date (bytes 24 to 35): not part of the colours.
    other[24..36].copy_from_slice(&[0x07, 0xd0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
    assert_ne!(other.as_slice(), srgb.as_slice());
    s.edit("retag", |doc, _| {
        doc.icc_profile = Some(std::sync::Arc::new(other));
        Ok(())
    })
    .unwrap();
    let r = s.execute("edit.convertToProfile", json!({"profile": "srgb", "flatten": true})).unwrap();
    assert_eq!(r["converted"], true, "{r}");
    assert_eq!(s.active().unwrap().doc.icc_profile.as_deref().map(Vec::as_slice), Some(srgb.as_slice()), "the destination's bytes");
}
