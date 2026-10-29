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

use egui::{Context, Id, Rect, Ui};
use std::collections::HashMap;

/// Space kept between a cutout and the controls either side of it (points).
const PAD: f32 = 2.0;

#[derive(Clone, Default)]
struct State {
    holes: Vec<Rect>,
    pass: u64,
    rows: HashMap<i32, Row>,
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

/// Whether the shell reported any cutout.
pub fn active(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<State>(id()).is_some_and(|s| !s.holes.is_empty()))
}

/// The cutout `ui`'s row reaches under, if any (left-to-right rows only).
fn hole(ui: &Ui, s: &State) -> Option<Rect> {
    if s.holes.is_empty() || ui.layout().main_dir() != egui::Direction::LeftToRight {
        return None;
    }
    let band = ui.max_rect().y_range();
    s.holes.iter().copied().find(|h| h.min.y < band.max && h.max.y > band.min)
}

fn row_key(ui: &Ui) -> i32 {
    ui.max_rect().top().round() as i32
}

/// On frame `pass`'s first use: last frame's widths become the ones to go by.
fn new_pass(pass: u64, s: &mut State) {
    if s.pass != pass {
        s.pass = pass;
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
        let h = hole(ui, s)?;
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
        let gap = (home < h.max.x + PAD && home + width > h.min.x - PAD).then_some(h.max.x + PAD - home);
        row.open = Some(x + gap.unwrap_or(0.0));
        gap
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
        if s.holes.is_empty() {
            return;
        }
        new_pass(pass, s);
        let x = ui.min_rect().right();
        let band = ui.max_rect().y_range();
        for (top, row) in &mut s.rows {
            if band.contains(*top as f32)
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
}
