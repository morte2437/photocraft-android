use super::{Result, Session, Value, json};

// ------------------------------------------------------------------ Purge

fn history_bytes(s: &Session, all: bool) -> usize {
    let docs: Vec<&crate::DocState> = if all { s.documents().iter().collect() } else { s.active().into_iter().collect() };
    // Purge keeps the snapshots, so the tiles they share with the states aren't freed.
    docs.iter()
        .map(|d| {
            let snaps: Vec<&photocraft_doc::Document> = d.snapshots.iter().map(|s| &*s.doc).collect();
            d.history.unique_bytes_besides(&d.doc, &snaps)
        })
        .sum()
}

fn clip_bytes(s: &Session) -> usize {
    s.clipboard.as_ref().map_or(0, |c| c.surface.tiles().map(|(_, t)| t.bytes().len()).sum())
}

pub(super) fn purge(s: &mut Session, what: &str) -> Result<Value> {
    let mut freed = 0usize;
    let mut items: Vec<&str> = Vec::new();
    if matches!(what, "undo" | "all") && s.active().is_some_and(|d| d.history.can_undo()) {
        let before = history_bytes(s, false);
        if let Some(st) = s.active_mut() {
            st.history.purge_last();
        }
        freed += before.saturating_sub(history_bytes(s, false));
        items.push("undo");
    }
    if matches!(what, "clipboard" | "all") && (s.clipboard.is_some() || s.style_clipboard.is_some()) {
        freed += clip_bytes(s);
        s.clipboard = None;
        s.style_clipboard = None;
        items.push("clipboard");
    }
    if matches!(what, "histories" | "all") {
        let b = history_bytes(s, true);
        if s.documents().iter().any(|d| d.history.can_undo() || d.history.can_redo()) {
            for st in &mut s.docs {
                st.history.clear();
                st.coalesce = None;
            }
            freed += b;
            items.push("histories");
        }
    }
    if what == "all" {
        let fx = photocraft_compose::purge_effect_cache();
        if fx > 0 {
            freed += fx;
            items.push("effect cache");
        }
        let masks = photocraft_compose::masks::purge_cache();
        if masks > 0 {
            freed = freed.saturating_add(masks);
            items.push("mask cache");
        }
        let stages = crate::smart_cmds::purge_stack_cache();
        if stages > 0 {
            freed = freed.saturating_add(stages);
            items.push("smart filter cache");
        }
        let sources = crate::smart_cmds::purge_source_cache();
        if sources > 0 {
            freed = freed.saturating_add(sources);
            items.push("smart object source cache");
        }
        let strokes = photocraft_compose::shape_split::purge_cache();
        if strokes > 0 {
            freed = freed.saturating_add(strokes);
            items.push("shape stroke cache");
        }
    }
    s.edit_state.fade = None;
    let msg = if items.is_empty() { "nothing to purge".to_string() } else { format!("purged {}", items.join(", ")) };
    Ok(json!({"purged": items, "bytes": freed, "message": msg}))
}

pub(super) fn can_purge_undo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.history.can_undo()).map(|_| ()).ok_or_else(|| "nothing to purge".into())
}

pub(super) fn can_purge_clipboard(s: &Session) -> std::result::Result<(), String> {
    if s.clipboard.is_some() || s.style_clipboard.is_some() { Ok(()) } else { Err("the clipboard is empty".into()) }
}

pub(super) fn can_purge_histories(s: &Session) -> std::result::Result<(), String> {
    if s.documents().iter().any(|d| d.history.can_undo() || d.history.can_redo()) { Ok(()) } else { Err("no history to purge".into()) }
}

pub(super) fn can_purge_all(s: &Session) -> std::result::Result<(), String> {
    can_purge_histories(s).or_else(|_| can_purge_clipboard(s)).or_else(|_| {
        if photocraft_compose::effect_cache_bytes() > 0
            || photocraft_compose::masks::cache_bytes() > 0
            || crate::smart_cmds::stack_cache_bytes() > 0
            || crate::smart_cmds::source_cache_bytes() > 0
        {
            Ok(())
        } else {
            Err("nothing to purge".into())
        }
    })
}
