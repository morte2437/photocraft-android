use super::*;
use egui::{Event, Pos2, RawInput, ViewportId};
use photocraft_engine::Session;

/// #5826: the drag overlay under a dialog doesn't depend on the custom title bar. On macOS
/// (OS traffic lights over the app's own title strip) a dialog left the window undraggable.
#[test]
fn a_dialog_keeps_the_title_strip_draggable_with_either_title_bar() {
    for custom in [false, true] {
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.custom_titlebar = custom;
        app.run("file.new", serde_json::json!({"width": 8, "height": 8})).unwrap();
        assert!(!app.titlebar_modal_drag(true), "no dialog, no overlay");
        crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        assert!(app.titlebar_modal_drag(true), "custom title bar: {custom}");
        assert!(!app.titlebar_modal_drag(false), "chrome hidden (full screen): no title strip");
        app.ui.panels.menu_bar = false;
        assert!(!app.titlebar_modal_drag(true), "no menu bar, no title strip");
    }
}

/// A headless window `width` pt wide showing the title bar and the resize zones.
struct Win {
    ctx: egui::Context,
    app: PhotocraftApp,
    width: f32,
    time: f64,
    maximized: bool,
}

impl Win {
    fn new(width: f32) -> Self {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.custom_titlebar = true;
        Self { ctx, app, width, time: 0.0, maximized: false }
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.frame_with_modal(events, false)
    }

    fn frame_with_modal(&mut self, events: Vec<Event>, modal: bool) -> egui::FullOutput {
        self.time += 0.05;
        let mut input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(self.width, 600.0))), time: Some(self.time), events, ..Default::default() };
        if let Some(v) = input.viewports.get_mut(&ViewportId::ROOT) {
            v.maximized = Some(self.maximized);
        }
        // What `PhotocraftApp::raw_input_hook` does.
        release_after_os_resize(&self.ctx, &mut input);
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| {
            crate::panels::title_bar(app, ui);
            if modal {
                egui::Modal::new(Id::unique("titlebar-drag-test-modal")).backdrop_color(Color32::TRANSPARENT).show(ui.ctx(), |ui| {
                    ui.label("Modal dialog");
                });
                modal_drag_overlay(ui.ctx());
            }
            resize_zones(ui);
        });
        out.textures_delta.clear();
        out
    }

    /// Frames for a pointer gesture: move to `path[0]`, press, move along `path`, release. The
    /// viewport commands the frames sent.
    fn gesture(&mut self, path: &[Pos2]) -> Vec<ViewportCommand> {
        self.gesture_with_modal(path, false)
    }

    fn gesture_with_modal(&mut self, path: &[Pos2], modal: bool) -> Vec<ViewportCommand> {
        self.gesture_by(path, modal, egui::PointerButton::Primary)
    }

    fn gesture_by(&mut self, path: &[Pos2], modal: bool, btn: egui::PointerButton) -> Vec<ViewportCommand> {
        let mut events = vec![vec![Event::PointerMoved(path[0])], vec![button(path[0], true, btn)]];
        events.extend(path[1..].iter().map(|p| vec![Event::PointerMoved(*p)]));
        events.push(vec![button(*path.last().unwrap_or(&path[0]), false, btn)]);
        events
            .into_iter()
            .flat_map(|e| self.frame_with_modal(e, modal).viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default())
            .collect()
    }

    fn click(&mut self, p: Pos2) -> Vec<ViewportCommand> {
        self.gesture(&[p])
    }

    fn caption(&self, c: Caption) -> Rect {
        self.ctx.read_response(caption_id(c)).map(|r| r.rect).unwrap_or(Rect::NOTHING)
    }

    /// The bar's clickable widgets other than its drag gap and the caption buttons.
    fn bar_widgets(&self) -> Vec<Rect> {
        let skip = CAPTIONS.map(caption_id);
        self.ctx.viewport(|v| {
            v.prev_pass
                .widgets
                .layers()
                .filter(|(l, _)| l.order != Order::Foreground)
                .flat_map(|(_, ws)| ws.iter())
                .filter(|w| w.sense.senses_click() && !w.sense.senses_drag() && w.rect.top() < 30.0 && !skip.contains(&w.id))
                .map(|w| w.rect)
                .collect()
        })
    }
}

fn texts(out: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
            egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
            _ => {}
        }
    }
    let mut v = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
    v
}

fn button(p: Pos2, pressed: bool, button: egui::PointerButton) -> Event {
    Event::PointerButton { pos: p, button, pressed, modifiers: Default::default() }
}

#[test]
fn caption_buttons_take_the_right_end_and_nothing_overlaps_them() {
    for width in [760.0, 1100.0, 1440.0] {
        let mut w = Win::new(width);
        w.frame(vec![]);
        w.frame(vec![]);
        let [min, max, close] = CAPTIONS.map(|c| w.caption(c));
        assert_eq!((close.right(), close.top()), (width, 0.0), "Close sits in the window's corner at {width}");
        // The bar's bottom point is the panel's separator line, which the buttons leave visible.
        assert!(close.height() >= 31.0 && [min, max, close].iter().all(|r| r.width() == BUTTON_WIDTH));
        assert_eq!((min.right(), max.right()), (max.left(), close.left()));
        let widgets = w.bar_widgets();
        assert!(widgets.len() >= 10, "menus, search, theme, workspace: {widgets:?}");
        for a in &widgets {
            assert!(a.right() <= min.left(), "{a:?} runs into the caption buttons at {width}");
        }
    }
}

#[test]
fn the_title_bar_promotes_no_community_chat() {
    // Issue #1726: only the always-visible title-bar button is removed. The start-screen
    // and About buttons and Help › Discord remain available.
    for width in [760.0, 1100.0, 1440.0] {
        let mut w = Win::new(width);
        w.frame(vec![]);
        let out = w.frame(vec![]);
        assert!(!texts(&out).iter().any(|(t, _)| t.contains("Discord")), "no Discord in the bar at {width}");
    }
}

#[test]
fn the_brand_mark_leads_the_bar() {
    let mut w = Win::new(1440.0);
    w.frame(vec![]);
    let out = w.frame(vec![]);
    let mark = crate::brand::mark_rect(&w.ctx).expect("the brand mark is painted");
    let file = texts(&out).into_iter().find(|(t, _)| t == "File").map(|(_, r)| r).expect("the File menu title");
    assert!(mark.right() < file.left(), "the mark sits left of the menus: {mark:?} {file:?}");
    assert!(mark.left() < 20.0 && mark.width() >= 16.0, "{mark:?}");
}

#[test]
fn close_runs_file_exit_which_asks_about_unsaved_work() {
    let mut w = Win::new(1100.0);
    w.frame(vec![]);
    let close = w.caption(Caption::Close).center();
    let cmds = w.click(close);
    // File › Exit asks the window to close; the unsaved-changes guard (`discard_ui`) then
    // decides, so the caption button and the menu item behave the same.
    assert!(cmds.iter().any(|c| matches!(c, ViewportCommand::Close)), "{cmds:?}");
}

/// After the window's GPU device is lost, the prompt asking about unsaved work could never be
/// drawn: the close button quits instead of opening it (see `discard_ui::intercept`).
#[test]
fn close_quits_without_a_prompt_once_the_window_device_is_lost() {
    let mut w = Win::new(1100.0);
    w.app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    w.app.run("layer.new.layer", json!({})).unwrap();
    w.frame(vec![]);
    let close = w.caption(Caption::Close).center();
    let asks = w.click(close);
    assert!(!asks.iter().any(|c| matches!(c, ViewportCommand::Close)), "a window that can draw asks first: {asks:?}");
    assert!(w.app.discard.is_some());
    w.app.discard = None;
    w.app.window_lost = true;
    let quits = w.click(close);
    assert!(quits.iter().any(|c| matches!(c, ViewportCommand::Close)), "{quits:?}");
    assert!(w.app.discard.is_none(), "no prompt opened");
}

#[test]
fn minimize_and_maximize_follow_the_window_state() {
    let mut w = Win::new(1100.0);
    w.frame(vec![]);
    let (min, max) = (w.caption(Caption::Minimize).center(), w.caption(Caption::Maximize).center());
    assert!(w.click(min).contains(&ViewportCommand::Minimized(true)));
    assert!(w.click(max).contains(&ViewportCommand::Maximized(true)));
    w.maximized = true;
    assert!(w.click(max).contains(&ViewportCommand::Maximized(false)), "Restore");
}

#[test]
fn empty_bar_space_drags_and_double_click_maximizes() {
    let mut w = Win::new(1440.0);
    w.frame(vec![]);
    let empty = pos2(980.0, 16.0);
    assert!(w.bar_widgets().iter().all(|r| !r.contains(empty)), "the probe point must be empty bar space");
    let cmds = w.gesture(&[empty, empty + vec2(12.0, 4.0), empty + vec2(30.0, 8.0)]);
    assert_eq!(cmds.iter().filter(|c| matches!(c, ViewportCommand::StartDrag)).count(), 1, "{cmds:?}");
    let mut cmds = w.click(empty);
    cmds.extend(w.click(empty));
    assert!(cmds.contains(&ViewportCommand::Maximized(true)), "{cmds:?}");
    w.maximized = true;
    w.time += 1.0;
    let mut cmds = w.click(empty);
    cmds.extend(w.click(empty));
    assert!(cmds.contains(&ViewportCommand::Maximized(false)), "{cmds:?}");
    // Widgets keep their clicks: dragging from a menu title doesn't move the window.
    let file = texts(&w.frame(vec![])).into_iter().find(|(t, _)| t == "File").map(|(_, r)| r.center());
    let file = file.expect("the File menu title");
    let cmds = w.gesture(&[file, file + vec2(12.0, 4.0), file + vec2(30.0, 8.0)]);
    assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::StartDrag)), "{cmds:?}");
}

#[test]
fn empty_bar_space_still_drags_with_a_modal_open() {
    let mut w = Win::new(1440.0);
    w.frame(vec![]);
    w.frame_with_modal(vec![], true);
    let empty = pos2(980.0, 16.0);
    let drag_area = w.ctx.data(|d| d.get_temp::<Rect>(modal_drag_area_id()));
    assert!(drag_area.is_some_and(|r| r.contains(empty)), "the test point must be in the current title-bar gap: {drag_area:?}");
    assert!(w.bar_widgets().iter().all(|r| !r.contains(empty)), "the probe point must be empty bar space");
    let cmds = w.gesture_with_modal(&[empty, empty + vec2(12.0, 4.0), empty + vec2(30.0, 8.0)], true);
    assert_eq!(cmds.iter().filter(|c| matches!(c, ViewportCommand::StartDrag)).count(), 1, "{cmds:?}");
}

#[test]
fn middle_button_drag_in_the_gap_does_not_start_a_window_drag() {
    let mut w = Win::new(1440.0);
    w.frame(vec![]);
    let empty = pos2(980.0, 16.0);
    let path = [empty, empty + vec2(12.0, 4.0), empty + vec2(30.0, 8.0)];
    let cmds = w.gesture_by(&path, false, egui::PointerButton::Middle);
    assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::StartDrag)), "{cmds:?}");
    // An ordinary left drag afterwards still moves the window.
    let cmds = w.gesture(&path);
    assert_eq!(cmds.iter().filter(|c| matches!(c, ViewportCommand::StartDrag)).count(), 1, "{cmds:?}");
    // Same with a modal open (the overlay over the gap).
    w.frame_with_modal(vec![], true);
    let cmds = w.gesture_by(&path, true, egui::PointerButton::Middle);
    assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::StartDrag)), "{cmds:?}");
}

#[test]
fn modal_drag_overlay_does_not_cover_a_dialog_moved_into_the_gap() {
    let mut w = Win::new(1440.0);
    w.frame(vec![]);
    w.frame_with_modal(vec![], true);
    let area = w.ctx.data(|d| d.get_temp::<Rect>(modal_drag_area_id())).unwrap_or(Rect::NOTHING);
    assert!(area.contains(pos2(980.0, 16.0)), "the probe point must be in the gap: {area:?}");
    w.ctx.data_mut(|d| d.insert_temp(Id::unique("pc-dialog-rects"), vec![area]));
    let cmds = w.gesture_with_modal(&[area.center(), area.center() + vec2(12.0, 4.0)], true);
    assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::StartDrag)), "{cmds:?}");
}

/// #1581: KDE Wayland keeps the button release of an OS resize. egui then kept the edge
/// pressed (no hover until the next click) and egui-winit never re-sent the resize cursor that
/// the compositor had replaced: the cursor came back only on every other press.
#[test]
fn after_an_os_resize_the_edge_hovers_and_shows_its_cursor_again() {
    let mut w = Win::new(1000.0);
    w.frame(vec![]);
    let p = pos2(3.0, 300.0);
    // Press and drag; the compositor takes over, so no release ever arrives.
    let mut cmds = vec![];
    for e in [vec![Event::PointerMoved(p)], vec![button(p, true, egui::PointerButton::Primary)], vec![Event::PointerMoved(p + vec2(6.0, 0.0))]] {
        cmds.extend(w.frame(e).viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default());
    }
    assert!(cmds.contains(&ViewportCommand::BeginResize(ResizeDirection::West)), "{cmds:?}");
    w.frame(vec![]);
    assert!(!w.ctx.input(|i| i.pointer.primary_down()), "the press ends with the hand-off");
    // The pointer comes back: one frame of the default cursor, so the next one is re-sent.
    let out = w.frame(vec![Event::PointerMoved(p)]);
    assert_eq!(out.platform_output.cursor_icon, CursorIcon::Default);
    let out = w.frame(vec![Event::PointerMoved(p + vec2(1.0, 0.0))]);
    assert_eq!(out.platform_output.cursor_icon, CursorIcon::ResizeWest);
    // And the next press resizes again (it alternated before).
    assert!(w.gesture(&[p, p + vec2(6.0, 0.0)]).contains(&ViewportCommand::BeginResize(ResizeDirection::West)));
}

/// Where the OS does send the release of a resize (Windows), no second one is added: a
/// release right after it could count as a click on what lies under the edge.
#[test]
fn a_release_the_os_sends_is_not_doubled() {
    let mut w = Win::new(1000.0);
    w.frame(vec![]);
    let p = pos2(3.0, 300.0);
    w.frame(vec![Event::PointerMoved(p)]);
    w.frame(vec![button(p, true, egui::PointerButton::Primary)]);
    w.frame(vec![Event::PointerMoved(p + vec2(6.0, 0.0))]);
    // The release arrives with the next input.
    let mut raw = RawInput { events: vec![button(p + vec2(6.0, 0.0), false, egui::PointerButton::Primary)], ..Default::default() };
    release_after_os_resize(&w.ctx, &mut raw);
    assert_eq!(raw.events.len(), 1, "{:?}", raw.events);
    // Or it already arrived in an earlier input: egui has the button up, nothing to end.
    let mut w = Win::new(1000.0);
    w.frame(vec![]);
    w.gesture(&[p, p + vec2(6.0, 0.0)]);
    let mut raw = RawInput::default();
    release_after_os_resize(&w.ctx, &mut raw);
    assert!(raw.events.is_empty(), "{:?}", raw.events);
}

/// #5725: the window manager keeps the button release of an OS move just as of a resize, so
/// egui kept the drag: every other move of the window failed, and so did a resize right after one.
#[test]
fn after_an_os_move_the_bar_and_the_edges_take_the_next_press() {
    /// Press and drag along `path`; the window manager takes over, so no release ever arrives.
    fn handed_off(w: &mut Win, path: &[Pos2]) -> Vec<ViewportCommand> {
        let mut events = vec![vec![Event::PointerMoved(path[0])], vec![button(path[0], true, egui::PointerButton::Primary)]];
        events.extend(path[1..].iter().map(|p| vec![Event::PointerMoved(*p)]));
        events.push(vec![]);
        events.into_iter().flat_map(|e| w.frame(e).viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default()).collect()
    }
    let moves = |cmds: &[ViewportCommand]| cmds.iter().filter(|c| matches!(c, ViewportCommand::StartDrag)).count();
    let mut w = Win::new(1440.0);
    w.frame(vec![]);
    let empty = pos2(980.0, 16.0);
    assert!(w.bar_widgets().iter().all(|r| !r.contains(empty)), "the probe point must be empty bar space");
    let drag = [empty, empty + vec2(12.0, 4.0), empty + vec2(30.0, 8.0)];
    assert_eq!(moves(&handed_off(&mut w, &drag)), 1);
    assert!(!w.ctx.input(|i| i.pointer.primary_down()), "the press ends with the hand-off");
    assert_eq!(moves(&w.gesture(&drag)), 1, "the next move");
    assert_eq!(moves(&handed_off(&mut w, &drag)), 1);
    let edge = pos2(3.0, 300.0);
    assert!(w.gesture(&[edge, edge + vec2(6.0, 0.0)]).contains(&ViewportCommand::BeginResize(ResizeDirection::West)), "a resize after a move");
}

#[test]
fn edge_zones_resize_unless_maximized() {
    use ResizeDirection::*;
    let mut w = Win::new(1000.0);
    w.frame(vec![]);
    for (p, dir) in [
        (pos2(1.0, 300.0), West),
        (pos2(998.0, 300.0), East),
        (pos2(500.0, 1.0), North),
        (pos2(500.0, 598.0), South),
        (pos2(2.0, 2.0), NorthWest),
        (pos2(997.0, 3.0), NorthEast),
        (pos2(3.0, 597.0), SouthWest),
        (pos2(998.0, 598.0), SouthEast),
    ] {
        let cmds = w.gesture(&[p, p + vec2(6.0, 6.0)]);
        assert_eq!(cmds, vec![ViewportCommand::BeginResize(dir)], "at {p:?}");
    }
    // The first motion after those resizes shows the default cursor for a frame (see
    // `reshow_cursor_after_os_resize`).
    w.frame(vec![Event::PointerMoved(pos2(500.0, 300.0))]);
    // One-sided cursors, which every cursor theme has (#1581), anywhere in the 8 pt edge.
    for (p, cursor) in [
        (pos2(7.0, 300.0), CursorIcon::ResizeWest),
        (pos2(993.0, 300.0), CursorIcon::ResizeEast),
        (pos2(500.0, 7.0), CursorIcon::ResizeNorth),
        (pos2(500.0, 593.0), CursorIcon::ResizeSouth),
        (pos2(2.0, 2.0), CursorIcon::ResizeNwSe),
        (pos2(997.0, 3.0), CursorIcon::ResizeNeSw),
        (pos2(3.0, 597.0), CursorIcon::ResizeNeSw),
        (pos2(998.0, 598.0), CursorIcon::ResizeNwSe),
    ] {
        let out = w.frame(vec![Event::PointerMoved(p)]);
        assert_eq!(out.platform_output.cursor_icon, cursor, "at {p:?}");
    }
    let cmds = w.gesture(&[pos2(7.0, 300.0), pos2(13.0, 306.0)]);
    assert_eq!(cmds, vec![ViewportCommand::BeginResize(West)], "the edge zone is 8 pt deep");
    // Clear of the zone and of egui's interact radius around it.
    let cmds = w.gesture(&[pos2(20.0, 300.0), pos2(26.0, 306.0)]);
    assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::BeginResize(_))), "{cmds:?}");
    // Maximized: no zones, and the window's top-right corner is Close.
    w.maximized = true;
    let cmds = w.gesture(&[pos2(1.0, 300.0), pos2(8.0, 300.0)]);
    assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::BeginResize(_))), "{cmds:?}");
    assert!(w.click(pos2(999.5, 0.5)).iter().any(|c| matches!(c, ViewportCommand::Close)));
}

#[test]
fn gtk_layouts_pick_the_buttons_and_their_sides() {
    use Caption::*;
    let s = CaptionStyle::from_gtk_layout(CaptionFamily::Adwaita, "appmenu:minimize,maximize,close");
    assert_eq!((s.left, s.right), (vec![], vec![Minimize, Maximize, Close]));
    // Unknown names are skipped, spaces trimmed, a repeat placed once, a second colon is a name.
    let s = CaptionStyle::from_gtk_layout(CaptionFamily::Adwaita, "close,close , minimize:icon,maximize:close");
    assert_eq!((s.left, s.right), (vec![Close, Minimize], vec![]));
    let s = CaptionStyle::from_gtk_layout(CaptionFamily::Breeze, "minimize,close");
    assert_eq!((s.left, s.right), (vec![Minimize, Close], vec![]), "no colon: GTK puts them all at the start");
    let s = CaptionStyle::from_gtk_layout(CaptionFamily::Breeze, "");
    assert!(s.left.is_empty() && s.right.is_empty());
    assert_eq!(CaptionFamily::parse(" Breeze "), Some(CaptionFamily::Breeze));
    assert_eq!(CaptionFamily::parse("aqua"), None);
    assert_eq!(CaptionStyle::default(), CaptionStyle { family: CaptionFamily::Windows, left: vec![], right: CAPTIONS.to_vec(), icons: None });
}

#[test]
fn linux_buttons_sit_where_the_layout_says_and_drag_the_window() {
    use Caption::*;
    for (family, size, gap, end) in [(CaptionFamily::Adwaita, 24.0, 6.5, 10.0), (CaptionFamily::Breeze, 18.0, 4.5, 8.0)] {
        let width = 1100.0;
        let mut w = Win::new(width);
        w.app.services.caption_style = Some(Box::new(move |_| CaptionStyle { family, left: vec![Close], right: vec![Minimize, Maximize], icons: None }));
        w.frame(vec![]);
        w.frame(vec![]);
        let [min, max, close] = CAPTIONS.map(|c| w.caption(c));
        // Flush with the window's edges, the edge gap outermost, the hit areas touching.
        assert_eq!((close.left(), close.width()), (0.0, end + size + gap), "{family:?}");
        assert_eq!((max.right(), max.width()), (width, gap + size + end), "{family:?}");
        assert_eq!((min.right(), min.width()), (max.left(), 2.0 * gap + size), "{family:?}");
        // GNOME's header bars are taller than Photoshop's bar; KDE's are not.
        if family == CaptionFamily::Adwaita {
            assert_eq!(close.height(), 46.0);
        } else {
            assert!((31.0..46.0).contains(&close.height()), "{close:?}");
        }
        let mark = crate::brand::mark_rect(&w.ctx).expect("the brand mark is painted");
        assert!(mark.left() > close.right(), "the bar starts after the left buttons: {mark:?} {close:?}");
        for a in w.bar_widgets() {
            assert!(a.left() >= close.right() && a.right() <= min.left(), "{a:?} runs into the caption buttons ({family:?})");
        }
        assert!(w.click(max.center()).contains(&ViewportCommand::Maximized(true)));
        assert!(w.click(close.center()).iter().any(|c| matches!(c, ViewportCommand::Close)));
        // A drag that starts on a button moves the window instead of pressing it.
        let at = min.center();
        let cmds = w.gesture(&[at, at + vec2(12.0, 4.0), at + vec2(30.0, 8.0)]);
        assert_eq!(cmds.iter().filter(|c| matches!(c, ViewportCommand::StartDrag)).count(), 1, "{cmds:?}");
        assert!(!cmds.contains(&ViewportCommand::Minimized(true)), "{cmds:?}");
    }
}

/// A side without buttons reserves nothing; without the custom title bar, neither does any.
#[test]
fn only_sides_with_buttons_take_room() {
    let mut w = Win::new(1000.0);
    w.app.services.caption_style = Some(Box::new(|_| CaptionStyle::from_gtk_layout(CaptionFamily::Adwaita, "appmenu:close")));
    assert_eq!(reserved(&w.app, &w.ctx), (0.0, 6.5 + 24.0 + 10.0));
    w.app.custom_titlebar = false;
    assert_eq!(reserved(&w.app, &w.ctx), (0.0, 0.0));
    w.app.custom_titlebar = true;
    w.app.services.caption_style = None;
    assert_eq!(reserved(&w.app, &w.ctx), (0.0, 3.0 * BUTTON_WIDTH));
}

/// Theme glyphs upload once per theme and size; an icon the theme lacks, a wrong-sized mask or
/// an absurd size gives `None`, so the button draws its own glyph.
#[test]
fn theme_icons_are_cached_and_fall_back() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = std::sync::Arc::new(AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&calls);
    let render = move |icon: CaptionIcon, px: u32| {
        counter.fetch_add(1, Ordering::Relaxed);
        match icon {
            CaptionIcon::Close => Some(vec![255; (px * px) as usize]),
            CaptionIcon::Minimize => Some(vec![255; 3]),
            _ => None,
        }
    };
    let icons = CaptionIcons { theme: "Test".into(), render: std::sync::Arc::new(render) };
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(RawInput::default(), |ui| {
        let tex = theme_icon(ui.ctx(), &icons, CaptionIcon::Close, 16).expect("rendered");
        assert_eq!(tex.size(), [16, 16]);
        assert!(theme_icon(ui.ctx(), &icons, CaptionIcon::Close, 16).is_some());
        assert!(theme_icon(ui.ctx(), &icons, CaptionIcon::Restore, 16).is_none());
        assert!(theme_icon(ui.ctx(), &icons, CaptionIcon::Restore, 16).is_none());
        assert!(theme_icon(ui.ctx(), &icons, CaptionIcon::Minimize, 16).is_none(), "a mask of the wrong size");
        assert!(theme_icon(ui.ctx(), &icons, CaptionIcon::Close, 0).is_none());
        assert!(theme_icon(ui.ctx(), &icons, CaptionIcon::Close, 100_000).is_none());
    });
    out.textures_delta.clear();
    assert_eq!(calls.load(Ordering::Relaxed), 3, "each icon and size rendered once; absurd sizes never");
}
