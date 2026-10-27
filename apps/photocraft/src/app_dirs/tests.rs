use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

fn temp(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("photocraft-app-dirs-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn env_of(pairs: &[(&str, &Path)]) -> impl Fn(&str) -> Option<OsString> + use<> {
    let m: HashMap<String, OsString> = pairs.iter().map(|(k, v)| (k.to_string(), v.as_os_str().to_owned())).collect();
    move |k| m.get(k).cloned()
}

/// A fake user folder that every platform's convention resolves under.
fn fake_home(root: &Path) -> impl Fn(&str) -> Option<OsString> + use<> {
    let home = root.join("home");
    let appdata = root.join("appdata");
    env_of(&[("HOME", &home), ("APPDATA", &appdata)])
}

#[test]
fn no_marker_uses_the_platform_dir() {
    let root = temp("platform");
    let exe = root.join("bin");
    std::fs::create_dir_all(&exe).unwrap();
    let env = fake_home(&root);
    let d = resolve(&env, Some(&exe));
    assert_eq!(d.mode, Mode::Platform);
    assert!(d.warning.is_none());
    let dir = d.dir.unwrap();
    assert_eq!(Some(dir.clone()), platform_dir(&env));
    assert!(!dir.starts_with(&exe));
    assert!(!exe.join(PORTABLE_DATA_DIR).exists(), "no portable folder without a marker");
    // The convention per platform.
    if cfg!(target_os = "macos") {
        assert_eq!(dir, root.join("home/Library/Application Support/Photocraft"));
    } else if cfg!(windows) {
        assert_eq!(dir, root.join("appdata").join("Photocraft"));
    } else {
        assert_eq!(dir, root.join("home/.config/photocraft"));
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn either_marker_keeps_everything_beside_the_exe() {
    for marker in PORTABLE_MARKERS {
        let root = temp("portable");
        let exe = root.join("PhotoCraft");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::write(exe.join(marker), b"").unwrap();
        let d = resolve(fake_home(&root), Some(&exe));
        assert_eq!(d.mode, Mode::Portable, "{marker}");
        assert!(d.warning.is_none());
        let dir = d.dir.unwrap();
        assert_eq!(dir, exe.join(PORTABLE_DATA_DIR));
        assert!(dir.is_dir(), "the data folder is created");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "the write probe is removed");
        // Every derived path sits under the exe dir.
        for sub in ["preferences.json", "Presets", "Recovery", crate::gpu_startup::MARKER_FILE] {
            assert!(dir.join(sub).starts_with(&exe));
        }
        assert!(!root.join("home").exists() && !root.join("appdata").exists(), "nothing written to the user folder");
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn a_marker_folder_is_not_a_marker() {
    let root = temp("dir-marker");
    let exe = root.join("bin");
    std::fs::create_dir_all(exe.join("portable.txt")).unwrap();
    assert_eq!(resolve(fake_home(&root), Some(&exe)).mode, Mode::Platform);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unwritable_portable_folder_falls_back_with_a_warning() {
    let root = temp("unwritable");
    let exe = root.join("bin");
    std::fs::create_dir_all(&exe).unwrap();
    std::fs::write(exe.join("portable.txt"), b"").unwrap();
    // A file where the data folder should go: it can't be created, on every platform.
    std::fs::write(exe.join(PORTABLE_DATA_DIR), b"in the way").unwrap();
    let env = fake_home(&root);
    let d = resolve(&env, Some(&exe));
    assert_eq!(d.mode, Mode::Platform);
    assert_eq!(d.dir, platform_dir(&env));
    let w = d.warning.unwrap();
    assert!(w.contains("portable") && w.contains("isn't writable"), "{w}");
    // No user folder at all: still no panic, just no directory.
    let d = resolve(|_| None, Some(&exe));
    assert!(d.dir.is_none() && d.warning.is_some());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn override_wins_over_portable() {
    let root = temp("override");
    let exe = root.join("bin");
    std::fs::create_dir_all(&exe).unwrap();
    std::fs::write(exe.join("portable.txt"), b"").unwrap();
    let custom = root.join("custom");
    let d = resolve(env_of(&[("PHOTOCRAFT_CONFIG_DIR", &custom)]), Some(&exe));
    assert_eq!((d.mode, d.dir), (Mode::Override, Some(custom)));
    assert!(!exe.join(PORTABLE_DATA_DIR).exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// #5648: on Windows, autosaves go to the machine-local `%LOCALAPPDATA%`, not the roaming
/// profile; an override or portable folder keeps them inside itself.
#[test]
fn autosaves_stay_out_of_the_roaming_profile() {
    let root = temp("recovery-local");
    let settings = root.join("appdata").join("Photocraft");
    let local = root.join("local");
    let env = env_of(&[("LOCALAPPDATA", &local)]);
    let platform = DataDir { dir: Some(settings.clone()), mode: Mode::Platform, warning: None };
    let expected = if cfg!(windows) { local.join("Photocraft").join("Recovery") } else { settings.join("Recovery") };
    assert_eq!(resolve_recovery(&platform, &env), Some(expected));
    for mode in [Mode::Override, Mode::Portable] {
        let d = DataDir { dir: Some(settings.clone()), mode, warning: None };
        assert_eq!(resolve_recovery(&d, &env), Some(settings.join("Recovery")), "{mode:?}");
    }
    // No (or an empty) LOCALAPPDATA: beside the settings, as before.
    let empty = PathBuf::new();
    assert_eq!(resolve_recovery(&platform, env_of(&[("LOCALAPPDATA", &empty)])), Some(settings.join("Recovery")));
    assert_eq!(resolve_recovery(&platform, |_| None), Some(settings.join("Recovery")));
    let nowhere = DataDir { dir: None, mode: Mode::Platform, warning: None };
    assert_eq!(resolve_recovery(&nowhere, |_| None), None);
    let _ = std::fs::remove_dir_all(&root);
}

/// #6612: on Linux (and other XDG systems) autosaves go to `$XDG_STATE_HOME`, by default
/// `~/.local/state`, not the config folder; a relative `XDG_STATE_HOME` is ignored. Windows
/// and macOS don't read it, and an override or portable folder keeps them inside itself.
#[test]
fn autosaves_go_to_the_xdg_state_folder() {
    let root = temp("recovery-xdg");
    let settings = root.join("home").join(".config").join("photocraft");
    let home = root.join("home");
    let state = root.join("state");
    let platform = DataDir { dir: Some(settings.clone()), mode: Mode::Platform, warning: None };
    let xdg = !cfg!(windows) && !cfg!(target_os = "macos");
    let expect = |xdg_path: PathBuf| Some(if xdg { xdg_path } else { settings.join("Recovery") });
    assert_eq!(resolve_recovery(&platform, env_of(&[("XDG_STATE_HOME", &state), ("HOME", &home)])), expect(state.join("photocraft").join("Recovery")));
    let default = home.join(".local").join("state").join("photocraft").join("Recovery");
    assert_eq!(resolve_recovery(&platform, env_of(&[("HOME", &home)])), expect(default.clone()));
    let relative = PathBuf::from("relative/state");
    assert_eq!(resolve_recovery(&platform, env_of(&[("XDG_STATE_HOME", &relative), ("HOME", &home)])), expect(default.clone()));
    let empty = PathBuf::new();
    assert_eq!(resolve_recovery(&platform, env_of(&[("XDG_STATE_HOME", &empty), ("HOME", &home)])), expect(default));
    // No home at all: beside the settings, as before.
    assert_eq!(resolve_recovery(&platform, |_| None), Some(settings.join("Recovery")));
    for mode in [Mode::Override, Mode::Portable] {
        let d = DataDir { dir: Some(settings.clone()), mode, warning: None };
        assert_eq!(resolve_recovery(&d, env_of(&[("XDG_STATE_HOME", &state), ("HOME", &home)])), Some(settings.join("Recovery")), "{mode:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// An autosave bundle and its sidecar, as `format::Autosaver` writes them.
fn autosave(dir: &Path, key: &str) {
    std::fs::create_dir_all(dir.join(format!("{key}.pcraft"))).unwrap();
    std::fs::write(dir.join(format!("{key}.pcraft")).join("manifest.json"), key).unwrap();
    std::fs::write(dir.join(format!("{key}.json")), key).unwrap();
}

#[test]
fn autosaves_from_the_old_folder_move_to_the_new_one() {
    let root = temp("recovery-move");
    let legacy = root.join("roaming").join("Recovery");
    let dir = root.join("local").join("Photocraft").join("Recovery");
    autosave(&legacy, "a");
    assert_eq!(adopt_legacy_recovery(&legacy, &dir), dir);
    assert!(!legacy.exists(), "the old folder is gone");
    assert_eq!(std::fs::read_to_string(dir.join("a.pcraft").join("manifest.json")).unwrap(), "a");
    assert!(dir.join("a.json").is_file());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn autosaves_merge_into_an_existing_folder_without_overwriting() {
    let root = temp("recovery-merge");
    let legacy = root.join("roaming").join("Recovery");
    let dir = root.join("local").join("Recovery");
    autosave(&legacy, "old");
    autosave(&legacy, "both");
    // A temp file of an interrupted write moves along, for the recovery store's sweep.
    let temp_file = ".old.json.4294967294-0.pcsave.tmp";
    std::fs::write(legacy.join(temp_file), b"partial").unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("both.json"), "newer").unwrap();
    assert_eq!(adopt_legacy_recovery(&legacy, &dir), dir);
    assert!(dir.join(temp_file).is_file() && !legacy.join(temp_file).exists());
    assert!(dir.join("old.pcraft").is_dir() && dir.join("old.json").is_file(), "a whole autosave moves");
    assert_eq!(std::fs::read_to_string(dir.join("both.json")).unwrap(), "newer", "never overwritten");
    assert!(!dir.join("both.pcraft").exists(), "a clashing autosave isn't split");
    assert!(legacy.join("both.pcraft").is_dir() && legacy.join("both.json").is_file(), "it stays behind whole");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn autosaves_that_cannot_move_keep_being_used_where_they_are() {
    let root = temp("recovery-stuck");
    let legacy = root.join("roaming").join("Recovery");
    autosave(&legacy, "a");
    // A file where the new folder's parent should be: nothing can be created there.
    std::fs::write(root.join("blocked"), b"in the way").unwrap();
    let dir = root.join("blocked").join("Recovery");
    assert_eq!(adopt_legacy_recovery(&legacy, &dir), legacy);
    assert!(legacy.join("a.json").is_file(), "nothing lost");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_empty_old_folder_is_removed_even_where_it_cannot_move() {
    let root = temp("recovery-empty");
    let legacy = root.join("roaming").join("Recovery");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(root.join("blocked"), b"in the way").unwrap();
    let dir = root.join("blocked").join("Recovery");
    assert_eq!(adopt_legacy_recovery(&legacy, &dir), dir, "nothing to keep listed");
    assert!(!legacy.exists(), "the empty old folder is gone");
    let _ = std::fs::remove_dir_all(&root);
}

/// Where a rename can't cross volumes (a roaming profile on a network share), an autosave
/// is copied whole, nested folders included, and only then removed.
#[test]
fn a_move_across_volumes_copies_then_removes() {
    let root = temp("recovery-copy");
    let legacy = root.join("roaming").join("Recovery");
    autosave(&legacy, "a");
    std::fs::create_dir_all(legacy.join("a.pcraft").join("tiles")).unwrap();
    std::fs::write(legacy.join("a.pcraft").join("tiles").join("t0.zst"), b"tile").unwrap();
    let dir = root.join("local").join("Recovery");
    std::fs::create_dir_all(root.join("local")).unwrap();
    move_across(&legacy, &dir).unwrap();
    assert!(!legacy.exists(), "the original is removed after the copy");
    assert_eq!(side_leftovers(&root.join("roaming")), 0, "nothing is left held aside");
    assert_eq!(std::fs::read_to_string(dir.join("a.pcraft").join("manifest.json")).unwrap(), "a");
    assert_eq!(std::fs::read(dir.join("a.pcraft").join("tiles").join("t0.zst")).unwrap(), b"tile");
    assert_eq!(std::fs::read_to_string(dir.join("a.json")).unwrap(), "a");
    // One file on its own, as the per-autosave moves do.
    move_across(&dir.join("a.json"), &root.join("a.json")).unwrap();
    assert!(!dir.join("a.json").exists() && root.join("a.json").is_file());
    assert_eq!(side_leftovers(&dir) + side_leftovers(&root), 0);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_failed_copy_puts_the_original_back_and_leaves_no_partial_copy() {
    let root = temp("recovery-copy-fail");
    let legacy = root.join("Recovery");
    autosave(&legacy, "a");
    // Deeper than any bundle: the copy gives up part way.
    let mut deep = legacy.join("a.pcraft");
    for _ in 0..=MAX_COPY_DEPTH {
        deep = deep.join("d");
    }
    std::fs::create_dir_all(&deep).unwrap();
    let dir = root.join("Local");
    assert!(move_across(&legacy, &dir).is_err());
    assert!(!dir.exists(), "nothing is published");
    assert_eq!(side_leftovers(&root), 0, "no staged copy, nothing left held aside");
    assert!(legacy.join("a.json").is_file() && deep.is_dir(), "the original is back in place");
    // A target that already exists is never copied into or removed.
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("keep"), b"mine").unwrap();
    assert!(move_across(&legacy.join("a.json"), &dir).is_err());
    assert_eq!(std::fs::read(dir.join("keep")).unwrap(), b"mine");
    assert!(legacy.join("a.json").is_file());
    assert_eq!(side_leftovers(&root) + side_leftovers(&legacy), 0);
    let _ = std::fs::remove_dir_all(&root);
}

/// Staged (`.moving`) and held (`.held`) side paths left in `dir`.
fn side_leftovers(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| side_of(n, STAGED).is_some() || side_of(n, HELD).is_some()))
        .count()
}

/// The target appears after the copy is staged (another process got there first): publishing
/// fails, that process's target keeps its contents, the staged copy is removed and the
/// original is left as it was. Folder and file alike.
#[test]
fn a_target_that_appears_while_copying_is_never_replaced_or_removed() {
    let root = temp("recovery-race");
    let legacy = root.join("roaming").join("Recovery");
    autosave(&legacy, "a");
    let local = root.join("local");
    std::fs::create_dir_all(&local).unwrap();
    // A whole folder.
    let dir = local.join("Recovery");
    let staged = stage_copy(&legacy, &dir).unwrap();
    autosave(&dir, "theirs");
    let e = publish(&staged, &dir).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(dir.join("theirs.json")).unwrap(), "theirs", "their folder is intact");
    assert!(!dir.join("a.json").exists(), "nothing was copied into it");
    assert!(!staged.exists() && side_leftovers(&local) == 0, "only the staged copy is removed");
    assert!(legacy.join("a.json").is_file(), "the original is untouched");
    // One file.
    let to = dir.join("a.json");
    let staged = stage_copy(&legacy.join("a.json"), &to).unwrap();
    std::fs::write(&to, "theirs").unwrap();
    assert_eq!(publish(&staged, &to).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&to).unwrap(), "theirs");
    assert!(!staged.exists() && legacy.join("a.json").is_file());
    let _ = std::fs::remove_dir_all(&root);
}

/// A process holds the lock and is copying the old folder (held aside, copy staged) when a
/// second launch gives up waiting. The second launch must get the new folder, never the old
/// one, and what it writes there survives the first one finishing: the publish refuses the
/// now-existing folder and the original is put back, so both autosaves stay (#6412 review).
#[test]
fn a_launch_that_times_out_never_writes_where_a_move_removes() {
    let _spawns = crate::no_child_spawns();
    let root = temp("recovery-timeout");
    let legacy = root.join("roaming").join("Recovery");
    autosave(&legacy, "old");
    let local = root.join("local");
    std::fs::create_dir_all(&local).unwrap();
    let dir = local.join("Recovery");
    // First process: takes the lock, sets the original aside and stages its copy, then pauses.
    let first = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(local.join(MIGRATION_LOCK)).unwrap();
    first.lock().unwrap();
    let held = hold(&legacy).unwrap();
    let staged = stage_copy(&held, &dir).unwrap();
    // Second launch: times out and writes a newer autosave where it was told to.
    let chosen = adopt_legacy_recovery_waiting(&legacy, &dir, std::time::Duration::ZERO);
    assert_eq!(chosen, dir, "a launch that couldn't get the lock never uses the old folder");
    autosave(&chosen, "new");
    // First process resumes: publish, and on failure put the original back (as `move_across`).
    assert_eq!(publish(&staged, &dir).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    put_back(&held, &legacy);
    drop(first);
    assert_eq!(std::fs::read_to_string(dir.join("new.json")).unwrap(), "new", "the newer autosave survives");
    assert_eq!(std::fs::read_to_string(legacy.join("old.json")).unwrap(), "old", "the old one is back in place");
    assert_eq!(side_leftovers(&local) + side_leftovers(&root.join("roaming")), 0);
    // The next launch merges them.
    assert_eq!(adopt_legacy_recovery_waiting(&legacy, &dir, std::time::Duration::ZERO), dir);
    assert!(dir.join("old.json").is_file() && dir.join("new.json").is_file() && !legacy.exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// A writer that still uses the old path while it is being copied (an older PhotoCraft)
/// starts a new folder there; the move removes only the original it held aside.
#[test]
fn a_write_to_the_old_path_during_a_copy_is_not_removed() {
    let root = temp("recovery-old-writer");
    let legacy = root.join("roaming").join("Recovery");
    autosave(&legacy, "old");
    let local = root.join("local");
    std::fs::create_dir_all(&local).unwrap();
    let dir = local.join("Recovery");
    let held = hold(&legacy).unwrap();
    let staged = stage_copy(&held, &dir).unwrap();
    autosave(&legacy, "new");
    publish(&staged, &dir).unwrap();
    std::fs::remove_dir_all(&held).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("old.json")).unwrap(), "old");
    assert_eq!(std::fs::read_to_string(legacy.join("new.json")).unwrap(), "new", "the new write is untouched");
    let _ = std::fs::remove_dir_all(&root);
}

/// What a process killed mid-move left is tidied at the next move: a held original is put
/// back (it may be the only copy) and moved, a staged copy is removed; other dot-files and
/// look-alike names are not touched.
#[test]
fn a_killed_move_is_tidied_next_time() {
    let root = temp("recovery-stale");
    let roaming = root.join("roaming");
    let legacy = roaming.join("Recovery");
    autosave(&legacy, "a");
    let held = hold(&legacy).unwrap();
    assert!(!legacy.exists() && held.is_dir());
    let local = root.join("local");
    let dir = local.join("Recovery");
    let stale = local.join(".Recovery.4294967294-3.moving");
    autosave(&stale, "half");
    let kept = [local.join(".Recovery.moving"), local.join(".keep.1-x.moving"), local.join("notes.txt")];
    for k in &kept {
        std::fs::write(k, b"x").unwrap();
    }
    assert_eq!(adopt_legacy_recovery_waiting(&legacy, &dir, std::time::Duration::ZERO), dir);
    assert_eq!(std::fs::read_to_string(dir.join("a.json")).unwrap(), "a", "the held original was put back and moved");
    assert!(!stale.exists(), "the stale staged copy is gone");
    assert_eq!(side_leftovers(&roaming), 0);
    assert!(kept.iter().all(|k| k.is_file()), "only side names are touched");
    assert_eq!(side_of(".a.json.12-0.moving", STAGED), Some("a.json"));
    assert!(side_of("a.json.12-0.moving", STAGED).is_none() && side_of(".a.json.-0.moving", STAGED).is_none());
    assert!(side_of(".a.json.12-0.held", STAGED).is_none() && side_of(".a.json.12-0.held", HELD) == Some("a.json"));
    let _ = std::fs::remove_dir_all(&root);
}

/// While another process holds the move lock, a launch moves nothing and uses the new folder;
/// once the lock is released, the move happens.
#[test]
fn a_move_waits_for_another_process_holding_the_lock() {
    let _spawns = crate::no_child_spawns();
    let root = temp("recovery-lock");
    let legacy = root.join("roaming").join("Recovery");
    autosave(&legacy, "a");
    let dir = root.join("local").join("Recovery");
    std::fs::create_dir_all(root.join("local")).unwrap();
    let held = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(root.join("local").join(MIGRATION_LOCK)).unwrap();
    held.lock().unwrap();
    assert_eq!(adopt_legacy_recovery_waiting(&legacy, &dir, std::time::Duration::ZERO), dir);
    assert!(legacy.join("a.json").is_file() && !dir.exists(), "nothing moved under another's lock");
    drop(held);
    assert_eq!(adopt_legacy_recovery_waiting(&legacy, &dir, std::time::Duration::ZERO), dir);
    assert!(dir.join("a.json").is_file() && !legacy.exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn no_old_folder_or_the_same_folder_changes_nothing() {
    let root = temp("recovery-noop");
    let dir = root.join("Recovery");
    assert_eq!(adopt_legacy_recovery(&root.join("missing"), &dir), dir);
    assert!(!dir.exists(), "nothing is created up front");
    autosave(&dir, "a");
    assert_eq!(adopt_legacy_recovery(&dir, &dir), dir);
    assert!(dir.join("a.json").is_file());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn missing_exe_dir_or_env_never_panics() {
    let d = resolve(|_| None, None);
    assert_eq!((d.mode, d.dir), (Mode::Platform, None));
    let empty = PathBuf::new();
    let d = resolve(env_of(&[("PHOTOCRAFT_CONFIG_DIR", &empty), ("HOME", &empty), ("APPDATA", &empty)]), None);
    assert_eq!(d.dir, None, "empty variables are ignored");
}
