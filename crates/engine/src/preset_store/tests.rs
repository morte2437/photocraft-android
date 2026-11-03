use super::*;
use photocraft_psd::abr::{AbrSample, LegacyBrush, LegacyTip, write_v12};
use serde_json::json;
use std::path::PathBuf;

/// A fresh, empty directory under the system temp dir (tests never write anywhere else).
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pc-preset-store-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        TempDir(d)
    }
    fn files(&self) -> Vec<String> {
        let mut v: Vec<String> = DirBackend::new(&self.0).list().unwrap().into_iter().map(|(n, _)| n).collect();
        v.sort();
        v
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A session with the store in `dir` attached.
fn session(dir: &TempDir) -> (Session, Vec<String>) {
    let mut s = Session::new();
    let w = s.attach_preset_store(open_dir(&dir.0));
    (s, w)
}

fn tip8(w: u32, h: u32, seed: u32) -> GrayTile {
    GrayTile::from_fn(w, h, |x, y| ((x * 7 + y * 13 + seed) % 256) as f32 / 255.0)
}

fn tip16(w: u32, h: u32, seed: u32) -> GrayTile {
    let data = (0..w * h).map(|i| (i.wrapping_mul(2654435761).wrapping_add(seed) >> 16) as u16).collect();
    GrayTile { width: w, height: h, data }
}

fn sampled(name: &str, group: &str, tip: GrayTile) -> BrushPreset {
    let brush = BrushSettings { size: 42.0, spacing: 0.1, tip: TipShape::Sampled(tip), ..Default::default() };
    BrushPreset { name: name.into(), brush, builtin: false, group: group.into(), folder: Vec::new(), captures: Default::default() }
}

fn user(s: &Session) -> Vec<BrushPreset> {
    s.tools.presets.iter().filter(|p| !p.builtin).cloned().collect()
}

#[test]
fn sessions_have_no_store_by_default() {
    let mut s = Session::new();
    assert!(s.preset_store.is_none(), "headless sessions must stay hermetic");
    s.execute("brush.presets.save", json!({"name": "Mine"})).unwrap();
    assert!(s.preset_store.is_none());
}

#[test]
fn tip_codec_round_trips_8_and_16_bit() {
    let t8 = tip8(37, 21, 3);
    let t16 = tip16(40, 33, 9);
    let e8 = encode_tip(&t8);
    let e16 = encode_tip(&t16);
    assert_eq!(e8[6], 8, "8-bit sourced tips are stored at 8 bits");
    assert_eq!(e16[6], 16);
    assert_eq!(decode_tip(&e8).unwrap(), t8);
    assert_eq!(decode_tip(&e16).unwrap(), t16);
}

#[test]
fn tip_decoder_rejects_garbage() {
    let good = encode_tip(&tip16(16, 16, 1));
    for n in 0..good.len() {
        assert!(decode_tip(&good[..n]).is_err(), "truncated at {n}");
    }
    let mut bad = good.clone();
    bad[6] = 12;
    assert!(decode_tip(&bad).is_err());
    // A header claiming a huge bitmap is refused before allocating.
    let mut huge = good.clone();
    huge[7..11].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_tip(&huge).is_err());
    let mut big = good.clone();
    big[7..11].copy_from_slice(&MAX_TIP_SIDE.to_le_bytes());
    big[11..15].copy_from_slice(&MAX_TIP_SIDE.to_le_bytes());
    assert!(decode_tip(&big).is_err());
    let mut rng = 0x1234_5678u32;
    for _ in 0..200 {
        let len = (rng % 64) as usize;
        let v: Vec<u8> = (0..len)
            .map(|_| {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                rng as u8
            })
            .collect();
        let _ = decode_tip(&v);
        let mut w = TIP_MAGIC.to_vec();
        w.extend_from_slice(&v);
        let _ = decode_tip(&w);
    }
}

#[test]
fn group_with_sampled_8_and_16_bit_tips_round_trips() {
    let dir = TempDir::new("roundtrip");
    let (mut s, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert!(dir.files().is_empty(), "attaching an empty store writes nothing");
    let mut a = sampled("Eight", "Inks", tip8(64, 48, 1));
    a.brush.dual_brush.tip = TipShape::Sampled(tip8(12, 12, 5));
    a.brush.texture.pattern = Pattern::Tile(tip16(32, 32, 7));
    let b = sampled("Sixteen", "Inks", tip16(50, 70, 2));
    let mut c = sampled("Shared tip", "Other", tip8(64, 48, 1));
    c.brush.size = 9.0;
    s.tools.presets.extend([a, b, c]);
    s.brush_presets_changed();
    s.sync_preset_store();
    let files = dir.files();
    assert_eq!(files.iter().filter(|f| f.ends_with(".pcbrushes")).count(), 2, "{files:?}");
    // Four distinct bitmaps (the 64×48 tip is shared by two presets) plus the previews of the two
    // 16-bit ones: an 8-bit tip this small is its own preview.
    assert_eq!(files.iter().filter(|f| f.starts_with("tips/")).count(), 6, "{files:?}");
    // The session keeps previews, not the bitmaps (#1843).
    assert!(user(&s).iter().all(|p| matches!(&p.brush.tip, TipShape::Stored(r) if !r.is_loaded())));
    assert!(files.contains(&INDEX_FILE.to_string()));

    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(user(&t), user(&s));
    let order: Vec<String> = user(&t).into_iter().map(|p| p.name).collect();
    assert_eq!(order, ["Eight", "Sixteen", "Shared tip"]);
    // Nothing changed: reloading rewrote nothing.
    let before: Vec<_> = DirBackend::new(&dir.0).list().unwrap();
    let mut t = t;
    t.brush_presets_changed();
    t.sync_preset_store();
    assert_eq!(DirBackend::new(&dir.0).list().unwrap(), before);
}

#[test]
fn commands_persist_and_delete_removes_from_disk() {
    let dir = TempDir::new("commands");
    let (mut s, _) = session(&dir);
    s.tools.brush.tip = TipShape::Sampled(tip16(20, 20, 4));
    s.execute("brush.presets.save", json!({"name": "Scribble"})).unwrap();
    assert_eq!(dir.files().iter().filter(|f| f.starts_with("tips/")).count(), 2, "the tip and its preview");
    // The tool's brush now is the saved preset's stored tip, still loaded.
    assert!(matches!(&s.tools.brush.tip, TipShape::Stored(r) if r.is_loaded()));
    let (t, _) = session(&dir);
    let p = photocraft_paint::presets::find(&t.tools.presets, "Scribble").expect("saved preset reloads");
    assert_eq!(p.brush.tip, s.tools.brush.tip);

    // Rename through the Preset Manager, then delete.
    s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "brushes", "name": "Scribble", "newName": "Doodle"})).unwrap();
    let (t, _) = session(&dir);
    assert!(photocraft_paint::presets::find(&t.tools.presets, "Doodle").is_some());
    assert!(photocraft_paint::presets::find(&t.tools.presets, "Scribble").is_none());
    s.execute("brush.presets.delete", json!({"name": "Doodle"})).unwrap();
    assert_eq!(dir.files(), [INDEX_FILE], "the group file and its tip are gone");
    let (t, _) = session(&dir);
    assert!(photocraft_paint::presets::find(&t.tools.presets, "Doodle").is_none());
}

/// The New Brush Preset dialog's boxes (#4490) are saved with the preset and come back with it;
/// a preset saved without them reloads with the default captures and writes no field for them.
#[test]
fn preset_captures_persist() {
    let dir = TempDir::new("captures");
    let (mut s, _) = session(&dir);
    s.execute("brush.presets.save", json!({"name": "Bare", "captureSize": false, "toolSettings": false, "color": true})).unwrap();
    s.execute("brush.presets.save", json!({"name": "Plain"})).unwrap();
    let (t, _) = session(&dir);
    let find = |n: &str| photocraft_paint::presets::find(&t.tools.presets, n).expect("reloads").captures.clone();
    assert_eq!(find("Bare"), PresetCaptures { size: false, tool_settings: false, color: true });
    assert!(find("Plain").is_default());
    let written: String = dir.files().iter().filter(|f| !f.starts_with("tips/")).map(|f| std::fs::read_to_string(dir.0.join(f)).unwrap_or_default()).collect();
    assert_eq!(written.matches("\"captures\"").count(), 1, "only the preset with boxes records them");
}

#[test]
fn imported_abr_group_persists() {
    let dir = TempDir::new("abr");
    let (mut s, _) = session(&dir);
    let tip = AbrSample { id: String::new(), width: 6, height: 4, depth: 8, data: (0..24).map(|i| i * 10).collect() };
    let abr = write_v12(
        2,
        &[
            LegacyBrush { name: "Grit".into(), spacing: 25, anti_alias: true, tip: LegacyTip::Sampled(tip) },
            LegacyBrush {
                name: "Round 9".into(),
                spacing: 10,
                anti_alias: true,
                tip: LegacyTip::Computed { diameter: 9, hardness: 100, angle: 0, roundness: 100 },
            },
        ],
        true,
    )
    .unwrap();
    let data = photocraft_paint::tile::b64_encode(&abr);
    s.execute("brush.presets.importAbr", json!({"data": data, "group": "Legacy Set"})).unwrap();
    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    let grp: Vec<&BrushPreset> = t.tools.presets.iter().filter(|p| p.group == "Legacy Set").collect();
    assert_eq!(grp.len(), 2);
    assert_eq!(user(&t), user(&s));
    // Deleting every preset of the group removes its file and tips.
    let names: Vec<String> = grp.iter().map(|p| p.name.clone()).collect();
    for n in names {
        s.execute("brush.presets.delete", json!({"name": n})).unwrap();
    }
    assert!(!dir.files().iter().any(|f| f.ends_with(".pcbrushes") || f.starts_with("tips/")), "{:?}", dir.files());
}

#[test]
fn deleted_builtins_stay_deleted_and_overrides_replace_them() {
    let dir = TempDir::new("builtins");
    let (mut s, _) = session(&dir);
    let first = s.tools.presets[0].name.clone();
    let second = s.tools.presets[1].name.clone();
    s.execute("brush.presets.delete", json!({"name": first})).unwrap();
    s.tools.brush.size = 77.0;
    s.execute("brush.presets.save", json!({"name": second})).unwrap();
    let (t, _) = session(&dir);
    assert!(photocraft_paint::presets::find(&t.tools.presets, &first).is_none());
    let p = photocraft_paint::presets::find(&t.tools.presets, &second).unwrap();
    assert_eq!((p.builtin, p.brush.size), (false, 77.0));
    assert_eq!(t.tools.presets.iter().filter(|p| p.name.eq_ignore_ascii_case(&second)).count(), 1);
    // No built-in is ever written.
    let n = dir.files().iter().filter(|f| f.ends_with(".pcbrushes")).count();
    assert_eq!(n, 1);
}

#[test]
fn startup_with_many_groups_loads_them_all() {
    let dir = TempDir::new("many");
    let (mut s, _) = session(&dir);
    for g in 0..25 {
        for i in 0..4 {
            s.tools.presets.push(sampled(&format!("P{g}-{i}"), &format!("Group {g}"), tip8(16 + i, 16, g)));
        }
    }
    s.brush_presets_changed();
    s.sync_preset_store();
    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(user(&t).len(), 100);
    let groups: Vec<String> = user(&t).iter().map(|p| p.group.clone()).fold(Vec::new(), |mut v, g| {
        if !v.contains(&g) {
            v.push(g);
        }
        v
    });
    assert_eq!(groups, (0..25).map(|g| format!("Group {g}")).collect::<Vec<_>>(), "group order survives");
}

#[test]
fn reordering_presets_and_groups_persists() {
    let names = |s: &Session| s.tools.presets.iter().map(|p| (p.name.clone(), p.group.clone())).collect::<Vec<_>>();
    let dir = TempDir::new("reorder");
    let (mut s, _) = session(&dir);
    for (n, g) in [("Ink 1", "Inks"), ("Ink 2", "Inks"), ("Wash", "Washes")] {
        s.execute("brush.presets.save", json!({"name": n})).unwrap();
        s.tools.presets.iter_mut().find(|p| p.name == n).unwrap().group = g.into();
    }
    s.brush_presets_changed();
    s.sync_preset_store();
    // Untouched order: no order list is written (the default order is implied).
    let (t, _) = session(&dir);
    assert_eq!(names(&t), names(&s));
    // Reorder a built-in within its group, a user preset across groups, and whole groups.
    let builtin_group = s.tools.presets[0].group.clone();
    let last_builtin = s.tools.presets.iter().rfind(|p| p.builtin && p.group == builtin_group).unwrap().name.clone();
    let first = s.tools.presets[0].name.clone();
    s.execute("brush.presets.move", json!({"name": last_builtin, "before": first})).unwrap();
    s.execute("brush.presets.move", json!({"name": "Ink 2", "group": "Washes", "index": 0})).unwrap();
    s.execute("brush.presets.moveGroup", json!({"group": "Washes", "index": 0})).unwrap();
    s.execute("brush.presets.moveGroup", json!({"group": "Inks", "before": builtin_group})).unwrap();
    assert_eq!(s.tools.presets[0].name, "Ink 2");
    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(names(&t), names(&s), "the panel order survives a restart");
    assert!(t.tools.presets.iter().find(|p| p.name == last_builtin).unwrap().builtin, "reordering keeps a built-in built-in");
    // Renaming and deleting a group persist too.
    s.execute("brush.presets.renameGroup", json!({"group": "Inks", "newName": "Pens"})).unwrap();
    s.execute("brush.presets.deleteGroup", json!({"group": "Washes"})).unwrap();
    let (t, _) = session(&dir);
    assert_eq!(names(&t), names(&s));
    assert!(t.tools.presets.iter().all(|p| p.group != "Washes" && p.group != "Inks"));
}

#[test]
fn corrupt_and_oversized_files_are_skipped_with_a_warning() {
    let dir = TempDir::new("corrupt");
    let (mut s, _) = session(&dir);
    s.tools.presets.push(sampled("Good", "Fine", tip8(8, 8, 1)));
    s.tools.presets.push(sampled("Lost", "Broken tip", tip16(9, 9, 2)));
    s.brush_presets_changed();
    s.sync_preset_store();
    let be = DirBackend::new(&dir.0);
    be.write("junk-00000000.pcbrushes", b"{not json").unwrap();
    be.write("wrong-00000000.pcbrushes", br#"{"format":"other","version":1,"group":"x","presets":[]}"#).unwrap();
    // Corrupt the 16-bit tip of "Broken tip".
    let h = tip_hash(&tip16(9, 9, 2));
    be.write(&tip_file(&h), b"PCTIP1\x10garbage").unwrap();
    let (mut t, w) = session(&dir);
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(photocraft_paint::presets::find(&t.tools.presets, "Good").is_some());
    // Full tips load when a brush is used (#1843): the broken one fails then, not at start.
    assert!(photocraft_paint::presets::find(&t.tools.presets, "Lost").is_some());
    let before = t.tools.brush.clone();
    let e = t.execute("tools.setBrush", json!({"preset": "Lost"})).unwrap_err().to_string();
    assert!(e.contains(&h), "{e}");
    assert_eq!(t.tools.brush, before, "a failed pick leaves the brush alone");
    t.execute("tools.setBrush", json!({"preset": "Good"})).unwrap();
    // The broken files are left alone for the user (never deleted by a sync).
    t.execute("brush.presets.save", json!({"name": "Another"})).unwrap();
    let files = dir.files();
    assert!(files.contains(&"junk-00000000.pcbrushes".to_string()) && files.contains(&tip_file(&h)), "{files:?}");

    // Oversized files are refused without reading them whole.
    let mem = MemBackend::default();
    mem.files.lock().unwrap().insert("big-00000000.pcbrushes".into(), vec![b' '; MAX_GROUP_BYTES as usize + 1]);
    mem.files.lock().unwrap().insert(INDEX_FILE.into(), b"[1,2".to_vec());
    let o = open(Box::new(mem));
    assert!(o.presets.is_empty());
    assert_eq!(o.warnings.len(), 2, "{:?}", o.warnings);
}

#[test]
fn write_failures_are_warnings() {
    let dir = TempDir::new("readonly");
    // The store "directory" is a file: every write fails.
    std::fs::write(&dir.0, b"not a dir").unwrap();
    let mut s = Session::new();
    s.attach_preset_store(open_dir(&dir.0));
    s.tools.brush.tip = TipShape::Sampled(tip8(4, 4, 0));
    s.execute("brush.presets.save", json!({"name": "Nope"})).unwrap();
    let w = s.preset_store.as_mut().unwrap().take_warnings();
    assert!(!w.is_empty());
    let _ = std::fs::remove_file(&dir.0);
}

#[test]
fn backends_refuse_paths_outside_the_store() {
    let mem = MemBackend::default();
    for n in ["../x.pcbrushes", "a/b.pcbrushes", "tips/../x", ".hidden", "", "tips/"] {
        assert!(mem.write(n, b"x").is_err(), "{n}");
    }
    assert!(group_file_name("../../etc/passwd").ends_with(".pcbrushes"));
    assert!(valid_name(&group_file_name("../../etc/passwd")));
    assert!(valid_name(&group_file_name("")));
    assert!(valid_name(&group_file_name("日本語 ブラシ")));
    assert_ne!(group_file_name("a/b"), group_file_name("a_b"));
}

/// `cargo test --release -p photocraft-engine --lib preset_store::tests::bench_load_500 -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_load_500_sampled_presets() {
    let dir = TempDir::new("bench");
    let (mut s, _) = session(&dir);
    for i in 0..500u32 {
        // A mix of 8-bit (most .abr tips) and 16-bit tips, 64..319 px.
        let side = 64 + (i * 37) % 256;
        let t = if i % 4 == 0 { tip16(side, side, i) } else { tip8(side, side, i) };
        s.tools.presets.push(sampled(&format!("Brush {i}"), &format!("Set {}", i / 50), t));
    }
    s.brush_presets_changed();
    let t0 = std::time::Instant::now();
    s.sync_preset_store();
    let write = t0.elapsed();
    let bytes = s.preset_store.as_ref().unwrap().bytes();
    let t0 = std::time::Instant::now();
    let o = open_dir(&dir.0);
    let load = t0.elapsed();
    assert_eq!(o.presets.len(), 500);
    let t0 = std::time::Instant::now();
    s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "brushes", "name": "Brush 3", "newName": "Renamed"})).unwrap();
    let rename = t0.elapsed();
    eprintln!("500 sampled presets: {:.1} MB on disk; write {write:?}, load {load:?}, rename {rename:?}", bytes as f64 / 1e6);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_store_directory_that_does_not_exist_yet_opens_without_warnings() {
    // Windows reports a file under a missing directory as "path not found" (os error 3), not
    // "file not found"; both mean an empty store, as on a first launch.
    let parent = TempDir::new("missing-store");
    let dir = parent.0.join("not-created-yet");
    assert!(!dir.exists());
    let opened = open_dir(&dir);
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    assert!(opened.actions.is_empty());
    let err = DirBackend::new(&dir).read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap_err();
    assert!(missing_file(&err), "{err}");
}

// ------------------------------------------------------------------ on-demand tips (#1843)

/// A session on an in-memory store holding `n` presets with distinct `side`-px 8-bit tips (one
/// group), synced; plus the backend to reopen it.
fn library(n: u32, side: u32) -> (Session, MemBackend) {
    let mem = MemBackend::default();
    let mut s = Session::new();
    s.attach_preset_store(open(Box::new(mem.clone())));
    for i in 0..n {
        s.tools.presets.push(sampled(&format!("Big {i}"), "Big Set", tip8(side, side, i)));
    }
    s.brush_presets_changed();
    s.sync_preset_store();
    (s, mem)
}

/// Bitmap samples a preset library holds (full tips, or previews of stored ones not loaded).
fn library_samples(s: &Session) -> usize {
    let b = |p: &BrushPreset| {
        [
            p.brush.tip.bitmap().map(|g| g.data.len()),
            p.brush.dual_brush.tip.bitmap().map(|g| g.data.len()),
            p.brush.texture.pattern.bitmap().map(|g| g.data.len()),
        ]
    };
    s.tools.presets.iter().flat_map(b).flatten().sum()
}

fn cache(s: &Session) -> TipCacheStats {
    s.preset_store.as_ref().unwrap().tip_cache()
}

#[test]
fn reopening_a_library_decodes_no_tips_and_using_a_brush_decodes_only_its_own() {
    let (n, side) = (20u32, 512u32);
    let (s, mem) = library(n, side);
    // Once written, the imported presets keep previews only (built-ins keep their small tips).
    let budget = (n as usize + 40) * (PREVIEW_SIDE * PREVIEW_SIDE) as usize;
    assert!(library_samples(&s) < budget, "{}", library_samples(&s));
    assert_eq!(cache(&s).decodes, 0);
    let files = mem.list().unwrap();
    drop(s);

    // A restart: nothing is decoded at full size.
    let mut t = Session::new();
    let w = t.attach_preset_store(open(Box::new(mem.clone())));
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(mem.list().unwrap(), files, "attaching an unchanged store rewrites nothing");
    let user_presets = user(&t);
    assert_eq!(user_presets.len(), n as usize);
    for p in &user_presets {
        let TipShape::Stored(r) = &p.brush.tip else { panic!("{} is not a stored tip", p.name) };
        assert!(!r.is_loaded() && (r.width, r.height) == (side, side));
        assert!(r.preview.width <= PREVIEW_SIDE && r.preview.height <= PREVIEW_SIDE);
    }
    assert!(library_samples(&t) < budget);
    assert_eq!(cache(&t), TipCacheStats { tips: 0, bytes: 0, decodes: 0, limit: TIP_CACHE_BYTES });

    // Using one brush decodes exactly its tip; painting and picking it again reuse it.
    t.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    t.execute("tools.setBrush", json!({"preset": "Big 3"})).unwrap();
    let TipShape::Stored(r) = &t.tools.brush.tip else { panic!("the brush keeps its stored tip") };
    assert_eq!(r.full.as_deref(), Some(&tip8(side, side, 3)), "the tool paints with the full tip");
    assert_eq!(cache(&t).decodes, 1);
    t.execute("paint.stroke", json!({"points": [[10, 10], [50, 50]], "brush": {"size": 30}})).unwrap();
    t.execute("paint.stroke", json!({"points": [[10, 50], [50, 10]], "preset": "Big 3"})).unwrap();
    t.execute("tools.setBrush", json!({"preset": "Big 3"})).unwrap();
    assert_eq!((cache(&t).decodes, cache(&t).tips), (1, 1));
    // Picking a preset never loads the library's other tips, and the library stays light.
    assert!(user(&t).iter().all(|p| matches!(&p.brush.tip, TipShape::Stored(r) if !r.is_loaded())));
    // The panel's "current brush" match still works on a stored tip.
    let picked = photocraft_paint::presets::find(&t.tools.presets, "Big 3").unwrap();
    assert_eq!(picked.brush.tip, t.tools.brush.tip);
}

#[test]
fn the_decoded_tip_cache_stays_within_its_budget() {
    let (s, mem) = library(6, 256);
    drop(s);
    let mut t = Session::new();
    t.attach_preset_store(open(Box::new(mem)));
    let one = 256 * 256 * 2;
    t.preset_store.as_ref().unwrap().set_tip_cache_limit(one * 2 + 1);
    for i in 0..6 {
        t.execute("tools.setBrush", json!({"preset": format!("Big {i}")})).unwrap();
        let c = cache(&t);
        assert!(c.bytes <= c.limit && c.tips <= 2, "{c:?}");
    }
    assert_eq!(cache(&t).decodes, 6);
    // The two most recently used stay: picking them again decodes nothing.
    t.execute("tools.setBrush", json!({"preset": "Big 4"})).unwrap();
    t.execute("tools.setBrush", json!({"preset": "Big 5"})).unwrap();
    assert_eq!(cache(&t).decodes, 6);
    t.execute("tools.setBrush", json!({"preset": "Big 0"})).unwrap();
    assert_eq!(cache(&t).decodes, 7);
    // A budget below one tip still loads it (the brush in use holds its own reference).
    t.preset_store.as_ref().unwrap().set_tip_cache_limit(0);
    assert_eq!(cache(&t).tips, 0);
    t.execute("tools.setBrush", json!({"preset": "Big 1"})).unwrap();
    assert!(matches!(&t.tools.brush.tip, TipShape::Stored(r) if r.is_loaded()));
}

#[test]
fn missing_corrupt_or_unknown_tips_fail_when_used_without_panicking() {
    let (s, mem) = library(3, 64);
    drop(s);
    let mut t = Session::new();
    t.attach_preset_store(open(Box::new(mem.clone())));
    t.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    let key = |t: &Session, name: &str| match &photocraft_paint::presets::find(&t.tools.presets, name).unwrap().brush.tip {
        TipShape::Stored(r) => r.key.clone(),
        other => panic!("{other:?}"),
    };
    // Deleted behind the store's back.
    let k0 = key(&t, "Big 0");
    mem.remove(&tip_file(&k0)).unwrap();
    t.execute("tools.setBrush", json!({"preset": "Hard Round"})).unwrap();
    let broken = t.tools.presets.iter_mut().find(|p| p.name == "Big 0").unwrap();
    broken.captures.color = true;
    broken.brush.color = [1.0, 0.0, 0.0, 1.0];
    t.tools.foreground = [0.0, 0.0, 1.0, 1.0];
    let before = t.tools.brush.clone();
    let preset_before = t.tools.current_preset.clone();
    let foreground_before = t.tools.foreground;
    let revision_before = t.tools.brush_rev;
    assert!(t.execute("tools.setBrush", json!({"preset": "Big 0"})).is_err());
    assert!(t.execute("paint.stroke", json!({"points": [[1, 1], [9, 9]], "preset": "Big 0"})).is_err());
    assert_eq!(t.tools.brush, before);
    assert_eq!(t.tools.current_preset, preset_before);
    assert_eq!(t.tools.foreground, foreground_before);
    assert_eq!(t.tools.brush_rev, revision_before);
    // Corrupt, or a different bitmap under the same name.
    let k1 = key(&t, "Big 1");
    mem.write(&tip_file(&k1), b"PCTIP1\x08garbage").unwrap();
    assert!(t.execute("tools.setBrush", json!({"preset": "Big 1"})).is_err());
    let k2 = key(&t, "Big 2");
    mem.write(&tip_file(&k2), &encode_tip(&tip8(10, 10, 0))).unwrap();
    let e = t.execute("tools.setBrush", json!({"preset": "Big 2"})).unwrap_err().to_string();
    assert!(e.contains("expected 64×64"), "{e}");
    // Exporting a library whose tip is gone is an error too, not a file with a hole.
    assert!(t.execute("edit.presets.exportImportPresets", json!({"action": "export"})).is_err());
    // A stored reference the store doesn't know, or with no store at all.
    let stray = json!({"tip": {"stored": {"key": "../../etc/passwd", "width": 4, "height": 4,
        "preview": {"width": 1, "height": 1, "data": [1.0]}}}});
    assert!(t.execute("tools.setBrush", json!({"brush": stray})).is_err());
    let mut headless = Session::new();
    assert!(headless.execute("tools.setBrush", json!({"brush": stray})).is_err());
    assert!(headless.execute("brush.presets.list", json!({"full": true})).is_ok());
}

#[test]
fn a_store_without_previews_makes_them_once_and_then_loads_lazily() {
    // A store written before #1843: group files without tip sizes, no preview files.
    let mem = MemBackend::default();
    let tips = [tip16(300, 200, 1), tip8(40, 40, 2)];
    let hashes: Vec<String> = tips.iter().map(tip_hash).collect();
    for (t, h) in tips.iter().zip(&hashes) {
        mem.write(&tip_file(h), &encode_tip(t)).unwrap();
    }
    let group = json!({"format": GROUP_FORMAT, "version": 1, "group": "Old", "presets": [
        {"name": "Old A", "brush": {"size": 30}, "tips": {"tip": hashes[0], "texture": hashes[1]}},
        {"name": "Old B", "brush": {"size": 12}, "tips": {"tip": hashes[1]}},
    ]});
    let file = group_file_name("Old");
    mem.write(&file, &serde_json::to_vec(&group).unwrap()).unwrap();

    let mut s = Session::new();
    let w = s.attach_preset_store(open(Box::new(mem.clone())));
    assert!(w.is_empty(), "{w:?}");
    let a = photocraft_paint::presets::find(&s.tools.presets, "Old A").unwrap().clone();
    assert!(matches!(&a.brush.tip, TipShape::Stored(r) if (r.width, r.height) == (300, 200) && !r.is_loaded()));
    assert!(matches!(&a.brush.texture.pattern, Pattern::Stored(r) if r.key == hashes[1]));
    // Attaching rewrote the group with sizes and previews.
    let rewritten: serde_json::Value = serde_json::from_slice(&mem.read(&file, MAX_GROUP_BYTES).unwrap()).unwrap();
    assert_eq!(rewritten["tips"][&hashes[0]]["width"], 300);
    assert_eq!(rewritten["presets"][0]["brush"]["size"], 30.0);
    // The next start reads previews only and changes nothing.
    let files = mem.list().unwrap();
    let mut t = Session::new();
    assert!(t.attach_preset_store(open(Box::new(mem.clone()))).is_empty());
    assert_eq!(mem.list().unwrap(), files);
    assert_eq!(user(&t), user(&s));
    t.execute("tools.setBrush", json!({"preset": "Old A"})).unwrap();
    assert_eq!(t.tools.brush.tip.bitmap(), Some(&tips[0]));
    assert_eq!(t.tools.brush.texture.pattern.bitmap(), Some(&tips[1]));
    assert_eq!(cache(&t).decodes, 2);
    // Exported presets embed the full bitmaps, as before.
    let out = t.execute("edit.presets.exportImportPresets", json!({"action": "export"})).unwrap();
    let back: Vec<BrushPreset> = serde_json::from_value(out["data"]["brushes"].clone()).unwrap();
    let a = back.iter().find(|p| p.name == "Old A").unwrap();
    assert_eq!(a.brush.tip, TipShape::Sampled(tips[0].clone()));
    assert_eq!(a.brush.texture.pattern, Pattern::Tile(tips[1].clone()));
}

/// A v6 `.abr` whose presets sit in Photoshop folders (`phry`).
fn abr_with_folders(presets: &[(&str, &[&str])]) -> Vec<u8> {
    use photocraft_psd::descriptor::{Descriptor, UnicodeString, Value};
    let descs: Vec<Descriptor> = presets.iter().map(|(n, _)| Descriptor::new("brushPreset").with("Nm  ", Value::Text(UnicodeString::new_nul(n)))).collect();
    let folders: Vec<Vec<String>> = presets.iter().map(|(_, f)| f.iter().map(|x| x.to_string()).collect()).collect();
    photocraft_psd::abr::write_v6_folders(2, &[], &[], &descs, &folders, true).unwrap()
}

#[test]
fn imported_abr_folders_persist_and_old_stores_load_flat() {
    let dir = TempDir::new("abr-folders");
    let (mut s, _) = session(&dir);
    let abr = abr_with_folders(&[("Loose", &[]), ("Pen", &["Inks"]), ("Nib", &["Inks", "Fine"]), ("Kit Chalk", &["Dry"])]);
    let r = s.execute("brush.presets.importAbr", json!({"data": photocraft_paint::tile::b64_encode(&abr), "group": "Kit"})).unwrap();
    assert_eq!((r["count"].as_u64(), r["folders"].as_u64()), (Some(4), Some(3)));
    let folder = |s: &Session, n: &str| photocraft_paint::presets::find(&s.tools.presets, n).unwrap().folder.join("/");
    assert_eq!((folder(&s, "Loose"), folder(&s, "Nib"), folder(&s, "Kit Chalk")), ("".into(), "Inks/Fine".into(), "Dry".into()));
    // One group file still holds the whole imported set, folders included.
    let groups: Vec<String> = dir.files().into_iter().filter(|f| f.ends_with(".pcbrushes")).collect();
    assert_eq!(groups, [group_file_name("Kit")]);
    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(user(&t), user(&s));
    // Renaming a nested folder rewrites the group; the next start sees the new name.
    s.execute("brush.presets.renameGroup", json!({"group": "Kit", "folder": ["Inks"], "newName": "Ink"})).unwrap();
    let (t, _) = session(&dir);
    assert_eq!(folder(&t, "Nib"), "Ink/Fine");
    // Exported preset files keep the folders.
    let out = s.execute("edit.presets.exportImportPresets", json!({"action": "export"})).unwrap();
    assert_eq!(out["data"]["brushes"].as_array().unwrap().iter().find(|b| b["name"] == "Nib").unwrap()["folder"], json!(["Ink", "Fine"]));

    // A group file written before #1851 has no `folder`: every preset loads directly in its group,
    // without a warning and without rewriting the file.
    let mem = MemBackend::default();
    let old = json!({"format": GROUP_FORMAT, "version": 1, "group": "Old", "presets": [{"name": "Plain", "brush": {"size": 5}}]});
    let file = group_file_name("Old");
    mem.write(&file, &serde_json::to_vec(&old).unwrap()).unwrap();
    let before = mem.read(&file, MAX_GROUP_BYTES).unwrap();
    let mut o = Session::new();
    assert!(o.attach_preset_store(open(Box::new(mem.clone()))).is_empty());
    let p = photocraft_paint::presets::find(&o.tools.presets, "Plain").unwrap();
    assert!(p.group == "Old" && p.folder.is_empty());
    assert_eq!(mem.read(&file, MAX_GROUP_BYTES).unwrap(), before);
    // A hand-edited folder path is bounded.
    let deep = json!({"format": GROUP_FORMAT, "version": 1, "group": "Deep", "presets": [
        {"name": "Deep", "brush": {}, "folder": vec!["x"; 1000]},
        {"name": "Blank", "brush": {}, "folder": ["", "  "]},
    ]});
    mem.write(&group_file_name("Deep"), &serde_json::to_vec(&deep).unwrap()).unwrap();
    let mut d = Session::new();
    d.attach_preset_store(open(Box::new(mem.clone())));
    assert_eq!(photocraft_paint::presets::find(&d.tools.presets, "Deep").unwrap().folder.len(), crate::brush_preset_cmds::MAX_FOLDER_DEPTH);
    assert!(photocraft_paint::presets::find(&d.tools.presets, "Blank").unwrap().folder.is_empty());
}

/// Moving a group into another group's folder rewrites the target group's file and removes the
/// moved group's; moving a folder out to the top level makes a group file for it (#2014).
#[test]
fn folders_moved_into_other_folders_persist() {
    let dir = TempDir::new("folder-into");
    let (mut s, _) = session(&dir);
    let set = abr_with_folders(&[("Kit Loose", &[]), ("Kit Pen", &["Inks"]), ("Kit Nib", &["Inks", "Fine"])]);
    let other = abr_with_folders(&[("Pack Chalk", &["Dry"])]);
    s.execute("brush.presets.importAbr", json!({"data": photocraft_paint::tile::b64_encode(&set), "group": "Kit"})).unwrap();
    s.execute("brush.presets.importAbr", json!({"data": photocraft_paint::tile::b64_encode(&other), "group": "Pack"})).unwrap();
    let group_files = |d: &TempDir| -> Vec<String> { d.files().into_iter().filter(|f| f.ends_with(".pcbrushes")).collect() };
    let mut both = vec![group_file_name("Kit"), group_file_name("Pack")];
    both.sort();
    assert_eq!(group_files(&dir), both);
    s.execute("brush.presets.moveGroup", json!({"group": "Pack", "into": {"group": "Kit", "folder": ["Inks"]}})).unwrap();
    assert_eq!(group_files(&dir), [group_file_name("Kit")]);
    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(user(&t), user(&s));
    let folder = |s: &Session, n: &str| {
        let p = photocraft_paint::presets::find(&s.tools.presets, n).unwrap();
        format!("{}:{}", p.group, p.folder.join("/"))
    };
    assert_eq!(folder(&t, "Pack Chalk"), "Kit:Inks/Pack/Dry");
    // Out to the top level: a group (and file) of its own.
    s.execute("brush.presets.moveGroup", json!({"group": "Kit", "folder": ["Inks", "Pack"], "into": {}})).unwrap();
    let mut both = vec![group_file_name("Kit"), group_file_name("Pack")];
    both.sort();
    assert_eq!(group_files(&dir), both);
    let (t, _) = session(&dir);
    assert_eq!(folder(&t, "Pack Chalk"), "Pack:Dry");
    assert_eq!(folder(&t, "Kit Nib"), "Kit:Inks/Fine");
}

#[test]
fn a_group_file_that_failed_to_load_is_not_overwritten() {
    // e.g. written by a newer PhotoCraft: skipped on load, so a save must not replace it.
    let mem = MemBackend::default();
    let file = group_file_name("");
    let newer = br#"{"format":"photocraft-brush-group","version":2,"group":"","presets":[]}"#.to_vec();
    mem.files.lock().unwrap().insert(file.clone(), newer.clone());
    let mut s = Session::new();
    s.attach_preset_store(open(Box::new(mem.clone())));
    s.tools.brush.tip = TipShape::Sampled(tip8(4, 4, 0));
    s.execute("brush.presets.save", json!({"name": "New"})).unwrap();
    assert_eq!(mem.files.lock().unwrap().get(&file), Some(&newer), "a file the store could not read was overwritten");
    assert!(!s.preset_store.as_mut().unwrap().take_warnings().is_empty());
}

#[test]
fn an_actions_file_that_failed_to_load_is_not_overwritten() {
    // e.g. written by a newer PhotoCraft: skipped on load, so recording an action must not replace it.
    let mem = MemBackend::default();
    let newer = br#"{"version":2,"actions":[{"name":"Keep","steps":[]}]}"#.to_vec();
    mem.files.lock().unwrap().insert(ACTIONS_FILE.into(), newer.clone());
    let mut s = Session::new();
    s.attach_preset_store(open(Box::new(mem.clone())));
    s.execute("actions.record", json!({"name": "New"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    s.sync_preset_store();
    assert_eq!(mem.files.lock().unwrap().get(ACTIONS_FILE), Some(&newer), "an actions file the store could not read was overwritten");
    assert!(!s.preset_store.as_mut().unwrap().take_warnings().is_empty());
}

/// A large pattern opens as a small preview (texture libraries run to gigabytes) and is read at
/// full size when it is used; its preview file goes with it on delete.
#[test]
fn large_patterns_load_on_use() {
    let dir = TempDir::new("patterns-large");
    let (mut s, _) = session(&dir);
    s.execute("file.new", json!({"width": 300, "height": 200})).unwrap();
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    s.execute("select.rect", json!({"x": 250, "y": 150, "width": 50, "height": 50})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("edit.definePattern", json!({"name": "Big"})).unwrap();
    assert_eq!(dir.files().iter().filter(|f| f.ends_with(".preview.pat")).count(), 1);

    let (mut t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    let held = t.patterns.items.iter().find(|p| p.name == "Big").unwrap();
    assert!(held.width <= PATTERN_PREVIEW_SIDE && held.height <= PATTERN_PREVIEW_SIDE, "held as a preview: {}x{}", held.width, held.height);
    let listed = t.execute("pattern.list", json!({})).unwrap();
    let big = listed.as_array().unwrap().iter().find(|p| p["name"] == "Big").unwrap();
    assert_eq!((big["width"].as_u64(), big["height"].as_u64()), (Some(300), Some(200)), "the list reports the full size");
    // Checks that only need the pattern's id (Pattern Stamp's enabled state) don't read its file.
    let id = t.patterns.items.iter().find(|p| p.name == "Big").unwrap().id.clone();
    let file = dir.0.join(patterns::file_name_for_test(&id));
    let moved = file.with_extension("moved");
    std::fs::rename(&file, &moved).unwrap();
    assert_eq!(crate::pattern_cmds::resolve_id(&t, "Big"), Some(id.clone()));
    assert!(crate::pattern_cmds::resolve(&t, "Big").is_none(), "the full pixels come from the file");
    std::fs::rename(&moved, &file).unwrap();
    let full = crate::pattern_cmds::resolve(&t, "Big").unwrap();
    assert_eq!((full.width, full.height), (300, 200));
    assert!(full.surface.pixel(280, 180)[1] < 0.01 && full.surface.pixel(10, 10)[1] > 0.99, "full pixels");
    // Once read, it is kept: using it again doesn't read the file.
    std::fs::rename(&file, &moved).unwrap();
    assert_eq!(crate::pattern_cmds::resolve(&t, "Big").map(|p| p.width), Some(300));
    std::fs::rename(&moved, &file).unwrap();
    // A Pattern Fill layer gets the full pattern into the document.
    t.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
    t.execute("layer.newFillLayer.pattern", json!({"pattern": "Big"})).unwrap();
    let doc_pat = &t.active().unwrap().doc.patterns[0];
    assert_eq!((doc_pat.width, doc_pat.height), (300, 200));
    // Exporting the library writes full pixels too.
    let out = dir.0.join("export.pat");
    t.execute("pattern.export", json!({"path": out.to_string_lossy()})).unwrap();
    let back = photocraft_io::pattern_map::read_pat(&std::fs::read(&out).unwrap()).unwrap();
    assert!(back.iter().any(|p| p.name == "Big" && p.width == 300));
    std::fs::remove_file(&out).unwrap();

    t.execute("pattern.delete", json!({"pattern": "Big"})).unwrap();
    assert!(!dir.files().iter().any(|f| f.starts_with("pattern-")), "{:?}", dir.files());
}

/// A corrupt pattern file is skipped with a warning and left on disk; the others load, and later
/// saves never delete or overwrite it. A corrupt index is never overwritten.
#[test]
fn corrupt_pattern_files_are_skipped_and_kept() {
    let dir = TempDir::new("patterns-corrupt");
    let (mut s, _) = session(&dir);
    define_pattern(&mut s, "Grit", "#202020", 8);
    define_pattern(&mut s, "Paper", "#f0e0c0", 12);
    let grit = s.patterns.items.iter().find(|p| p.name == "Grit").unwrap().id.clone();
    let file = dir.0.join(patterns::file_name_for_test(&grit));
    std::fs::write(&file, b"not a pattern").unwrap();

    let (mut t, w) = session(&dir);
    assert!(w.iter().any(|w| w.contains("Grit")), "{w:?}");
    assert_eq!(user_patterns(&t), [("Paper".to_string(), 12)]);
    define_pattern(&mut t, "Ink", "#000000", 4);
    assert_eq!(std::fs::read(&file).unwrap(), b"not a pattern", "the unreadable file is left alone");
    let index = std::fs::read_to_string(dir.0.join(PATTERNS_FILE)).unwrap();
    assert!(index.contains("Grit") && index.contains("Ink"), "{index}");

    std::fs::write(dir.0.join(PATTERNS_FILE), b"{").unwrap();
    let (mut u, w) = session(&dir);
    assert!(w.iter().any(|w| w.contains(PATTERNS_FILE)), "{w:?}");
    assert!(user_patterns(&u).is_empty());
    define_pattern(&mut u, "Late", "#808080", 4);
    assert_eq!(std::fs::read(dir.0.join(PATTERNS_FILE)).unwrap(), b"{", "an unreadable index is never overwritten");
}

/// Imported and defined patterns survive a restart: one file each, written once, plus the index;
/// a rename rewrites only the index, a delete removes the file.
#[test]
fn patterns_survive_a_restart() {
    let dir = TempDir::new("patterns");
    let (mut s, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    define_pattern(&mut s, "Grit", "#202020", 8);
    define_pattern(&mut s, "Paper", "#f0e0c0", 12);
    let pattern_files = |d: &TempDir| d.files().into_iter().filter(|f| f.starts_with("pattern-")).collect::<Vec<_>>();
    assert_eq!(pattern_files(&dir).len(), 2);
    assert!(dir.files().contains(&PATTERNS_FILE.to_string()));
    assert!(!dir.files().iter().any(|f| f.contains("Checkerboard")), "built-ins are never written");

    let (mut t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(user_patterns(&t), [("Grit".to_string(), 8), ("Paper".to_string(), 12)]);
    // The pixels come back too.
    let paper = t.patterns.items.iter().find(|p| p.name == "Paper").unwrap();
    let px = paper.surface.pixel(3, 3);
    assert!((px[0] - 240.0 / 255.0).abs() < 0.01, "{px:?}");

    let before = std::fs::metadata(dir.0.join(&pattern_files(&dir)[0])).unwrap().modified().unwrap();
    t.execute("pattern.rename", json!({"pattern": "Grit", "name": "Toner"})).unwrap();
    assert_eq!(std::fs::metadata(dir.0.join(&pattern_files(&dir)[0])).unwrap().modified().unwrap(), before, "a rename rewrites only the index");
    t.execute("pattern.delete", json!({"pattern": "Paper"})).unwrap();
    assert_eq!(pattern_files(&dir).len(), 1, "the deleted pattern's file is gone");

    let (u, _) = session(&dir);
    assert_eq!(user_patterns(&u), [("Toner".to_string(), 8)]);
}

fn define_pattern(s: &mut Session, name: &str, color: &str, width: u32) {
    s.execute("file.new", json!({"width": width, "height": 6})).unwrap();
    s.execute("edit.fill", json!({"color": color})).unwrap();
    s.execute("edit.definePattern", json!({"name": name})).unwrap();
}

fn user_patterns(s: &Session) -> Vec<(String, u32)> {
    let builtins: Vec<String> = crate::pattern_cmds::builtin().into_iter().map(|p| p.id).collect();
    s.patterns.items.iter().filter(|p| !builtins.contains(&p.id)).map(|p| (p.name.clone(), p.width)).collect()
}

/// #9317: a defined or imported pattern survives a restart, as in Photoshop; the built-in
/// patterns are never written, and deleting the last user pattern removes the file.
#[test]
fn user_patterns_survive_a_restart() {
    let dir = TempDir::new("patterns");
    let builtin = Session::new().patterns.items.len();
    let (mut s, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    s.execute("file.new", json!({"width": 12, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#1ea05a"})).unwrap();
    let id = s.execute("edit.definePattern", json!({"name": "Green Tile"})).unwrap()["pattern"].as_str().unwrap().to_string();
    s.sync_preset_store();
    assert!(dir.files().contains(&PATTERNS_FILE.to_string()), "{:?}", dir.files());
    // Only the user's pattern is in the file.
    let bytes = std::fs::read(dir.0.join(PATTERNS_FILE)).unwrap();
    let index: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let saved: Vec<photocraft_doc::Pattern> = index["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| photocraft_io::pattern_map::read_pat(&std::fs::read(dir.0.join(e["file"].as_str().unwrap())).unwrap()).unwrap().remove(0))
        .collect();
    assert_eq!(saved.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Green Tile"]);
    drop(s);

    let (mut t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(t.patterns.items.len(), builtin + 1);
    let back = t.patterns.items.iter().find(|p| p.id == id).expect("the defined pattern is back after a restart");
    assert_eq!((back.name.as_str(), back.width, back.height), ("Green Tile", 12, 10));
    assert_eq!(back.surface, saved[0].surface, "with its pixels");
    // Renames are kept; deleting the last user pattern removes the file.
    t.execute("pattern.rename", json!({"pattern": id, "name": "Moss"})).unwrap();
    t.sync_preset_store();
    let (u, _) = session(&dir);
    assert!(u.patterns.items.iter().any(|p| p.id == id && p.name == "Moss"));
    t.execute("pattern.delete", json!({"pattern": id})).unwrap();
    t.sync_preset_store();
    assert!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(dir.0.join(PATTERNS_FILE)).unwrap()).unwrap()["patterns"].as_array().unwrap().is_empty(),
        "empty authoritative index prevents resurrection"
    );
}

#[test]
fn a_patterns_file_that_failed_to_load_is_not_overwritten() {
    let mem = MemBackend::default();
    let unreadable = b"not a pattern file".to_vec();
    mem.files.lock().unwrap().insert(PATTERNS_FILE.into(), unreadable.clone());
    let mut s = Session::new();
    let w = s.attach_preset_store(open(Box::new(mem.clone())));
    assert!(w.iter().any(|w| w.contains(PATTERNS_FILE)), "{w:?}");
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    s.execute("edit.definePattern", json!({"name": "New"})).unwrap();
    s.sync_preset_store();
    assert_eq!(mem.files.lock().unwrap().get(PATTERNS_FILE), Some(&unreadable), "a patterns file the store could not read was overwritten");
    assert!(!s.preset_store.as_mut().unwrap().take_warnings().is_empty());
}

/// A `.pat` of one 4×4 pattern with id `id`, filled with `rgba`.
fn pat_bytes(id: &str, rgba: [f32; 4]) -> String {
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;
    let mut surface = Surface::new(photocraft_color::PixelFormat::RGBA8);
    surface.fill_rect(Rect::new(0, 0, 4, 4), &rgba);
    let mut p = photocraft_doc::Pattern::new("Same", surface, 4, 4);
    p.id = id.into();
    photocraft_paint::tile::b64_encode(&photocraft_io::pattern_map::write_pat(&[p]).unwrap())
}

/// #9318 review: re-importing a pattern under the same id, name and size but with other pixels
/// (an edited `.pat`) is written, so a restart doesn't bring the old pixels back.
#[test]
fn a_same_id_pattern_replacement_survives_a_restart() {
    let mem = MemBackend::default();
    let mut s = Session::new();
    s.attach_preset_store(open(Box::new(mem.clone())));
    let id = "6f1c2c1e-0000-4000-8000-000000009318";
    s.execute("pattern.import", json!({"data": pat_bytes(id, [1.0, 0.0, 0.0, 1.0])})).unwrap();
    s.sync_preset_store();
    s.execute("pattern.import", json!({"data": pat_bytes(id, [0.0, 0.0, 1.0, 1.0])})).unwrap();
    s.sync_preset_store();
    let mut t = Session::new();
    t.attach_preset_store(open(Box::new(mem.clone())));
    let back = t.patterns.items.iter().find(|p| p.id == id).expect("the pattern is back");
    let want = s.patterns.items.iter().find(|p| p.id == id).unwrap();
    assert_eq!(back.surface, want.surface, "with the replacement's pixels, not the first import's");
}

/// A [`MemBackend`] whose removals fail while `fail` is set.
#[derive(Clone, Default)]
struct FlakyRemove {
    mem: MemBackend,
    fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl PresetBackend for FlakyRemove {
    fn list(&self) -> Result<Vec<(String, u64)>, String> {
        self.mem.list()
    }
    fn read(&self, name: &str, max: u64) -> Result<Vec<u8>, String> {
        self.mem.read(name, max)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.mem.write(name, bytes)
    }
    fn remove(&self, name: &str) -> Result<(), String> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(format!("{name}: busy"));
        }
        self.mem.remove(name)
    }
}

/// #9318 review: deleting the last user pattern while `patterns.pat` can't be removed is
/// warned and retried, so the deleted pattern doesn't come back after a restart.
#[test]
fn a_failed_patterns_removal_is_retried() {
    let backend = FlakyRemove::default();
    let mut s = Session::new();
    s.attach_preset_store(open(Box::new(backend.clone())));
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    let id = s.execute("edit.definePattern", json!({"name": "Gone"})).unwrap()["pattern"].as_str().unwrap().to_string();
    s.sync_preset_store();
    assert!(backend.mem.files.lock().unwrap().contains_key(PATTERNS_FILE));
    backend.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    s.execute("pattern.delete", json!({"pattern": id})).unwrap();
    s.sync_preset_store();
    assert!(!s.preset_store.as_mut().unwrap().take_warnings().is_empty(), "the failure is warned");
    backend.fail.store(false, std::sync::atomic::Ordering::SeqCst);
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    s.sync_preset_store();
    assert!(!backend.mem.files.lock().unwrap().keys().any(|f| f.starts_with("pattern-")), "owned asset removal is retried");
    assert!(
        serde_json::from_slice::<serde_json::Value>(backend.mem.files.lock().unwrap().get(PATTERNS_FILE).unwrap()).unwrap()["patterns"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

/// #9318 review: an imported `.pat` that replaces a built-in pattern's pixels (same id) is saved
/// and replaces the built-in after a restart, once; untouched built-ins are still never written.
#[test]
fn an_imported_builtin_replacement_survives_a_restart() {
    let mem = MemBackend::default();
    let mut s = Session::new();
    s.attach_preset_store(open(Box::new(mem.clone())));
    let builtin = s.patterns.items[0].id.clone();
    s.execute("pattern.import", json!({"data": pat_bytes(&builtin, [0.9, 0.1, 0.4, 1.0])})).unwrap();
    s.sync_preset_store();
    let index: serde_json::Value = serde_json::from_slice(mem.files.lock().unwrap().get(PATTERNS_FILE).expect("written")).unwrap();
    let saved: Vec<photocraft_doc::Pattern> = index["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| photocraft_io::pattern_map::read_pat(mem.files.lock().unwrap().get(e["file"].as_str().unwrap()).unwrap()).unwrap().remove(0))
        .collect();
    assert_eq!(saved.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), [builtin.as_str()], "only the replaced built-in");
    let mut t = Session::new();
    t.attach_preset_store(open(Box::new(mem.clone())));
    let back: Vec<_> = t.patterns.items.iter().filter(|p| p.id == builtin).collect();
    assert_eq!(back.len(), 1, "once");
    let want = s.patterns.items.iter().find(|p| p.id == builtin).unwrap();
    assert_eq!(back[0].surface, want.surface, "with the imported pixels");
}

/// #9318 review: a pattern imported before the store finished loading, with the id, name and
/// size of a saved one but other pixels, wins the merge and is written, so a restart keeps it.
#[test]
fn an_early_same_id_import_wins_and_is_saved() {
    let mem = MemBackend::default();
    let id = "6f1c2c1e-0000-4000-8000-00000000e4a1";
    let mut s = Session::new();
    s.attach_preset_store(open(Box::new(mem.clone())));
    s.execute("pattern.import", json!({"data": pat_bytes(id, [1.0, 0.0, 0.0, 1.0])})).unwrap();
    s.sync_preset_store();
    // Next start: the same pattern with other pixels arrives before the store is attached.
    let mut t = Session::new();
    t.execute("pattern.import", json!({"data": pat_bytes(id, [0.0, 1.0, 0.0, 1.0])})).unwrap();
    let early = t.patterns.items.iter().find(|p| p.id == id).unwrap().surface.clone();
    t.attach_preset_store(open(Box::new(mem.clone())));
    t.sync_preset_store();
    let mut u = Session::new();
    u.attach_preset_store(open(Box::new(mem.clone())));
    assert_eq!(u.patterns.items.iter().find(|p| p.id == id).unwrap().surface, early, "the early import's pixels were saved");
}
#[derive(Clone)]
struct MigrationFail {
    mem: MemBackend,
    fail_name: String,
}
impl PresetBackend for MigrationFail {
    fn list(&self) -> Result<Vec<(String, u64)>, String> {
        self.mem.list()
    }
    fn read(&self, n: &str, m: u64) -> Result<Vec<u8>, String> {
        self.mem.read(n, m)
    }
    fn write(&self, n: &str, b: &[u8]) -> Result<(), String> {
        if n == self.fail_name { Err("injected migration write failure".into()) } else { self.mem.write(n, b) }
    }
    fn remove(&self, n: &str) -> Result<(), String> {
        self.mem.remove(n)
    }
}
fn legacy_store() -> (MemBackend, Vec<u8>, String) {
    let mem = MemBackend::default();
    let id = "6f1c2c1e-0000-4000-8000-000000005176".to_string();
    let bytes = photocraft_paint::tile::b64_decode(&pat_bytes(&id, [0.2, 0.6, 0.8, 1.0])).unwrap();
    mem.write(LEGACY_PATTERNS_FILE, &bytes).unwrap();
    (mem, bytes, id)
}
#[test]
fn legacy_migration_commits_last_and_never_resurrects_renames_or_deletions() {
    let (mem, old, id) = legacy_store();
    let mut s = Session::new();
    assert!(s.attach_preset_store(open(Box::new(mem.clone()))).is_empty());
    assert!(mem.read(PATTERNS_FILE, 1 << 20).is_ok());
    assert_eq!(mem.read(LEGACY_PATTERNS_FILE, MAX_PATTERNS_BYTES).unwrap(), old);
    s.execute("pattern.rename", json!({"pattern":id,"name":"Migrated"})).unwrap();
    s.sync_preset_store();
    let mut t = Session::new();
    t.attach_preset_store(open(Box::new(mem.clone())));
    assert_eq!(t.patterns.items.iter().find(|p| p.id == id).unwrap().name, "Migrated");
    t.execute("pattern.delete", json!({"pattern":id})).unwrap();
    t.sync_preset_store();
    for _ in 0..2 {
        let mut u = Session::new();
        u.attach_preset_store(open(Box::new(mem.clone())));
        assert!(!u.patterns.items.iter().any(|p| p.id == id));
    }
    assert_eq!(mem.read(LEGACY_PATTERNS_FILE, MAX_PATTERNS_BYTES).unwrap(), old);
}
#[test]
fn interrupted_migration_preserves_legacy_and_reuses_only_identical_orphans() {
    for fail_index in [false, true] {
        let (mem, old, id) = legacy_store();
        let file = patterns::file_name_for_test(&id);
        let fail_name = if fail_index { PATTERNS_FILE.to_string() } else { file.clone() };
        let opened = open(Box::new(MigrationFail { mem: mem.clone(), fail_name }));
        assert!(!opened.warnings.is_empty());
        assert!(opened.patterns.iter().any(|p| p.pattern.id == id), "legacy remains usable");
        assert!(mem.read(PATTERNS_FILE, 1 << 20).is_err(), "no commit marker on failure");
        assert_eq!(mem.read(LEGACY_PATTERNS_FILE, MAX_PATTERNS_BYTES).unwrap(), old);
        let retry = open(Box::new(mem.clone()));
        assert!(retry.warnings.is_empty(), "{:?}", retry.warnings);
        assert!(retry.patterns.iter().any(|p| p.pattern.id == id));
    }
    let (mem, old, id) = legacy_store();
    let file = patterns::file_name_for_test(&id);
    mem.write(&file, b"unknown content").unwrap();
    let opened = open(Box::new(mem.clone()));
    assert!(!opened.warnings.is_empty());
    assert_eq!(mem.read(&file, 100).unwrap(), b"unknown content");
    assert!(mem.read(PATTERNS_FILE, 1 << 20).is_err());
    assert_eq!(mem.read(LEGACY_PATTERNS_FILE, MAX_PATTERNS_BYTES).unwrap(), old);
}
#[test]
fn malformed_authoritative_index_never_falls_back_to_stale_legacy() {
    let (mem, _, id) = legacy_store();
    drop(open(Box::new(mem.clone())));
    for bytes in [b"{".as_slice(), b"{\"version\":1,\"patterns\":[{\"id\":\"x\",\"name\":\"bad\",\"file\":\"../secret\"}]}".as_slice()] {
        mem.write(PATTERNS_FILE, bytes).unwrap();
        let opened = open(Box::new(mem.clone()));
        assert!(!opened.warnings.is_empty());
        assert!(opened.patterns.is_empty());
        assert_eq!(mem.read(PATTERNS_FILE, 1 << 20).unwrap(), bytes);
        assert!(!opened.patterns.iter().any(|p| p.pattern.id == id));
    }
    let mem = MemBackend::default();
    mem.write(LEGACY_PATTERNS_FILE, b"broken PAT").unwrap();
    let opened = open(Box::new(mem.clone()));
    assert!(!opened.warnings.is_empty());
    assert!(mem.read(PATTERNS_FILE, 1 << 20).is_err());
    assert_eq!(mem.read(LEGACY_PATTERNS_FILE, 20).unwrap(), b"broken PAT");
}
#[test]
fn migration_preserves_duplicate_names_but_refuses_duplicate_ids() {
    let (mem, _, id) = legacy_store();
    let bytes = mem.read(LEGACY_PATTERNS_FILE, MAX_PATTERNS_BYTES).unwrap();
    let mut pats = photocraft_io::pattern_map::read_pat(&bytes).unwrap();
    let mut other = pats[0].clone();
    other.id = "6f1c2c1e-0000-4000-8000-000000005177".into();
    pats.push(other);
    mem.write(LEGACY_PATTERNS_FILE, &photocraft_io::pattern_map::write_pat(&pats).unwrap()).unwrap();
    let opened = open(Box::new(mem.clone()));
    assert!(opened.warnings.is_empty());
    assert_eq!(opened.patterns.len(), 2);
    let mem = MemBackend::default();
    pats[1].id = id;
    let old = photocraft_io::pattern_map::write_pat(&pats).unwrap();
    mem.write(LEGACY_PATTERNS_FILE, &old).unwrap();
    let opened = open(Box::new(mem.clone()));
    assert!(!opened.warnings.is_empty());
    assert!(mem.read(PATTERNS_FILE, 1 << 20).is_err());
    assert_eq!(mem.read(LEGACY_PATTERNS_FILE, MAX_PATTERNS_BYTES).unwrap(), old);
}
