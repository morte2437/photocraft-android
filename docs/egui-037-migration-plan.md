# PhotoCraft: egui 0.37 migration plan

Status update (2026-10-25): local implementation includes main `b278ff72459cbf9754863365edd822ba2248aa07`, preserving its JPEG dependency, Affinity import and raster changes. All nine CI stages passed: 11,590 passing test executions, zero failed, 91 ignored across 93 summaries; the ignored panic hunt, strict lint and WASM compilation passed. The current-main integration received independent review. Native/browser GUI, native IME, PRIMARY, packaged WASM size, real-world 0.36 settings and paired application performance remain open. Publication requires separate explicit approval; human review and human merge are required.

The original planning record below is historical.

Current status — 2026-10-25: the port is implemented and reconciled with main
`486afd82cce99255e999224bcfe28de00fd766aa`. The full managed CI run passed all nine stages:
formatting, strict clippy, layers, scorecard, workspace tests, the ignored panic hunt,
doctests, WASM compilation and the translation report. It recorded 11,580 passed and
91 ignored test executions across 93 summaries, with zero failures. These totals include
overlapping test invocations and are not a count of unique tests.

The production graph uses canonical GitHub sources: egui `6d518fc799681571963842f95b55a6b68431b15a`,
winit `a517b3846ee8457d84ee4975110bfda6797c44dc`, and shared UI/print
`25da1314d443171b4adeaad2cd9454b273f35c6f`. The checked-in Cargo-generated lock has SHA256
`714ce3bc78f32bc35bd69e27c4a785ac2f38983c77df9896dcddc8e5fa79ccb2`; no absolute path
patch is required. The lifecycle/capture port, Type-tool surrounding-text and byte-deletion
adapter, reset acknowledgement handling, and selection-action changes are implemented.

The research and progress entries below retain their original sequence. Their statements
about missing APIs, private path patches, unresolved pins and absent build results are historical
and superseded by this status. Native/browser GUI and IME acceptance, PRIMARY behavior, packaged
WASM size, broad visual/accessibility sweeps and paired app performance remain open. The synthetic
settings fixtures qualify migration rules, not a captured real-world 0.36 save. Human review
and human merge are required.
Revision 2 (2026-10-23): resolves the independent review at
`plan-reviews/photocraft-v1.md`, incorporates the shared E21b/W15P contracts and the
lifecycle source audit, and removes the superseded sampling, PRIMARY, screenshot and
rollback wording. The reviewed v1 document hash is retained in the evidence ledger;
this revision needs its own independent review before publication.
Baseline: `ff4738810134d7f7e68b49f2e4f231b686282ff3` ("Release 0.14.0", 2026-10-22).
`Cargo.lock` baseline: sha256 `36a1de83fea16781dc7186d33d8064cb5dfa878a2a0847228737640b1e64e2d0`,
git blob `5c96f56119b6ddb0feb2da9cff3e23232f2dc63c`.

Target: egui/eframe/epaint family **0.37.0**, not upstream `main`. Upstream tag object
`418687fa18fdac8a9b1b6c8f4a2eb1afd2df4c47` points at commit
`841a328ba2dd8f6996beb7189ebb7fc00e668a2a`. The audited Craft fork candidate is
`aiongg/egui` `craft/0.37` at `a4f9a17f64a6bae61350fb85804c9df0d9865916`; the audited
winit candidate is `aiongg/winit` `craft/0.30.13-lang` at
`8b35388bd719e8cc86a95971ba0bc026c9a8f058`, based on `storytold/winit`
`craft/0.30.13` `ffae5198770d7730e718043f035a772b7dbc3b1b`.

Those revisions are **historical, read-only evidence**. They are not accepted revisions and
are not publication authority. Every implementation claim below must be re-run against the
final published revisions.

Research basis: upstream tag `0.37.0` source and changelogs in the read-only
checkout `/home/codex/work/CraftApps/egui037-migration-20261022/egui`, the
candidate fork commits and `PATCHES.md`, and the PhotoCraft source at the
baseline SHA. No web research was needed for the API facts below; upstream
release notes and source are the primary evidence. Historical fork/audit notes
were used only to locate refs, not as acceptance evidence.

Capture acceptance follows the suite contract: a queued-not-issued hidden request and
each settling frame must request `RequestPaintWhileHidden` from logic; only an issued
screenshot command forces paint itself. Callback arrival has no implicit wake, so the
live request owns the viewport waker. Record known issuance UI bounds and a geometry
epoch, reject/retry any intervening resize/DPI/zoom/surface change even if it changes
back, and derive pixel scale from returned dimensions only under an unchanged epoch.
Default limits are 4 requests, 128 MiB/image and 256 MiB aggregate. Count reserved
in-flight and completed images together.


## 1. Decision summary

1. Move directly to 0.37.0; do not port 0.36.2 patches to a new 0.36.x branch.
2. Keep the existing `App::logic` / `App::ui` split. PhotoCraft already uses it
   (`crates/ui-egui/src/lib.rs:2244` and `:2371`), so there is no blanket
   `Context`-bridge migration. The work is preserving the split across hidden
   windows, screenshots, control input and multiple passes.
3. Pin the whole egui family and winit by immutable git revisions. A consumer's
   `[patch]` section does not propagate to dependency manifests, so the root
   workspace must patch every family crate it resolves.
4. Keep the existing `eframe` opt-out from default features. Add only the features
   PhotoCraft actually needs: `wgpu`, `accesskit`, `wayland`, `x11`, `persistence`,
   and the new `clipboard`. Add `monochrome_emoji_fonts` to the
   `epaint_default_fonts` build-dependency. Do not enable `system_fonts` in the first
   cut; the existing `cjk_fonts` system-provider path is measured separately (#8512).
5. Reproduce 0.37's `egui-icons` face and family order. The current build script
   fails at once because `NOTO_EMOJI_REGULAR` and `EMOJI_ICON` are now
   feature-gated (`crates/ui-egui/build.rs:42`, `:44`), and `EGUI_ICONS` is
   unconditional. The glyph, accessible-name and feature fixes are part of the
   first green PR, not a later fixture sweep.
6. Migrate `WidgetType` to `accesskit::Role` (the old name is a deprecated alias;
   several variants were renamed), `Id::new` to `Id::unique`, `Ui::id` to
   `Ui::scope_id`, panel IDs to `AsIdSalt`, screenshots to a bounded
   `ScreenshotCallback` completion slot, and remove `UserData` /
   `Event::Screenshot`. `Ui::scope_id` is the renamed old `Ui::id`;
   `Ui::unique_id` is a new globally unique but **unstable** id and must never be
   used for persisted state.
7. Migrate two persisted schemas deliberately, in the first compatible PR:
   `egui::PanelState` IDs and `egui_winit::WindowSettings`. `PanelState` gains a
   parent-scoped id; `WindowSettings` changes from flat fields to
   `normal_geometry`. The panel migration is additive (the old key stays), but the
   window rewrite is one-way. Preserve panel widths and window size/fullscreen/
   maximized state explicitly; document the rollback loss of window position.
8. Adopt the language fork as a second, separately reviewed stage. E5/E12/E13/E16/
   E17/E20 change text layout and AccessKit semantics; `bidi_text.rs` must not
   pre-reorder text once E5 is active. This stage also adopts E21b: `IMEOutput`
   gains a bounded logical surrounding-text payload, and the Type tool supplies a
   real model adapter. E21b is new work, not existing E21/W27 capability.
9. Treat E11 as optional. PhotoCraft does not enable `epaint/multithreading`; the
   patch's forced single-threaded glyph context is a suite fix for Servo/VectorCraft,
   not a PhotoCraft requirement.
10. No PR may contain an absolute `path` patch. Final production manifests pin the
    reviewed, published `storytold` full 40-hex SHAs for the whole family and winit,
    including E21b and W15P. Stock 0.37.0 and the aiongg candidate revisions are
    research controls, never staged release pins.
11. Every performance comparison is paired on one machine. The committed
    `perf/baseline.json` is a macOS M4 Pro measurement and must not be used as a
    cross-machine verdict for this Linux/Windows work.
12. Both native and web run `App::logic` and `App::ui` inside `Context::run_ui`,
    once per layout pass. Logic-only ticks leave events and time stale, and a
    completed `run_ui` resets `current_pass_index` to zero. Fresh input is
    interpreted only in a known UI phase, with a forced hidden paint when the host
    is hidden. Background timers use monotonic clocks, not
    `ctx.input(|i| i.time)`, and every queue is consumed exactly once.
13. The before/after harness exists on the 0.36.2 baseline before any production
    edit. The baseline PR adds the text-tessellation, renderer-construction,
    selection-action and screenshot-lifecycle probes and records the paired
    baseline; it does not change production behavior.

## 2. Dependency model and publication

### 2.1 Current resolved family

`Cargo.toml:55-59` pins `eframe`, `egui`, `egui_kittest` at `0.36`;
`crates/ui-egui/Cargo.toml:33` pins `egui_extras` `0.36`;
`crates/ui-egui/Cargo.toml:73` pins `epaint_default_fonts` `0.36`;
winit is `=0.30.13` (`apps/photocraft/Cargo.toml:86`) and the lock resolves
egui-family `0.36.2`, winit `0.30.13`, wgpu `30.0.1`, accesskit `0.24.1`.
`craft-ui` is pinned to `5c874d51eeb3de70e735c8ce87a59a7dedd47675`,
`craft-print` to `3d37da389a3618ab9b76109da11416e9ca4ea719`, and
`craft-docpath` to `82a24b006eefa036b43c09a67fb9a3fbd17ffe63`
(`Cargo.toml:13`, `:42-43`).

### 2.2 Target family

Target the whole compatible set together:

| Package | Target | Source |
|---|---|---|
| `egui`, `eframe`, `egui-winit`, `egui-wgpu`, `egui_glow`, `egui_extras`, `egui_kittest`, `epaint`, `emath`, `ecolor`, `epaint_default_fonts` | 0.37.0 | one reviewed, published `storytold/egui` full 40-hex final P8 revision containing E1-E21, P2b/P7/P7b/E21b and P7c |
| `winit` | 0.30.13 | one reviewed, published `storytold/winit` full 40-hex revision containing W1-W27 **and W15P** |
| `wgpu` | 30.0.1 | registry (unchanged; `egui-wgpu` 0.37 is built for wgpu 30) |
| `accesskit` / `accesskit_winit` | 0.25.1 / 0.34.1 | registry, pulled by egui 0.37 |

The root workspace needs a `[patch.crates-io]` entry for **each** of those egui-family
packages and `winit`; patching only `egui` splits the graph. The same revision for all
egui crates is required because the fork's workspace crates depend on each other by
path. `epaint_default_fonts` is a build-dependency and must be patched too.

`egui_extras` must be `version = "0.37"`; do not assume crate version numbers match
the release name (`egui_plot`, if ever added, is independently versioned).

The candidate fork currently uses a **mutable branch**:
`aiongg/egui` commit `137398e6` sets `winit = { git = ".../aiongg/winit", branch =
"craft/0.30.13-lang" }`. The plan requires replacing that with an exact `rev`. A branch
pin is not reproducible and can change between two CI runs.

Pin identities are deliberately split:

- **production pins**: reviewed, published `storytold/egui` and `storytold/winit`
  full SHAs, reachable from the public URLs, carrying every selected E/W patch,
  E21b and W15P, with `PATCHES.md` and removal conditions;
- **research controls**: upstream `0.37.0` (`841a328b...`), upstream `0.36.2`
  (`49682f8b...`), aiongg `a4f9a17f...`/`8b35388b...` and any stock-only experiment
  used to attribute #8731/#8736. These are benchmark controls and are never a
  staged release, a consumer pin or a rollback baseline.

### 2.3 Targets and feature matrix

| Target | Package / binary | egui work |
|---|---|---|
| Desktop app | `photocraft` binary (`apps/photocraft`) | eframe 0.37 native, wgpu, accesskit, wayland, x11, persistence, clipboard; no `default_fonts`, no `system_fonts` in the first cut |
| Browser app | `photocraft-web` binary (`apps/photocraft-web`) | eframe web 0.37, wgpu (WebGPU + WebGL2), accesskit; browser glyph rasterizer; `web_file` |
| Browser bootstrap | `photocraft-web-boot` binary | no egui dependency; only wasm-bindgen/web-sys; keep out of the UI changes |
| Headless CLI | `photocraft-cli` | no egui dependency; control bridge/MCP only |
| UI library | `photocraft-ui-egui` | egui 0.37, eframe 0.37 (wgpu + accesskit + clipboard), `egui_extras` 0.37 (svg), `craft-ui` at the reviewed PR 199 port |
| Tests/benches | `crates/ui-egui/tests/it`, `examples/*`, `photocraft-testkit` | kittest 0.37 (wgpu, snapshot, eframe), `snapshot`, `perf_scenarios`, `interactive_bench`, `fx_bench`, `bevel_bench`, `camera_raw_bench`, `type_bench`, `large_image_bench`, `large_file_bench` |
| Platform helper | `photocraft-tablet` | no egui dependency; keep its winit-independent X11/AppKit/Windows path; E1 is only an enabler for a future native Wayland pen |

`apps/photocraft` is the only target with the direct Linux `winit` dependency
(`=0.30.13`, x11 feature, for `EventLoopBuilderExtX11`). It must resolve to the
same patched winit revision as eframe. `apps/photocraft-web` must not add a
native winit dependency.

### 2.4 Manifest and workspace audit

- The root workspace is `resolver = "3"`, edition 2024, MSRV 1.95;
  members are `crates/*`, `apps/*`, `xtask`; default members include the
  desktop app, CLI and xtask (`Cargo.toml:1-6`).
- `crates/*/fuzz/Cargo.toml` are independent fuzz workspaces. None depends on
  egui, eframe, winit, epaint or craft-ui; no change is expected. Verify with
  `rg` after the bump.
- `examples/plugins/invert-rs/Cargo.toml` is a separate `[workspace]` for a
  wasm plugin and has no UI dependency.
- `apps/photocraft-web-boot` and `apps/photocraft-cli` have no egui dependency.
- There is no `vendor/` directory, `[source]` replacement, or machine-local
  `.cargo/config.toml` patch in this checkout. The only `.cargo/config.toml`
  defines the `xtask` alias. Do not introduce a vendor tree for this migration.
- `[profile.wasm-release.package.egui|epaint|eframe]` entries remain valid
  because the package names do not change; re-check wasm size after the bump.

### 2.5 Publication and rollback

Publication gates, in order:

1. Canonical public `storytold/egui` `craft/0.37` revision, one commit per E-patch
   (including E21b), with `PATCHES.md`, tests and removal conditions.
2. Canonical public `storytold/winit` `craft/0.30.13` revision, one commit per W-patch
   (including W15P), with `PATCHES.md`, tests and removal conditions.
3. `storytold/craft-libs` revision on `main` containing only the reviewed port
   hunks (`8614e4a`/`1764bca`) applied to current `main`. Do not pin a PR ref or
   merge the stale `b68de8c`/`ec8840a` trees.
4. PhotoCraft PRs below, each with a `Cargo.lock` generated by `--locked` resolution
   from the public revisions.

Rollback is the previous coherent pair: old `Cargo.lock` plus the 0.36.2 registry
family and winit 0.30.13. Keep that exact lockfile under evidence before the first
bump. A revert of the dependency commit restores the source graph.

Persistence is not symmetric. The `PanelState` migration is additive: the old
`Id::unique(name)` key remains in `ui.ron` and 0.36.2 continues to read it. The
`WindowSettings` rewrite is **one-way**: 0.36.2 cannot deserialize the nested
`normal_geometry` value and opens with default geometry after a rollback. The first
0.37 run therefore keeps a pre-migration `ui.ron.pre-0.37` backup, and the plan
documents the window-geometry/position loss rather than calling both migrations
additive. Add a reverse-compatibility test with a 0.36-shaped settings struct and
fixtures for old macOS `inner_position_pixels`, old outer position, maximized and
fullscreen.

Reachability checks before publication:

- `git ls-remote` returns the exact revision from the public URL.
- `cargo tree -d` shows one egui, one eframe, one winit.
- `rg 'path\s*=\s*"/' Cargo.toml` is empty.
- Every `[patch]` URL is `https://github.com/storytold/...` with `rev = "<40 hex>"`.

## 3. What the source actually does

### 3.1 Native entry point

`apps/photocraft/src/main.rs:720` calls `eframe::run_native`. The app creator
installs fonts/theme through `PhotocraftApp::setup_context`
(`crates/ui-egui/src/lib.rs:2850`), attaches the GPU canvas, control server,
native menu, single-instance service and platform services, then returns
`PhotocraftApp`.

`native_options` (`apps/photocraft/src/main.rs:104`) sets a custom viewport,
`persistence_path` `ui.ron`, and a `window_builder` hook `restored_window`
(`:132`) that enforces minimum size and disables restored fullscreen.
`options.centered` is decided from `ui_state::has_saved_position`
(`apps/photocraft/src/main.rs:667`, `apps/photocraft/src/ui_state.rs:33`).

`gpu_startup` configures `options.wgpu_options`; `presentation` applies present
mode and frame-latency overrides. `eframe::Error::Wgpu(_)` is the signal for the
CPU-retry path (`apps/photocraft/src/main.rs:934`,
`apps/photocraft/src/gpu_startup.rs:477`). E2/E3 add `WgpuError::ConfigureSurface`
and `CreateRenderer`; the existing `Error::Wgpu(_)` match keeps working, but the
retry decision must be tested with the new variants.

### 3.2 Web entry point

`apps/photocraft-web/src/web.rs:47` starts `eframe::WebRunner`. `WebShell`
implements `raw_input_hook`, `logic` and `ui` (`:325`, `:332`, `:362`); its
`logic` drains dropped files, serves fonts, calls the app's `logic`, and saves
presets. Its `ui` adds a browser preset-storage banner and calls the app's `ui`.

`apps/photocraft-web/index.html` is Trunk-based and the wasm is gzipped and split
into parts by `packaging/web/package.sh`. Its 24 MiB limit applies to **each shipped
file**, not to the raw wasm; the boot module inflates the parts. 0.37 adds
`send_wrapper` and changes `epaint`/`vello_cpu`; the raw wasm and the gzipped total
must both be measured against baseline, not assumed to fit a raw-wasm cap.
`docs/development.md:459` is the reference.

### 3.3 Logic versus UI, and the pass model

The pass model is the one verified in
`/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/lifecycle-source-audit.md`
against 0.37.0:

- Native (`crates/eframe/src/native/epi_integration.rs::update`) and web
  (`crates/eframe/src/web/app_runner.rs`) both call `App::logic` and `App::ui`
  **inside `Context::run_ui`**. Each can run once per layout pass, and
  `request_discard` adds passes. Logic is not once per rendered frame.
- A hidden/occluded window may be ticked through `update_logic_only` ->
  `Context::run_logic`. That path still calls `prepare_raw_input` and
  `App::raw_input_hook`, but does **not** begin a pass: `ctx.input().events`,
  `time`, `screen_rect` and viewport geometry remain from the previous UI pass.
  `ViewportCommand::RequestPaintWhileHidden` can force a real UI pass while
  `visible()` is false, so visibility does not identify a logic-only callback.
- After a normally completed `run_ui`, `current_pass_index()` is zero because
  `end_pass` takes the output. Do not build a first-pass guard on a stale nonzero
  index. A guard may protect genuinely once-per-layout-frame work only with a
  regression proving normal hidden calls still run; it must not suppress fresh
  async results, control commands or widget actions.

Consequences for PhotoCraft:

`PhotocraftApp::logic` (`crates/ui-egui/src/lib.rs:2244`) does stateful work that
must survive a hidden window: frame accounting, `sync_views`, GPU checks, control
drain, native menu, commit/end operations, screenshot issue/collect, preferences
tick, monitor polling, i18n sync, work-area fit, close guard, job tick, shortcuts,
file drops, file dialogs. Because it runs per pass, every item must consume its
queue once, use a revision/generation or monotonic deadline, and tolerate a
second pass with no new input. `self.frame += 1` (`:2257`) is a logic-pass count,
not a rendered-frame identity; screenshot settling and GPU startup checks must use
`ctx.cumulative_frame_nr()` (or an equivalent UI-phase rendered-frame counter).

`PhotocraftApp::ui` (`:2371`) draws panels, windows, dialogs, notices and the
central canvas, and does UI-local persistence (`brush_picker::persist`,
`last_brush::persist`, `tool_memory::persist`, `view_cmds::persist`) plus
`native_menu::sync`, `gpu_status::check_window`, and the current synthetic-input
wakeup. Re-running a sizing pass must not duplicate persistence, commands or
notices; move work that must happen with no paint to `logic`, and make the rest
idempotent.

`raw_input_hook` (`:2338`) normalizes synthetic input, clipboard key events,
legacy pinch, pen-hold, titlebar resize and stroke tracing. It runs for the root
and deferred viewports, not immediate viewports (unless E1 is installed), and it
also runs before a hidden `run_logic`. `take_synthetic_step` (`:2356`) removes
events from the app queue even on a logic-only tick; eframe preserves the events it
appends in `pending_raw_input`, but PhotoCraft must set a `synthetic_in_flight`
flag in the hook and send `RequestPaintWhileHidden` from `logic` (which runs
after the hook in the same tick) so a UI pass consumes them. Do not start stroke
tracing or interpret fresh input as if a paint occurred: either gate it on that
flag or move the start to `App::ui`. `egui-winit` turns
Cmd/Ctrl+C/X/V into `Event::Copy`/`Cut`/`Paste`; PhotoCraft maps those back to key
presses in `shortcuts::clipboard_keys` (`crates/ui-egui/src/shortcuts.rs:282`).

### 3.4 Panels, docking and the root Ui

The root Ui is passed to `App::ui`; panels are shown on it:
`panels::title_bar` (`crates/ui-egui/src/panels.rs:592`),
`options_bar` (`:969`), `status_bar` (`:1726`), `toolbar` (`:93`),
`right_dock` (`:1792`), and `dock_panels` (`:1866`). The custom docking surface
uses `craft_ui::docking::DockArea` with the stable ID
`photocraft-panel-docking` (`crates/ui-egui/src/panel_docking.rs:424`) and an
`egui::Panel::right("shared-panel-dock")` (`:448`).

`craft_ui` is a real runtime dependency (`panel_docking.rs:4-5`,
`tab_strip.rs:11`), not a stale manifest entry. `craft_ui`'s docking and tab
components are in the 0.37 API blast radius; PR 199 is required.

### 3.5 Fonts and text

`theme::install_fonts` (`crates/ui-egui/src/theme.rs:847`) builds a
`FontDefinitions` from `egui_fonts::definitions`, prepends Inter/JetBrains Mono,
adds named `medium`/`semibold` families, scales faces with `UiFontSizePlugin`
(`:946`), calls `ctx.set_fonts`, then installs the lazy CJK/script fallback
(`crates/ui-egui/src/cjk_fonts.rs:352`, `:375`). The fallback is app-specific:
script order follows the UI locale, reads at most one system font per frame, and
tracks its own `tried`/`loaded` state. It reads system fonts even when eframe's
`system_fonts` feature is off, so #8512's discovery cost is still on PhotoCraft's
startup/first-use path and must be measured separately.

`cjk_fonts::add_to_all_families` reads `FontData.font` as bytes
(`cjk_fonts.rs:354`, `:355`, `:358`) and a test compares the pointer
(`:879`). In 0.37 `FontData.font` becomes
`Arc<dyn AsRef<[u8]> + Send + Sync>` (`epaint::text::FontData`), so those four
uses do not type-check as written.

### 3.6 Screenshots and control

`control.rs:1265` handles `ui.screenshot`; `PhotocraftApp::drain_control`
(`lib.rs:2156`) queues `(token, earliest_time, frames)` in
`queued_screenshots`; `issue_screenshots` (`:2183`) sends
`ViewportCommand::Screenshot(UserData::new(token))`; `collect_screenshots`
(`:2196`) reads `Event::Screenshot { user_data, image }` and calls
`control::save_screenshot` (`control.rs:1410`). This is exactly the API 0.37
removes: `ViewportCommand::Screenshot` now takes a `ScreenshotCallback`, and
`Event::Screenshot` / `UserData` no longer exist. The current records are removed
only by a matching callback, so a zero-sized surface, hidden viewport, device
loss or dropped capture leaves the reply pending until the 60 s deadline and
retains its image path. Section 9.3 replaces that with a bounded, cancelable
slot and an explicit timeout drain.

### 3.7 Persistence

`ui.ron` contains two independently versioned payloads:

- eframe window geometry, restored by `egui_winit::WindowSettings`;
- egui memory, including `PanelState` entries and persisted widget data.

`apps/photocraft/src/ui_state.rs` sanitizes the file before eframe reads it and
decides whether a saved position exists. `saved_position` is platform-dependent:
macOS looks for `inner_position_pixels`, everywhere else for
`outer_position_pixels` (`ui_state.rs:41-44`). 0.37 replaces the flat
`WindowSettings` with `normal_geometry: WindowGeometry` containing
`outer_position_pixels`, `scale_factor` and `inner_size_points`. The old value
does not deserialize, eframe ignores it, and a top-level probe becomes a false
positive: the window opens uncentered at the OS default. Section 9.2 specifies the
explicit pre-load rewrite and the new platform-independent position key.

`PanelState` is keyed by an `Id`. In 0.36 `Panel::left("dock")` used
`Id::new("dock")`; in 0.37 the panel ID is
`parent_ui.scope_id().with_salt(IdSalt::new("dock"))` (`Id::with`, not
`Id::with_salt` + an `AsIdSalt`). For the root Ui that is
`ctx.viewport_id().root_ui_id().with(salt)`. Existing saved panel widths are under
the old IDs and will not be restored unless migrated. Tests that read
`PanelState::load(&ctx, Id::unique("dock"))` also break
(`apps/photocraft/src/main.rs:1004`, `crates/ui-egui/src/panels.rs:6604`,
`:6934`, `crates/ui-egui/src/workspace_ui.rs:594`,
`crates/ui-egui/src/panel_docking_tests.rs:569`). `Id::unique(name)` has the same
hash as the old `Id::new(name)`, so the old key can still be read.

### 3.8 Tests and performance infrastructure

The repository already has the right harnesses:

- `crates/ui-egui/examples/snapshot.rs` renders the full UI offscreen with
  `egui_kittest` + wgpu, deterministic `RendererOptions`, scripts, themes and
  scales.
- `crates/ui-egui/src/panel_docking_tests.rs:179` captures theme/width/scale
  docking fixtures when `CRAFT_UI_DOCKING_FIXTURES` is set.
- `crates/ui-egui/src/panels.rs:6904` captures compact chrome fixtures under
  `CRAFT_UI_VISUAL_FIXTURES`.
- `perf/budgets.toml` maps P1…Pn rows to `perf_scenarios`, `interactive_bench`,
  `fx_bench`, `bevel_bench`, `camera_raw_bench`, `type_bench`,
  `large_image_bench`, `large_file_bench`, plus the new `text_tessellation`
  row; `xtask perf` runs them in release and writes p50/p95 (and p99 for frame
  rows with enough samples), peak RSS, GPU bytes and load averages.
- `crates/ui-egui/tests/it` has wgpu kittest tests for canvas, GPU loss,
  color management, exports and workspace presets.
- `apps/photocraft/tests/mac_menu_appkit.rs` is the existing real-macOS menu
  check; it is not a substitute for IME/focus testing.

There are 481 `Harness::builder()` uses (source count at
`ff473881`). They will inherit 0.37's `check_accessibility: true` and
`missing_glyph_policy: Panic` (`egui_kittest/src/builder.rs:50-51`). Both defaults
are good, but they turn latent UI defects into test failures. The minimal
accessible-name, glyph-policy and feature fixes therefore belong in the first
green PR; do not disable either check globally and do not defer them to a later
fixture sweep.

### 3.9 Multiple viewports

PhotoCraft creates extra document windows with
`ctx.show_viewport_immediate(ViewportId::from_hash_of(("docwin", w.id)), ...)`
(`crates/ui-egui/src/canvas.rs:6551`). These are immediate viewports, not deferred
ones. The implications are:

- `App::raw_input_hook` does not run for immediate viewports; E1's
  `ViewportHooks::raw_input` is the only per-viewport hook that does. PhotoCraft
  has no consumer today, but the extra-window path must be tested with E1 present
  and absent.
- The root-only services (`tool_cursor`, `monitor_status`, `caps_lock`, screenshot
  capture, control server) correctly guard on `ViewportId::ROOT`
  (`tool_cursor.rs:103`, `monitor_status.rs:274`); do not accidentally make them
  viewport-global.
- #8748 (hidden-parent deferred children) is **closed and unmerged**, so it is not
  behavior in 0.37.0. PhotoCraft has no `show_viewport_deferred` call sites
  (`canvas.rs:6551`, `screen_picker.rs:296`, `camera_raw_ui.rs:1224` are all
  `show_viewport_immediate`), so the upstream deferred-child bug is not directly
  applicable. Still test the hidden/occluded root plus an immediate child's
  open/update/close, because that is the path PhotoCraft actually uses.
- `ui.screenshot` is documented as main-window only (`docs/control-protocol.md`);
  do not promise child-window screenshots in 0.37.
- Extra windows use the same `PhotocraftApp` state; a pass for a child viewport
  may call `canvas_view` while the root UI is not being drawn. Keep document
  mutation and undo transactions inside `logic` or the existing single-window
  paths; test open/close of an extra window during a stroke, a dialog and a
  screenshot request.

### 3.10 Platform cfg inventory

The migration can compile on one platform and still break another; these cfg
boundaries need an explicit check:

- Native-only: `sys-locale`, `rayon`, `winsafe`, `xcap`, `x11rb`, `wl-clipboard-rs`,
  `zbus`, `muda`, `dispatch2`, `fmv-macos-events`, `core-graphics2` in
  `apps/photocraft` and `crates/ui-egui`.
- macOS: `apple_events`, `mac_menu`, `mac_window`, `muda`, `dispatch2`,
  `objc2*`, `photocraft-tablet::macos`, `tests/mac_menu_appkit.rs`.
- Windows: `window_frame`, `cursor`, `photocraft-tablet::windows`,
  `photocraft-ipp`, `winsafe`.
- Linux: `portal_save`, `print_portal`, `icon_theme`, `tablet::spawn_x11`,
  `linux_libs`, `winit` x11 event-loop builder.
- wasm32: `apps/photocraft-web/src/web.rs`, `cjk_fonts::Sources::system` (no
  system fonts), `file_ui/web_preview.rs`, `gpu_canvas` web adapter limits,
  `photocraft-web-boot`.

At minimum run `cargo check --target wasm32-unknown-unknown` (`cargo xtask
wasm`), Linux native, Windows native and macOS native for the changed crates.
Do not treat a Linux-only compile as platform acceptance.

## 4. 0.37 breaking changes mapped to PhotoCraft

| Change | PhotoCraft impact and exact work | Count / anchor |
|---|---|---|
| `WidgetType` is now a deprecated alias of `accesskit::Role`; many variants were renamed (`SelectableLabel→Button`, `Checkbox→CheckBox`, `ColorButton→ColorWell`, `CollapsingHeader→DisclosureTriangle`, `Other→Unknown`) | Mechanical mapping. `Button→Button`, `SelectableLabel→Button`, `CheckBox→CheckBox`, `RadioButton→RadioButton`, `Label→Label`, `Image→Image`, `CollapsingHeader→DisclosureTriangle`, `ComboBox→ComboBox`, `ColorButton→ColorWell`, `Slider→Slider`, `Other→Unknown`. Keep selection in `WidgetInfo::selected`. | 114 uses in 43 files, e.g. `shape_stroke_ui/editor.rs:47`, `props_layout.rs:73`; PR 199's mapping is the reference |
| `Id::new` deprecated | `Id::new` → `Id::unique` (same hash; persisted IDs stay stable). | 394 uses in 121 files |
| `Ui::id` deprecated | `ui.id()` → `ui.scope_id()` (a rename of the old id). Do **not** substitute `Ui::unique_id`: that is a new globally unique but unstable id used for auto ids and AccessKit identity. | 93 uses |
| `Panel` constructors take `AsIdSalt`; IDs become parent-scoped | `direction.rs:71/76` currently takes `impl Into<egui::Id>` and passes it to `Panel::left`; change to `impl egui::AsIdSalt`. Add `panels::panel_state_id(ctx, salt) = ctx.viewport_id().root_ui_id().with(salt)` (`Id::with`, because `with_salt` takes an `IdSalt`). Migrate old `PanelState` keys in the first compatible PR and keep the old key readable. | 11 direct panel constructors; 5 `PanelState::load` sites |
| `UiBuilder::id` → `scope_id` | No PhotoCraft call sites; `craft_ui` PR 199 has two. | `craft_ui/docking/surface.rs` |
| `TextEdit::hint_text` requires `IntoAtoms<'static>` | `widgets::text_field`/`text_field_ex` take `hint: &str` and pass it directly; `crop_size::text_field` does too. Convert to owned atoms once (`hint.to_owned()`) or change the helper parameter to `impl IntoAtoms<'static>` and update callers. | `widgets.rs:848`, `crop_size.rs:282` |
| `FontData.font` is a `Blob` (`Arc<dyn AsRef<[u8]>>`) | Use `data.bytes()` or `data.font.as_ref().as_ref()`; update pointer test. | `cjk_fonts.rs:354`, `:355`, `:358`, `:879` |
| `ViewportCommand::Screenshot` callback; `Event::Screenshot`/`UserData` removed | Replace `issue_screenshots` with a bounded, cancelable per-request completion slot (section 9.3), not an unbounded `mpsc` channel. The callback holds only a weak slot reference, stores only the image, and may run on an arbitrary callback thread. Collect in `logic`; use rendered-frame identity plus a monotonic settle deadline; keep token correlation. `clipping_indicator_tests.rs:94/:99` also use the removed API and must be migrated in the same PR. | `lib.rs:2183`, `:2196`, `:2214`; `control.rs:1265`; `clipping_indicator_tests.rs:94`, `:99` |
| `Event::PasteImage` new | egui-winit now emits image paste when the clipboard has no text. PhotoCraft's `clipboard_press` only maps `Event::Paste(_)`. Adopt it as the document paste shortcut through the existing `import_os_clipboard`/session path, never a second clipboard read; a test asserts one import and no text-paste double handling. | `shortcuts.rs:221`, `:282` |
| `Event::MiddleClickPaste` new | Text fields handle it; verify PhotoCraft's canvas/field interaction does not double-handle it. | `shortcuts::clipboard_keys` |
| `ScrollAreaState.offset` field private; `offset()` deprecated | Replace reads with `clamped_offset()`/`unclamped_offset()`. Replace the direct write `st.offset.y = target` with a request passed to `ScrollArea::vertical_scroll_offset` on the next frame, then clear the request. | `menu_nav.rs:504`, `:535`; `field_wheel.rs:173-174` |
| `egui-winit` clipboard feature is no longer implicit in eframe when default features are off | Add `clipboard` to the `eframe` feature list in the app and ui-egui. Without it, OS copy/paste silently falls back to an in-app-only clipboard. | `apps/photocraft/Cargo.toml:32`, `crates/ui-egui/Cargo.toml:34` |
| `system_font_fallback` defaults true | Workspace `eframe` has default features off, so the provider is only the "missing system fonts" warning. Set `NativeOptions::system_font_fallback = false` explicitly to preserve the existing CJK plugin and deterministic rendering; evaluate `system_fonts` separately. | `main.rs:104` |
| `default_fonts` is no longer default; emoji fonts are feature-gated; `egui-icons` is new | Add `monochrome_emoji_fonts` to the `epaint_default_fonts` build-dependency; deflate `EGUI_ICONS`; rebuild the family stack exactly as 0.37 does (Ubuntu-Light, egui-icons, NotoEmoji, emoji-icon-font; Hack first for monospace). Update the equality test. No new font file is committed; verify `ATTRIBUTION.md` needs no row, and add one in the same PR if a packaged asset is added. | `build.rs:37-45`, `egui_fonts.rs:47-56`, `:79` |
| `Ui::id`/`Id` semantics in tests | kittest queries by IDs still work after `Id::unique`; panel-state queries do not. `Ui::unique_id` is unstable and must not become a persisted/test key. | see persistence above |
| `AccessKit` role/label checks in kittest | Add `Response::labelled_by`/`accessible_name` where a field has no hint; keep `check_accessibility` on. These minimal name fixes are in the first green PR because the default check runs on every harness. | all 481 harnesses |
| `MissingGlyphPolicy::Panic` in kittest | Audit special characters (`✓`, `▲▼`, `▶◀`, `×`, `…`, `⏵`, `⋮`, `egui::special_emojis`) and add a deterministic fallback or `allow_missing_glyphs()` only where tofu is intentional. Enable `monochrome_emoji_fonts`/`EGUI_ICONS` and add the glyph-coverage test in the first green PR. | 481 harnesses |
| `#8763` TextEdit click-away selection loss | Reproduce select-text-then-click-toolbar/context-action. The fork owns the selected fix (with pointer and keyboard regressions) or a tested app workaround preserving range and undo; do not silently accept. | `widgets.rs`, `text_field_menu.rs`, new kittest/native row |
| `egui` 0.37 style defaults (roomier buttons, larger radii) | PhotoCraft overrides most `Visuals`/`Spacing` fields in `theme::apply`, but atom-based `Button`/`ComboBox` and popup/table internals still change. Pixel fixtures must be regenerated and reviewed, not blindly accepted. | `theme.rs:995` |
| `ScrollArea` kinetic/rubber-band defaults | New `ScrollStyle::kinetic`; PhotoCraft's thin scrollbars keep their explicit opacity, but menu wheel/arrow scrolling and canvas pan must be re-tested. | `theme.rs:1065`, `menu_nav.rs` |

## 5. E1–E21 and E21b audit

Base: `841a328b`. The candidate branch is `a4f9a17f`, with one commit per topic
and `PATCHES.md` at that revision. All rows need a final audit against the
published commits; the notes below state what PhotoCraft specifically needs.

| # | Change | PhotoCraft relevance and decision | Required evidence / removal |
|---|---|---|---|
| E1 | eframe `NativeOptions::viewport_hooks` (per-viewport window create/destroy and raw input, immediate viewports included) | PhotoCraft does not call it today. Keep in the shared fork because craft-tablet's native Wayland pen path needs it; do not claim pen support until craft-tablet consumes it. Verify the global hook storage does not leak between contexts/tests. | Fork unit test for install/replace; PhotoCraft full-suite run; remove when upstream offers equivalent per-viewport hooks or native Wayland pen is solved another way |
| E2+E3 | egui-wgpu surface fit to `max_texture_dimension_2d`; configure/renderer errors returned as `WgpuError::ConfigureSurface` / `CreateRenderer` | Directly relevant to multi-monitor/large restored windows and `--safe-gpu` fallback. Adopt. Add a test with a synthetic surface size above the adapter limit and a forced configure error; assert no panic and a recoverable error. | `surface_fit` unit tests; PhotoCraft large-window test; remove when upstream closes emilk/egui#8361 and ships an equivalent |
| E4 | egui-winit uses winit's Wayland clipboard when available; smithay not started; PRIMARY currently falls back to arboard | Regular clipboard/file drops are relevant on Hyprland and require W15, `wayland` and `clipboard`. Adopt the regular path. **PRIMARY is not accepted through arboard**: the default is the shared W15P winit-owned PRIMARY path on the existing connection/event queue and per-seat lifecycle; arboard is only a measured fallback if W15P cannot meet a proven requirement, with compositor evidence and an explicit unresolved gate. | Regular/image/drop tests; W15P PRIMARY matrix on wlroots/KWin/Mutter; remove when upstream #8657 lands |
| E5 | epaint bidi shaping, visual row order, marks, end caret/selection; word-motion underflow fix; tests without `default_fonts` | Core for Arabic/Hebrew UI labels. Adopt only with E12/E13/E16 and with `bidi_text.rs` removed or reduced; otherwise text is reversed twice. | Arabic/Hebrew label, mixed LTR/RTL, caret/selection tests; remove when #8577 is released |
| E6 | `request_focus` on the focused widget no longer interrupts IME | Relevant to every text field and Preferences search. Adopt. | IME preedit/commit across focus requests |
| E7 | Windows letter keys from virtual-key code on non-Latin layouts | Relevant; PhotoCraft has layout-specific symbol logic. Adopt. | Windows layout matrix (Russian/Arabic/Hebrew/German) |
| E8 | TextEdit no longer clips glyphs above/below the row | Relevant to Thai/Vietnamese/Arabic marks and small fields. Adopt; pixel tests. | Text field row geometry at 1x/1.5x/2x |
| E9 | `Style::right_to_left` mirrors menus, scrollbars, headers, title bars, combo boxes, checkboxes, popups, resize grips | PhotoCraft already mirrors shell chrome explicitly in `direction.rs`. Enabling E9 while keeping those explicit layouts will double-mirror. Adopt only after a call-site audit; default `right_to_left=false` means it is inert until set. | RTL chrome fixture matrix; remove overlapping `direction` paths |
| E10 | eframe web: named keys not typed as text; letter keys from `KeyboardEvent.code`; `lang` on hidden input/speech | Relevant to browser shortcuts and IME. Adopt. | Browser keyboard/IME workflow; no wasm size regression |
| E11 | epaint glyphs rasterized on a single-threaded vello context (fixes `vello_cpu` multithreading panic) | PhotoCraft does not enable `epaint/multithreading`; forcing `num_threads: 0` could slow other consumers. Keep out of PhotoCraft's qualifying revision unless the shared fork requires it; if included, benchmark glyph rasterization and test all fonts. | epaint tests with/without multithreading; perf rows |
| E12 | `LayoutJob::direction`, RTL right alignment, ellipsis at visual end, UAX #9 L1, linear reorder keeping isolate initiators/PDI/marks | Needed with E5; replaces `bidi_text::visual_ltr` and `elide`. Adopt in the language stage. | Title bar `*صورة.psd — PhotoCraft`, layer-name truncation, bidi controls |
| E13 | caret/hit-test/Delete/Backspace by grapheme cluster | TextEdit and Type tool. Adopt; the Type tool has its own char-based editor (`type_tool.rs:562`, `:751`, `:964`), which must be audited for combining marks/emoji. | Grapheme deletion/movement tests in both TextEdit and Type tool |
| E14 | `FontTweak::fallback_align`: fallback fonts on primary baseline, rows grow to fit; default unchanged | PhotoCraft already computes `y_offset_factor` manually. Adopt, but do not set `fallback_align` unless the manual metrics are removed; test row height and CJK/Thai/Arabic baselines. | Font fallback baseline fixtures; `cjk_fonts` tests |
| E15 | `LineBreaker` trait (`SimpleLineBreaker` preserves rules), grapheme-safe emergency breaks, `LayoutJob::language` | Default preserves behavior; the suite uses it for craft-text line breaking, and E20 uses the language. PhotoCraft document text is parley-based, so the app impact is UI text wrapping/truncation. Adopt; test wrapping/truncation. | Wrapping fixtures with Latin/CJK/Thai |
| E16 | visual cursor movement with affinity, Home/End in reading order, password mask per cluster, neutral paragraphs inherit direction | Needed for RTL and grapheme editing. Adopt with E5/E12/E13. | Caret affinity tests, mixed-direction paragraphs |
| E17 | `Context::set_language`, `UiBuilder::language`, `Response::accessible_language`, AccessKit root language, one text run per bidi level | PhotoCraft's `i18n::sync_context` (`i18n/mod.rs:409`) must call `set_language`; add AccessKit language assertions. Adopt. | AccessKit root/label language; screen-reader smoke test |
| E18 | eframe `Frame::set_ui_language` sets `Context::set_language`; web falls back to `Context::language` | Call from `PhotocraftApp::logic` when the preference changes. Adopt. | Live language switch without losing UI state |
| E19 | one letter-key rule across Windows/macOS/Linux; `RawInput::keyboard_layout_is_latin` and `latin_layout_chars` | Relevant to non-Latin shortcuts; requires winit W25. Adopt, but keep `layout_keys.rs`'s Photoshop-specific German bracket table and test that the two rules do not fight. | Layout matrix; `layout_keys` tests |
| E20 | widget text laid out in the language of its `Ui` | Adopt with E15/E17; PhotoCraft's `tl!` remains the source text. | Translated widget labels in AccessKit and layout |
| E21 | `ImeEvent::DeleteSurroundingBytes`: TextEdit deletes exact bytes an IME replaces, in the commit's undo step | Relevant to macOS press-and-hold/dictation and Wayland IME. Requires winit W26/W27. The candidate emits this on macOS and Wayland only; Windows `ime_delete_surrounding` returns `None`. Restrict E21 acceptance to macOS/Wayland. The enum variant is a compile break for exhaustive matches; `type_tool.rs` has a catch-all but must implement the deletion for the Type tool. Adopt. | macOS/Wayland IME replacement tests; undo one step. A Windows IME row is general coverage, not E21. |
| E21b | `IMEOutput` gains a bounded logical surrounding-text payload (committed text, UTF-8 byte cursor/anchor, owning widget identity; preedit excluded) and egui-winit sends it through W27 on Wayland | **New prerequisite, not existing E21/W27 capability.** The candidate never calls `set_ime_surrounding_text`; `IMEOutput` has no logical text. Adopt with E21. PhotoCraft's custom Type tool must supply a real model adapter, not `None`: `type_tool::draw_overlay` constructs the `IMEOutput`; use the layer's logical text (`current_text`/`layout`), char-to-byte via `byte_of`, exclude the `preedit` char range, clamp to the 4000-byte payload at char boundaries, remap cursor/anchor bytes, and identify the document+layer+viewport so two windows cannot share a payload. Password fields never supply text; focus loss, no IME and unavailable context clear it. | PhotoCraft adapter plus tests: delete+commit and deletion-only, preedit exclusion, reversed selection, emoji/combining/RTL logical order, caret movement, >4000-byte line, two documents, focus/reset/late serial, multipass final output, one undo; native Wayland predictive/Vietnamese trace. Revert with the custom-editor adapters as one API set. |

The E-patches are **not** a blanket "merge the branch". In particular, E5/E12/E13/E16
form a dependency cluster; E14/E15/E20 depend on text-layout changes; E9 must not
be enabled with the current explicit RTL mirroring; E11 has no PhotoCraft consumer.
E21b depends on E21/W27 and is a separate reviewed change (egui P7b), not a
retroactive description of E21.

## 6. Winit patch audit and compatibility

Base: upstream `v0.30.13` `e9809ef5`; candidate
`8b35388b` (storytold `craft/0.30.13` `ffae5198` + W17–W27). W15P is **new
work** in the reviewed published revision, not part of the candidate; the final
production pin must include it.

| # | Change | PhotoCraft decision |
|---|---|---|
| W1 | Windows 11 `WM_DPICHANGED` suggested rect | Adopt; custom title bar and mixed-DPI drags |
| W2 | Wayland destroy seat text input only when seat is removed | Adopt |
| W3 | Windows IME caret from `GCS_CURSORPOS` only when set | Adopt |
| W4 | Windows hidden maximized windows | Adopt |
| W5 | X11 focus replacement IME context | Adopt as part of W20; audit whether W20 supersedes and remove W5's refocus path |
| W6 | macOS `NSTextInputClient` commit flow | Adopt |
| W7 | macOS discard marked text when IME disabled | Adopt |
| W8 | macOS emoji picker text as `Ime::Commit` | Adopt |
| W9 | macOS Korean space not duplicated | Adopt |
| W10 | macOS first Apple Korean key | Adopt |
| W11 | X11 server DPI before panel physical DPI | Adopt |
| W12 | macOS `mouseDownCanMoveWindow` from `movable_by_window_background` | PhotoCraft calls `NSWindow::setMovable(false)` (`apps/photocraft/src/mac_window.rs`) and drags only its own gap; keep suite-wide but not PhotoCraft-required. Verify no regression. |
| W13 | Wayland pixel/continuous scroll phases report `Moved` | Adopt; zoom/pan latch tests |
| W14 | macOS opt-in cancelable termination | PhotoCraft quits via `NSApplication terminate:` (`mac_window.rs`); not required, harmless if opt-in. |
| W15 | Wayland file drops + winit clipboard data device | Adopt; needed by E4 regular clipboard and the app's drop support |
| W15P | winit-owned PRIMARY selection on the existing connection/event queue, with per-seat `PrimarySelectionManagerState`/device/source/offer lifecycle | **Required shared path.** Distinct PRIMARY protocol object; do not bind a second regular `wl_data_device`. Adopt before E4's PRIMARY acceptance. Keyboard-only focus, serial freshness, owner/seat changes, empty/absent offers, self-paste, canceled/slow transfer, password exclusion and unchanged-selection reclaim policy are native gates on wlroots/KWin/Mutter. |
| W16 | Wayland pinch/rotation gestures | Defer; PhotoCraft has `wheel_nav::fold_legacy_pinch`, which must be re-tested |
| W17 | Regression tests for W2/W3/W5–W10/W12/W13 | Adopt with the branch |
| W18 | Windows hidden system caret for IME | Adopt |
| W19 | Windows IME `VK_PROCESSKEY` not sent as key press | Adopt |
| W20 | X11 one input context per window (replaces W5 refocus) | Adopt; resolves W5 duplication |
| W21 | X11 IME spot after 100 ms quiet | Adopt |
| W22 | macOS `ImeDocument`, replacement ranges, typed text without composition | Adopt |
| W23 | macOS ⌘ key names from layout's ⌘ table | Adopt |
| W24 | `Window::reset_ime()` | Adopt; expose through eframe if a PhotoCraft call site needs it (currently none) |
| W25 | X11/Wayland `KeyEventExtLatinLayout` | Adopt; required by E19 |
| W26 | macOS delete replaced text by UTF-8 byte count | Adopt; required by E21 |
| W27 | Wayland text-input-v3 surrounding text + delete-surrounding | Adopt; required by E21 and E21b. The winit state/serial/payload tests are part of the E21b acceptance, not optional coverage. |

Compatibility notes:

- Winit 0.30.13 is the last 0.30 release; the 0.31 port is not a migration target.
  The fork must keep the package version `0.30.13` so `=0.30.13` and
  `egui-winit`'s dependency unify.
- W14 takes `dpi` from crates.io so git consumers do not get a second `dpi`.
- The fork's `rwh_06` and `x11`/`wayland` features must remain compatible with
  `accesskit_winit` and eframe.
- The app's direct Linux `winit` dependency exists only for
  `EventLoopBuilderExtX11` (`apps/photocraft/src/main.rs:658`); it must resolve to
  the same patched revision as eframe's winit.
- Do not add `wayland-csd-adwaita` here; it is a suite feature decision, not a
  PhotoCraft migration requirement.

## 7. craft-libs PR 199

PR 199 is an open API-port, not a mergeable tree. Its head
`b68de8c5f4f1c060904d211a7b1320aa0f7bce78` and the historical audit tree
`ec8840a` are stale relative to current `main` `49190884278f16c9a9f5823d7698a8ffb5deb062`:
they omit `craft-platform` and revert security, settings, docking and printing
fixes. Take only the reusable port hunks from
`8614e4a04fae516fba16c22b31ff1008cb6be10b` plus the relevant README changes from
`1764bca` and apply them on current `main`; never merge `b68de8c`/`ec8840a` as
trees and never revert the intervening fixes.

The relevant `craft-ui` port changes are:

- `crates/ui/src/buttons/mod.rs`: `WidgetType::Button` → `Role::Button`.
- `crates/ui/src/docking/surface.rs`: `WidgetType` → `Role`, `UiBuilder::id` →
  `scope_id`, plus a clippy `collapsible_match` fix.
- `crates/ui/src/layout/resize.rs`, `tabs/mod.rs` and tests: `Id::new` →
  `Id::unique`, `Ui::id` → `scope_id`, `WidgetType` → `Role`.
- `crates/ui/Cargo.toml`: `egui_extras`/`egui_kittest` 0.37.

PhotoCraft uses `craft_ui::docking::{DockArea, Layout, Node}`,
`craft_ui::tabs::{Tab, fit}`, and `craft_ui::layout` in
`panel_docking.rs`, `tab_strip.rs`, `dock.rs` and `state.rs`. The port must be
present before PhotoCraft can compile. There is no additional shared UI helper
needed for the mechanical Role/Id migration: PhotoCraft's `WidgetInfo` calls are
local and the mappings are direct. Do not invent a compatibility shim in
`craft-ui`.

Pin policy: update **only** `craft-ui` to the published revision containing those
port hunks. `craft-print` (`3d37da389a3618ab9b76109da11416e9ca4ea719`) and
`craft-docpath` (`82a24b006eefa036b43c09a67fb9a3fbd17ffe63`) are not egui
consumers and keep their current pins; bumping all three to one `main` would pull
the unrelated printer-name/description change between `3d37da3` and the port. If
the suite later requires one craft-libs revision, audit that diff and run
PhotoCraft's print tests before changing these pins. Unaffected local vendors
(`krilla`, `hayro*`, `lopdf`, `vello_common`) and unrelated source fixes are
preserved.

## 8. Lifecycle and root-Ui design

### 8.1 Logic and hidden iterations

The verified model is section 3.3, not "logic when hidden, ui when visible".
Native and web both run logic and ui inside `run_ui`, once per pass; hidden hosts
may call `update_logic_only`, which invokes `raw_input_hook` and `App::logic`
without beginning a pass. `run_logic` leaves events, input time and layout stale.
A forced hidden paint (`RequestPaintWhileHidden`) runs the normal UI phase.

| Work | Current site | Decision |
|---|---|---|
| Control requests, synthetic input queue | `logic` `drain_control`; `raw_input_hook` `take_synthetic_step` | Keep the queue in logic. `take_synthetic_step` may consume on a hidden logic tick, so set `synthetic_in_flight` in the hook, send `RequestPaintWhileHidden` from `logic` until a real UI pass consumes the preserved events, and clear the flag in `App::ui` after that pass. Do not use the end of `ui` as the only wake. |
| Screenshots | `logic` issue/collect | Keep in logic; bounded slot; the screenshot command itself requests paint while hidden, so a hidden screenshot pass is not evidence that synthetic input would have woken the app. Use rendered-frame identity + monotonic settle deadline. |
| Job polling | `jobs_ui::tick` in logic; `jobs_ui::keep_polling` at end of ui | Move `keep_polling` to logic or call from both. Consume job results by revision/sequence exactly once per pass; a repeated pass must not re-apply a result. |
| Autosave/recovery | `format::Autosaver` background thread; `on_exit` saves tool memory | Use `std::time::Instant` (or `web_time::Instant`) for intervals and deadlines, never `ctx.input(\|i\| i.time)` in logic. Ensure `on_exit` runs for both renderers. |
| Preferences/theme/fonts | `logic` syncs language and calls `install_fonts`; `ui` applies theme on first frame | Keep setup in `setup_context` when possible; `logic` must finish setup if the creator did not. |
| File drops | `logic` drains OS/web inboxes and dropped files | Keep; `RequestPaintWhileHidden` when a drop arrives with no visible frame. |
| Native menu | `logic` runs menu; `ui` syncs it | `native_menu::sync` is UI-dependent; leave in ui, but queue menu events for the next paint. |
| GPU status/device loss | `logic` calls `check_gpu`; `ui` calls `check_window` | `check_gpu` stays in logic; `check_window` can stay in ui because it reports a painted frame, not a logic-pass count. |
| UI-local persistence (`tool_memory`, `last_brush`, `view_cmds`) | end of `ui` | Only runs when UI runs; acceptable because no user edit happened while hidden, but `on_exit` must flush the last state. A second pass must not write twice. |

`shortcuts::handle`, `drain_control`, `file_ui` drops and `jobs_ui::tick` all run
per pass. Give each a consumed queue (drain once), a revision/generation check, or
a monotonic deadline; do not count logic calls as rendered frames. Do not start
stroke tracing or interpret fresh input on a logic-only tick: gate it on
`synthetic_in_flight`/a real UI phase, or move the start to `App::ui`.

Avoid a blanket `Context` bridge. Use `ctx.input`, `ctx.data`, `ctx.memory` only
where the existing code already does; do not replay the whole UI from `logic`.
Do not poll a prior UI event twice in `run_logic`; the events there are stale.

### 8.2 Multiple passes

`Context::run_ui` and `Context::run_pass` share the root Ui; plugins run inside
`root_ui`. PhotoCraft's `setup_context` registers `CursorLifecycle`,
`SingleLineDrag`, `TextFieldMenu`, `UiFontSizePlugin`, `CjkFontPlugin`.
`on_begin_pass`/`on_end_pass` and popup sizing passes can execute content more
than once per frame. `current_pass_index` is not a durable "first pass" flag: a
completed `run_ui` leaves it at zero. Guard all side effects in `ui`:

- `self.frame += 1` stays in `logic`, but is a pass count; use
  `ctx.cumulative_frame_nr()` for rendered-frame delays;
- control/screenshot queues stay in `logic`;
- `notices::show` and persistence must be idempotent for repeated content passes;
- no counters inside popup/window closures that change on a sizing pass.

Action playback (`actions.rs`) and autosave (`format::Autosaver` /
`RecoveryStore`) are not UI loops: playback advances through engine commands,
and autosave is a background thread. Still, a playback step must not run twice
because a menu/popup content closure was re-run, and an autosave request must
not be duplicated by a repeated `ui` pass. Undo is engine-owned
(`photocraft-engine` history); the rule is one user action → one command → one
history step, never a command in a sizing pass with no user event.

Add tests for: a forced popup sizing pass (`Popup::sizing_pass`) with unchanged
document, undo stack and control replies; a playback step whose dialog content
pass repeats with unchanged step/history counts; a forced second `ui` pass with
one command; and a background completion between passes applied once.

### 8.3 Hidden-window and control input

`ui.pointer` and `ui.key` must work while the window is occluded or minimized.
Screenshots must work while occluded; while *minimized* the window has no surface
that can be trusted, so the request fails with an actionable "the window is
minimized" error instead of saving a blank/stale surface. The minimized flag is
part of the geometry epoch, so a minimize/restore between issuance and collection
retries. Send `ViewportCommand::RequestPaintWhileHidden` when synthetic input
is in flight, `input_waiters` is non-empty, or a screenshot/control reply waits on
a pass. Keep the existing reply deadlines (60 s) and `Dispatch`/`AfterInput`
semantics. A hidden window must not turn a queued request into a `timeout: not
run` error.

Test the real host order, not a once-per-frame fiction:

- visible single- and multi-pass frames;
- repeated hidden `update_logic_only` ticks after a completed multi-pass frame;
- a hidden forced UI paint with queued synthetic input, then a visible pass;
- a background completion between passes, applied once;
- bare `run_logic` followed by `run_ui` proving stale input is not replayed.

Screenshots request their own paint; a passing hidden screenshot does not prove
`RequestPaintWhileHidden` for synthetic input. The mutation check for that is
section 13.6.

## 9. Panels, docking and screenshots

### 9.1 Panel state

0.37 derives a panel id from its parent scope: for the root Ui,
`ctx.viewport_id().root_ui_id().with(salt)`. `Id::with` takes an `AsIdSalt`;
`Id::with_salt` takes an `IdSalt`, so the v1 helper's `with_salt(salt)` did not
compile. Add one helper in `panels.rs`:

```rust
pub fn panel_state_id(ctx: &egui::Context, salt: impl egui::AsIdSalt) -> egui::Id {
    ctx.viewport_id().root_ui_id().with(salt)
}
```

Add a compile-level unit test for the helper (and for the `direction.rs`
`AsIdSalt` bounds), then use it in `main.rs:1004`, `panels.rs:6604`,
`panels.rs:6934`, `workspace_ui.rs:594`, `panel_docking_tests.rs:569`, and in an
idempotent first-frame migration:

```rust
for name in ["dock", "title_bar", "options_bar", "status_bar", "toolbar", "rail", "shared-panel-dock"] {
    let new_id = panel_state_id(ctx, name);
    if PanelState::load(ctx, new_id).is_none() {
        if let Some(old) = PanelState::load(ctx, egui::Id::unique(name)) {
            ctx.data_mut(|d| d.insert_persisted(new_id, old));
        }
    }
}
```

This migration belongs to the **first compatible PR**: it is the same PR that
changes the constructors to `AsIdSalt`. The old `Id::unique(name)` key (same hash
as the old `Id::new(name)`) stays untouched, so 0.36 rollback still reads panel
widths; the new key is what 0.37 reads. `Ui::scope_id` is the renamed old
`Ui::id`; `Ui::unique_id` is unstable and must not be used for a persisted key.
Test old and new `ui.ron` fixtures, including a real 0.14.0 save with `dock`
width 410 and `shared-panel-dock` 290; then resize in 0.37, save and restart twice,
asserting the new width survives even though the old key remains. Also test the
helper/old-key equality and partially migrated state; existing new keys always win.

### 9.2 Window settings

Current `ui_state.rs` reads the old flat schema and is platform-dependent:
macOS checks `inner_position_pixels`, others `outer_position_pixels`
(`ui_state.rs:41-44`). 0.37's `WindowSettings` is:

- `normal_geometry: { outer_position_pixels, scale_factor, inner_size_points }`
  (`inner_size_points` is a plain `Vec2`, not `Option<Vec2>`);
- `fullscreen`, `maximized`.

The old value does not deserialize into 0.37. Merely "understanding both schemas"
is insufficient because eframe reads the file before PhotoCraft runs. Add an
explicit pre-load rewrite in `ui_state::sanitize` (called from
`apps/photocraft/src/main.rs:664` before `run_native`):

1. Parse the outer map of key -> RON string; leave a non-RON file alone.
2. If `window` is the old flat schema, write a new `normal_geometry` value:
   preserve valid `Some(inner_size_points)` using pattern matching (never `unwrap`);
   when missing, use PhotoCraft's existing configured default 1440×900 points (`native_options`, `main.rs:110`) and
   record that fallback; reject non-finite/oversize values without rewriting,
   `fullscreen`, `maximized`; set `scale_factor = 1.0` for the Windows position
   path. Preserve `outer_position_pixels` only on Windows (physical pixels; 0.37
   re-applies the physical position after creation). On macOS/Linux drop the old
   position and let the window center: the old macOS `inner_position_pixels` is
   not an outer position, and the old file has no scale factor for a logical
   Linux restore. Never guess a scale factor or move the window after first paint.
3. Keep a one-time `ui.ron.pre-0.37` backup before rewriting; log and leave the
   file untouched if the rewrite fails.

`has_saved_position` must look for `normal_geometry.outer_position_pixels` on
every OS and return true only when eframe actually restores that position (the
Windows path). It must stop looking for macOS `inner_position_pixels`.

Fixtures: old macOS inner position, old Windows/macOS outer position,
maximized and fullscreen, a `Some`/missing `inner_size_points`, and 0.36-schema
round-trip. Test that `(616,430)`/`1100x760` yields a 1100x760 window without a
startup failure, and that macOS/Linux center while preserving size/maximized/
fullscreen. Document the rollback loss: after 0.37 writes the nested schema,
0.36 ignores the window entry unless the backup is restored; add a
reverse-compatibility test using a 0.36-shaped settings struct.

### 9.3 Screenshot callback

Replace the UserData/Event path with a **bounded, cancelable per-request
completion slot**, not one unbounded `mpsc` channel per request. Use the shared
`craft-ui` capture-slot API at the approved CL-2F revision, a P0/P1 prerequisite.
There is no local-slot alternative for PhotoCraft. Keep tokens,
paths, settle delays, deadlines and reply schemas app-owned.

Design:

```text
PendingCapture record (app-owned)
  token, path, reply, deadline, issued_at, viewport_id,
  document/revision generation,
  issued_geometry: { logical_rect, pixels_per_point, expected_surface_size, geometry_epoch },
  slot: Arc<CaptureSlot>               // bounded, one result, canceled flag
CaptureSlot
  result: Mutex<Option<Arc<ColorImage>>>,
  canceled: AtomicBool,
  wake: egui::Context                  // live record only
callback = move |image| {
  let Some(slot) = weak_slot.upgrade() else { return }; // late/canceled: drop image
  if slot.store_if_pending(image) { slot.wake.request_repaint_of(viewport_id); } // after unlock
}
```

- The callback holds only `Weak<CaptureSlot>`; the live record holds the `Arc`.
  The slot may hold a `Context` clone for wakeup, but the callback must not hold a
  `Context`, `PhotocraftApp`, the pending record, or a channel sender. No
  callback -> Context -> callback cycle.
- Admit at most 4 pending requests, at most 128 MiB of image bytes per request and
  256 MiB in aggregate across reserved in-flight and completed images. Use checked
  width × height × 4 arithmetic and reject before issuance with a clear size/queue
  error. Test equality and one-over boundaries, arithmetic overflow and cancellation
  releasing capacity. Transient GPU readback/encoder RSS is measured separately;
  a late oversized result is dropped, not counted as proof of a pre-allocation bound.
- The callback supplies pixels only, not frame metadata. Record issuance geometry
  from the target viewport's UI and a monotonically increasing geometry epoch (resize,
  DPI/zoom/surface changes), plus document/revision identity. Revalidate before issuing
  and before saving. Any intervening epoch change, even a change back to the original
  size, rejects/retries within the deadline; never label the image with guessed geometry.
  If the epoch is unchanged, derive x/y pixel scale from returned image dimensions
  versus the recorded logical bounds, check aspect/rounding and apply region cropping
  with those scales. E2 can fit the surface smaller. Do not use current DPI alone to
  label an older image. A native resize/DPI race must show that stale metadata cannot
  produce a saved image; callback-only tests assert the same rejection policy.

- On timeout/close/cancel, mark the slot canceled and drop the record; a late
  callback upgrades `Weak` to `None`, drops the image, writes no file and sends
  no reply. There is no "late file/reply" path.
- Keep PNG encoding and the existing `ColorImage` -> PNG path on the app/UI
  thread in `control::save_screenshot`; never encode or write on the callback
  thread. The callback thread is arbitrary: `egui-wgpu` completes it from a
  `map_async` callback (`egui-wgpu/src/capture.rs:193`), not a guaranteed render
  thread.
- While a capture is pending, `logic` keeps a hidden-paint request alive. A
  screenshot command itself requests paint while hidden
  (`ViewportCommand::Screenshot` docs), so a hidden screenshot test does **not**
  prove synthetic-input `RequestPaintWhileHidden`; section 13.6 tests that
  separately.
- Settle with `ctx.cumulative_frame_nr()` plus a monotonic animation deadline,
  not `app.frame` (a logic-pass counter) and not a 3-pass count.

Tests: two concurrent captures with distinct tokens/paths and exactly one reply
each; callback exactly once; a callback after timeout/cancel writes no file and
sends no reply; close/shutdown drains to zero retained bytes; zero-sized surface
(`egui-wgpu` returns without calling the callback) times out cleanly with an
actionable error; unsupported capture format (also returns without callbacks)
times out; delayed GPU readback; out-of-order completion; reused viewport does
not bind an old capture to a new document; admission count and aggregate memory
caps (4 requests, 128 MiB/request, 256 MiB aggregate); a 4K resize/DPI race; `focus: true` raises first and correlates; `path:
null` base64; bad path/authority is an error, not a panic.

For distinct-image/out-of-order tests, intercept emitted screenshot callbacks and
complete them manually with different synthetic images in reversed order. Kittest's
normal renderer sends the same single rendered image to all callbacks and cannot prove
that correlation. Use an adapter-qualified native/GPU row for actual capture timing,
readback and viewport pixels; keep ordinary kittest for single-frame rendered checks.

### 9.4 Docking

`craft_ui::docking` remains the owner of the layout tree; PhotoCraft keeps
`UiState.docking` serialization. The 0.37 migration must not change
`Layout<String>`/`Location<String>` serde. Test:

- float a tab, move it, redock, split resize, close/reopen, save workspace,
  reload `UiState`; layout equality and panel rects;
- `Escape` during a first drag restores the legacy workspace;
- `workspace_locked` rejects moves atomically;
- floating group strip is opaque in Studio themes;
- panel IDs stay unique after tabs reorder.

### 9.5 GPU resource behavior

PhotoCraft's canvas is a custom `egui_wgpu` paint callback
(`crates/ui-egui/src/gpu_canvas.rs`), not an egui image. 0.37 changes the
surface integration, not the callback contract, but these must be checked:

- E2/E3: a root or child window larger than `max_texture_dimension_2d` is
  configured at a fitted size and drawn at a matching scale; a configure or
  renderer error returns `WgpuError::ConfigureSurface`/`CreateRenderer` and
  reaches the CPU-retry path, not a panic.
- `GpuCanvas::new` reads the device's `max_texture_dimension_2d` and tile size;
  `use_adapter_limits` requests the adapter limits on web/desktop. Confirm the
  egui surface limit and the canvas texture limit are independent.
- Texture deltas: assert `TexturesDelta::set/free` are applied once per frame, no
  texture leaks after a dialog closes, and screenshot readback does not retain a
  full-window buffer.
- Device loss: `gpu_status`/`DeviceHealth` still switches to the CPU canvas; no
  egui-wgpu panic on a lost device; `ui.gpu.simulateLoss` test remains green.
- Intel/DX12 texel guard: `texel_guard::install` still sees the same adapter;
  zeroed texels are not exposed after the 0.37 vertex/format changes.
- #8736: measure Windows `Renderer::new`/pipeline creation separately from the
  first frame, with ≥20 fresh processes. The reported 0.37 cost is around 57 ms vs
  14 ms for renderer construction and 197 ms -> 258 ms first frame on an
  Intel/DX12 system; this is an external report, not our measurement. Do not use
  a 200 ms startup allowance to hide it and do not accept clamp-only shader
  filtering changes for Repeat/MirroredRepeat.

### 9.6 Accessibility

Map all 114 `WidgetInfo` calls as in §4. Keep `check_accessibility` on and add
`Response::labelled_by`/`accessible_name` for fields without hints. Add a
screen-wide `unnamed_widgets().is_empty()` assertion for the main screen and
each dialog. After E17/E18, assert AccessKit root language and
`Response::accessible_language`.

## 10. Fonts, glyph coverage and text

### 10.1 Build and definitions

Required edits:

- `crates/ui-egui/Cargo.toml:73`: `epaint_default_fonts = { version = "0.37", features = ["monochrome_emoji_fonts"] }` (or the git-patched package with that feature).
- `crates/ui-egui/Cargo.toml:57`: dev `egui` features `["default_fonts", "monochrome_emoji_fonts"]` so the oracle test matches.
- `build.rs:37-45`: add `("egui-icons", epaint_default_fonts::EGUI_ICONS)`; keep the two emoji fonts behind the feature.
- `egui_fonts.rs:47-56`: add `egui-icons` at 0.90; preserve 0.37 family order:
  - monospace: `Hack`, `Ubuntu-Light`, `egui-icons`, `NotoEmoji-Regular`, `emoji-icon-font`
  - proportional: `Ubuntu-Light`, `egui-icons`, `NotoEmoji-Regular`, `emoji-icon-font`
- `egui_fonts.rs:79`: compare to `FontDefinitions::default()` with the same features.

Do not enable `default_fonts` in production; the web size gate depends on the
deflated fonts. No new font file is committed by this change: `egui-icons`,
NotoEmoji and emoji-icon-font come from `epaint_default_fonts`. Verify
`ATTRIBUTION.md` needs no new row; if a packaged font asset is added, add its
path/author/title/source/license row in the same first PR. The font feature,
equality test, glyph coverage and accessible-name fixes are all part of the first
green PR.

### 10.2 Lazy fallback

`cjk_fonts.rs` remains the app's fallback owner. Set
`NativeOptions::system_font_fallback = false` in the first cut. If
`system_fonts` is enabled later, remove or reconcile `cjk_fonts` in the same
change; two providers would produce non-deterministic font choices.

#8512 is still on PhotoCraft's path: `cjk_fonts` reads at most one system font per
frame even with `system_fonts` off. Measure font discovery and first-use
separately from renderer construction: cold/warm startup with `system_fonts`
off, with the app's lazy CJK fallback, and with an explicitly configured system
provider; record glyph coverage before/after. Do not assume disabling the
eframe feature removes system-font discovery cost.

`ctx.add_font` / `set_fonts` now apply immediately when no text has been laid out
in the pass. Re-check `defer_after_reset`, `fonts_ready`, and the
`UiFontSizePlugin::defer_after_install` one-frame assumptions; add a test where
fonts are installed from inside the first pass and where a language change
rebuilds fonts mid-run.

### 10.3 Glyph coverage

Add a test that loads every `tl!`/menu-catalog/catalog string and asserts
`Fonts::has_glyphs` for each character after `install_fonts` (with test fonts).
Include `✓ ▲ ▼ ▶ ◀ × … ⏵ ⋮`, `egui::special_emojis`, Arabic/Hebrew/Devanagari/
Thai samples, and the CJK samples already used by `cjk_fonts` tests. A missing
glyph must fail the test, not render tofu. This test and the minimal
`allow_missing_glyphs()` exceptions (only intentional tofu) ship in the first
green PR; do not let 481 harnesses inherit `MissingGlyphPolicy::Panic` without
the glyph decisions.

## 11. Input, IME, clipboard and RTL

- `raw_input_hook` stays; add `latin_layout_chars` handling only if E19 is in the
  qualifying revision. Fresh input is interpreted only in a known UI phase; the
  hook may move queued synthetic events into eframe's preserved input, but it
  sets `synthetic_in_flight` so `logic` forces a hidden paint and no other code
  treats the logic-only tick as a paint.
- `shortcuts::clipboard_keys` handles `Event::PasteImage` as the document paste
  shortcut through the existing `import_os_clipboard`/session clipboard path, not
  a second clipboard read; add a test for one import.
- Middle-click PRIMARY paste is new; verify text fields and the canvas do not
  both act on it.
- E4 regular clipboard and PRIMARY are separate acceptance rows. PRIMARY's
  default is the shared W15P winit-owned path on the existing connection/event
  queue with per-seat lifecycle and distinct PRIMARY protocol objects; never bind
  a second regular `wl_data_device`. The arboard PRIMARY fallback in the candidate
  is not adopted as the default. If a fallback is retained, record exact
  compositor/capability evidence and keep PRIMARY an unresolved gate.
- PRIMARY native matrix (wlroots, KWin, Mutter): keyboard-only focus, serial
  freshness, owner/seat changes, empty/absent offers, self-paste, canceled/slow
  transfers, password selection never offered, unchanged selection not reclaimed,
  and no second regular data device. Test regular/image/drop separately.
- With E21, add `ImeEvent::DeleteSurroundingBytes` to the Type tool's event match
  (`type_tool.rs:964-967`) and to any other exhaustive match; implement deletion
  in bytes with char-boundary checks, one undo step. E21 acceptance is
  macOS/Wayland only; Windows `ime_delete_surrounding` returns `None` in the
  candidate, so a green Windows IME test is not E21 coverage.
- With E21b, extend the Type tool's custom `IMEOutput` producer
  (`type_tool::draw_overlay`, `type_tool.rs:1255`) with the bounded logical
  surrounding-text payload: committed model text, preedit excluded, UTF-8 byte
  cursor/anchor, owning document+layer+viewport identity. Use
  `current_text`/`layout` and `byte_of` (char-to-byte); do not pass visually
  reordered display text. Resolve `TextEdit.document` (`active_doc_id` when
  `None`) and `layer` into the payload identity. When removing the preedit char
  range, a caret/anchor inside or at the end of it maps to the range start.
  Clamp to the payload's 4000-byte limit at char boundaries; if the selection
  cannot fit, emit explicit unavailable context.
  Owner/context generation is present even for `Unavailable`; `Available` empty text
  is a distinct case. Clear cached and compositor state with W27's committed
  disable/enable transition on owner change/unavailable/focus loss/reset/no IME;
  normal updates preserve composition. Clear old pending batches. In the current
  focus epoch, a mismatched `done` serial still applies incoming edits once and defers
  outgoing state until a match; old epochs never edit the new owner. Latest completed
  UI-pass output, including None, is authoritative. Password fields never supply text. Built-in
  `TextEdit` fields inherit the upstream producer. E21b's payload is not
  `Copy`; audit every `IMEOutput` constructor/destructure in PhotoCraft (the
  Type tool is the only custom one) and keep `Clone`/`Eq` semantics.
- With E5/E12/E13/E16, remove `bidi_text::visual`/`visual_ltr` use
  (`layer_row_ui.rs:132/137`, `panels.rs:719`) and the `unicode-bidi` dependency,
  or reduce `bidi_text` to non-reordering helpers. Double reordering is the
  most likely user-visible RTL regression.
- With E9, audit `direction.rs` call sites before setting `Style::right_to_left`.
  Do not mirror twice.
- #8763 selection loss: reproduce "select text, click a toolbar/context action"
  in PhotoCraft's custom text fields/menu. The fork owns the selected fix with
  pointer and keyboard regressions (fork P7c, prerequisite in P0). A local workaround
  requires a demonstrated app-specific gap and removal condition; it is not an undecided
  alternative to the shared fix.
- Test keyboard/focus: Tab order, Escape, Enter, focus after dialog close,
  IME preedit/commit, dead keys, grapheme deletion, RTL caret affinity, and the
  selection-action row.

## 12. Implementation stages

### P0 — fork qualification (outside PhotoCraft)

Publish/qualify canonical egui and winit revisions, including P2b bounded transfers, P7 key pairing, E21b/P7b with W27 clear/lifecycle,
W15P and P7c selection/viewport regression fixes, and publish the craft-libs port
and CL-2F capture helper on current `main`. Acceptance: `PATCHES.md` complete;
each patch and W15P/E21b/P7c test; native Linux/Windows/macOS compile and targeted
tests; public immutable full SHAs; no path/branch patches. Stock 0.37 and aiongg
candidates remain research controls. No PhotoCraft PR can claim 0.37 until this
exists.

### P0b — baseline harness and measurement (before production)

On `ff473881` / 0.36.2, add or extend the harnesses that will judge the migration,
without changing production behavior:

- the fixed 120-label plain-text tessellation scene and a benchmark row;
- Windows renderer-construction/first-frame timing around `Renderer::new`;
- the select-text-then-click-action regression (kittest and native);
- screenshot lifecycle probes (timeout, cancel, late callback, zero-size surface)
  against the 0.36 API, adapted to the 0.37 slot contract later;
- panel/window migration fixtures containing a real 0.14.0 `ui.ron`;
- the paired baseline measurements and raw JSON/machine fingerprint.

This PR and its evidence land before P1. A harness invented after production
changes is not a before/after control. Record pre-existing baseline failures
separately; do not fabricate a baseline from a failing binary.

### P1 — first green 0.37 PR (atomic compatibility)

This is the first change set that must pass its own tests, assets, glyph,
accessibility, feature and gate checks. It includes:

- root/app/ui `Cargo.toml`, `Cargo.lock`, root `[patch.crates-io]` for the whole
  family and winit; features `wgpu`, `accesskit`, `wayland`, `x11`, `persistence`,
  `clipboard`; `epaint_default_fonts` with `monochrome_emoji_fonts`; dev
  `default_fonts` for the oracle;
- `crates/ui-egui/build.rs`, `egui_fonts.rs`, `cjk_fonts.rs` `FontData` uses,
  `TextEdit::hint_text` helpers, `theme.rs` family order;
- accessibility names and minimal glyph-policy decisions required by kittest's
  default `check_accessibility`/`MissingGlyphPolicy::Panic`;
- mechanical `WidgetType`/`Id::new`/`ui.id`/panel/direction migration, including
  the `panel_state_id` helper and the first-frame panel migration in this same PR;
- `lib.rs` screenshot slot, `control.rs`, `shortcuts.rs` paste handling,
  `menu_nav.rs`/`field_wheel.rs` scroll offsets, `main.rs` NativeOptions,
  `clipping_indicator_tests.rs`;
- `ui_state.rs` window-settings rewrite and backup, with old/new fixtures;
- E21b Type-tool producer and compatible `IMEOutput` literal, E21 byte deletion,
  owner/clear/final-pass semantics, with native evidence tracked explicitly;
- screenshot completion slot and its lifecycle tests; panel/window persistence
  fixtures; no duplicate replies; no lost panel widths;
- any `ATTRIBUTION.md` change needed by a packaged asset (none expected), and the
  corrected perf/snapshot/wasm gate commands.

Acceptance: `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo xtask layers`,
`cargo xtask wasm`, `cargo test -p photocraft-ui-egui --lib --tests`,
`cargo test -p photocraft --tests`, the build.rs font equality test, glyph
coverage test, explicit expected roles/names on the main screen and every opened dialog,
`unnamed_widgets` empty as a supplementary check, `Event::PasteImage` one-import regression,
and `packaging/web/package.sh`
(per-file limit). No deprecation warnings and no globally disabled kittest check.

### P1b — visual fixture expansion (follow-up, not a green requirement)

Files: `snapshot.rs` scripts/fixtures, `panel_docking_tests.rs`,
`panels.rs` visual capture, kittest harness helpers.

Acceptance: baseline vs candidate images for 6 representative themes × 3 scales,
exact geometry JSON; deliberate broken probe fails. This expands coverage; it does
not carry the names/glyph/feature fixes that P1 already needs to be green.

### P2 — extended language and native IME qualification

Files: `bidi_text.rs` removal/reduction; `direction.rs`/E9 audit;
`i18n::sync_context` + `Frame::set_ui_language`; `layout_keys.rs`; AccessKit language
and extended native tests. The final fork pin and required E21/E21b model adapter
already land atomically in P1; P2 does not introduce a second dependency stage.
Acceptance: E5/E12/E13/E16/E17/E18/E19/E20/E21/E21b tests; RTL/grapheme/IME
matrix; E21b outgoing text after commit/deletion, preedit exclusion, byte
indices, long selection, field/window switch and multipass final output; native
Wayland predictive/Vietnamese trace or an explicit unresolved gate; no wasm size
regression.

### P3 — web browser qualification

Files: `apps/photocraft-web` only if API changes; `WebShell` behavior tests.
Acceptance: `cargo xtask wasm`, `packaging/web/package.sh`, browser workflow
(open/type/paste/drop/screenshot); every shipped file ≤24 MiB and gzipped total
plus raw wasm within a stated tolerance (+3%) of baseline.

### P4 — performance and cleanup

Files: benchmark additions only if needed; devlog and parity/localization docs.
Acceptance: paired perf protocol below; cleanup of temporary shims; no production
change without a review.

Dependency DAG:

```text
P0 forks/craft-libs (E21b, W15P)
   └── P0b baseline harness/measurement
          └── P1 first green compatibility (names/glyph/features/panel/window)
                 ├── P1b visual expansion
                 └── P2 extended language/native qualification ── P3 web
                                                   └── P4 perf/cleanup
```

Rollback: revert P1 → old lock and 0.36 family plus the preserved `ui.ron`
backup (panel keys remain readable; window geometry needs the backup); P1b is
additive tests/fixtures; P2 reverts to the P1 fork rev, restores `bidi_text.rs`
and removes the E21b adapters as one API set; P3 is web-only; P4 is
evidence/docs.

### 12.5 Required gates

Run through the server task wrapper with one resource task and a fresh job ID per
command; do not run these directly as a delegated worker:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p photocraft-ui-egui --lib --tests
cargo test -p photocraft --tests
cargo test -p photocraft-web
cargo test -p photocraft-tablet
cargo test -p photocraft-automation
cargo xtask layers
cargo xtask wasm
cargo xtask parity
cargo xtask scorecard --check
cargo test -p photocraft-engine --test panic_hunt -- --ignored
```

Conditional gates: `cargo xtask test-corpus` if `io`, `psd`, `codecs`, `compose`,
`gpu`, `text` or `format` change (not expected in the UI-only stages);
`cargo xtask perf --quick` after each stage; full `cargo xtask perf` before the
performance sign-off, including `text_tessellation` and the Windows renderer row.
Windows/macOS native checks follow the corresponding server workflow and their own
VM/helper procedures.

Do not claim a gate passed from a compile-only run. A skipped optional test
(e.g. missing `CRAFT_FONTS_DIR`) is recorded as skipped, not passed. Baseline
evidence for P0b (raw samples, machine fingerprint, lock/source hashes) must exist
before any P1 production edit is measured.

## 13. Test plan: user-observable assertions

### 13.1 Stable fixtures

- Synthetic documents with fixed seeds: 1440×900 UI; a 400×300 document; a
  20 MP layered document; a 36 MP photo; a 14000×14000 document. Store fixture
  generation seed and SHA256.
- Themes: `ProMedium`, `Studio`, `Classic`, `Adwaita`, `BreezeDark`,
  `SoftLight` (representative; full 14-theme matrix nightly).
- Scale factors: 1.0, 1.5, 2.0; viewports 800×480, 1440×900, 3840×2160.
- Persistence fixtures: a real baseline `ui.ron` saved by 0.14.0, plus a
  hand-edited old-schema file.

### 13.2 Geometry and hit tests

- Panels are shown in the expected order (title/options/status/toolbar/dock,
  then central); no overlap; each panel is nested in the root Ui, not a child;
  central canvas rect positive; clipping contains the canvas; panel state IDs
  unique; panel sizes restore from both old and new `ui.ron` keys.
- Dock: float/redock/split/resize/close/reopen; exact rects within 0.5 pt.
- Menus: long menu scrolls to last row; arrows and wheel; submenu hidden-row
  behavior; taskbar-aware placement.
- Toolbars: options bar controls hit at 1x/1.5x/2x; no dead zones from new
  atom padding.
- Canvas: brush stroke, pan, zoom, crop box, transform handles; document pixels
  and overlays.

### 13.3 Pixel comparisons

Use `snapshot --out <png> --size <w>x<h> --scale <s> --system-theme <light|dark|none>
--script <json>` for each fixture. `--system-theme` only selects the OS
appearance; select a PhotoCraft theme through the control script, e.g.
`--script '[["ui.set",{"theme":"studio"}]]'`. Compare baseline and candidate with
the same wgpu adapter/driver/DPI. Proposed tolerance:

- 99.5% of pixels have per-channel delta ≤ 3;
- no pixel has delta > 12;
- geometry JSON (panel rects) equals within 0.5 pt;
- text antialiasing may differ only inside this tolerance; a larger diff is a
  font/style decision, not noise.

Justify any wider tolerance with the exact font/renderer change. Do not update
goldens to make a test pass.

### 13.4 Accessibility and semantics

- `unnamed_widgets()` empty for main screen and every dialog.
- Roles: buttons, checkboxes, radio buttons, sliders, spin buttons, combos,
  color wells, text inputs, disclosure triangles.
- Names/labels: icon-only buttons have accessible names; text fields have labels
  or placeholders; selected state is reported.
- After E17/E18: AccessKit root language and `Response::accessible_language`.
- RTL: reading order and label language; no double-reversed Arabic.

### 13.5 Keyboard, focus, selection and IME

- Tab/Shift+Tab through every field; Enter/Escape; focus returns after dialogs.
- Selection regression (#8763/#8764): select text, click a toolbar/context action;
  the intended selected range reaches the action, pointer and keyboard paths
  agree, and undo restores one step. Run both kittest and a real native workflow.
- IME: Japanese/Korean/Chinese preedit and commit; macOS press-and-hold;
  Wayland text-input-v3; Windows IME. `DeleteSurroundingBytes` deletion and one
  undo step are E21 coverage on macOS/Wayland only; Windows IME is general
  coverage, because the candidate returns `None` there.
- E21b: outgoing surrounding text immediately after delete+commit and
  deletion-only batches; preedit excluded; reversed selection; supplementary/
  combining/RTL logical order; cursor movement; >4000-byte line; two documents;
  field/window switch; focus/reset/late serial; password/no-IME clears; multipass
  final output; one undo. A native Wayland Vietnamese/predictive-IME protocol trace
  is an external gate.
- Non-Latin layouts: Russian/Arabic/Hebrew shortcuts; German bracket keys;
  Dvorak brackets; `⌘V` on Dvorak-QWERTY ⌘.
- Clipboard: text copy/paste/cut, image paste (`Event::PasteImage`), PRIMARY
  middle-click, file-list paste, empty clipboard, occupied clipboard.
- PRIMARY: keyboard-only focus, serial freshness, owner/seat changes, empty or
  absent offers, self-paste, canceled/slow transfer, password exclusion,
  unchanged-selection reclaim policy. Run on wlroots, KWin and Mutter; record
  capabilities. A green arboard-only run is not PRIMARY acceptance.

### 13.6 Broken-behavior probes (mutation checks)

- Temporarily remove one `widget_info` label in a test-only copy → accessibility
  test must fail.
- Temporarily pre-reorder an Arabic label with `bidi_text::visual` while E5 is
  active → RTL fixture must fail.
- Temporarily set the panel state key to the old ID → persistence test must fail.
- Temporarily return a fixed screenshot token → correlation test must fail.
- Temporarily disable the slot's late-callback drop → a timeout/cancel probe must
  fail (no file/reply).
- Temporarily disable `RequestPaintWhileHidden` for synthetic input → hidden
  control test must time out/fail. Do this separately from a hidden screenshot:
  the screenshot command itself requests paint, so it cannot prove the synthetic
  path.
- Temporarily replay a consumed screenshot/control event across a second pass →
  the duplicate-reply/command test must fail.

### 13.7 Headless `Context::run_ui` and `run_logic`

Every new logic/ui boundary must be exercised without eframe, with the real
callback order:

- `Context::run_ui` with the app's `ui` body, a fixed `RawInput`, and no GPU:
  assert panel geometry, control replies and state transitions.
- Pass the existing `&mut Ui` to code that needs it. `Context::root_ui` **panics**
  when the root Ui is already borrowed (0.37 `context.rs:1011-1024`); test that
  documented panic as a negative test and test a successful `run_pass`/`root_ui`
  call separately. Never introduce a recursive `root_ui` in production.
- A hidden-tick driver must call `App::raw_input_hook` in eframe's order
  (`prepare_raw_input`) before `App::logic`, not bare `run_logic`, and then run a
  later `run_ui`: assert synthetic/control input is preserved and consumed once,
  and that stale events/time are not replayed. Use an eframe/kittest hidden-window
  driver or a direct `update_logic_only` equivalent; do not invent a native
  once-per-frame host.
- A headless pass must not be treated as proof of native focus, IME, clipboard
  ownership or file pickers; those need the native rows below.

### 13.8 Native and browser workflows

- Linux X11 and Wayland (wlroots/KWin and GNOME/Mutter), Windows 11, macOS.
- Real focus/IME/file pickers cannot be proven by kittest; record named hardware
  rows and screenshots. macOS evidence is an unresolved gate until real hardware
  rows exist; do not mark the migration complete on Linux-only evidence.
- Browser: WebGPU and WebGL2; open a file, type in a field, paste, drop a file,
  take a screenshot, resize the page; verify no JS errors in the console.
- Extra document windows: open two, draw in each, close one during a stroke,
  screenshot the root while a child is open, and confirm control input still
  reaches the root; immediate-viewport raw input must not be duplicated.
- #8748 is closed/unmerged and PhotoCraft has no deferred viewports: record that
  explicit non-applicability, then test hidden/occluded root + immediate child
  open/update/close (including a child left open when the root is hidden).

## 14. Performance protocol

### 14.1 Baseline

Before P1 (the first production change), run the baseline on the **same** machine
that will run the candidate. The P0b harness is added on 0.36.2 first; stock 0.37
and the fork are controls, not replacement baselines.

- record `uname -a`, CPU model/cores, RAM, GPU adapter/vendor/driver, wgpu
  backend, OS build, Rust version, `Cargo.lock` SHA256, exact command lines and
  feature set;
- renderer: Vulkan (or Metal/DX12 as applicable), plus a separate `--cpu`
  software-renderer run;
- DPI: 1.0 and 2.0; viewport 1440×900, plus 3840×2160 for large-window tests;
- profile: `--release`, `--locked`, no `CRAFT_FONTS_DIR` unless the row is
  font-specific; record whether craft-fonts is embedded.

Run each row with the shared sample protocol; keep raw JSON. The repository's
existing 15-sample/5-sample bench counts and `interactive_bench --reps 5` are
smoke settings, not acceptance evidence. For migration evidence use at least:

- 10 warmups + 30 timed iterations for steady operations;
- 20 fresh processes for startup and renderer/font-discovery rows;
- at least 1,000 rendered frames for frame distributions; report p50/p95/p99 and
  only claim a p99 from those frames (never from 20/30/50 samples);
- 1 warmup + 5 samples for 196 MP / 2 GB rows is an exploratory smaller run, not
  tail acceptance; label it as such and do not report its p95/p99 as acceptance;
- interleave baseline/candidate (A/B/A/B) to control thermal drift;
- reject a run when load average is high or another build shares the machine;
  rerun rather than accepting a noisy result. Use a paired confidence interval
  where suitable and record peak RSS, allocations/texture uploads and idle
  repaint/CPU separately.

### 14.2 Metrics and thresholds

Thresholds are investigation triggers, not permissions to accept a regression.
A repeatable **>5% median** or **>10% p95** change is investigated; existing
stricter repository budgets win. Do not expand tolerance after seeing a failure;
an exception needs a documented tradeoff and owner approval before acceptance.
For sub-millisecond rows require the relative increase to exceed 0.05 ms or
corroborate it with stable instruction/allocation counts.

| Metric | Baseline source | Gate |
|---|---|---|
| UI pass p50 | new `frame_bench` or `perf` `ui_ms` | investigate >5%; no looser absolute allowance |
| UI pass p95 | same | investigate >10% |
| UI pass p99 | same, only from ≥1,000 rendered frames | report; investigate >10% when the sample supports it |
| Operation p50 / p95 | `perf_scenarios` rows / `interactive_bench` | investigate >5% / >10% |
| Peak RSS | `RssSampler` / `/usr/bin/time -l` | investigate >5%; report peak, not only completion |
| GPU bytes held | `perf_scenarios` `gpu_bytes` | investigate >5% |
| Texture uploads / vertices / UI allocations | new test-only counters from `FullOutput.textures_delta`, tessellation and a scoped allocator around the app pass | investigate >10% |
| Idle repaint/CPU | 60 s with no input: frame count, process CPU %, GPU util | any continuous repaint, new busy-loop or audio underrun is a failure |
| Startup to first frame | app log timestamps / `gpu_startup` | investigate >5% median or >10% p95; no +200 ms allowance |
| Renderer construction (#8736) | `Renderer::new`/pipeline timestamps, 20 fresh processes | investigate >5% median or >10% p95; separate from first frame |
| Font discovery (#8512) | cold/warm process trace with system fonts off/on | investigate >5% / >10%; report glyph coverage before/after |
| Text tessellation (#8731) | fixed 120-label `text_tessellation` row | investigate >5% median or >10% p95; compare 0.36, stock 0.37, fork |
| Selection action (#8763) | select-text-then-click-action regression | no lost range; pointer/keyboard agree; one undo step |
| Wasm size | `packaging/web/package.sh` output | every shipped file ≤24 MiB; gzipped total and raw wasm ≤ +3% vs baseline |
| Click-to-pixel latency | `stroke_trace` | investigate >5% median or >10% p95 |

Do not report compile duration as UI performance. Do not compare the committed
macOS `perf/baseline.json` with a Linux/Windows run. Do not auto-accept a changed
pixel golden or a changed budget; profile and either fix or document an explicit
accepted regression with owner approval.

### 14.3 Commands

All commands run through the server task wrapper, one resource task and a fresh
job ID per intentional execution:

```sh
python3 /home/codex/work/maintenance/task.py run \
  --job-id <task>-perf-baseline-01 --kind build --task <task> \
  --source /home/codex/work/CraftApps/egui037-migration-20261022/photocraft \
  --evidence /home/codex/work/ssd2/task-evidence/<task> \
  -- bash -c 'cargo xtask perf --bench perf_scenarios'
```

`xtask perf` appends its own `--json <target>/perf/<bench>.json`; it rejects a
user-supplied `--json` as an unknown option. After the run, copy
`$CARGO_TARGET_DIR/perf/perf_scenarios.json` and `summary.md` into evidence. The
bench's own `--json` is only for invoking the example directly.

Equivalent runs for `interactive_bench`, `fx_bench`, `bevel_bench`,
`camera_raw_bench`, `type_bench`, `large_image_bench`, `large_file_bench`, the new
`text_tessellation` row, plus `cargo xtask perf --quick` as a smoke check. Copy
raw JSON, `summary.md`, logs, machine fingerprint, command line, source SHA and
lock SHA into evidence. Startup/renderer/font rows run 20 fresh processes each;
frame rows use ≥1,000 rendered frames. Do not claim a p99 from an exploratory
small run.

## 15. Decisions and remaining gates

Resolved in revision 2 (they are requirements, not open choices):

- **E4 PRIMARY:** the shared W15P winit-owned path on the existing
  connection/event queue is the default; arboard is a measured fallback and an
  unresolved gate, never an adopted default.
- **WindowSettings:** preserve `inner_size_points`, `fullscreen` and `maximized`
  on every OS; preserve `outer_position_pixels` on Windows only; center on
  macOS/Linux; rewrite before eframe loads and keep a backup. Document the
  rollback loss.
- **PanelState:** migrate in the first compatible PR; a one-time width reset is
  not an accepted substitute.
- **craft-libs:** port only the `8614e4a`/`1764bca` hunks onto current `main`;
  keep `craft-print`/`craft-docpath` pins and unrelated vendors. Do not merge
  `b68de8c`/`ec8840a` trees.
- **PasteImage:** treat as document paste through the existing import path.
- **E21 and E21b:** implement both for the custom Type tool; E21 is
  macOS/Wayland-only, E21b adds the bounded logical surrounding-text producer.
- **E21b:** new prerequisite; `None` placeholders are not acceptance.
- **Screenshot:** bounded, cancelable weak-slot completion with capture-time
  geometry, no unbounded channel and no late file/reply.
- **Performance:** shared 10/30/20/1,000 sample minimums and >5% median /
  >10% p95 investigation; no looser absolute allowance.

Remaining open gates:

1. **Fork publication/ownership.** Canonical `storytold/egui` and
   `storytold/winit` full SHAs do not yet exist as published immutable commits
   containing E21b and W15P. Choosing aiongg branches as canonical would make an
   individual fork a required public dependency; choosing storytold requires
   pushing there. No approval is granted in this plan.
2. **E11 inclusion.** No PhotoCraft consumer enables `epaint/multithreading`.
   Decide whether the shared fork carries E11 for VectorCraft or whether
   PhotoCraft's qualifying revision excludes it.
3. **E9/`direction.rs` overlap.** Keep explicit shell mirroring and leave
   `Style::right_to_left` off, or switch to E9 and remove overlapping manual
   mirroring. Needs RTL fixtures first.
4. **System fonts / color emoji.** Keep bundled monochrome emoji and no
   `system_fonts`, or enable `system_fonts` (fontique + color fonts) and retire
   the custom CJK fallback. The latter needs a separate design and size/perf
   review; #8512 is measured either way.
5. **Final winit patch list.** W12/W14/W16 are not PhotoCraft-required.
   Confirm whether the shared published revision includes them; W15P, W26 and
   W27 are required regardless.
6. **External evidence gates.** Real macOS IME/focus, Windows native GUI and
   renderer timing, Wayland compositor/IME/PRIMARY, and browser runtime evidence
   remain gates. Cross-compilation, kittest and software-renderer VM results do
   not satisfy them.

## 16. Historical planning limitations (superseded by current status above)

- Planning only: no build, test, clippy, wasm, GUI, IME, screenshot or performance
  command was run. The historical audit log in the evidence directory shows the
  same first font build failure, but it is not fresh validation.
- Revision 2 addresses the v1 findings by source inspection, but it has not been
  re-reviewed. A fresh independent review of this exact document is required
  before any publication approval.
- The E/W patch audit is based on the candidate commits and `PATCHES.md`;
  every patch, including E21b and W15P, still needs a line-by-line review against
  its upstream PR/proposal and a test audit before publication.
- E21b has no existing implementation. Its egui payload, TextEdit producer,
  egui-winit Wayland adapter, PhotoCraft Type-tool adapter and native IME trace
  are all future work; no claim of existing surrounding-text support remains.
- Windows, macOS, Wayland and browser behavior are unverified. Headless kittest
  is not proof of native focus, IME, file pickers or compositor behavior.
- No final fork revision or `Cargo.lock` hash exists yet; the target lock cannot
  be produced until P0 is published with the reviewed full SHAs.
- The performance baseline machine and exact adapter/driver are not chosen; the
  thresholds above are pre-registered, not measured. P0b must add the before
  harness on 0.36.2 before P1.
- No commit, push, PR, comment or external message was made. Publication still
  requires explicit human approval for every push/new PR, with the exact
  publication and validation shown first.


## Local implementation update — 2026-10-23

The user clarified that reviewed plans remain local and implementation proceeds before publication. The earlier requirement to publish/land this plan first is superseded; pushes and PR publication still require explicit approval.

Canonical `storytold/photocraft` `main` now resolves to `e6d6fb45a40250f518d2e824236a8fdcaa4472c8`. The owned checkout was fast-forwarded to this revision. Earlier baseline SHAs and line anchors above are retained as historical review evidence; use this new revision and its exact lockfile for current-main integration and paired measurements. The original refs remain reachable.

Use release0.15.0 and its lockfile as integration baseline; preserve same-origin served fonts and encoded-path rejection, cumulative glyph raster budgets, Select and Mask tile-based edits/undo and new catalog keys. Retain0.14 upgrade fixtures too.

The complete static delta review and exact old/new object identities are retained in the task evidence: `canonical-diffs-refresh-20261023.json` and `overnight-consumer-review-20261023.md`. This amendment claims no new build, native GUI or performance acceptance.

## Local implementation progress — first compatibility/lifecycle stage (2026-10-23)

Local, uncommitted work only. No build, test, format, clippy, GUI or performance command
was run for this stage; compile and runtime results are pending the parent's managed
`task.py` jobs. Nothing is published.

Repair round after `egui037-photocraft-postport-review-20261023-root-v1.md`: the test-target
compile findings were fixed (`fonts_mut` for `has_glyphs` checks, the two missing
`MouseWheelSource` fields, the missing-size window test), together with the runtime findings:
geometry-change budget revalidation, monotonic epoch comparison across resize-and-back, a
bounded pre-geometry admission queue, exact-once synthetic reinjection, withdrawal of a
cancelled synthetic batch from preserved input, a monotonic active-document generation bound to
captures and synthetic input, a minimized-window refusal, bounded PNG encode copies, the panel
rollback stamp and web-shell `raw_input_hook` forwarding. The findings that remain open are
listed at the end of this section.

Implemented in this checkout (release 0.15.0 base `e6d6fb45`):

- **Dependency family**: workspace `egui`/`eframe`/`egui_kittest` 0.37, `egui_extras` 0.37,
  `epaint_default_fonts` 0.37 with `monochrome_emoji_fonts`, dev `egui` with `default_fonts`
  + `monochrome_emoji_fonts`, `eframe/clipboard` on the app, UI library and web shell, and a
  direct `web-time` dependency. The working tree's `Cargo.lock` is modified by the parent's
  local path patches for `egui-implementation`, `winit-implementation` and
  `craft-libs-implementation`; it is not a reviewable/publishable lock until the family and
  winit are pinned to published revisions and the lock is regenerated from them. The pinned
  `craft-ui` revision currently lacks `craft_ui::capture` (post-port review F1).
- **Fonts**: `build.rs` deflates `egui-icons` and the feature-gated emoji faces;
  `egui_fonts.rs` reproduces 0.37's faces, scale tweaks and family order; `cjk_fonts.rs` uses
  `FontData::bytes()`; a glyph test covers `egui::special_emojis` in both families;
  `NativeOptions::system_font_fallback = false` keeps PhotoCraft's own CJK/craft-fonts
  fallback the only provider.
- **Mechanical 0.37 API port**: `WidgetType` → `egui::Role` (with the renamed variants),
  `Id::new` → `Id::unique` (hash-preserving), `ui.id()` → `ui.scope_id()`,
  `Panel`/`direction` take `AsIdSalt`, `TextEdit::hint_text` takes owned text, `ScrollArea`
  offsets use `clamped_offset()` and a request applied through `vertical_scroll_offset`,
  `Event::PasteImage` maps to the document paste shortcut, and every `Event::MouseWheel`
  constructor carries `MouseWheelSource::Unknown`.
- **Panel persistence**: `panels::panel_state_id(ctx, salt)` is the 0.37 parent-scoped key;
  `panels::migrate_panel_state` copies the old `Id::unique(name)` `PanelState` entries, leaves
  the old keys intact for a 0.36 rollback, and stamps both keys.
  `panels::remember_panel_state` records the 0.37 size each pass. On a later start an old key
  that changed while the new one did not (a 0.36 rollback resize) wins; a 0.37 resize is kept.
  If both changed the order is unknown and the new key is kept, so a 0.37 resize after a 0.36
  rollback resize can still be lost; that ambiguity is documented and tested. Called from the
  first UI pass; tests cover key identity, the copy, the old-key retention, a 0.37 resize after
  migration and a 0.36 rollback resize (`panels.rs`), and panel widths across two restarts
  (`apps/photocraft/src/main.rs`).
- **Window settings**: `ui_state::sanitize` now rewrites a 0.36 flat `window` entry to
  0.37's `normal_geometry` before eframe reads it, after a one-time `ui.ron.pre-0.37` backup.
  It preserves a valid size (defaulting to 1440×900 when absent), `fullscreen` and
  `maximized`, preserves the old physical outer position only on Windows, and centres on
  macOS/Linux. Corrupt sizes are left untouched. `has_saved_position` reads only the nested
  outer position. Tests cover old macOS/Windows fixtures, missing size, corrupt values, the
  backup, the one-way reverse-compatibility loss, and maximized/fullscreen.
- **Screenshot lifecycle** (`crates/ui-egui/src/screenshot.rs`): bounded, cancelable capture
  requests using the shared `craft_ui::capture::CaptureSlot` with a weak callback. Admission
  enforces 4 outstanding requests, 128 MiB per image and 256 MiB aggregate over issued,
  completed-undrained, unissued and geometry-waiting requests, rejects before issuance with
  checked arithmetic, and is drained before admission so a finished request releases its
  budget. A request that
  arrives before the first UI pass has recorded the geometry waits in a count/deadline-bounded
  queue and is admitted on the next logic tick. When the geometry is finalized at issuance or
  after a retry, the per-image and aggregate budgets are recomputed and the reservation updated
  (or the request fails before any capture). The request identity is the document and its
  monotonic activation generation (revision recorded, not an invalidation: a switch away and
  back fails the capture). Geometry (size, scale, zoom, texture side, minimized) has a monotonic
  epoch updated only in a real UI pass; an epoch advance between issuance and collection retries
  even when the values change back, and the issuing pass must show the same activation at its
  start and end. Settling counts UI passes, issuance happens only from `App::ui` after fonts are
  ready, and deadlines use `web_time::Instant` (monotonic on native and wasm). Accepted images
  are the issued size within a pixel or the renderer's exact texture-side fit, never an
  arbitrarily smaller same-aspect surface. A late callback after timeout/cancel is inert;
  encoding/writing happens on the UI thread in `control::save_screenshot_rgba`, never in the
  callback. The retained image is dropped before the encoder allocates its RGBA copy, and that
  copy is bounded by the same per-image limit (`finish_captures` refuses an oversized image
  rather than allocating).
  Same-size surface recreation with no size/scale/zoom change is not observable and remains an
  explicit limitation.
- **Lifecycle**: hidden `run_logic` ticks request `RequestPaintWhileHidden` for unissued
  captures and queued synthetic input; one synthetic batch is in flight at a time, and a pass
  that returns early re-injects the same batch exactly once (a second hidden tick does not
  append it again). A document/focus change observed in `raw_input_hook` withdraws the batch
  (including events eframe already preserved) and fails the waiting replies, so cancelled input
  is never applied. A change made inside `logic` after the hook cannot withdraw events the pass
  has consumed, so the pass delivers them and keeps `automation_input` set instead of claiming
  cancellation while the input lands. Control replies wait for a pass that actually delivered
  the batch. The web shell now forwards `raw_input_hook` to the app after its touch-twin
  filter, so the clipboard `PasteImage` mapping, legacy pinch, pen-hold and titlebar
  normalization run in the browser too; the browser has no TCP control server, so the
  synthetic queue stays empty there and synthetic-input rows remain native-only evidence.
  `gpu_canvas::now_ms` is now monotonic on wasm (it was a constant 0), so autosave,
  preference retry backoff, frame caps and stroke traces measure real durations; preference
  retry and link-watch timing no longer read stale `run_logic` input time.

### Integration-test selector repair — 2026-10-24 (historical pre-validation record)

The parent's managed `--lib --tests` run (`egui037-photocraft-candidate-20261024-upstream-tests-18`)
passed 3847 UI unit tests and 116 integration tests, with 16 integration failures. All 16 were
kittest/AccessKit selector assumptions, not production behaviour changes:

- kittest 0.5 reads a `Role::Label` node's text from its AccessKit `value`, so a form caption and
  a control that carries the same accessible name both match `get_by_label`. Batch/Image
  Processor (`Format`, `Input`, `Quality`, `Starting serial`), custom print paper
  (`Paper Width`, `Paper Height`, `Top`, `Left`) and Export As (`Scale`, `Resample`) now select
  the actual `TextInput`/`SpinButton`/`ComboBox` by role and name; existing coordinate searches
  and ambiguous name-only lookups were removed. `custom_print_paper::shown` uses `query_all`
  because a caption and its field are both valid visible matches.
- egui 0.37 exposes a `CollapsingHeader` as `Role::DisclosureTriangle`, not `Role::Button`.
  Camera Raw histogram/settings and localization tests now click and assert that role; the
  section IDs and translated labels are unchanged.
- A Camera Raw slider row exposes both its `Slider` and its numeric `SpinButton` under the same
  name. `camera_raw_scope::alt_drag_slider` now addresses the `Slider` track directly instead
  of the caption, preserving the real Alt-drag and overlay/diagnostic assertions.

These edits keep the real clicks, drags, exported bytes, history/revision and engine-field
assertions; they add no sleeps, retries or relaxed checks. They do not claim a passing run: the
parent must re-run the managed full tests and lint. The broader 481-harness accessibility
naming/glyph sweep and visual fixture regeneration remain open.

The `ui.ron` fixtures under `apps/photocraft/tests/fixtures/` are schema probes reconstructed
from the documented layouts, not recorded machine saves (see that directory's README); they
prove the migration rules, not eframe/egui deserialization of a real save.

The compiler/API port, IME producer/adapter, byte deletion, selection-action adoption, shared
capture dependency and canonical lockfile are now implemented and covered by the current host
qualification above. The later fixture corrections preserve real native viewport geometry,
transform completion order, retired document editors and foreign viewport identity; they do
not bypass the reset acknowledgement path.

Remaining acceptance work: a real 0.36 docking-layout sample, the broader accessibility/glyph
sweep, visual fixture regeneration, packaged WASM size, native/browser/IME/PRIMARY workflows
and paired app performance. The app-owned docking JSON still falls back to the legacy layout
if it cannot deserialize; DockArea's transient drag/focus state is not persisted.
