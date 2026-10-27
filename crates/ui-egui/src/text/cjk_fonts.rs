//! Lazy CJK and other-script fallback fonts for the UI.
//!
//! The bundled Inter / JetBrains Mono have no Japanese, Chinese or Korean glyphs, and the OS
//! fonts that do are large (Hiragino ~10 MB, Apple SD Gothic Neo 28 MB, PingFang 78 MB, Noto
//! Sans CJK ~20 MB). Instead of reading them at startup, an egui plugin scans each frame's text
//! for CJK characters no registered font covers, and registers the next system font for that
//! character's script (`ctx.add_font`, active from the next frame, which it requests). Fonts are
//! appended at the lowest priority to every family, so Latin text keeps Inter.
//!
//! Other scripts the bundled fonts lack (Thai, Arabic, Hebrew, Devanagari and the other Indic
//! scripts, ...: [`photocraft_text::scripts`]) work the same way, each with its own attempt state,
//! independent of the CJK order: the first installed candidate file whose character map really
//! has the missing character is registered, once per script. egui shapes each font run with
//! HarfRust, so Thai marks and Arabic joining come from the font; it has no bidi reordering yet.
//!
//! Script order follows the effective UI locale ([`photocraft_text::cjk::script_order`]): Kana
//! prefers a Japanese font, Hangul a Korean one, Bopomofo a Traditional Chinese one, and Han the
//! locale's script. The effective locale is the translated UI language when it names
//! ja/ko/zh-Hans/zh-Hant; otherwise it is the OS locale. An English UI on a Japanese, Korean or
//! Chinese OS therefore follows that OS script, not the English baseline. The craft manifest
//! lists Shippori Mincho and BIZ UDPGothic before Noto Sans CJK SC, so a non-CJK/unknown
//! effective locale keeps the migration baseline's first covering face for shared Han and kana
//! (Shippori Mincho Regular); Hangul intentionally uses the Korean face (NanumGothic) instead,
//! because [`CjkChar::preferred`] always prefers Korean, even though the old linear stack would
//! have picked the broad Simplified Chinese face. An explicit zh-Hans/zh-Hant/ja/ko preference
//! selects its own form and keeps the UI-first face for its script (BIZ UDPGothic for Japanese).
//! The lazy loader reads at most one font per frame, each file at most once, and once every
//! script has been tried the scan stops.
//!
//! Builds made with the optional craft-fonts input (`CRAFT_FONTS_DIR`,
//! [`photocraft_text::craft_fonts`]) carry Japanese, Chinese, Korean and other faces. A
//! per-character [`FontProvider`] serves them, and the system font a locale defers to, in the
//! first frame; the lazy loader remains the later fallback. The provider has no lifetime read
//! cap. It retains at most [`MAX_SYSTEM_CACHE_BYTES`] of unique font-file blobs, charging a
//! collection (TTC) once however many faces it selects; a face that cannot be retained is
//! returned transiently for the current request, so a later Fontconfig candidate is never
//! skipped merely because earlier characters read other fonts.
//! Without craft-fonts (and on the web, which never embeds them) nothing changes.

use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{FallbackRequest, FontData, FontFamily, FontId, FontProvider, Shape};
use photocraft_text::cjk::{self, CjkChar, CjkScript, FontFile};
use photocraft_text::craft_fonts::{self, CraftFont};
use photocraft_text::scripts::{self, Script};
use std::path::PathBuf;

/// Skip absurdly large files (a corrupt or non-font path must not eat memory).
pub const MAX_FONT_BYTES: u64 = 128 << 20;
/// At most this many candidate files are read for one non-CJK script by the lazy loader.
const MAX_SCRIPT_READS: usize = 6;
/// Upper bound on the unique font-file blobs the eager provider retains in its own cache.
/// Structurally invalid candidates are not retained at all. A readable file is charged once,
/// whatever number of faces the provider selects from a collection; it is evicted before the
/// budget is exceeded. A file that cannot be retained is still used for the current request and
/// returned transiently to egui (which may own that installed face independently). There is
/// deliberately no lifetime read cap: a valid Fontconfig match appended after many
/// absent/non-covering static paths (or after earlier characters) must still be reached.
const MAX_SYSTEM_CACHE_BYTES: usize = 128 << 20;
/// Remembered face names are tiny but still bounded independently of the blob budget.
const MAX_SYSTEM_FACE_NAMES: usize = 1024;
/// Name prefix of the registered fallback fonts.
pub const FONT_PREFIX: &str = "system-cjk";

/// Where fonts come from; swapped out in tests.
#[derive(Clone)]
pub struct Sources {
    pub locale: fn() -> Option<String>,
    pub files: fn(CjkScript) -> Vec<FontFile>,
    pub last_resort: fn() -> Vec<FontFile>,
    /// Whether the background Fontconfig CJK lookup is still running. The provider must not fall
    /// through to a lower-priority script (for example Simplified Chinese for a Hant character)
    /// while the preferred script may still gain a system face. Injectable so tests (and the web
    /// build) never depend on timing.
    pub pending: fn() -> bool,
    /// Candidate files for a non-CJK script, most preferred first; read only when a visible
    /// character of that script is missing.
    pub scripts: fn(Script) -> Vec<PathBuf>,
    /// The face of a font file whose character map has the character (`None`: not this file).
    pub covers: fn(&[u8], char) -> Option<u32>,
    /// Embedded craft faces declaring an ISO 15924 script, in the script's own order. The
    /// provider and lazy loader both use this injection (tests can therefore pin craft-font
    /// selection without depending on the build input).
    pub embedded: fn(&'static str) -> Vec<&'static CraftFont>,
    /// Install the per-character craft-font provider, so a headless first frame can shape CJK
    /// and other scripts before the lazy loader gets a second pass (tests panic on a missing
    /// glyph). It selects a face for the character's script instead of putting a broad face into
    /// the family stack up front for every later character.
    pub eager_craft_fonts: bool,
}

/// Craft fonts declaring `iso`, Regular first and (for Japanese) the UI face first; empty
/// without craft-fonts. This is the explicit-locale order; a non-CJK/unknown effective locale
/// uses the raw manifest order through [`craft_face_manifest`] instead.
///
/// The lazy loader tries these before system files for a script, so its fallback order stays
/// `preferred script, then the locale order` even for a character the provider left to it.
pub fn craft_embedded_iso(iso: &'static str) -> Vec<&'static CraftFont> {
    craft_faces(iso).into_iter().map(|(_, f)| f).collect()
}

/// Craft fonts for a CJK script; a convenience wrapper for callers that already hold the enum.
pub fn craft_embedded(script: CjkScript) -> Vec<&'static CraftFont> {
    craft_embedded_iso(cjk_iso(script))
}

/// No embedded fonts (tests that pin the system-font behaviour).
pub fn no_embedded(_: &'static str) -> Vec<&'static CraftFont> {
    Vec::new()
}

/// Craft fonts chosen per character, the way the lazy loader chooses them for one missing glyph.
///
/// A single linear font stack cannot express the per-script preference: a broad Noto Sans CJK SC
/// face covers kana and Hangul too, so putting it in the stack would make it win there over the
/// Japanese or Korean face. A [`FontProvider`] is asked per missing character instead, and this
/// one returns the face for that character's script. When the preferred script has no craft face
/// (e.g. Traditional Chinese), it asks the system font for that script first, and only falls
/// through the locale order (so a Simplified Chinese face comes last) when the system has none.
///
/// egui still resolves a character from an already discovered broad face before asking a
/// provider again; a mixed-script run whose first character discovered SC can therefore serve a
/// later kana/Hangul character from SC. Removing that last ambiguity needs script-aware fallback
/// in epaint (or per-script text runs); this provider fixes the eager-stack case.
struct CraftFontProvider {
    locale: fn() -> Option<String>,
    files: fn(CjkScript) -> Vec<FontFile>,
    last_resort: fn() -> Vec<FontFile>,
    pending: fn() -> bool,
    scripts: fn(Script) -> Vec<PathBuf>,
    covers: fn(&[u8], char) -> Option<u32>,
    embedded: fn(&'static str) -> Vec<&'static CraftFont>,
    /// Interface > UI Font Size multiplier for the discovered faces (see `theme::ui_font_scale`).
    scale: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// System files, faces and failed lookups read for characters no craft face covers.
    system: std::sync::Mutex<SystemCache>,
    /// I/O for a candidate path; `read_font` in production, injectable by tests.
    reader: fn(&std::path::Path) -> Option<Vec<u8>>,
}

/// A readable, structurally valid font file retained as one shared blob, no matter how many
/// faces of it the provider selects. The blob is charged to the byte budget once.
struct CachedFile {
    path: PathBuf,
    data: FontData,
    bytes: usize,
}

struct SystemCache {
    /// Unique retained font blobs, oldest first; evicted when the byte budget is exceeded.
    /// A collection's faces share this one entry, so a TTC is not charged per face.
    files: Vec<CachedFile>,
    /// Paths whose read failed (missing, empty, oversized, unreadable): never retry them for
    /// each missing character.
    failed_reads: std::collections::HashSet<PathBuf>,
    /// Paths that were readable but structurally not a font (truncated download, unrelated
    /// file): negative-cached separately from an I/O failure and from a non-covering face.
    invalid_fonts: std::collections::HashSet<PathBuf>,
    /// Names of faces handed to egui, reusing their name across characters. These carry no font
    /// bytes (the blob stays in `files` while retained) and are capped by
    /// [`MAX_SYSTEM_FACE_NAMES`].
    faces: Vec<SystemFace>,
    /// `face_index_for_family_checked` results, including family misses, so a collection's
    /// `name`/directory tables are not parsed again for every character.
    family_indices: Vec<(PathBuf, String, Option<u32>)>,
    /// Bytes of the unique blobs in `files`. A selected face adds no bytes: egui's installed copy
    /// may outlive the provider's reference, but the provider charges and bounds only its own
    /// retained blob.
    retained_bytes: usize,
    /// Budget for `retained_bytes`; injectable (small) in tests.
    max_bytes: usize,
    /// Next face name; monotonic so names are never reused for a different face.
    next_face_name: usize,
    /// Successful I/O reads. Diagnostics/tests only: there is deliberately no lifetime cap.
    #[cfg_attr(not(test), allow(dead_code))]
    reads: usize,
}

impl Default for SystemCache {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            failed_reads: std::collections::HashSet::new(),
            invalid_fonts: std::collections::HashSet::new(),
            faces: Vec::new(),
            family_indices: Vec::new(),
            retained_bytes: 0,
            max_bytes: MAX_SYSTEM_CACHE_BYTES,
            next_face_name: 0,
            reads: 0,
        }
    }
}

impl SystemCache {
    /// Evict the oldest blobs until `extra` more bytes fit. Returns `false` when even an empty
    /// cache cannot hold `extra`; in that case nothing is evicted, because a request that must be
    /// transient should not discard useful cached candidates. Face names are not pinned blobs:
    /// egui may keep an installed copy, but the provider's own byte cache stays bounded.
    fn make_room(&mut self, extra: usize) -> bool {
        if extra > self.max_bytes {
            return false;
        }
        while self.retained_bytes.saturating_add(extra) > self.max_bytes {
            let Some(file) = self.files.first() else { return false };
            let bytes = file.bytes;
            self.files.remove(0);
            self.retained_bytes = self.retained_bytes.saturating_sub(bytes);
        }
        true
    }

    /// Remember a face name without retaining font bytes. Names are capped; forgetting an old
    /// name only means egui may install a face under a new name later.
    fn remember_face(&mut self, path: PathBuf, index: u32, name: String) {
        if self.faces.len() >= MAX_SYSTEM_FACE_NAMES {
            self.faces.remove(0);
        }
        self.faces.push(SystemFace { path, index, name });
    }
}

struct SystemFace {
    path: PathBuf,
    index: u32,
    name: String,
}

impl CraftFontProvider {
    fn new(sources: &Sources, scale: f32) -> Self {
        Self {
            locale: sources.locale,
            files: sources.files,
            last_resort: sources.last_resort,
            pending: sources.pending,
            scripts: sources.scripts,
            covers: sources.covers,
            embedded: sources.embedded,
            scale: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(scale.to_bits())),
            system: std::sync::Mutex::new(SystemCache::default()),
            reader: read_font,
        }
    }

    fn scale(&self) -> f32 {
        f32::from_bits(self.scale.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn set_scale(&self, scale: f32) {
        self.scale.store(scale.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    /// The bytes of `path`, read at most once while its blob is retained and shared even when a
    /// later request wants a different face of the same collection.
    ///
    /// The whole check/read/insert runs under the cache lock, so concurrent layout threads share
    /// one read per path and a success can never be shadowed by another thread's stale failure.
    /// Missing/unreadable paths and readable non-font bytes are negative-cached separately.
    fn system_source(&self, path: &std::path::Path) -> Option<FontData> {
        let mut cache = self.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.failed_reads.contains(path) || cache.invalid_fonts.contains(path) {
            return None;
        }
        if let Some(file) = cache.files.iter().find(|file| file.path.as_path() == path) {
            return Some(FontData::from_blob(file.data.font.clone(), 0));
        }
        let Some(bytes) = (self.reader)(path) else {
            cache.failed_reads.insert(path.to_path_buf());
            return None;
        };
        cache.reads = cache.reads.saturating_add(1);
        if !cjk::font_file_is_readable(&bytes) {
            // A readable non-font is an invalid candidate, not a valid face that merely lacks
            // this character: remember that separately and never retain its bytes.
            cache.invalid_fonts.insert(path.to_path_buf());
            return None;
        }
        let size = bytes.len();
        let data = FontData::from_owned(bytes);
        let source = FontData::from_blob(data.font.clone(), 0);
        if cache.make_room(size) {
            cache.retained_bytes = cache.retained_bytes.saturating_add(size);
            cache.files.push(CachedFile { path: path.to_path_buf(), data, bytes: size });
        }
        // If this file alone exceeds the budget, the bytes are still used for this request but
        // not retained. egui may keep the installed face; the provider's own cache stays bounded.
        Some(source)
    }

    /// The (cached) face index whose family name is `family`; `None` is cached as a family miss.
    fn checked_face_index(&self, path: &std::path::Path, family: &str, bytes: &[u8]) -> Option<u32> {
        {
            let cache = self.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((_, _, index)) = cache.family_indices.iter().find(|(cached, name, _)| cached.as_path() == path && name.as_str() == family) {
                return *index;
            }
        }
        let index = cjk::face_index_for_family_checked(bytes, family);
        let mut cache = self.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !cache.family_indices.iter().any(|(cached, name, _)| cached.as_path() == path && name.as_str() == family) {
            cache.family_indices.push((path.to_path_buf(), family.to_owned(), index));
        }
        index
    }

    /// Register one face of an already-read system file, reusing its name but applying the
    /// current UI scale on every return (a cache must never bake the scale in).
    ///
    /// A face never adds a second charge for its collection's blob. If the blob was not retained
    /// (it alone exceeds the budget, or another thread evicted it first), this still returns a
    /// complete transient `FontInsert`: egui installs and owns that copy, while the provider's
    /// own cache remains within `max_bytes`.
    fn system_insert_face(&self, path: PathBuf, index: u32, source: FontData) -> FontInsert {
        let mut cache = self.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let name = match cache.faces.iter().find(|f| f.path == path && f.index == index) {
            Some(face) => face.name.clone(),
            None => {
                let name = format!("craft-system-{}", cache.next_face_name);
                cache.next_face_name = cache.next_face_name.saturating_add(1);
                cache.remember_face(path, index, name.clone());
                name
            }
        };
        let mut data = FontData::from_blob(source.font, index);
        apply_font_tweaks(&mut data);
        font_insert(name, data, self.scale())
    }

    /// The first system face for `script` (or any last-resort face) whose requested family really
    /// covers `c`.
    ///
    /// The lazy loader reads these after the first frame; the provider needs the same source for
    /// the first frame, before a headless layout with `MissingGlyphPolicy::Panic` can finish.
    /// A non-empty family is resolved by name first, then checked against that exact face: never
    /// use the first face of a collection that happens to cover `c`.
    fn system_insert_files(&self, files: Vec<FontFile>, c: char) -> Option<FontInsert> {
        for file in files {
            let Some(source) = self.system_source(&file.path) else { continue };
            let index = if file.family.is_empty() {
                0
            } else {
                let Some(index) = self.checked_face_index(&file.path, &file.family, source.bytes()) else { continue };
                index
            };
            // A readable collection can still hold a malformed selected face whose `cmap` and
            // `name` work; do not hand that index to epaint, which panics when it installs a
            // configured face it cannot parse.
            if !cjk::font_face_is_readable(source.bytes(), index) {
                continue;
            }
            if !scripts::face_covers(source.bytes(), index, c) {
                continue;
            }
            return Some(self.system_insert_face(file.path, index, source));
        }
        None
    }

    /// The first non-CJK candidate path that maps `c`; these files have no requested family, so
    /// the same character-map rule as the lazy loader applies.
    fn system_insert_paths(&self, files: Vec<PathBuf>, c: char) -> Option<FontInsert> {
        for path in files {
            let Some(source) = self.system_source(&path) else { continue };
            let Some(index) = (self.covers)(source.bytes(), c) else { continue };
            if !cjk::font_face_is_readable(source.bytes(), index) {
                continue;
            }
            return Some(self.system_insert_face(path, index, source));
        }
        None
    }
}

impl FontProvider for CraftFontProvider {
    fn font_for(&self, request: &FallbackRequest<'_>) -> Option<FontInsert> {
        let c = request.cluster.chars().next()?;
        let locale = (self.locale)();
        let order = cjk::script_order(locale.as_deref());
        // The migration baseline appended every craft face to the family stack in manifest
        // order, so a non-CJK/unknown effective locale drew shared Han/kana with the first
        // manifest face that covered the character (Shippori Mincho Regular). An explicit
        // ja/ko/zh preference is a new choice and keeps the UI-first order for its script.
        // `locale` already resolves an English UI on a CJK OS to that OS locale, so this
        // baseline does not apply there.
        let manifest_baseline = cjk::locale_script(locale.as_deref()).is_none();
        let craft = |iso: &'static str| {
            if manifest_baseline { craft_face_manifest(self.embedded, iso, c) } else { craft_face_for(self.embedded, iso, c) }
        };
        if let Some(kind) = cjk::classify(c) {
            // Han follows the locale's script. For a non-CJK/unknown effective locale, try the
            // manifest's Japanese faces first (exactly as the baseline stack did) before the
            // SC-first system order.
            if kind == cjk::CjkChar::Han
                && manifest_baseline
                && let Some((index, face)) = craft_face_manifest(self.embedded, "Jpan", c)
            {
                return Some(craft_insert(index, face, self.scale()));
            }
            // The same order as `CjkFallback::next_font`: the character's preferred script first,
            // then the locale order; craft faces and system files per script. A Traditional
            // Chinese locale therefore checks its system Hant font before any Simplified craft
            // face, and only falls through when the system has no Hant face either.
            //
            // While fontconfig is still looking for the preferred script's face, do not cache a
            // lower-priority face (SC for a Hant character): egui keeps that wrong choice for the
            // installed font chain. Returning `None` lets the next layout retry after the scan.
            let mut scripts = vec![kind.preferred(&order)];
            scripts.extend(order);
            let mut tried: Vec<CjkScript> = Vec::new();
            for script in scripts {
                if tried.contains(&script) {
                    continue;
                }
                tried.push(script);
                if let Some((index, face)) = craft(cjk_iso(script)) {
                    return Some(craft_insert(index, face, self.scale()));
                }
                if let Some(insert) = self.system_insert_files((self.files)(script), c) {
                    return Some(insert);
                }
                if (self.pending)() {
                    return None;
                }
            }
            return self.system_insert_files((self.last_resort)(), c);
        }
        let Some(script) = scripts::classify(c) else {
            return craft("Zsym").map(|(index, face)| craft_insert(index, face, self.scale()));
        };
        if let Some((index, face)) = craft(script_iso(script)) {
            return Some(craft_insert(index, face, self.scale()));
        }
        if let Some(insert) = self.system_insert_paths((self.scripts)(script), c) {
            return Some(insert);
        }
        // The lazy loader's last-resort list is broad (Arial Unicode, Droid Sans Fallback): it is
        // the same final source for non-CJK scripts when no script-specific file is installed.
        self.system_insert_files((self.last_resort)(), c)
    }
}

/// Whether `face` really maps `c` in the face [`craft_insert`] would install.
///
/// `craft_insert` builds `FontData::from_static(face.bytes)`, whose default face index is 0, so
/// testing any face of a collection would select a face whose glyphs egui never uses. Test face
/// 0 here, exactly like the lazy loader's `load_embedded_faces`.
fn craft_face_installs_coverage(face: &CraftFont, c: char) -> bool {
    !face.bytes.is_empty() && cjk::font_face_is_readable(face.bytes, 0) && scripts::face_covers(face.bytes, 0, c)
}

/// The best injected craft face declaring `iso` that really covers `c` in the face
/// [`craft_insert`] installs.
fn craft_face_for(embedded: fn(&'static str) -> Vec<&'static CraftFont>, iso: &'static str, c: char) -> Option<(usize, &'static CraftFont)> {
    embedded(iso).into_iter().find_map(|face| {
        let index = craft_index(face)?;
        craft_face_installs_coverage(face, c).then_some((index, face))
    })
}

/// The injected craft face the migration baseline would have used first: the earliest manifest
/// face declaring `iso` that really covers `c` in the face [`craft_insert`] installs. (The
/// baseline appended the whole manifest in order; for Japanese that is Shippori Mincho Regular,
/// not the UI-first BIZ UDPGothic.)
fn craft_face_manifest(embedded: fn(&'static str) -> Vec<&'static CraftFont>, iso: &'static str, c: char) -> Option<(usize, &'static CraftFont)> {
    let mut faces = embedded(iso);
    // Stable: faces outside the global table (test doubles) keep their injected order.
    faces.sort_by_key(|face| craft_index(face).unwrap_or(usize::MAX));
    faces.into_iter().find_map(|face| {
        let index = craft_index(face)?;
        craft_face_installs_coverage(face, c).then_some((index, face))
    })
}

fn cjk_iso(script: CjkScript) -> &'static str {
    match script {
        CjkScript::Japanese => "Jpan",
        CjkScript::SimplifiedChinese => "Hans",
        CjkScript::TraditionalChinese => "Hant",
        CjkScript::Korean => "Kore",
    }
}

fn script_iso(script: Script) -> &'static str {
    use Script::*;
    match script {
        Thai => "Thai",
        Lao => "Laoo",
        Khmer => "Khmr",
        Myanmar => "Mymr",
        Tibetan => "Tibt",
        Arabic => "Arab",
        Hebrew => "Hebr",
        Armenian => "Armn",
        Georgian => "Geor",
        Ethiopic => "Ethi",
        Devanagari => "Deva",
        Bengali => "Beng",
        Gurmukhi => "Guru",
        Gujarati => "Gujr",
        Oriya => "Orya",
        Tamil => "Taml",
        Telugu => "Telu",
        Kannada => "Knda",
        Malayalam => "Mlym",
        Sinhala => "Sinh",
    }
}

/// The manifest index of a craft face (the name suffix of its `FontInsert`).
fn craft_index(face: &CraftFont) -> Option<usize> {
    craft_fonts::CRAFT_FONTS.iter().position(|candidate| std::ptr::eq(candidate, face))
}

/// Craft faces declaring `iso`, Regular first, with the Japanese UI face ahead of the other
/// Japanese faces (the explicit-locale choice; a non-CJK/unknown effective locale keeps the
/// manifest order).
fn craft_faces(iso: &str) -> Vec<(usize, &'static CraftFont)> {
    let mut faces: Vec<(usize, &'static CraftFont)> =
        craft_fonts::CRAFT_FONTS.iter().enumerate().filter(|(_, f)| !f.bytes.is_empty() && f.scripts.contains(&iso)).collect();
    faces.sort_by_key(|(_, f)| (f.family != craft_fonts::UI_JAPANESE_FAMILY, f.style != "Regular"));
    faces
}

/// Baseline and variable-weight tweaks shared by craft and system fallbacks.
fn apply_font_tweaks(data: &mut FontData) {
    let offset = vertical_metrics(data.bytes(), data.index)
        .zip(vertical_metrics(photocraft_text::fonts::INTER_REGULAR.as_slice(), 0))
        .map(|(face, primary)| baseline_offset(face, primary));
    let weight = regular_weight(data.bytes(), data.index);
    if let Some(offset) = offset {
        data.tweak.y_offset_factor = offset;
    }
    if let Some(weight) = weight {
        data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
    }
}

fn font_insert(name: String, mut data: FontData, scale: f32) -> FontInsert {
    data.tweak.scale = scale;
    let families: Vec<InsertFontFamily> =
        [FontFamily::Proportional, FontFamily::Monospace].into_iter().map(|family| InsertFontFamily { family, priority: FontPriority::Lowest }).collect();
    FontInsert::new(&name, data, families)
}

/// One craft face as an egui font, with the same baseline/weight tweaks as a lazy fallback and the
/// current UI font-size multiplier. Added to both UI families: a discovered face is appended to
/// each family that asks for it, and a headless frame may use either. `FontData::from_static`
/// leaves the face index at its default 0; the coverage helpers above test that same face.
fn craft_insert(index: usize, face: &'static CraftFont, scale: f32) -> FontInsert {
    let mut data = FontData::from_static(face.bytes);
    apply_font_tweaks(&mut data);
    font_insert(format!("craft-font-{index}"), data, scale)
}

fn craft_provider_id() -> egui::Id {
    egui::Id::unique("photocraft-craft-font-provider")
}

/// The effective locale for CJK choice: an explicitly translated UI language
/// (ja/ko/zh-Hans/zh-Hant) wins; otherwise the OS locale, when known.
///
/// An English UI on a Japanese, Korean or Chinese OS therefore uses that OS script, not the
/// migration baseline's manifest-first face. The baseline applies only when the effective locale
/// is non-CJK or unknown.
fn effective_locale(ui_code: &str, os_locale: Option<&str>) -> Option<String> {
    match ui_code {
        "ja" | "ko" | "zh-hans" | "zh-hant" => Some(ui_code.to_owned()),
        _ => os_locale.map(str::to_owned),
    }
}

impl Sources {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn system() -> Self {
        cjk::start_fontconfig_cjk_scan();
        Self {
            locale: || effective_locale(crate::i18n::current().code(), cjk::ui_locale()),
            files: cjk::font_files,
            last_resort: cjk::last_resort_files,
            pending: cjk::fontconfig_cjk_scan_pending,
            scripts: scripts::system_candidates,
            covers: scripts::covering_face,
            embedded: craft_embedded_iso,
            eager_craft_fonts: true,
        }
    }
    #[cfg(target_arch = "wasm32")]
    pub fn system() -> Self {
        Self {
            locale: || None,
            files: |_| Vec::new(),
            last_resort: Vec::new,
            pending: || false,
            scripts: |_| Vec::new(),
            covers: scripts::covering_face,
            embedded: craft_embedded_iso,
            // The web build embeds no craft fonts; keep its font stack as it was.
            eager_craft_fonts: false,
        }
    }
}

/// Headless test frame pumps do not wait for `request_repaint_after`, so let the optional
/// Fontconfig worker finish before asserting on installed system fonts. `pub` (rather than
/// `pub(crate)`) so strict-policy integration tests can use the same wait as the unit tests.
///
/// Returns whether the scan finished; it never panics in production. Test callers assert the
/// result (and choose their own message).
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn wait_for_fontconfig_cjk_scan() -> bool {
    cjk::start_fontconfig_cjk_scan();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while cjk::fontconfig_cjk_scan_pending() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    !cjk::fontconfig_cjk_scan_pending()
}

/// The lazy loader's state: which scripts were tried and which files were read.
pub struct CjkFallback {
    sources: Sources,
    /// set_fonts takes effect next pass; do not collide with a previous loader's font names.
    defer_after_reset: bool,
    order: Option<[CjkScript; 4]>,
    tried: Vec<CjkScript>,
    last_resort_tried: bool,
    /// Non-CJK scripts whose candidates were tried (each at most once per install).
    scripts_tried: Vec<Script>,
    loaded: Vec<PathBuf>,
    /// Fonts registered so far: (name, path, face index).
    pub registered: Vec<(String, PathBuf, u32)>,
}

impl CjkFallback {
    pub fn new(sources: Sources) -> Self {
        Self {
            sources,
            defer_after_reset: false,
            order: None,
            tried: Vec::new(),
            last_resort_tried: false,
            scripts_tried: Vec::new(),
            loaded: Vec::new(),
            registered: Vec::new(),
        }
    }

    /// Script order for the UI locale (resolved on first use, not at startup).
    pub fn order(&mut self) -> [CjkScript; 4] {
        *self.order.get_or_insert_with(|| cjk::script_order((self.sources.locale)().as_deref()))
    }

    /// Every script and the last-resort fonts have been tried: nothing more to load.
    pub fn exhausted(&self) -> bool {
        self.tried.len() >= 4 && self.last_resort_tried
    }

    /// The next font to register for `c` (kind `kind`) that no current font covers: the first
    /// readable file of the first untried script (the character's preferred script, then the
    /// locale order), then the last-resort fonts. For a non-CJK/unknown effective locale the
    /// craft faces for shared Han and kana follow the manifest's own order (Shippori Mincho
    /// before BIZ UDPGothic), exactly like the migration baseline's family stack; Hangul keeps
    /// the Korean face. `None` when nothing is left or while the preferred script's Fontconfig
    /// lookup is still pending.
    pub fn next_font(&mut self, c: char, kind: CjkChar) -> Option<(String, FontData)> {
        // The craft manifest lists Shippori Mincho first. A non-CJK effective locale keeps that
        // baseline choice for Han and kana; an explicit zh-Hans/zh-Hant/ja/ko preference keeps the
        // script's UI-first order.
        let manifest_baseline = cjk::locale_script((self.sources.locale)().as_deref()).is_none();
        if kind == CjkChar::Han
            && manifest_baseline
            && let Some(f) = self.load_embedded_manifest(CjkScript::Japanese, c)
        {
            return Some(f);
        }
        let order = self.order();
        let scripts: Vec<CjkScript> = std::iter::once(kind.preferred(&order)).chain(order).collect();
        for s in scripts {
            if self.tried.contains(&s) {
                continue;
            }
            let embedded = if manifest_baseline { self.load_embedded_manifest(s, c) } else { self.load_embedded(s, c) };
            if let Some(f) = embedded {
                self.tried.push(s);
                return Some(f);
            }
            let fontconfig_pending = (self.sources.pending)();
            let files = (self.sources.files)(s);
            if let Some(f) = self.load_first(&files, c) {
                self.tried.push(s);
                return Some(f);
            }
            if fontconfig_pending {
                return None;
            }
            self.tried.push(s);
        }
        if !self.last_resort_tried {
            self.last_resort_tried = true;
            let files = (self.sources.last_resort)();
            return self.load_first(&files, c);
        }
        None
    }

    /// Every non-CJK script has been tried.
    pub fn scripts_exhausted(&self) -> bool {
        self.scripts_tried.len() >= Script::ALL.len()
    }

    /// The first candidate font for `c`'s script that really maps `c`; the script is tried once.
    pub fn next_script_font(&mut self, c: char) -> Option<(String, FontData)> {
        let script = scripts::classify(c)?;
        if self.scripts_tried.contains(&script) {
            return None;
        }
        self.scripts_tried.push(script);
        let mut reads = 0;
        for path in (self.sources.scripts)(script) {
            if reads >= MAX_SCRIPT_READS {
                break;
            }
            if self.loaded.contains(&path) {
                continue;
            }
            let Some(bytes) = read_font(&path) else { continue };
            reads += 1;
            if !cjk::font_file_is_readable(&bytes) {
                continue;
            }
            let Some(index) = (self.sources.covers)(&bytes, c) else { continue };
            if !cjk::font_face_is_readable(&bytes, index) {
                continue;
            }
            self.loaded.push(path.clone());
            let name = format!("{FONT_PREFIX}-{}", self.registered.len());
            log::info!("UI font fallback: registered {} (face {index}) for {script:?} as {name}", path.display());
            self.registered.push((name.clone(), path, index));
            let mut data = FontData::from_owned(bytes);
            data.index = index;
            return Some((name, data));
        }
        log::info!("UI font fallback: no installed font covers {script:?}");
        None
    }

    /// The first embedded (craft-fonts) face for `s` in the UI-first order (explicit locale).
    fn load_embedded(&mut self, s: CjkScript, c: char) -> Option<(String, FontData)> {
        let faces = (self.sources.embedded)(cjk_iso(s));
        self.load_embedded_faces(faces, c)
    }

    /// The first embedded face for `s` in the migration baseline's manifest order
    /// (non-CJK/unknown effective locale): Shippori Mincho Regular before BIZ UDPGothic.
    fn load_embedded_manifest(&mut self, s: CjkScript, c: char) -> Option<(String, FontData)> {
        let mut faces = (self.sources.embedded)(cjk_iso(s));
        faces.sort_by_key(|face| craft_index(face).unwrap_or(usize::MAX));
        self.load_embedded_faces(faces, c)
    }

    /// The first of `faces` that really covers `c`, registered from its static bytes.
    fn load_embedded_faces(&mut self, faces: Vec<&'static CraftFont>, c: char) -> Option<(String, FontData)> {
        let f = faces.into_iter().find(|f| !f.bytes.is_empty() && cjk::font_face_is_readable(f.bytes, 0) && scripts::face_covers(f.bytes, 0, c))?;
        let path = PathBuf::from(format!("craft-fonts/{} {}", f.family, f.style));
        if self.loaded.contains(&path) {
            return None;
        }
        self.loaded.push(path.clone());
        let name = format!("{FONT_PREFIX}-{}", self.registered.len());
        log::info!("UI font fallback: registered embedded {} as {name}", path.display());
        self.registered.push((name.clone(), path, 0));
        Some((name, FontData::from_static(f.bytes)))
    }

    /// The first file whose requested collection face (or face 0 for an empty family) really
    /// covers `c`. A family miss skips the file rather than using whichever face happens to cover
    /// the character.
    fn load_first(&mut self, files: &[FontFile], c: char) -> Option<(String, FontData)> {
        for f in files {
            if self.loaded.contains(&f.path) {
                continue;
            }
            let Some(bytes) = read_font(&f.path) else { continue };
            // Register only what the text engine can actually parse. A readable collection may
            // still have a selected face whose `cmap` is valid while another table points outside
            // the file; `font_face_is_readable` below rejects such a face before registration.
            if !cjk::font_file_is_readable(&bytes) {
                continue;
            }
            let index = if f.family.is_empty() {
                0
            } else {
                let Some(index) = cjk::face_index_for_family_checked(&bytes, &f.family) else { continue };
                index
            };
            // `face_covers` only reads `cmap`; validate the selected face's whole table directory
            // before `ctx.add_font` may install (and panic on) a face another table makes unreadable.
            if !cjk::font_face_is_readable(&bytes, index) {
                continue;
            }
            if !scripts::face_covers(&bytes, index, c) {
                continue;
            }
            self.loaded.push(f.path.clone());
            let name = format!("{FONT_PREFIX}-{}", self.registered.len());
            log::info!("UI font fallback: registered {} (face {index}) as {name}", f.path.display());
            self.registered.push((name.clone(), f.path.clone(), index));
            let mut data = FontData::from_owned(bytes);
            data.index = index;
            return Some((name, data));
        }
        None
    }
}

/// A font file's bytes, or `None` for missing, empty, oversized or unreadable files.
fn read_font(path: &std::path::Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_FONT_BYTES {
        return None;
    }
    std::fs::read(path).ok()
}

/// First supported missing character for which a fallback has not yet been exhausted.
fn missing_char(ctx: &egui::Context, shapes: &[egui::epaint::ClippedShape], fallback: &CjkFallback) -> Option<char> {
    fn collect(shape: &Shape, out: &mut Vec<char>, fallback: &CjkFallback, cjk_pending: bool) {
        let wanted = |c: char| (cjk_pending && cjk::classify(c).is_some()) || scripts::classify(c).is_some_and(|s| !fallback.scripts_tried.contains(&s));
        match shape {
            Shape::Text(t) => {
                let text = &t.galley.job.text;
                if !text.is_ascii() {
                    for c in text.chars().filter(|c| wanted(*c)) {
                        if out.len() >= 256 {
                            return;
                        }
                        if !out.contains(&c) {
                            out.push(c);
                        }
                    }
                }
            }
            Shape::Vec(v) => v.iter().for_each(|s| collect(s, out, fallback, cjk_pending)),
            _ => {}
        }
    }
    let mut chars = Vec::new();
    for s in shapes {
        collect(&s.shape, &mut chars, fallback, !fallback.exhausted());
    }
    if chars.is_empty() {
        return None;
    }
    let font = FontId::proportional(12.0);
    ctx.fonts_mut(|f| chars.into_iter().find(|c| !f.has_glyph(&font, *c)))
}

/// Big-endian readers over a font's bytes; `None` past the end.
fn be_u16(font: &[u8], o: usize) -> Option<u16> {
    font.get(o..o.checked_add(2)?).map(|b| u16::from_be_bytes([b[0], b[1]]))
}

fn be_u32(font: &[u8], o: usize) -> Option<u32> {
    font.get(o..o.checked_add(4)?).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// The offset of table `tag` in face `index` of `font` (a collection or a single face).
fn table_offset(font: &[u8], index: u32, tag: &[u8; 4]) -> Option<usize> {
    let dir = if font.get(..4)? == b"ttcf" {
        // Collection: the offset table of face `index` follows a 12-byte header.
        be_u32(font, 12usize.checked_add((index as usize).checked_mul(4)?)?)? as usize
    } else {
        0
    };
    let tables = be_u16(font, dir.checked_add(4)?)? as usize;
    (0..tables).find_map(|i| {
        let rec = dir.checked_add(12)?.checked_add(i.checked_mul(16)?)?;
        (font.get(rec..rec.checked_add(4)?)? == tag).then_some(())?;
        Some(be_u32(font, rec.checked_add(8)?)? as usize)
    })
}

/// (ascent, descent, line gap) of a face in em, from its `hhea` table; `descent` is negative.
fn vertical_metrics(font: &[u8], index: u32) -> Option<(f32, f32, f32)> {
    let i16_at = |o: usize| be_u16(font, o).map(|v| v as i16 as f32);
    let (hhea, head) = (table_offset(font, index, b"hhea")?, table_offset(font, index, b"head")?);
    let upm = be_u16(font, head.checked_add(18)?)? as f32;
    if upm <= 0.0 {
        return None;
    }
    Some((i16_at(hhea.checked_add(4)?)? / upm, i16_at(hhea.checked_add(6)?)? / upm, i16_at(hhea.checked_add(8)?)? / upm))
}

/// The weight a variable face should be drawn at: Regular (400) within its `wght` axis, or
/// `None` when it has no weight axis or already defaults to Regular. Noto Sans CJK's variable
/// collection defaults to Thin (100), so the UI's CJK text came out hairline (#4820).
fn regular_weight(font: &[u8], index: u32) -> Option<f32> {
    let fvar = table_offset(font, index, b"fvar")?;
    let axes = fvar.checked_add(be_u16(font, fvar.checked_add(4)?)? as usize)?;
    let count = be_u16(font, fvar.checked_add(8)?)? as usize;
    let size = be_u16(font, fvar.checked_add(10)?)? as usize;
    let fixed = |o: usize| be_u32(font, o).map(|v| v as i32 as f32 / 65536.0);
    (0..count.min(64)).find_map(|i| {
        let rec = axes.checked_add(i.checked_mul(size)?)?;
        (font.get(rec..rec.checked_add(4)?)? == b"wght").then_some(())?;
        let (min, default, max) = (fixed(rec.checked_add(4)?)?, fixed(rec.checked_add(8)?)?, fixed(rec.checked_add(12)?)?);
        let regular = 400f32.clamp(min.min(max), max.max(min));
        (regular.is_finite() && (regular - default).abs() > 0.5).then_some(regular)
    })
}

/// `y_offset_factor` that puts `face`'s baseline where `primary`'s would be. egui centres a
/// fallback face's row in the primary font's row, so a face with a big line gap (Hiragino: 0.5em)
/// otherwise rides about 0.23em high next to Latin text.
fn baseline_offset(face: (f32, f32, f32), primary: (f32, f32, f32)) -> f32 {
    let height = |m: (f32, f32, f32)| m.0 - m.1 + m.2;
    let off = primary.0 - face.0 - 0.5 * (height(primary) - height(face));
    if off.is_finite() { off.clamp(-0.5, 0.5) } else { 0.0 }
}

/// Registers `name` at the lowest priority in every font family.
fn add_to_all_families(ctx: &egui::Context, name: String, mut data: FontData) {
    // Align to Inter, the primary of every UI family; JetBrains Mono differs by under 0.005em.
    let primary = ctx.fonts(|f| f.definitions().font_data.get("Inter").and_then(|p| vertical_metrics(p.bytes(), p.index)));
    if let (Some(face), Some(primary)) = (vertical_metrics(data.bytes(), data.index), primary) {
        data.tweak.y_offset_factor = baseline_offset(face, primary);
    }
    if let Some(weight) = regular_weight(data.bytes(), data.index) {
        data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
    }
    let mut families: Vec<FontFamily> = ctx.fonts(|f| f.definitions().families.keys().cloned().collect());
    for f in [FontFamily::Proportional, FontFamily::Monospace] {
        if !families.contains(&f) {
            families.push(f);
        }
    }
    let families = families.into_iter().map(|family| InsertFontFamily { family, priority: FontPriority::Lowest }).collect();
    crate::theme::size_ui_font(ctx, &name, &mut data);
    ctx.add_font(FontInsert { name, data, families });
}

/// The egui plugin that watches drawn text and loads fallback fonts on demand.
pub struct CjkFontPlugin(pub CjkFallback);

impl egui::Plugin for CjkFontPlugin {
    fn debug_name(&self) -> &'static str {
        "photocraft-cjk-fonts"
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        if std::mem::take(&mut self.0.defer_after_reset) {
            ctx.request_repaint();
            return;
        }
        if self.0.exhausted() && self.0.scripts_exhausted() {
            return;
        }
        let Some(c) = missing_char(ctx, &output.shapes, &self.0) else { return };
        let font = match cjk::classify(c) {
            Some(kind) => self.0.next_font(c, kind),
            None => self.0.next_script_font(c),
        };
        if let Some((name, data)) = font {
            add_to_all_families(ctx, name, data);
            ctx.request_repaint();
        } else if cjk::classify(c).is_some() && (self.0.sources.pending)() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        } else if !self.0.exhausted() || !self.0.scripts_exhausted() {
            // Nothing readable for this script; try the next one on the next frame.
            ctx.request_repaint();
        }
    }
}

/// Installs the lazy CJK and other-script fallback (system fonts; nothing on the web).
pub fn install(ctx: &egui::Context) {
    install_with(ctx, Sources::system());
}

pub fn install_with(ctx: &egui::Context, sources: Sources) {
    // egui keeps the first plugin of a type. set_fonts removes loaded fallbacks, so reset the
    // existing loader too; otherwise its tried/loaded sets prevent fonts being added again.
    let mut fallback = CjkFallback::new(sources.clone());
    fallback.defer_after_reset = true;
    if ctx
        .with_plugin::<CjkFontPlugin, _>(|plugin| {
            plugin.0 = CjkFallback::new(sources.clone());
            plugin.0.defer_after_reset = true;
        })
        .is_none()
    {
        ctx.add_plugin(CjkFontPlugin(fallback));
    }
    // The provider resolves a craft face for each missing character in the first frame, before the
    // lazy loader's next-pass system fallback. Without it a headless frame panics on CJK text.
    if sources.eager_craft_fonts && !craft_fonts::CRAFT_FONTS.is_empty() {
        let provider = std::sync::Arc::new(CraftFontProvider::new(&sources, crate::theme::ui_font_scale(ctx)));
        let as_provider: std::sync::Arc<dyn FontProvider> = provider.clone();
        ctx.data_mut(|d| d.insert_temp(craft_provider_id(), provider));
        ctx.set_font_providers(vec![as_provider]);
    }
}

/// Keep provider-discovered craft faces in step with Interface > UI Font Size. The size plugin
/// calls this after `ctx.set_fonts` with the rescaled stack. A face discovered while the old stack
/// was still active stays at the old scale for the rest of that pass; recreating `Fonts` for the
/// next pass drops the discovered faces (they are not part of `FontDefinitions`) and the provider
/// rediscovers them at its new scale.
pub(crate) fn set_craft_font_scale(ctx: &egui::Context, scale: f32) {
    let Some(provider) = ctx.data(|d| d.get_temp::<std::sync::Arc<CraftFontProvider>>(craft_provider_id())) else { return };
    provider.set_scale(scale);
    let as_provider: std::sync::Arc<dyn FontProvider> = provider;
    ctx.set_font_providers(vec![as_provider]);
}
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;
