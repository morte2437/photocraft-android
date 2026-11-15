//! Edit > Assign Profile's enabled state per colour mode, measured against the reference
//! application (#14238).
//!
//! The reference greys Assign Profile for Lab, Bitmap and Multichannel documents and with no
//! document open, and a script that runs it there fails. It stays enabled for RGB, CMYK,
//! Grayscale and Indexed documents at every depth, and Convert to Profile stays enabled for Lab.
//! Duotone is *not measured* and keeps today's behaviour (enabled).

use super::*;

const CMD: &str = "edit.assignProfile";

/// A white document of `mode` (as `file.new` names it) at `depth` bits.
fn session(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": mode, "depth": depth, "background": "white"})).unwrap();
    s
}

fn mode(s: &Session) -> ColorMode {
    s.active().unwrap().doc.mode
}

/// Greyed, and a scripted run is refused without touching the document's profile.
fn assert_refused(s: &mut Session, what: &str) {
    assert!(!s.is_enabled(CMD), "{what}: Assign Profile is greyed");
    let before = s.active().map(|st| st.doc.icc_profile.clone());
    let e = s.execute(CMD, json!({"profile": "none"})).unwrap_err();
    assert!(matches!(e, EngineError::Disabled(..)), "{what}: {e}");
    assert_eq!(s.active().map(|st| st.doc.icc_profile.clone()), before, "{what}: profile unchanged");
}

#[test]
fn no_document_greys_it_out() {
    assert_refused(&mut Session::new(), "no document");
}

#[test]
fn lab_greys_assign_but_not_convert() {
    for depth in [8, 16] {
        let mut s = session("lab", depth);
        assert_eq!(mode(&s), ColorMode::Lab);
        // `file.new` leaves Lab untagged; tag it, so "profile unchanged" compares a real profile.
        std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).icc_profile = Some(Builtin::LabD50.profile().to_bytes());
        assert_refused(&mut s, &format!("Lab {depth}-bit"));
        let e = s.execute(CMD, json!({"profile": "lab-d50"})).unwrap_err();
        assert!(e.to_string().contains("Lab mode"), "the reason names the mode: {e}");
        assert!(s.is_enabled("edit.convertToProfile"), "Lab {depth}-bit: Convert to Profile stays enabled");
    }
}

#[test]
fn bitmap_greys_it_out() {
    let mut s = session("gray", 8);
    s.execute("image.mode.bitmap", json!({"method": "threshold"})).unwrap();
    assert_eq!(mode(&s), ColorMode::Bitmap);
    assert_refused(&mut s, "Bitmap");
}

#[test]
fn multichannel_greys_it_out() {
    let mut s = session("rgb", 8);
    s.execute("image.mode.multichannel", json!({})).unwrap();
    assert_eq!(mode(&s), ColorMode::Multichannel);
    assert_refused(&mut s, "Multichannel");
}

#[test]
fn leaving_lab_enables_it_again() {
    let mut s = session("rgb", 8);
    s.execute("image.mode.lab", json!({})).unwrap();
    assert!(!s.is_enabled(CMD), "Lab");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(mode(&s), ColorMode::Rgb);
    assert!(s.is_enabled(CMD), "back in RGB");
}

#[test]
fn rgb_cmyk_and_grayscale_stay_enabled_at_every_depth() {
    for (name, profile) in [("rgb", "display-p3"), ("cmyk", "coated-cmyk"), ("gray", "sgray")] {
        for depth in [8, 16, 32] {
            let mut s = session(name, depth);
            assert!(s.is_enabled(CMD), "{name} {depth}-bit");
            s.execute(CMD, json!({"profile": profile})).unwrap();
            assert!(s.active().unwrap().doc.icc_profile.is_some(), "{name} {depth}-bit: assigned");
            s.execute(CMD, json!({"profile": "none"})).unwrap();
            assert!(s.active().unwrap().doc.icc_profile.is_none(), "{name} {depth}-bit: none");
        }
    }
}

#[test]
fn indexed_stays_enabled() {
    let mut s = session("rgb", 8);
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false})).unwrap();
    assert_eq!(mode(&s), ColorMode::Indexed);
    assert!(s.is_enabled(CMD), "Indexed");
    s.execute(CMD, json!({"profile": "display-p3"})).unwrap();
    assert!(s.active().unwrap().doc.icc_profile.is_some(), "Indexed: assigned");
}

/// Duotone is not measured: it keeps today's behaviour, enabled with Gray profiles.
#[test]
fn duotone_keeps_todays_behaviour() {
    let mut s = session("gray", 8);
    s.execute("image.mode.duotone", json!({"type": "monotone"})).unwrap();
    assert_eq!(mode(&s), ColorMode::Duotone);
    assert!(s.is_enabled(CMD), "Duotone");
    s.execute(CMD, json!({"profile": "sgray"})).unwrap();
}
