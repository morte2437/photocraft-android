# Smart Filters in the Layers panel

A Smart Object with filters has a rings indicator and a disclosure arrow beside
its name. The arrow hides or shows both its layer effects and Smart Filters; it
does not disable them or change the document. Alt-click applies that view choice
to all layers with effects or Smart Filters.

The Smart Filters header shows the shared filter mask and the stack's visibility
control. Filters appear newest first, while the engine evaluates them from bottom
to top. Each filter has its own visibility control.

Double-click a filter's name to edit its saved parameters in its existing editor.
OK updates that filter; it does not append a second copy. Parameter-free filters
such as Invert have no settings dialog. Camera Raw retains its dedicated editor.

Double-click the sliders icon at the right of a filter to open **Blending Options**.
Mode and Opacity belong to that filter, independently of the layer and other
filters. Opacity is shown as a percentage (1% means 0.01). Preview changes the
temporary canvas composite and the dialog's image viewport. Zoom controls and
scrolling affect only that viewport. The viewport uses a bounded image thumbnail;
it is a navigation preview, not a full-resolution detail inspection.

OK applies one undoable edit. Cancel or Escape leaves the document untouched.
Editors capture their document snapshot, revision, layer and filter. Reopening
the same saved document also invalidates a retained dialog. A document switch,
Undo/Redo, filter deletion/reorder or pending document job requires reopening the
dialog rather than applying settings to a changed target.

Ctrl+I / Image > Adjustments > Invert on Smart Object content uses the existing
engine adjustment command to add an Invert Smart Filter, preserving the source.
When a layer mask or channel is targeted, its existing adjustment semantics apply.

## Storage and current limits

`.pcraft` stores the complete Smart Filter stack, parameters, shared mask,
visibility and per-filter blending. PSD only supports the filters mapped by
`crates/io/src/smart_map.rs`: Invert and Smart Sharpen currently lack live PSD
mappings. PSD export can warn and omit unmapped live filters. This interface
change does not extend the PSD mapping table or unknown-descriptor support.

Preview reuses the native filter worker and reduced document pipeline. Some
filters require full-resolution previews. Committing edits and toggling filter
eyes retain the cached stages and background jobs already supplied by the engine
(PR #2739). This change does not add a new GPU filter implementation or establish
large-image performance budgets.

Filter opacity interpolates premultiplied input and filtered results, so
alpha-preserving filters retain translucent coverage at partial strength. This
also applies to Normal mode when a filter changes coverage. Non-Normal modes
use source-over compositing when the filtered surface changes alpha, preserving
input-only and filtered-only pixels around blur edges. The choice applies to
the whole filtered surface. This is scoped to Smart Filters; global layer
compositing is unchanged. Dissolve
currently follows the inherited Smart Filter color-blending path without
per-pixel coverage sampling; exact Dissolve parity remains a separate task.
