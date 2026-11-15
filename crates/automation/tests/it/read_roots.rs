//! Real-document regressions for ordered read roots and independent write authority.

use std::path::{Path, PathBuf};

use photocraft_automation::{AuthorizedWorkspace, Headless, files};
use photocraft_engine::actions_cmds::Action;
use serde_json::{Value, json};

struct Roots {
    base: PathBuf,
    library: PathBuf,
    work: PathBuf,
    outside: PathBuf,
}

impl Roots {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!("pc-read-roots-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let roots = Self { library: base.join("library"), work: base.join("work"), outside: base.join("outside"), base };
        for root in [&roots.library, &roots.work, &roots.outside] {
            std::fs::create_dir_all(root).unwrap();
        }
        roots
    }

    fn workspace(&self) -> AuthorizedWorkspace {
        AuthorizedWorkspace::with_read_roots([self.library.as_path(), self.work.as_path()], Some(self.work.as_path())).unwrap()
    }

    fn headless(&self) -> Headless {
        Headless::with_workspace(self.workspace())
    }

    fn seed(&self) -> (Vec<u8>, Vec<u8>) {
        let library = layered_bytes(11, "white");
        let work = layered_bytes(7, "black");
        std::fs::write(self.library.join("same.pcraft"), &library).unwrap();
        std::fs::write(self.work.join("same.pcraft"), &work).unwrap();
        (library, work)
    }
}

impl Drop for Roots {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn layered_bytes(width: u32, background: &str) -> Vec<u8> {
    let mut headless = Headless::new();
    headless.command_run("file.new", json!({"width": width, "height": 5, "background": background})).unwrap();
    headless.command_run("layer.new.layer", json!({"name": "Source marker"})).unwrap();
    let doc = &headless.session.active().unwrap().doc;
    files::save_bytes(doc, "seed.pcraft", None, &Default::default()).unwrap().0
}

fn snapshot(headless: &mut Headless) -> (Value, Value, Vec<u8>) {
    (headless.session_list(), headless.inspect(None).unwrap(), headless.render_png(None, 32).unwrap())
}

fn assert_source(headless: &Headless, width: u32, rgba: [u8; 4]) {
    let state = headless.session.active().unwrap();
    assert_eq!(state.doc.size.width, width);
    assert_eq!(state.doc.size.height, 5);
    let preview = photocraft_compose::thumbnail(&state.doc, 32);
    assert!(!preview.pixels.is_empty());
    assert!(preview.pixels.as_chunks::<4>().0.iter().all(|pixel| pixel == &rgba), "opened the wrong source pixels");
}

fn edit(headless: &mut Headless, name: &str) {
    headless.command_run("layer.new.layer", json!({"name": name})).unwrap();
    assert!(headless.session.active().unwrap().is_dirty());
}

#[test]
fn library_first_open_cannot_overwrite_same_named_work_until_explicit_save_as() {
    let roots = Roots::new("save-identity");
    let (library, work) = roots.seed();
    let mut headless = roots.headless();
    headless.open(Path::new("same.pcraft")).unwrap();
    assert_source(&headless, 11, [255; 4]);
    assert!(headless.session.active().unwrap().path.is_none());
    edit(&mut headless, "Library edit");

    let before = snapshot(&mut headless);
    let error = headless.save(None, None, None, &Default::default()).unwrap_err().to_string();
    assert!(error.contains("pass `path`"), "{error}");
    assert_eq!(snapshot(&mut headless), before, "denial must preserve pixels, history, selection, path and dirty revision");
    assert_eq!(std::fs::read(roots.library.join("same.pcraft")).unwrap(), library);
    assert_eq!(std::fs::read(roots.work.join("same.pcraft")).unwrap(), work);

    // Naming a target is the explicit transition from a library document to a work file.
    headless.save(None, Some(Path::new("same.pcraft")), None, &Default::default()).unwrap();
    assert_eq!(headless.session.active().unwrap().path.as_deref(), Some("same.pcraft"));
    assert!(!headless.session.active().unwrap().is_dirty());
    assert_ne!(std::fs::read(roots.work.join("same.pcraft")).unwrap(), work);
    assert_eq!(std::fs::read(roots.library.join("same.pcraft")).unwrap(), library);

    edit(&mut headless, "Later work edit");
    assert_eq!(headless.save(None, None, None, &Default::default()).unwrap()["path"], "same.pcraft");
    assert!(!headless.session.active().unwrap().is_dirty());
    assert_eq!(std::fs::read(roots.library.join("same.pcraft")).unwrap(), library);

    let mut reopened = roots.headless();
    reopened.open(&roots.work.join("same.pcraft")).unwrap();
    assert_source(&reopened, 11, [255; 4]);
    assert_eq!(reopened.inspect(None).unwrap()["layers"][0]["name"], "Later work edit");
}

#[test]
fn absolute_path_selects_the_second_read_root_and_allows_its_own_implicit_save() {
    let roots = Roots::new("absolute-work");
    let (library, work) = roots.seed();
    let mut headless = roots.headless();
    let path = roots.work.join("same.pcraft");
    headless.open(&path).unwrap();
    assert_source(&headless, 7, [0, 0, 0, 255]);
    assert_eq!(headless.session.active().unwrap().path.as_deref(), path.to_str());
    edit(&mut headless, "Absolute work edit");
    assert_eq!(headless.save(None, None, None, &Default::default()).unwrap()["path"], path.to_str().unwrap());
    assert!(!headless.session.active().unwrap().is_dirty());
    assert_eq!(std::fs::read(roots.library.join("same.pcraft")).unwrap(), library);
    assert_ne!(std::fs::read(&path).unwrap(), work);
}

#[test]
fn missing_library_file_falls_through_to_a_writable_work_document() {
    let roots = Roots::new("relative-work");
    let work = layered_bytes(7, "black");
    std::fs::write(roots.work.join("unique.pcraft"), &work).unwrap();
    let mut headless = roots.headless();
    headless.open(Path::new("unique.pcraft")).unwrap();
    assert_source(&headless, 7, [0, 0, 0, 255]);
    assert_eq!(headless.session.active().unwrap().path.as_deref(), Some("unique.pcraft"));
    edit(&mut headless, "Relative work edit");
    headless.save(None, None, None, &Default::default()).unwrap();
    assert_ne!(std::fs::read(roots.work.join("unique.pcraft")).unwrap(), work);
    assert!(!roots.library.join("unique.pcraft").exists());
}

#[test]
fn read_only_roots_can_reopen_exported_work_without_acquiring_write_authority() {
    let roots = Roots::new("readonly-reopen");
    let (library, _) = roots.seed();
    {
        let mut writer = roots.headless();
        writer.open(Path::new("same.pcraft")).unwrap();
        edit(&mut writer, "Exported for review");
        writer.save(None, Some(Path::new("review.pcraft")), None, &Default::default()).unwrap();
    }
    let exported = std::fs::read(roots.work.join("review.pcraft")).unwrap();
    let workspace = AuthorizedWorkspace::with_read_roots([roots.library.as_path(), roots.work.as_path()], None).unwrap();
    let mut reader = Headless::with_workspace(workspace);
    for path in [PathBuf::from("review.pcraft"), roots.work.join("review.pcraft")] {
        reader.open(&path).unwrap();
        assert_source(&reader, 11, [255; 4]);
        assert_eq!(reader.inspect(None).unwrap()["layers"][0]["name"], "Exported for review");
        assert!(reader.session.active().unwrap().path.is_none());
        edit(&mut reader, "Unsaved review edit");
        let before = snapshot(&mut reader);
        assert!(reader.save(None, None, None, &Default::default()).is_err());
        let error = reader.save(None, Some(Path::new("review.pcraft")), None, &Default::default()).unwrap_err().to_string();
        assert!(error.contains("write authority is absent"), "{error}");
        assert_eq!(snapshot(&mut reader), before);
    }
    assert_eq!(std::fs::read(roots.work.join("review.pcraft")).unwrap(), exported);
    assert_eq!(std::fs::read(roots.library.join("same.pcraft")).unwrap(), library);
}

#[test]
fn a_write_root_does_not_implicitly_grant_read_access_to_saved_work() {
    let roots = Roots::new("write-is-not-read");
    let _ = roots.seed();
    let workspace = AuthorizedWorkspace::with_read_roots([roots.library.as_path()], Some(roots.work.as_path())).unwrap();
    let mut headless = Headless::with_workspace(workspace);
    headless.open(Path::new("same.pcraft")).unwrap();
    headless.save(None, Some(Path::new("written.pcraft")), None, &Default::default()).unwrap();
    let before = snapshot(&mut headless);
    for path in [PathBuf::from("written.pcraft"), roots.work.join("written.pcraft")] {
        assert!(headless.open(&path).is_err());
        assert_eq!(snapshot(&mut headless), before);
    }
    assert!(roots.work.join("written.pcraft").is_file());
}

#[test]
fn flat_export_does_not_arm_implicit_save_for_a_library_document() {
    let roots = Roots::new("flat-export");
    let (library, work) = roots.seed();
    let mut headless = roots.headless();
    headless.open(Path::new("same.pcraft")).unwrap();
    edit(&mut headless, "Still unsaved");
    let before = snapshot(&mut headless);
    headless.save(None, Some(Path::new("preview.png")), None, &Default::default()).unwrap();
    assert_eq!(snapshot(&mut headless), before, "a flat export must not clear dirty or attach a save path");
    let exported = std::fs::read(roots.work.join("preview.png")).unwrap();
    assert_eq!(photocraft_codecs::decode(&exported).unwrap().dimensions(), (11, 5));
    assert!(headless.save(None, None, None, &Default::default()).is_err());
    assert_eq!(snapshot(&mut headless), before);
    assert_eq!(std::fs::read(roots.work.join("preview.png")).unwrap(), exported);
    assert_eq!(std::fs::read(roots.library.join("same.pcraft")).unwrap(), library);
    assert_eq!(std::fs::read(roots.work.join("same.pcraft")).unwrap(), work);
}

#[test]
fn library_absolute_paths_and_traversal_cannot_be_used_as_save_targets() {
    let roots = Roots::new("rejected-writes");
    let (library, work) = roots.seed();
    let canary = layered_bytes(3, "black");
    std::fs::write(roots.outside.join("canary.pcraft"), &canary).unwrap();
    let mut headless = roots.headless();
    headless.open(&roots.library.join("same.pcraft")).unwrap();
    assert!(headless.session.active().unwrap().path.is_none());
    edit(&mut headless, "Must remain unsaved");
    let before = snapshot(&mut headless);
    for path in [
        roots.library.join("same.pcraft"),
        PathBuf::from("../library/same.pcraft"),
        PathBuf::from("../outside/canary.pcraft"),
        roots.work.join("..").join("outside").join("canary.pcraft"),
    ] {
        let error = headless.save(None, Some(&path), None, &Default::default()).unwrap_err().to_string();
        assert!(error.contains("automation path rejected"), "{path:?}: {error}");
        assert_eq!(snapshot(&mut headless), before);
    }
    assert_eq!(std::fs::read(roots.library.join("same.pcraft")).unwrap(), library);
    assert_eq!(std::fs::read(roots.work.join("same.pcraft")).unwrap(), work);
    assert_eq!(std::fs::read(roots.outside.join("canary.pcraft")).unwrap(), canary);
}

#[cfg(any(unix, windows))]
#[test]
fn a_work_directory_symlink_cannot_write_into_another_read_root() {
    let roots = Roots::new("symlink-write");
    let (library, work) = roots.seed();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&roots.library, roots.work.join("library-link")).unwrap();
    #[cfg(windows)]
    if let Err(error) = std::os::windows::fs::symlink_dir(&roots.library, roots.work.join("library-link")) {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            eprintln!("symlink regression requires Windows symlink permission: {error}");
            return;
        }
        panic!("create test symlink: {error}");
    }
    let mut headless = roots.headless();
    headless.open(Path::new("same.pcraft")).unwrap();
    edit(&mut headless, "Cannot follow the write link");
    let before = snapshot(&mut headless);
    for path in ["library-link/same.pcraft", "library-link/new.pcraft"] {
        assert!(headless.save(None, Some(Path::new(path)), None, &Default::default()).is_err());
        assert_eq!(snapshot(&mut headless), before);
    }
    assert_eq!(std::fs::read(roots.library.join("same.pcraft")).unwrap(), library);
    assert_eq!(std::fs::read(roots.work.join("same.pcraft")).unwrap(), work);
    assert!(!roots.library.join("new.pcraft").exists());
}

#[test]
fn corrupt_first_match_is_an_error_instead_of_falling_through_to_a_valid_document() {
    let roots = Roots::new("corrupt-first");
    let (_, work) = roots.seed();
    let corrupt = b"this is not a native document";
    std::fs::write(roots.library.join("same.pcraft"), corrupt).unwrap();
    let mut headless = roots.headless();
    // The later file is valid, and a failed open must leave this active document intact.
    headless.open(&roots.work.join("same.pcraft")).unwrap();
    assert_source(&headless, 7, [0, 0, 0, 255]);
    edit(&mut headless, "Existing edit");
    let before = snapshot(&mut headless);
    assert!(headless.open(Path::new("same.pcraft")).is_err());
    assert_eq!(snapshot(&mut headless), before);
    assert_eq!(std::fs::read(roots.library.join("same.pcraft")).unwrap(), corrupt);
    assert_eq!(std::fs::read(roots.work.join("same.pcraft")).unwrap(), work);
}

#[test]
fn multiple_roots_do_not_grant_ambient_commands_or_nested_action_file_access() {
    let roots = Roots::new("ambient-commands");
    let (library, work) = roots.seed();
    let mut headless = roots.headless();
    headless.open(Path::new("same.pcraft")).unwrap();
    let before = snapshot(&mut headless);
    let target = roots.work.join("ambient.psd");
    let error = headless.command_run("file.saveACopy", json!({"path": target})).unwrap_err().to_string();
    assert!(error.contains("ambient filesystem paths"), "{error}");
    // A writable destination still cannot be reached through an ambient engine command.
    headless.session.actions.list.push(Action::new("Write work", vec![("file.saveACopy".into(), json!({"path": target}))]));
    headless.session.actions.list.push(Action::new("Nested write", vec![("actions.play".into(), json!({"action": "Write work"}))]));
    let played = headless.command_run("actions.play", json!({"action": "Nested write"})).unwrap();
    assert!(played["failed"].to_string().contains("ambient filesystem paths"), "{played}");
    assert_eq!(snapshot(&mut headless), before);
    assert!(!target.exists());
    assert_eq!(std::fs::read(roots.library.join("same.pcraft")).unwrap(), library);
    assert_eq!(std::fs::read(roots.work.join("same.pcraft")).unwrap(), work);
}
