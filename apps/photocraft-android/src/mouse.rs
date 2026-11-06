//! A mouse wheel or a trackpad's scroll (Samsung DeX, a Bluetooth mouse) as egui events. winit
//! drops Android's `ACTION_SCROLL`, so the Activity passes it to the tablet hook and each frame
//! turns what came in into these (Android port #29). And the mouse pointer's shape: Android
//! shows its own arrow unless the view sets another, so the brush's outline had the arrow on top
//! of it in DeX (#27).

use egui::{CursorIcon, Event, Modifiers, MouseWheelSource, MouseWheelUnit, Pos2, TouchPhase, vec2};

/// The pointer moving to `pos` (points), then a wheel of `h` steps right and `v` steps up
/// (`AXIS_HSCROLL`, `AXIS_VSCROLL`). egui's delta is the way the content moves: up a step
/// scrolls the content down, right a step moves it left. The modifiers are filled in later.
pub fn wheel(pos: Pos2, h: f32, v: f32) -> [Event; 2] {
    [
        Event::PointerMoved(pos),
        Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(-h, v),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
            source: MouseWheelSource::Wheel,
        },
    ]
}

/// Android's `PointerIcon` type for egui's cursor: `TYPE_NULL` (0) hides it, for tools that draw
/// their own (the brush outline).
pub fn pointer_icon(c: CursorIcon) -> i32 {
    use CursorIcon::*;
    match c {
        None => 0,
        ContextMenu => 1001,
        PointingHand => 1002,
        Help => 1003,
        Progress | Wait => 1004,
        Cell => 1006,
        Crosshair => 1007,
        Text => 1008,
        VerticalText => 1009,
        Alias => 1010,
        Copy => 1011,
        NoDrop | NotAllowed => 1012,
        Move | AllScroll => 1013,
        ResizeHorizontal | ResizeEast | ResizeWest | ResizeColumn => 1014,
        ResizeVertical | ResizeNorth | ResizeSouth | ResizeRow => 1015,
        ResizeNeSw | ResizeNorthEast | ResizeSouthWest => 1016,
        ResizeNwSe | ResizeNorthWest | ResizeSouthEast => 1017,
        ZoomIn => 1018,
        ZoomOut => 1019,
        Grab => 1020,
        Grabbing => 1021,
        Default => 1000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn a_wheel_step_scrolls_like_a_desktop_wheel() {
        let [moved, wheel] = wheel(pos2(10.0, 20.0), 0.0, 1.0);
        assert_eq!(moved, Event::PointerMoved(pos2(10.0, 20.0)), "under the pointer");
        let Event::MouseWheel { unit, delta, source, .. } = wheel else { panic!("{wheel:?}") };
        assert_eq!((unit, source), (MouseWheelUnit::Line, MouseWheelSource::Wheel));
        assert_eq!(delta, vec2(0.0, 1.0), "up: the content comes down, as winit's LineDelta");
        let [_, Event::MouseWheel { delta, .. }] = super::wheel(pos2(0.0, 0.0), 2.0, -1.0) else { panic!() };
        assert_eq!(delta, vec2(-2.0, -1.0), "right and down");
    }

    /// #27: a brush hides the arrow (it draws its outline); other shapes get Android's own.
    #[test]
    fn cursors_map_to_android_pointer_icons() {
        assert_eq!(pointer_icon(CursorIcon::None), 0);
        assert_eq!(pointer_icon(CursorIcon::Default), 1000);
        assert_eq!(pointer_icon(CursorIcon::Crosshair), 1007);
        assert_eq!(pointer_icon(CursorIcon::Text), 1008);
        assert_eq!(pointer_icon(CursorIcon::Grabbing), 1021);
        assert_eq!(pointer_icon(CursorIcon::ResizeHorizontal), pointer_icon(CursorIcon::ResizeColumn));
    }
}
