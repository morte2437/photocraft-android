//! Non-blocking notices: a small stack of cards in the lower-right corner of the window for things
//! the user should see but that must not interrupt work (import/export warnings such as "adjustment
//! flattened", files that couldn't be opened). Notices live in [`UiState`](crate::UiState) so the
//! control channel can read them (`ui.inspect`) and clear them (`ui.set`).

use serde::{Deserialize, Serialize};

use crate::PhotocraftApp;

const CLIPBOARD_UNREADABLE: &str = "Pasting from other apps is unavailable";
/// At most this many notices are kept; older ones drop off.
pub const MAX_NOTICES: usize = 3;
/// Lines shown per notice before "…and N more".
const MAX_LINES: usize = 8;
/// A full canvas refresh on the CPU compositor slower than this (ms) gets a notice.
const SLOW_CPU_REFRESH_MS: f64 = 1000.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub lines: Vec<String>,
    /// An error (warning colour) rather than an informational notice.
    #[serde(default)]
    pub error: bool,
    /// Preference key to set when this notice is dismissed.
    #[serde(default)]
    pub dismiss_pref: Option<String>,
    /// `{name}` values filled into the title and lines after they are translated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<(String, String)>,
    /// egui input time (seconds) this notice was first drawn; the auto-hide timer runs from here
    /// and is reset while the pointer rests on the stack (issue #2022). `None` until first shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown_at: Option<f64>,
    /// Stays until dismissed (or until whatever it warns of has passed), whatever the auto-hide
    /// setting: for a warning the user must not miss, such as nearly full memory.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sticky: bool,
}

impl Notice {
    /// `s` (the title or a line) in the current language, with [`Notice::args`] filled in.
    pub fn text(&self, s: &str) -> String {
        let args: Vec<(&str, &str)> = self.args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        crate::i18n::fmt(crate::i18n::t(s), &args)
    }
}

/// Show a notice (newest last); returns its id.
pub fn post(app: &mut PhotocraftApp, title: impl Into<String>, lines: Vec<String>, error: bool, dismiss_pref: Option<&str>) -> u64 {
    let id = app.ui.alloc_id();
    app.ui.notices.push(Notice {
        id,
        title: title.into(),
        lines,
        error,
        dismiss_pref: dismiss_pref.map(str::to_owned),
        args: Vec::new(),
        shown_at: None,
        sticky: false,
    });
    cap_notices(app);
    id
}

fn cap_notices(app: &mut PhotocraftApp) {
    while app.ui.notices.len() > MAX_NOTICES {
        let oldest_temporary = app.ui.notices.iter().position(|notice| notice.dismiss_pref.is_none() && !notice.sticky).unwrap_or(0);
        app.ui.notices.remove(oldest_temporary);
    }
}

/// How to paste in hints: Edit › Paste's effective shortcut (`Ctrl+V`), else the menu path.
#[cfg(test)]
pub(crate) fn paste_hint(app: &PhotocraftApp) -> String {
    crate::shortcuts::shortcut_label(app, "edit.paste").unwrap_or_else(|| tl!("Edit › Paste").to_string())
}

/// A paste found no clipboard to read (#5031). In a Flatpak on Wayland the sandbox hides the
/// data-control protocol that other apps' images are read through, and there is no X server to
/// fall back to, so the clipboard may hold an image all the same. Say so, and give the command
/// that starts this install under XWayland, where the X11 clipboard works, when there is one.
pub fn clipboard_unreadable(app: &mut PhotocraftApp) {
    app.ui.status = tl!("Nothing to paste: PhotoCraft can't read the system clipboard here").into();
    app.ui.status_error = false;
    if app.ui.notices.iter().any(|notice| notice.title == CLIPBOARD_UNREADABLE) {
        return;
    }
    // English templates, translated when drawn, as in `wayland_file_drop_guidance`.
    let mut lines =
        vec!["PhotoCraft can't reach the system clipboard in this session. On Wayland, a Flatpak sandbox hides the protocol that images from other apps are read through.".to_owned()];
    let mut args = Vec::new();
    if let Some(command) = &app.services.xwayland_command {
        lines.push("To paste from other apps, start PhotoCraft under XWayland: `{command}`".to_owned());
        args.push(("command".to_owned(), command.clone()));
    }
    let id = post(app, CLIPBOARD_UNREADABLE, lines, false, None);
    if let Some(notice) = app.ui.notices.iter_mut().find(|notice| notice.id == id) {
        notice.args = args;
    }
}

/// A full refresh of document `doc` fell back to the CPU compositor (`reason`, the GPU's) and took
/// `ms`. The canvas doesn't respond while it runs, so once per document say why it was slow and
/// which edits will do it again (#1015).
pub fn slow_cpu_refresh(app: &mut PhotocraftApp, doc: u64, ms: f64, reason: Option<&crate::render_path::CpuReason>) {
    if matches!(reason, Some(crate::render_path::CpuReason::CompositorBuilding))
        || ms.is_nan()
        || ms < SLOW_CPU_REFRESH_MS
        || app.ui.slow_refresh_noticed.contains(&doc)
    {
        return;
    }
    app.ui.slow_refresh_noticed.push(doc);
    let seconds = format!("{:.1}", ms / 1000.0);
    let mut lines = vec![
        crate::i18n::fmt(tl!("This redraw took {seconds} s on the CPU."), &[("seconds", &seconds)]),
        tl!("Edits that change the whole document, such as a layer's blend mode, opacity or visibility, redraw all of it.").into(),
    ];
    if let Some(&crate::render_path::CpuReason::OverBudget { needed, budget }) = reason {
        if needed > budget {
            lines.push(crate::i18n::fmt(
                tl!("Its layers need {needed} of GPU memory; the budget leaves them {budget}."),
                &[("needed", &crate::chrome_ui::fmt_bytes(needed)), ("budget", &crate::chrome_ui::fmt_bytes(budget))],
            ));
        }
        lines.push(
            tl!("Set GPU Memory in Preferences › Performance to what your graphics card can hold, at most 3/8 of its memory. Automatic follows Memory Usage, within a quarter of this computer's memory and 3/8 of the card's, and doesn't count the document against a card whose memory is known. The status bar shows GPU Memory Full while they don't fit.")
                .into(),
        );
    } else if let Some(r) = reason {
        lines.push(crate::i18n::fmt(tl!("The GPU compositor wasn't used: {reason}"), &[("reason", &r.to_string())]));
    }
    post(app, tl!("Large document: redrawing on the CPU"), lines, false, None);
}

fn dismiss(app: &mut PhotocraftApp, id: u64) {
    let dismiss_pref = app.ui.notices.iter().find(|notice| notice.id == id).and_then(|notice| notice.dismiss_pref.clone());
    app.ui.notices.retain(|notice| notice.id != id);
    if let Some(key) = dismiss_pref {
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert(key, serde_json::Value::Bool(true));
        });
    }
}

/// Report import/export `warnings` for the file operation `what` (e.g. "Opened a.psd"): the status
/// bar says how many there were, and a notice lists them. Nothing happens when there are none.
pub fn io_warnings(app: &mut PhotocraftApp, what: &str, warnings: &[String]) {
    let Some(first) = warnings.first() else { return };
    let n = warnings.len();
    let title = crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n as u64, "{what} with {n} warning", "{what} with {n} warnings"), &[("what", what)]);
    app.ui.status = if n == 1 { format!("{what}: {first}") } else { format!("{title}: {first} …") };
    app.ui.status_error = true;
    post(app, title, warnings.to_vec(), false, None);
}

/// Report the files a folder run (Batch, Image Processor, …) could not process, one line each:
/// the status bar shows the first as an error, and a notice titled with how many failed lists
/// them all until dismissed (#10682).
pub fn file_failures(app: &mut PhotocraftApp, what: &str, failures: &[String]) {
    let Some(first) = failures.first() else { return };
    let n = failures.len();
    let title = crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n as u64, "{what}: {n} file failed", "{what}: {n} files failed"), &[("what", what)]);
    app.ui.status = if n == 1 { format!("{what}: {first}") } else { format!("{title}: {first} …") };
    app.ui.status_error = true;
    post(app, title, failures.to_vec(), true, None);
}

/// Report a failed file operation: the status bar shows it as an error and a notice keeps it on
/// screen until dismissed.
pub fn error(app: &mut PhotocraftApp, message: String) {
    app.ui.status = message.clone();
    app.ui.status_error = true;
    post(app, message, Vec::new(), true, None);
}

/// Disclose the salvage warnings a footage-loading command result carries (#11113): a GIF or
/// WebP damaged after some frames keeps its earlier frames and returns the decoder's warning in
/// `warnings`, which must reach the user like any other import warning. Nothing happens when
/// every frame came through.
pub fn footage_warnings(app: &mut PhotocraftApp, result: &serde_json::Value, what: &str) {
    let warnings: Vec<String> = result["warnings"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str).map(str::to_owned).collect();
    io_warnings(app, what, &warnings);
}

/// Auto-hide (issue #2022): drop notices whose delay has elapsed, pausing the timers of any the
/// pointer is resting on. `hovered` is true when the pointer is over the notice stack; `just_left`
/// is true on the frame it moves away. In both cases the timers restart from `now`, so a long
/// stationary hover is never counted as elapsed time. Returns the seconds until the next notice
/// would expire, so the caller can request a repaint then.
fn expire(app: &mut PhotocraftApp, now: f64, hovered: bool, just_left: bool, auto_hide: bool, duration: f64) -> Option<f64> {
    if !auto_hide {
        return None;
    }
    let mut expired = Vec::new();
    let mut next = f64::INFINITY;
    for n in app.ui.notices.iter_mut().filter(|n| !n.sticky) {
        if n.shown_at.is_none() {
            n.shown_at = Some(now);
        }
        if hovered {
            n.shown_at = Some(now);
            continue;
        }
        if just_left {
            n.shown_at = Some(now);
        }
        let shown_at = n.shown_at.unwrap_or(now);
        let elapsed = (now - shown_at).max(0.0);
        if elapsed >= duration {
            expired.push(n.id);
        } else {
            next = next.min(duration - elapsed);
        }
    }
    if !expired.is_empty() {
        app.ui.notices.retain(|n| !expired.contains(&n.id));
    }
    next.is_finite().then_some(next)
}

/// Draw the notices; each has a close button. When Interface › Notification › Auto Hide Notices
/// is on, a notice disappears once its delay is up unless the pointer rests on the stack.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.notices.is_empty() {
        app.notices_hovered = false;
        return;
    }
    let t = crate::theme::Tokens::get(ctx);
    let mut dismiss_id = None;
    // Clear the status bar (~24 px) and leave the dock's edge some air.
    let area = egui::Area::new(egui::Id::unique("photocraft-notices"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -36.0))
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_max_width(360.0);
            for n in &app.ui.notices {
                egui::Frame::NONE
                    .fill(t.card)
                    .stroke(egui::Stroke::new(1.0, if n.error { t.warning } else { t.card_border }))
                    .corner_radius(t.radius)
                    .inner_margin(egui::Margin::same(10))
                    .shadow(egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: t.shadow })
                    .show(ui, |ui| {
                        ui.set_width(340.0);
                        ui.horizontal(|ui| {
                            let title = egui::RichText::new(n.text(&n.title)).strong().color(if n.error { t.warning } else { t.text });
                            ui.add(egui::Label::new(title).wrap());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                if ui.add(egui::Button::new(egui::RichText::new("×").color(t.text_dim)).frame(false)).on_hover_text(tl!("Dismiss")).clicked() {
                                    dismiss_id = Some(n.id);
                                }
                            });
                        });
                        for line in n.lines.iter().take(MAX_LINES) {
                            let line = n.text(line);
                            ui.add(egui::Label::new(egui::RichText::new(format!("• {line}")).color(t.text_dim)).wrap());
                        }
                        if n.lines.len() > MAX_LINES {
                            let text = crate::i18n::fmt(tl!("…and {n} more"), &[("n", &(n.lines.len() - MAX_LINES).to_string())]);
                            ui.label(egui::RichText::new(text).color(t.text_faint));
                        }
                    });
                ui.add_space(6.0);
            }
        });
    if let Some(id) = dismiss_id {
        dismiss(app, id);
    }
    let (auto_hide, duration) = {
        let i = &app.session.prefs().interface;
        (i.notification_auto_hide, f64::from(i.notification_duration_seconds.max(1)))
    };
    let now = ctx.input(|input| input.time);
    let hovered = ctx.pointer_hover_pos().is_some_and(|p| area.response.rect.contains(p));
    // The frame the pointer leaves the stack starts a fresh delay, so time spent hovering never
    // counts towards the auto-hide (#2022).
    let just_left = std::mem::replace(&mut app.notices_hovered, hovered) && !hovered;
    let before = app.ui.notices.len();
    let next = expire(app, now, hovered, just_left, auto_hide, duration);
    if app.ui.notices.len() < before {
        // The cards dropped this frame were already painted by it: ask for another frame at once
        // so the refreshed stack is drawn without waiting for unrelated input.
        ctx.request_repaint();
    }
    if let Some(wait) = next {
        ctx.request_repaint_after(std::time::Duration::try_from_secs_f64(wait).unwrap_or_default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PhotocraftApp, Services};
    use photocraft_engine::Session;

    #[test]
    fn wayland_session_does_not_post_unavailable_notice() {
        let app = PhotocraftApp::new(Session::new(), Services { is_wayland: true, ..Default::default() });
        assert!(app.ui.notices.is_empty());
    }

    /// #5031: a paste that can't reach the clipboard (a Flatpak on Wayland) says why and how to
    /// paste anyway, once while shown; a clipboard that simply holds no image stays a quiet no-op.
    #[test]
    fn unreadable_clipboard_paste_explains_the_sandbox() {
        let command = "flatpak run --nosocket=wayland --socket=x11 ai.storyteller.photocraft";
        let services = |unreadable: bool| Services {
            clipboard_get_image: Some(Box::new(move || if unreadable { Err("no clipboard backend".into()) } else { Ok(None) })),
            clipboard_unreadable: Some(Box::new(move || unreadable)),
            xwayland_command: Some(command.to_string()),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(Session::new(), services(true));
        for _ in 0..2 {
            let r = crate::menus::invoke(&mut app, &ctx, "edit.paste", serde_json::json!({})).unwrap();
            assert_eq!(r["pasted"], serde_json::json!(false));
        }
        assert_eq!(app.ui.status, "Nothing to paste: PhotoCraft can't read the system clipboard here");
        let shown: Vec<_> = app.ui.notices.iter().filter(|notice| notice.title == CLIPBOARD_UNREADABLE).collect();
        assert_eq!(shown.len(), 1, "a second paste doesn't stack another notice");
        let guidance = english(shown[0]);
        assert!(guidance.contains(&format!("under XWayland: `{command}`")), "{guidance}");

        let mut empty = PhotocraftApp::new(Session::new(), services(false));
        crate::menus::invoke(&mut empty, &ctx, "edit.paste", serde_json::json!({})).unwrap();
        assert_eq!(empty.ui.status, "Nothing to paste: the clipboard holds no image");
        assert!(empty.ui.notices.iter().all(|notice| notice.title != CLIPBOARD_UNREADABLE));
    }

    /// A notice's lines in English with their arguments filled in.
    fn english(notice: &Notice) -> String {
        let args: Vec<(&str, &str)> = notice.args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        notice.lines.iter().map(|line| crate::i18n::fmt(line, &args)).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn paste_hint_follows_the_shortcut_and_falls_back_to_the_menu() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        assert_eq!(paste_hint(&app), crate::shortcuts::shortcut_label(&app, "edit.paste").unwrap());
        app.session.prefs.edit(|prefs| {
            prefs.shortcuts.insert("edit.paste".into(), String::new());
        });
        assert_eq!(paste_hint(&app), "Edit › Paste");
    }

    #[test]
    fn preparing_compositor_does_not_warn_about_large_documents() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        slow_cpu_refresh(&mut app, 1, 60_000.0, Some(&crate::render_path::CpuReason::CompositorBuilding));
        assert!(app.ui.notices.is_empty());
        assert!(app.ui.slow_refresh_noticed.is_empty());
    }

    #[test]
    fn a_slow_cpu_refresh_is_explained_once_per_document() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        let over = crate::render_path::CpuReason::OverBudget { needed: 3600 << 20, budget: 3400 << 20 };
        slow_cpu_refresh(&mut app, 1, 400.0, Some(&over));
        slow_cpu_refresh(&mut app, 1, f64::NAN, Some(&over));
        assert!(app.ui.notices.is_empty(), "a quick refresh needs no explanation");

        // The layers of document 1 need 3.6 GB and may hold 3.4 GB: the notice says so.
        slow_cpu_refresh(&mut app, 1, 55_000.0, Some(&over));
        assert_eq!(app.ui.notices.len(), 1);
        let text = app.ui.notices[0].lines.join(" ");
        assert!(text.contains("55.0 s"), "{text}");
        assert!(text.contains("blend mode, opacity or visibility"), "{text}");
        assert!(text.contains("need 3.5G of GPU memory; the budget leaves them 3.3G"), "{text}");
        assert!(text.contains("GPU Memory in Preferences › Performance"), "{text}");
        assert!(!app.ui.notices[0].error);

        slow_cpu_refresh(&mut app, 1, 55_000.0, Some(&over));
        assert_eq!(app.ui.notices.len(), 1, "once per document");

        slow_cpu_refresh(&mut app, 2, 3_000.0, Some(&crate::render_path::CpuReason::Unsupported("Blend If on `Overlay` (composited on the CPU)".into())));
        assert_eq!(app.ui.notices.len(), 2);
        let text = app.ui.notices[1].lines.join(" ");
        assert!(text.contains("Blend If on `Overlay`"), "{text}");
        assert!(!text.contains("Memory usage"), "the budget hint is only for the budget fallback: {text}");
    }

    #[test]
    fn import_warning_punctuation_is_localized_without_changing_english() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        // English in every build; Czech too when the craft-i18n catalogs are embedded.
        let czech = crate::i18n::needs_catalogs().then(|| crate::i18n::Lang::from_code("cs").expect("cs registered"));
        for lang in std::iter::once(crate::i18n::Lang::EN).chain(czech) {
            crate::i18n::with_language(lang, || {
                let what = crate::i18n::fmt(tl!("Opened {name}"), &[("name", "x.psd")]);
                let warnings = vec!["first warning".into(), "second warning".into(), "third warning".into()];
                io_warnings(&mut app, &what, &warnings);
                if lang == crate::i18n::Lang::EN {
                    assert_eq!(app.ui.status, "Opened x.psd with 3 warnings: first warning …");
                }
                let title = crate::i18n::fmt(&crate::i18n::trn(lang, 3, "{what} with {n} warning", "{what} with {n} warnings"), &[("what", &what)]);
                assert_eq!(app.ui.status, format!("{title}: first warning …"));
                assert_eq!(app.ui.notices.last().unwrap().lines, warnings);
                io_warnings(&mut app, &what, &warnings[..1]);
                assert_eq!(app.ui.status, format!("{what}: first warning"));
            });
        }
    }

    #[test]
    fn dismissing_a_notice_writes_its_own_preference_key() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        post(&mut app, "Dismissible", Vec::new(), false, Some("ui.testNoticeDismissed"));
        let id = app.ui.notices[0].id;
        dismiss(&mut app, id);
        assert_eq!(app.session.prefs().dialogs.get("ui.testNoticeDismissed").and_then(serde_json::Value::as_bool), Some(true));
    }

    #[test]
    fn notices_auto_hide_after_their_delay() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        post(&mut app, "Transient", Vec::new(), false, None);
        // The first draw stamps the timer and schedules the hide.
        assert_eq!(expire(&mut app, 100.0, false, false, true, 6.0), Some(6.0));
        assert_eq!(app.ui.notices.len(), 1);
        // Still there just before the delay, the next repaint a second away.
        assert_eq!(expire(&mut app, 105.0, false, false, true, 6.0), Some(1.0));
        assert_eq!(app.ui.notices.len(), 1);
        // Gone once the delay has elapsed.
        assert_eq!(expire(&mut app, 106.0, false, false, true, 6.0), None);
        assert!(app.ui.notices.is_empty());
    }

    #[test]
    fn hover_pauses_the_autohide_timer() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        post(&mut app, "Transient", Vec::new(), false, None);
        // While the pointer rests on the stack the timer keeps resetting: it never expires.
        for t in [5.0, 6.0, 7.0, 100.0] {
            assert_eq!(expire(&mut app, t, true, false, true, 6.0), None, "paused at {t}");
            assert_eq!(app.ui.notices.len(), 1, "still shown at {t}");
        }
    }

    #[test]
    fn leaving_a_long_hover_gives_a_fresh_delay() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        post(&mut app, "Transient", Vec::new(), false, None);
        // Hovering starts the pause...
        assert_eq!(expire(&mut app, 0.0, true, false, true, 6.0), None);
        // ...and the pointer then rests still for far longer than the delay, so no frame
        // refreshes the stamp. The leave frame must restart the timer, not expire at once.
        assert_eq!(expire(&mut app, 10.0, false, true, true, 6.0), Some(6.0));
        assert_eq!(app.ui.notices.len(), 1, "leaving a hover is not elapsed time");
    }

    #[test]
    fn autohide_can_be_switched_off() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        post(&mut app, "Persistent", Vec::new(), false, None);
        assert_eq!(expire(&mut app, 0.0, false, false, false, 6.0), None);
        assert_eq!(expire(&mut app, 1_000_000.0, false, false, false, 6.0), None);
        assert_eq!(app.ui.notices.len(), 1, "off: the notice stays until dismissed");
    }

    /// #2022 review: the frame that drops a notice has already painted it, so it must ask for
    /// another frame at once; otherwise the card lingers until unrelated input.
    #[test]
    fn expiring_a_notice_requests_a_frame_to_clear_it() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        post(&mut app, "Transient", Vec::new(), false, None);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        assert_eq!(app.ui.notices.len(), 1);
        // Past the 6 s default: the notice is dropped and a repaint is requested in the same frame.
        let input = egui::RawInput { time: Some(10.0), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        assert!(app.ui.notices.is_empty());
        assert!(ctx.has_requested_repaint(), "the frame that drops a notice asks for another");
    }
}
