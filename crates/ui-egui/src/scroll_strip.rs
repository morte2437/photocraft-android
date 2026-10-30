//! Bars that scroll sideways when they run past the window (the options bar, a phone's menu bar).
//! A touch screen scrolls them with a sideways swipe ([`scroll`]), set on the bar's scroll area
//! itself rather than sent as the wheel, which a dropdown under the finger would take.
//!
//! A bar wraps its scroll area with [`begin`] and reports how it laid out with [`record`]; the
//! touch router asks for the [`strips`] that overflow, as laid out last frame.

use egui::{Context, Id, Rect, ScrollArea};
use std::collections::HashMap;

/// A strip as laid out: where it is, its scroll area's id and how far it scrolls, and the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Strip {
    rect: Rect,
    area: Id,
    max: f32,
    pass: u64,
}

fn strips_id() -> Id {
    Id::unique("scroll-strips")
}

/// Where a swipe asked strip `key` to scroll to, for its next frame.
fn scroll_to_id(key: Id) -> Id {
    key.with("scroll-strip-to")
}

/// Strip `key`'s scroll area, scrolled where a swipe asked.
pub fn begin(ctx: &Context, key: Id, area: ScrollArea) -> ScrollArea {
    match ctx.data_mut(|d| d.remove_temp::<f32>(scroll_to_id(key))) {
        Some(x) => area.horizontal_scroll_offset(x),
        None => area,
    }
}

/// Strip `key` laid out as `out`: a swipe can scroll it while it overflows.
pub fn record<R>(ctx: &Context, key: Id, out: &egui::scroll_area::ScrollAreaOutput<R>) {
    let max = out.content_size.x - out.inner_rect.width();
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let all = d.get_temp_mut_or_default::<HashMap<Id, Strip>>(strips_id());
        if max > 1.0 {
            all.insert(key, Strip { rect: out.inner_rect, area: out.id, max, pass });
        } else {
            all.remove(&key);
        }
    });
}

/// The strips that overflow, as laid out last frame (a bar no longer drawn drops out).
pub fn strips(ctx: &Context) -> Vec<Rect> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<HashMap<Id, Strip>>(strips_id()))
        .unwrap_or_default()
        .into_values()
        .filter(|s| pass.saturating_sub(s.pass) <= 2)
        .map(|s| s.rect)
        .collect()
}

/// Scroll the strip laid out at `strip` by `dx` points (positive: the content follows a finger
/// moving right).
pub fn scroll(ctx: &Context, strip: Rect, dx: f32) {
    let found = ctx.data(|d| d.get_temp::<HashMap<Id, Strip>>(strips_id())).unwrap_or_default().into_iter().find(|(_, s)| s.rect == strip);
    let Some((key, s)) = found else { return };
    let now = egui::scroll_area::State::load(ctx, s.area).map_or(0.0, |st| st.clamped_offset().x);
    let pending = ctx.data(|d| d.get_temp::<f32>(scroll_to_id(key))).unwrap_or(now);
    let to = (pending - dx).clamp(0.0, s.max.max(0.0));
    // Applied by the strip's scroll area when it next draws (egui's state can't be set directly).
    ctx.data_mut(|d| d.insert_temp(scroll_to_id(key), to));
    ctx.request_repaint();
}
