//! The pattern library in the preset store: imported (`.pat`) and defined patterns survive a
//! restart, as Photoshop keeps its Patterns panel.
//!
//! - `pattern-<hash>.pat`: one pattern per file (a one-pattern Photoshop `.pat`), written once
//!   when the pattern enters the library and deleted when it leaves it;
//! - `pattern-<hash>.preview.pat`: the same pattern shrunk to at most [`PREVIEW_SIDE`] px, for
//!   patterns larger than that;
//! - `patterns.json`: the library order, names and full sizes (renaming rewrites only this file).
//!
//! Opening the store reads the index and the previews, never a large pattern's pixels: texture
//! libraries run to gigabytes (Debaser's colour libraries hold twenty 4500 × 4500 px patterns per
//! file). Such a pattern joins the library as its preview, with its full size in
//! [`crate::pattern_cmds::PatternLibrary::stored`], and its pixels are read when it is used
//! ([`super::PresetStore::load_pattern`]).
//!
//! Built-in patterns are never written. A pattern file or index that fails to load is left on
//! disk untouched and never overwritten or deleted; a missing index is an empty library.

use std::collections::HashSet;

use photocraft_doc::Pattern;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};

use super::{PresetBackend, missing_file};
use crate::pattern_cmds::PatternLibrary;

/// Library order and names.
pub const PATTERNS_FILE: &str = "patterns.json";
/// Largest `patterns.json` read or written.
pub const MAX_PATTERN_INDEX_BYTES: u64 = 4 << 20;
/// Largest pattern file read or written.
pub const MAX_PATTERN_BYTES: u64 = 256 << 20;
/// Most patterns loaded.
pub const MAX_PATTERNS: usize = 10_000;
/// Patterns larger than this (either side) are loaded as a preview of this size until used.
pub const PREVIEW_SIDE: u32 = 128;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub(super) struct Entry {
    pub id: String,
    pub name: String,
    pub file: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    patterns: Vec<Entry>,
}

/// The store's view of the pattern files it owns.
#[derive(Default)]
pub(super) struct PatternFiles {
    /// The index as last loaded or written.
    pub index: Vec<Entry>,
    /// `patterns.json` exists but failed to load: never written.
    pub index_skipped: bool,
    /// Pattern files that failed to load: never written or deleted.
    pub skipped: HashSet<String>,
    /// Patterns that couldn't be written (too large, I/O error): not retried this session, so
    /// one failure is one warning.
    pub failed: HashSet<String>,
    pub pixels_rev: u64,
    pub pending_removals: HashSet<String>,
}

/// A stored pattern as loaded: the pattern (a preview when `full` is set) and its full size.
pub struct Loaded {
    pub pattern: Pattern,
    pub full: Option<(u32, u32)>,
}

fn hash(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// The file a pattern is stored in (its id hashed: ids are UUIDs or content hashes, but may hold
/// any character).
fn file_name(id: &str) -> String {
    format!("pattern-{:016x}.pat", hash(id))
}

fn preview_name(id: &str) -> String {
    format!("pattern-{:016x}.preview.pat", hash(id))
}

#[cfg(test)]
pub(super) fn file_name_for_test(id: &str) -> String {
    file_name(id)
}

fn is_large(width: u32, height: u32) -> bool {
    width.max(height) > PREVIEW_SIDE
}

/// `p` shrunk to at most [`PREVIEW_SIDE`] px (nearest sample: a swatch, not a resample).
fn preview(p: &Pattern) -> Pattern {
    let (w, h) = (p.width.max(1), p.height.max(1));
    let step = f64::from(w.max(h)) / f64::from(PREVIEW_SIDE);
    let pw = ((f64::from(w) / step).round() as u32).clamp(1, PREVIEW_SIDE);
    let ph = ((f64::from(h) / step).round() as u32).clamp(1, PREVIEW_SIDE);
    let fmt = p.surface.format();
    let mut data = Vec::with_capacity(pw as usize * ph as usize * fmt.channels());
    for y in 0..ph {
        for x in 0..pw {
            let sx = ((f64::from(x) * step) as u32).min(w - 1);
            let sy = ((f64::from(y) * step) as u32).min(h - 1);
            data.extend(p.surface.pixel(sx as i32, sy as i32));
        }
    }
    let mut surface = Surface::new(fmt);
    surface.write_region(Rect::new(0, 0, pw as i32, ph as i32), &data);
    Pattern { id: p.id.clone(), name: p.name.clone(), width: pw, height: ph, surface }
}

fn read_one(backend: &dyn PresetBackend, file: &str) -> Result<Pattern, String> {
    let mut v = photocraft_io::pattern_map::read_pat(&backend.read(file, MAX_PATTERN_BYTES)?)?;
    if v.is_empty() { Err("no pattern in the file".to_string()) } else { Ok(v.swap_remove(0)) }
}

/// A stored pattern's full pixels (named and identified as in `entry`).
pub(super) fn load_full(backend: &dyn PresetBackend, entry: &Entry) -> Result<Pattern, String> {
    let mut p = read_one(backend, &entry.file)?;
    p.id = entry.id.clone();
    p.name = entry.name.clone();
    Ok(p)
}

/// Load the stored patterns, in library order: small ones whole, large ones as previews.
pub(super) fn load(backend: &dyn PresetBackend, warnings: &mut Vec<String>) -> (Vec<Loaded>, PatternFiles) {
    let mut files = PatternFiles::default();
    let bytes = match backend.read(PATTERNS_FILE, MAX_PATTERN_INDEX_BYTES) {
        Ok(b) => b,
        Err(e) if missing_file(&e) => return migrate_legacy(backend, warnings),
        Err(e) => {
            warnings.push(format!("patterns: {PATTERNS_FILE} skipped: {e}"));
            files.index_skipped = true;
            return (Vec::new(), files);
        }
    };
    let index = match serde_json::from_slice::<IndexFile>(&bytes) {
        Ok(f) if f.version == 1 => f.patterns,
        Ok(f) => {
            warnings.push(format!("patterns: {PATTERNS_FILE} skipped: unsupported version {}", f.version));
            files.index_skipped = true;
            return (Vec::new(), files);
        }
        Err(e) => {
            warnings.push(format!("patterns: {PATTERNS_FILE} skipped: {e}"));
            files.index_skipped = true;
            return (Vec::new(), files);
        }
    };
    let mut ids = HashSet::new();
    let valid = index.len() <= MAX_PATTERNS && index.iter().all(|e| e.file == file_name(&e.id) && ids.insert(e.id.clone()));
    if !valid {
        warnings
            .push("patterns: invalid/duplicate index entries; preserve files and recover patterns.json before saving (legacy PAT is not reimported)".into());
        files.index_skipped = true;
        return (Vec::new(), files);
    }
    let mut patterns = Vec::new();
    let mut seen = HashSet::new();
    for e in index.into_iter().take(MAX_PATTERNS) {
        // Only generated names: an edited index can't reach other files.
        if e.file != file_name(&e.id) || !seen.insert(e.id.clone()) {
            warnings.push(format!("patterns: entry \"{}\" skipped: bad file name", e.name));
            continue;
        }
        let loaded = if is_large(e.width, e.height) {
            // The preview; without one (a failed write), the full file once to make it.
            match read_one(backend, &preview_name(&e.id)) {
                Ok(mut p) => {
                    p.id = e.id.clone();
                    p.name = e.name.clone();
                    Ok(Loaded { pattern: p, full: Some((e.width, e.height)) })
                }
                Err(_) => load_full(backend, &e).map(|p| Loaded { pattern: preview(&p), full: Some((p.width, p.height)) }),
            }
        } else {
            load_full(backend, &e).map(|p| Loaded { pattern: p, full: None })
        };
        match loaded {
            Ok(l) => {
                patterns.push(l);
                files.index.push(e);
            }
            Err(err) => {
                warnings.push(format!("patterns: {} (\"{}\") skipped: {err}", e.file, e.name));
                files.skipped.insert(e.file.clone());
                // Kept in the index, so a later write doesn't forget the file.
                files.index.push(e);
            }
        }
    }
    (patterns, files)
}

/// Write what changed: new patterns' files (and previews), the index when the order or a name
/// changed, and delete the files of patterns that left the library. Failures are warnings.
pub(super) fn sync(backend: &dyn PresetBackend, files: &mut PatternFiles, library: &PatternLibrary, warnings: &mut Vec<String>) {
    files.pending_removals.retain(|f| match backend.remove(f) {
        Ok(()) => false,
        Err(e) => {
            warnings.push(format!("Couldn't delete pattern file {f}: {e}"));
            true
        }
    });
    let builtins = crate::pattern_cmds::builtin();
    let pixels_changed = files.pixels_rev != library.pixels_rev;
    let mut want: Vec<Entry> = library
        .items
        .iter()
        .filter(|p| !crate::pattern_cmds::is_pristine_builtin(p, &builtins))
        .map(|p| {
            let (width, height) = library.stored.get(&p.id).copied().unwrap_or((p.width, p.height));
            Entry { id: p.id.clone(), name: p.name.clone(), file: file_name(&p.id), width, height }
        })
        .collect();
    // Unreadable files stay listed (at the end) until the library takes their pattern back.
    for e in &files.index {
        if files.skipped.contains(&e.file) && !want.iter().any(|w| w.id == e.id) {
            want.push(e.clone());
        }
    }
    if want == files.index && !pixels_changed {
        return;
    }
    if files.index_skipped {
        warnings.push(format!("Couldn't save patterns: {PATTERNS_FILE} could not be read, so it was left unchanged"));
        files.index = want;
        return;
    }
    let have: HashSet<&str> = files.index.iter().map(|e| e.id.as_str()).collect();
    let mut written = Vec::with_capacity(want.len());
    for e in want {
        // Already on disk (a preview in the library included: its pixels never left the disk).
        if (have.contains(e.id.as_str()) && !pixels_changed) || files.skipped.contains(&e.file) || library.stored.contains_key(&e.id) {
            written.push(e);
            continue;
        }
        let Some(p) = library.items.iter().find(|p| p.id == e.id) else { continue };
        match write_pattern(backend, p, &e, have.contains(e.id.as_str())) {
            Ok(()) => written.push(e),
            Err(err) => {
                warnings.push(err);
                files.failed.insert(e.id);
                return;
            }
        }
    }
    if written.len() > MAX_PATTERNS {
        warnings.push("patterns: index count limit exceeded".into());
        return;
    }
    let index = IndexFile { version: 1, patterns: written };
    let bytes = match serde_json::to_vec_pretty(&index) {
        Ok(b) => b,
        Err(e) => {
            warnings.push(format!("Couldn't save patterns: {e}"));
            return;
        }
    };
    if bytes.len() as u64 > MAX_PATTERN_INDEX_BYTES {
        warnings.push("patterns: index byte limit exceeded".into());
        return;
    }
    if let Err(e) = backend.write(PATTERNS_FILE, &bytes) {
        warnings.push(format!("Couldn't save patterns: {e}"));
        return;
    }
    // Files of patterns that left the library (written by an earlier index) are deleted only
    // once the new index no longer names them.
    let keep: HashSet<&str> = index.patterns.iter().map(|e| e.file.as_str()).collect();
    for e in &files.index {
        if !keep.contains(e.file.as_str()) && !files.skipped.contains(&e.file) {
            for f in [e.file.clone(), preview_name(&e.id)] {
                if let Err(err) = backend.remove(&f) {
                    warnings.push(format!("Couldn't delete pattern file {f}: {err}"));
                    files.pending_removals.insert(f);
                }
            }
        }
    }
    files.index = index.patterns;
    files.pixels_rev = library.pixels_rev;
}

fn write_pattern(backend: &dyn PresetBackend, p: &Pattern, e: &Entry, owned: bool) -> Result<(), String> {
    let bytes = photocraft_io::pattern_map::write_pat(std::slice::from_ref(p)).map_err(|err| format!("Couldn't save pattern \"{}\": {err}", p.name))?;
    if bytes.len() as u64 > MAX_PATTERN_BYTES {
        return Err(format!("pattern \"{}\" is too large to save ({} MB limit)", p.name, MAX_PATTERN_BYTES >> 20));
    }
    if is_large(p.width, p.height) {
        // The preview first: a full file without one still loads (by making it).
        let small = photocraft_io::pattern_map::write_pat(&[preview(p)]).map_err(|err| format!("Couldn't save pattern \"{}\": {err}", p.name))?;
        write_asset(backend, &preview_name(&e.id), &small, owned).map_err(|err| format!("Couldn't save pattern \"{}\": {err}", p.name))?;
    }
    write_asset(backend, &e.file, &bytes, owned).map_err(|err| format!("Couldn't save pattern \"{}\": {err}", p.name))
}

/// The index itself is the atomic migration commit. Never consult stale PAT when it exists,
/// including when corrupt. An interrupted asset write leaves the legacy bytes authoritative.
fn migrate_legacy(backend: &dyn PresetBackend, warnings: &mut Vec<String>) -> (Vec<Loaded>, PatternFiles) {
    let mut files = PatternFiles::default();
    let bytes = match backend.read(super::LEGACY_PATTERNS_FILE, super::MAX_PATTERNS_BYTES) {
        Ok(b) => b,
        Err(e) if missing_file(&e) => return (Vec::new(), files),
        Err(e) => {
            warnings.push(format!("patterns: legacy file preserved: {e}"));
            files.index_skipped = true;
            return (Vec::new(), files);
        }
    };
    let patterns = match photocraft_io::pattern_map::read_pat(&bytes) {
        Ok(p) => p,
        Err(e) => {
            warnings.push(format!("patterns: legacy file preserved: {e}"));
            files.index_skipped = true;
            return (Vec::new(), files);
        }
    };
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    if patterns.len() > MAX_PATTERNS || patterns.iter().any(|p| !ids.insert(p.id.clone()) || !names.insert(file_name(&p.id))) {
        warnings.push("patterns: legacy migration refused duplicate IDs, file hash collision or count limit; legacy preserved".into());
        files.index_skipped = true;
        return (patterns.into_iter().map(|pattern| Loaded { pattern, full: None }).collect(), files);
    }
    let migration = (|| -> Result<(), String> {
        let entries: Vec<Entry> =
            patterns.iter().map(|p| Entry { id: p.id.clone(), name: p.name.clone(), file: file_name(&p.id), width: p.width, height: p.height }).collect();
        let index = serde_json::to_vec_pretty(&IndexFile { version: 1, patterns: entries.clone() }).map_err(|e| e.to_string())?;
        if index.len() as u64 > MAX_PATTERN_INDEX_BYTES {
            return Err("index exceeds size limit".into());
        }
        for (p, e) in patterns.iter().zip(&entries) {
            // Never overwrite unknown files, even if a name collides. Identical orphan assets
            // from an interrupted migration can be reused; the legacy file stays untouched.
            let full = photocraft_io::pattern_map::write_pat(std::slice::from_ref(p))?;
            if full.len() as u64 > MAX_PATTERN_BYTES {
                return Err("pattern exceeds size limit".into());
            }
            write_migration_asset(backend, &e.file, &full)?;
            if is_large(p.width, p.height) {
                let small = photocraft_io::pattern_map::write_pat(&[preview(p)])?;
                write_migration_asset(backend, &preview_name(&p.id), &small)?;
            }
        }
        // Atomic backend write last: no partial or failed migration can become authoritative.
        backend.write(PATTERNS_FILE, &index)?;
        Ok(())
    })();
    match migration {
        Ok(()) => load(backend, warnings),
        Err(e) => {
            warnings.push(format!("patterns: migration not committed, legacy remains usable: {e}"));
            files.index_skipped = true;
            (patterns.into_iter().map(|pattern| Loaded { pattern, full: None }).collect(), files)
        }
    }
}
fn write_migration_asset(backend: &dyn PresetBackend, name: &str, bytes: &[u8]) -> Result<(), String> {
    match backend.read(name, MAX_PATTERN_BYTES) {
        Ok(existing) if existing == bytes => Ok(()),
        Ok(_) => Err(format!("unrecognized existing asset {name} preserved")),
        Err(e) if missing_file(&e) => backend.write(name, bytes),
        Err(e) => Err(format!("existing asset {name} preserved: {e}")),
    }
}

fn write_asset(backend: &dyn PresetBackend, name: &str, bytes: &[u8], owned: bool) -> Result<(), String> {
    if owned { backend.write(name, bytes) } else { write_migration_asset(backend, name, bytes) }
}
