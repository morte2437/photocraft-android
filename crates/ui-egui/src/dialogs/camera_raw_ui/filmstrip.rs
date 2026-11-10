//! The open-time dialog's filmstrip: raws opened together wait side by side in one Camera Raw
//! dialog, as in Photoshop's. Each keeps its own settings and demosaic choice when another is
//! shown, Open develops every one and Cancel closes every one.

use super::open::{Target, commit_open_raw, open_raw_with, return_on_failure};
use super::sidecar::read_sidecar;
use super::{
    Align2, CameraRaw, CameraRawDialog, Color32, ERect, FontId, PhotocraftApp, RawOpen, Rect, Sense, Stroke, Tokens, Value, json, pos2, vec2, widgets,
};

/// The filmstrip column, shown when more than one raw is being opened.
pub(super) const FILM_W: f32 = 150.0;
/// Long side of a filmstrip thumbnail, in pixels.
const THUMB_SIDE: usize = 240;

/// A raw waiting in the open-time dialog: opened (developed with the defaults) and reviewed
/// before Open keeps it.
pub struct HeldRaw {
    pub raw: RawOpen,
    /// Its settings and the settings it started from (its sidecar's, or the defaults) once it
    /// has been shown and left for another (`None`: not edited yet).
    pub(super) kept: Option<(CameraRaw, CameraRaw)>,
    /// The filmstrip thumbnail until it is first drawn, then its texture.
    pub(super) thumb: Option<egui::ColorImage>,
    tex: Option<egui::TextureHandle>,
}

impl HeldRaw {
    pub(super) fn new(app: &PhotocraftApp, raw: RawOpen) -> Self {
        let thumb = super::open::document_index(app, raw.document).and_then(|i| thumbnail(app, i));
        HeldRaw { raw, kept: None, thumb, tex: None }
    }
}

/// A filmstrip thumbnail of document `index` (a just-developed raw: its first pixel layer).
fn thumbnail(app: &PhotocraftApp, index: usize) -> Option<egui::ColorImage> {
    let doc = &app.session.documents().get(index)?.doc;
    let surf = doc.layers.iter().find_map(|l| l.surface())?;
    let area = doc.bounds();
    let (w, h) = (area.width() as usize, area.height() as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let k = w.max(h).div_ceil(THUMB_SIDE).max(1);
    let (tw, th) = (w.div_ceil(k), h.div_ceil(k));
    let mut acc = vec![[0.0f32; 5]; tw * th];
    let mut row = vec![[0.0f32; 4]; w];
    for y in (0..h).step_by(k.div_ceil(2).max(1)) {
        surf.read_rgba_into(Rect::new(area.x0, area.y0 + y as i32, area.x1, area.y0 + y as i32 + 1), &mut row);
        for (x, q) in row.iter().enumerate().step_by(k.div_ceil(2).max(1)) {
            let a = &mut acc[(y / k) * tw + x / k];
            for c in 0..4 {
                a[c] += q[c];
            }
            a[4] += 1.0;
        }
    }
    let px: Vec<[f32; 4]> = acc.iter().map(|a| if a[4] > 0.0 { [a[0] / a[4], a[1] / a[4], a[2] / a[4], a[3] / a[4]] } else { [0.0; 4] }).collect();
    let display = app.session.color.canvas_display(doc).ok()?;
    Some(CameraRawDialog::image(&px, tw, th, &display))
}

/// `px` (`w`×`h`) box-averaged down to at most [`THUMB_SIDE`] pixels on its long side.
fn shrink(px: &[[f32; 4]], w: usize, h: usize) -> (Vec<[f32; 4]>, usize, usize) {
    let k = w.max(h).div_ceil(THUMB_SIDE).max(1);
    let (tw, th) = (w.div_ceil(k), h.div_ceil(k));
    let mut acc = vec![[0.0f32; 5]; tw * th];
    for (i, q) in px.iter().enumerate().take(w * h) {
        if let Some(a) = acc.get_mut((i / w / k) * tw + (i % w) / k) {
            for c in 0..4 {
                a[c] += q[c];
            }
            a[4] += 1.0;
        }
    }
    (acc.iter().map(|a| if a[4] > 0.0 { [a[0] / a[4], a[1] / a[4], a[2] / a[4], a[3] / a[4]] } else { [0.0; 4] }).collect(), tw, th)
}

/// Shows raw `i` of the filmstrip, with the settings it was left with (its sidecar's, the
/// Camera Raw Defaults' Auto, or the defaults the first time).
pub(super) fn open_held(app: &mut PhotocraftApp, ctx: &egui::Context, i: usize) -> Result<(), String> {
    let held = app.raw_shelf.get(i).ok_or("no raw file at that place in the filmstrip")?;
    let (raw, kept) = (held.raw.clone(), held.kept.clone());
    open_raw_with(app, ctx, raw, kept)
}

/// Shows raw `to` of the filmstrip; the one being left keeps its settings, and its thumbnail
/// shows them.
pub(super) fn show_raw(app: &mut PhotocraftApp, ctx: &egui::Context, to: usize) -> Result<(), String> {
    let Some(d) = app.camera_raw.as_ref() else { return Ok(()) };
    let current = d.raw_open.as_ref().map(|r| r.document);
    if d.target != Target::OpenRaw || app.raw_shelf.get(to).is_none_or(|h| Some(h.raw.document) == current) {
        return Ok(());
    }
    keep_settings(app);
    // On success this replaces the dialog. On failure the dialog stays on the raw being left, and
    // the other keeps its place in the filmstrip.
    let active = app.session.active_index();
    let shown = open_held(app, ctx, to);
    if shown.is_err()
        && let Some(i) = active
    {
        app.session.set_active(i);
    }
    shown
}

/// Stores the dialog's settings (and demosaic choice) on its filmstrip raw, and refreshes the
/// raw's thumbnail from the preview.
pub(super) fn keep_settings(app: &mut PhotocraftApp) {
    let Some(d) = app.camera_raw.as_ref() else { return };
    let Some(raw) = &d.raw_open else { return };
    let Some(held) = app.raw_shelf.iter_mut().find(|h| h.raw.document == raw.document) else { return };
    held.kept = Some((d.params.clone(), d.original.clone()));
    held.raw.demosaic = raw.demosaic;
    // The preview may not have caught up with the last edit: the thumbnail shows what was kept.
    let frame = (d.dirty || d.render_pending()).then(|| d.proxy_job().run().px);
    let developed = frame.as_deref().unwrap_or(&d.processed);
    if !developed.is_empty() {
        let (px, w, h) = shrink(developed, d.pw, d.ph);
        held.thumb = Some(CameraRawDialog::image(&px, w, h, &d.display));
        held.tex = None;
    }
}

/// The settings Open develops a never-shown filmstrip raw with, and what they started from:
/// its sidecar's, else the Camera Raw Defaults' Auto when that is on, else the defaults.
pub(super) fn first_settings(app: &PhotocraftApp, raw: &RawOpen) -> (CameraRaw, Option<CameraRaw>) {
    if let Some((loaded, _, _)) = raw.path.as_deref().and_then(read_sidecar) {
        return (loaded.clone(), Some(loaded));
    }
    let mut p = CameraRaw::default();
    // From the proxy the dialog would use, so showing a raw doesn't change what Auto gives it.
    if app.session.prefs().raw_defaults.apply_auto_tone
        && let Some(st) = super::open::document_index(app, raw.document).and_then(|i| app.session.documents().get(i))
        && let Some(surf) = st.doc.layers.iter().find_map(|l| l.surface())
        && let Ok((area, w, h)) = super::convert::preview_area(st.doc.bounds(), surf)
    {
        let (proxy, ..) = super::convert::box_proxy(surf, area, w, h);
        photocraft_algo::camera_raw::auto_tone(&proxy, &mut p);
    }
    (p, None)
}

/// Open with raws in the filmstrip: the shown one as the dialog has it, then every other with
/// its own settings. One reply for one raw, `{"opened": [...]}` for several. A raw leaves the
/// filmstrip once it has opened or its background develop has started: one that fails stays or
/// comes back there with its settings, so the dialog returns for it (or Cancel still closes it).
pub(super) fn open_all(app: &mut PhotocraftApp, params: &CameraRaw, raw: &RawOpen, original: &CameraRaw) -> Result<Value, String> {
    keep_settings(app);
    let others: Vec<photocraft_doc::DocId> = app.raw_shelf.iter().map(|h| h.raw.document).filter(|id| *id != raw.document).collect();
    let first = commit_open_raw(app, params, raw, Some(original));
    if let Ok(v) = &first {
        return_on_failure(app, v, original);
        app.raw_shelf.retain(|h| h.raw.document != raw.document);
    }
    if others.is_empty() {
        return first;
    }
    let mut opened = Vec::new();
    let mut failed = Vec::new();
    match first {
        Ok(v) => opened.push(v),
        Err(e) => failed.push(format!("{}: {e}", raw.name)),
    }
    for id in others {
        let Some(held) = app.raw_shelf.iter().find(|h| h.raw.document == id) else { continue };
        let (held_raw, kept) = (held.raw.clone(), held.kept.clone());
        let (p, loaded) = match kept {
            Some((p, original)) => (p, Some(original)),
            None => first_settings(app, &held_raw),
        };
        match commit_open_raw(app, &p, &held_raw, loaded.as_ref()) {
            Ok(v) => {
                return_on_failure(app, &v, loaded.as_ref().unwrap_or(&CameraRaw::default()));
                opened.push(v);
                app.raw_shelf.retain(|h| h.raw.document != id);
            }
            Err(e) => failed.push(format!("{}: {e}", held_raw.name)),
        }
    }
    if !failed.is_empty() {
        let e = failed.join("; ");
        if opened.is_empty() {
            return Err(e);
        }
        crate::notices::error(app, e);
    }
    Ok(json!({"opened": opened}))
}

/// Done: saves each raw's settings to its sidecar, as Camera Raw's Done does, and opens none of
/// them (they close again).
pub(super) fn done(app: &mut PhotocraftApp) -> Value {
    use super::sidecar::Stored;
    keep_settings(app);
    let Some(d) = app.camera_raw.as_ref() else { return Value::Null };
    let Some(raw) = d.raw_open.clone() else { return Value::Null };
    let mut raws = vec![(raw.clone(), d.params.clone())];
    for held in app.raw_shelf.iter().filter(|h| h.raw.document != raw.document) {
        let p = match &held.kept {
            Some((p, _)) => p.clone(),
            None => first_settings(app, &held.raw).0,
        };
        raws.push((held.raw.clone(), p));
    }
    let stored: Vec<Stored> = raws.iter().map(|(raw, p)| super::sidecar::store_sidecar(app, raw, p)).collect();
    let saved: Vec<&String> = stored.iter().filter_map(|s| if let Stored::Written(name) = s { Some(name) } else { None }).collect();
    // A failed write (already in a notice) keeps the dialog and every raw open, so no edit is lost.
    if stored.contains(&Stored::Failed) {
        app.ui.status = tl!("Some settings could not be saved; the raws stay open").to_string();
        app.ui.status_error = true;
        return json!({"done": false, "saved": saved});
    }
    app.ui.status = if !saved.is_empty() {
        crate::i18n::fmt(tl!("Saved RAW Develop settings to {file}"), &[("file", &saved.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))])
    } else if stored.iter().all(|s| *s == Stored::Defaults) {
        tl!("Nothing to save: these raws are at their default settings").to_string()
    } else {
        tl!("No settings saved: these raws can't keep a sidecar here").to_string()
    };
    app.ui.status_error = saved.is_empty() && stored.contains(&Stored::Skipped);
    let saved = json!(saved);
    app.camera_raw = None;
    app.raw_shelf.clear();
    for (raw, _) in &raws {
        if let Some(index) = super::open::document_index(app, raw.document) {
            app.session.close(index);
        }
    }
    app.sync_views();
    json!({"done": true, "saved": saved})
}

/// The filmstrip of raws being opened: a thumbnail and name each, the shown one outlined, a dot
/// on edited ones. Returns the one clicked.
pub(super) fn filmstrip(ui: &mut egui::Ui, rect: ERect, shelf: &mut [HeldRaw], current: Option<photocraft_doc::DocId>, t: &Tokens) -> Option<usize> {
    let mut clicked = None;
    let mut film = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 6.0))));
    film.set_clip_rect(rect.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical().id_salt("camera-raw-filmstrip").auto_shrink([false, false]).show(&mut film, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let w = ui.available_width();
        for (i, held) in shelf.iter_mut().enumerate() {
            if held.tex.is_none()
                && let Some(img) = held.thumb.take()
            {
                held.tex = Some(ui.ctx().load_texture(format!("camera-raw-thumb-{}", held.raw.document.0), img, egui::TextureOptions::LINEAR));
            }
            let (tile, response) = ui.allocate_exact_size(vec2(w, w * 0.75 + 18.0), Sense::click());
            let frame = ERect::from_min_size(tile.min, vec2(w, w * 0.75));
            let p = ui.painter();
            p.rect_filled(frame, 2.0, if response.hovered() { t.hover } else { t.canvas });
            if let Some(tex) = &held.tex {
                let size = tex.size_vec2();
                let fit = ((frame.width() - 8.0) / size.x.max(1.0)).min((frame.height() - 8.0) / size.y.max(1.0));
                let r = ERect::from_center_size(frame.center(), size * fit);
                widgets::checker(p, r, 6.0);
                p.image(tex.id(), r, ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            if Some(held.raw.document) == current {
                p.rect_stroke(frame, 2.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            }
            if held.kept.as_ref().is_some_and(|(p, _)| !p.is_identity()) {
                p.circle_filled(frame.right_top() + vec2(-7.0, 7.0), 3.0, t.text);
            }
            let name = ERect::from_min_max(pos2(tile.left(), frame.bottom()), tile.max);
            let font = FontId::proportional(10.5);
            let label = tail_fitting(p, &held.raw.name, font.clone(), name.width());
            p.with_clip_rect(name).text(name.center(), Align2::CENTER_CENTER, label, font, t.text_dim);
            let response = response.on_hover_text(&held.raw.name);
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, Some(held.raw.document) == current, &held.raw.name));
            if response.clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// `name`, or "…" and as much of its end as fits in `width`: raws from one shoot differ at the
/// end of their names (the frame number).
fn tail_fitting(p: &egui::Painter, name: &str, font: FontId, width: f32) -> String {
    let fits = |s: &str| p.layout_no_wrap(s.to_owned(), font.clone(), Color32::WHITE).size().x <= width;
    if fits(name) {
        return name.to_owned();
    }
    let chars: Vec<char> = name.chars().collect();
    (1..chars.len()).map(|start| format!("…{}", chars[start..].iter().collect::<String>())).find(|s| fits(s)).unwrap_or_default()
}
