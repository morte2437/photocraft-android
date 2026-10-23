use super::*;
use std::sync::{Arc, Mutex};

#[test]
fn file_uris_become_local_paths() {
    assert_eq!(file_uri_path("file:///home/me/Untitled-1.psd"), Some(PathBuf::from("/home/me/Untitled-1.psd")));
    assert_eq!(file_uri_path("file:///home/me/A%20b%C3%A9.png"), Some(PathBuf::from("/home/me/A bé.png")));
    assert_eq!(file_uri_path("file://localhost/tmp/x.png"), Some(PathBuf::from("/tmp/x.png")));
    assert_eq!(file_uri_path("sftp://host/x.png"), None);
    assert_eq!(file_uri_path("file://host/x.png"), None);
}

#[test]
fn x11_windows_are_named_to_the_portal() {
    use eframe::wgpu::rwh::{RawWindowHandle, XlibWindowHandle};
    assert_eq!(parent_id(Some(RawWindowHandle::Xlib(XlibWindowHandle::new(0x3a0000f)))), "x11:3a0000f");
    assert_eq!(parent_id(None), "");
}

/// What the mock portal answers to one `SaveFile` call: the response code, and the file and
/// filter label the user "picked".
type Script = Vec<(u32, Option<(&'static str, &'static str)>)>;

/// The options of each `SaveFile` call the mock portal received: current_name and the label of
/// current_filter.
type Seen = Arc<Mutex<Vec<(Option<String>, Option<String>)>>>;

/// A private session bus (`dbus-daemon`) with a mock `org.freedesktop.portal.FileChooser` that
/// answers `SaveFile` from `script`. `None` when there is no `dbus-daemon` to run.
fn mock_portal(script: Script) -> Option<(zbus::blocking::Connection, Seen, std::process::Child)> {
    mock_portal_with(script, false)
}

/// [`mock_portal`]; with `own_paths` it is a portal older than 0.9, which ignores `handle_token`,
/// answers on a request path of its own and returns that from `SaveFile`, as a real dialog does
/// once the user is done with it (here after 300 ms).
fn mock_portal_with(script: Script, own_paths: bool) -> Option<(zbus::blocking::Connection, Seen, std::process::Child)> {
    use std::io::BufRead;
    let spawns = crate::CHILD_SPAWNS.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut daemon =
        std::process::Command::new("dbus-daemon").args(["--session", "--nofork", "--print-address=1"]).stdout(std::process::Stdio::piped()).spawn().ok()?;
    drop(spawns);
    let mut address = String::new();
    std::io::BufReader::new(daemon.stdout.take()?).read_line(&mut address).ok()?;
    let address = address.trim().to_string();
    let server = zbus::blocking::connection::Builder::address(address.as_str()).unwrap().name("org.freedesktop.portal.Desktop").unwrap().build().unwrap();
    let seen: Seen = Arc::default();
    let log = seen.clone();
    // A stream only gets the messages that arrive after it exists: made in the thread, it could
    // miss a `SaveFile` sent before the thread got going, and the call would wait forever.
    let messages = zbus::blocking::MessageIterator::from(&server);
    std::thread::spawn(move || {
        let mut script = script.into_iter();
        for msg in messages {
            let Ok(msg) = msg else { break };
            let header = msg.header();
            if header.message_type() != zbus::message::Type::MethodCall || header.member().map(|m| m.as_str()) != Some("SaveFile") {
                continue;
            }
            let (_parent, _title, options): (String, String, HashMap<String, OwnedValue>) = msg.body().deserialize().unwrap();
            let text = |key: &str| options.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok());
            let filter = options.get("current_filter").and_then(|v| {
                let (label, _): (String, Vec<(u32, String)>) = v.try_clone().ok()?.try_into().ok()?;
                Some(label)
            });
            log.lock().unwrap().push((text("current_name"), filter));
            let sender = header.sender().unwrap().as_str().trim_start_matches(':').replace('.', "_");
            let token = if own_paths { format!("old{}", log.lock().unwrap().len()) } else { text("handle_token").unwrap() };
            let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
            server.reply(&header, &OwnedObjectPath::try_from(path.as_str()).unwrap()).unwrap();
            if own_paths {
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            let (code, pick) = script.next().unwrap_or((2, None));
            let mut results: HashMap<&str, Value<'_>> = HashMap::new();
            if let Some((uri, label)) = pick {
                results.insert("uris", Value::from(vec![uri.to_string()]));
                results.insert("current_filter", Value::from((label.to_string(), vec![(0u32, "*.x".to_string())])));
            }
            server
                .emit_signal(
                    header.sender().cloned().map(zbus::names::BusName::from),
                    path.as_str(),
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(code, results),
                )
                .unwrap();
        }
    });
    let client = zbus::blocking::connection::Builder::address(address.as_str()).unwrap().build().unwrap();
    Some((client, seen, daemon))
}

fn request(name: &str) -> SaveRequest {
    let filters = [("PSD Document", &["psd", "psb"][..]), ("PNG", &["png"]), ("JPEG", &["jpg"])];
    SaveRequest {
        parent: String::new(),
        name: Some(name.into()),
        suggested: Some(name.into()),
        folder: Some(PathBuf::from("/tmp")),
        filters: filters.iter().map(|(l, e)| (l.to_string(), e.iter().map(|s| s.to_string()).collect())).collect(),
        current: Some(0),
    }
}

/// #6121 end to end through a real D-Bus: the name comes back as typed with the type picked,
/// and Save As writes it with that type's extension.
#[test]
fn save_as_follows_the_file_type_picked_in_the_portal_dialog() {
    let dir = std::env::temp_dir().join(format!("photocraft-portal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let typed = format!("file://{}/Untitled-1.psd", dir.display());
    let typed: &'static str = Box::leak(typed.into_boxed_str());
    let Some((conn, seen, mut daemon)) = mock_portal(vec![(0, Some((typed, "PNG"))), (1, None), (0, Some((typed, "PSD Document")))]) else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    // PNG picked with `Untitled-1.psd` still in the name: written as Untitled-1.png.
    assert_eq!(save_as(&conn, request("Untitled-1.psd")).unwrap(), Some(dir.join("Untitled-1.png")));
    assert_eq!(seen.lock().unwrap()[0], (Some("Untitled-1.psd".into()), Some("PSD Document".into())), "the request carried the name and type");
    // Cancel.
    assert_eq!(save_as(&conn, request("Untitled-1.psd")).unwrap(), None);
    // The type the name already has: unchanged.
    assert_eq!(save_as(&conn, request("Untitled-1.psd")).unwrap(), Some(dir.join("Untitled-1.psd")));
    daemon.kill().ok();
    std::fs::remove_dir_all(&dir).ok();
}

/// The dialog opens on the bare name (`test`, see `services::dialog_file_name`), so GTK doesn't
/// ask to replace test.psd when PNG is picked: the type picked gives the extension, and a name
/// that then exists is asked about under that name.
#[test]
fn a_bare_name_takes_the_picked_type_and_an_existing_target_is_asked_about() {
    let dir = std::env::temp_dir().join(format!("photocraft-portal-bare-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("test.psd"), b"open document").unwrap();
    let uri = |name: &str| -> &'static str { Box::leak(format!("file://{}/{name}", dir.display()).into_boxed_str()) };
    let Some((conn, seen, mut daemon)) = mock_portal(vec![
        (0, Some((uri("test"), "PNG"))),
        (0, Some((uri("test.psd"), "PNG"))),
        (0, Some((uri("test"), "PSD Document"))),
        (0, Some((uri("test.psd"), "PSD Document"))),
    ]) else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    // As File › Save As opens it: `test` shown, `test.psd` suggested.
    let opened = || SaveRequest { suggested: Some("test.psd".into()), ..request("test") };
    // PNG picked: test.png, and nothing was asked about test.psd.
    assert_eq!(save_as(&conn, opened()).unwrap(), Some(dir.join("test.png")));
    // The suggested extension back in the name (typed again, or GTK's completion) with PNG
    // picked: test.png too, not a PSD written over test.psd.
    assert_eq!(save_as(&conn, opened()).unwrap(), Some(dir.join("test.png")));
    // The type untouched: test.psd, which exists, so the dialog asks again as test.psd (the
    // portal's replace prompt is then about the file that is written).
    assert_eq!(save_as(&conn, opened()).unwrap(), Some(dir.join("test.psd")));
    let seen = seen.lock().unwrap().clone();
    let names: Vec<_> = seen.iter().map(|(name, _)| name.as_deref()).collect();
    assert_eq!(names, [Some("test"), Some("test"), Some("test"), Some("test.psd")], "{seen:?}");
    assert_eq!(seen[3].1.as_deref(), Some("PSD Document"));
    daemon.kill().ok();
    std::fs::remove_dir_all(&dir).ok();
}

/// GNOME's dialog closed with Esc answers 2 ("ended some other way"): no file, like Cancel, and
/// no fallback to rfd, which opened a second Save As.
#[test]
fn a_dialog_closed_some_other_way_is_a_cancel() {
    let Some((conn, _seen, mut daemon)) = mock_portal(vec![(2, None)]) else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    assert_eq!(save_as(&conn, request("test")), Ok(None));
    daemon.kill().ok();
}

/// An extension the user typed is theirs: `art.png` with the PSD type left as the dialog opened
/// saves a PNG, as it did before the type was read.
#[test]
fn a_typed_extension_wins_over_an_untouched_file_type() {
    let typed: &'static str = "file:///tmp/photocraft-none/art.png";
    let Some((conn, _seen, mut daemon)) = mock_portal(vec![(0, Some((typed, "PSD Document")))]) else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    assert_eq!(save_as(&conn, request("Untitled-1.psd")).unwrap(), Some(PathBuf::from("/tmp/photocraft-none/art.png")));
    daemon.kill().ok();
}

/// A portal older than 0.9 answers on a request path of its own: the wait follows the handle
/// `SaveFile` returns, as the Request documentation asks, instead of the predicted path.
#[test]
fn the_answer_is_awaited_on_the_handle_the_portal_returns() {
    let typed: &'static str = "file:///tmp/photocraft-none/Untitled-1.psd";
    let Some((conn, _seen, mut daemon)) = mock_portal_with(vec![(0, Some((typed, "PNG")))], true) else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    assert_eq!(save_as(&conn, request("Untitled-1.psd")).unwrap(), Some(PathBuf::from("/tmp/photocraft-none/Untitled-1.png")));
    daemon.kill().ok();
}

/// When the extension the type gives names a file that exists, the dialog opens again under that
/// name (so the portal asks before replacing it) instead of overwriting it unasked.
#[test]
fn a_renamed_file_that_exists_is_confirmed_in_the_dialog() {
    let dir = std::env::temp_dir().join(format!("photocraft-portal-exists-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("art.png"), b"old").unwrap();
    let typed: &'static str = Box::leak(format!("file://{}/art.psd", dir.display()).into_boxed_str());
    let confirmed: &'static str = Box::leak(format!("file://{}/art.png", dir.display()).into_boxed_str());
    let Some((conn, seen, mut daemon)) = mock_portal(vec![(0, Some((typed, "PNG"))), (0, Some((confirmed, "PNG")))]) else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    assert_eq!(save_as(&conn, request("art.psd")).unwrap(), Some(dir.join("art.png")));
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "asked again: {seen:?}");
    assert_eq!(seen[1], (Some("art.png".into()), Some("PNG".into())), "under the new name, on the picked type");
    assert_eq!(std::fs::read(dir.join("art.png")).unwrap(), b"old", "nothing written by the dialog");
    daemon.kill().ok();
    std::fs::remove_dir_all(&dir).ok();
}

/// With only one type offered (the Swatches panel's .aco) a typed extension is kept.
#[test]
fn a_single_file_type_keeps_the_typed_extension() {
    let typed: &'static str = "file:///tmp/photocraft-none/Swatches.ase";
    let Some((conn, _seen, mut daemon)) = mock_portal(vec![(0, Some((typed, "Color Swatches")))]) else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let req = SaveRequest { filters: vec![("Color Swatches".into(), vec!["aco".into()])], ..request("Swatches.aco") };
    assert_eq!(save_as(&conn, req).unwrap(), Some(PathBuf::from("/tmp/photocraft-none/Swatches.ase")));
    daemon.kill().ok();
}

/// The same single type, with the name left as the dialog shows it: the bare stem comes back, and
/// that one type gives it its extension instead of a file named `Swatches` being written.
#[test]
fn a_single_file_type_gives_the_bare_name_its_extension() {
    let typed: &'static str = "file:///tmp/photocraft-none/Swatches";
    let Some((conn, _seen, mut daemon)) = mock_portal(vec![(0, Some((typed, "Color Swatches")))]) else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let req = SaveRequest { filters: vec![("Color Swatches".into(), vec!["aco".into()])], ..request("Swatches.aco") };
    assert_eq!(save_as(&conn, req).unwrap(), Some(PathBuf::from("/tmp/photocraft-none/Swatches.aco")));
    daemon.kill().ok();
}

/// Which type's extensions the returned name gets. The portal cases above need a `dbus-daemon`,
/// so the rule itself is checked here too, which needs none.
#[test]
fn the_type_that_gives_the_extension_is_the_one_picked() {
    let one: Vec<Filter> = vec![("Color Swatches".into(), vec!["aco".into()])];
    // One type offered: the dialog reports no pick, and it was given the stem.
    assert_eq!(chosen_type(&one, None), Some(0));
    assert_eq!(chosen_type(&one, Some(0)), Some(0));
    let many: Vec<Filter> = request("test").filters;
    assert_eq!(chosen_type(&many, Some(1)), Some(1));
    // Several types and no pick: nothing to add, as before.
    assert_eq!(chosen_type(&many, None), None);
    // An answer naming a type this dialog never offered is not a pick either.
    assert_eq!(chosen_type(&many, Some(99)), None);
}
