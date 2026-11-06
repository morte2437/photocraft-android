//! A save that won't fit in memory is refused before any encoding starts, with a message that
//! says how to free memory; the document stays open and unsaved (#10024). Its own test binary:
//! the memory simulation is process-wide.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;

use photocraft_engine::Session;
use photocraft_raster::headroom;
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;

/// The simulation is process-wide: the tests take turns.
static LOCK: Mutex<()> = Mutex::new(());

fn app(writes: Rc<RefCell<u32>>) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(
        Session::new(),
        Services {
            export: Some(Box::new(|doc, path, _| {
                photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(move |_, _| {
                *writes.borrow_mut() += 1;
                Ok(())
            })),
            ..Default::default()
        },
    );
    app.run("file.new", json!({"width": 8, "height": 8, "name": "Untitled-1"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

fn identity(app: &PhotocraftApp) -> (String, Option<String>, u64, u64) {
    let st = app.session.active().unwrap();
    (st.doc.name.clone(), st.path.clone(), st.revision, st.saved_revision)
}

#[test]
fn a_save_that_does_not_fit_fails_and_leaves_the_document_unsaved() {
    let _turn = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let writes = Rc::new(RefCell::new(0));
    let mut app = app(writes.clone());
    let before = identity(&app);
    headroom::simulate_available(Some(1 << 20));
    let refused = app.save_as(Some("big.psd".into()));
    let refused_automation = app.save_automation(Some("auto.psd".into()));
    headroom::simulate_available(None);
    let err = refused.unwrap_err();
    assert!(err.contains("Not enough memory to save \u{201c}big.psd\u{201d}"), "{err}");
    assert!(err.contains("Edit \u{203a} Purge \u{203a} All"), "{err}");
    let automation_err = refused_automation.unwrap_err();
    assert!(automation_err.contains("Not enough memory to save \u{201c}auto.psd\u{201d}"), "{automation_err}");
    assert_eq!(*writes.borrow(), 0, "nothing was encoded or written");
    assert_eq!(identity(&app), before, "the document is unsaved and unchanged");
    app.save_as(Some("big.psd".into())).unwrap();
    assert_eq!(*writes.borrow(), 1);
}

fn warnings(app: &PhotocraftApp) -> usize {
    app.ui.notices.iter().filter(|n| n.title == photocraft_ui_egui::memory_warning::TITLE).count()
}

/// Going low posts one notice that stays; hovering low doesn't post again; recovering past the
/// higher threshold takes it down and arms the next warning.
#[test]
fn the_low_memory_warning_posts_once_and_rearms_with_hysteresis() {
    let _turn = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    use photocraft_ui_egui::memory_warning::{REARM_ABOVE, WARN_BELOW, tick};
    let mut app = app(Rc::new(RefCell::new(0)));
    let mut now = 0.0;
    let mut look = |app: &mut PhotocraftApp, available: Option<u64>| {
        headroom::simulate_available(available);
        now += 2.5;
        tick(app, now);
    };
    look(&mut app, None);
    assert_eq!(warnings(&app), 0, "plenty of memory (and always so on the desktop)");
    look(&mut app, Some(WARN_BELOW - 1));
    assert_eq!(warnings(&app), 1);
    assert!(app.ui.notices.iter().all(|n| n.sticky), "stays until dismissed");
    look(&mut app, Some(WARN_BELOW - 1));
    look(&mut app, Some(WARN_BELOW + 1));
    assert_eq!(warnings(&app), 1, "no second notice while it stays low");
    look(&mut app, Some(REARM_ABOVE));
    assert_eq!(warnings(&app), 0, "recovered: the stale notice is taken down");
    look(&mut app, Some(WARN_BELOW - 1));
    assert_eq!(warnings(&app), 1, "low again: warned again");
    // Checks are two seconds apart at most.
    headroom::simulate_available(None);
    app.ui.notices.clear();
    headroom::simulate_available(Some(0));
    tick(&mut app, now + 0.5);
    assert_eq!(warnings(&app), 0);
    headroom::simulate_available(None);
}

#[test]
fn no_warning_without_a_document() {
    let _turn = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    headroom::simulate_available(Some(0));
    photocraft_ui_egui::memory_warning::tick(&mut app, 100.0);
    headroom::simulate_available(None);
    assert_eq!(warnings(&app), 0);
}
