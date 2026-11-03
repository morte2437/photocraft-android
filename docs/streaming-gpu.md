# Viewport pages (GPU canvas for over-budget documents)

A document's display texture set (every tile, every mip level) is held to the per-document
canvas budget (`gpu_canvas::budget::canvas_budget`: a quarter of physical memory, capped by a
discrete GPU's own memory, from the detected RAM and VRAM, so also with GPU Memory on Auto).
A document whose texture set fits keeps it, with everything around it unchanged: the layer
budget (LayerFit), deferred refreshes (`DEFER_PX`), background redraw and the compositor's
output admission. There is no pixel-count cutoff.

A document whose texture set doesn't fit used to fall back to the CPU canvas
(`canvasTooLarge`). It is now drawn on the GPU canvas from viewport pages
(`ui-egui/src/canvas/viewport_pages.rs`, `gpu_canvas/pages.rs`):

- Pages are 512 texels square at a power-of-two reduction chosen so a texel covers at most one
  screen pixel; a view asks for at most 256 pages and coarsens beyond that. Each page is rendered
  from its core plus a one-texel apron (`photocraft_compose::viewport`), so independently rendered
  pages filter their shared edges identically; the shader discards the apron.
- Two background workers render pages (over all documents), centre of the view first. At a
  reduction, a faithful document shows a quick nearest-sample preview of every page in view first,
  then the exact (area-averaged) page. Work for a page that left the view, changed zoom or was
  edited is cancelled, and a late result of older work is dropped.
- Uploads are bounded per frame (2 pages, 32 MB of texels), so staging stays bounded however
  many pages finish at once.
- Pages are held to half the per-document canvas budget (the rest is headroom for the
  compositor); the least recently wanted pages are evicted first and wanted pages never are.
  The global wgpu resource budget and the split of the GPU memory budget between documents are
  not changed.
- An edit dirties only the pages its damage (plus the layers' effect reach) touches; a dirty page
  stays on screen until its replacement is uploaded. Invalidation costs the pages held, not the
  document's layers or tiles.
- When no page fits the budget, or a page fails to render or upload, the document is drawn on the
  CPU canvas as before (`canvasTooLarge`). A failed page is retried when the scratch storage or
  memory configuration changes. Closing the document, a GPU fault or dropping the GPU canvas
  cancels its work and frees its pages. Pattern Preview and size-changing previews keep main's
  path.
