# Canvas view rotation shortcuts: issue #4911

Refs https://github.com/storytold/photocraft/issues/4911

View > Rotate View Left and Rotate View Right turn the active canvas camera by 15 degrees.
The default bindings are Alt+Shift+, and Alt+Shift+. (Option+Shift on macOS).
They can be changed or removed in Edit > Keyboard Shortcuts, under View.

Each key press, including keyboard repeats, adds one relative step. An existing 7-degree
angle becomes 22 degrees after rotating right; it is not snapped to the nearest grid angle.
Angles wrap in the existing (-180, 180] range. Left/right describe the visible direction,
including when View > Flip Horizontal is enabled.

The commands change view state only. They preserve document pixels, history, zoom, center
and other tabs. Existing Reset View returns the angle to zero. Text fields retain the
default bindings, and modal dialogs block them through the normal shortcut dispatcher.

The control channel can invoke view.rotateLeft or view.rotateRight with an empty params
object, through the same shell-command route as view.rotateView and view.resetView.
No unmerged Shift+middle-drag work from #4042 is included.

Regression coverage in rotate_view tests checks relative steps, wrapping, reset,
flipped-view direction, unchanged document/history/other tabs, empty-document and malformed
parameter errors, repeated key events, rebinding, persisted preferences and modal suppression.
