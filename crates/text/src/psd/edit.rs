use super::build::{MAX_TEXT_INDEX, utf16_len};
use super::import::parse_txt2;
use super::{D, E, Id, parse_tysh, write_tysh};

/// The most values a `Txt2` object may hold to serve as the template for new objects. Photoshop's
/// own objects hold a few hundred (611 at most in the file from #7698).
pub(super) const TEMPLATE_NODES: usize = 4096;

/// Cumulative node and byte budgets across all run dictionaries cloned while refitting one `Txt2`
/// object (`runs_per` and `runs_to_length`), so an edited object with a large preserved run and
/// many output paragraphs falls back to a fresh object instead of amplifying memory.
const MAX_REFIT_RUN_NODES: usize = 16_384;

const MAX_REFIT_RUN_BYTES: usize = 1 << 20;

/// Whether `v` holds at most `budget` values, counting down `budget` (stops at the first one over).
pub(super) fn fits(v: &E, budget: &mut usize) -> bool {
    let Some(left) = budget.checked_sub(1) else { return false };
    *budget = left;
    match v {
        E::Dict(items) => items.iter().all(|(_, x)| fits(x, budget)),
        E::Array(items) => items.iter().all(|x| fits(x, budget)),
        _ => true,
    }
}

/// Whether `v` fits within the remaining `(nodes, bytes)` budget, deducting its nodes and string
/// bytes as it walks.
fn fits_budget(v: &E, nodes: &mut usize, bytes: &mut usize) -> bool {
    let Some(left) = nodes.checked_sub(1) else { return false };
    *nodes = left;
    match v {
        E::Dict(items) => items.iter().all(|(k, x)| {
            let Some(rem) = bytes.checked_sub(k.len()) else { return false };
            *bytes = rem;
            fits_budget(x, nodes, bytes)
        }),
        E::Array(items) => items.iter().all(|x| fits_budget(x, nodes, bytes)),
        E::String(s) | E::Name(s) => {
            let Some(rem) = bytes.checked_sub(s.len()) else { return false };
            *bytes = rem;
            true
        }
        _ => true,
    }
}

/// Whether the text object `object` has a view naming one frame (`/1 /0 [ << /0 n >> ]`), as
/// Photoshop writes it.
pub(super) fn names_one_frame(object: &E) -> bool {
    object.path(&["1", "0"]).and_then(E::as_array).is_some_and(|f| f.len() == 1 && f.first().and_then(|x| x.get("0")).is_some())
}

/// A text object for the slot `at`, which the file's block has none for: the file's object
/// `template` refitted to `text` by [`edited_object`], its view now naming frame `at` and its view
/// runs (`/1 /1 /0`, one per paragraph) remade for the new paragraphs. `None` when the template
/// can't be refitted.
pub(super) fn new_object(template: &E, at: usize, text: &str) -> Option<E> {
    let paragraphs: Vec<usize> = text.split_inclusive('\r').map(|p| p.encode_utf16().count()).collect();
    let frame = template.path(&["1", "0"]).and_then(E::as_array).and_then(|f| f.first()).and_then(|f| f.get("0")).cloned();
    let at = i64::try_from(at).ok()?;
    let mut object = edited_object(template, text)?;
    let view = object.get_mut("1")?;
    view.set("0", E::Array(vec![E::Dict(vec![("0".into(), E::Int(at))])]));
    // The rendered strikes' frame strike (`/F`) names the template's frame too.
    if let Some(E::Array(strikes)) = view.get_mut("2") {
        for s in strikes.iter_mut() {
            let Some(E::Array(parts)) = s.get_mut("6") else { continue };
            for p in parts.iter_mut().filter(|p| p.get("99").and_then(E::as_str) == Some("F") && p.get("10") == frame.as_ref()) {
                p.set("10", E::Int(at));
            }
        }
    }
    if let Some(runs) = view.path(&["1", "0"]).and_then(E::as_array) {
        let (mut nodes, mut bytes) = (MAX_REFIT_RUN_NODES, MAX_REFIT_RUN_BYTES);
        let fitted = runs_per(runs, &paragraphs, "1", &mut nodes, &mut bytes)?;
        view.get_mut("1")?.set("0", E::Array(fitted));
    }
    Some(object)
}

/// Whether `object` is the bare `<< /0 << /0 text /6 runs >> >>` object [`build_txt2`] writes
/// when it has nothing to take a shape from: no view, no paragraph runs.
pub(super) fn is_stub(object: &E) -> bool {
    let only = |v: &E, keys: &[&str]| matches!(v, E::Dict(items) if items.iter().all(|(k, _)| keys.contains(&k.as_str())));
    only(object, &["0"]) && object.get("0").is_some_and(|m| only(m, &["0", "6"]))
}

/// Gives the block `outer` a frame (`/0 /8 /0`) for each of its `count` text objects: Photoshop
/// writes one per object, and a reader indexes them by object number. Missing frames are the
/// small point-text frame Photoshop writes (`<< /0 << /2 << /6 [ -1.0 -1.0 ] /11 << /4 -1 /7
/// false >> >> >> >>`), never copies of a file's frame: the slot count reaches
/// [`MAX_TEXT_INDEX`], and a large frame copied that often would blow up the save. A block
/// without a frame list is left as it is.
pub(super) fn pad_frames(outer: &mut E, count: usize) {
    let Some(E::Array(frames)) = outer.get_mut("0").and_then(|v| v.get_mut("8")).and_then(|v| v.get_mut("0")) else { return };
    let count = count.min(MAX_TEXT_INDEX as usize + 1);
    if frames.len() < count {
        frames.resize(count, crate::psd_path::point_frame());
    }
}

/// The file's `Txt2` text object `object` (`<< /0 model /1 view … >>`) for the changed engine
/// text `text` (`\r` breaks, trailing `\r`): the model's text replaced, its paragraph runs (`/5`)
/// one per paragraph of the new text (each keeping the style of the file's paragraph at that
/// position, else its last), and every other run array of the model (style runs `/6`, …) cut or
/// extended at its end to the new length. The view and the other keys are kept, except the glyph
/// pen positions (`/21`), which belong to the old text. `None` when the object or one of its run
/// arrays isn't shaped as Photoshop writes it: the caller writes a fresh object instead.
pub(super) fn edited_object(object: &E, text: &str) -> Option<E> {
    let mut object = object.clone();
    let E::Dict(items) = &mut object else { return None };
    items.retain(|(k, _)| k != "21");
    let model = object.get_mut("0").filter(|m| matches!(m, E::Dict(_)))?;
    let total = utf16_len(text) as usize;
    let paragraphs: Vec<usize> = text.split_inclusive('\r').map(|p| p.encode_utf16().count()).collect();
    let E::Dict(entries) = model else { return None };
    let (mut nodes, mut bytes) = (MAX_REFIT_RUN_NODES, MAX_REFIT_RUN_BYTES);
    for (key, value) in entries.iter_mut() {
        if key.as_str() == "0" {
            *value = E::String(text.to_string());
            continue;
        }
        let Some(runs) = value.get("0").and_then(E::as_array).filter(|r| r.iter().all(|x| matches!(x, E::Dict(_)) && x.get("1").is_some())) else {
            continue;
        };
        let fitted = if key.as_str() == "5" {
            runs_per(runs, &paragraphs, "1", &mut nodes, &mut bytes)?
        } else {
            runs_to_length(runs, total, "1", &mut nodes, &mut bytes)?
        };
        value.set("0", E::Array(fitted));
    }
    Some(object)
}

/// The older Photoshop's `Txt2` text object `object` (`<< /Model << /Text … /ParagraphRun … >>
/// /View … >>`) for the changed engine text `text` (`\r` breaks, trailing `\r`): the model's text
/// replaced and every run array of the model refitted the way [`edited_object`] refits the
/// numbered objects' — the paragraph runs (`/ParagraphRun`) one per paragraph of the new text, the
/// others (`/StyleRun`, `/KernRun`, …) cut or extended to the new total. The model's other keys,
/// the view and the rest are kept: these blocks (the objects at `/DocumentObjects /TextObjects`
/// that older Photoshop versions write and re-render from, #6971) name their keys and number
/// their runs with `/Length` in UTF-16 units instead of `/1`. `None` when the object or one of
/// its run arrays isn't shaped as Photoshop writes it: the caller then leaves the entry as the
/// file has it, rather than write text its runs don't cover (#6968).
pub(super) fn td_edited_object(object: &E, text: &str) -> Option<E> {
    let mut object = object.clone();
    let model = object.get_mut("Model").filter(|m| matches!(m, E::Dict(_)))?;
    let E::Dict(entries) = model else { return None };
    // The text must be there to replace: refitted runs over a text the model doesn't hold would
    // be a half-edited object.
    if !entries.iter().any(|(k, v)| k == "Text" && matches!(v, E::String(_))) {
        return None;
    }
    let total = utf16_len(text) as usize;
    let paragraphs: Vec<usize> = text.split_inclusive('\r').map(|p| p.encode_utf16().count()).collect();
    let (mut nodes, mut bytes) = (MAX_REFIT_RUN_NODES, MAX_REFIT_RUN_BYTES);
    for (key, value) in entries.iter_mut() {
        if key.as_str() == "Text" {
            *value = E::String(text.to_string());
            continue;
        }
        if !matches!(value, E::Dict(_)) {
            continue;
        }
        // A model dict without `RunArray` isn't a run array (the view's keys live elsewhere), so
        // it is skipped; a named `RunArray` is unambiguous — malformed, the whole entry is left to
        // the file rather than half-updated.
        let Some(runs) = value.get("RunArray") else { continue };
        let fitted = match runs {
            E::Array(runs) if runs.iter().all(|x| matches!(x, E::Dict(_)) && x.get("Length").is_some()) => {
                if key.as_str() == "ParagraphRun" {
                    runs_per(runs, &paragraphs, "Length", &mut nodes, &mut bytes)?
                } else {
                    runs_to_length(runs, total, "Length", &mut nodes, &mut bytes)?
                }
            }
            _ => return None,
        };
        value.set("RunArray", E::Array(fitted));
    }
    Some(object)
}

/// `runs` (`<< … /1 length >>`, the length key named `len_key`) remade with the lengths `lens`:
/// run i keeps the content of the file's run i, else of its last one. `None` for an empty `runs`
/// or when the cumulative `nodes` / `bytes` budget is exceeded.
fn runs_per(runs: &[E], lens: &[usize], len_key: &str, nodes: &mut usize, bytes: &mut usize) -> Option<Vec<E>> {
    let last = runs.last()?;
    lens.iter()
        .enumerate()
        .map(|(i, &len)| {
            let src = runs.get(i).unwrap_or(last);
            if !fits_budget(src, nodes, bytes) {
                return None;
            }
            let mut r = src.clone();
            r.set(len_key, E::Int(i64::try_from(len).ok()?));
            Some(r)
        })
        .collect()
}

/// `runs` (`<< … /1 length >>`, the length key named `len_key`) covering `total` UTF-16 units:
/// the leading runs as they were, cut where `total` ends, the last kept run extended when `total`
/// is longer. Empty runs are dropped. `None` when `runs` is empty, a length isn't a count, or the
/// cumulative `nodes` / `bytes` budget is exceeded.
fn runs_to_length(runs: &[E], total: usize, len_key: &str, nodes: &mut usize, bytes: &mut usize) -> Option<Vec<E>> {
    let mut out: Vec<(E, usize)> = Vec::new();
    let mut left = total;
    for r in runs {
        let len = r.get(len_key).and_then(E::as_i64).and_then(|l| usize::try_from(l).ok())?;
        let take = len.min(left);
        if take > 0 {
            if !fits_budget(r, nodes, bytes) {
                return None;
            }
            out.push((r.clone(), take));
            left -= take;
        }
    }
    if left > 0 {
        match out.last_mut() {
            Some(last) => last.1 = last.1.checked_add(left)?,
            None => {
                let last = runs.last()?;
                if !fits_budget(last, nodes, bytes) {
                    return None;
                }
                out.push((last.clone(), left));
            }
        }
    }
    out.into_iter()
        .map(|(mut r, len)| {
            r.set(len_key, E::Int(i64::try_from(len).ok()?));
            Some(r)
        })
        .collect()
}

/// The file's own `Txt2` style runs `runs` with the auto-kern modes `modes` (`(mode, UTF-16
/// length)`, see [`txt2_mode_runs`]) set at `/0 /0 /6 /11`, keeping every other style key. A
/// run a mode change falls inside is split there. `None` when the runs don't cover the text or
/// lack a style dictionary: the caller writes its own runs instead.
pub(super) fn with_modes(runs: &[E], modes: &[(i64, usize)]) -> Option<Vec<E>> {
    let lens: Vec<usize> = runs.iter().map(|r| r.get("1").and_then(E::as_i64).and_then(|l| usize::try_from(l).ok())).collect::<Option<_>>()?;
    let total = modes.iter().try_fold(0usize, |a, m| a.checked_add(m.1))?;
    if lens.iter().try_fold(0usize, |a, l| a.checked_add(*l))? != total {
        return None;
    }
    let mut out = Vec::with_capacity(runs.len());
    let (mut mi, mut mode_end) = (0usize, modes.first()?.1);
    let mut at = 0usize;
    for (run, len) in runs.iter().zip(lens) {
        if len == 0 {
            out.push(run.clone());
            continue;
        }
        let end = at + len;
        while at < end {
            while at >= mode_end && mi + 1 < modes.len() {
                mi += 1;
                mode_end = mode_end.saturating_add(modes.get(mi)?.1);
            }
            let piece_end = end.min(mode_end);
            if piece_end <= at {
                return None;
            }
            let mut piece = run.clone();
            if piece_end - at != len {
                piece.set("1", E::Int(i64::try_from(piece_end - at).ok()?));
            }
            let style = piece.get_mut("0").and_then(|v| v.get_mut("0")).and_then(|v| v.get_mut("6")).filter(|v| matches!(v, E::Dict(_)))?;
            let mode = modes.get(mi)?.0;
            // Keep the file's value (and its number type) when the mode is the same.
            if style.get("11").and_then(E::as_i64) != Some(mode) {
                style.set("11", E::Int(mode));
            }
            out.push(piece);
            at = piece_end;
        }
    }
    Some(out)
}

/// The layer's text-object number in the document's `Txt2` block, when its `TySh` names one
/// within `0..=`[`MAX_TEXT_INDEX`]. PSD export keeps a preserved number — its regenerated `Txt2`
/// object sits at that slot, and [`apply_txt2`] looks the slot up — and numbers layers without
/// one (or with a file-controlled out-of-range one) afresh.
pub fn text_index(tysh: &[u8]) -> Option<i32> {
    let t = parse_tysh(tysh)?;
    match t.text.get("TextIndex") {
        Some(D::Integer(i)) if (0..=MAX_TEXT_INDEX).contains(i) => Some(*i),
        _ => None,
    }
}

/// How many text objects the document `Txt2` block `data` holds (`/1 /1`), named by a layer or
/// not; 0 when it doesn't parse. PSD export numbers new type layers past them, so a new layer
/// never lands on a deleted layer's object.
pub fn txt2_object_count(data: &[u8]) -> usize {
    parse_txt2(data).and_then(|t| t.path(&["1", "1"]).and_then(E::as_array).map(<[E]>::len)).unwrap_or(0)
}

/// Rewrites a `TySh`'s `TextIndex`: the layer's text-object number in the document's `Txt2`
/// block ([`build_txt2`] writes them in this order). `None` when `tysh` doesn't parse.
pub fn set_text_index(tysh: &[u8], index: i32) -> Option<Vec<u8>> {
    let mut t = parse_tysh(tysh)?;
    match t.text.items.iter_mut().find(|(k, _)| k.is("TextIndex")) {
        Some(e) => e.1 = D::Integer(index),
        // Where Photoshop writes it: right before `EngineData`.
        None => {
            let at = t.text.items.iter().position(|(k, _)| k.is("EngineData")).unwrap_or(t.text.items.len());
            t.text.items.insert(at, (Id::new("TextIndex"), D::Integer(index)));
        }
    }
    Some(write_tysh(&t))
}
