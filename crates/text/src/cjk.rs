//! CJK font fallback: which installed system fonts cover Japanese, Simplified Chinese,
//! Traditional Chinese and Korean, and in which order to try them for the user's locale.
//!
//! Han characters are shared between the four, but their preferred glyph forms differ, so the
//! font for the locale's script goes first: Japanese forms only for a Japanese locale, Chinese
//! forms for zh-Hans / zh-Hant, Korean (Hanja-capable) fonts for ko. Kana always prefers a
//! Japanese font, Hangul a Korean one and Bopomofo a Traditional Chinese one.
//!
//! Used by the egui shell (UI fallback fonts, loaded lazily) and by [`crate::fonts::FontDb`]
//! (Type tool fallback families).

/// A CJK writing system with its own preferred fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CjkScript {
    Japanese,
    SimplifiedChinese,
    TraditionalChinese,
    Korean,
}

/// The kind of CJK character that needs a fallback font.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CjkChar {
    /// Ideographs, CJK punctuation and full-width forms: font choice follows the locale.
    Han,
    Kana,
    Hangul,
    Bopomofo,
}

impl CjkChar {
    /// The script whose fonts should be tried first for this character.
    pub fn preferred(self, order: &[CjkScript; 4]) -> CjkScript {
        match self {
            CjkChar::Kana => CjkScript::Japanese,
            CjkChar::Hangul => CjkScript::Korean,
            CjkChar::Bopomofo => CjkScript::TraditionalChinese,
            CjkChar::Han => order[0],
        }
    }
}

/// Classifies a character that needs a CJK font, or `None` for everything else.
pub fn classify(c: char) -> Option<CjkChar> {
    let u = c as u32;
    Some(match u {
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7AF | 0xD7B0..=0xD7FF | 0xFFA0..=0xFFDC => CjkChar::Hangul,
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF65..=0xFF9F | 0x1B000..=0x1B16F => CjkChar::Kana,
        0x3100..=0x312F | 0x31A0..=0x31BF => CjkChar::Bopomofo,
        0x2E80..=0x2FDF | 0x3000..=0x303F | 0x3190..=0x319F | 0x31C0..=0x31EF | 0x3200..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF => CjkChar::Han,
        0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF64 | 0xFFE0..=0xFFEF | 0x20000..=0x3FFFF => CjkChar::Han,
        _ => return None,
    })
}

/// A locale tag reduced to what [`script_order`] and [`locale_script`] compare:
/// lowercase, `_` as `-`, without an encoding or modifier suffix.
fn normalized_locale(locale: Option<&str>) -> String {
    locale.unwrap_or("").split(['.', '@']).next().unwrap_or("").to_ascii_lowercase().replace('_', "-")
}

/// The CJK script a locale explicitly asks for, or `None` for a non-CJK (or unknown) locale.
///
/// [`script_order`] also gives non-CJK locales a default order (Simplified Chinese first). The
/// craft-font manifest, however, lists its Japanese faces before Noto Sans CJK SC, so the eager
/// provider keeps that Japanese-first baseline for Han unless the user's locale names another
/// script. This helper distinguishes a non-CJK/unknown effective locale (use the baseline
/// manifest order) from "zh-Hans/zh-Hant/ja/ko, use this script".
pub fn locale_script(locale: Option<&str>) -> Option<CjkScript> {
    use CjkScript::*;
    let tag = normalized_locale(locale);
    let mut parts = tag.split('-');
    let lang = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    match lang {
        "ja" => Some(Japanese),
        "ko" => Some(Korean),
        "zh" | "yue" => {
            let hant = rest.iter().any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo"));
            let hans = rest.iter().any(|p| matches!(*p, "hans" | "cn" | "sg"));
            if (hant || lang == "yue") && !hans { Some(TraditionalChinese) } else { Some(SimplifiedChinese) }
        }
        _ => None,
    }
}

/// Script order for a locale tag such as `ja_JP.UTF-8`, `zh-Hant-TW`, `zh_CN` or `ko-KR`.
/// Non-CJK (or unknown) locales get Simplified Chinese first for system fonts, so Japanese
/// forms only win for shared Han when the locale is Japanese (craft faces keep the manifest's
/// Japanese-first order through [`locale_script`]).
pub fn script_order(locale: Option<&str>) -> [CjkScript; 4] {
    use CjkScript::*;
    match locale_script(locale) {
        Some(Japanese) => [Japanese, SimplifiedChinese, TraditionalChinese, Korean],
        Some(Korean) => [Korean, TraditionalChinese, SimplifiedChinese, Japanese],
        Some(TraditionalChinese) => [TraditionalChinese, SimplifiedChinese, Japanese, Korean],
        Some(SimplifiedChinese) | None => [SimplifiedChinese, TraditionalChinese, Japanese, Korean],
    }
}

/// The user's UI locale (cached). `PHOTOCRAFT_LOCALE` overrides; then macOS's preferred
/// language list, the Windows user locale, and finally `LC_ALL` / `LC_MESSAGES` / `LANG` /
/// `LANGUAGE`. `None` on the web or when nothing is set.
pub fn ui_locale() -> Option<&'static str> {
    static LOCALE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    LOCALE.get_or_init(detect_locale).as_deref()
}

/// [`script_order`] for [`ui_locale`].
pub fn ui_script_order() -> [CjkScript; 4] {
    script_order(ui_locale())
}

fn detect_locale() -> Option<String> {
    let env = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty() && v != "C" && v != "POSIX" && !v.starts_with("C."));
    if let Some(v) = env("PHOTOCRAFT_LOCALE") {
        return Some(v);
    }
    if let Some(v) = os_locale() {
        return Some(v);
    }
    ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"].iter().find_map(|k| env(k)).map(|v| v.split(':').next().unwrap_or("").to_string())
}

#[cfg(target_os = "macos")]
fn os_locale() -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let path = std::path::Path::new(&home).join("Library/Preferences/.GlobalPreferences.plist");
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() > 4 << 20 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    bplist_first_language(&bytes)
}

#[cfg(target_os = "windows")]
fn os_locale() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg")
        .args(["query", "HKCU\\Control Panel\\International", "/v", "LocaleName"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // "    LocaleName    REG_SZ    ja-JP"
    text.lines().find(|l| l.contains("LocaleName")).and_then(|l| l.split_whitespace().last()).map(str::to_string)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn os_locale() -> Option<String> {
    None
}

/// First entry of `AppleLanguages` in a binary property list (macOS `.GlobalPreferences.plist`).
/// A minimal, bounds-checked reader of the documented `bplist00` format; `None` on anything else.
pub fn bplist_first_language(b: &[u8]) -> Option<String> {
    if b.len() < 40 || !b.starts_with(b"bplist00") {
        return None;
    }
    let t = b.get(b.len() - 32..)?;
    let offset_size = *t.get(6)? as usize;
    let ref_size = *t.get(7)? as usize;
    let be = |s: &[u8]| s.iter().fold(0u64, |a, &x| (a << 8) | x as u64);
    let num_objects = usize::try_from(be(t.get(8..16)?)).ok()?;
    let top = usize::try_from(be(t.get(16..24)?)).ok()?;
    let table = usize::try_from(be(t.get(24..32)?)).ok()?;
    if !(1..=8).contains(&offset_size) || !(1..=8).contains(&ref_size) || num_objects > b.len() {
        return None;
    }
    let offset = |obj: usize| -> Option<usize> {
        if obj >= num_objects {
            return None;
        }
        let at = table.checked_add(obj.checked_mul(offset_size)?)?;
        usize::try_from(be(b.get(at..at.checked_add(offset_size)?)?)).ok()
    };
    // (marker high nibble, count, start of payload)
    let header = |obj: usize| -> Option<(u8, usize, usize)> {
        let at = offset(obj)?;
        let m = *b.get(at)?;
        let (kind, low) = (m >> 4, (m & 0x0F) as usize);
        if low != 0x0F || !matches!(kind, 0x5 | 0x6 | 0xA | 0xD) {
            return Some((kind, low, at + 1));
        }
        let im = *b.get(at + 1)?;
        if im >> 4 != 0x1 {
            return None;
        }
        let n = 1usize.checked_shl((im & 0x0F) as u32)?;
        let count = usize::try_from(be(b.get(at + 2..at.checked_add(2 + n)?)?)).ok()?;
        Some((kind, count, at + 2 + n))
    };
    let string = |obj: usize| -> Option<String> {
        let (kind, n, start) = header(obj)?;
        match kind {
            0x5 => b.get(start..start.checked_add(n)?).map(|s| String::from_utf8_lossy(s).into_owned()),
            0x6 => {
                let s = b.get(start..start.checked_add(n.checked_mul(2)?)?)?;
                Some(String::from_utf16_lossy(&s.as_chunks::<2>().0.iter().map(|p| u16::from_be_bytes(*p)).collect::<Vec<_>>()))
            }
            _ => None,
        }
    };
    let obj_ref = |at: usize| -> Option<usize> { usize::try_from(be(b.get(at..at.checked_add(ref_size)?)?)).ok() };
    let (kind, n, start) = header(top)?;
    if kind != 0xD {
        return None;
    }
    for i in 0..n {
        let key = obj_ref(start.checked_add(i.checked_mul(ref_size)?)?)?;
        if string(key).as_deref() != Some("AppleLanguages") {
            continue;
        }
        let val = obj_ref(start.checked_add(n.checked_add(i)?.checked_mul(ref_size)?)?)?;
        let (akind, an, astart) = header(val)?;
        if akind != 0xA || an == 0 {
            return None;
        }
        return string(obj_ref(astart)?).filter(|s| !s.is_empty());
    }
    None
}

/// Family names (as the Type tool's font database knows them) per script, most preferred first.
pub fn families(script: CjkScript) -> &'static [&'static str] {
    match script {
        CjkScript::Japanese => &["Hiragino Sans", "Hiragino Kaku Gothic ProN", "Yu Gothic", "Meiryo", "Noto Sans CJK JP", "Noto Sans JP", "IPAexGothic"],
        CjkScript::SimplifiedChinese => {
            &["PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", "Noto Sans CJK SC", "Noto Sans SC", "WenQuanYi Micro Hei", "WenQuanYi Zen Hei", "Heiti SC"]
        }
        CjkScript::TraditionalChinese => &["PingFang TC", "Microsoft JhengHei", "Noto Sans CJK TC", "Noto Sans TC", "Hiragino Sans CNS", "Heiti TC"],
        CjkScript::Korean => &["Apple SD Gothic Neo", "Malgun Gothic", "Noto Sans CJK KR", "Noto Sans KR", "NanumGothic", "UnDotum"],
    }
}

/// An installed font file to try: its path and the family of the wanted face in a collection
/// (empty: face 0). Matching by family name keeps working when a `.ttc` reorders its faces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFile {
    pub path: std::path::PathBuf,
    pub family: String,
}

#[cfg(target_os = "linux")]
struct FontconfigCjkScan {
    started: std::sync::atomic::AtomicBool,
    result: std::sync::Mutex<Option<[Option<FontFile>; 4]>>,
}

#[cfg(target_os = "linux")]
fn fontconfig_cjk_scan() -> &'static FontconfigCjkScan {
    static SCAN: std::sync::OnceLock<FontconfigCjkScan> = std::sync::OnceLock::new();
    SCAN.get_or_init(|| FontconfigCjkScan { started: std::sync::atomic::AtomicBool::new(false), result: std::sync::Mutex::new(None) })
}

#[cfg(target_os = "linux")]
/// Start the optional Fontconfig CJK-family lookup in the background.
///
/// The scan uses `fc-match` when available and leaves the built-in path candidates unchanged
/// when it is missing or cannot resolve a font.
pub fn start_fontconfig_cjk_scan() {
    use std::sync::atomic::Ordering;

    let scan = fontconfig_cjk_scan();
    if scan.started.swap(true, Ordering::Relaxed) {
        return;
    }
    let scan = fontconfig_cjk_scan();
    if std::thread::Builder::new()
        .name("photocraft-fontconfig-cjk".into())
        .spawn(move || {
            let result = [CjkScript::Japanese, CjkScript::SimplifiedChinese, CjkScript::TraditionalChinese, CjkScript::Korean].map(fontconfig_match);
            *scan.result.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
        })
        .is_err()
    {
        *fontconfig_cjk_scan().result.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(std::array::from_fn(|_| None));
    }
}

#[cfg(not(target_os = "linux"))]
/// No-op outside Linux, where the Fontconfig lookup is unavailable.
pub fn start_fontconfig_cjk_scan() {}

#[cfg(target_os = "linux")]
/// Whether the optional background Fontconfig CJK lookup is still running.
pub fn fontconfig_cjk_scan_pending() -> bool {
    let scan = fontconfig_cjk_scan();
    scan.started.load(std::sync::atomic::Ordering::Relaxed) && scan.result.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_none()
}

#[cfg(not(target_os = "linux"))]
/// Fontconfig lookup is unavailable outside Linux.
pub fn fontconfig_cjk_scan_pending() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn fontconfig_match(script: CjkScript) -> Option<FontFile> {
    use std::ffi::OsString;
    use std::io::Read as _;
    use std::os::unix::ffi::OsStringExt;

    const MAX_FONT_BYTES: u64 = 128 << 20;

    let language = match script {
        CjkScript::Japanese => "ja",
        CjkScript::SimplifiedChinese => "zh-cn",
        CjkScript::TraditionalChinese => "zh-tw",
        CjkScript::Korean => "ko",
    };
    let output = std::process::Command::new("fc-match").args(["--format=%{file}\\t%{family[0]}\\n", &format!(":lang={language}")]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let line = output.stdout.split(|byte| *byte == b'\n').find(|line| !line.is_empty())?;
    let separator = line.iter().position(|byte| *byte == b'\t')?;
    let path = line.get(..separator)?;
    let family = line.get(separator.checked_add(1)?..)?;
    let path = std::path::PathBuf::from(OsString::from_vec(path.to_vec()));
    if !path.is_absolute()
        || !path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc"))
        || !path.is_file()
    {
        return None;
    }
    let family = String::from_utf8_lossy(family).trim().to_string();
    if family.is_empty() {
        return None;
    }

    let file = std::fs::File::open(&path).ok()?;
    let length = file.metadata().ok()?.len();
    if length == 0 || length > MAX_FONT_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(usize::try_from(length).ok()?).ok()?;
    file.take(MAX_FONT_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if u64::try_from(bytes.len()).ok()? > MAX_FONT_BYTES || !fontconfig_font_covers_sample(&bytes, &family, script) {
        return None;
    }

    Some(FontFile { path: path.canonicalize().unwrap_or(path), family })
}

#[cfg(target_os = "linux")]
fn fontconfig_font_covers_sample(bytes: &[u8], family: &str, script: CjkScript) -> bool {
    use skrifa::MetadataProvider;

    let Some(index) = face_index_for_family_checked(bytes, family) else { return false };
    if !font_face_is_readable(bytes, index) {
        return false;
    }
    let Ok(font) = skrifa::FontRef::from_index(bytes, index) else { return false };
    let sample = match script {
        CjkScript::Japanese => "レイヤー図",
        CjkScript::SimplifiedChinese => "图层",
        CjkScript::TraditionalChinese => "圖層",
        CjkScript::Korean => "카드 배경",
    };
    sample.chars().all(|character| font.charmap().map(character).is_some())
}

#[cfg(target_os = "linux")]
fn fontconfig_cjk_font(script: CjkScript) -> Option<FontFile> {
    use std::sync::atomic::Ordering;

    let scan = fontconfig_cjk_scan();
    if !scan.started.load(Ordering::Relaxed) {
        return None;
    }
    let index = match script {
        CjkScript::Japanese => 0,
        CjkScript::SimplifiedChinese => 1,
        CjkScript::TraditionalChinese => 2,
        CjkScript::Korean => 3,
    };
    scan.result.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref()?.get(index)?.clone()
}

#[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
fn fontconfig_cjk_font(_: CjkScript) -> Option<FontFile> {
    None
}

/// System font files for `script`, most preferred first. Paths that don't exist are included;
/// callers skip them.
#[cfg(not(target_arch = "wasm32"))]
pub fn font_files(script: CjkScript) -> Vec<FontFile> {
    use CjkScript::*;
    let f = |p: &str, family: &'static str| FontFile { path: p.into(), family: family.to_string() };
    if cfg!(target_os = "macos") {
        let pingfang = |family| mac_pingfang().map(|path| FontFile { path, family });
        match script {
            Japanese => vec![f("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", ""), f("/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc", "")],
            SimplifiedChinese => pingfang("PingFang SC".to_string())
                .into_iter()
                .chain([f("/System/Library/Fonts/Hiragino Sans GB.ttc", ""), f("/System/Library/Fonts/STHeiti Light.ttc", "Heiti SC")])
                .collect(),
            TraditionalChinese => pingfang("PingFang TC".to_string()).into_iter().chain([f("/System/Library/Fonts/STHeiti Light.ttc", "Heiti TC")]).collect(),
            Korean => vec![f("/System/Library/Fonts/AppleSDGothicNeo.ttc", "Apple SD Gothic Neo")],
        }
    } else if cfg!(target_os = "windows") {
        let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let w = |file: &str, family: &'static str| FontFile { path: std::path::Path::new(&dir).join("Fonts").join(file), family: family.to_string() };
        match script {
            Japanese => vec![w("YuGothR.ttc", "Yu Gothic"), w("YuGothM.ttc", "Yu Gothic"), w("meiryo.ttc", "Meiryo"), w("msgothic.ttc", "")],
            SimplifiedChinese => vec![w("msyh.ttc", "Microsoft YaHei"), w("msyh.ttf", ""), w("simsun.ttc", "")],
            TraditionalChinese => vec![w("msjh.ttc", "Microsoft JhengHei"), w("msjh.ttf", ""), w("mingliu.ttc", "")],
            Korean => vec![w("malgun.ttf", ""), w("gulim.ttc", "")],
        }
    } else {
        // One Noto Sans CJK collection covers all four scripts; the face picks the glyph forms.
        let noto = match script {
            Japanese => "Noto Sans CJK JP",
            SimplifiedChinese => "Noto Sans CJK SC",
            TraditionalChinese => "Noto Sans CJK TC",
            Korean => "Noto Sans CJK KR",
        };
        let mut v: Vec<FontFile> = [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            // FreeBSD ports (x11-fonts/noto-sans-cjk and friends) install under /usr/local.
            "/usr/local/share/fonts/noto/NotoSansCJK-Regular.ttc",
            "/usr/local/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/local/share/fonts/noto/NotoSansCJK-VF.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-sans-cjk-fonts/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-VF.ttc",
        ]
        .iter()
        .map(|p| f(p, noto))
        .collect();
        v.extend(match script {
            Japanese => vec![
                f("/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf", ""),
                f("/usr/share/fonts/truetype/fonts-japanese-gothic.ttf", ""),
                f("/usr/share/fonts/truetype/takao-gothic/TakaoPGothic.ttf", ""),
            ],
            SimplifiedChinese | TraditionalChinese => vec![
                f("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", ""),
                f("/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc", ""),
                f("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", ""),
                f("/usr/share/fonts/wenquanyi/wqy-zenhei/wqy-zenhei.ttc", ""),
            ],
            Korean => vec![
                f("/usr/share/fonts/truetype/nanum/NanumGothic.ttf", ""),
                f("/usr/share/fonts/nanum/NanumGothic.ttf", ""),
                f("/usr/share/fonts/truetype/unfonts-core/UnDotum.ttf", ""),
                f("/usr/share/fonts/un-core/UnDotum.ttf", ""),
            ],
        });
        let mut files = with_flatpak_host_fonts(v);
        if let Some(font) = fontconfig_cjk_font(script)
            && !files.iter().any(|file| file.path == font.path)
        {
            files.push(font);
        }
        files
    }
}

/// Flatpak mounts the host system font directory outside the runtime's `/usr/share/fonts`.
/// Preserve the collection face and font preference order when trying that mirror.
#[cfg(not(target_arch = "wasm32"))]
fn with_flatpak_host_fonts(files: Vec<FontFile>) -> Vec<FontFile> {
    let mut candidates = Vec::new();
    for file in files {
        let host = if cfg!(target_os = "linux") {
            file.path
                .strip_prefix("/usr/share/fonts")
                .ok()
                .map(|relative| FontFile { path: std::path::Path::new("/run/host/fonts").join(relative), family: file.family.clone() })
        } else {
            None
        };
        candidates.push(file);
        candidates.extend(host);
    }
    candidates
}

/// Broad-coverage fonts tried after every script's own fonts (they cover Han, kana and Hangul
/// but look worse than the script fonts).
#[cfg(not(target_arch = "wasm32"))]
pub fn last_resort_files() -> Vec<FontFile> {
    let f = |p: &str| FontFile { path: p.into(), family: String::new() };
    if cfg!(target_os = "macos") {
        vec![f("/System/Library/Fonts/Supplemental/Arial Unicode.ttf"), f("/Library/Fonts/Arial Unicode.ttf")]
    } else if cfg!(target_os = "windows") {
        let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        vec![FontFile { path: std::path::Path::new(&dir).join("Fonts").join("ARIALUNI.TTF"), family: String::new() }]
    } else {
        with_flatpak_host_fonts(vec![
            f("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf"),
            f("/usr/share/fonts/google-droid-sans-fonts/DroidSansFallbackFull.ttf"),
        ])
    }
}

/// macOS ships PingFang as a pre-installed "mobile asset" outside `/System/Library/Fonts`
/// (`/System/Library/AssetsV2/com_apple_MobileAsset_Font*/<hash>.asset/AssetData/PingFang.ttc`).
/// Looks there (a few small directory listings) and in the pre-Catalina location.
#[cfg(not(target_arch = "wasm32"))]
pub fn mac_pingfang() -> Option<std::path::PathBuf> {
    use std::path::{Path, PathBuf};
    let old = PathBuf::from("/System/Library/Fonts/PingFang.ttc");
    if old.is_file() {
        return Some(old);
    }
    let roots = [Path::new("/System/Library/AssetsV2"), Path::new("/System/Library/AssetsV2/PreinstalledAssetsV2/InstallWithOs")];
    for root in roots {
        let Ok(rd) = std::fs::read_dir(root) else { continue };
        for e in rd.flatten().take(512) {
            if !e.file_name().to_string_lossy().starts_with("com_apple_MobileAsset_Font") {
                continue;
            }
            let Ok(assets) = std::fs::read_dir(e.path()) else { continue };
            for a in assets.flatten().take(4096) {
                let p = a.path().join("AssetData/PingFang.ttc");
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Maximum collection faces and cumulative `name` records inspected by
/// [`face_index_for_family_checked`].
const MAX_COLLECTION_FACES: usize = 256;
const MAX_NAME_RECORDS: usize = 8192;

/// Whether `font` has a family record exactly equal to `family`, charging each inspected record
/// to `budget` so malformed fonts cannot make this scan unbounded.
fn family_name_matches(font: &skrifa::FontRef<'_>, family: &str, budget: &mut usize) -> bool {
    use skrifa::{MetadataProvider, string::StringId};

    for id in [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME] {
        for s in font.localized_strings(id) {
            let Some(rem) = budget.checked_sub(1) else {
                return false;
            };
            *budget = rem;
            if s.chars().eq(family.chars()) {
                return true;
            }
        }
    }
    false
}

/// Whether every table in the selected face of `bytes` really is present in the file.
///
/// `FontRef` reads tables lazily: a face whose `cmap` and `name` are valid can still point another
/// table outside the file. Coverage checks only read `cmap`, so callers that register a selected
/// collection face (by family index, coverage result or face 0) must check this first; epaint
/// panics when it installs a configured face the parser cannot read.
fn face_tables_are_readable(font: &skrifa::raw::FontRef<'_>) -> bool {
    let directory = font.table_directory();
    let records = directory.table_records();
    !records.is_empty() && records.len() == usize::from(directory.num_tables()) && records.iter().all(|record| font.table_data(record.tag()).is_some())
}

/// Whether face `index` of a font file or collection has a complete, in-bounds table directory.
///
/// This is the selected-index counterpart of [`font_file_is_readable`]. A readable collection may
/// still contain a malformed face at `index`; the face's `cmap` and `name` can work while another
/// table points outside the file. Returns `false` for an out-of-range index or malformed data.
/// Never panics.
pub fn font_face_is_readable(bytes: &[u8], index: u32) -> bool {
    skrifa::raw::FontRef::from_index(bytes, index).is_ok_and(|font| face_tables_are_readable(&font))
}

/// Whether `bytes` is a font file or font collection that the text engine's parser can read.
///
/// This uses the same `skrifa` parser the text engine lays out and rasterizes with, instead of a
/// hand-written table-directory check with invented face/table caps. A readable non-font or
/// truncated file returns `false`; valid SFNT, OTF and TTC data returns `true`. For a collection,
/// at least one face must have a complete table directory. That does **not** make every face
/// readable: a face selected by family name, coverage or the empty-family face-0 contract must be
/// checked with [`font_face_is_readable`], because its `cmap` and `name` can work while another
/// table points out of bounds.
pub fn font_file_is_readable(bytes: &[u8]) -> bool {
    use skrifa::raw::FileRef;

    match FileRef::new(bytes) {
        Ok(FileRef::Font(font)) => face_tables_are_readable(&font),
        Ok(FileRef::Collection(faces)) => faces.iter().any(|face| face.is_ok_and(|font| face_tables_are_readable(&font))),
        Err(_) => false,
    }
}

/// Face index in a font file (or collection) whose family name is `family`.
///
/// An empty `family` is the explicit "face 0" contract. A single-face file must actually carry
/// the requested family name; a collection must contain a face with it. `None` when the family is
/// missing, the data is malformed or the name budget is exhausted. The returned index has a
/// matching `name`, but its other tables are not validated: use [`font_face_is_readable`] before
/// registering it. Never panics.
pub fn face_index_for_family_checked(bytes: &[u8], family: &str) -> Option<u32> {
    use skrifa::raw::{FileRef, types::Tag};

    if family.is_empty() {
        return Some(0);
    }
    let mut budget = MAX_NAME_RECORDS;
    match FileRef::new(bytes).ok()? {
        FileRef::Font(font) => family_name_matches(&font, family, &mut budget).then_some(0),
        FileRef::Collection(c) => {
            let mut seen_faces = std::collections::HashSet::new();
            let mut seen_names = std::collections::HashSet::new();
            for (i, f) in c.iter().take(MAX_COLLECTION_FACES).enumerate() {
                if let Some(off_bytes) = bytes.get(12 + i * 4..12 + (i + 1) * 4)
                    && let Ok(arr) = <[u8; 4]>::try_from(off_bytes)
                    && !seen_faces.insert(u32::from_be_bytes(arr))
                {
                    continue;
                }
                let Ok(f) = f else { continue };
                if let Some(rec) = f.table_directory.table_records().iter().find(|r| r.tag() == Tag::new(b"name"))
                    && !seen_names.insert((rec.offset(), rec.length()))
                {
                    continue;
                }
                if family_name_matches(&f, family, &mut budget) {
                    return u32::try_from(i).ok();
                }
                if budget == 0 {
                    return None;
                }
            }
            None
        }
    }
}

/// Face index in a font file (or collection) whose family name is `family`; 0 when `family` is
/// empty or no face matches. Kept for callers that prefer the historical face-0 fallback; new
/// system-font selection should use [`face_index_for_family_checked`], verify the selected face
/// with [`font_face_is_readable`], and then verify coverage. Never panics on malformed data.
pub fn face_index_for_family(bytes: &[u8], family: &str) -> u32 {
    face_index_for_family_checked(bytes, family).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use CjkScript::*;

    #[test]
    fn explicit_locale_scripts_are_distinct_from_the_non_cjk_default() {
        assert_eq!(locale_script(Some("ja_JP.UTF-8")), Some(Japanese));
        assert_eq!(locale_script(Some("ko-KR")), Some(Korean));
        assert_eq!(locale_script(Some("zh-Hans-CN")), Some(SimplifiedChinese));
        assert_eq!(locale_script(Some("zh_CN")), Some(SimplifiedChinese));
        assert_eq!(locale_script(Some("zh-Hant-TW")), Some(TraditionalChinese));
        assert_eq!(locale_script(Some("yue-HK")), Some(TraditionalChinese));
        for unknown in [None, Some(""), Some("en_US.UTF-8"), Some("C"), Some("de"), Some("!!@@")] {
            assert_eq!(locale_script(unknown), None, "{unknown:?}");
        }
    }

    #[test]
    fn locale_orders() {
        assert_eq!(script_order(Some("ja_JP.UTF-8"))[0], Japanese);
        assert_eq!(script_order(Some("ja"))[0], Japanese);
        assert_eq!(script_order(Some("zh_CN.UTF-8"))[0], SimplifiedChinese);
        assert_eq!(script_order(Some("zh-Hans-CN"))[0], SimplifiedChinese);
        assert_eq!(script_order(Some("zh-Hant-TW"))[0], TraditionalChinese);
        assert_eq!(script_order(Some("zh_TW"))[0], TraditionalChinese);
        assert_eq!(script_order(Some("zh-HK"))[0], TraditionalChinese);
        assert_eq!(script_order(Some("zh-Hans-HK"))[0], SimplifiedChinese);
        assert_eq!(script_order(Some("ko-KR"))[0], Korean);
        // Non-CJK and unknown: Japanese forms are never first.
        for l in [None, Some(""), Some("en_US.UTF-8"), Some("C"), Some("de"), Some("!!@@")] {
            let o = script_order(l);
            assert_eq!(o[0], SimplifiedChinese);
            assert_ne!(o[0], Japanese);
        }
        // Every order is a permutation of all four scripts.
        for l in ["ja", "ko", "zh-TW", "zh", "en"] {
            let o = script_order(Some(l));
            for s in [Japanese, SimplifiedChinese, TraditionalChinese, Korean] {
                assert!(o.contains(&s), "{l}: {o:?}");
            }
        }
    }

    #[test]
    fn type_tool_fallback_follows_locale() {
        let pos = |v: &[&str], f: &str| v.iter().position(|x| *x == f).unwrap();
        let ja = crate::fonts::fallback_candidates(&script_order(Some("ja_JP")));
        assert!(pos(&ja, "Hiragino Sans") < pos(&ja, "PingFang SC"));
        assert!(pos(&ja, "Noto Sans CJK JP") < pos(&ja, "Noto Sans CJK SC"));
        let sc = crate::fonts::fallback_candidates(&script_order(Some("zh_CN")));
        assert!(pos(&sc, "PingFang SC") < pos(&sc, "Hiragino Sans"));
        assert!(pos(&sc, "Microsoft YaHei") < pos(&sc, "Microsoft JhengHei"));
        let tc = crate::fonts::fallback_candidates(&script_order(Some("zh-Hant")));
        assert!(pos(&tc, "PingFang TC") < pos(&tc, "PingFang SC"));
        let ko = crate::fonts::fallback_candidates(&script_order(Some("ko")));
        assert!(pos(&ko, "Apple SD Gothic Neo") < pos(&ko, "PingFang SC"));
        assert!(pos(&ko, "Malgun Gothic") < pos(&ko, "Yu Gothic"));
        // Arial Unicode covers Han too: it must not pre-empt the script fonts.
        for v in [&ja, &sc, &tc, &ko] {
            assert!(pos(v, "Arial Unicode MS") > pos(v, "Noto Sans CJK KR"));
            assert!(pos(v, "Noto Sans") < pos(v, "Hiragino Sans"));
        }
    }

    #[test]
    fn classify_chars() {
        let order = script_order(Some("en"));
        assert_eq!(classify('a'), None);
        assert_eq!(classify('é'), None);
        assert_eq!(classify('图'), Some(CjkChar::Han));
        assert_eq!(classify('圖'), Some(CjkChar::Han));
        assert_eq!(classify('レ'), Some(CjkChar::Kana));
        assert_eq!(classify('ー'), Some(CjkChar::Kana));
        assert_eq!(classify('카'), Some(CjkChar::Hangul));
        assert_eq!(classify('ㄅ'), Some(CjkChar::Bopomofo));
        assert_eq!(CjkChar::Kana.preferred(&order), Japanese);
        assert_eq!(CjkChar::Hangul.preferred(&order), Korean);
        assert_eq!(CjkChar::Han.preferred(&order), SimplifiedChinese);
        assert_eq!(CjkChar::Han.preferred(&script_order(Some("ja"))), Japanese);
    }

    /// `plutil -convert binary1` of a dict with AKLastLocale, AppleLanguages
    /// [zh-Hant-TW, en-US] and AppleLocale.
    const SAMPLE: &[u8] = &[
        0x62, 0x70, 0x6c, 0x69, 0x73, 0x74, 0x30, 0x30, 0xd3, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x5b, 0x41, 0x70, 0x70, 0x6c, 0x65, 0x4c, 0x6f, 0x63, 0x61,
        0x6c, 0x65, 0x5c, 0x41, 0x4b, 0x4c, 0x61, 0x73, 0x74, 0x4c, 0x6f, 0x63, 0x61, 0x6c, 0x65, 0x5e, 0x41, 0x70, 0x70, 0x6c, 0x65, 0x4c, 0x61, 0x6e, 0x67,
        0x75, 0x61, 0x67, 0x65, 0x73, 0x55, 0x7a, 0x68, 0x5f, 0x54, 0x57, 0x55, 0x65, 0x6e, 0x5f, 0x55, 0x53, 0xa2, 0x07, 0x08, 0x5a, 0x7a, 0x68, 0x2d, 0x48,
        0x61, 0x6e, 0x74, 0x2d, 0x54, 0x57, 0x55, 0x65, 0x6e, 0x2d, 0x55, 0x53, 0x08, 0x0f, 0x1b, 0x28, 0x37, 0x3d, 0x43, 0x46, 0x51, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x57,
    ];

    #[test]
    fn reads_apple_languages_and_rejects_garbage() {
        assert_eq!(bplist_first_language(SAMPLE).as_deref(), Some("zh-Hant-TW"));
        assert_eq!(script_order(bplist_first_language(SAMPLE).as_deref())[0], TraditionalChinese);
        assert_eq!(bplist_first_language(b""), None);
        assert_eq!(bplist_first_language(b"bplist00"), None);
        assert_eq!(bplist_first_language(&[0xFF; 64]), None);
        // Every truncation and every single-byte corruption must fail gracefully, never panic.
        for n in 0..SAMPLE.len() {
            let _ = bplist_first_language(&SAMPLE[..n]);
        }
        for i in 0..SAMPLE.len() {
            for v in [0x00, 0x0F, 0x5F, 0xAF, 0xDF, 0xFF] {
                let mut b = SAMPLE.to_vec();
                b[i] = v;
                let _ = bplist_first_language(&b);
            }
        }
    }

    /// A two-face TrueType collection built at runtime from the repository's OFL-licensed
    /// Inter and JetBrains Mono files. Each face directory is rewritten with offsets relative to
    /// the collection, so no font fixture is committed.
    fn two_face_ttc(first: &[u8], second: &[u8]) -> Vec<u8> {
        let font_count = 2usize;
        let tables = |font: &[u8]| u16::from_be_bytes([font[4], font[5]]) as usize;
        let dir_lengths = [12 + 16 * tables(first), 12 + 16 * tables(second)];
        let mut dir_offsets = Vec::with_capacity(font_count);
        let mut cursor = 12 + 4 * font_count;
        for length in dir_lengths {
            dir_offsets.push(cursor);
            cursor += length;
        }
        let data_start = cursor.next_multiple_of(4);
        let mut bases = Vec::with_capacity(font_count);
        let mut total = data_start;
        for font in [first, second] {
            bases.push(total);
            total += font.len().next_multiple_of(4);
        }
        let mut out = vec![0u8; total];
        out[..4].copy_from_slice(b"ttcf");
        out[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        out[8..12].copy_from_slice(&u32::try_from(font_count).expect("two faces").to_be_bytes());
        for (i, offset) in dir_offsets.iter().enumerate() {
            let at = 12 + i * 4;
            out[at..at + 4].copy_from_slice(&u32::try_from(*offset).expect("collection fits").to_be_bytes());
        }
        for (i, font) in [first, second].into_iter().enumerate() {
            let dir = dir_offsets[i];
            let base = bases[i];
            out[dir..dir + font.len().min(12)].copy_from_slice(&font[..font.len().min(12)]);
            for record in 0..tables(font) {
                let src = 12 + 16 * record;
                let dst = dir + 12 + 16 * record;
                out[dst..dst + 8].copy_from_slice(&font[src..src + 8]);
                let table_offset = u32::from_be_bytes(font[src + 8..src + 12].try_into().expect("four bytes"));
                let collection_offset = u32::try_from(base).expect("collection fits") + table_offset;
                out[dst + 8..dst + 12].copy_from_slice(&collection_offset.to_be_bytes());
                out[dst + 12..dst + 16].copy_from_slice(&font[src + 12..src + 16]);
            }
            out[base..base + font.len()].copy_from_slice(font);
        }
        out
    }

    #[test]
    fn checked_face_lookup_uses_the_requested_collection_family_and_exact_coverage() {
        use crate::scripts::face_covers;

        let inter = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        let jetbrains = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
        let ttc = two_face_ttc(inter, jetbrains);

        assert_eq!(face_index_for_family_checked(&ttc, "Inter"), Some(0));
        assert_eq!(face_index_for_family_checked(&ttc, "JetBrains Mono"), Some(1));
        assert_eq!(face_index_for_family_checked(&ttc, "Noto Sans CJK TC"), None);
        assert_eq!(face_index_for_family(&ttc, "JetBrains Mono"), 1, "the old wrapper keeps its face-0 fallback only on a miss");
        assert_eq!(face_index_for_family_checked(inter, "Inter"), Some(0));
        assert_eq!(face_index_for_family_checked(inter, "JetBrains Mono"), None, "a single-face file must carry the requested family");
        assert_eq!(face_index_for_family_checked(inter, ""), Some(0), "empty family is the explicit face-0 contract");
        assert_eq!(face_index_for_family_checked(b"", "Inter"), None);

        // Both faces cover A. JetBrains Mono does not cover Latin Small Letter B with Stroke,
        // while Inter does, so a first-covering-face lookup cannot satisfy this pair.
        assert!(face_covers(&ttc, 0, 'A'));
        assert!(face_covers(&ttc, 1, 'A'));
        assert!(face_covers(&ttc, 0, 'ƀ'));
        assert!(!face_covers(&ttc, 1, 'ƀ'));
        assert!(!face_covers(&ttc, 1, '日'));
        assert!(!face_covers(&ttc, 2, 'A'), "out-of-range face index");
    }

    #[test]
    fn font_file_validation_uses_the_layout_parser() {
        let inter = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        let jetbrains = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
        assert!(font_file_is_readable(inter));
        assert!(font_file_is_readable(&two_face_ttc(inter, jetbrains)));
        assert!(!font_file_is_readable(b"this is not a font"));
        assert!(!font_file_is_readable(&inter[..20]));
        let mut missing_table = inter.to_vec();
        // The first directory record begins at 12; its offset occupies bytes 20..24.
        missing_table[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(!font_file_is_readable(&missing_table), "a complete directory cannot point outside the file");
    }

    /// A readable collection can contain a malformed non-selected face, so the file-level check
    /// must not stand in for validating the face a caller selects. The selected face keeps a valid
    /// `cmap` (coverage succeeds) and `name` (family lookup succeeds) while another table points
    /// past the end of the file.
    #[test]
    fn selected_face_validation_rejects_a_malformed_face_in_a_readable_collection() {
        use crate::scripts::face_covers;

        let inter = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        let jetbrains = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
        let mut malformed = jetbrains.to_vec();
        let tables = u16::from_be_bytes([malformed[4], malformed[5]]) as usize;
        let glyf = (0..tables).map(|i| 12 + 16 * i).find(|&rec| malformed.get(rec..rec + 4) == Some(&b"glyf"[..])).expect("JetBrains Mono has a glyf table");
        malformed[glyf + 12..glyf + 16].copy_from_slice(&u32::MAX.to_be_bytes());
        let ttc = two_face_ttc(inter, &malformed);

        assert!(font_file_is_readable(&ttc), "face 0 remains a readable face");
        assert_eq!(face_index_for_family_checked(&ttc, "JetBrains Mono"), Some(1), "the malformed face's name still resolves");
        assert!(face_covers(&ttc, 1, 'A'), "the malformed face's cmap still covers A");
        assert!(!font_face_is_readable(&ttc, 1), "its glyf table lies outside the collection");
        assert!(font_face_is_readable(&ttc, 0), "positive control: the good face stays readable");
        assert!(!font_face_is_readable(&ttc, 2), "out-of-range face");
        assert!(font_face_is_readable(&two_face_ttc(inter, jetbrains), 1), "positive control: a valid selected face passes");
    }

    #[test]
    fn face_index_handles_bad_data() {
        assert_eq!(face_index_for_family(b"", "PingFang SC"), 0);
        assert_eq!(face_index_for_family(b"ttcf\0\0\0\0garbage", "PingFang SC"), 0);
        let inter = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        assert_eq!(face_index_for_family(inter, "Inter"), 0);
        assert_eq!(face_index_for_family(inter, ""), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fontconfig_candidates_must_cover_the_requested_script() {
        let inter = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        for script in [Japanese, SimplifiedChinese, TraditionalChinese, Korean] {
            assert!(!fontconfig_font_covers_sample(inter, "Inter", script), "{script:?}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fontconfig_matches_cover_the_requested_script() {
        for script in [Japanese, SimplifiedChinese, TraditionalChinese, Korean] {
            let Some(file) = fontconfig_match(script) else { continue };
            let bytes = std::fs::read(&file.path).expect("Fontconfig match remains readable");
            assert!(fontconfig_font_covers_sample(&bytes, &file.family, script), "{script:?}: {}", file.path.display());
        }
    }

    /// On a Mac with PingFang, the SC and TC faces are found by name (not a hard-coded index).
    #[cfg(target_os = "macos")]
    #[test]
    fn pingfang_faces_by_name() {
        let Some(p) = mac_pingfang() else { return };
        let Ok(bytes) = std::fs::read(p) else { return };
        let sc = face_index_for_family(&bytes, "PingFang SC");
        let tc = face_index_for_family(&bytes, "PingFang TC");
        assert_ne!(sc, tc);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod flatpak_tests {
    use super::*;

    #[test]
    fn cjk_discovery_includes_host_fonts_with_the_matching_collection_face() {
        for script in [CjkScript::Japanese, CjkScript::SimplifiedChinese, CjkScript::TraditionalChinese, CjkScript::Korean] {
            let files = font_files(script);
            let native = &files[0];
            let host = &files[1];
            assert_eq!(native.path, std::path::Path::new("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"));
            assert_eq!(host.path, std::path::Path::new("/run/host/fonts/opentype/noto/NotoSansCJK-Regular.ttc"));
            assert_eq!(host.family, families(script).iter().find(|family| family.starts_with("Noto Sans CJK")).copied().unwrap());
            for pair in files.windows(2) {
                if let Ok(relative) = pair[0].path.strip_prefix("/usr/share/fonts") {
                    assert_eq!(pair[1].path, std::path::Path::new("/run/host/fonts").join(relative));
                    assert_eq!(pair[1].family, pair[0].family);
                }
            }
        }
    }

    #[test]
    fn last_resort_discovery_includes_the_host_droid_font() {
        let files = last_resort_files();
        assert!(
            files.iter().any(|file| file.path == std::path::Path::new("/run/host/fonts/truetype/droid/DroidSansFallbackFull.ttf") && file.family.is_empty())
        );
    }
}
