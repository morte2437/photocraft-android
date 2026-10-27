use super::*;

// ------------------------------------------------------------------ Find and Replace Text

/// Byte ranges of `find` in `text` (case-insensitive unless `case`; whole words only with `whole`).
pub fn find_matches(text: &str, find: &str, case: bool, whole: bool) -> Vec<(usize, usize)> {
    if find.is_empty() {
        return Vec::new();
    }
    let norm = |c: char| -> String { if case { c.to_string() } else { c.to_lowercase().collect() } };
    let needle: String = find.chars().map(norm).collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        // Compare char by char so case folding that changes byte lengths stays aligned.
        let mut hay = String::new();
        let mut end = at;
        for (i, c) in text[at..].char_indices() {
            if hay.len() >= needle.len() {
                break;
            }
            hay.push_str(&norm(c));
            end = at + i + c.len_utf8();
        }
        if hay == needle {
            let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            let before = text[..at].chars().next_back();
            let after = text[end..].chars().next();
            if !whole || (!word(before) && !word(after)) {
                out.push((at, end));
                at = end;
                continue;
            }
        }
        at += text[at..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

fn type_layers(doc: &Document, forward: bool) -> Vec<LayerId> {
    // Layers panel order: top first.
    let mut ids: Vec<LayerId> =
        doc.walk().into_iter().rev().filter(|(p, _, l)| matches!(l.content, LayerContent::Text(_)) && !doc.locks_at(p).all).map(|(_, _, l)| l.id).collect();
    if !forward {
        ids.reverse();
    }
    ids
}

pub(super) fn has_type_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if type_layers(&d.doc, true).is_empty() { Err("the document has no type layers".into()) } else { Ok(()) }
}

pub(super) fn find_replace(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "edit.findAndReplaceText";
    let find = p.get("find").and_then(Value::as_str).filter(|f| !f.is_empty()).ok_or_else(|| bad(cmd, "missing `find`"))?.to_string();
    let replace = str_or(p, "replace", "").to_string();
    let (case, whole, forward, all_layers) =
        (bool_or(p, "caseSensitive", false), bool_or(p, "wholeWord", false), bool_or(p, "forward", true), bool_or(p, "allLayers", true));
    let action = str_or(p, "action", "changeAll").to_string();
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let mut layers = type_layers(&st.doc, forward);
    if !all_layers {
        let active = layer_param(s, p).ok();
        layers.retain(|id| Some(*id) == active);
    }
    let text_of = |doc: &Document, id: LayerId| -> Option<String> {
        match &doc.layer(id)?.content {
            LayerContent::Text(t) => Some(t.text.clone()),
            _ => None,
        }
    };
    match action.as_str() {
        "changeAll" => {
            let doc = st.doc.clone();
            let plan: Vec<(LayerId, Vec<(usize, usize)>)> =
                layers.iter().filter_map(|id| Some((*id, find_matches(&text_of(&doc, *id)?, &find, case, whole)))).filter(|(_, m)| !m.is_empty()).collect();
            let count: usize = plan.iter().map(|(_, m)| m.len()).sum();
            if count == 0 {
                return Ok(json!({"count": 0, "layers": 0}));
            }
            s.edit("Replace All Text", |doc, _| {
                let (dpi, format) = (doc.resolution_dpi, doc.pixel_format());
                for (id, matches) in &plan {
                    if let Some(LayerContent::Text(t)) = doc.layer_mut(*id).map(|l| &mut l.content) {
                        for (a, b) in matches.iter().rev() {
                            crate::type_cmds::replace_text(t, *a, *b, &replace);
                        }
                        crate::type_cmds::refresh_at(dpi, format, t);
                    }
                }
                Ok(())
            })?;
            s.edit_state.find_cursor = None;
            Ok(json!({"count": count, "layers": plan.len()}))
        }
        "find" | "change" | "changeFind" => {
            let doc = st.doc.clone();
            let cursor = s.edit_state.find_cursor;
            let mut changed = None;
            if action != "find"
                && let Some((id, a, b)) = cursor
                && text_of(&doc, id).is_some_and(|t| find_matches(&t, &find, case, whole).contains(&(a, b)))
            {
                crate::type_cmds::check_text_unlocked(&doc, id)?;
                s.edit("Replace Text", |doc, _| {
                    let (dpi, format) = (doc.resolution_dpi, doc.pixel_format());
                    if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
                        crate::type_cmds::replace_text(t, a, b, &replace);
                        crate::type_cmds::refresh_at(dpi, format, t);
                    }
                    Ok(())
                })?;
                let end = a + replace.len();
                // Collapse the cursor on the side the search moves away from, so the next search
                // skips the replaced text even when the replacement still matches.
                let c = if forward { end } else { a };
                s.edit_state.find_cursor = Some((id, c, c));
                changed = Some((id, a, end));
                if action == "change" {
                    return Ok(json!({"changed": {"layer": id.0, "start": a, "end": end}}));
                }
            }
            // Find the next match after the cursor (wrapping through the layers).
            let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
            let cursor = s.edit_state.find_cursor;
            let start = cursor.and_then(|(id, _, _)| layers.iter().position(|l| *l == id)).unwrap_or(0);
            for k in 0..=layers.len() {
                // `allLayers: false` with a non-type layer active leaves no layers to search (#703).
                let Some(&id) = layers.get((start + k) % layers.len().max(1)) else { break };
                let Some(text) = text_of(&doc, id) else { continue };
                let ms = find_matches(&text, &find, case, whole);
                let hit = match (k, cursor) {
                    (0, Some((cid, a, b))) if cid == id => {
                        if forward {
                            ms.into_iter().find(|m| m.0 >= b)
                        } else {
                            ms.into_iter().rev().find(|m| m.1 <= a)
                        }
                    }
                    _ => {
                        if forward {
                            ms.into_iter().next()
                        } else {
                            ms.into_iter().next_back()
                        }
                    }
                };
                if let Some((a, b)) = hit {
                    s.edit_state.find_cursor = Some((id, a, b));
                    let _ = s.select_layer(id);
                    return Ok(
                        json!({"found": {"layer": id.0, "start": a, "end": b, "text": &text[a..b]}, "changed": changed.map(|(l, a, b)| json!({"layer": l.0, "start": a, "end": b}))}),
                    );
                }
            }
            s.edit_state.find_cursor = None;
            Ok(json!({"found": null, "changed": changed.map(|(l, a, b)| json!({"layer": l.0, "start": a, "end": b}))}))
        }
        other => Err(bad(cmd, format!("unknown action `{other}` (find|change|changeFind|changeAll)"))),
    }
}
