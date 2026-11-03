# Viewport streaming

Large documents render original-pixel viewport pages instead of a document-sized canvas texture.
Visible pages use a power-of-two sampling level appropriate to the screen density; zooming in
requests finer pages from the original document. The document's pixels and dimensions are unchanged.
The CPU renderer follows the same page requests when the GPU is unavailable.

Requests and ready pages belong to document IDs and views, rather than shifting tab indices.
Zoom, edits and display changes invalidate or rebase pages; obsolete workers cancel, and their
completion cannot replace current work. Closing a tab removes its stream. CPU page caches share
a bounded allowance across views; persistent GPU input and output pages have separate caps.

GPU uploads submit bounded batches. Staging limits account for padded readback/transfer sizes and
keep transient buffers bounded independently of resident texture memory. Resident-only composition
uses a short nonblocking eviction barrier; disk-backed pages compose on a worker instead of
faulting pixel data on the UI thread. Incomplete output keeps requesting current visible pages
until it finishes or reports an actionable error.

The integration preserves upstream's GPU rotation uniforms, asynchronous filter previews and
full-canvas background redraw. Streaming pages and complete redraw textures have distinct cache
identities, so a partial page cache cannot stand in for a complete image. Pattern Preview retains
the upstream full-canvas/proxy route for its repeated copies; ordinary large canvases use pages.
Synthetic regressions cover oversized staging/readback allocations, coarse/fine page replacement,
cancelled workers, eviction and tab switching. Native stress uses Earth.psb with the 2K/4K/8K/16K
Milky Way EXR variants, rapid magnifier clicks and tab changes, then the original-size 64K EXR.
