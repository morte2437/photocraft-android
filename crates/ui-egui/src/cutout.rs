//! Keeps the bars' controls out from under a front camera's cutout (a phone's punch hole or
//! notch), splitting a row left and right of it instead of covering it with chrome.
//!
//! The shell reports where the cutouts are ([`set`]); nothing happens without any. Rows step
//! through their controls with [`step`] before each one and [`end`] after the last: a control
//! that last frame would have reached under a cutout starts past it instead. Widths come from
//! the previous frame, so a row of any widgets works: those laid out without a step move with the
//! control before them. A row that scrolls (the options bar) is split as it lies unscrolled, and
//! the gap scrolls with it: splitting it afresh as controls slide under the camera would change
//! its width under the finger.
//!
//! The screen's rounded corners ([`set_corners`]) work the same way at a row's start: a control
//! in the curve moves clear of it. At a row's far end the bar makes room instead ([`row_end`]),
//! and strips along the side edges ask [`column_span`] how far the corners reach into them.

use egui::{Context, Id, Pos2, Rangef, Rect, Ui};
use std::collections::HashMap;

/// Space kept between a cutout and the controls either side of it (points).
const PAD: f32 = 2.0;

/// One of the screen's rounded corners (points): its curve's centre and radius, and which screen
/// corner it rounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    pub centre: Pos2,
    pub radius: f32,
    pub left: bool,
    pub top: bool,
}

impl Corner {
    /// How far into the screen the curve reaches across a line `at` (a y for a row, an x for a
    /// column) from the edge `along` it: the x (`row`) or y where the screen starts to show.
    fn reach(&self, row: bool, at: f32) -> Option<f32> {
        let (across, along, near) = if row { (self.centre.y, self.centre.x, self.left) } else { (self.centre.x, self.centre.y, self.top) };
        let past = if row { self.top } else { self.left };
        // Only lines between the curve's centre and its edge cross the curve.
        let d = if past { across - at } else { at - across };
        if d <= 0.0 {
            return None;
        }
        // Past the curve's end (at the very edge) the screen starts where the curve does.
        let d = d.min(self.radius);
        let half = (self.radius * self.radius - d * d).sqrt();
        Some(if near { along - half } else { along + half })
    }
}

#[derive(Clone, Default)]
struct State {
    holes: Vec<Rect>,
    corners: Vec<Corner>,
    pass: u64,
    rows: HashMap<RowKey, Row>,
}

/// One row's controls: their widths last frame and this frame, and where the current one began.
/// `first` is where the row's first control lies unscrolled (the furthest right it was seen), and
/// `shift` how far the row is scrolled this frame.
#[derive(Clone, Default)]
struct Row {
    last: Vec<f32>,
    now: Vec<f32>,
    open: Option<f32>,
    first: Option<f32>,
    shift: f32,
}

fn id() -> Id {
    Id::unique("photocraft-cutouts")
}

/// The cutouts on screen (points), from the shell each frame; empty when there are none.
pub fn set(ctx: &Context, holes: Vec<Rect>) {
    let holes: Vec<Rect> = holes.into_iter().filter(|r| r.is_finite() && r.is_positive()).collect();
    ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_default::<State>(id());
        if s.holes != holes {
            s.holes = holes;
            s.rows.clear();
        }
    });
}

/// The screen's rounded corners (points), from the shell each frame; empty on square screens.
pub fn set_corners(ctx: &Context, corners: Vec<Corner>) {
    let corners: Vec<Corner> = corners.into_iter().filter(|c| c.centre.is_finite() && c.radius.is_finite() && c.radius > 0.0).collect();
    ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_default::<State>(id());
        if s.corners != corners {
            s.corners = corners;
            s.rows.clear();
        }
    });
}

/// Whether the shell reported any cutout or rounded corner.
pub fn active(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<State>(id()).is_some_and(|s| !s.holes.is_empty() || !s.corners.is_empty()))
}

/// Where a line through a row or column sits for the corners: a bit in from its middle towards
/// the screen's edge, where its controls' outer corners are.
fn probe(band: Rangef, top: bool) -> f32 {
    if top { band.min + band.span() * 0.3 } else { band.max - band.span() * 0.3 }
}

/// The x range a row spanning `band` (y) can use clear of the rounded corners.
fn row_limits(s: &State, band: Rangef) -> Rangef {
    let mut out = Rangef::new(f32::NEG_INFINITY, f32::INFINITY);
    for c in &s.corners {
        if let Some(x) = c.reach(true, probe(band, c.top)) {
            if c.left { out.min = out.min.max(x + PAD) } else { out.max = out.max.min(x - PAD) }
        }
    }
    out
}

/// The y range a column spanning `band` (x) can use clear of the rounded corners, for strips
/// along the screen's side edges (`NEG_INFINITY..INFINITY` when nothing reaches into it).
pub fn column_span(ctx: &Context, band: Rangef) -> Rangef {
    ctx.data(|d| {
        let mut out = Rangef::new(f32::NEG_INFINITY, f32::INFINITY);
        for c in d.get_temp::<State>(id()).map(|s| s.corners).unwrap_or_default() {
            if let Some(y) = c.reach(false, probe(band, c.left)) {
                if c.top { out.min = out.min.max(y + PAD) } else { out.max = out.max.min(y - PAD) }
            }
        }
        out
    })
}

/// Where `ui`'s row has to end to stay clear of the rounded corners at its far end, if one
/// reaches into it (a bar that can't scroll gives up space, one that can makes room to scroll
/// its last control clear).
pub fn row_end(ui: &Ui) -> Option<f32> {
    let end = ui.ctx().data(|d| d.get_temp::<State>(id()).map(|s| row_limits(&s, ui.max_rect().y_range()).max))?;
    end.is_finite().then_some(end)
}

/// What `ui`'s row reaches under, left to right: the cutouts and a rounded corner at its start
/// (left-to-right rows only).
fn holes(ui: &Ui, s: &State) -> Vec<Rect> {
    if ui.layout().main_dir() != egui::Direction::LeftToRight {
        return Vec::new();
    }
    let band = ui.max_rect().y_range();
    let mut out: Vec<Rect> = s.holes.iter().copied().filter(|h| h.min.y < band.max && h.max.y > band.min).collect();
    let start = row_limits(s, band).min;
    if start.is_finite() {
        // Already padded: the hole ends where the row may start.
        out.push(Rect::from_x_y_ranges(f32::NEG_INFINITY..=start - PAD, band));
    }
    out.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
    out
}

/// A row: its top, and the left and right of what shows of it (its bar, or a scrolling row's
/// viewport). A bar that moves or resizes (the Shortcut Pad switching sides, or hidden) is a new
/// row, learned afresh: its old widths and unscrolled start no longer apply.
type RowKey = (i32, i32, i32);

fn row_key(ui: &Ui) -> RowKey {
    let clip = ui.clip_rect();
    (ui.max_rect().top().round() as i32, clip.left().round() as i32, clip.right().round() as i32)
}

/// On frame `pass`'s first use: last frame's widths become the ones to go by.
fn new_pass(pass: u64, s: &mut State) {
    if s.pass != pass {
        s.pass = pass;
        // Rows not laid out last frame are gone (a bar that moved is a new row).
        s.rows.retain(|_, row| !row.now.is_empty());
        for row in s.rows.values_mut() {
            row.last = std::mem::take(&mut row.now);
            row.open = None;
        }
    }
}

/// Before a control in a row: skips past a cutout this control would reach under.
pub fn step(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    // Read before taking the data lock: the context's own lock can't be taken inside it.
    let pass = ctx.cumulative_pass_nr();
    let gap = ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_default::<State>(id());
        let holes = holes(ui, s);
        if holes.is_empty() {
            return None;
        }
        new_pass(pass, s);
        let row = s.rows.entry(row_key(ui)).or_default();
        let x = ui.cursor().left();
        if let Some(start) = row.open.take() {
            row.now.push((x - start).max(0.0));
        }
        if row.now.is_empty() {
            let first = row.first.map_or(x, |f| f.max(x));
            row.first = Some(first);
            row.shift = first - x;
        }
        // Where this control lies unscrolled.
        let home = x + row.shift;
        let width = row.last.get(row.now.len()).copied().unwrap_or(0.0).max(1.0);
        let mut gap = 0.0;
        for h in &holes {
            let at = home + gap;
            if at < h.max.x + PAD && at + width > h.min.x - PAD {
                gap = h.max.x + PAD - home;
            }
        }
        row.open = Some(x + gap);
        Some(gap)
    });
    if let Some(gap) = gap.filter(|g| g.is_finite() && *g > 0.0) {
        ui.add_space(gap);
    }
}

/// After a row's last control: closes its width. `ui` is the row, or one holding it (the row's
/// right end is the right end of what `ui` holds).
pub fn end(ui: &Ui) {
    let ctx = ui.ctx().clone();
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_default::<State>(id());
        if s.holes.is_empty() && s.corners.is_empty() {
            return;
        }
        new_pass(pass, s);
        let x = ui.min_rect().right();
        let band = ui.max_rect().y_range();
        let (_, left, right) = row_key(ui);
        for (&(top, l, r), row) in &mut s.rows {
            if band.contains(top as f32)
                && (l, r) == (left, right)
                && let Some(start) = row.open.take()
            {
                row.now.push((x - start).max(0.0));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    /// A row of six 50-point buttons at the top of a 400-point screen, with a cutout at
    /// x 120–160 reaching into it (when `hole`). Returns the buttons' rects.
    fn row(ctx: &Context, hole: bool) -> Vec<Rect> {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))), ..Default::default() };
        let mut rects = Vec::new();
        let mut out = ctx.run_ui(raw, |ui| {
            set(ui.ctx(), if hole { vec![Rect::from_min_max(pos2(120.0, 0.0), pos2(160.0, 20.0))] } else { Vec::new() });
            let bar = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 30.0));
            let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
            bar.spacing_mut().item_spacing.x = 0.0;
            for _ in 0..6 {
                step(&mut bar);
                rects.push(bar.allocate_exact_size(vec2(50.0, 24.0), egui::Sense::click()).0);
            }
            end(&bar);
        });
        out.textures_delta.clear();
        rects
    }

    #[test]
    fn controls_split_left_and_right_of_the_camera() {
        let ctx = Context::default();
        let plain = row(&ctx, false);
        assert_eq!(plain[2].left(), 100.0, "no cutout: side by side");
        // The first frame learns the widths, the next one splits the row, and it stays split.
        row(&ctx, true);
        for _ in 0..3 {
            let split = row(&ctx, true);
            assert_eq!(split[..2], plain[..2], "left of the camera: unchanged");
            assert_eq!(split[2].left(), 160.0 + PAD, "the button that reached under it starts past it");
            assert_eq!(split[3].left(), 210.0 + PAD, "the rest follow");
            assert!(split.iter().all(|r| r.right() <= 120.0 - PAD || r.left() >= 160.0 + PAD), "{split:?}");
        }
        // Gone again (a rotation): back side by side.
        assert_eq!(row(&ctx, false), plain);
    }

    /// A scrolled row keeps the split it has unscrolled: its gap moves with it, so its width
    /// never changes under a swipe.
    #[test]
    fn a_scrolled_row_keeps_its_split() {
        let ctx = Context::default();
        let frame = |scroll: f32| {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))), ..Default::default() };
            let mut rects = Vec::new();
            let mut out = ctx.run_ui(raw, |ui| {
                set(ui.ctx(), vec![Rect::from_min_max(pos2(120.0, 0.0), pos2(160.0, 20.0))]);
                let bar = Rect::from_min_size(pos2(-scroll, 0.0), vec2(800.0, 30.0));
                let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
                bar.spacing_mut().item_spacing.x = 0.0;
                for _ in 0..10 {
                    step(&mut bar);
                    rects.push(bar.allocate_exact_size(vec2(50.0, 24.0), egui::Sense::click()).0);
                }
                end(&bar);
            });
            out.textures_delta.clear();
            rects
        };
        frame(0.0);
        let still = frame(0.0);
        assert_eq!(still[2].left(), 160.0 + PAD);
        for scroll in [10.0, 37.0, 80.0, 133.0, 0.0] {
            let r = frame(scroll);
            let moved: Vec<Rect> = r.iter().map(|r| r.translate(vec2(scroll, 0.0))).collect();
            assert_eq!(moved, still, "scrolled {scroll}: the same row, moved");
        }
    }

    #[test]
    fn rows_clear_of_the_camera_and_bad_reports_change_nothing() {
        let ctx = Context::default();
        let raw = || egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))), ..Default::default() };
        for _ in 0..2 {
            let mut out = ctx.run_ui(raw(), |ui| {
                set(ui.ctx(), vec![Rect::from_min_max(pos2(120.0, 0.0), pos2(160.0, 20.0)), Rect::NAN, Rect::NOTHING]);
                // A row lower down the screen.
                let low = Rect::from_min_size(pos2(0.0, 100.0), vec2(400.0, 30.0));
                let mut low = ui.new_child(egui::UiBuilder::new().max_rect(low).layout(egui::Layout::left_to_right(egui::Align::Center)));
                low.spacing_mut().item_spacing.x = 0.0;
                for i in 0..6 {
                    step(&mut low);
                    let r = low.allocate_exact_size(vec2(50.0, 24.0), egui::Sense::click()).0;
                    assert_eq!(r.left(), i as f32 * 50.0);
                }
                end(&low);
            });
            out.textures_delta.clear();
        }
    }

    /// A 400 x 300 screen with 30-point rounded corners.
    fn rounded() -> Vec<Corner> {
        vec![
            Corner { centre: pos2(30.0, 30.0), radius: 30.0, left: true, top: true },
            Corner { centre: pos2(370.0, 30.0), radius: 30.0, left: false, top: true },
            Corner { centre: pos2(370.0, 270.0), radius: 30.0, left: false, top: false },
            Corner { centre: pos2(30.0, 270.0), radius: 30.0, left: true, top: false },
        ]
    }

    /// Lays out three 50-point buttons in a row at `bar`, two frames running, and returns the
    /// last frame's rects and where the row has to end.
    fn corner_row(ctx: &Context, bar: Rect) -> (Vec<Rect>, Option<f32>) {
        let mut got = (Vec::new(), None);
        for _ in 0..2 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))), ..Default::default() };
            let mut rects = Vec::new();
            let mut end_at = None;
            let mut out = ctx.run_ui(raw, |ui| {
                set_corners(ui.ctx(), rounded());
                let mut row = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
                row.spacing_mut().item_spacing.x = 0.0;
                for _ in 0..3 {
                    step(&mut row);
                    rects.push(row.allocate_exact_size(vec2(50.0, 20.0), egui::Sense::click()).0);
                }
                end(&row);
                end_at = row_end(&row);
            });
            out.textures_delta.clear();
            got = (rects, end_at);
        }
        got
    }

    #[test]
    fn a_rounded_corner_moves_only_what_sits_in_its_curve() {
        let ctx = Context::default();
        // A bar along the top edge: its first button moves out of the curve, the rest follow,
        // and its far end stops short of the other corner's curve.
        let (top, end_at) = corner_row(&ctx, Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 30.0)));
        let curve = 30.0 - (30.0f32 * 30.0 - 21.0 * 21.0).sqrt();
        assert!((top[0].left() - (curve + PAD)).abs() < 0.1, "{top:?}");
        assert_eq!(top[1].left(), top[0].right());
        let end_at = end_at.expect("the far corner reaches into the bar");
        assert!((end_at - (400.0 - curve - PAD)).abs() < 0.01, "{end_at}");
        // A bar starting clear of the curve, and one lower down, stay put.
        let (inset, _) = corner_row(&ctx, Rect::from_min_size(pos2(60.0, 0.0), vec2(300.0, 30.0)));
        assert_eq!(inset[0].left(), 60.0);
        let (low, end_at) = corner_row(&ctx, Rect::from_min_size(pos2(0.0, 100.0), vec2(400.0, 30.0)));
        assert_eq!(low[0].left(), 0.0);
        assert_eq!(end_at, None);
    }

    #[test]
    fn a_strip_along_the_side_edge_ends_clear_of_both_corners() {
        let ctx = Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| set_corners(ui.ctx(), rounded()));
        out.textures_delta.clear();
        // A 60-point strip on the left edge: the curves reach into its outer side.
        let strip = column_span(&ctx, Rangef::new(0.0, 60.0));
        let curve = 30.0 - (30.0f32 * 30.0 - 12.0 * 12.0).sqrt();
        assert!((strip.min - (curve + PAD)).abs() < 0.01, "{strip:?}");
        assert!((strip.max - (300.0 - curve - PAD)).abs() < 0.01, "{strip:?}");
        // A column in the middle of the screen: nothing.
        let middle = column_span(&ctx, Rangef::new(150.0, 200.0));
        assert!(middle.min.is_infinite() && middle.max.is_infinite());
        // Square corners (or a bad report): nothing.
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| set_corners(ui.ctx(), vec![Corner { centre: pos2(f32::NAN, 0.0), radius: 30.0, left: true, top: true }]));
        out.textures_delta.clear();
        assert!(!active(&ctx));
    }

    /// The Shortcut Pad switching sides moves a bar under the camera: it splits where it is now,
    /// not where it was.
    #[test]
    fn a_bar_that_moves_splits_afresh() {
        let ctx = Context::default();
        let frame = |left: f32| {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 300.0))), ..Default::default() };
            let mut rects = Vec::new();
            let mut out = ctx.run_ui(raw, |ui| {
                set(ui.ctx(), vec![Rect::from_min_max(pos2(220.0, 0.0), pos2(260.0, 20.0))]);
                let bar = Rect::from_min_size(pos2(left, 0.0), vec2(400.0, 30.0));
                let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
                bar.set_clip_rect(bar.max_rect());
                bar.spacing_mut().item_spacing.x = 0.0;
                for _ in 0..6 {
                    step(&mut bar);
                    rects.push(bar.allocate_exact_size(vec2(50.0, 24.0), egui::Sense::click()).0);
                }
                end(&bar);
            });
            out.textures_delta.clear();
            rects
        };
        for left in [100.0, 0.0, 100.0] {
            frame(left);
            for _ in 0..2 {
                let r = frame(left);
                assert!(r.iter().all(|r| r.right() <= 220.0 - PAD || r.left() >= 260.0 + PAD), "bar at {left}: {r:?}");
                // Only the button that reached under the camera moved: the gap is no wider.
                let gap = r.windows(2).map(|w| w[1].left() - w[0].right()).fold(0.0, f32::max);
                assert!(gap <= 40.0 + 2.0 * PAD + 50.0, "bar at {left}: {r:?}");
                assert_eq!(r[0].left(), left);
            }
        }
    }
}
