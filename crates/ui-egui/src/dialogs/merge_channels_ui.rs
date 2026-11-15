//! Channels panel › Merge Channels…: the mode (RGB, CMYK or Lab Color) and, for each of its
//! channels, the open grayscale document that becomes it. Photoshop asks in two steps (the mode,
//! then the documents); here both are one dialog. Only the modes the open documents can fill are
//! offered (RGB and Lab need three, CMYK four), OK waits for a different document per channel,
//! and a merge that still fails opens the dialog again with the reason. OK runs the engine's
//! `channel.merge`, which closes the sources; agents call it directly with params (#11451).
//!
//! The dialog names documents by their [`photocraft_doc::DocId`], not their tab index: work that
//! runs while it is open (a script moving or closing a document) can shift the indices, and OK
//! must merge, and close, the documents that were picked. They are looked up again at OK.

use photocraft_doc::ColorMode;
use photocraft_engine::channel_cmds;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

pub const COMMAND: &str = "channel.merge";
const MARK: &str = "__mergeChannels";
/// The most channels a mode has (CMYK).
const SLOTS: usize = 4;
const MODES: [(&str, &str); 3] = [("rgb", "RGB Color"), ("cmyk", "CMYK Color"), ("lab", "Lab Color")];
const TOO_FEW: &str = "Not enough grayscale documents for this mode.";
const REPEATED: &str = "Pick a different document for each channel.";
const CLOSED: &str = "A picked document is no longer open.";
const CHANGED: &str = "A picked document is no longer a grayscale document of the same size.";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

fn mode_of(key: &str) -> ColorMode {
    match key {
        "cmyk" => ColorMode::Cmyk,
        "lab" => ColorMode::Lab,
        _ => ColorMode::Rgb,
    }
}

/// The modes `count` documents can fill, in the menu's order.
fn modes(count: usize) -> Vec<(String, &'static str)> {
    MODES.iter().filter(|(key, _)| mode_of(key).color_channels() <= count).map(|(key, label)| (key.to_string(), *label)).collect()
}

/// The documents that can be merged, by document id and name: the open grayscale ones of the
/// active document's size, the active one first, as the engine picks them without params.
fn sources(app: &PhotocraftApp) -> Vec<(u64, String)> {
    let Some(active) = app.session.active() else { return Vec::new() };
    let docs = app.session.documents();
    let fits = |i: usize| docs.get(i).filter(|d| d.doc.mode == ColorMode::Grayscale && d.doc.size == active.doc.size);
    let current = app.session.active_index();
    let others = (0..docs.len()).filter(|i| Some(*i) != current);
    current.into_iter().chain(others).filter_map(|i| fits(i).map(|d| (d.doc.id.0, d.doc.name.clone()))).collect()
}

fn mode_field(f: &Map<String, Value>) -> &str {
    f.get("mode").and_then(Value::as_str).unwrap_or("rgb")
}

fn sources_field(f: &Map<String, Value>) -> Vec<(u64, String)> {
    f.get("__sources").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

/// The picked document ids, one per channel of the mode.
fn picks(f: &Map<String, Value>) -> Vec<u64> {
    let all: Vec<u64> = f.get("documents").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    all.into_iter().take(mode_of(mode_field(f)).color_channels()).collect()
}

/// The dialog's fields: the first mode the documents can fill (RGB Color from three) and the
/// merge-able documents in order, one per channel, or `None` without a document.
pub fn fields(app: &PhotocraftApp) -> Option<Map<String, Value>> {
    app.session.active()?;
    let sources = sources(app);
    let picks: Vec<u64> = sources.iter().take(SLOTS).map(|(id, _)| *id).collect();
    let mode = modes(sources.len()).first().map_or_else(|| "rgb".to_string(), |(key, _)| key.clone());
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("Merge Channels…"));
    f.insert("__sources".into(), json!(sources));
    f.insert("mode".into(), json!(mode));
    f.insert("documents".into(), json!(picks));
    Some(f)
}

/// Opens Merge Channels, or `None` without a document.
pub fn open(app: &mut PhotocraftApp) -> Option<u64> {
    let f = fields(app)?;
    Some(app.ui.open_dialog(crate::state::DialogKind::Command, f))
}

/// Why OK is greyed (English, for `tl!`): the mode needs more documents than are offered, or the
/// picks repeat a document or name one the dialog did not offer. `None` when OK can merge, and
/// for another dialog's fields.
pub fn problem(f: &Map<String, Value>) -> Option<&'static str> {
    if !owns(f) {
        return None;
    }
    let n = mode_of(mode_field(f)).color_channels();
    let sources = sources_field(f);
    let picks = picks(f);
    if sources.len() < n || picks.len() < n {
        return Some(TOO_FEW);
    }
    let mut seen = std::collections::HashSet::new();
    let offered = |p: &u64| sources.iter().any(|(id, _)| id == p);
    if picks.iter().any(|p| !seen.insert(*p) || !offered(p)) {
        return Some(REPEATED);
    }
    None
}

/// Whether OK is live: always for other dialogs, here while [`problem`] finds none.
pub fn can_confirm(f: &Map<String, Value>) -> bool {
    problem(f).is_none()
}

/// The picked documents' current tab indices, looked up by id; refused (English, for `tl!`)
/// when one was closed, or is no longer a grayscale document of the others' size.
fn resolve(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<Vec<usize>, &'static str> {
    if let Some(why) = problem(f) {
        return Err(why);
    }
    let docs = app.session.documents();
    let indices = picks(f).iter().map(|id| docs.iter().position(|d| d.doc.id.0 == *id).ok_or(CLOSED)).collect::<Result<Vec<_>, _>>()?;
    let first = indices.first().and_then(|&i| docs.get(i)).map(|d| d.doc.size);
    let fits = |i: &usize| docs.get(*i).is_some_and(|d| d.doc.mode == ColorMode::Grayscale && Some(d.doc.size) == first);
    if indices.iter().all(fits) { Ok(indices) } else { Err(CHANGED) }
}

/// Checked before the dialog closes (`dialogs::confirm_preflight`), so a refused OK keeps it
/// open with the reason under the pickers: too few or repeated documents, or a picked one that
/// was closed or changed while the dialog was open.
pub fn preflight(app: &mut PhotocraftApp, id: u64) -> Result<(), String> {
    let Some(f) = app.ui.dialogs.iter().find(|d| d.id == id).map(|d| &d.fields).filter(|f| owns(f)) else { return Ok(()) };
    let Err(why) = resolve(app, f) else { return Ok(()) };
    if let Some(d) = app.ui.dialog_mut(id) {
        d.fields.insert("__error".into(), json!(why));
    }
    Err(why.to_string())
}

/// The `channel.merge` params for the dialog's fields: the mode and the picked documents' current
/// indices, one per channel.
pub fn params(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let documents = resolve(app, f).map_err(str::to_string)?;
    Ok(json!({ "mode": mode_field(f), "documents": documents }))
}

/// Runs the merge. Should it still be refused, the dialog opens again with its picks and the
/// reason, rather than going away.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let result = params(app, f).and_then(|p| app.run(COMMAND, p));
    if let Err(e) = &result {
        let mut again = f.clone();
        again.insert("__error".into(), json!(e));
        app.ui.open_dialog(crate::state::DialogKind::Command, again);
    }
    result
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let sources = sources_field(f);
    let mut mode = mode_field(f).to_string();
    let modes = modes(sources.len());
    ui.horizontal(|ui| {
        ui.label(tl!("Mode:"));
        if crate::widgets::dropdown(ui, "merge-channels-mode", tl!("Mode:"), &mut mode, &modes, 160.0) {
            f.insert("mode".into(), json!(mode));
            f.remove("__error");
        }
    });
    crate::widgets::section_label(ui, tl!("Specify Channels:"));
    crate::widgets::hairline(ui);
    let options: Vec<(u64, &str)> = sources.iter().map(|(id, name)| (*id, name.as_str())).collect();
    let mut picks: Vec<u64> = f.get("documents").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let mut changed = false;
    for (k, channel) in channel_cmds::color_names(mode_of(&mode)).iter().enumerate() {
        let Some(pick) = picks.get_mut(k) else { break };
        ui.horizontal(|ui| {
            let caption = format!("{}:", tl!(channel));
            ui.add_sized([84.0, 22.0], egui::Label::new(&caption));
            changed |= crate::widgets::dropdown_literal(ui, &format!("merge-channels-{k}"), &caption, pick, &options, 200.0);
        });
    }
    if changed {
        f.insert("documents".into(), json!(picks));
        f.remove("__error");
    }
    // Why OK is greyed, or why the last OK was refused (the dialog's reasons are catalogued; the
    // engine's own are shown as they come).
    let why: Option<&str> = problem(f);
    let why = why.or_else(|| f.get("__error").and_then(Value::as_str)).map(|w| tl!(w).to_string());
    if let Some(why) = why {
        ui.label(egui::RichText::new(why).color(Tokens::get(ui.ctx()).danger).size(11.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(docs: usize) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for _ in 0..docs {
            app.run("file.new", json!({"width": 8, "height": 8, "mode": "grayscale"})).unwrap();
        }
        app
    }

    fn id(app: &PhotocraftApp, index: usize) -> u64 {
        app.session.documents()[index].doc.id.0
    }

    fn offered(f: &Map<String, Value>) -> Vec<String> {
        modes(sources_field(f).len()).into_iter().map(|(key, _)| key).collect()
    }

    /// The merged document's first pixel as 8-bit RGB.
    fn merged_pixel(app: &PhotocraftApp) -> [u8; 3] {
        let doc = &app.session.active().unwrap().doc;
        let px = photocraft_compose::render(doc, doc.bounds()).px[0];
        [px[0], px[1], px[2]].map(|v| (v * 255.0).round() as u8)
    }

    /// Three grayscale documents filled black, mid gray and white, then the active one is `active`.
    fn shaded(active: usize) -> PhotocraftApp {
        let mut app = app(3);
        for (i, gray) in ["#000000", "#808080", "#ffffff"].into_iter().enumerate() {
            app.session.set_active(i);
            app.run("edit.fill", json!({"color": gray})).unwrap();
        }
        app.session.set_active(active);
        app
    }

    #[test]
    fn fields_list_the_sources_by_id_and_params_take_their_indices() {
        let mut app = app(3);
        app.run("file.new", json!({"width": 9, "height": 8, "mode": "grayscale"})).unwrap();
        app.run("file.new", json!({"width": 8, "height": 8, "mode": "rgb"})).unwrap();
        app.session.set_active(2);
        let f = fields(&app).unwrap();
        let ids = [id(&app, 2), id(&app, 0), id(&app, 1)];
        assert_eq!(f["__sources"].as_array().unwrap().iter().map(|s| s[0].as_u64().unwrap()).collect::<Vec<_>>(), ids, "the active one first");
        assert_eq!((f["mode"].clone(), f["documents"].clone()), (json!("rgb"), json!(ids)), "one each, none repeated");
        assert_eq!(params(&app, &f).unwrap(), json!({"mode": "rgb", "documents": [2, 0, 1]}));
        assert!(fields(&PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())).is_none());
    }

    /// Three documents fill RGB and Lab, not CMYK; four fill CMYK too.
    #[test]
    fn only_the_modes_the_documents_fill_are_offered() {
        let f = fields(&app(3)).unwrap();
        assert_eq!(offered(&f), ["rgb", "lab"]);
        assert!(can_confirm(&f));
        let mut cmyk = f.clone();
        cmyk.insert("mode".into(), json!("cmyk"));
        assert_eq!(problem(&cmyk), Some(TOO_FEW), "CMYK from three documents (as automation could ask) greys OK");
        let app = app(4);
        let f = fields(&app).unwrap();
        assert_eq!(offered(&f), ["rgb", "cmyk", "lab"]);
        let mut cmyk = f.clone();
        cmyk.insert("mode".into(), json!("cmyk"));
        assert!(can_confirm(&cmyk));
        assert_eq!(params(&app, &cmyk).unwrap()["documents"], json!([3, 0, 1, 2]), "the active (last opened) one first");
    }

    /// A document picked for two channels greys OK with the reason; a refused OK keeps the dialog
    /// open and merges nothing.
    #[test]
    fn repeated_picks_grey_ok_and_keep_the_dialog_open() {
        let mut app = app(3);
        let (a, b) = (id(&app, 0), id(&app, 1));
        let id = open(&mut app).unwrap();
        let d = app.ui.dialog_mut(id).unwrap();
        d.fields.insert("documents".into(), json!([a, a, b]));
        assert_eq!(problem(&d.fields), Some(REPEATED));
        assert!(!can_confirm(&d.fields));
        d.fields.insert("documents".into(), json!([a, b, 999_999]));
        assert_eq!(problem(&d.fields), Some(REPEATED), "a document the dialog did not offer");
        assert!(crate::dialogs::confirm(&mut app, id).is_err());
        assert!(app.ui.dialogs.iter().any(|d| d.id == id), "the dialog stays open");
        assert_eq!(app.session.documents().len(), 3, "nothing merged or closed");
        assert!(can_confirm(&Map::new()), "other dialogs are not affected");
    }

    /// Documents moved while the dialog is open: OK still merges the ones picked, by id, into the
    /// channels picked for them.
    #[test]
    fn ok_merges_the_picked_documents_after_a_reorder() {
        let mut app = shaded(0);
        let (black, gray, white) = (id(&app, 0), id(&app, 1), id(&app, 2));
        let dialog = open(&mut app).unwrap();
        app.ui.dialog_mut(dialog).unwrap().fields.insert("documents".into(), json!([black, white, gray]));
        // A script reorders the tabs while the dialog is open: white first, black last.
        app.session.execute("document.move", json!({"document": 2, "to": 0})).unwrap();
        app.session.execute("document.move", json!({"document": 1, "to": 2})).unwrap();
        assert_eq!([id(&app, 0), id(&app, 1), id(&app, 2)], [white, gray, black]);
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert_eq!((app.session.documents().len(), app.session.active().unwrap().doc.mode), (1, ColorMode::Rgb));
        assert_eq!(merged_pixel(&app), [0, 255, 128], "red, green and blue from the documents picked, not their old tabs");
    }

    /// A picked document closed while the dialog is open: OK refuses with the reason, keeps the
    /// dialog and closes nothing, even when another same-size grayscale document took its tab.
    #[test]
    fn a_closed_pick_is_refused_and_no_other_document_merges() {
        let mut app = shaded(0);
        app.run("file.new", json!({"width": 8, "height": 8, "mode": "grayscale"})).unwrap();
        app.session.set_active(0);
        let spare = id(&app, 3);
        let dialog = open(&mut app).unwrap();
        let picked: Vec<u64> = (0..3).map(|i| id(&app, i)).collect();
        app.ui.dialog_mut(dialog).unwrap().fields.insert("documents".into(), json!(picked));
        app.session.execute("file.close", json!({"document": 1})).unwrap();
        assert_eq!(id(&app, 2), spare, "the spare document is where a picked one was");
        let why = crate::dialogs::confirm(&mut app, dialog).unwrap_err();
        assert_eq!(why, CLOSED);
        let d = app.ui.dialogs.iter().find(|d| d.id == dialog).expect("the dialog stays open");
        assert_eq!(d.fields["__error"], json!(CLOSED), "with the reason under the pickers");
        assert_eq!(app.session.documents().len(), 3, "nothing merged or closed");
        assert!(app.session.documents().iter().any(|d| d.doc.id.0 == spare));
    }

    /// A picked document turned into another mode while the dialog is open is refused too; should
    /// the engine refuse a merge after OK, the dialog comes back with the reason.
    #[test]
    fn a_changed_pick_or_a_refused_merge_keeps_the_dialog() {
        let mut app = app(3);
        let dialog = open(&mut app).unwrap();
        let f = app.ui.dialog_mut(dialog).unwrap().fields.clone();
        app.run("image.mode.rgb", json!({})).unwrap();
        assert_eq!(crate::dialogs::confirm(&mut app, dialog).unwrap_err(), CHANGED);
        assert!(app.ui.dialogs.iter().any(|d| d.id == dialog));
        app.ui.dialogs.clear();
        assert!(confirm(&mut app, &f).is_err());
        let again = &app.ui.dialogs.last().expect("the dialog again").fields;
        assert!(owns(again) && again["__error"].is_string());
        assert_eq!(app.session.documents().len(), 3);
    }

    #[test]
    fn body_renders_every_mode_and_the_reason() {
        let app = app(4);
        let ids: Vec<u64> = (0..4).map(|i| id(&app, i)).collect();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for (mode, docs, error) in
            [("rgb", json!(ids[..3]), None), ("cmyk", json!([ids[0], ids[1], ids[2], ids[2]]), None), ("lab", json!(ids[1..]), Some(CLOSED))]
        {
            let mut f = fields(&app).unwrap();
            f.insert("mode".into(), json!(mode));
            f.insert("documents".into(), docs);
            if let Some(e) = error {
                f.insert("__error".into(), json!(e));
            }
            ctx.run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
            assert!(owns(&f));
        }
    }
}
