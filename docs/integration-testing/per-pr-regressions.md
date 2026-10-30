> CURRENT SCOPE: 87 retained PRs. All 21 requested exclusions have been removed from this appendix. Rerun relevant cases against the new frozen SHA; older results are historical.

# Per-PR regression execution appendix

Each automated case uses the synthetic fixture/setup and assertions in the named checked-in regression,
linked at the reviewed head. The action is the command below. Expected: at least one matching test
executes, every assertion passes, and the behavior in the case label is retained. A zero-match run is
BLOCKED, never PASSED. After a moved-module port, locate the same test in the frozen integration
tree and retain its fixture and assertions. Manual combined cases are in the main test plan.

All cases initially NOT RUN. Record Actual, Evidence and Tester in the empty fields; the status vocabulary
is PASSED / FAILED / BLOCKED / NOT RUN / SKIPPED. Missing fixtures, unavailable adapters and unsupported
platforms are BLOCKED or SKIPPED with a reason, rather than silently passing.

## Reviewed-case execution rules

Run every listed test separately on the final frozen integration SHA, recording its final-tree source path,
full discovered name, actual count and outcome. Historical reviewed-head links establish provenance only.
Zero-match, ignored-only or early-return-without-required-capability runs are BLOCKED, not PASSED.
Locate preserved equivalent assertions when a test moved; never substitute an unrelated guard.
Automated and native/manual subcases have separate results. Use owned temporary settings/outputs;
never migrate actual user settings, publish a release, bypass security or force a driver crash.

## Final-tree path guide

Reviewed-head links below retain historical provenance. Use these current paths after module ports; the named test must still be discovered and executed on the frozen build.

| Case / area | Current source path |
|---|---|
| #6492 occupied control port | `apps/photocraft/tests/it/control_exit_code.rs` |
| #8347 type-tool checkbox/canvas coverage | `crates/ui-egui/src/tools/type_tool/canvas_tests.rs` |
| Engine command regressions | `crates/engine/src/cmd/` and the command's live test submodule; do not use deleted root monoliths |
| #7153 Liquify smart-filter editing | `crates/ui-egui/src/dialogs/liquify_ui/tests.rs` |
| #8896 Select and Mask | `crates/ui-egui/src/dialogs/select_mask_ui/tests.rs` |


## PR-3748: control: suggest near matches for unknown command ids and methods (#3674)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-automation --locked unknown_names_suggest_the_right_method_or_command -- --nocapture` — [crates/automation/src/rpc.rs / unknown_names_suggest_the_right_method_or_command](https://github.com/storytold/photocraft/blob/c45d01eafe84e600bb28033a6853cc19c827b7bf/crates/automation/src/rpc.rs)
  - `cargo test -p photocraft-automation --locked errors_are_tool_errors_not_crashes -- --nocapture` — [crates/automation/tests/mcp.rs / errors_are_tool_errors_not_crashes](https://github.com/storytold/photocraft/blob/c45d01eafe84e600bb28033a6853cc19c827b7bf/crates/automation/tests/mcp.rs)
  - `cargo test -p photocraft-engine --locked protocol_listing_method_points_at_command_list -- --nocapture` — [crates/engine/src/command_hints.rs / protocol_listing_method_points_at_command_list](https://github.com/storytold/photocraft/blob/c45d01eafe84e600bb28033a6853cc19c827b7bf/crates/engine/src/command_hints.rs)
  - `cargo test -p photocraft-engine --locked near_misses_suggest_registered_ids -- --nocapture` — [crates/engine/src/command_hints.rs / near_misses_suggest_registered_ids](https://github.com/storytold/photocraft/blob/c45d01eafe84e600bb28033a6853cc19c827b7bf/crates/engine/src/command_hints.rs)
  - `cargo test -p photocraft-engine --locked suggestions_are_capped_and_ordered -- --nocapture` — [crates/engine/src/command_hints.rs / suggestions_are_capped_and_ordered](https://github.com/storytold/photocraft/blob/c45d01eafe84e600bb28033a6853cc19c827b7bf/crates/engine/src/command_hints.rs)
  - `cargo test -p photocraft-engine --locked protocol_method_hints -- --nocapture` — [crates/engine/src/command_hints.rs / protocol_method_hints](https://github.com/storytold/photocraft/blob/c45d01eafe84e600bb28033a6853cc19c827b7bf/crates/engine/src/command_hints.rs)
  - `cargo test -p photocraft-text --locked tokenizer_and_distance -- --nocapture` — [crates/text/src/spell.rs / tokenizer_and_distance](https://github.com/storytold/photocraft/blob/c45d01eafe84e600bb28033a6853cc19c827b7bf/crates/text/src/spell.rs)
  - `cargo test -p photocraft-ui-egui --locked unknown_names_suggest_the_right_method_or_command -- --nocapture` — [crates/ui-egui/src/control.rs / unknown_names_suggest_the_right_method_or_command](https://github.com/storytold/photocraft/blob/c45d01eafe84e600bb28033a6853cc19c827b7bf/crates/ui-egui/src/control.rs)

- Expected behavior: unknown names suggest the right method or command; errors are tool errors not crashes; protocol listing method points at command list; near misses suggest registered ids; suggestions are capped and ordered; protocol method hints; tokenizer and distance; unknown names suggest the right method or command. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`unknown_names_suggest_the_right_method_or_command` setup/action excerpt:

```rust
let mut h = Headless::new();
        let err = |h: &mut Headless, m: &str, p: Value| h.handle(m, p).unwrap_err().to_string();
        let e = err(&mut h, "engine.execute", json!({"command": "engine.commands"}));
        assert!(e.contains("did you mean `command.list`") && e.contains("protocol method"), "{e}");
        let e = err(&mut h, "engine.execute", json!({"command": "doc.inspect"}));
        assert!(e.contains("`doc.inspect` is a protocol method"), "{e}");
        let e = err(&mut h, "engine.execute", json!({"command": "edit.undoo"}));
        assert!(e.contains("did you mean `edit.undo`"), "{e}");
        let e = err(&mut h, "doc.inspct", Value::Null);
        assert!(e.contains("did you mean `doc.inspect`") && e.contains("(try `methods`)"), "{e}");
        let e = err(&mut h, "edit.undo", Value::Null);
        assert!(e.contains("engine.execute {\"command\": \"edit.undo\"}"), "{e}");
```

Expected assertion context (overlapping windows merged):

```rust
assert!(e.contains("did you mean `command.list`") && e.contains("protocol method"), "{e}");
        let e = err(&mut h, "engine.execute", json!({"command": "doc.inspect"}));
        assert!(e.contains("`doc.inspect` is a protocol method"), "{e}");
        let e = err(&mut h, "engine.execute", json!({"command": "edit.undoo"}));
        assert!(e.contains("did you mean `edit.undo`"), "{e}");
        let e = err(&mut h, "doc.inspct", Value::Null);
        assert!(e.contains("did you mean `doc.inspect`") && e.contains("(try `methods`)"), "{e}");
        let e = err(&mut h, "edit.undo", Value::Null);
        assert!(e.contains("engine.execute {\"command\": \"edit.undo\"}"), "{e}");
    }
```

`errors_are_tool_errors_not_crashes` setup/action excerpt:

```rust
let client = connect(PhotocraftMcp::headless()).await;
    let r = call(&client, "command_run", json!({"id": "no.such.command"})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("unknown command"));
    // #3674: the error names the nearest registered id.
    let r = call(&client, "command_run", json!({"id": "engine.commands"})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("did you mean `command.list`"), "{}", text(&r));
    let r = call(&client, "doc_inspect", json!({"index": 0})).await;
    assert_eq!(r.is_error, Some(true));
    let r = call(&client, "doc_open", json!({"path": "/definitely/missing.png"})).await;
    assert_eq!(r.is_error, Some(true));
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("unknown command"));
    // #3674: the error names the nearest registered id.
    let r = call(&client, "command_run", json!({"id": "engine.commands"})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("did you mean `command.list`"), "{}", text(&r));
    let r = call(&client, "doc_inspect", json!({"index": 0})).await;
    assert_eq!(r.is_error, Some(true));
    let r = call(&client, "doc_open", json!({"path": "/definitely/missing.png"})).await;
    assert_eq!(r.is_error, Some(true));
    let r = call(&client, "ui_inspect", json!({})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("bridge"));
    let r = call(&client, "doc_export", json!({})).await;
    assert_eq!(r.is_error, Some(true));
    // Server still alive.
    json_of(&call(&client, "session_list", json!({})).await);
    client.cancel().await.unwrap();
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-4993: Prevent control-channel engine commands from editing behind modal dialogs

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked engine_execute_cannot_change_document_behind_adjustment_dialog -- --nocapture` — [crates/ui-egui/src/control.rs / engine_execute_cannot_change_document_behind_adjustment_dialog](https://github.com/storytold/photocraft/blob/81964dc5ef77a71a2ae2a938958cf3ed77d90719/crates/ui-egui/src/control.rs)

- Expected behavior: engine execute cannot change document behind adjustment dialog. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`engine_execute_cannot_change_document_behind_adjustment_dialog` setup/action excerpt:

```rust
let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 60, "height": 40, "background": "#3366cc"})).unwrap();
        let opened = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.adjustments.hueSaturation"}));
        assert_eq!(opened["ok"], true, "{opened}");
        let dialog = opened["result"]["dialog"].as_u64().unwrap();
        let revision = app.session.active().unwrap().revision;
        let fields = call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["dialogs"][0]["fields"].clone();

        for method in ["engine.execute", "ui.menu.invoke"] {
            let blocked = call(&mut app, &ctx, method, json!({"command": "image.adjustments.invert"}));
            assert_eq!(blocked["ok"], false, "{method}: {blocked}");
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(opened["ok"], true, "{opened}");
        let dialog = opened["result"]["dialog"].as_u64().unwrap();
        let revision = app.session.active().unwrap().revision;
        let fields = call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["dialogs"][0]["fields"].clone();
            assert_eq!(blocked["ok"], false, "{method}: {blocked}");
            assert_eq!(app.session.active().unwrap().revision, revision, "{method} changed the document");
            let snapshot = call(&mut app, &ctx, "ui.inspect", json!({}));
            assert_eq!(snapshot["result"]["dialogs"][0]["id"], dialog);
            assert_eq!(snapshot["result"]["dialogs"][0]["fields"], fields);
        }

        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);
        assert_eq!(call(&mut app, &ctx, "engine.execute", json!({"command": "image.adjustments.invert"}))["ok"], true);
        assert!(app.session.active().unwrap().revision > revision);
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-5176: Play CopyCat, AJ and Line and Color action sets; keep patterns across restarts; Shift Edge scale

- Fixture/setup: Synthetic ATN mappings, smart-object Edit Contents action, temporary pattern stores and matting hard/soft edges.
- Action: run each command separately:

  - `cargo test -p photocraft-engine --locked every_mapped_command_exports_and_imports_back -- --nocapture`
  - `cargo test -p photocraft-engine --locked save_in_edit_contents_updates_the_smart_object -- --nocapture`
  - `cargo test -p photocraft-engine --locked patterns_survive_a_restart -- --nocapture`
  - `cargo test -p photocraft-engine --locked corrupt_pattern_files_are_skipped_and_kept -- --nocapture`
  - `cargo test -p photocraft-engine --locked large_patterns_load_on_use -- --nocapture`
  - `cargo test -p photocraft-algo --locked matting::tests -- --nocapture`

- Expected: Mappings round-trip parameters; Save in Edit Contents updates parent smart content. Pattern names/pixels and rename/delete survive restart; corrupt assets are warned about and preserved; large patterns load full pixels lazily. Hard edge ±100% shifts one pixel, ±50% gives half coverage within 0.02; feather 4/+100% reaches four pixels.
- Separate coverage / prerequisites: Named CopyCat/AJ/Line and Color playback needs licensed fixture identities and required document/pattern setup; missing action sets leave that subcase BLOCKED. Mapping tests alone do not prove full set playback. Also run all new legacy/new-store migration, interruption, corruption, duplicate-ID and deletion-authority regressions.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-6492: Fail control startup promptly when its port is occupied

- Fixture/setup: Process-isolated occupied loopback listener and successful prebound control-server fixtures in apps/photocraft tests.
- Action: run each command separately:

  - `cargo test -p photocraft --locked an_occupied_control_port_exits_with_status_2 -- --nocapture`
  - `cargo test -p photocraft --locked a_prebound_listener_authenticates_and_dispatches_requests -- --nocapture`

- Expected: Occupied port exits with status 2 before window/GPU initialization; successful prebound control authenticates and dispatches requests. Preserve timing, cleanup and child-exit assertions.
- Separate coverage / prerequisites: Use only the test-owned listener and process.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-8905: fix(plugins): bound a run's total plug-in memory, not just each instance

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-plugins --locked parallel_bands_fit_the_total_memory_budget -- --nocapture` — [crates/plugins/tests/abi.rs / parallel_bands_fit_the_total_memory_budget](https://github.com/storytold/photocraft/blob/0fdc6259193aab32b8feab7347ddc36196f40a19/crates/plugins/tests/abi.rs)
  - `cargo test -p photocraft-plugins --locked one_band_at_a_time_matches_parallel_bands -- --nocapture` — [crates/plugins/tests/abi.rs / one_band_at_a_time_matches_parallel_bands](https://github.com/storytold/photocraft/blob/0fdc6259193aab32b8feab7347ddc36196f40a19/crates/plugins/tests/abi.rs)

- Expected behavior: parallel bands fit the total memory budget; one band at a time matches parallel bands. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`parallel_bands_fit_the_total_memory_budget` setup/action excerpt:

```rust
let d = Limits::default();
    assert_eq!(d.parallel_bands(16), 4);
    assert!(d.parallel_bands(64) * d.max_memory_bytes <= d.max_total_memory_bytes);
    assert_eq!(d.parallel_bands(2), 2, "fewer workers than the budget allows");
    let small = Limits { max_memory_bytes: 64 << 20, ..Limits::default() };
    assert_eq!(small.parallel_bands(16), 16, "small instances keep every worker busy");
    let tight = Limits { max_total_memory_bytes: 1 << 20, ..Limits::default() };
    assert_eq!(tight.parallel_bands(16), 1, "at least one band runs");
    assert_eq!(d.parallel_bands(0), 1);
}
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(d.parallel_bands(16), 4);
    assert!(d.parallel_bands(64) * d.max_memory_bytes <= d.max_total_memory_bytes);
    assert_eq!(d.parallel_bands(2), 2, "fewer workers than the budget allows");
    let small = Limits { max_memory_bytes: 64 << 20, ..Limits::default() };
    assert_eq!(small.parallel_bands(16), 16, "small instances keep every worker busy");
    let tight = Limits { max_total_memory_bytes: 1 << 20, ..Limits::default() };
    assert_eq!(tight.parallel_bands(16), 1, "at least one band runs");
    assert_eq!(d.parallel_bands(0), 1);
}
```

`one_band_at_a_time_matches_parallel_bands` setup/action excerpt:

```rust
let pool = rayon::ThreadPoolBuilder::new().num_threads(8).build().unwrap();
    let s = surface(PixelFormat::RGBA8, 600, 300);
    let canvas = Rect::new(0, 0, 600, 300);
    let run = |limits: Limits| {
        let p = Plugin::load(INVERT, limits).unwrap();
        pool.install(|| p.apply(&s, canvas, None, &json!({}))).unwrap()
    };
    let parallel = run(Limits { band_bytes: 1, ..Limits::default() });
    let serial = run(Limits { band_bytes: 1, max_total_memory_bytes: 512 << 20, ..Limits::default() });
    assert_eq!(serial, parallel);
    assert_ne!(serial, s, "the filter ran");
}
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(serial, parallel);
    assert_ne!(serial, s, "the filter ran");
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-3874: fix(layer-style): support Blend If Alt-split handle feathering and throttle live preview

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked blend_if_alt_split_and_drag_behavior -- --nocapture` — [crates/ui-egui/src/layer_style.rs / blend_if_alt_split_and_drag_behavior](https://github.com/storytold/photocraft/blob/92488307722c3b840507827cac7ee9a88470fc8a/crates/ui-egui/src/layer_style.rs)

- Expected behavior: blend if alt split and drag behavior. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`blend_if_alt_split_and_drag_behavior` setup/action excerpt:

```rust
// Case 1: Alt-drag on unsplit black handle [0, 0, 255, 255] splits the right half
        let mut mode = BlendDrag::Pair { pair: 0, origin_v: 0 };
        let q = [0, 0, 255, 255];
        let q1 = step_blend_drag(&mut mode, &q, 45, true);
        assert_eq!(q1, [0, 45, 255, 255], "Alt-drag right must split black triangle into [0, 45]");

        // Case 2: Moving the split right half further to 70
        let q2 = step_blend_drag(&mut mode, &q1, 70, false);
        assert_eq!(q2, [0, 70, 255, 255]);

        // Case 3: Dragging the split half back to 0 merges the handle
        let q3 = step_blend_drag(&mut mode, &q2, 0, false);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(q1, [0, 45, 255, 255], "Alt-drag right must split black triangle into [0, 45]");

        // Case 2: Moving the split right half further to 70
        let q2 = step_blend_drag(&mut mode, &q1, 70, false);
        assert_eq!(q2, [0, 70, 255, 255]);

        // Case 3: Dragging the split half back to 0 merges the handle
        let q3 = step_blend_drag(&mut mode, &q2, 0, false);
        assert_eq!(q3, [0, 0, 255, 255], "Dragging back to origin reunites the triangle");

        // Case 4: Alt-drag on unsplit white handle [0, 0, 255, 255] splits the left half
        let mut mode_w = BlendDrag::Pair { pair: 2, origin_v: 255 };
        assert_eq!(qw1, [0, 0, 200, 255], "Alt-drag left must split white triangle into [200, 255]");

        // Case 5: Dragging without Alt moves both halves together
        let mut mode_solid = BlendDrag::Pair { pair: 0, origin_v: 0 };
        assert_eq!(qs1, [35, 35, 255, 255], "Drag without Alt moves solid handle together");

        // Case 6: Clamping prevents black handle from crossing white handle
        let q_close = [0, 100, 150, 255];
        assert_eq!(q_clamped, [0, 150, 150, 255], "Black max cannot exceed white min");
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-4023: i18n(es): add 548 missing rows, fix five filter and video terms

- Fixture/setup: Select Spanish UI, synthetic document, and menus/dialogs for video footage, Blur Gallery, Pixelate and Sharpen. Record Spanish language selection and screenshots.
- Action: Check Interpret Footage = Interpretar material de vídeo; Replace Footage = Reemplazar material de vídeo; Tilt-Shift = Desenfoque de inclinación y desplazamiento; Mezzotint = Grabado a media tinta; Smart Sharpen = Enfoque inteligente. Check their ellipsis variants in menus. Exercise a Count label with value 3 and confirm Recuento: 3, with no literal {n}.
- Expected: Specified Spanish strings match the final catalog, ellipses remain where defined, and values replace placeholders without English fallback or raw {n}/{file} tokens.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester:

## PR-4711: Treat Dissolve as a per-pixel choice

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-compose --locked clipped_dissolve_is_binary_and_preserves_the_base_shape -- --nocapture` — [crates/compose/src/tests.rs / clipped_dissolve_is_binary_and_preserves_the_base_shape](https://github.com/storytold/photocraft/blob/db546ff4822a011a7915b66af3834f94c05767db/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-compose --locked clipped_dissolve_matches_whole_render_at_offset_regions_and_depths -- --nocapture` — [crates/compose/src/tests.rs / clipped_dissolve_matches_whole_render_at_offset_regions_and_depths](https://github.com/storytold/photocraft/blob/db546ff4822a011a7915b66af3834f94c05767db/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-compose --locked dissolve_adjustment_is_a_per_pixel_choice -- --nocapture` — [crates/compose/src/tests.rs / dissolve_adjustment_is_a_per_pixel_choice](https://github.com/storytold/photocraft/blob/db546ff4822a011a7915b66af3834f94c05767db/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-compose --locked clipped_dissolve_fill_is_a_per_pixel_choice -- --nocapture` — [crates/compose/src/tests.rs / clipped_dissolve_fill_is_a_per_pixel_choice](https://github.com/storytold/photocraft/blob/db546ff4822a011a7915b66af3834f94c05767db/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-compose --locked color_overlay_keeps_dissolve_coverage -- --nocapture` — [crates/compose/src/tests.rs / color_overlay_keeps_dissolve_coverage](https://github.com/storytold/photocraft/blob/db546ff4822a011a7915b66af3834f94c05767db/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-gpu --locked groups_masks_and_clipping -- --nocapture` — [crates/gpu/tests/parity.rs / groups_masks_and_clipping](https://github.com/storytold/photocraft/blob/db546ff4822a011a7915b66af3834f94c05767db/crates/gpu/tests/parity.rs)
  - `cargo test -p photocraft-gpu --locked dissolve_adjustments_and_effects_match_cpu_in_offset_regions -- --nocapture` — [crates/gpu/tests/parity.rs / dissolve_adjustments_and_effects_match_cpu_in_offset_regions](https://github.com/storytold/photocraft/blob/db546ff4822a011a7915b66af3834f94c05767db/crates/gpu/tests/parity.rs)

- Expected behavior: clipped dissolve is binary and preserves the base shape; clipped dissolve matches whole render at offset regions and depths; dissolve adjustment is a per pixel choice; clipped dissolve fill is a per pixel choice; color overlay keeps dissolve coverage; groups masks and clipping; dissolve adjustments and effects match cpu in offset regions. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`clipped_dissolve_is_binary_and_preserves_the_base_shape` setup/action excerpt:

```rust
let mut d = doc_white(40, 12);
    d.layers.push(solid_layer("base", Rect::new(0, 0, 20, 12), [0.0, 0.0, 1.0, 1.0]));
    let mut clip = solid_layer("clip", d.bounds(), [1.0, 0.0, 0.0, 1.0]);
    clip.clipped = true;
    clip.blend = BlendMode::Dissolve;
    clip.opacity = 0.6;
    clip.fill_opacity = 0.5;
    d.layers.push(clip);
    let mut red = 0;
    let mut blue = 0;
    for y in 0..12 {
        for x in 0..20 {
```

Expected assertion context (overlapping windows merged):

```rust
assert!(close4(p, [0.0, 0.0, 1.0, 1.0]), "Dissolve must not interpolate: {p:?} at ({x},{y})");
                blue += 1;
            }
        }
        assert!(close4(px(&d, 30, y), [1.0; 4]), "outside the base the clip is invisible");
    }
    assert!(red > 0 && blue > 0, "Dissolve must expose both source and base: {red}/{blue}");
    assert!((red as f32 / (red + blue) as f32 - 0.3).abs() < 0.1, "coverage should follow 60% opacity × 50% fill: {red}/{blue}");
}
```

`clipped_dissolve_matches_whole_render_at_offset_regions_and_depths` setup/action excerpt:

```rust
for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let mut d = Document::new("coverage", Size::new(32, 18), ColorMode::Rgb, fmt.sample);
        let bounds = Rect::new(-3, -2, 32, 18);
        let mut base = Layer::raster("base", fmt);
        base.surface_mut().unwrap().fill_rect(bounds, &[0.0, 0.0, 1.0, 0.5]);
        d.layers.push(base);
        let mut clip = Layer::raster("clip", fmt);
        clip.surface_mut().unwrap().fill_rect(bounds, &[1.0, 0.0, 0.0, 0.75]);
        clip.clipped = true;
        clip.blend = BlendMode::Dissolve;
        clip.opacity = 0.6;
        clip.fill_opacity = 0.5;
```

Expected assertion context (overlapping windows merged):

```rust
assert!(close4(part.get(x, y), whole.get(x, y)), "{fmt:?}: region mismatch at ({x},{y})");
                }
            }
        }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-4992: Preserve Dissolve coverage when compositing clipped layers

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-compose --locked clipped_dissolve_is_binary_and_preserves_the_base_shape -- --nocapture` — [crates/compose/src/tests.rs / clipped_dissolve_is_binary_and_preserves_the_base_shape](https://github.com/storytold/photocraft/blob/194a8976cd6c14441222b287b9b4f4841a0eb2f9/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-compose --locked clipped_dissolve_matches_whole_render_at_offset_regions_and_depths -- --nocapture` — [crates/compose/src/tests.rs / clipped_dissolve_matches_whole_render_at_offset_regions_and_depths](https://github.com/storytold/photocraft/blob/194a8976cd6c14441222b287b9b4f4841a0eb2f9/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-gpu --locked groups_masks_and_clipping -- --nocapture` — [crates/gpu/tests/parity.rs / groups_masks_and_clipping](https://github.com/storytold/photocraft/blob/194a8976cd6c14441222b287b9b4f4841a0eb2f9/crates/gpu/tests/parity.rs)
  - `cargo test -p photocraft-ui-egui --locked native_rotate_event_reaches_canvas_when_trackpad_rotation_is_enabled -- --nocapture` — [crates/ui-egui/src/rotate_view.rs / native_rotate_event_reaches_canvas_when_trackpad_rotation_is_enabled](https://github.com/storytold/photocraft/blob/194a8976cd6c14441222b287b9b4f4841a0eb2f9/crates/ui-egui/src/rotate_view.rs)
  - `cargo test -p photocraft-ui-egui --locked native_pinch_resets_legacy_gesture_continuation -- --nocapture` — [crates/ui-egui/src/wheel_nav.rs / native_pinch_resets_legacy_gesture_continuation](https://github.com/storytold/photocraft/blob/194a8976cd6c14441222b287b9b4f4841a0eb2f9/crates/ui-egui/src/wheel_nav.rs)

- Expected behavior: clipped dissolve is binary and preserves the base shape; clipped dissolve matches whole render at offset regions and depths; groups masks and clipping; native rotate event reaches canvas when trackpad rotation is enabled; native pinch resets legacy gesture continuation. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`clipped_dissolve_is_binary_and_preserves_the_base_shape` setup/action excerpt:

```rust
let mut d = doc_white(40, 12);
    d.layers.push(solid_layer("base", Rect::new(0, 0, 20, 12), [0.0, 0.0, 1.0, 1.0]));
    let mut clip = solid_layer("clip", d.bounds(), [1.0, 0.0, 0.0, 1.0]);
    clip.clipped = true;
    clip.blend = BlendMode::Dissolve;
    clip.opacity = 0.6;
    clip.fill_opacity = 0.5;
    d.layers.push(clip);
    let mut red = 0;
    let mut blue = 0;
    for y in 0..12 {
        for x in 0..20 {
```

Expected assertion context (overlapping windows merged):

```rust
assert!(close4(p, [0.0, 0.0, 1.0, 1.0]), "Dissolve must not interpolate: {p:?} at ({x},{y})");
                blue += 1;
            }
        }
        assert!(close4(px(&d, 30, y), [1.0; 4]), "outside the base the clip is invisible");
    }
    assert!(red > 0 && blue > 0, "Dissolve must expose both source and base: {red}/{blue}");
    assert!((red as f32 / (red + blue) as f32 - 0.3).abs() < 0.1, "coverage should follow 60% opacity × 50% fill: {red}/{blue}");
}
```

`clipped_dissolve_matches_whole_render_at_offset_regions_and_depths` setup/action excerpt:

```rust
for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let mut d = Document::new("coverage", Size::new(32, 18), ColorMode::Rgb, fmt.sample);
        let bounds = Rect::new(-3, -2, 32, 18);
        let mut base = Layer::raster("base", fmt);
        base.surface_mut().unwrap().fill_rect(bounds, &[0.0, 0.0, 1.0, 0.5]);
        d.layers.push(base);
        let mut clip = Layer::raster("clip", fmt);
        clip.surface_mut().unwrap().fill_rect(bounds, &[1.0, 0.0, 0.0, 0.75]);
        clip.clipped = true;
        clip.blend = BlendMode::Dissolve;
        clip.opacity = 0.6;
        clip.fill_opacity = 0.5;
```

Expected assertion context (overlapping windows merged):

```rust
assert!(close4(part.get(x, y), whole.get(x, y)), "{fmt:?}: region mismatch at ({x},{y})");
                }
            }
        }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-7903: Filter Gallery › Cutout 4/4: trace the image into flat paper shapes

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-algo --locked rough_pastels_and_angled_strokes_respond_to_every_slider -- --nocapture` — [crates/algo/src/artistic_fx.rs / rough_pastels_and_angled_strokes_respond_to_every_slider](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/artistic_fx.rs)
  - `cargo test -p photocraft-algo --locked cutout_makes_flat_shapes_with_clean_edges -- --nocapture` — [crates/algo/src/artistic_fx.rs / cutout_makes_flat_shapes_with_clean_edges](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/artistic_fx.rs)
  - `cargo test -p photocraft-algo --locked cutout_keeps_the_tone_of_surviving_islands_at_low_fidelity -- --nocapture` — [crates/algo/src/artistic_fx.rs / cutout_keeps_the_tone_of_surviving_islands_at_low_fidelity](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/artistic_fx.rs)
  - `cargo test -p photocraft-algo --locked a_stack_around_a_cutout_is_tile_independent -- --nocapture` — [crates/algo/src/artistic_fx.rs / a_stack_around_a_cutout_is_tile_independent](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/artistic_fx.rs)
  - `cargo test -p photocraft-algo --locked oklab_round_trips_and_marks_the_gamut -- --nocapture` — [crates/algo/src/cutout/colour.rs / oklab_round_trips_and_marks_the_gamut](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked out_of_gamut_colours_lose_strength_not_hue -- --nocapture` — [crates/algo/src/cutout/colour.rs / out_of_gamut_colours_lose_strength_not_hue](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked smoothing_flattens_noise_and_keeps_edges -- --nocapture` — [crates/algo/src/cutout/colour.rs / smoothing_flattens_noise_and_keeps_edges](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked the_palette_finds_the_pictures_own_colours -- --nocapture` — [crates/algo/src/cutout/colour.rs / the_palette_finds_the_pictures_own_colours](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked a_tone_of_two_different_hues_splits_in_two -- --nocapture` — [crates/algo/src/cutout/colour.rs / a_tone_of_two_different_hues_splits_in_two](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked an_empty_image_still_has_a_palette -- --nocapture` — [crates/algo/src/cutout/colour.rs / an_empty_image_still_has_a_palette](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked douglas_peucker_keeps_only_points_off_the_line -- --nocapture` — [crates/algo/src/cutout/curves.rs / douglas_peucker_keeps_only_points_off_the_line](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/curves.rs)
  - `cargo test -p photocraft-algo --locked lines_cross_where_they_should -- --nocapture` — [crates/algo/src/cutout/curves.rs / lines_cross_where_they_should](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/curves.rs)
  - `cargo test -p photocraft-algo --locked rounding_off_keeps_an_open_chains_ends -- --nocapture` — [crates/algo/src/cutout/curves.rs / rounding_off_keeps_an_open_chains_ends](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/curves.rs)
  - `cargo test -p photocraft-algo --locked a_square_keeps_its_four_corners_and_straight_sides -- --nocapture` — [crates/algo/src/cutout/curves.rs / a_square_keeps_its_four_corners_and_straight_sides](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/curves.rs)
  - `cargo test -p photocraft-algo --locked a_rounded_corner_is_squared_back -- --nocapture` — [crates/algo/src/cutout/curves.rs / a_rounded_corner_is_squared_back](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/curves.rs)
  - `cargo test -p photocraft-algo --locked an_open_chain_ends_where_it_started_at_any_fidelity -- --nocapture` — [crates/algo/src/cutout/curves.rs / an_open_chain_ends_where_it_started_at_any_fidelity](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/curves.rs)
  - `cargo test -p photocraft-algo --locked curves_pass_through_their_vertices -- --nocapture` — [crates/algo/src/cutout/curves.rs / curves_pass_through_their_vertices](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/curves.rs)
  - `cargo test -p photocraft-algo --locked fitting_keeps_small_islands_and_their_winding -- --nocapture` — [crates/algo/src/cutout/curves.rs / fitting_keeps_small_islands_and_their_winding](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/curves.rs)
  - `cargo test -p photocraft-algo --locked bounds_cover_every_point -- --nocapture` — [crates/algo/src/cutout/curves.rs / bounds_cover_every_point](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/curves.rs)
  - `cargo test -p photocraft-algo --locked the_edge_between_two_regions_is_one_chain_between_junctions -- --nocapture` — [crates/algo/src/cutout/edges.rs / the_edge_between_two_regions_is_one_chain_between_junctions](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/edges.rs)
  - `cargo test -p photocraft-algo --locked an_island_is_one_closed_chain -- --nocapture` — [crates/algo/src/cutout/edges.rs / an_island_is_one_closed_chain](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/edges.rs)
  - `cargo test -p photocraft-algo --locked an_anti_aliased_edge_is_placed_inside_its_pixel -- --nocapture` — [crates/algo/src/cutout/edges.rs / an_anti_aliased_edge_is_placed_inside_its_pixel](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/edges.rs)
  - `cargo test -p photocraft-algo --locked a_square_covers_its_area_exactly -- --nocapture` — [crates/algo/src/cutout/fill.rs / a_square_covers_its_area_exactly](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/fill.rs)
  - `cargo test -p photocraft-algo --locked shapes_cover_every_pixel_once_with_no_seams -- --nocapture` — [crates/algo/src/cutout/mod.rs / shapes_cover_every_pixel_once_with_no_seams](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/mod.rs)
  - `cargo test -p photocraft-algo --locked edge_fidelity_changes_the_outlines -- --nocapture` — [crates/algo/src/cutout/mod.rs / edge_fidelity_changes_the_outlines](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/mod.rs)
  - `cargo test -p photocraft-algo --locked empty_images_stop_before_the_transpose_and_row_passes -- --nocapture` — [crates/algo/src/cutout/mod.rs / empty_images_stop_before_the_transpose_and_row_passes](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/mod.rs)
  - `cargo test -p photocraft-algo --locked filling_does_not_depend_on_the_tiling -- --nocapture` — [crates/algo/src/cutout/mod.rs / filling_does_not_depend_on_the_tiling](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/mod.rs)
  - `cargo test -p photocraft-algo --locked noise_merges_into_neighbours_and_noise_apart_colours_are_one -- --nocapture` — [crates/algo/src/cutout/mod.rs / noise_merges_into_neighbours_and_noise_apart_colours_are_one](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/mod.rs)
  - `cargo test -p photocraft-algo --locked components_are_four_connected_and_numbered_in_raster_order -- --nocapture` — [crates/algo/src/cutout/regions.rs / components_are_four_connected_and_numbered_in_raster_order](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/regions.rs)
  - `cargo test -p photocraft-algo --locked small_regions_merge_into_the_neighbour_they_share_most_border_with -- --nocapture` — [crates/algo/src/cutout/regions.rs / small_regions_merge_into_the_neighbour_they_share_most_border_with](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/regions.rs)
  - `cargo test -p photocraft-algo --locked slivers_merge_though_they_are_large -- --nocapture` — [crates/algo/src/cutout/regions.rs / slivers_merge_though_they_are_large](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/regions.rs)
  - `cargo test -p photocraft-algo --locked paper_is_stretched_towards_black_and_white_and_keeps_its_hue -- --nocapture` — [crates/algo/src/cutout/regions.rs / paper_is_stretched_towards_black_and_white_and_keeps_its_hue](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/regions.rs)
  - `cargo test -p photocraft-algo --locked local_colour_drifts_within_a_shape_but_never_crosses_a_cut -- --nocapture` — [crates/algo/src/cutout/regions.rs / local_colour_drifts_within_a_shape_but_never_crosses_a_cut](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/regions.rs)
  - `cargo test -p photocraft-algo --locked transposing_twice_gives_the_image_back -- --nocapture` — [crates/algo/src/cutout/util.rs / transposing_twice_gives_the_image_back](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/util.rs)
  - `cargo test -p photocraft-algo --locked find_returns_the_root_and_shortens_the_path -- --nocapture` — [crates/algo/src/cutout/util.rs / find_returns_the_root_and_shortens_the_path](https://github.com/storytold/photocraft/blob/5c30699087b02edcaa23397a0f7076bf503be048/crates/algo/src/cutout/util.rs)

- Expected behavior: rough pastels and angled strokes respond to every slider; cutout makes flat shapes with clean edges; cutout keeps the tone of surviving islands at low fidelity; a stack around a cutout is tile independent; oklab round trips and marks the gamut; out of gamut colours lose strength not hue; smoothing flattens noise and keeps edges; the palette finds the pictures own colours; a tone of two different hues splits in two; an empty image still has a palette; douglas peucker keeps only points off the line; lines cross where they should; rounding off keeps an open chains ends; a square keeps its four corners and straight sides; a rounded corner is squared back; an open chain ends where it started at any fidelity; curves pass through their vertices; fitting keeps small islands and their winding; bounds cover every point; the edge between two regions is one chain between junctions; an island is one closed chain; an anti aliased edge is placed inside its pixel; a square covers its area exactly; shapes cover every pixel once with no seams; edge fidelity changes the outlines; empty images stop before the transpose and row passes; filling does not depend on the tiling; noise merges into neighbours and noise apart colours are one; components are four connected and numbered in raster order; small regions merge into the neighbour they share most border with; slivers merge though they are large; paper is stretched towards black and white and keeps its hue; local colour drifts within a shape but never crosses a cut; transposing twice gives the image back; find returns the root and shortens the path. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`rough_pastels_and_angled_strokes_respond_to_every_slider` setup/action excerpt:

```rust
// #879: these smeared or drowned the image in texture, and some sliders did nothing.
        use crate::GalleryFilter as G;
        let b = Rect::new(0, 0, 56, 40);
        let s = pattern(SampleType::F32);
        let run = |e: &GalleryEffect, tile: i32| {
            let fp = FilterParams::FilterGallery { effects: vec![e.clone()] };
            apply_tiled(&s, &fp, b, b, None, tile, Some(b)).read_region(b)
        };
        for f in [G::RoughPastels, G::AngledStrokes, G::Cutout] {
            let base = run(&GalleryEffect::new(f), 256);
            for prm in f.params() {
                let crate::GalleryParamKind::Range { min, max, .. } = prm.kind else { continue };
```

Expected assertion context (overlapping windows merged):

```rust
assert_ne!(a, base, "{} {} = {v} changed nothing", f.key(), prm.key);
                    let t = run(&e, 16);
                    let worst = a.iter().zip(&t).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
                    assert!(worst < 1e-3, "{} {} = {v} differs across tilings by {worst}", f.key(), prm.key);
                }
            }
        }
```

`cutout_makes_flat_shapes_with_clean_edges` setup/action excerpt:

```rust
// It used to posterize every pixel after a blur: noise survived as speckle, edges were
        // ragged and smooth gradients banded instead of becoming flat paper.
        let noisy: Vec<Vec<f32>> = (0..32).map(|y| (0..64).map(|x| if x < 32 { 0.2 } else { 0.8 } + (hash01(x, y, 0, 7) - 0.5) * 0.16).collect()).collect();
        let out = cutout_on(&noisy, &[]);
        for (x0, x1) in [(2, 28), (36, 62)] {
            let vals: Vec<f32> = (2..30).flat_map(|y| out[y][x0..x1].to_vec()).collect();
            let spread = vals.iter().fold(f32::MIN, |a, &v| a.max(v)) - vals.iter().fold(f32::MAX, |a, &v| a.min(v));
            assert!(spread < 1e-4, "a noisy area should be one flat tone, spread {spread}");
        }
        // The cut is an anti-aliased pixel or two, not a blur.
        for y in 0..32 {
            let (lo, hi) = (out[y][16], out[y][48]);
```

Expected assertion context (overlapping windows merged):

```rust
assert!(spread < 1e-4, "a noisy area should be one flat tone, spread {spread}");
        }
        // The cut is an anti-aliased pixel or two, not a blur.
        for y in 0..32 {
            assert!(soft <= 2, "row {y}: {soft} edge pixels");
        }
        // A ramp becomes `numberOfLevels` flat steps (the palette is the image's own colours, so
        // one may go to the wide flat margins and merge as noise).
            assert!(tones.len() <= levels as usize && tones.len() + 1 >= levels as usize, "levels {levels}: tones {tones:?}");
            assert!(tones.windows(2).all(|t| t[0] < t[1]), "levels {levels}: tones {tones:?}");
        }
        // Edge Simplicity drops specks.
        let speck: Vec<Vec<f32>> = (0..32).map(|y| (0..32).map(|x| if (15..18).contains(&x) && (15..18).contains(&y) { 0.9 } else { 0.2 }).collect()).collect();
        assert!(detailed[16][16] - detailed[2][2] > 0.5, "the speck should survive at simplicity 0");
        assert!((simple[16][16] - simple[2][2]).abs() < 1e-4, "the speck should go at simplicity 10");
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-8039: Compose: Extend Softer outer glows past Size (#7899)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-compose --locked softer_outer_glow_keeps_a_fading_tail_past_its_size -- --nocapture` — [crates/compose/src/effects.rs / softer_outer_glow_keeps_a_fading_tail_past_its_size](https://github.com/storytold/photocraft/blob/5579cf5439856fed93f41a669a3fc9b56d5813cf/crates/compose/src/effects.rs)
  - `cargo test -p photocraft-compose --locked softer_outer_glow_margin_includes_its_tail_support -- --nocapture` — [crates/compose/src/effects.rs / softer_outer_glow_margin_includes_its_tail_support](https://github.com/storytold/photocraft/blob/5579cf5439856fed93f41a669a3fc9b56d5813cf/crates/compose/src/effects.rs)
  - `cargo test -p photocraft-gpu --locked softer_outer_glow_program_keeps_the_tent_and_adds_a_distance_tail -- --nocapture` — [crates/gpu/src/fx.rs / softer_outer_glow_program_keeps_the_tent_and_adds_a_distance_tail](https://github.com/storytold/photocraft/blob/5579cf5439856fed93f41a669a3fc9b56d5813cf/crates/gpu/src/fx.rs)

- Expected behavior: softer outer glow keeps a fading tail past its size; softer outer glow margin includes its tail support; softer outer glow program keeps the tent and adds a distance tail. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`softer_outer_glow_keeps_a_fading_tail_past_its_size` setup/action excerpt:

```rust
let mut shape = Map::new(220, 220, 0.0);
        for y in 0..shape.h {
            for x in 20..100 {
                shape.v[y * shape.w + x] = 1.0;
            }
        }
        let glow = Glow {
            common: photocraft_doc::FxCommon::new(BlendMode::Screen, 1.0),
            paint: FxPaint::Color(photocraft_color::Color::WHITE),
            technique: GlowTechnique::Softer,
            spread: 0.0,
            size: 60.0,
```

Expected assertion context (overlapping windows merged):

```rust
assert!(at(50) > 0.024, "the softer shoulder should stay visible: {}", at(50));
        assert!(at(60) > 1.0 / 255.0, "the glow must retain a visible tail at its Size: {}", at(60));
        assert!(at(62) > 0.5 / 255.0, "the glow must extend beyond its Size: {}", at(62));
        assert!(at(66) < 1e-6, "the added tail should fade to zero: {}", at(66));
    }
```

`softer_outer_glow_margin_includes_its_tail_support` setup/action excerpt:

```rust
let glow = Glow {
            common: photocraft_doc::FxCommon::new(BlendMode::Screen, 1.0),
            paint: FxPaint::Color(photocraft_color::Color::WHITE),
            technique: GlowTechnique::Softer,
            spread: 0.0,
            size: 60.0,
            contour: Contour::Linear,
            anti_alias: false,
            range: 0.5,
            jitter: 0.0,
            noise: 0.0,
            source: GlowSource::Edge,
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(margin(&layer), 68);
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-8191: Filter Gallery › Cutout 1/4: colour and palette stage

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-algo --locked oklab_round_trips_and_marks_the_gamut -- --nocapture` — [crates/algo/src/cutout/colour.rs / oklab_round_trips_and_marks_the_gamut](https://github.com/storytold/photocraft/blob/91e4cec43525ca739c05cff614701fbd3576e389/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked out_of_gamut_colours_lose_strength_not_hue -- --nocapture` — [crates/algo/src/cutout/colour.rs / out_of_gamut_colours_lose_strength_not_hue](https://github.com/storytold/photocraft/blob/91e4cec43525ca739c05cff614701fbd3576e389/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked smoothing_flattens_noise_and_keeps_edges -- --nocapture` — [crates/algo/src/cutout/colour.rs / smoothing_flattens_noise_and_keeps_edges](https://github.com/storytold/photocraft/blob/91e4cec43525ca739c05cff614701fbd3576e389/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked the_palette_finds_the_pictures_own_colours -- --nocapture` — [crates/algo/src/cutout/colour.rs / the_palette_finds_the_pictures_own_colours](https://github.com/storytold/photocraft/blob/91e4cec43525ca739c05cff614701fbd3576e389/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked a_tone_of_two_different_hues_splits_in_two -- --nocapture` — [crates/algo/src/cutout/colour.rs / a_tone_of_two_different_hues_splits_in_two](https://github.com/storytold/photocraft/blob/91e4cec43525ca739c05cff614701fbd3576e389/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked an_empty_image_still_has_a_palette -- --nocapture` — [crates/algo/src/cutout/colour.rs / an_empty_image_still_has_a_palette](https://github.com/storytold/photocraft/blob/91e4cec43525ca739c05cff614701fbd3576e389/crates/algo/src/cutout/colour.rs)
  - `cargo test -p photocraft-algo --locked transposing_twice_gives_the_image_back -- --nocapture` — [crates/algo/src/cutout/util.rs / transposing_twice_gives_the_image_back](https://github.com/storytold/photocraft/blob/91e4cec43525ca739c05cff614701fbd3576e389/crates/algo/src/cutout/util.rs)
  - `cargo test -p photocraft-algo --locked find_returns_the_root_and_shortens_the_path -- --nocapture` — [crates/algo/src/cutout/util.rs / find_returns_the_root_and_shortens_the_path](https://github.com/storytold/photocraft/blob/91e4cec43525ca739c05cff614701fbd3576e389/crates/algo/src/cutout/util.rs)

- Expected behavior: oklab round trips and marks the gamut; out of gamut colours lose strength not hue; smoothing flattens noise and keeps edges; the palette finds the pictures own colours; a tone of two different hues splits in two; an empty image still has a palette; transposing twice gives the image back; find returns the root and shortens the path. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`oklab_round_trips_and_marks_the_gamut` setup/action excerpt:

```rust
let white = oklab([1.0, 1.0, 1.0]);
        assert!((white[0] - 1.0).abs() < 1e-3 && white[1].abs() < 1e-3 && white[2].abs() < 1e-3, "{white:?}");
        assert!(oklab([0.0, 0.0, 0.0])[0].abs() < 1e-6);
        for p in [[0.2, 0.5, 0.9], [0.95, 0.1, 0.3], [0.5, 0.5, 0.5], [0.0, 1.0, 0.0]] {
            let back = srgb(oklab(p)).unwrap_or([-1.0; 3]);
            assert!(dist2(back, p) < 1e-6, "{p:?} came back as {back:?}");
        }
        // Very light and very colourful: no sRGB colour is that.
        assert_eq!(srgb([0.95, 0.3, 0.0]), None);
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!((white[0] - 1.0).abs() < 1e-3 && white[1].abs() < 1e-3 && white[2].abs() < 1e-3, "{white:?}");
        assert!(oklab([0.0, 0.0, 0.0])[0].abs() < 1e-6);
        for p in [[0.2, 0.5, 0.9], [0.95, 0.1, 0.3], [0.5, 0.5, 0.5], [0.0, 1.0, 0.0]] {
            let back = srgb(oklab(p)).unwrap_or([-1.0; 3]);
            assert!(dist2(back, p) < 1e-6, "{p:?} came back as {back:?}");
        }
        // Very light and very colourful: no sRGB colour is that.
        assert_eq!(srgb([0.95, 0.3, 0.0]), None);
    }
```

`out_of_gamut_colours_lose_strength_not_hue` setup/action excerpt:

```rust
let (l, a, b) = (0.95, 0.3, 0.1);
        let c = in_gamut(l, a, b);
        let lab = oklab(c);
        assert!((lab[0] - l).abs() < 0.01, "lightness kept: {lab:?}");
        let chroma = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
        assert!(chroma > 0.01 && chroma < (a * a + b * b).sqrt(), "weaker but still coloured: {chroma}");
        assert!((lab[2] / lab[1] - b / a).abs() < 0.05, "same hue: {lab:?}");
        // In gamut it is just the conversion.
        assert_eq!(in_gamut(0.6, 0.02, -0.03), srgb([0.6, 0.02, -0.03]).unwrap_or_default());
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!((lab[0] - l).abs() < 0.01, "lightness kept: {lab:?}");
        let chroma = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
        assert!(chroma > 0.01 && chroma < (a * a + b * b).sqrt(), "weaker but still coloured: {chroma}");
        assert!((lab[2] / lab[1] - b / a).abs() < 0.05, "same hue: {lab:?}");
        // In gamut it is just the conversion.
        assert_eq!(in_gamut(0.6, 0.02, -0.03), srgb([0.6, 0.02, -0.03]).unwrap_or_default());
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-8192: Filter Gallery › Cutout 2/4: regions stage

- Fixture/setup: Synthetic label maps in cutout/regions.rs.
- Action: run each command separately:

  - `cargo test -p photocraft-algo --locked components_are_four_connected_and_numbered_in_raster_order -- --nocapture`
  - `cargo test -p photocraft-algo --locked small_regions_merge_into_the_neighbour_they_share_most_border_with -- --nocapture`
  - `cargo test -p photocraft-algo --locked slivers_merge_though_they_are_large -- --nocapture`
  - `cargo test -p photocraft-algo --locked local_colour_drifts_within_a_shape_but_never_crosses_a_cut -- --nocapture`

- Expected: Four-connected components have deterministic raster numbering; small regions choose the shared-border neighbour; narrow slivers merge despite area; colour variation never crosses a cut.
- Separate coverage / prerequisites: The earlier Oklab test is supporting coverage, not the regions-stage oracle.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-8193: Filter Gallery › Cutout 3/4: edges and curves stage

- Fixture/setup: Two-region boundaries, islands, antialiased edges, squares and open chains in cutout/edges.rs and curves.rs.
- Action: run each command separately:

  - `cargo test -p photocraft-algo --locked the_edge_between_two_regions_is_one_chain_between_junctions -- --nocapture`
  - `cargo test -p photocraft-algo --locked an_island_is_one_closed_chain -- --nocapture`
  - `cargo test -p photocraft-algo --locked an_anti_aliased_edge_is_placed_inside_its_pixel -- --nocapture`
  - `cargo test -p photocraft-algo --locked a_square_keeps_its_four_corners_and_straight_sides -- --nocapture`
  - `cargo test -p photocraft-algo --locked an_open_chain_ends_where_it_started_at_any_fidelity -- --nocapture`
  - `cargo test -p photocraft-algo --locked fitting_keeps_small_islands_and_their_winding -- --nocapture`
  - `cargo test -p photocraft-algo --locked cutout_makes_flat_shapes_with_clean_edges -- --nocapture`

- Expected: Shared boundaries are single chains; islands close; subpixel edge placement, square corners, open endpoints and winding survive fitting. Complete Cutout yields the asserted clean flat shapes.
- Separate coverage / prerequisites: Run after the complete #8191 → #8192 → #8193 → #7903 chain.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-8903: Box-filter minified patterns in the CPU and GPU samplers (#8831)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-compose --locked minified_pattern_is_filtered_not_aliased -- --nocapture` — [crates/compose/src/pattern.rs / minified_pattern_is_filtered_not_aliased](https://github.com/storytold/photocraft/blob/d387f52f1f2f5681b2c7d1c517aa5b5992c7bc88/crates/compose/src/pattern.rs)
  - `cargo test -p photocraft-gpu --locked pattern_fill_layers -- --nocapture` — [crates/gpu/tests/it/parity.rs / pattern_fill_layers](https://github.com/storytold/photocraft/blob/d387f52f1f2f5681b2c7d1c517aa5b5992c7bc88/crates/gpu/tests/it/parity.rs)

- Expected behavior: minified pattern is filtered not aliased; pattern fill layers. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`minified_pattern_is_filtered_not_aliased` setup/action excerpt:

```rust
// 1-px dark / light stripes (mean 0.5), shown at 25 %: every pixel covers 4 tile pixels.
        // The 0.125 px phase puts each pixel centre on a light tile pixel's centre, so a single
        // bilinear tap would return pure white.
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 8, 8), &[1.0, 1.0, 1.0, 1.0]);
        for x in (0..8).step_by(2) {
            s.fill_rect(Rect::new(x, 0, x + 1, 8), &[0.0, 0.0, 0.0, 1.0]);
        }
        let t = Tile::new(&Pattern::new("stripes", s, 8, 8)).unwrap();
        let place = Placement::new(Rect::EMPTY, false, (0.125, 0.0), 0.25, 0.0);
        let (u, _) = place.map(0.5, 0.5);
        assert_eq!(t.sample(u, 0.5)[0], 1.0, "the single tap lands on a light stripe");
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(t.sample(u, 0.5)[0], 1.0, "the single tap lands on a light stripe");
        for p in render(&t, &place, Rect::new(0, 0, 8, 1)) {
            assert!((p[0] - 0.5).abs() < 0.05, "{p:?}");
        }
        // 100 % stays exact.
        let one = Placement::new(Rect::EMPTY, false, (0.0, 0.0), 1.0, 0.0);
        assert_eq!((row[0][0], row[1][0]), (0.0, 1.0));
    }
```

`pattern_fill_layers` setup/action excerpt:

```rust
let Some(mut g) = gpu() else { return };
    let pat = checker_pattern();
    for (link, scale, angle, masked) in [(true, 1.0, 0.0, false), (false, 0.7, 25.0, true), (true, 1.6, -40.0, false), (true, 0.3, 10.0, false)] {
        let mut d = base_doc(70, 50);
        d.patterns.push(pat.clone());
        let fill = Fill::Pattern { name: pat.name.clone(), id: pat.id.clone(), scale, angle, link, phase: (3.0, -2.0) };
        let mut l = Layer::new("pat", LayerContent::Fill(fill));
        l.opacity = 0.85;
        l.blend = BlendMode::Overlay;
        if masked {
            l.mask = Some(mask(Rect::new(5, 5, 60, 45), 61, 0.0));
        }
```

Expected assertion context (overlapping windows merged):

```rust

```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9472: fix(io): import a Displace smart filter that names its map file without embedding it (#9162)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked displace_psd_filter_with_only_a_map_path_renders_after_replace_contents -- --nocapture` — [crates/engine/tests/it/smart_psd.rs / displace_psd_filter_with_only_a_map_path_renders_after_replace_contents](https://github.com/storytold/photocraft/blob/4bab657cd4fb4a91ced44d3721e63a002ca5068c/crates/engine/tests/it/smart_psd.rs)
  - `cargo test -p photocraft-io --locked a_map_named_only_by_its_path_imports_as_map_path -- --nocapture` — [crates/io/src/displace_map.rs / a_map_named_only_by_its_path_imports_as_map_path](https://github.com/storytold/photocraft/blob/4bab657cd4fb4a91ced44d3721e63a002ca5068c/crates/io/src/displace_map.rs)

- Expected behavior: displace psd filter with only a map path renders after replace contents; a map named only by its path imports as map path. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`displace_psd_filter_with_only_a_map_path_renders_after_replace_contents` setup/action excerpt:

```rust
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value};
    let dir = std::env::temp_dir().join(format!("pcraft-9162-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = |name: &str, size: (u32, u32), c: [f32; 4]| {
        let d = Document::with_background(
            "m",
            photocraft_geom::Size::new(size.0, size.1),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
            photocraft_color::Color::rgba(c[0], c[1], c[2], c[3]),
        );
        let path = dir.join(name);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(out["warnings"], json!([]), "depth {depth}");
        let cache = smart(&s2.active().unwrap().doc).1.cache.unwrap();
        // 10 % of 128 px at full map contrast: the design (x 8..80) moves 12.8 px left.
        let alpha = |x: i32| cache.pixel(x, 30)[3];
        assert!(alpha(2) > 0.99 && alpha(75) < 0.01, "depth {depth}: {} {}", alpha(2), alpha(75));
    }
    std::fs::remove_dir_all(&dir).ok();
}
```

`a_map_named_only_by_its_path_imports_as_map_path` setup/action excerpt:

```rust
let with_file = |file: Value| {
            let mut i = item(Vec::new());
            let mut f = get_desc(&i, "Fltr").unwrap().clone();
            f.items.retain(|(k, _)| !k.is(MAP_KEY));
            set(&mut f, "DspF", file);
            set(&mut i, "Fltr", Value::Descriptor(f));
            i
        };
        let src = with_file(pth("C:\\maps\\repro-map.psd"));
        let f = filter_from_item(&src);
        assert_eq!(f.command, COMMAND);
        assert_eq!(f.params["mapPath"], json!("C:\\maps\\repro-map.psd"));
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(f.command, COMMAND);
        assert_eq!(f.params["mapPath"], json!("C:\\maps\\repro-map.psd"));
        assert!(embedded_map(&f.params).is_none());
        assert_eq!(item_for_filter(&f).unwrap(), src, "written back as it was");
        let mut moved = f.clone();
        moved.params["mapPath"] = json!("/elsewhere/map.png");
        assert!(item_for_filter(&moved).is_err(), "another map path has no place in the template");
        let mut no_path = f.clone();
        no_path.params.as_object_mut().unwrap().remove("mapPath");
        assert!(item_for_filter(&no_path).is_err(), "a path-only template needs its mapPath, or the PSD names a map the render never read");
        // A record whose size field disagrees with its units, or without the final NUL.
        let mut wrong_size = pth("map.psd");
        if let Value::Path(b) = &mut wrong_size {
            assert_eq!(filter_from_item(&src).command, UNSUPPORTED_FILTER);
        }
        // A damaged embedded map is never replaced by a readable path.
        let mut damaged = item(vec![0; 4]);
        assert_eq!(filter_from_item(&damaged).command, UNSUPPORTED_FILTER);
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9509: fix: an edit at full coverage keeps its result over huge 32-bit values

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-compose --locked full_coverage_adjustment_keeps_its_result_over_huge_hdr_values -- --nocapture` — [crates/compose/src/tests.rs / full_coverage_adjustment_keeps_its_result_over_huge_hdr_values](https://github.com/storytold/photocraft/blob/d78d7493d3f0b6e760c88de969ac810e9bc5d910/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-compose --locked reused_render_matches_previous_allocating_stack -- --nocapture` — [crates/compose/src/tests.rs / reused_render_matches_previous_allocating_stack](https://github.com/storytold/photocraft/blob/d78d7493d3f0b6e760c88de969ac810e9bc5d910/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-engine --locked mixing_through_a_selection_matches_sampling_it_per_pixel -- --nocapture` — [crates/engine/src/pixels_tests.rs / mixing_through_a_selection_matches_sampling_it_per_pixel](https://github.com/storytold/photocraft/blob/d78d7493d3f0b6e760c88de969ac810e9bc5d910/crates/engine/src/pixels_tests.rs)
  - `cargo test -p photocraft-engine --locked full_coverage_keeps_the_edit_over_huge_hdr_values -- --nocapture` — [crates/engine/src/pixels_tests.rs / full_coverage_keeps_the_edit_over_huge_hdr_values](https://github.com/storytold/photocraft/blob/d78d7493d3f0b6e760c88de969ac810e9bc5d910/crates/engine/src/pixels_tests.rs)

- Expected behavior: full coverage adjustment keeps its result over huge hdr values; reused render matches previous allocating stack; mixing through a selection matches sampling it per pixel; full coverage keeps the edit over huge hdr values. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`full_coverage_adjustment_keeps_its_result_over_huge_hdr_values` setup/action excerpt:

```rust
for v in [1.0e6_f32, 1.0e8, -1.0e6] {
        let mut d = Document::new("hdr", Size::new(2, 2), ColorMode::Rgb, SampleType::F32);
        let mut base = Layer::raster("b", PixelFormat::RGBA32F);
        base.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 2, 2), &[v, v, v, 1.0]);
        let look = Adjustment::ColorLookup { name: "half".into(), lut: Some(std::sync::Arc::new(vec![0.5; 24])), size: 2, tetrahedral: false, dither: false };
        d.layers = vec![base, Layer::new("look", LayerContent::Adjustment(look))];
        let got = px(&d, 0, 0);
        for c in &got[..3] {
            assert!((c - 0.5).abs() < 1e-6, "{v}: {got:?}");
        }
    }
}
```

Expected assertion context (overlapping windows merged):

```rust
assert!((c - 0.5).abs() < 1e-6, "{v}: {got:?}");
        }
    }
}
```

`reused_render_matches_previous_allocating_stack` setup/action excerpt:

```rust
for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = Document::new("equivalence", Size::new(513, 7), ColorMode::Rgb, depth);
        let rect = doc.bounds();
        for (name, color) in [("bottom", [0.2, 0.4, 0.8, 1.0]), ("top", [1.5, -0.25, 0.6, 0.5])] {
            let mut layer = Layer::raster(name, doc.pixel_format());
            layer.surface_mut().unwrap().fill_rect(rect, &color);
            doc.layers.push(layer);
        }
        let mut levels = Adjustment::identity_levels();
        if let Adjustment::Levels { master, .. } = &mut levels {
            master.gamma = 1.4;
        }
```

Expected assertion context (overlapping windows merged):

```rust

```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9512: fix(compose, gpu): adjustment layers apply Fill inside the special eight blend modes

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-compose --locked adjustment_fill_changes_the_special_blend_modes -- --nocapture` — [crates/compose/src/tests.rs / adjustment_fill_changes_the_special_blend_modes](https://github.com/storytold/photocraft/blob/6c55561128caf5ac917eb3d174e8e7bb4bd48e23/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-gpu --locked special_eight_fill_on_adjustment_layers_matches_cpu -- --nocapture` — [crates/gpu/tests/it/parity.rs / special_eight_fill_on_adjustment_layers_matches_cpu](https://github.com/storytold/photocraft/blob/6c55561128caf5ac917eb3d174e8e7bb4bd48e23/crates/gpu/tests/it/parity.rs)

- Expected behavior: adjustment fill changes the special blend modes; special eight fill on adjustment layers matches cpu. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`adjustment_fill_changes_the_special_blend_modes` setup/action excerpt:

```rust
for (mode, want) in [(BlendMode::ColorDodge, [80, 160, 240]), (BlendMode::LinearBurn, [13, 77, 141])] {
        let mut d = Document::with_background("fill", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut base = Layer::raster("b", PixelFormat::RGBA8);
        base.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 1, 1), &[64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 1.0]);
        let look = Adjustment::ColorLookup { name: "half".into(), lut: Some(std::sync::Arc::new(vec![0.5; 24])), size: 2, tetrahedral: false, dither: false };
        let mut adj = Layer::new("look", LayerContent::Adjustment(look));
        adj.blend = mode;
        adj.fill_opacity = 0.4;
        d.layers.push(base);
        d.layers.push(adj);
        let got = px(&d, 0, 0);
        let got: Vec<i32> = got[..3].iter().map(|v| (v * 255.0).round() as i32).collect();
```

Expected assertion context (overlapping windows merged):

```rust
assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1), "{mode:?}: {got:?} vs Photoshop {want:?}");
    }
}
```

`special_eight_fill_on_adjustment_layers_matches_cpu` setup/action excerpt:

```rust
let Some(mut g) = gpu() else { return };
    let special = [
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::HardMix,
        BlendMode::Difference,
    ];
    for mode in special {
```

Expected assertion context (overlapping windows merged):

```rust

```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9584: fix(compose, gpu): Blend If hard points match Photoshop at 16 bits and for Gray

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-compose --locked blend_if_16_bit_cutoffs_follow_photoshops_steps -- --nocapture` — [crates/compose/src/tests.rs / blend_if_16_bit_cutoffs_follow_photoshops_steps](https://github.com/storytold/photocraft/blob/fc0f7b1132f799b5bba7be642c9047499d95a887/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-compose --locked blend_if_gray_follows_photoshops_weights_and_rounding -- --nocapture` — [crates/compose/src/tests.rs / blend_if_gray_follows_photoshops_weights_and_rounding](https://github.com/storytold/photocraft/blob/fc0f7b1132f799b5bba7be642c9047499d95a887/crates/compose/src/tests.rs)

- Expected behavior: blend if 16 bit cutoffs follow photoshops steps; blend if gray follows photoshops weights and rounding. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`blend_if_16_bit_cutoffs_follow_photoshops_steps` setup/action excerpt:

```rust
let shown = |v16: u16, black: u8, white: u8| {
        let v = f32::from(v16) / 65535.0;
        let mut d = Document::new("t", Size::new(1, 1), ColorMode::Rgb, SampleType::U16);
        let layer16 = |name: &str, c: [f32; 4]| {
            let mut l = Layer::raster(name, PixelFormat::RGBA16);
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 1, 1), &c);
            l
        };
        d.layers.push(layer16("under", [v, v, v, 1.0]));
        let mut l = layer16("top", [1.0, 1.0, 1.0, 1.0]);
        l.blend_if.set(3, [FULL, range([black, black], [white, white])]);
        d.layers.push(l);
```

Expected assertion context (overlapping windows merged):

```rust
assert!(shown(at(step), c, 255), "black {c}: step {step} shows");
        assert!(!shown(at(step - 1), c, 255), "black {c}: step {} hides", step - 1);
        assert!(shown(at(step), 0, c), "white {c}: step {step} shows");
        assert!(!shown(at(step + 1), 0, c), "white {c}: step {} hides", step + 1);
    }
    assert!(shown(32895, 128, 255) && !shown(32894, 128, 255), "the reporter's RGB16 boundary");
}
```

`blend_if_gray_follows_photoshops_weights_and_rounding` setup/action excerpt:

```rust
let shown = |rgb: [u8; 3], black: u8, white: u8| {
        let mut d = Document::new("t", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let c = rgb.map(|v| f32::from(v) / 255.0);
        d.layers.push(solid_layer("under", Rect::new(0, 0, 1, 1), [c[0], c[1], c[2], 1.0]));
        let mut l = solid_layer("top", Rect::new(0, 0, 1, 1), [1.0, 1.0, 1.0, 1.0]);
        l.blend_if.set(0, [FULL, range([black, black], [white, white])]);
        d.layers.push(l);
        px(&d, 0, 0)[0] > 0.999 && px(&d, 0, 0)[2] > 0.999
    };
    // (colour hidden by a black point at `cut`, the next colour shown), single channels.
    for (cut, below, at) in [
        (20, [65, 0, 0], [66, 0, 0]),
```

Expected assertion context (overlapping windows merged):

```rust
assert!(!shown(below, cut, 255), "{below:?} hidden by black {cut}");
        assert!(shown(at, cut, 255), "{at:?} shown by black {cut}");
    }
    // The reporter's mixed colours, and the white point's half-down rounding.
    assert!(shown([123, 126, 151], 128, 255), "127.85 is 128");
    assert!(shown([120, 129, 149], 0, 128), "128.5 is 128");
    assert!(!shown([121, 129, 149], 0, 128), "128.8 is 129");
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9589: fix(compose, gpu): an adjustment layer's This Layer range judges its result before Opacity

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-compose --locked adjustment_blend_if_judges_the_result_before_opacity -- --nocapture` — [crates/compose/src/tests.rs / adjustment_blend_if_judges_the_result_before_opacity](https://github.com/storytold/photocraft/blob/da157945077190e550b53a5e14b0536f62761292/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-gpu --locked blend_if -- --nocapture` — [crates/gpu/tests/it/parity.rs / blend_if](https://github.com/storytold/photocraft/blob/da157945077190e550b53a5e14b0536f62761292/crates/gpu/tests/it/parity.rs)

- Expected behavior: adjustment blend if judges the result before opacity; blend if. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`adjustment_blend_if_judges_the_result_before_opacity` setup/action excerpt:

```rust
for (black, white, want) in [(150u8, 255u8, [115, 128, 140]), (0, 150, [64, 128, 192])] {
        let mut d = Document::with_background("bi", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        d.layers.push(solid_layer("base", Rect::new(0, 0, 1, 1), [64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 1.0]));
        let mut inv = Layer::new("invert", LayerContent::Adjustment(Adjustment::Invert));
        inv.opacity = 0.4;
        inv.blend_if.set(1, [range([black, black], [white, white]), FULL]);
        d.layers.push(inv);
        let got: Vec<i32> = px(&d, 0, 0)[..3].iter().map(|v| (v * 255.0).round() as i32).collect();
        assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1), "black {black} white {white}: {got:?} vs Photoshop {want:?}");
    }
}
```

Expected assertion context (overlapping windows merged):

```rust
assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1), "black {black} white {white}: {got:?} vs Photoshop {want:?}");
    }
}
```

`blend_if` setup/action excerpt:

```rust
use photocraft_doc::BlendRange;
    let Some(mut g) = gpu() else { return };
    let r = |b0: u8, b1: u8, w0: u8, w1: u8| BlendRange { black: [b0, b1], white: [w0, w1] };
    // (name, entry, [this layer, underlying], has a hard point)
    let cases: [(&str, usize, [BlendRange; 2], bool); 7] = [
        ("gray this hard", 0, [r(90, 90, 255, 255), BlendRange::FULL], true),
        ("gray under split", 0, [BlendRange::FULL, r(30, 140, 150, 230)], false),
        ("red both", 1, [r(0, 60, 200, 200), r(20, 20, 180, 250)], true),
        ("red both split", 1, [r(0, 60, 190, 230), r(10, 50, 180, 250)], false),
        ("green white split", 2, [r(0, 0, 100, 220), BlendRange::FULL], false),
        ("blue under hard", 3, [BlendRange::FULL, r(128, 128, 255, 255)], true),
        ("everything hidden", 0, [r(255, 255, 0, 0), BlendRange::FULL], true),
```

Expected assertion context (overlapping windows merged):

```rust
assert!(hits.prefix_hits > 0, "{depth:?} {name}: the edit above reuses the lower stack");

            // Masked, with channel restrictions.
            let mut m = l.clone();
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9595: fix(compose, gpu): a Pass Through group with Blend If renders isolated

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-compose --locked pass_through_group_with_blend_if_is_isolated -- --nocapture` — [crates/compose/src/tests.rs / pass_through_group_with_blend_if_is_isolated](https://github.com/storytold/photocraft/blob/75fa71a4f55a098061b12d91508b514565ab8dcf/crates/compose/src/tests.rs)
  - `cargo test -p photocraft-gpu --locked pass_through_group_with_blend_if_is_isolated -- --nocapture` — [crates/gpu/tests/it/parity.rs / pass_through_group_with_blend_if_is_isolated](https://github.com/storytold/photocraft/blob/75fa71a4f55a098061b12d91508b514565ab8dcf/crates/gpu/tests/it/parity.rs)

- Expected behavior: pass through group with blend if is isolated; pass through group with blend if is isolated. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`pass_through_group_with_blend_if_is_isolated` setup/action excerpt:

```rust
for (gated, want) in [(true, [64, 128, 192]), (false, [191, 127, 63])] {
        let mut d = Document::with_background("pt", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        d.layers.push(solid_layer("base", Rect::new(0, 0, 1, 1), [64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 1.0]));
        let mut g = Layer::group("group", vec![Layer::new("invert", LayerContent::Adjustment(Adjustment::Invert))]);
        g.blend = BlendMode::PassThrough;
        if gated {
            g.blend_if.set(1, [FULL, range([0, 0], [110, 110])]);
        }
        d.layers.push(g);
        let got: Vec<i32> = px(&d, 0, 0)[..3].iter().map(|v| (v * 255.0).round() as i32).collect();
        assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1), "Blend If {gated}: {got:?} vs Photoshop {want:?}");
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1), "Blend If {gated}: {got:?} vs Photoshop {want:?}");
    }
}
```

`pass_through_group_with_blend_if_is_isolated` setup/action excerpt:

```rust
use photocraft_doc::BlendRange;
    let Some(mut g) = gpu() else { return };
    for gated in [true, false] {
        let mut d = base_doc(64, 48);
        d.layers[0] = noise_layer("bg", PixelFormat::RGBA8, Rect::from_xywh(0, 0, 64, 48), 5, 1.0);
        let mut grp = Layer::group("group", vec![Layer::new("inv", LayerContent::Adjustment(Adjustment::Invert))]);
        grp.blend = BlendMode::PassThrough;
        if gated {
            grp.blend_if.set(1, [BlendRange::FULL, BlendRange { black: [0, 0], white: [110, 110] }]);
        }
        d.layers.push(grp);
        check(&mut g, &d, &format!("pass-through group, Blend If {gated}"));
```

Expected assertion context (overlapping windows merged):

```rust

```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9651: perf(algo): row-parallel local_sharpen with fast-path protect_detail (#5426)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-algo --locked matches_scalar_sharpen_bit_for_bit_across_layouts_edges_and_hdr -- --nocapture` — [crates/algo/tests/it/retouch_sharpen_exact.rs / matches_scalar_sharpen_bit_for_bit_across_layouts_edges_and_hdr](https://github.com/storytold/photocraft/blob/67666bdf72a58066df868868cef82e5b9cc838de/crates/algo/tests/it/retouch_sharpen_exact.rs)
  - `cargo test -p photocraft-algo --locked empty_channel_layout_remains_empty -- --nocapture` — [crates/algo/tests/it/retouch_sharpen_exact.rs / empty_channel_layout_remains_empty](https://github.com/storytold/photocraft/blob/67666bdf72a58066df868868cef82e5b9cc838de/crates/algo/tests/it/retouch_sharpen_exact.rs)

- Expected behavior: matches scalar sharpen bit for bit across layouts edges and hdr; empty channel layout remains empty. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`matches_scalar_sharpen_bit_for_bit_across_layouts_edges_and_hdr` setup/action excerpt:

```rust
for (w, h) in [(1, 1), (3, 7), (37, 29)] {
        for ch in 1..=5 {
            for alpha in std::iter::once(None).chain((0..ch).map(Some)) {
                let data: Vec<f32> = (0..w * h * ch)
                    .map(|i| if Some(i % ch) == alpha { [0.0, 1e-8, 0.25, 0.75, 1.0][(i / ch) % 5] } else { ((i * 73 + 19) % 113) as f32 / 17.0 - 2.0 })
                    .collect();
                for win in [(0, 0, w, h), (w / 2, h / 2, w, h), (0, 0, 1, 1), (0, 0, 0, h)] {
                    for amount in [0.0, 0.5, 1.0, 2.5] {
                        for protect_detail in [false, true] {
                            let expected = scalar_local_sharpen(&data, w, h, ch, alpha, win, amount, protect_detail);
                            let actual = local_sharpen(&data, w, h, ch, alpha, win, amount, protect_detail);
                            assert_eq!(actual.len(), expected.len());
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(actual.len(), expected.len());
                            for (i, (a, b)) in actual.iter().zip(&expected).enumerate() {
                                assert_eq!(
                                    a.to_bits(),
                                    b.to_bits(),
                                    "{w}x{h} ch={ch} alpha={alpha:?} win={win:?} amount={amount} protect={protect_detail} sample={i}"
```

`empty_channel_layout_remains_empty` setup/action excerpt:

```rust
assert!(local_sharpen(&[], 5, 5, 0, None, (0, 0, 5, 5), 1.0, true).is_empty());
}
```

Expected assertion context (overlapping windows merged):

```rust
assert!(local_sharpen(&[], 5, 5, 0, None, (0, 0, 5, 5), 1.0, true).is_empty());
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9703: Fix adjustment layer name clicks targeting masks

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked a_new_adjustment_or_fill_layer_shows_its_settings_until_the_mask_thumbnail_is_clicked -- --nocapture` — [crates/ui-egui/src/mask_thumbs_ui/tests.rs / a_new_adjustment_or_fill_layer_shows_its_settings_until_the_mask_thumbnail_is_clicked](https://github.com/storytold/photocraft/blob/3605d30c38294c4d970e52f4a02f44f97e8bc1a9/crates/ui-egui/src/mask_thumbs_ui/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked adjustment_names_target_contents_instead_of_masks -- --nocapture` — [crates/ui-egui/src/mask_thumbs_ui/tests.rs / adjustment_names_target_contents_instead_of_masks](https://github.com/storytold/photocraft/blob/3605d30c38294c4d970e52f4a02f44f97e8bc1a9/crates/ui-egui/src/mask_thumbs_ui/tests.rs)

- Expected behavior: a new adjustment or fill layer shows its settings until the mask thumbnail is clicked; adjustment names target contents instead of masks. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`a_new_adjustment_or_fill_layer_shows_its_settings_until_the_mask_thumbnail_is_clicked` setup/action excerpt:

```rust
for (command, settings) in [("layer.newAdjustmentLayer.curves", Some("Reset to defaults")), ("layer.newFillLayer.solidColor", None)] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let mut h = harness(s, 0, 1.0, 290.0);
        let ctx = h.ctx.clone();
        let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "history", "navigator"]}}));
        crate::control::handle(h.state_mut(), &ctx, &req);
        h.state_mut().run(command, json!({})).unwrap();
        h.run_steps(6);
        let id = h.state().session.active().unwrap().active_layer.unwrap().0;
        assert!(h.state().ui.mask_target, "{command}: the new mask is targeted");
```

Expected assertion context (overlapping windows merged):

```rust
assert!(h.state().ui.mask_target, "{command}: the new mask is targeted");
        assert_eq!(crate::canvas::paint_target(h.state()), json!("mask"), "{command}: painting edits the mask");
        assert!(h.query_all_by_label("Density").next().is_none(), "{command}: Properties shows the layer, not its mask");
        if let Some(label) = settings {
            assert!(h.query_all_by_label(label).next().is_some(), "{command}: its settings show");
        }
        // Clicking the row (not a thumbnail) keeps the settings.
        let row = h.get_by_label(&layer(&h, id).name).rect().center();
        assert_eq!(h.state().ui.mask_target, command.starts_with("layer.newFillLayer."));
        assert!(!h.state().ui.vector_mask_target);
        assert!(h.query_all_by_label("Density").next().is_none(), "{command}: a row click keeps the settings");
        // The mask thumbnail switches Properties to the mask.
        let r = mask_rect(&h, id, MaskKind::Pixel);
        click_with(&mut h, r.center(), Modifiers::NONE);
        assert!(h.state().ui.mask_target);
        assert!(h.query_all_by_label("Density").next().is_some(), "{command}: the mask thumbnail shows the mask controls");
        if let Some(label) = settings {
            assert!(h.query_all_by_label(label).next().is_none());
        }
    }
}
```

`adjustment_names_target_contents_instead_of_masks` setup/action excerpt:

```rust
for command in ["layer.newAdjustmentLayer.hueSaturation", "layer.newAdjustmentLayer.curves"] {
        for ppp in [1.0, 2.0] {
            let (mut s, _, other) = session();
            s.execute(command, json!({})).unwrap();
            let id = s.active().unwrap().active_layer.unwrap().0;
            let path = json!({"subpaths": [{"knots": [[20, 10], [120, 10], [120, 90], [20, 90]]}]});
            s.execute("layer.vectorMask.add", json!({"layer": id, "path": path})).unwrap();
            let mut h = harness(s, 0, ppp, 360.0);
            for kind in [MaskKind::Pixel, MaskKind::Vector] {
                let at = mask_rect(&h, id, kind).center();
                click_with(&mut h, at, Modifiers::NONE);
                assert_eq!(h.state().ui.mask_target, kind == MaskKind::Pixel);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(h.state().ui.mask_target, kind == MaskKind::Pixel);
                assert_eq!(h.state().ui.vector_mask_target, kind == MaskKind::Vector);
                let name = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == id).unwrap().name.unwrap().center();
                click_with(&mut h, name, Modifiers::NONE);
                assert!(!h.state().ui.mask_target && !h.state().ui.vector_mask_target, "{command} @{ppp}x after {kind:?}");
                let thumb = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == id).unwrap().thumbnail.unwrap();
                assert!(near(super::brackets(&h.ctx).unwrap(), thumb.expand(3.0)), "the content thumbnail is selected");
                assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, id);
            }
            let name = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == other).unwrap().name.unwrap().center();
            click_with(&mut h, name, Modifiers::NONE);
            assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, id);
            assert!(!h.state().ui.mask_target && !h.state().ui.vector_mask_target, "{command} @{ppp}x from another layer");
            assert_eq!(crate::canvas::paint_target(h.state()), json!("mask"));
            let content = layer(&h, id).content;
            h.state_mut().run("edit.fill", json!({"contents": "black"})).unwrap();
            assert_eq!(layer(&h, id).content, content, "filling must preserve the adjustment or fill content");
            assert_eq!(layer(&h, id).mask.unwrap().surface.sample_channel(50, 50, 0), 0.0, "Fill edits the mask");
            assert!(!h.state().ui.mask_target && !h.state().ui.vector_mask_target, "painting leaves the content thumbnail selected");
        }
    }
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-3142: Read applied Sony RAW metadata and crop ILCE-6000 images

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-raw --locked measured_image_areas_crop_the_borders_without_touching_the_rest -- --nocapture` — [crates/raw/src/cameras.rs / measured_image_areas_crop_the_borders_without_touching_the_rest](https://github.com/storytold/photocraft/blob/e2ed234e654e574109ffcdb9756f8e8d1b8a00b2/crates/raw/src/cameras.rs)
  - `cargo test -p photocraft-raw --locked encrypted_applied_white_balance_and_black_reach_the_developer -- --nocapture` — [crates/raw/src/sony_metadata.rs / encrypted_applied_white_balance_and_black_reach_the_developer](https://github.com/storytold/photocraft/blob/e2ed234e654e574109ffcdb9756f8e8d1b8a00b2/crates/raw/src/sony_metadata.rs)
  - `cargo test -p photocraft-raw --locked plain_levels_win_and_invalid_encrypted_levels_are_ignored -- --nocapture` — [crates/raw/src/sony_metadata.rs / plain_levels_win_and_invalid_encrypted_levels_are_ignored](https://github.com/storytold/photocraft/blob/e2ed234e654e574109ffcdb9756f8e8d1b8a00b2/crates/raw/src/sony_metadata.rs)
  - `cargo test -p photocraft-raw --locked hostile_private_metadata_is_bounded_and_optional -- --nocapture` — [crates/raw/src/sony_metadata.rs / hostile_private_metadata_is_bounded_and_optional](https://github.com/storytold/photocraft/blob/e2ed234e654e574109ffcdb9756f8e8d1b8a00b2/crates/raw/src/sony_metadata.rs)
  - `cargo test -p photocraft-raw --locked measured_keystream_recurrence_and_unencrypted_tail -- --nocapture` — [crates/raw/src/sony_metadata.rs / measured_keystream_recurrence_and_unencrypted_tail](https://github.com/storytold/photocraft/blob/e2ed234e654e574109ffcdb9756f8e8d1b8a00b2/crates/raw/src/sony_metadata.rs)
  - `cargo test -p photocraft-raw --locked decode_matches_the_measured_oracles -- --nocapture` — [crates/raw/tests/raw_corpus.rs / decode_matches_the_measured_oracles](https://github.com/storytold/photocraft/blob/e2ed234e654e574109ffcdb9756f8e8d1b8a00b2/crates/raw/tests/raw_corpus.rs)
  - `cargo test -p photocraft-raw --locked developing_produces_the_crop_in_16bit_rgb -- --nocapture` — [crates/raw/tests/raw_corpus.rs / developing_produces_the_crop_in_16bit_rgb](https://github.com/storytold/photocraft/blob/e2ed234e654e574109ffcdb9756f8e8d1b8a00b2/crates/raw/tests/raw_corpus.rs)

- Expected behavior: measured image areas crop the borders without touching the rest; encrypted applied white balance and black reach the developer; plain levels win and invalid encrypted levels are ignored; hostile private metadata is bounded and optional; measured keystream recurrence and unencrypted tail; decode matches the measured oracles; developing produces the crop in 16bit rgb. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`measured_image_areas_crop_the_borders_without_touching_the_rest` setup/action excerpt:

```rust
let mut s = nef("NIKON D800E", 7424, 4924);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(8, 6, 7360, 4912));
        // Only the image area is measured: levels, colour and tone stay untouched.
        assert!(s.color.calibrations.is_empty() && s.tone_curve.is_empty());
        assert_eq!(s.white, [16383.0; 3]);

        let mut s = sensor("SONY", "ILCE-7", 6048, 4024);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(12, 12, 6000, 4000));

        let mut s = sensor("SONY", "ILCE-6000", 6048, 4024);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(s.crop, Rect::new(8, 6, 7360, 4912));
        // Only the image area is measured: levels, colour and tone stay untouched.
        assert!(s.color.calibrations.is_empty() && s.tone_curve.is_empty());
        assert_eq!(s.white, [16383.0; 3]);

        let mut s = sensor("SONY", "ILCE-7", 6048, 4024);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(12, 12, 6000, 4000));

        let mut s = sensor("SONY", "ILCE-6000", 6048, 4024);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(12, 12, 6000, 4000));
        assert!(s.color.calibrations.is_empty() && s.tone_curve.is_empty());
        // A file's explicit crop wins, including a different in-camera aspect ratio.
        s.crop = Rect::new(12, 324, 6000, 3376);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(12, 324, 6000, 3376));

        let mut s = sensor("SONY", "ILCE-6000", 3024, 2012);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(0, 0, 3024, 2012));

        // Another size or model of the same maker is left alone.
        let mut s = nef("NIKON D800E", 2496, 1646);
        assert_eq!(s.crop, Rect::new(0, 0, 2496, 1646));
        let mut s = nef("NIKON D800", 7424, 4924);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(0, 0, 7424, 4924));
    }
```

`encrypted_applied_white_balance_and_black_reach_the_developer` setup/action excerpt:

```rust
for be in [false, true] {
            let bytes = fixture(be, false, [512, 513, 514, 515], [3072, 1024, 1024, 2048]);
            let s = decode(&bytes, &Limits::default()).unwrap();
            assert_eq!(s.format, RawFormat::Arw);
            assert_eq!(s.camera_wb, Some([3.0, 1.0, 2.0]));
            assert_eq!(s.black.values, vec![512.0, 513.0, 514.0, 515.0]);
            let dev = develop_sensor(&s, &DevelopOptions::default()).unwrap();
            assert_eq!(dev.info.wb_multipliers, [3.0, 1.0, 2.0]);
            assert!(!dev.warnings.iter().any(|w| w.contains("grey world") || w.contains("assumed 0")));
            let explicit = develop_sensor(&s, &DevelopOptions { white_balance: WhiteBalance::Multipliers([3.0, 1.0, 2.0]), ..Default::default() }).unwrap();
            assert_eq!(dev.rgb, explicit.rgb);
        }
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(s.format, RawFormat::Arw);
            assert_eq!(s.camera_wb, Some([3.0, 1.0, 2.0]));
            assert_eq!(s.black.values, vec![512.0, 513.0, 514.0, 515.0]);
            let dev = develop_sensor(&s, &DevelopOptions::default()).unwrap();
            assert_eq!(dev.info.wb_multipliers, [3.0, 1.0, 2.0]);
            assert!(!dev.warnings.iter().any(|w| w.contains("grey world") || w.contains("assumed 0")));
            let explicit = develop_sensor(&s, &DevelopOptions { white_balance: WhiteBalance::Multipliers([3.0, 1.0, 2.0]), ..Default::default() }).unwrap();
            assert_eq!(dev.rgb, explicit.rgb);
        }
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-3387: feat(codecs): add lossless JPEG XL export for 8/16-bit images

- Fixture/setup: Native 7×5 Gray/GrayA/RGB/RGBA U8/U16 fixtures plus 2/513/8193-pixel-wide alpha samples in codecs/tests/jxl_export.rs.
- Action: run each command separately:

  - `cargo test -p photocraft-codecs --locked jpeg_xl_decodes_losslessly_with_independent_decoder -- --nocapture`
  - `cargo test -p photocraft-codecs --locked jpeg_xl_preserves_sixteen_bit_alpha_across_dimension_header_sizes -- --nocapture`
  - `cargo test -p photocraft-codecs --locked jpeg_xl_encoding_is_deterministic_and_converts_float_to_integer -- --nocapture`
  - `cargo test -p photocraft-codecs --locked jpeg_xl_is_export_only_and_detects_both_signatures -- --nocapture`
  - `cargo test -p photocraft-codecs --locked jpeg_xl_rejects_one_pixel_images_without_panicking -- --nocapture`

- Expected: Independent jxl_oxide decoding equals original integer samples exactly, including alpha and GrayA widening. Dimensions/channels and deterministic output are correct; capabilities remain export-only; 1×1 refuses without panic.
- Separate coverage / prerequisites: These native independent-decoder checks cannot be established by wasm or malformed-header rejection alone.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-3464: fix(vector): target selected saved paths and own Pen gestures

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked appended_work_and_saved_components_preserve_existing_geometry_and_undo -- --nocapture` — [crates/engine/tests/pen_path_append.rs / appended_work_and_saved_components_preserve_existing_geometry_and_undo](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/engine/tests/pen_path_append.rs)
  - `cargo test -p photocraft-engine --locked appending_valid_components_cannot_exceed_total_path_budget -- --nocapture` — [crates/engine/tests/pen_path_append.rs / appending_valid_components_cannot_exceed_total_path_budget](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/engine/tests/pen_path_append.rs)
  - `cargo test -p photocraft-engine --locked invalid_path_coordinates_leave_work_and_saved_paths_unchanged -- --nocapture` — [crates/engine/tests/pen_path_append.rs / invalid_path_coordinates_leave_work_and_saved_paths_unchanged](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/engine/tests/pen_path_append.rs)
  - `cargo test -p photocraft-engine --locked large_straight_work_and_saved_paths_append_and_undo_without_shape_coordinate_limits -- --nocapture` — [crates/engine/tests/pen_path_append.rs / large_straight_work_and_saved_paths_append_and_undo_without_shape_coordinate_limits](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/engine/tests/pen_path_append.rs)
  - `cargo test -p photocraft-ui-egui --locked shape_layer_paths_are_edited_and_a_direct_selection_is_cleared -- --nocapture` — [crates/ui-egui/src/anchor_tools_tests.rs / shape_layer_paths_are_edited_and_a_direct_selection_is_cleared](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/anchor_tools_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked add_and_delete_anchor_tools_edit_the_selected_saved_path -- --nocapture` — [crates/ui-egui/src/anchor_tools_tests.rs / add_and_delete_anchor_tools_edit_the_selected_saved_path](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/anchor_tools_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked command_with_freeform_and_curvature_edits_the_selected_saved_path_after_modifier_release -- --nocapture` — [crates/ui-egui/src/direct_select.rs / command_with_freeform_and_curvature_edits_the_selected_saved_path_after_modifier_release](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select.rs)
  - `cargo test -p photocraft-ui-egui --locked command_cannot_edit_an_existing_path_when_the_freeform_trace_failed_to_finish -- --nocapture` — [crates/ui-egui/src/direct_select.rs / command_cannot_edit_an_existing_path_when_the_freeform_trace_failed_to_finish](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select.rs)
  - `cargo test -p photocraft-ui-egui --locked dragging_an_anchor_moves_it_in_one_undoable_step -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / dragging_an_anchor_moves_it_in_one_undoable_step](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked arrow_keys_nudge_the_selected_saved_path -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / arrow_keys_nudge_the_selected_saved_path](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked selected_saved_and_work_paths_win_overlapping_shape_hits -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / selected_saved_and_work_paths_win_overlapping_shape_hits](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked direct_selection_cannot_apply_to_another_document_or_a_reopened_identity -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / direct_selection_cannot_apply_to_another_document_or_a_reopened_identity](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked external_revision_cancels_preview_and_preserves_the_new_path -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / external_revision_cancels_preview_and_preserves_the_new_path](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked layer_path_and_tool_changes_cancel_the_owned_edit -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / layer_path_and_tool_changes_cancel_the_owned_edit](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked closing_a_document_and_changing_marquee_target_discard_old_events -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / closing_a_document_and_changing_marquee_target_discard_old_events](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked serialized_anchor_selection_has_no_runtime_ownership_and_is_not_restored -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / serialized_anchor_selection_has_no_runtime_ownership_and_is_not_restored](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked command_drag_after_failed_pen_commit_preserves_draft_and_existing_work_path -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / command_drag_after_failed_pen_commit_preserves_draft_and_existing_work_path](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked switching_between_selection_and_convert_tools_cancels_an_in_flight_drag -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / switching_between_selection_and_convert_tools_cancels_an_in_flight_drag](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked the_convert_point_tool_edits_selected_saved_paths_without_changing_the_work_path -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / the_convert_point_tool_edits_selected_saved_paths_without_changing_the_work_path](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked changing_the_channel_vector_mask_target_cancels_an_owned_edit -- --nocapture` — [crates/ui-egui/src/direct_select_tests.rs / changing_the_channel_vector_mask_target_cancels_an_owned_edit](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/direct_select_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked esc_ends_a_pen_path_and_keeps_it -- --nocapture` — [crates/ui-egui/src/input_tests.rs / esc_ends_a_pen_path_and_keeps_it](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/input_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked pen_shows_the_close_path_pointer_over_the_first_anchor -- --nocapture` — [crates/ui-egui/src/input_tests.rs / pen_shows_the_close_path_pointer_over_the_first_anchor](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/input_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked real_freeform_freehand_and_alt_straight_segments_append_to_the_owned_saved_path_once -- --nocapture` — [crates/ui-egui/src/lasso_ui/tests.rs / real_freeform_freehand_and_alt_straight_segments_append_to_the_owned_saved_path_once](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/lasso_ui/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked real_freeform_stale_target_source_revision_and_document_cancel_without_finishing_elsewhere -- --nocapture` — [crates/ui-egui/src/lasso_ui/tests.rs / real_freeform_stale_target_source_revision_and_document_cancel_without_finishing_elsewhere](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/lasso_ui/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked another_freeform_trace_cannot_replace_a_failed_owned_draft_or_retry_it_automatically -- --nocapture` — [crates/ui-egui/src/lasso_ui/tests.rs / another_freeform_trace_cannot_replace_a_failed_owned_draft_or_retry_it_automatically](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/lasso_ui/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked freeform_alt_release_in_the_next_press_keeps_the_failed_fit -- --nocapture` — [crates/ui-egui/src/lasso_ui/tests.rs / freeform_alt_release_in_the_next_press_keeps_the_failed_fit](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/lasso_ui/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked clicking_the_vector_mask_targets_it_for_the_path_tools -- --nocapture` — [crates/ui-egui/src/mask_thumbs_ui/tests.rs / clicking_the_vector_mask_targets_it_for_the_path_tools](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/mask_thumbs_ui/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked cmd_enter_while_drawing_finishes_the_path_and_loads_it -- --nocapture` — [crates/ui-egui/src/path_selection_tests.rs / cmd_enter_while_drawing_finishes_the_path_and_loads_it](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/path_selection_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked freeform_and_path_selection_releases_cannot_edit_a_different_document -- --nocapture` — [crates/ui-egui/src/pen_gesture_owner_tests.rs / freeform_and_path_selection_releases_cannot_edit_a_different_document](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_gesture_owner_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked freeform_release_cancels_after_revision_or_target_change -- --nocapture` — [crates/ui-egui/src/pen_gesture_owner_tests.rs / freeform_release_cancels_after_revision_or_target_change](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_gesture_owner_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked freeform_valid_release_is_one_undoable_path_edit -- --nocapture` — [crates/ui-egui/src/pen_gesture_owner_tests.rs / freeform_valid_release_is_one_undoable_path_edit](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_gesture_owner_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked frame_cancellation_consumes_old_events_after_switching_tools -- --nocapture` — [crates/ui-egui/src/pen_gesture_owner_tests.rs / frame_cancellation_consumes_old_events_after_switching_tools](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_gesture_owner_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked selected_work_and_saved_paths_append_pen_and_curvature_components_once -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / selected_work_and_saved_paths_append_pen_and_curvature_components_once](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked deselected_work_path_is_replaced_without_touching_saved_paths -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / deselected_work_path_is_replaced_without_touching_saved_paths](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked captured_path_and_shape_mode_do_not_follow_later_ui_selection -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / captured_path_and_shape_mode_do_not_follow_later_ui_selection](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked captured_pen_paint_preserves_float_colour_and_alpha_after_toolbar_changes -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / captured_pen_paint_preserves_float_colour_and_alpha_after_toolbar_changes](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked stale_drafts_are_cancelled_before_tool_switch_or_pointer_input -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / stale_drafts_are_cancelled_before_tool_switch_or_pointer_input](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked sync_views_and_input_apis_discard_unowned_or_closed_document_drafts -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / sync_views_and_input_apis_discard_unowned_or_closed_document_drafts](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked same_named_saved_paths_do_not_share_selection_or_drafts_across_tabs -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / same_named_saved_paths_do_not_share_selection_or_drafts_across_tabs](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked reopened_same_id_and_revision_do_not_reuse_the_saved_path_selection -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / reopened_same_id_and_revision_do_not_reuse_the_saved_path_selection](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked ordinary_path_edits_undo_and_redo_preserve_the_explicit_saved_path_selection -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / ordinary_path_edits_undo_and_redo_preserve_the_explicit_saved_path_selection](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked failed_pen_and_curvature_commits_keep_the_exact_draft_and_do_not_retry_each_frame -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / failed_pen_and_curvature_commits_keep_the_exact_draft_and_do_not_retry_each_frame](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked ordinary_tool_switch_commits_the_owned_draft_once -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / ordinary_tool_switch_commits_the_owned_draft_once](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked selected_saved_paths_move_and_free_transform_without_changing_work_or_shape -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / selected_saved_paths_move_and_free_transform_without_changing_work_or_shape](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked explicit_work_path_precedes_vector_mask_and_deselection_restores_mask_fallback -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / explicit_work_path_precedes_vector_mask_and_deselection_restores_mask_fallback](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked pen_keeps_its_captured_vector_mask_when_another_path_is_selected -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / pen_keeps_its_captured_vector_mask_when_another_path_is_selected](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked freeform_finish_uses_its_captured_saved_path_and_rejects_foreign_documents -- --nocapture` — [crates/ui-egui/src/pen_path_ownership_tests.rs / freeform_finish_uses_its_captured_saved_path_and_rejects_foreign_documents](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_ownership_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked saved_path_drag_visual_fixture -- --nocapture --ignored` — [crates/ui-egui/src/pen_path_visual_tests.rs / saved_path_drag_visual_fixture](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/pen_path_visual_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked enter_selects_a_stroke_style_and_escape_closes_its_popups_without_committing_pen -- --nocapture` — [crates/ui-egui/src/shape_stroke_ui/shortcut_tests.rs / enter_selects_a_stroke_style_and_escape_closes_its_popups_without_committing_pen](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/shape_stroke_ui/shortcut_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked transparent_and_absent_paints_remain_distinct_and_open_pen_keeps_foreground -- --nocapture` — [crates/ui-egui/src/vector_paint_tests.rs / transparent_and_absent_paints_remain_distinct_and_open_pen_keeps_foreground](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/vector_paint_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked all_shape_tools_and_open_pen_use_the_complete_stroke_defaults -- --nocapture` — [crates/ui-egui/src/vector_ui.rs / all_shape_tools_and_open_pen_use_the_complete_stroke_defaults](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/vector_ui.rs)
  - `cargo test -p photocraft-ui-egui --locked freeform_straight_segments_keep_the_captured_saved_path_and_reject_a_foreign_document -- --nocapture` — [crates/ui-egui/src/vector_ui.rs / freeform_straight_segments_keep_the_captured_saved_path_and_reject_a_foreign_document](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/vector_ui.rs)
  - `cargo test -p photocraft-ui-egui --locked failed_freeform_straight_segment_commit_keeps_its_owned_fitted_draft -- --nocapture` — [crates/ui-egui/src/vector_ui.rs / failed_freeform_straight_segment_commit_keeps_its_owned_fitted_draft](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/vector_ui.rs)
  - `cargo test -p photocraft-ui-egui --locked show_target_path_and_extras_hide_the_path_outlines -- --nocapture` — [crates/ui-egui/src/vector_ui.rs / show_target_path_and_extras_hide_the_path_outlines](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/vector_ui.rs)
  - `cargo test -p photocraft-ui-egui --locked pen_last_anchor_hit_tolerance_tracks_zoom_and_draft_round_trips -- --nocapture` — [crates/ui-egui/src/vector_ui.rs / pen_last_anchor_hit_tolerance_tracks_zoom_and_draft_round_trips](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/ui-egui/src/vector_ui.rs)
  - `cargo test -p photocraft-vector --locked general_paths_keep_large_straight_coordinates_and_shape_limits_stay_put -- --nocapture` — [crates/vector/src/flatten.rs / general_paths_keep_large_straight_coordinates_and_shape_limits_stay_put](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/vector/src/flatten.rs)
  - `cargo test -p photocraft-vector --locked general_paths_reject_coordinates_that_overflow_the_ui_or_segment_arithmetic -- --nocapture` — [crates/vector/src/flatten.rs / general_paths_reject_coordinates_that_overflow_the_ui_or_segment_arithmetic](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/vector/src/flatten.rs)
  - `cargo test -p photocraft-vector --locked general_paths_bound_total_knots_subpaths_and_combined_tessellation -- --nocapture` — [crates/vector/src/flatten.rs / general_paths_bound_total_knots_subpaths_and_combined_tessellation](https://github.com/storytold/photocraft/blob/1eecd00b2dff4ae4c642269cec211cd06ce1c3aa/crates/vector/src/flatten.rs)

- Expected behavior: appended work and saved components preserve existing geometry and undo; appending valid components cannot exceed total path budget; invalid path coordinates leave work and saved paths unchanged; large straight work and saved paths append and undo without shape coordinate limits; shape layer paths are edited and a direct selection is cleared; add and delete anchor tools edit the selected saved path; command with freeform and curvature edits the selected saved path after modifier release; command cannot edit an existing path when the freeform trace failed to finish; dragging an anchor moves it in one undoable step; arrow keys nudge the selected saved path; selected saved and work paths win overlapping shape hits; direct selection cannot apply to another document or a reopened identity; external revision cancels preview and preserves the new path; layer path and tool changes cancel the owned edit; closing a document and changing marquee target discard old events; serialized anchor selection has no runtime ownership and is not restored; command drag after failed pen commit preserves draft and existing work path; switching between selection and convert tools cancels an in flight drag; the convert point tool edits selected saved paths without changing the work path; changing the channel vector mask target cancels an owned edit; esc ends a pen path and keeps it; pen shows the close path pointer over the first anchor; real freeform freehand and alt straight segments append to the owned saved path once; real freeform stale target source revision and document cancel without finishing elsewhere; another freeform trace cannot replace a failed owned draft or retry it automatically; freeform alt release in the next press keeps the failed fit; clicking the vector mask targets it for the path tools; cmd enter while drawing finishes the path and loads it; freeform and path selection releases cannot edit a different document; freeform release cancels after revision or target change; freeform valid release is one undoable path edit; frame cancellation consumes old events after switching tools; selected work and saved paths append pen and curvature components once; deselected work path is replaced without touching saved paths; captured path and shape mode do not follow later ui selection; captured pen paint preserves float colour and alpha after toolbar changes; stale drafts are cancelled before tool switch or pointer input; sync views and input apis discard unowned or closed document drafts; same named saved paths do not share selection or drafts across tabs; reopened same id and revision do not reuse the saved path selection; ordinary path edits undo and redo preserve the explicit saved path selection; failed pen and curvature commits keep the exact draft and do not retry each frame; ordinary tool switch commits the owned draft once; selected saved paths move and free transform without changing work or shape; explicit work path precedes vector mask and deselection restores mask fallback; pen keeps its captured vector mask when another path is selected; freeform finish uses its captured saved path and rejects foreign documents; saved path drag visual fixture; enter selects a stroke style and escape closes its popups without committing pen; transparent and absent paints remain distinct and open pen keeps foreground; all shape tools and open pen use the complete stroke defaults; freeform straight segments keep the captured saved path and reject a foreign document; failed freeform straight segment commit keeps its owned fitted draft; show target path and extras hide the path outlines; pen last anchor hit tolerance tracks zoom and draft round trips; general paths keep large straight coordinates and shape limits stay put; general paths reject coordinates that overflow the ui or segment arithmetic; general paths bound total knots subpaths and combined tessellation. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`appended_work_and_saved_components_preserve_existing_geometry_and_undo` setup/action excerpt:

```rust
for name in ["work", "Saved drawing"] {
        let mut session = session();
        let first = json!({"subpaths": [{"closed": false, "knots": [[20, 20], [80, 20]]}]});
        let second = json!({"subpaths": [{"closed": false, "knots": [[30, 60], [90, 60]]}]});
        session.execute("path.set", json!({"name": name, "path": first})).unwrap();
        let before = session.execute("path.info", json!({"name": name})).unwrap();
        session.execute("path.set", json!({"name": name, "path": second, "op": "combine"})).unwrap();
        let after = session.execute("path.info", json!({"name": name})).unwrap();
        assert_eq!(after["path"]["subpaths"].as_array().unwrap().len(), 2);
        assert_eq!(after["path"]["subpaths"][0], before["path"]["subpaths"][0]);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.execute("path.info", json!({"name": name})).unwrap(), before);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(after["path"]["subpaths"].as_array().unwrap().len(), 2);
        assert_eq!(after["path"]["subpaths"][0], before["path"]["subpaths"][0]);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.execute("path.info", json!({"name": name})).unwrap(), before);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.execute("path.info", json!({"name": name})).unwrap(), after);
    }
}
```

`appending_valid_components_cannot_exceed_total_path_budget` setup/action excerpt:

```rust
let mut session = session();
    let subpath = json!({"closed": false, "knots": []});
    session.execute("path.set", json!({"path": {"subpaths": vec![subpath.clone(); 2_049]}})).unwrap();
    let before = session.active().unwrap().doc.work_path.clone();
    let history = session.active().unwrap().history.entries().len();
    let error = session.execute("path.set", json!({"path": {"subpaths": vec![subpath; 2_048]}, "op": "combine"})).unwrap_err();
    assert!(error.to_string().contains("4096"));
    assert_eq!(session.active().unwrap().doc.work_path, before);
    assert_eq!(session.active().unwrap().history.entries().len(), history);
}
```

Expected assertion context (overlapping windows merged):

```rust
assert!(error.to_string().contains("4096"));
    assert_eq!(session.active().unwrap().doc.work_path, before);
    assert_eq!(session.active().unwrap().history.entries().len(), history);
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-3519: feat(export): optionally trim transparent margins from layer files

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked layers_to_files_can_trim_transparent_margins_without_changing_the_document -- --nocapture` — [crates/engine/src/file_cmds/tests.rs / layers_to_files_can_trim_transparent_margins_without_changing_the_document](https://github.com/storytold/photocraft/blob/807de6c043a512cb947d86b944c758b3db1277c7/crates/engine/src/file_cmds/tests.rs)
  - `cargo test -p photocraft-engine --locked streamed_layer_export_respects_trim -- --nocapture` — [crates/engine/src/file_cmds/tests.rs / streamed_layer_export_respects_trim](https://github.com/storytold/photocraft/blob/807de6c043a512cb947d86b944c758b3db1277c7/crates/engine/src/file_cmds/tests.rs)
  - `cargo test -p photocraft-engine --locked trimming_an_empty_layer_keeps_a_valid_nonzero_canvas -- --nocapture` — [crates/engine/src/file_cmds/tests.rs / trimming_an_empty_layer_keeps_a_valid_nonzero_canvas](https://github.com/storytold/photocraft/blob/807de6c043a512cb947d86b944c758b3db1277c7/crates/engine/src/file_cmds/tests.rs)

- Expected behavior: layers to files can trim transparent margins without changing the document; streamed layer export respects trim; trimming an empty layer keeps a valid nonzero canvas. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`layers_to_files_can_trim_transparent_margins_without_changing_the_document` setup/action excerpt:

```rust
let dir = tmp("layers-trim");
    for depth in [8, 16, 32] {
        let mut s = session(12, 11, depth);
        s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
        s.execute("select.rect", json!({"x": 3, "y": 2, "width": 4, "height": 5})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();

        let before = doc(&s).clone();
        let regular = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": format!("full-{depth}"), "format": "png"})).unwrap();
        let full = std::fs::read(regular["files"][0].as_str().unwrap()).unwrap();
        let full = photocraft_io::import("full.png", &full).unwrap().document;
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!((full.size.width, full.size.height), (12, 11), "default is unchanged");

        let trimmed = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": format!("trim-{depth}"), "format": "png", "trim": true})).unwrap();
        let image = std::fs::read(trimmed["files"][0].as_str().unwrap()).unwrap();
        assert_eq!((cropped.size.width, cropped.size.height), (4, 5), "depth {depth}");
        let px = photocraft_compose::render(&cropped, Rect::new(0, 0, 1, 1)).px[0];
        assert!(px[0] > 0.95 && px[1] < 0.05 && px[2] < 0.05, "depth {depth}: {px:?}");
        assert_eq!(*doc(&s), before, "export must not crop the original document");
    }
}
```

`streamed_layer_export_respects_trim` setup/action excerpt:

```rust
let mut s = session(12, 11, 8);
    s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
    s.execute("select.rect", json!({"x": 3, "y": 2, "width": 4, "height": 5})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let mut encoded = Vec::new();
    let names = super::export_layers_with(&s, &json!({"trim": true, "format": "png"}), |name, bytes| {
        encoded.push((name.to_string(), bytes.to_vec()));
        Ok(())
    })
    .unwrap();
    assert_eq!(names.len(), encoded.len());
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(names.len(), encoded.len());
    let cropped = photocraft_io::import(&encoded[0].0, &encoded[0].1).unwrap().document;
    assert_eq!((cropped.size.width, cropped.size.height), (4, 5));
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-6383: Windows releases ship a symbols zip: the release build's PDBs for profilers (#688)

- Fixture/setup: Portable/executable artifact and symbols ZIP from the SAME immutable final-SHA build run, per claimed architecture.
- Action: run each command separately:

  - Perform the explicit artifact inspection below; mark BLOCKED if fixtures are absent.

- Expected: ZIP has photocraft.pdb, photocraft_cli.pdb and symbols.txt. Each PDB GUID/age matches its same-run executable; debugger/profiler resolves a PhotoCraft function. Portable/MSI launches without optional symbols.
- Separate coverage / prerequisites: Inspect PE CodeView/PDB identity, record run/job/artifact IDs and hashes. Same SHA rebuilt is not the same PDB identity. Source lines are not required because release omits line tables. Missing packaging artifacts/identity evidence is BLOCKED; do not publish a release just for fixtures.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-8892: ui: larger Align icons and overflow button; Save for Web keeps its width on an invalid Matte

- Fixture/setup: Studio Dark and Studio Light, UI scale 100% and 200%, 1280x800 then 900x600 logical-point window; synthetic document with at least two selected layers, Move tool, overflowing document/panel tabs, and Save for Web. Record physical pixel dimensions and pixels-per-point.
- Action: Inspect Move alignment controls and Align and distribute ellipsis. Force tab overflow and open its chevron menu. Open Save for Web, record dialog width, enter an invalid Matte value such as not-a-colour, and record width/error placement; correct the value.
- Expected: After the combined #8892/#9004 ports, Move icons and ellipsis use 20-point artwork in 28-point hit targets (40/56 physical pixels at 2x). Tab overflow uses a 16-point chevrons-right icon in its 18-point slot (32/36 pixels at 2x), with reachable menu items. Controls remain legible/unclipped. Invalid Matte displays its error without expanding Save for Web width; correction clears it.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester:

## PR-8897: fix(io): read linked smart objects for PSD export as regular files only

- Fixture/setup: Unix test-owned FIFO linked by synthetic 8×6 document; bounded-reader synthetic fixtures.
- Action: run each command separately:

  - `cargo test -p photocraft-io --locked a_linked_fifo_is_not_read -- --nocapture`
  - `cargo test -p photocraft-format --locked reads_stop_at_the_total_limit -- --nocapture`
  - `cargo test -p photocraft-format --locked an_exact_hint_is_not_over_allocated -- --nocapture`

- Expected: FIFO export completes within 30 seconds without a writer, exports cached pixels and warns of fallback/not-regular-file. Exact-limit read succeeds, over-limit/never-ending reads fail FileTooLarge; size hints do not overallocate.
- Separate coverage / prerequisites: Use only owned temporary FIFO, never a real device. Non-Unix exclusion is BLOCKED for FIFO qualification, not PASSED.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-8986: Path tools: edit the saved path selected in the Paths panel (#8478)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked direct_selection_drags_an_anchor_of_the_selected_saved_path -- --nocapture` — [crates/ui-egui/src/saved_path_edit_tests.rs / direct_selection_drags_an_anchor_of_the_selected_saved_path](https://github.com/storytold/photocraft/blob/390dfbbcb026b33d46c52c62360b65deca00f258/crates/ui-egui/src/saved_path_edit_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked direct_selection_marquee_selects_and_arrow_params_name_the_saved_path -- --nocapture` — [crates/ui-egui/src/saved_path_edit_tests.rs / direct_selection_marquee_selects_and_arrow_params_name_the_saved_path](https://github.com/storytold/photocraft/blob/390dfbbcb026b33d46c52c62360b65deca00f258/crates/ui-egui/src/saved_path_edit_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked the_saved_path_is_only_edited_while_the_paths_panel_selects_it -- --nocapture` — [crates/ui-egui/src/saved_path_edit_tests.rs / the_saved_path_is_only_edited_while_the_paths_panel_selects_it](https://github.com/storytold/photocraft/blob/390dfbbcb026b33d46c52c62360b65deca00f258/crates/ui-egui/src/saved_path_edit_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked the_selected_saved_path_comes_before_the_work_path -- --nocapture` — [crates/ui-egui/src/saved_path_edit_tests.rs / the_selected_saved_path_comes_before_the_work_path](https://github.com/storytold/photocraft/blob/390dfbbcb026b33d46c52c62360b65deca00f258/crates/ui-egui/src/saved_path_edit_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked path_selection_moves_the_selected_saved_path -- --nocapture` — [crates/ui-egui/src/saved_path_edit_tests.rs / path_selection_moves_the_selected_saved_path](https://github.com/storytold/photocraft/blob/390dfbbcb026b33d46c52c62360b65deca00f258/crates/ui-egui/src/saved_path_edit_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked free_transform_path_names_the_selected_saved_path -- --nocapture` — [crates/ui-egui/src/saved_path_edit_tests.rs / free_transform_path_names_the_selected_saved_path](https://github.com/storytold/photocraft/blob/390dfbbcb026b33d46c52c62360b65deca00f258/crates/ui-egui/src/saved_path_edit_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked add_and_delete_anchor_point_edit_the_selected_saved_path -- --nocapture` — [crates/ui-egui/src/saved_path_edit_tests.rs / add_and_delete_anchor_point_edit_the_selected_saved_path](https://github.com/storytold/photocraft/blob/390dfbbcb026b33d46c52c62360b65deca00f258/crates/ui-egui/src/saved_path_edit_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked a_saved_path_read_from_a_psd_is_editable -- --nocapture` — [crates/ui-egui/src/saved_path_edit_tests.rs / a_saved_path_read_from_a_psd_is_editable](https://github.com/storytold/photocraft/blob/390dfbbcb026b33d46c52c62360b65deca00f258/crates/ui-egui/src/saved_path_edit_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked arrows_leave_a_saved_path_the_paths_panel_no_longer_selects -- --nocapture` — [crates/ui-egui/src/saved_path_edit_tests.rs / arrows_leave_a_saved_path_the_paths_panel_no_longer_selects](https://github.com/storytold/photocraft/blob/390dfbbcb026b33d46c52c62360b65deca00f258/crates/ui-egui/src/saved_path_edit_tests.rs)

- Expected behavior: direct selection drags an anchor of the selected saved path; direct selection marquee selects and arrow params name the saved path; the saved path is only edited while the paths panel selects it; the selected saved path comes before the work path; path selection moves the selected saved path; free transform path names the selected saved path; add and delete anchor point edit the selected saved path; a saved path read from a psd is editable; arrows leave a saved path the paths panel no longer selects. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`direct_selection_drags_an_anchor_of_the_selected_saved_path` setup/action excerpt:

```rust
let mut app = app(Tool::DirectSelection);
    with_saved_path(&mut app);
    let (before, n) = (saved(&app), steps(&app));
    drag(&mut app, [120.5, 120.5], [130.5, 135.5], Modifiers::NONE);
    let after = saved(&app);
    assert_eq!(after.subpaths[0].knots[2].anchor, Point::new(130.0, 135.0), "the dragged anchor moved");
    assert_eq!(after.subpaths[0].knots[0].anchor, Point::new(40.0, 40.0), "the others stay");
    assert_eq!(steps(&app), n + 1, "one history step");
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Saved(0)), vec![[0, 2]]));
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "no work path was made");
    assert!(app.session.undo());
    assert_eq!(saved(&app), before);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(after.subpaths[0].knots[2].anchor, Point::new(130.0, 135.0), "the dragged anchor moved");
    assert_eq!(after.subpaths[0].knots[0].anchor, Point::new(40.0, 40.0), "the others stay");
    assert_eq!(steps(&app), n + 1, "one history step");
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Saved(0)), vec![[0, 2]]));
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "no work path was made");
    assert!(app.session.undo());
    assert_eq!(saved(&app), before);
}
```

`direct_selection_marquee_selects_and_arrow_params_name_the_saved_path` setup/action excerpt:

```rust
let mut app = app(Tool::DirectSelection);
    with_saved_path(&mut app);
    drag(&mut app, [30.0, 30.0], [50.0, 50.0], Modifiers::NONE);
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Saved(0)), vec![[0, 0]]));
    assert_eq!(PathRef::Saved(0).params(&app), json!({"name": NAME}));
    // A path that is gone gets no name, which the engine refuses instead of editing another one.
    assert_eq!(PathRef::Saved(7).params(&app), json!({"name": ""}));
    assert!(app.run("path.moveAnchors", json!({"name": "", "anchors": [[0, 0]], "move": [1, 1]})).is_err());
}
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Saved(0)), vec![[0, 0]]));
    assert_eq!(PathRef::Saved(0).params(&app), json!({"name": NAME}));
    // A path that is gone gets no name, which the engine refuses instead of editing another one.
    assert_eq!(PathRef::Saved(7).params(&app), json!({"name": ""}));
    assert!(app.run("path.moveAnchors", json!({"name": "", "anchors": [[0, 0]], "move": [1, 1]})).is_err());
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9339: fix(export): embed requested sRGB profile for untagged sources (#9325)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked ico_export_is_available_and_rejects_oversize_output -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / ico_export_is_available_and_rejects_oversize_output](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked nearest_neighbor_doubles_each_pixel_into_an_exact_block -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / nearest_neighbor_doubles_each_pixel_into_an_exact_block](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked resample_choices_are_checked -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / resample_choices_are_checked](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked export_document_scales_height_when_rounded_width_is_unchanged -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / export_document_scales_height_when_rounded_width_is_unchanged](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked export_document_proxy_limits_both_dimensions_before_rounding -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / export_document_proxy_limits_both_dimensions_before_rounding](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked the_estimate_is_the_exported_files_size -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / the_estimate_is_the_exported_files_size](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked export_as_downscales_stroke_only_shapes_and_preview -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / export_as_downscales_stroke_only_shapes_and_preview](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked export_document_scales_and_flattens -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / export_document_scales_and_flattens](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked export_document_scales_when_only_the_height_changes -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / export_document_scales_when_only_the_height_changes](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked layered_preview_runs_outside_the_ui_frame -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / layered_preview_runs_outside_the_ui_frame](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked exact_pixels_lock_aspect_and_leave_the_source -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / exact_pixels_lock_aspect_and_leave_the_source](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked untagged_srgb_export_tags_without_rewriting_pixels -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / untagged_srgb_export_tags_without_rewriting_pixels](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked untagged_srgb_export_24mp_keeps_pixel_tiles -- --nocapture --ignored` — [crates/ui-egui/src/export_dialog.rs / untagged_srgb_export_24mp_keeps_pixel_tiles](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked untagged_jpeg_export_embeds_the_requested_srgb_profile -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / untagged_jpeg_export_embeds_the_requested_srgb_profile](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked preview_cache_tracks_working_profiles_but_prefers_embedded_profiles -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / preview_cache_tracks_working_profiles_but_prefers_embedded_profiles](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked preview_workers_keep_the_captured_source_profile -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / preview_workers_keep_the_captured_source_profile](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked replay_converts_untagged_pixels_from_the_original_working_profile -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / replay_converts_untagged_pixels_from_the_original_working_profile](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)
  - `cargo test -p photocraft-ui-egui --locked profile_and_metadata_follow_the_existing_exporter -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / profile_and_metadata_follow_the_existing_exporter](https://github.com/storytold/photocraft/blob/393645ab91460af2e37cc3ba8f6921e7551f676e/crates/ui-egui/src/export_dialog.rs)

- Expected behavior: ico export is available and rejects oversize output; nearest neighbor doubles each pixel into an exact block; resample choices are checked; export document scales height when rounded width is unchanged; export document proxy limits both dimensions before rounding; the estimate is the exported files size; export as downscales stroke only shapes and preview; export document scales and flattens; export document scales when only the height changes; layered preview runs outside the ui frame; exact pixels lock aspect and leave the source; untagged srgb export tags without rewriting pixels; untagged srgb export 24mp keeps pixel tiles; untagged jpeg export embeds the requested srgb profile; preview cache tracks working profiles but prefers embedded profiles; preview workers keep the captured source profile; replay converts untagged pixels from the original working profile; profile and metadata follow the existing exporter. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`ico_export_is_available_and_rejects_oversize_output` setup/action excerpt:

```rust
let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let mut f = Map::new();
        f.insert("format".into(), json!("ico"));
        f.insert("scale".into(), json!(100));
        assert!(FORMATS.iter().any(|(ext, _)| *ext == "ico"));
        assert!(default_export_document(&app.session.active().unwrap().doc, &f, None).err().is_some_and(|e| e.contains("256")));
        f.insert("scale".into(), json!(50));
        let resized = default_export_document(&app.session.active().unwrap().doc, &f, None).unwrap();
        assert_eq!((resized.size.width, resized.size.height), (150, 100));
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!(FORMATS.iter().any(|(ext, _)| *ext == "ico"));
        assert!(default_export_document(&app.session.active().unwrap().doc, &f, None).err().is_some_and(|e| e.contains("256")));
        f.insert("scale".into(), json!(50));
        let resized = default_export_document(&app.session.active().unwrap().doc, &f, None).unwrap();
        assert_eq!((resized.size.width, resized.size.height), (150, 100));
    }
```

`nearest_neighbor_doubles_each_pixel_into_an_exact_block` setup/action excerpt:

```rust
use photocraft_doc::SampleType;
        for depth in [SampleType::U8, SampleType::U16] {
            let doc = checker_doc(depth);
            let before = doc.clone();
            let mut f = Map::new();
            f.insert("format".into(), json!("png"));
            f.insert("scale".into(), json!(200));
            f.insert("resample".into(), json!("nearest"));
            let out = default_export_document(&doc, &f, None).unwrap();
            assert_eq!((out.size.width, out.size.height), (10, 6));
            assert_eq!(out.depth, depth);
            let surf = out.layers[0].surface().unwrap();
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!((out.size.width, out.size.height), (10, 6));
            assert_eq!(out.depth, depth);
            let surf = out.layers[0].surface().unwrap();
            for y in 0..6 {
                for x in 0..10 {
                    assert_eq!(surf.rgba(x, y), checker(x / 2, y / 2), "{depth:?} pixel {x},{y}");
                }
            }
            assert_eq!(doc, before, "{depth:?}: export leaves the source document unchanged");

            // The preview proxy keeps Nearest Neighbor's hard edges too.
            let proxy = default_export_document(&doc, &f, Some(512)).unwrap();
            assert_eq!(proxy.layers[0].surface().unwrap().rgba(1, 1), checker(0, 0), "{depth:?}: preview proxy is nearest as well");
            assert_eq!(preview_filter(&f), egui::TextureOptions::NEAREST);

            f.insert("resample".into(), json!(DEFAULT_RESAMPLE));
            let smooth = default_export_document(&doc, &f, None).unwrap();
            assert!(blended, "{depth:?}: Bicubic Automatic blends neighbouring squares");
            assert_eq!(preview_filter(&f), egui::TextureOptions::LINEAR);
        }
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9342: fix(io, engine): Embed Color Profile writes sRGB for an untagged document (#9325)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-automation --locked doc_save_rejects_a_bad_quality_instead_of_ignoring_or_clamping_it -- --nocapture` — [crates/automation/src/rpc.rs / doc_save_rejects_a_bad_quality_instead_of_ignoring_or_clamping_it](https://github.com/storytold/photocraft/blob/c46a74621d36adae9707ab8f3eb39cf6b4c302b9/crates/automation/src/rpc.rs)
  - `cargo test -p photocraft-engine --locked exif_srgb_jpeg_round_trip -- --nocapture` — [crates/engine/src/color_cmds.rs / exif_srgb_jpeg_round_trip](https://github.com/storytold/photocraft/blob/c46a74621d36adae9707ab8f3eb39cf6b4c302b9/crates/engine/src/color_cmds.rs)
  - `cargo test -p photocraft-engine --locked embed_color_profile_tags_an_untagged_document_srgb -- --nocapture` — [crates/engine/src/web_cmds/tests.rs / embed_color_profile_tags_an_untagged_document_srgb](https://github.com/storytold/photocraft/blob/c46a74621d36adae9707ab8f3eb39cf6b4c302b9/crates/engine/src/web_cmds/tests.rs)
  - `cargo test -p photocraft-io --locked untagged_documents_embed_the_profile_they_are_assumed_to_be_in -- --nocapture` — [crates/io/src/flat.rs / untagged_documents_embed_the_profile_they_are_assumed_to_be_in](https://github.com/storytold/photocraft/blob/c46a74621d36adae9707ab8f3eb39cf6b4c302b9/crates/io/src/flat.rs)
  - `cargo test -p photocraft-ui-egui --locked the_estimate_is_the_exported_files_size -- --nocapture` — [crates/ui-egui/src/export_dialog.rs / the_estimate_is_the_exported_files_size](https://github.com/storytold/photocraft/blob/c46a74621d36adae9707ab8f3eb39cf6b4c302b9/crates/ui-egui/src/export_dialog.rs)

- Expected behavior: doc save rejects a bad quality instead of ignoring or clamping it; exif srgb jpeg round trip; embed color profile tags an untagged document srgb; untagged documents embed the profile they are assumed to be in; the estimate is the exported files size. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`doc_save_rejects_a_bad_quality_instead_of_ignoring_or_clamping_it` setup/action excerpt:

```rust
let dir = std::env::temp_dir().join(format!("pc-rpc-save-quality-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.webp");
        let mut h = Headless::trusted_local();
        h.handle("doc.new", json!({"width": 16, "height": 16})).unwrap();
        for q in [json!(85.0), json!("85"), json!(0), json!(500), json!(-5), json!(true)] {
            let err = h.handle("doc.save", json!({"path": path.to_string_lossy(), "quality": q})).unwrap_err().to_string();
            assert!(err.contains("quality"), "{q}: {err}");
            assert!(!path.exists(), "{q} wrote a file");
        }
        h.handle("doc.save", json!({"path": path.to_string_lossy(), "quality": 85})).unwrap();
        let bytes = std::fs::read(&path).unwrap();
```

Expected assertion context (overlapping windows merged):

```rust
assert!(err.contains("quality"), "{q}: {err}");
            assert!(!path.exists(), "{q} wrote a file");
        }
        h.handle("doc.save", json!({"path": path.to_string_lossy(), "quality": 85})).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.windows(4).any(|c| c == b"VP8 ") && !bytes.windows(4).any(|c| c == b"VP8L"), "a WebP saved with a quality is lossy");
        std::fs::remove_dir_all(dir).unwrap();
    }
```

`exif_srgb_jpeg_round_trip` setup/action excerpt:

```rust
let mut s = Session::new();
        s.execute("edit.colorSettings", json!({"workingRgb": "display-p3"})).unwrap();
        // Embed Color Profile off: an untagged document is otherwise tagged sRGB on export (#9325).
        let mut opts = photocraft_io::ExportOptions::default();
        opts.encode.embed_icc = false;
        let r = photocraft_io::export(&untagged_camera_file(Some(1)), "x.jpg", &opts).unwrap();
        let back = photocraft_io::import("x.jpg", &r.bytes).unwrap().document;
        assert!(back.icc_profile.is_none(), "no ICC profile in the file");
        let (_, r) = s.open_document(back, None);
        assert_eq!((r["action"].as_str(), r["exif"].as_bool()), (Some("kept"), Some(true)));
        assert!(desc(&s.active().unwrap().doc).unwrap().starts_with("sRGB"));
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!(back.icc_profile.is_none(), "no ICC profile in the file");
        let (_, r) = s.open_document(back, None);
        assert_eq!((r["action"].as_str(), r["exif"].as_bool()), (Some("kept"), Some(true)));
        assert!(desc(&s.active().unwrap().doc).unwrap().starts_with("sRGB"));
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9401: fix(format): a save on macOS keeps the file's creation time (#9388)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-automation --locked write_keeps_the_replaced_files_creation_time -- --nocapture` — [crates/automation/src/workspace.rs / write_keeps_the_replaced_files_creation_time](https://github.com/storytold/photocraft/blob/f241bba1adfb9c7ec5dd5ed9d86f95b63c7bc2c3/crates/automation/src/workspace.rs)
  - `cargo test -p photocraft-format --locked keeps_the_original_creation_time -- --nocapture` — [crates/format/src/atomic/tests.rs / keeps_the_original_creation_time](https://github.com/storytold/photocraft/blob/f241bba1adfb9c7ec5dd5ed9d86f95b63c7bc2c3/crates/format/src/atomic/tests.rs)
  - `cargo test -p photocraft-format --locked keeps_the_creation_time_under_a_deny_write_acl -- --nocapture` — [crates/format/src/atomic/tests.rs / keeps_the_creation_time_under_a_deny_write_acl](https://github.com/storytold/photocraft/blob/f241bba1adfb9c7ec5dd5ed9d86f95b63c7bc2c3/crates/format/src/atomic/tests.rs)

- Expected behavior: write keeps the replaced files creation time; keeps the original creation time; keeps the creation time under a deny write acl. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`write_keeps_the_replaced_files_creation_time` setup/action excerpt:

```rust
use std::os::macos::fs::FileTimesExt;
        let (inside, _, workspace) = roots("created");
        let path = inside.join("doc.psd");
        workspace.write("doc.psd", b"original").unwrap();
        let born = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_times(std::fs::FileTimes::new().set_created(born)).unwrap();
        workspace.write("doc.psd", b"replacement").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(std::fs::metadata(&path).unwrap().created().unwrap(), born, "the creation time survives the automation write");
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(std::fs::metadata(&path).unwrap().created().unwrap(), born, "the creation time survives the automation write");
    }
```

`keeps_the_original_creation_time` setup/action excerpt:

```rust
use std::os::macos::fs::FileTimesExt;
    let d = TempDir::new("created");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    // A creation time well in the past, so a reset to "now" can't pass for it.
    let born = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_times(std::fs::FileTimes::new().set_created(born)).unwrap();
    assert_eq!(std::fs::metadata(&p).unwrap().created().unwrap(), born);
    atomic_write(&p, b"new").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"new");
    assert_eq!(std::fs::metadata(&p).unwrap().created().unwrap(), born, "the creation time survives the save");
    assert_no_temp(&d.0);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(std::fs::metadata(&p).unwrap().created().unwrap(), born);
    atomic_write(&p, b"new").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"new");
    assert_eq!(std::fs::metadata(&p).unwrap().created().unwrap(), born, "the creation time survives the save");
    assert_no_temp(&d.0);
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9416: Affinity reader: contiguous curve records and a cheaper object tag index

- Fixture/setup: Synthetic repeated tags and fixed-record stream in affinity/src/tests.rs; licensed curve document with known topology/reference.
- Action: run each command separately:

  - `cargo test -p photocraft-affinity --locked lookup_keeps_first_match_for_every_object_size -- --nocapture`
  - `cargo test -p photocraft-affinity --locked a_fixed_size_array_is_one_buffer_and_one_value_per_record -- --nocapture`

- Expected: Tag lookup preserves first occurrence for sizes 0–59 and positional-tag exclusion; five 18-byte records occupy one buffer and report five elements. Full curve import preserves finite control points, nodes and topology.
- Separate coverage / prerequisites: Also run full affinity suite and applicable pinned corpus tests. Record curve fixture hash and compare imported geometry/appearance; if no end-to-end contiguous-curve fixture exists, mark that subcase BLOCKED. Storage tests alone do not establish render fidelity.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9590: fix(io): PSD layer-effect contours keep at most 16 points

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-io --locked psd_contours_keep_at_most_16_points_and_their_peaks -- --nocapture` — [crates/io/src/effects_map.rs / psd_contours_keep_at_most_16_points_and_their_peaks](https://github.com/storytold/photocraft/blob/a9744ed44dc782613b19a97b8c159a091f73e669/crates/io/src/effects_map.rs)
  - `cargo test -p photocraft-io --locked a_point_on_a_tiny_straight_span_is_not_a_bend -- --nocapture` — [crates/io/src/effects_map.rs / a_point_on_a_tiny_straight_span_is_not_a_bend](https://github.com/storytold/photocraft/blob/a9744ed44dc782613b19a97b8c159a091f73e669/crates/io/src/effects_map.rs)

- Expected behavior: psd contours keep at most 16 points and their peaks; a point on a tiny straight span is not a bend. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`psd_contours_keep_at_most_16_points_and_their_peaks` setup/action excerpt:

```rust
let points: Vec<CurvePoint> = (0..29)
            .map(|i| {
                let x = i as f32 / 28.0;
                // Rings at x = 0.25 and 0.75, small wiggles between.
                let y = (1.0 - ((x - 0.25).abs() * 8.0)).max(0.0).max(1.0 - (x - 0.75).abs() * 8.0) + if i % 2 == 0 { 0.0 } else { 0.01 };
                CurvePoint { input: x, output: y.clamp(0.0, 1.0) }
            })
            .collect();
        let c = Contour::Custom { name: "Two rings".into(), points: points.clone(), corners: vec![false; 29] };
        let out = crv(&contour_value(&c));
        assert_eq!(out.len(), 16);
        assert_eq!(out.first().copied(), Some((0.0, f64::from(points[0].output) * 255.0)));
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(out.len(), 16);
        assert_eq!(out.first().copied(), Some((0.0, f64::from(points[0].output) * 255.0)));
        assert_eq!(out.last().map(|p| p.0), Some(255.0));
        let peak = |x: f32| out.iter().any(|p| ((p.0 / 255.0) as f32 - x).abs() < 0.03 && p.1 > 240.0);
        assert!(peak(0.25) && peak(0.75), "both rings kept: {out:?}");
        assert!(out.windows(2).all(|w| w[0].0 < w[1].0), "in order: {out:?}");

        let short = Contour::Custom { name: "Short".into(), points: points[..16].to_vec(), corners: vec![false; 16] };
        assert_eq!(crv(&contour_value(&short)).len(), 16);
    }
```

`a_point_on_a_tiny_straight_span_is_not_a_bend` setup/action excerpt:

```rust
let p = |input: f32, output: f32| CurvePoint { input, output };
        // Three distinct inputs a few ulps apart near 0.5, on a straight rise from 0 to 1.
        let (a, b) = (0.5_f32, f32::from_bits(0.5_f32.to_bits() + 2));
        let mid = f32::from_bits(0.5_f32.to_bits() + 1);
        let mut points = vec![p(0.0, 0.0), p(0.2, 0.0)];
        points.extend([p(a, 0.0), p(mid, 0.5), p(b, 1.0)]);
        // Fourteen gentle bends after it, each one a real (if small) peak.
        points.extend((0..14).map(|k| p(0.55 + k as f32 * 0.03, if k % 2 == 0 { 1.0 } else { 0.9 })));
        points.push(p(1.0, 1.0));
        let kept = psd_contour_points(&points);
        assert_eq!(kept.len(), 16);
        assert!(!kept.contains(&3), "the point on the tiny straight span was kept over a real bend: {kept:?}");
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(kept.len(), 16);
        assert!(!kept.contains(&3), "the point on the tiny straight span was kept over a real bend: {kept:?}");
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-4763: refactor(liquify): reuse native brush resizing and anchored HUD feedback

- Fixture/setup: Windows portable build, synthetic grid in Liquify, Warp tool, brush diameter 40 px, density 50%, preview zoom 100%; repeat at 200%. Record document/history and brush settings before the gesture.
- Action: Hold Alt and right-drag without Shift, or Ctrl+Alt and primary-drag. Move 25 screen points right: at 100% zoom diameter becomes 90 px, at 200% zoom 65 px. From density 50%, move 100 screen points down: density becomes 100% at either zoom; 100 points up becomes 0%. Release, then Cancel Liquify. Alt+primary alone remains the opposite-tool gesture; Shift+Alt+right is not brush resize.
- Expected: HUD and circle remain anchored at press origin, show diameter/density, and follow the shared zoom-corrected resize formula. Resizing starts on the press and creates no paint stroke/history; releasing ends resize. Cancel restores original document and history.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester:

## PR-4985: feat(ui): add Shift+right-click brush mode picker

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked shift_right_mode_picker_selects_journals_and_clear_erases -- --nocapture` — [crates/ui-egui/src/canvas/paint_mouse/tests.rs / shift_right_mode_picker_selects_journals_and_clear_erases](https://github.com/storytold/photocraft/blob/1dc98f80f60e993f3edf20af815391b6a7e7add7/crates/ui-egui/src/canvas/paint_mouse/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked mode_picker_outside_dismissal_does_not_paint_or_erase -- --nocapture` — [crates/ui-egui/src/canvas/paint_mouse/tests.rs / mode_picker_outside_dismissal_does_not_paint_or_erase](https://github.com/storytold/photocraft/blob/1dc98f80f60e993f3edf20af815391b6a7e7add7/crates/ui-egui/src/canvas/paint_mouse/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked mode_control_can_pick_a_row_with_ui_click -- --nocapture` — [crates/ui-egui/src/canvas/paint_mouse/tests.rs / mode_control_can_pick_a_row_with_ui_click](https://github.com/storytold/photocraft/blob/1dc98f80f60e993f3edf20af815391b6a7e7add7/crates/ui-egui/src/canvas/paint_mouse/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked shift_right_mode_picker_cancels_and_other_gestures_keep_their_routes -- --nocapture` — [crates/ui-egui/src/canvas/paint_mouse/tests.rs / shift_right_mode_picker_cancels_and_other_gestures_keep_their_routes](https://github.com/storytold/photocraft/blob/1dc98f80f60e993f3edf20af815391b6a7e7add7/crates/ui-egui/src/canvas/paint_mouse/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked escape_dismissal_consumes_a_held_secondary_release -- --nocapture` — [crates/ui-egui/src/canvas/paint_mouse/tests.rs / escape_dismissal_consumes_a_held_secondary_release](https://github.com/storytold/photocraft/blob/1dc98f80f60e993f3edf20af815391b6a7e7add7/crates/ui-egui/src/canvas/paint_mouse/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked shift_released_before_the_right_button_still_opens_the_mode_picker -- --nocapture` — [crates/ui-egui/src/canvas/paint_mouse/tests.rs / shift_released_before_the_right_button_still_opens_the_mode_picker](https://github.com/storytold/photocraft/blob/1dc98f80f60e993f3edf20af815391b6a7e7add7/crates/ui-egui/src/canvas/paint_mouse/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked shift_right_brush_does_not_open_presets_or_paint -- --nocapture` — [crates/ui-egui/src/control/tests.rs / shift_right_brush_does_not_open_presets_or_paint](https://github.com/storytold/photocraft/blob/1dc98f80f60e993f3edf20af815391b6a7e7add7/crates/ui-egui/src/control/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked brush_mode_control_routes_modifiers_and_reports_choices_without_history -- --nocapture` — [crates/ui-egui/src/control/tests.rs / brush_mode_control_routes_modifiers_and_reports_choices_without_history](https://github.com/storytold/photocraft/blob/1dc98f80f60e993f3edf20af815391b6a7e7add7/crates/ui-egui/src/control/tests.rs)

- Expected behavior: shift right mode picker selects journals and clear erases; mode picker outside dismissal does not paint or erase; mode control can pick a row with ui click; shift right mode picker cancels and other gestures keep their routes; escape dismissal consumes a held secondary release; shift released before the right button still opens the mode picker; shift right brush does not open presets or paint; brush mode control routes modifiers and reports choices without history. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`shift_right_mode_picker_selects_journals_and_clear_erases` setup/action excerpt:

```rust
use egui_kittest::kittest::Queryable;
    let mut h = harness(Some("erase"));
    let history = h.state().session.active().unwrap().history.past_len();
    let journal = h.state().session.journal.len();
    mode_right_click(&mut h, Modifiers::SHIFT);
    assert!(h.state().ui.brush_mode_menu.is_some());
    assert!(h.state().ui.brush_picker.is_none() && h.state().drag.is_none());
    assert_eq!(h.state().session.journal.len(), journal);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    h.get_by_label("Multiply").click();
    h.run_steps(2);
    assert!(h.state().ui.brush_mode_menu.is_none());
```

Expected assertion context (overlapping windows merged):

```rust
assert!(h.state().ui.brush_mode_menu.is_some());
    assert!(h.state().ui.brush_picker.is_none() && h.state().drag.is_none());
    assert_eq!(h.state().session.journal.len(), journal);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    h.get_by_label("Multiply").click();
    h.run_steps(2);
    assert!(h.state().ui.brush_mode_menu.is_none());
    assert_eq!(h.state().session.tools.brush.mode, photocraft_doc::BlendMode::Multiply);
    assert_eq!(h.state().session.journal.last().unwrap(), &("tools.setBrush".into(), json!({"brush": {"mode": "Multiply"}})));
    let (a, b) = drag(&mut h, PointerButton::Primary);
    let mid = a + (b - a) * 0.5;
    assert!(alpha_at(&h, mid) > 0.9);
    mode_right_click(&mut h, Modifiers::SHIFT);
    h.get_by_label("Clear").click();
    h.run_steps(2);
    assert!(h.state().ui.tool_options.brush_clear);
    assert_eq!(h.state().session.tools.brush.mode, photocraft_doc::BlendMode::Multiply, "Clear keeps the underlying mode, as the options bar does");
    drag(&mut h, PointerButton::Primary);
    assert_eq!(alpha_at(&h, mid), 0.0);
    assert!(strokes(&h).last().unwrap()["erase"].as_bool().unwrap());
}
```

`mode_picker_outside_dismissal_does_not_paint_or_erase` setup/action excerpt:

```rust
for button in [PointerButton::Primary, PointerButton::Secondary] {
        let mut h = harness(Some("erase"));
        drag(&mut h, PointerButton::Primary);
        let journal = h.state().session.journal.len();
        let history = h.state().session.active().unwrap().history.past_len();
        mode_right_click(&mut h, Modifiers::SHIFT);
        // A full outside drag must only dismiss, including its release click.
        drag(&mut h, button);
        assert!(h.state().ui.brush_mode_menu.is_none());
        assert_eq!(h.state().session.journal.len(), journal, "dismissal must not paint/erase with {button:?}");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!(h.state().ui.brush_mode_menu.is_none());
        assert_eq!(h.state().session.journal.len(), journal, "dismissal must not paint/erase with {button:?}");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    }
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-5082: Mixer Brush Alt-click loads reservoir while preserving foreground

Historical PR title: fix(mixer-brush): Alt-click samples the foreground colour like the Brush (#4873). The acceptance behavior is reservoir loading with foreground unchanged.


- Fixture/setup: Red/green canvas with blue foreground from alt_click_tests.rs; engine 80×40 transparent canvas with blue stroke and red foreground.
- Action: run each command separately:

  - `cargo test -p photocraft-ui-egui --locked alt_click_with_the_mixer_brush_loads_the_reservoir_not_the_foreground -- --nocapture`
  - `cargo test -p photocraft-engine --locked mixer_load_fills_the_reservoir_from_the_canvas_and_keeps_the_foreground -- --nocapture`

- Expected: Alt-click loads Current Brush Load/reservoir, preserving foreground and document revision and painting nothing. Dry stroke uses sampled blue until foreground changes; green foreground then paints green. Transparent sampling loads nothing; invalid coordinates preserve all state.
- Separate coverage / prerequisites: Historical PR title is stale: this case follows reviewed reservoir behavior, not foreground replacement. Repeat Alt/Option-click natively as a separate NOT RUN subcase.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-5165: Fix: brush section selection

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked selecting_a_controls_section_keeps_its_enabled_flag_and_brush_history -- --nocapture` — [crates/ui-egui/src/brush_panel_tests.rs / selecting_a_controls_section_keeps_its_enabled_flag_and_brush_history](https://github.com/storytold/photocraft/blob/27bc6e4b37b701480efa69559ab13eafc52bd94c/crates/ui-egui/src/brush_panel_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked controls_section_checkbox_toggles_without_selecting_its_settings -- --nocapture` — [crates/ui-egui/src/brush_panel_tests.rs / controls_section_checkbox_toggles_without_selecting_its_settings](https://github.com/storytold/photocraft/blob/27bc6e4b37b701480efa69559ab13eafc52bd94c/crates/ui-egui/src/brush_panel_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked f5_edits_the_default_brush_with_every_section -- --nocapture` — [crates/ui-egui/src/brush_panel_tests.rs / f5_edits_the_default_brush_with_every_section](https://github.com/storytold/photocraft/blob/27bc6e4b37b701480efa69559ab13eafc52bd94c/crates/ui-egui/src/brush_panel_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked section_boxes_show_a_pressed_state -- --nocapture` — [crates/ui-egui/src/brush_panel_tests.rs / section_boxes_show_a_pressed_state](https://github.com/storytold/photocraft/blob/27bc6e4b37b701480efa69559ab13eafc52bd94c/crates/ui-egui/src/brush_panel_tests.rs)

- Expected behavior: selecting a controls section keeps its enabled flag and brush history; controls section checkbox toggles without selecting its settings; f5 edits the default brush with every section; section boxes show a pressed state. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`selecting_a_controls_section_keeps_its_enabled_flag_and_brush_history` setup/action excerpt:

```rust
use egui_kittest::kittest::Queryable;

    for (section, name) in [(1, "Shape Dynamics"), (2, "Scattering"), (3, "Texture")] {
        let mut h = harness(0, 0);
        let before = h.state().session.tools.brush.clone();
        let journal = h.state().session.journal.len();

        h.get_by_label(name).click();
        h.run_steps(3);

        assert_eq!(h.state().ui.brush_section, section, "{name} settings are visible");
        assert_eq!(h.state().session.tools.brush, before, "{name} remains disabled");
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(h.state().ui.brush_section, section, "{name} settings are visible");
        assert_eq!(h.state().session.tools.brush, before, "{name} remains disabled");
        assert_eq!(h.state().session.journal.len(), journal, "selecting {name} is not a brush edit");
    }
}
```

`controls_section_checkbox_toggles_without_selecting_its_settings` setup/action excerpt:

```rust
use egui_kittest::kittest::Queryable;

    let mut h = harness(0, 0);
    let row = h.get_by_label("Scattering").rect();
    let checkbox = egui::pos2(row.left() + 12.0, row.center().y);
    let journal = h.state().session.journal.len();

    for enabled in [true, false] {
        h.hover_at(checkbox);
        h.run_steps(1);
        h.drag_at(checkbox);
        h.run_steps(1);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(h.state().session.tools.brush.scattering.enabled, enabled);
        assert_eq!(h.state().ui.brush_section, 0, "the checkbox does not select Scattering");
        assert_eq!(last_journal(h.state()).unwrap().1["brush"], json!({"scattering": {"enabled": enabled}}));
    }
    assert_eq!(h.state().session.journal.len(), journal + 2);
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-6139: Fix painting workflows, File Info and layer selection

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-automation --locked a_refresh_outside_command_execution_surfaces_the_denial_instead_of_rendering_unfiltered -- --nocapture` — [crates/automation/tests/smart_filter_authorization.rs / a_refresh_outside_command_execution_surfaces_the_denial_instead_of_rendering_unfiltered](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/automation/tests/smart_filter_authorization.rs)
  - `cargo test -p photocraft-engine --locked behind_brush_and_pencil_preserve_foreground_and_round_trip_history -- --nocapture` — [crates/engine/src/brush_cmds/tests.rs / behind_brush_and_pencil_preserve_foreground_and_round_trip_history](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/engine/src/brush_cmds/tests.rs)
  - `cargo test -p photocraft-paint --locked behind_preserves_native_channels_and_respects_alpha_selection_and_locks -- --nocapture` — [crates/paint/src/brush_tests.rs / behind_preserves_native_channels_and_respects_alpha_selection_and_locks](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/paint/src/brush_tests.rs)
  - `cargo test -p photocraft-paint --locked live_union_of_passes_matches_merging_the_finished_passes_after_every_chunk -- --nocapture` — [crates/paint/src/brush_tests.rs / live_union_of_passes_matches_merging_the_finished_passes_after_every_chunk](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/paint/src/brush_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked german_color_picker_fits_its_actions_and_translates_the_title -- --nocapture` — [crates/ui-egui/src/dialogs.rs / german_color_picker_fits_its_actions_and_translates_the_title](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/ui-egui/src/dialogs.rs)
  - `cargo test -p photocraft-ui-egui --locked file_info_multiline_fields_keep_enter_and_cancel_discards_edits -- --nocapture` — [crates/ui-egui/src/dialogs.rs / file_info_multiline_fields_keep_enter_and_cancel_discards_edits](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/ui-egui/src/dialogs.rs)
  - `cargo test -p photocraft-ui-egui --locked file_info_keeps_multiline_metadata_on_confirm_and_reopen -- --nocapture` — [crates/ui-egui/src/dialogs.rs / file_info_keeps_multiline_metadata_on_confirm_and_reopen](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/ui-egui/src/dialogs.rs)
  - `cargo test -p photocraft-ui-egui --locked mask_sampling_sets_gray_swatches_without_changing_pixel_sampling -- --nocapture` — [crates/ui-egui/src/eyedropper_ui.rs / mask_sampling_sets_gray_swatches_without_changing_pixel_sampling](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/ui-egui/src/eyedropper_ui.rs)
  - `cargo test -p photocraft-ui-egui --locked move_alt_copy_cursor_follows_the_latched_duplication_drag -- --nocapture` — [crates/ui-egui/src/input_tests.rs / move_alt_copy_cursor_follows_the_latched_duplication_drag](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/ui-egui/src/input_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked assignable_pick_layer_shortcut_follows_transformed_canvas_without_editing -- --nocapture` — [crates/ui-egui/src/input_tests.rs / assignable_pick_layer_shortcut_follows_transformed_canvas_without_editing](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/ui-egui/src/input_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked the_wheel_over_the_brush_mode_dropdown_steps_the_painting_mode -- --nocapture` — [crates/ui-egui/src/options_bar_mode_wheel_tests.rs / the_wheel_over_the_brush_mode_dropdown_steps_the_painting_mode](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/ui-egui/src/options_bar_mode_wheel_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked space_repositions_a_pen_anchor_and_keeps_its_handles -- --nocapture` — [crates/ui-egui/src/pen_draw_tests.rs / space_repositions_a_pen_anchor_and_keeps_its_handles](https://github.com/storytold/photocraft/blob/e2347f8ead85d929193044f139788b850e31c06f/crates/ui-egui/src/pen_draw_tests.rs)

- Expected behavior: a refresh outside command execution surfaces the denial instead of rendering unfiltered; behind brush and pencil preserve foreground and round trip history; behind preserves native channels and respects alpha selection and locks; live union of passes matches merging the finished passes after every chunk; german color picker fits its actions and translates the title; file info multiline fields keep enter and cancel discards edits; file info keeps multiline metadata on confirm and reopen; mask sampling sets gray swatches without changing pixel sampling; move alt copy cursor follows the latched duplication drag; assignable pick layer shortcut follows transformed canvas without editing; the wheel over the brush mode dropdown steps the painting mode; space repositions a pen anchor and keeps its handles. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`a_refresh_outside_command_execution_surfaces_the_denial_instead_of_rendering_unfiltered` setup/action excerpt:

```rust
// link-watch and transform paths re-render through Session::edit, not Session::execute or
    // jobs::run; the gate must reach them and the denial must not silently pass pixels through
    // into the process-wide stack cache.
    for lookup in [false, true] {
        let f = Fixture::new();
        let (doc, layer) = f.saved_filter(lookup);
        let mut h = f.headless();
        h.open(Path::new("stored.pcraft")).unwrap();
        let before = h.session.active().unwrap().doc.clone();
        // Session::edit is the outside-execute mutation path (an edit closure the UI or a
        // linked-file change runs); the stored filter must be denied there too.
        assert_denied(h.session.edit("Re-render", |d, _| match smart_cmds::refresh(d, layer) {
```

Expected assertion context (overlapping windows merged):

```rust
assert!(Arc::ptr_eq(&before, &h.session.active().unwrap().doc));
        // A trusted session (no gate) still renders the same document.
        let mut trusted = Session::new();
        trusted.add_document(doc, None);
        assert!(smart_cmds::refresh(&mut d, layer).is_ok());
    }
}
```

`behind_brush_and_pencil_preserve_foreground_and_round_trip_history` setup/action excerpt:

```rust
for cmd in ["paint.stroke", "paint.pencil"] {
        let mut s = session(40, 20);
        s.execute(cmd, json!({"points": [[10, 10]], "size": 6, "color": "#ff0000", "hardness": 1.0})).unwrap();
        let before = surface(&s);
        s.execute(cmd, json!({"points": [[2, 10], [38, 10]], "size": 6, "color": "#0000ff", "mode": "behind", "hardness": 1.0})).unwrap();
        assert_eq!(rgba(&s, 10, 10), [1.0, 0.0, 0.0, 1.0], "{cmd}: existing opaque paint");
        assert_eq!(rgba(&s, 25, 10), [0.0, 0.0, 1.0, 1.0], "{cmd}: transparent pixels");
        let painted = surface(&s);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(same_pixels(&surface(&s), &before, Rect::new(0, 0, 40, 20)));
        s.execute("edit.redo", json!({})).unwrap();
        assert!(same_pixels(&surface(&s), &painted, Rect::new(0, 0, 40, 20)));
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(rgba(&s, 10, 10), [1.0, 0.0, 0.0, 1.0], "{cmd}: existing opaque paint");
        assert_eq!(rgba(&s, 25, 10), [0.0, 0.0, 1.0, 1.0], "{cmd}: transparent pixels");
        let painted = surface(&s);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(same_pixels(&surface(&s), &before, Rect::new(0, 0, 40, 20)));
        s.execute("edit.redo", json!({})).unwrap();
        assert!(same_pixels(&surface(&s), &painted, Rect::new(0, 0, 40, 20)));
    }
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-7848: fix(retouch): preserve Smudge pickup with live-pixel stroke commits

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked pressure_shape_and_smoothing_match_replay -- --nocapture` — [crates/engine/tests/live_dab_matches_replay.rs / pressure_shape_and_smoothing_match_replay](https://github.com/storytold/photocraft/blob/389a1d5bd67072d5581c27989080222a7065b585/crates/engine/tests/live_dab_matches_replay.rs)
  - `cargo test -p photocraft-engine --locked native_modes_selection_and_lock_match_replay -- --nocapture` — [crates/engine/tests/live_dab_matches_replay.rs / native_modes_selection_and_lock_match_replay](https://github.com/storytold/photocraft/blob/389a1d5bd67072d5581c27989080222a7065b585/crates/engine/tests/live_dab_matches_replay.rs)
  - `cargo test -p photocraft-engine --locked changed_document_refuses_live_pixels -- --nocapture` — [crates/engine/tests/live_dab_matches_replay.rs / changed_document_refuses_live_pixels](https://github.com/storytold/photocraft/blob/389a1d5bd67072d5581c27989080222a7065b585/crates/engine/tests/live_dab_matches_replay.rs)
  - `cargo test -p photocraft-engine --locked smudge_pickup_matches_full_neighborhood_when_pressure_expands -- --nocapture` — [crates/engine/tests/live_dab_matches_replay.rs / smudge_pickup_matches_full_neighborhood_when_pressure_expands](https://github.com/storytold/photocraft/blob/389a1d5bd67072d5581c27989080222a7065b585/crates/engine/tests/live_dab_matches_replay.rs)
  - `cargo test -p photocraft-engine --locked pressure_sized_live_dabs_commit_only_the_requested_mask_or_channel -- --nocapture` — [crates/engine/tests/live_dab_matches_replay.rs / pressure_sized_live_dabs_commit_only_the_requested_mask_or_channel](https://github.com/storytold/photocraft/blob/389a1d5bd67072d5581c27989080222a7065b585/crates/engine/tests/live_dab_matches_replay.rs)
  - `cargo test -p photocraft-ui-egui --locked pressure_sized_smudge_release_matches_replay_and_undo_redo -- --nocapture` — [crates/ui-egui/src/stroke_timing_tests.rs / pressure_sized_smudge_release_matches_replay_and_undo_redo](https://github.com/storytold/photocraft/blob/389a1d5bd67072d5581c27989080222a7065b585/crates/ui-egui/src/stroke_timing_tests.rs)

- Expected behavior: pressure shape and smoothing match replay; native modes selection and lock match replay; changed document refuses live pixels; smudge pickup matches full neighborhood when pressure expands; pressure sized live dabs commit only the requested mask or channel; pressure sized smudge release matches replay and undo redo. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`pressure_shape_and_smoothing_match_replay` setup/action excerpt:

```rust
let mut failures = Vec::new();
    for cmd in ["paint.smudge", "paint.blur", "paint.sharpen", "paint.dodge", "paint.burn", "paint.sponge"] {
        for case in ["pressure", "locked", "ellipse", "smoothing", "scattering"] {
            for depth in [8, 16, 32] {
                let mut s = setup(depth, "rgb");
                match case {
                    "locked" => {
                        s.edit("lock alpha", |doc, active| {
                            doc.layer_mut(active.unwrap()).unwrap().locks.transparency = true;
                            Ok(())
                        })
                        .unwrap();
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(result["fromLive"], true);
                let got = pixels(&s);
                let count = got.iter().zip(&want).filter(|(a, b)| a != b).count();
                let max = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
```

`native_modes_selection_and_lock_match_replay` setup/action excerpt:

```rust
for mode in ["rgb", "gray", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            let mut s = setup(depth, mode);
            s.edit("lock alpha", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().locks.transparency = true;
                Ok(())
            })
            .unwrap();
            s.execute("select.rect", json!({"x":30,"y":20,"width":110,"height":60})).unwrap();
            let points = [[-10.0, 30.0], [80.0, 50.0], [160.0, 80.0], [60.0, 60.0]];
            for cmd in ["paint.blur", "paint.sharpen", "paint.dodge", "paint.burn", "paint.sponge", "paint.smudge"] {
                let params = json!({"points":points,"size":30,"hardness":40,"strength":70,"opacity":37});
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(result["fromLive"], true);
                assert_eq!(result["damage"], expected_damage);
                assert_eq!(pixels(&s), want, "{mode} {depth} {cmd}");
                s.execute("edit.undo", json!({})).unwrap();
            }
        }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9623: Pencil: keep a 1 px freehand line connected where it turns

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-paint --locked pencil_1px_freehand_line_never_breaks_where_it_turns -- --nocapture` — [crates/paint/src/brush_tests.rs / pencil_1px_freehand_line_never_breaks_where_it_turns](https://github.com/storytold/photocraft/blob/652075f4da7fe905ad651eebc3a6c276fe1d7a5d/crates/paint/src/brush_tests.rs)

- Expected behavior: pencil 1px freehand line never breaks where it turns. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`pencil_1px_freehand_line_never_breaks_where_it_turns` setup/action excerpt:

```rust
// #8040: each segment of a freehand stroke samples its own line, so where the stroke turned
    // the last pixel of one segment and the first of the next could sit two pixels apart.
    let b = BrushSettings { aliased: true, hardness: 1.0, size: 1.0, ..brush() };
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for _ in 0..300 {
        let mut pts = vec![StrokePoint::new(20.0 + rnd() * 8.0, 20.0 + rnd() * 8.0, 1.0)];
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(reached.len(), painted.len(), "the line breaks along {path:?}");
    }
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-3554: fix(packaging): launch bundled CLI directly from AppImage

- Fixture/setup: Run the checked-in AppRun CLI test; it builds stand-in binaries under a path with spaces.
- Action: bash packaging/linux/apprun-cli-test.sh
- Expected: Argument boundaries, MCP stdio round trip, exit status 37 and unchanged GUI entry behavior all pass the script assertions.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester:

## PR-4742: fix(macos): bundle photocraft-cli and resolve it from droplet scripts

- Fixture/setup: Frozen-SHA macOS bundle containing executable Contents/MacOS/photocraft-cli; synthetic droplet and image paths with spaces.
- Action: run each command separately:

  - `cargo test -p photocraft-engine --locked droplets_are_written_and_run -- --nocapture`

- Expected: Explicit PHOTOCRAFT_CLI wins, then supported /Applications and ~/Applications bundles, then PATH. Arguments remain separate; the intended action runs without standalone CLI installation; ordinary signature verification succeeds.
- Separate coverage / prerequisites: Use an authorized test installation; do not replace user's app. Installed-bundle resolution is BLOCKED if required bundle is absent. Finder dropping onto a plain shell shim is not promised; app wrapper is separate.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-7743: packaging: the portable zip holds the files, not a folder with the files in it

- Fixture/setup: Obtain an existing portable Windows ZIP built from the frozen integration; record its artifact identity.
- Action: List root archive entries and extract into a new empty directory; launch the app from the extracted root.
- Expected: Application files are directly at ZIP root without a redundant wrapping folder; extracted layout launches correctly.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester:

## PR-8985: Color panel and Color Picker: clip the round cursor to the colour field

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked field_marker_stays_inside_the_field -- --nocapture` — [crates/ui-egui/src/color_picker_ui.rs / field_marker_stays_inside_the_field](https://github.com/storytold/photocraft/blob/8e695caf885b529f01f482b3f0c90cdb34df1915/crates/ui-egui/src/color_picker_ui.rs)
  - `cargo test -p photocraft-ui-egui --locked color_field_cursor_stays_inside_the_field -- --nocapture` — [crates/ui-egui/src/panels.rs / color_field_cursor_stays_inside_the_field](https://github.com/storytold/photocraft/blob/8e695caf885b529f01f482b3f0c90cdb34df1915/crates/ui-egui/src/panels.rs)

- Expected behavior: field marker stays inside the field; color field cursor stays inside the field. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`field_marker_stays_inside_the_field` setup/action excerpt:

```rust
// Black in the default "h" mode: the marker is at the bottom-left corner.
        let h = picker("#000000");
        let mut field = None;
        let mut circles = Vec::new();
        for clipped in &h.output().shapes {
            match &clipped.shape {
                egui::Shape::Mesh(m) if m.vertices.len() == 33 * 33 => {
                    // The component slider is a 33 x 33 mesh too, drawn after the field.
                    field.get_or_insert(m.calc_bounds());
                }
                egui::Shape::Circle(c) => circles.push((c.center, c.radius, clipped.clip_rect)),
                _ => {}
```

Expected assertion context (overlapping windows merged):

```rust
assert!(!circles.is_empty(), "the marker is drawn");
        for (center, radius, clip) in circles {
            let visible = Rect::from_center_size(center, vec2(2.0 * radius, 2.0 * radius)).intersect(clip);
            assert!(field.contains_rect(visible), "marker {visible:?} leaves the field {field:?}");
        }
    }
```

`color_field_cursor_stays_inside_the_field` setup/action excerpt:

```rust
let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        // Black is s = 0, v = 0: the bottom-left corner.
        app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
        let h = field_harness(app);
        let mut field = None;
        let mut circles = Vec::new();
        for clipped in &h.output().shapes {
            match &clipped.shape {
                // The saturation/brightness field is the 17 x 17 mesh; the hue strip is a 25 x 25 grid.
                egui::Shape::Mesh(m) if m.vertices.len() == 17 * 17 => field = Some(m.calc_bounds()),
                egui::Shape::Circle(c) => circles.push((c.center, c.radius, clipped.clip_rect)),
                _ => {}
```

Expected assertion context (overlapping windows merged):

```rust
assert!(rings.iter().any(|(_, radius, _)| *radius == 5.0), "inner ring sits at the field's bottom-left corner");
        assert!(rings.iter().any(|(_, radius, _)| *radius == 6.5), "outer ring sits at the field's bottom-left corner");
        for (center, radius, clip) in rings {
            let visible = egui::Rect::from_center_size(center, vec2(2.0 * radius, 2.0 * radius)).intersect(clip);
            assert!(field.contains_rect(visible), "cursor ring {visible:?} leaves the field {field:?}");
        }
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9019: fix(xtask): reject unknown performance benchmark selections

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p xtask --locked unknown_bench_is_rejected_before_updating_the_baseline -- --nocapture` — [xtask/src/perf.rs / unknown_bench_is_rejected_before_updating_the_baseline](https://github.com/storytold/photocraft/blob/364c4be1c19728aefa601ee924bc7a0cd6a09789/xtask/src/perf.rs)

- Expected behavior: unknown bench is rejected before updating the baseline. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`unknown_bench_is_rejected_before_updating_the_baseline` setup/action excerpt:

```rust
let root = std::env::temp_dir().join(format!("photocraft-perf-unknown-bench-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("perf")).unwrap();
        std::fs::write(root.join(BUDGETS), "[[bench]]\nname = \"known\"\npackage = \"unused\"\nrequires = \"missing.rs\"\n").unwrap();
        let baseline = "{\"schema\":1,\"sentinel\":true}";
        std::fs::write(root.join(BASELINE), baseline).unwrap();
        let results = [
            run(&root, &["--bench", "unknown", "--reuse", "--update-baseline"]),
            run(&root, &["--bench", "known", "--bench", "unknown", "--quick", "--reuse", "--update-baseline"]),
        ];
        let after = std::fs::read_to_string(root.join(BASELINE)).unwrap();
        std::fs::remove_dir_all(root).unwrap();
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(result, Err(format!("perf: unknown bench `unknown` (see {BUDGETS})")));
        }
        assert_eq!(after, baseline, "an unknown bench must not update the baseline");
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9110: test: GPU tests honour WGPU_BACKEND, so they run on Windows with an Intel GPU (#4)

- Fixture/setup: Windows Intel host with functioning DX12; record OS, GPU and driver; frozen integration build.
- Action: run each command separately:

  - `cargo test -p photocraft-gpu --locked injected_fault_skips_the_gpu -- --nocapture`
  - `cargo test -p photocraft-ui-egui --locked layout_document_composites_on_the_gpu_like_the_cpu -- --nocapture`

- Expected: Every process honors WGPU_BACKEND=dx12. Example completes real composition; fault test renders before injecting loss and returns expected lost-device error; layout test executes CPU/GPU assertions.
- Separate coverage / prerequisites: Run cargo run --release -p photocraft-gpu --locked --example bench_stack_reuse -- --quick with WGPU_BACKEND=dx12. Record adapter/vendor/device/backend/driver and proof each comparison executed. Silent early returns are BLOCKED. Do not load the known-crashing Intel Vulkan backend.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9436: Startup: a failed launch says why in a dialog instead of closing silently (#7643)

- Fixture/setup: startup_report.rs terminal/automation/display combinations and synthetic renderer/error/log text.
- Action: run each command separately:

  - `cargo test -p photocraft --locked a_desktop_launch_asks_the_user_and_a_script_does_not -- --nocapture`
  - `cargo test -p photocraft --locked the_text_names_the_error_renderer_version_and_log -- --nocapture`
  - `cargo test -p photocraft --locked a_fallback_that_was_not_the_preference_is_explained -- --nocapture`
  - `cargo test -p photocraft --locked path_names_the_attached_log_file -- --nocapture`

- Expected: Desktop launch with display and without terminal/control asks via dialog; terminal/headless/automation do not hang. Text contains error, attempted renderer, version, actual log path and applicable fallback reason; no invented log path.
- Separate coverage / prerequisites: Native dialog subcase uses only an existing safe failure reproduction in temporary settings. If unavailable, mark BLOCKED; never force a GPU driver crash.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-2300: Scroll the viewport while outlining with Polygonal Lasso

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked polygon_edge_scroll_continues_with_button_up_and_keeps_vertices_fixed -- --nocapture` — [crates/ui-egui/src/polygon_lasso_tests.rs / polygon_edge_scroll_continues_with_button_up_and_keeps_vertices_fixed](https://github.com/storytold/photocraft/blob/ebf3777aec961d1815aa141d9987eaa645c2273f/crates/ui-egui/src/polygon_lasso_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked polygon_edge_scroll_places_the_next_vertex_in_the_panned_view_and_commits_once -- --nocapture` — [crates/ui-egui/src/polygon_lasso_tests.rs / polygon_edge_scroll_places_the_next_vertex_in_the_panned_view_and_commits_once](https://github.com/storytold/photocraft/blob/ebf3777aec961d1815aa141d9987eaa645c2273f/crates/ui-egui/src/polygon_lasso_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked polygon_edge_scroll_keeps_pixel_aspect_and_shift_constrained_segments -- --nocapture` — [crates/ui-egui/src/polygon_lasso_tests.rs / polygon_edge_scroll_keeps_pixel_aspect_and_shift_constrained_segments](https://github.com/storytold/photocraft/blob/ebf3777aec961d1815aa141d9987eaa645c2273f/crates/ui-egui/src/polygon_lasso_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked polygon_edge_scroll_pauses_for_dialogs_and_temporary_navigation -- --nocapture` — [crates/ui-egui/src/polygon_lasso_tests.rs / polygon_edge_scroll_pauses_for_dialogs_and_temporary_navigation](https://github.com/storytold/photocraft/blob/ebf3777aec961d1815aa141d9987eaa645c2273f/crates/ui-egui/src/polygon_lasso_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked polygon_edge_scroll_never_moves_an_inactive_pane_or_survives_a_document_or_tool_switch -- --nocapture` — [crates/ui-egui/src/polygon_lasso_tests.rs / polygon_edge_scroll_never_moves_an_inactive_pane_or_survives_a_document_or_tool_switch](https://github.com/storytold/photocraft/blob/ebf3777aec961d1815aa141d9987eaa645c2273f/crates/ui-egui/src/polygon_lasso_tests.rs)

- Expected behavior: polygon edge scroll continues with button up and keeps vertices fixed; polygon edge scroll places the next vertex in the panned view and commits once; polygon edge scroll keeps pixel aspect and shift constrained segments; polygon edge scroll pauses for dialogs and temporary navigation; polygon edge scroll never moves an inactive pane or survives a document or tool switch. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`polygon_edge_scroll_continues_with_button_up_and_keeps_vertices_fixed` setup/action excerpt:

```rust
for (dx, dy, flip, rotation) in [
        (-1.0, 0.0, false, 0.0),
        (1.0, 0.0, false, 0.0),
        (0.0, -1.0, false, 0.0),
        (0.0, 1.0, false, 0.0),
        (-1.0, -1.0, false, 0.0),
        (1.0, -1.0, false, 0.0),
        (-1.0, 1.0, false, 0.0),
        (1.0, 1.0, false, 0.0),
        (1.0, 0.0, true, 0.0),
        (1.0, 0.0, false, 90.0),
    ] {
```

Expected assertion context (overlapping windows merged):

```rust
assert!(movement.dot(direction) > 0.1, "edge ({dx},{dy}), flip={flip}, rotation={rotation}: {movement:?}");
        assert!((movement.x * direction.y - movement.y * direction.x).abs() < 0.001, "scroll follows the displayed axes");
        assert!((next[0] - first[0]).hypot(next[1] - first[1]) > 0.1, "stationary pointer keeps scrolling");
        assert_eq!(h.state().ui.polygon, points, "view motion never adds or moves vertices");
        assert_eq!(steps(&h), before);
        assert!(selection(&h).is_none());
        assert!(!h.ctx.input(|i| i.pointer.any_down()));
        assert!(h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay.is_zero());
        h.event(egui::Event::PointerMoved(xf.rect.center()));
        h.run_steps(1);
        let stopped = h.state().ui.views[0].center;
        assert_eq!(h.state().ui.views[0].center, stopped, "returning to the interior stops scrolling");
    }

    let mut h = zoomed_polygon();
    assert!((center[0] - (400.0 - xf.rect.width() / (2.0 * xf.zoom))).abs() < 0.001, "Overscroll off stops at the image's right edge");
    h.run_steps(10);
    assert_eq!(h.state().ui.views[0].center, center, "a long edge hold stays clamped");
    assert_eq!(h.state().ui.polygon, points);
    assert_eq!(steps(&h), before);
}
```

`polygon_edge_scroll_places_the_next_vertex_in_the_panned_view_and_commits_once` setup/action excerpt:

```rust
let mut h = zoomed_polygon();
    let before = steps(&h);
    let original = ViewXform::active(h.state()).unwrap();
    h.event(egui::Event::PointerMoved(original.rect.right_center() - vec2(4.0, 0.0)));
    h.run_steps(10);
    let panned = ViewXform::active(h.state()).unwrap();
    assert!(panned.center[0] > original.center[0], "the zoomed image pans toward the right edge");
    let at = panned.rect.center() + vec2(60.0, 0.0);
    let expected = panned.to_doc(at);
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(20);
    button(&mut h, at, true, Modifiers::NONE);
```

Expected assertion context (overlapping windows merged):

```rust
assert!(panned.center[0] > original.center[0], "the zoomed image pans toward the right edge");
    let at = panned.rect.center() + vec2(60.0, 0.0);
    let expected = panned.to_doc(at);
    h.event(egui::Event::PointerMoved(at));
    assert_eq!(h.state().ui.polygon[1], expected, "the next click uses the updated view transform");
    h.run_steps(30);
    click(&mut h, expected[0] as f32, 190.0, Modifiers::NONE);
    h.key_press(egui::Key::Enter);
    assert!(h.state().ui.polygon.is_empty());
    assert!(selection(&h).is_some_and(|r| near(r, 170, 130, expected[0].ceil() as i32, 190)), "{:?}", selection(&h));
    assert_eq!(steps(&h), before + 1);
    h.event(egui::Event::PointerMoved(original.rect.right_center()));
    h.run_steps(4);
    assert_eq!(h.state().ui.views[0].center, panned.center, "a finished polygon never scrolls");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(selection(&h).is_none(), "one undo removes the completed selection");
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-5010: Layer Style: Photoshop's layout (three columns, OK/Cancel/New Style/Preview on the right, one-line rows) [depends on #4938]

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked dragging_the_title_bar_moves_the_dialog -- --nocapture` — [crates/ui-egui/src/dialogs.rs / dragging_the_title_bar_moves_the_dialog](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/dialogs.rs)
  - `cargo test -p photocraft-ui-egui --locked layer_style_keeps_one_size_on_every_page -- --nocapture` — [crates/ui-egui/src/dialogs.rs / layer_style_keeps_one_size_on_every_page](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/dialogs.rs)
  - `cargo test -p photocraft-ui-egui --locked layer_style_fits_a_short_screen -- --nocapture` — [crates/ui-egui/src/dialogs.rs / layer_style_fits_a_short_screen](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/dialogs.rs)
  - `cargo test -p photocraft-ui-egui --locked layer_style_is_laid_out_like_photoshops -- --nocapture` — [crates/ui-egui/src/dialogs.rs / layer_style_is_laid_out_like_photoshops](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/dialogs.rs)
  - `cargo test -p photocraft-ui-egui --locked layer_style_ok_cancel_and_enter -- --nocapture` — [crates/ui-egui/src/dialogs.rs / layer_style_ok_cancel_and_enter](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/dialogs.rs)
  - `cargo test -p photocraft-ui-egui --locked layer_style_bin_deletes_the_selected_effect -- --nocapture` — [crates/ui-egui/src/dialogs.rs / layer_style_bin_deletes_the_selected_effect](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/dialogs.rs)
  - `cargo test -p photocraft-ui-egui --locked every_effect_row_shows_a_checkbox -- --nocapture` — [crates/ui-egui/src/layer_style.rs / every_effect_row_shows_a_checkbox](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/layer_style.rs)
  - `cargo test -p photocraft-ui-egui --locked move_instance_reorders_within_its_kind -- --nocapture` — [crates/ui-egui/src/layer_style.rs / move_instance_reorders_within_its_kind](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/layer_style.rs)
  - `cargo test -p photocraft-ui-egui --locked fx_menu_adds_or_shows_an_effect -- --nocapture` — [crates/ui-egui/src/layer_style.rs / fx_menu_adds_or_shows_an_effect](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/layer_style.rs)
  - `cargo test -p photocraft-ui-egui --locked pending_style_is_the_dialogs_and_leaves_the_document_alone -- --nocapture` — [crates/ui-egui/src/layer_style.rs / pending_style_is_the_dialogs_and_leaves_the_document_alone](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/layer_style.rs)
  - `cargo test -p photocraft-ui-egui --locked body_keeps_invalid_params_and_renders_without_panicking -- --nocapture` — [crates/ui-egui/src/layer_style.rs / body_keeps_invalid_params_and_renders_without_panicking](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/layer_style.rs)
  - `cargo test -p photocraft-ui-egui --locked blending_page_renders_for_every_mode_and_odd_fields -- --nocapture` — [crates/ui-egui/src/layer_style.rs / blending_page_renders_for_every_mode_and_odd_fields](https://github.com/storytold/photocraft/blob/cc865221894e21f846d92b721cce7421ea338363/crates/ui-egui/src/layer_style.rs)

- Expected behavior: dragging the title bar moves the dialog; layer style keeps one size on every page; layer style fits a short screen; layer style is laid out like photoshops; layer style ok cancel and enter; layer style bin deletes the selected effect; every effect row shows a checkbox; move instance reorders within its kind; fx menu adds or shows an effect; pending style is the dialogs and leaves the document alone; body keeps invalid params and renders without panicking; blending page renders for every mode and odd fields. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`dragging_the_title_bar_moves_the_dialog` setup/action excerpt:

```rust
use egui_kittest::{Harness, kittest::Queryable};

        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(1400.0, 1100.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
        harness.state_mut().ui.open_dialog(DialogKind::LayerStyle, serde_json::Map::new());
        harness.run_steps(3);
        let before = harness.get_by_label("Layer Style").rect();

        // Grab the title text itself: it must move the dialog, not select the text.
        let from = before.center();
        harness.hover_at(from);
```

Expected assertion context (overlapping windows merged):

```rust
assert!((moved - egui::vec2(-120.0, 80.0)).length() < 1.0, "dialog moved by {moved:?}");
        assert_eq!(harness.state().ui.dialogs.len(), 1, "dragging must not close the dialog");
    }
```

`layer_style_keeps_one_size_on_every_page` setup/action excerpt:

```rust
let mut h = layer_style_on_bevel(egui::vec2(1400.0, 900.0));
        let size = frame(&h).size();
        let dialog = h.state().ui.dialogs[0].id;
        for page in ["blendingOptions", "styles"] {
            h.state_mut().ui.dialog_mut(dialog).expect("still open").fields.insert("selected".into(), json!(page));
            h.run_steps(3);
            assert_eq!(frame(&h).size(), size, "{page}");
        }
        let grip = egui::Id::new(("dialog", dialog)).with("resize").with("__resize_corner");
        assert!(h.ctx.read_response(grip).is_none(), "no resize grip");
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(frame(&h).size(), size, "{page}");
        }
        let grip = egui::Id::new(("dialog", dialog)).with("resize").with("__resize_corner");
        assert!(h.ctx.read_response(grip).is_none(), "no resize grip");
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-5515: Selecting subject now automatically applies to selected layer, and us…

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked select_subject_samples_the_selected_layer_else_the_composite -- --nocapture` — [crates/engine/src/smartselect_cmds.rs / select_subject_samples_the_selected_layer_else_the_composite](https://github.com/storytold/photocraft/blob/375d4894a119b6cd5933d53e8fe7f506d2783392/crates/engine/src/smartselect_cmds.rs)
  - `cargo test -p photocraft-engine --locked select_subject_repeat_reuses_the_mask_until_pixels_change -- --nocapture` — [crates/engine/src/smartselect_cmds.rs / select_subject_repeat_reuses_the_mask_until_pixels_change](https://github.com/storytold/photocraft/blob/375d4894a119b6cd5933d53e8fe7f506d2783392/crates/engine/src/smartselect_cmds.rs)

- Expected behavior: select subject samples the selected layer else the composite; select subject repeat reuses the mask until pixels change. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`select_subject_samples_the_selected_layer_else_the_composite` setup/action excerpt:

```rust
let (mut s, inside) = disc(8);
        // A layer above holding only a small square in the corner: the disc is not on it.
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("paint.pencil", json!({"points": [[10, 10], [30, 10]], "size": 3, "color": "#000000"})).unwrap();
        s.execute("select.subject", json!({})).unwrap();
        assert!(cov(&s, 100, 80) < 0.5, "the disc on the layer below was selected");
        s.execute("select.deselect", json!({})).unwrap();
        // No layer selected: the whole image, so the disc is found.
        s.execute("select.deselectLayers", json!({})).unwrap();
        s.execute("select.subject", json!({})).unwrap();
        assert!(sel_iou(&s, 200, 160, &inside) > 0.9);
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!(cov(&s, 100, 80) < 0.5, "the disc on the layer below was selected");
        s.execute("select.deselect", json!({})).unwrap();
        // No layer selected: the whole image, so the disc is found.
        s.execute("select.deselectLayers", json!({})).unwrap();
        assert!(sel_iou(&s, 200, 160, &inside) > 0.9);
    }
```

`select_subject_repeat_reuses_the_mask_until_pixels_change` setup/action excerpt:

```rust
let (mut s, _) = disc(8);
        let sel = |s: &Session| {
            let mut v = Vec::new();
            s.active().unwrap().doc.selection.as_ref().unwrap().read_region_into(Rect::new(0, 0, 200, 160), &mut v);
            v
        };
        let cached = |s: &Session| s.subject_cache.region.clone().unwrap();
        s.execute("select.subject", json!({})).unwrap();
        let (first, mask) = (sel(&s), cached(&s));
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("select.subject", json!({})).unwrap();
        assert!(std::sync::Arc::ptr_eq(&mask, &cached(&s)), "the repeat recomputed the mask");
```

Expected assertion context (overlapping windows merged):

```rust
assert!(std::sync::Arc::ptr_eq(&mask, &cached(&s)), "the repeat recomputed the mask");
        assert_eq!(sel(&s), first);
        // Another sampled target (the composite rather than the layer) is a miss.
        s.execute("select.subject", json!({"sampleAllLayers": true})).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&mask, &cached(&s)));
        // A pixel edit drops the entry; the next run recomputes it.
        s.edit("paint", |doc, _| {
            let bg = doc.layers[0].surface_mut().unwrap();
        assert!(s.subject_cache.region.is_none());
        s.execute("select.subject", json!({})).unwrap();
        assert_eq!(sel(&s), first);
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-5650: Guides: ⌥ turns a dragged guide; dropping one on either ruler deletes it (#5034)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked move_guide_turns_a_guide -- --nocapture` — [crates/engine/src/edit_cmds.rs / move_guide_turns_a_guide](https://github.com/storytold/photocraft/blob/4b1bd6fb6d264f885cb9caa3b8a6578ee527bd7b/crates/engine/src/edit_cmds.rs)
  - `cargo test -p photocraft-ui-egui --locked layer_panel_hover_outlines_layer_on_canvas -- --nocapture` — [crates/ui-egui/src/move_hover.rs / layer_panel_hover_outlines_layer_on_canvas](https://github.com/storytold/photocraft/blob/4b1bd6fb6d264f885cb9caa3b8a6578ee527bd7b/crates/ui-egui/src/move_hover.rs)
  - `cargo test -p photocraft-ui-egui --locked guide_drags_create_move_and_delete -- --nocapture` — [crates/ui-egui/src/rulers.rs / guide_drags_create_move_and_delete](https://github.com/storytold/photocraft/blob/4b1bd6fb6d264f885cb9caa3b8a6578ee527bd7b/crates/ui-egui/src/rulers.rs)
  - `cargo test -p photocraft-ui-egui --locked alt_turns_a_guide_dragged_out_of_a_ruler -- --nocapture` — [crates/ui-egui/src/rulers.rs / alt_turns_a_guide_dragged_out_of_a_ruler](https://github.com/storytold/photocraft/blob/4b1bd6fb6d264f885cb9caa3b8a6578ee527bd7b/crates/ui-egui/src/rulers.rs)
  - `cargo test -p photocraft-ui-egui --locked alt_turns_a_guide_moved_with_the_move_tool -- --nocapture` — [crates/ui-egui/src/rulers.rs / alt_turns_a_guide_moved_with_the_move_tool](https://github.com/storytold/photocraft/blob/4b1bd6fb6d264f885cb9caa3b8a6578ee527bd7b/crates/ui-egui/src/rulers.rs)
  - `cargo test -p photocraft-ui-egui --locked a_guide_dropped_on_either_ruler_is_deleted -- --nocapture` — [crates/ui-egui/src/rulers.rs / a_guide_dropped_on_either_ruler_is_deleted](https://github.com/storytold/photocraft/blob/4b1bd6fb6d264f885cb9caa3b8a6578ee527bd7b/crates/ui-egui/src/rulers.rs)

- Expected behavior: move guide turns a guide; layer panel hover outlines layer on canvas; guide drags create move and delete; alt turns a guide dragged out of a ruler; alt turns a guide moved with the move tool; a guide dropped on either ruler is deleted. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`move_guide_turns_a_guide` setup/action excerpt:

```rust
let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "horizontal", "position": 30, "color": "magenta"})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "horizontal", "position": 70})).unwrap();
        s.execute("view.moveGuide", json!({"orientation": "horizontal", "index": 0, "position": "50%", "toOrientation": "vertical"})).unwrap();
        let g = &s.active().unwrap().doc.guides;
        assert_eq!((g.horizontal.clone(), g.vertical.clone()), (vec![70.0], vec![100.0]), "50% of the width");
        assert_eq!(g.color(true, 100.0), Some([255, 0, 255]), "the colour moves with the guide");
        assert_eq!(g.color(false, 30.0), None);
        // The same orientation is a plain move.
        s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 20, "toOrientation": "vertical"})).unwrap();
        assert_eq!(s.active().unwrap().doc.guides.vertical, vec![20.0]);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!((g.horizontal.clone(), g.vertical.clone()), (vec![70.0], vec![100.0]), "50% of the width");
        assert_eq!(g.color(true, 100.0), Some([255, 0, 255]), "the colour moves with the guide");
        assert_eq!(g.color(false, 30.0), None);
        // The same orientation is a plain move.
        s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 20, "toOrientation": "vertical"})).unwrap();
        assert_eq!(s.active().unwrap().doc.guides.vertical, vec![20.0]);
        s.undo();
        s.undo();
        let g = &s.active().unwrap().doc.guides;
        assert_eq!((g.horizontal.clone(), g.vertical.clone()), (vec![30.0, 70.0], vec![]), "one undo step per move");
        // Bad params are refused and leave the guides alone.
        assert!(s.execute("view.moveGuide", json!({"orientation": "horizontal", "index": 0, "position": 5, "toOrientation": "diagonal"})).is_err());
        assert!(s.execute("view.moveGuide", json!({"orientation": "horizontal", "index": 0, "position": 5, "toOrientation": 1})).is_err());
        assert!(s.execute("view.newGuide", json!({"orientation": "horizontal", "position": 5, "toOrientation": "vertical"})).is_err());
        assert!(s.execute("view.moveGuide", json!({"orientation": "horizontal", "index": 9, "position": 5, "toOrientation": "vertical"})).is_err());
        let g = &s.active().unwrap().doc.guides;
        assert_eq!((g.horizontal.clone(), g.vertical.clone()), (vec![30.0, 70.0], vec![]));
    }
```

`layer_panel_hover_outlines_layer_on_canvas` setup/action excerpt:

```rust
let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, a| {
                doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(8, 8, 24, 24), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        let target_layer = app.session.active().unwrap().active_layer.unwrap();
        let a = Rect::new(8, 8, 24, 24);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(active_outline(&mut app, None, false, false), None);
        app.hover_layer = Some(target_layer);
        assert_eq!(active_outline(&mut app, None, false, false), Some(a));

        // When a drag or transform is in progress, no outline:
        app.guide_drag = Some(crate::rulers::GuideDrag { vertical: false, index: None, pos: 10.0, ..Default::default() });
        assert_eq!(active_outline(&mut app, None, false, false), None);
        app.guide_drag = None;

        // When rollover preference is disabled, no outline:
        assert_eq!(active_outline(&mut app, None, false, false), None);
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-6026: Marquee: Fixed Size selects exactly the entered width and height (fixes #6015)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked fixed_size_px_is_exactly_the_size -- --nocapture` — [crates/ui-egui/src/canvas.rs / fixed_size_px_is_exactly_the_size](https://github.com/storytold/photocraft/blob/248ba660576f4896f69cbb6639353e2f1d295271/crates/ui-egui/src/canvas.rs)
  - `cargo test -p photocraft-ui-egui --locked fixed_size_selects_exactly_the_entered_size -- --nocapture` — [crates/ui-egui/src/marquee_tests.rs / fixed_size_selects_exactly_the_entered_size](https://github.com/storytold/photocraft/blob/248ba660576f4896f69cbb6639353e2f1d295271/crates/ui-egui/src/marquee_tests.rs)

- Expected behavior: fixed size px is exactly the size; fixed size selects exactly the entered size. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`fixed_size_px_is_exactly_the_size` setup/action excerpt:

```rust
use super::fixed_size_px;
        // Down-right from (10.4, 20.6): 10 × 10 from the closest pixel edges.
        assert_eq!(fixed_size_px([10.4, 20.6], [20.4, 30.6], 10.0, 10.0), [10.0, 21.0, 20.0, 31.0]);
        // Up-left: the far corner is the low one; still exactly the size.
        assert_eq!(fixed_size_px([10.4, 20.6], [-53.6, -11.4], 64.0, 32.0), [-54.0, -11.0, 10.0, 21.0]);
        // 1 × 1 is one pixel, and a fractional size rounds to whole pixels.
        assert_eq!(fixed_size_px([3.5, 3.5], [4.5, 4.5], 1.0, 1.0), [4.0, 4.0, 5.0, 5.0]);
        assert_eq!(fixed_size_px([0.0, 0.0], [2.6, 2.4], 2.6, 2.4), [0.0, 0.0, 3.0, 2.0]);
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(fixed_size_px([10.4, 20.6], [20.4, 30.6], 10.0, 10.0), [10.0, 21.0, 20.0, 31.0]);
        // Up-left: the far corner is the low one; still exactly the size.
        assert_eq!(fixed_size_px([10.4, 20.6], [-53.6, -11.4], 64.0, 32.0), [-54.0, -11.0, 10.0, 21.0]);
        // 1 × 1 is one pixel, and a fractional size rounds to whole pixels.
        assert_eq!(fixed_size_px([3.5, 3.5], [4.5, 4.5], 1.0, 1.0), [4.0, 4.0, 5.0, 5.0]);
        assert_eq!(fixed_size_px([0.0, 0.0], [2.6, 2.4], 2.6, 2.4), [0.0, 0.0, 3.0, 2.0]);
    }
```

`fixed_size_selects_exactly_the_entered_size` setup/action excerpt:

```rust
for tool in [Tool::RectMarquee, Tool::EllipseMarquee] {
        for (w, ht) in [(10, 10), (1, 1), (64, 32)] {
            let mut h = harness(tool);
            let o = &mut h.state_mut().ui.tool_options;
            o.marquee_style = "fixedSize".into();
            (o.marquee_width, o.marquee_height) = (w as f32, ht as f32);
            press_at(&mut h, 50.4, 60.6, Modifiers::NONE);
            assert!(readout(&h, w, ht), "{tool:?} {w} × {ht}: the readout shows the entered size");
            release_at(&mut h, 90.0, 90.0, Modifiers::NONE);
            assert_eq!(selection(&h), Rect::new(50, 61, 50 + w, 61 + ht), "{tool:?} {w} × {ht}: dragged");

            // A click: the button goes down and up at one point.
```

Expected assertion context (overlapping windows merged):

```rust
assert!(readout(&h, w, ht), "{tool:?} {w} × {ht}: the readout shows the entered size");
            release_at(&mut h, 90.0, 90.0, Modifiers::NONE);
            assert_eq!(selection(&h), Rect::new(50, 61, 50 + w, 61 + ht), "{tool:?} {w} × {ht}: dragged");

            // A click: the button goes down and up at one point.
            h.state_mut().run("select.deselect", json!({})).unwrap();
            assert_eq!(selection(&h), Rect::new(121, 100, 121 + w, 100 + ht), "{tool:?} {w} × {ht}: clicked");
        }
    }
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-6236: Shift+Alt+right-click opens Photoshop's HUD Color Picker (#6098)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked shift_alt_right_press_opens_the_hud_not_the_picker -- --nocapture` — [crates/ui-egui/src/color_hud.rs / shift_alt_right_press_opens_the_hud_not_the_picker](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/color_hud.rs)
  - `cargo test -p photocraft-ui-egui --locked quick_selections_shift_alt_right_opens_only_the_selection_menu -- --nocapture` — [crates/ui-egui/src/color_hud.rs / quick_selections_shift_alt_right_opens_only_the_selection_menu](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/color_hud.rs)
  - `cargo test -p photocraft-ui-egui --locked a_hud_clamped_under_the_still_pointer_picks_nothing_until_it_moves -- --nocapture` — [crates/ui-egui/src/color_hud.rs / a_hud_clamped_under_the_still_pointer_picks_nothing_until_it_moves](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/color_hud.rs)
  - `cargo test -p photocraft-ui-egui --locked dragging_over_the_hud_picks_the_foreground_live_and_release_keeps_it -- --nocapture` — [crates/ui-egui/src/color_hud.rs / dragging_over_the_hud_picks_the_foreground_live_and_release_keeps_it](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/color_hud.rs)
  - `cargo test -p photocraft-ui-egui --locked escape_restores_the_starting_colour -- --nocapture` — [crates/ui-egui/src/color_hud.rs / escape_restores_the_starting_colour](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/color_hud.rs)
  - `cargo test -p photocraft-ui-egui --locked plain_right_click_still_opens_the_picker_and_alt_drag_still_resizes -- --nocapture` — [crates/ui-egui/src/color_hud.rs / plain_right_click_still_opens_the_picker_and_alt_drag_still_resizes](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/color_hud.rs)
  - `cargo test -p photocraft-ui-egui --locked shift_alt_right_drag_does_not_resize_the_brush -- --nocapture` — [crates/ui-egui/src/color_hud.rs / shift_alt_right_drag_does_not_resize_the_brush](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/color_hud.rs)
  - `cargo test -p photocraft-ui-egui --locked the_control_channel_opens_and_closes_the_hud -- --nocapture` — [crates/ui-egui/src/color_hud.rs / the_control_channel_opens_and_closes_the_hud](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/color_hud.rs)
  - `cargo test -p photocraft-ui-egui --locked hue_strip_mesh_indices_stay_within_its_rows -- --nocapture` — [crates/ui-egui/src/color_hud.rs / hue_strip_mesh_indices_stay_within_its_rows](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/color_hud.rs)
  - `cargo test -p photocraft-ui-egui --locked pick_at_maps_the_rects_and_ignores_the_gaps -- --nocapture` — [crates/ui-egui/src/color_hud.rs / pick_at_maps_the_rects_and_ignores_the_gaps](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/color_hud.rs)
  - `cargo test -p photocraft-ui-egui --locked shift_alt_right_through_the_control_channel_respects_the_erase_preference -- --nocapture` — [crates/ui-egui/src/paint_mouse.rs / shift_alt_right_through_the_control_channel_respects_the_erase_preference](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/paint_mouse.rs)
  - `cargo test -p photocraft-ui-egui --locked a_control_channel_up_closes_the_hud_without_repeating_its_modifiers -- --nocapture` — [crates/ui-egui/src/paint_mouse.rs / a_control_channel_up_closes_the_hud_without_repeating_its_modifiers](https://github.com/storytold/photocraft/blob/ce546dd4b5451a17b843ef1e129bdb40fbfe3160/crates/ui-egui/src/paint_mouse.rs)

- Expected behavior: shift alt right press opens the hud not the picker; quick selections shift alt right opens only the selection menu; a hud clamped under the still pointer picks nothing until it moves; dragging over the hud picks the foreground live and release keeps it; escape restores the starting colour; plain right click still opens the picker and alt drag still resizes; shift alt right drag does not resize the brush; the control channel opens and closes the hud; hue strip mesh indices stay within its rows; pick at maps the rects and ignores the gaps; shift alt right through the control channel respects the erase preference; a control channel up closes the hud without repeating its modifiers. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`shift_alt_right_press_opens_the_hud_not_the_picker` setup/action excerpt:

```rust
let mut h = harness();
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        let hud = h.state().ui.color_hud.expect("the HUD opened");
        assert!(hud.hold);
        assert_eq!(hud.start, [0.0, 0.0, 1.0, 1.0], "seeded from the foreground");
        assert_eq!(h.state().ui.brush_picker, None, "the Brush Preset picker stays closed");
        // The HUD shows under the pointer, on screen.
        let (strip, square) = rects(&h);
        assert!(strip.height() > 100.0 && square.width() > 100.0);
```

Expected assertion context (overlapping windows merged):

```rust
assert!(hud.hold);
        assert_eq!(hud.start, [0.0, 0.0, 1.0, 1.0], "seeded from the foreground");
        assert_eq!(h.state().ui.brush_picker, None, "the Brush Preset picker stays closed");
        // The HUD shows under the pointer, on screen.
        let (strip, square) = rects(&h);
        assert!(strip.height() > 100.0 && square.width() > 100.0);
        assert!(square.right() < h.ctx.content_rect().right());
        // Nothing painted.
        assert!(!h.state().session.journal.iter().any(|(id, _)| id == "paint.stroke"));
    }
```

`quick_selections_shift_alt_right_opens_only_the_selection_menu` setup/action excerpt:

```rust
let mut h = harness();
        h.state_mut().ui.tool = Tool::QuickSelection;
        h.run_steps(1);
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        assert_eq!(h.state().ui.color_hud, None, "no HUD for Quick Selection");
        assert_eq!(h.state().ui.brush_picker, None, "and no Brush Preset picker");
        button(&mut h, c, false, SHIFT_ALT);
        assert_eq!(h.state().ui.color_hud, None, "the release does not open it either");
        assert!(h.state().ui.canvas_tool_menu.is_some(), "the selection menu opens, once");
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(h.state().ui.color_hud, None, "no HUD for Quick Selection");
        assert_eq!(h.state().ui.brush_picker, None, "and no Brush Preset picker");
        button(&mut h, c, false, SHIFT_ALT);
        assert_eq!(h.state().ui.color_hud, None, "the release does not open it either");
        assert!(h.state().ui.canvas_tool_menu.is_some(), "the selection menu opens, once");
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-6510: Automation: resolve configured root aliases for absolute paths (#6426)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-automation --locked canonical_windows_prefixes_match_without_ambient_resolution -- --nocapture` — [crates/automation/src/workspace/tests.rs / canonical_windows_prefixes_match_without_ambient_resolution](https://github.com/storytold/photocraft/blob/8bb0a5e67474d36ae08d565251edb1ae780dd044/crates/automation/src/workspace/tests.rs)
  - `cargo test -p photocraft-automation --locked windows_root_case_variants_keep_io_inside_the_grant -- --nocapture` — [crates/automation/src/workspace/tests.rs / windows_root_case_variants_keep_io_inside_the_grant](https://github.com/storytold/photocraft/blob/8bb0a5e67474d36ae08d565251edb1ae780dd044/crates/automation/src/workspace/tests.rs)
  - `cargo test -p photocraft-automation --locked unix_root_prefixes_remain_case_sensitive -- --nocapture` — [crates/automation/src/workspace/tests.rs / unix_root_prefixes_remain_case_sensitive](https://github.com/storytold/photocraft/blob/8bb0a5e67474d36ae08d565251edb1ae780dd044/crates/automation/src/workspace/tests.rs)
  - `cargo test -p photocraft-automation --locked configured_parent_components_accept_resolved_absolute_paths -- --nocapture` — [crates/automation/src/workspace/tests.rs / configured_parent_components_accept_resolved_absolute_paths](https://github.com/storytold/photocraft/blob/8bb0a5e67474d36ae08d565251edb1ae780dd044/crates/automation/src/workspace/tests.rs)
  - `cargo test -p photocraft-automation --locked relative_configured_root_accepts_resolved_absolute_paths -- --nocapture` — [crates/automation/src/workspace/tests.rs / relative_configured_root_accepts_resolved_absolute_paths](https://github.com/storytold/photocraft/blob/8bb0a5e67474d36ae08d565251edb1ae780dd044/crates/automation/src/workspace/tests.rs)
  - `cargo test -p photocraft-automation --locked trusted_root_names_are_not_validated_as_relative_request_names -- --nocapture` — [crates/automation/src/workspace/tests.rs / trusted_root_names_are_not_validated_as_relative_request_names](https://github.com/storytold/photocraft/blob/8bb0a5e67474d36ae08d565251edb1ae780dd044/crates/automation/src/workspace/tests.rs)
  - `cargo test -p photocraft-automation --locked linked_root_accepts_both_spellings_and_new_nested_exports -- --nocapture` — [crates/automation/src/workspace/tests.rs / linked_root_accepts_both_spellings_and_new_nested_exports](https://github.com/storytold/photocraft/blob/8bb0a5e67474d36ae08d565251edb1ae780dd044/crates/automation/src/workspace/tests.rs)
  - `cargo test -p photocraft-automation --locked root_alias_tests_work_with_an_aliased_windows_temp -- --nocapture` — [crates/automation/src/workspace/tests.rs / root_alias_tests_work_with_an_aliased_windows_temp](https://github.com/storytold/photocraft/blob/8bb0a5e67474d36ae08d565251edb1ae780dd044/crates/automation/src/workspace/tests.rs)
  - `cargo test -p photocraft-automation --locked ungranted_absolute_aliases_and_link_escapes_are_refused -- --nocapture` — [crates/automation/src/workspace/tests.rs / ungranted_absolute_aliases_and_link_escapes_are_refused](https://github.com/storytold/photocraft/blob/8bb0a5e67474d36ae08d565251edb1ae780dd044/crates/automation/src/workspace/tests.rs)
  - `cargo test -p photocraft-automation --locked retargeting_root_alias_does_not_regrant_the_held_authority -- --nocapture` — [crates/automation/src/workspace/tests.rs / retargeting_root_alias_does_not_regrant_the_held_authority](https://github.com/storytold/photocraft/blob/8bb0a5e67474d36ae08d565251edb1ae780dd044/crates/automation/src/workspace/tests.rs)

- Expected behavior: canonical windows prefixes match without ambient resolution; windows root case variants keep io inside the grant; unix root prefixes remain case sensitive; configured parent components accept resolved absolute paths; relative configured root accepts resolved absolute paths; trusted root names are not validated as relative request names; linked root accepts both spellings and new nested exports; root alias tests work with an aliased windows temp; ungranted absolute aliases and link escapes are refused; retargeting root alias does not regrant the held authority. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`canonical_windows_prefixes_match_without_ambient_resolution` setup/action excerpt:

```rust
for (request, root) in [
        (r"c:\root\new.bin", r"\\?\C:\root"),
        (r"\\?\C:\root\new.bin", r"C:\root"),
        (r"\\server\share\root\new.bin", r"\\?\UNC\SERVER\SHARE\root"),
        (r"c:\ROOT\new.bin", r"\\?\C:\root"),
        (r"\\?\C:\ROOT\new.bin", r"c:\root"),
        (r"\\server\share\ROOT\new.bin", r"\\?\UNC\SERVER\SHARE\root"),
    ] {
        let remaining = path_beneath_root(Path::new(request), Path::new(root)).unwrap();
        assert_eq!(remaining.as_path(), Path::new("new.bin"));
    }
    for request in [r"C:\root-sibling\new.bin", r"D:\root\new.bin", r"\\.\C:\root\new.bin", r"\\other\share\root\new.bin"] {
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(remaining.as_path(), Path::new("new.bin"));
    }
    for request in [r"C:\root-sibling\new.bin", r"D:\root\new.bin", r"\\.\C:\root\new.bin", r"\\other\share\root\new.bin"] {
        assert!(path_beneath_root(Path::new(request), Path::new(r"\\?\C:\root")).is_none());
    }
}
```

`windows_root_case_variants_keep_io_inside_the_grant` setup/action excerpt:

```rust
let (inside, outside, workspace) = roots("root-case");
    let configured = PathBuf::from(inside.to_string_lossy().to_ascii_uppercase());
    let canonical = PathBuf::from(std::fs::canonicalize(&inside).unwrap().to_string_lossy().to_ascii_uppercase());
    for spelling in [&configured, &canonical] {
        let path = spelling.join("new.bin");
        workspace.write(&path.to_string_lossy(), b"inside").unwrap();
        assert_eq!(workspace.read(&path.to_string_lossy()).unwrap(), b"inside");
        assert!(workspace.write(&spelling.join("..").join("outside").join("blocked.bin").to_string_lossy(), b"no").is_err());
    }
    assert_eq!(std::fs::read(inside.join("new.bin")).unwrap(), b"inside");
    assert!(workspace.write(&configured.with_file_name("INSIDE-SIBLING").join("blocked.bin").to_string_lossy(), b"no").is_err());
    assert!(!outside.join("blocked.bin").exists());
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(workspace.read(&path.to_string_lossy()).unwrap(), b"inside");
        assert!(workspace.write(&spelling.join("..").join("outside").join("blocked.bin").to_string_lossy(), b"no").is_err());
    }
    assert_eq!(std::fs::read(inside.join("new.bin")).unwrap(), b"inside");
    assert!(workspace.write(&configured.with_file_name("INSIDE-SIBLING").join("blocked.bin").to_string_lossy(), b"no").is_err());
    assert!(!outside.join("blocked.bin").exists());
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-7648: Pen: Path Operations menu, as in Photoshop (#1397)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked merge_shape_components_bakes_a_paths_operations -- --nocapture` — [crates/engine/src/cmd/path_edit_cmds.rs / merge_shape_components_bakes_a_paths_operations](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/engine/src/cmd/path_edit_cmds.rs)
  - `cargo test -p photocraft-engine --locked merge_shape_components_on_a_shape_layer_rerenders_it -- --nocapture` — [crates/engine/src/cmd/path_edit_cmds.rs / merge_shape_components_on_a_shape_layer_rerenders_it](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/engine/src/cmd/path_edit_cmds.rs)
  - `cargo test -p photocraft-engine --locked merging_a_vector_mask_to_nothing_keeps_what_it_shows -- --nocapture` — [crates/engine/src/cmd/path_edit_cmds.rs / merging_a_vector_mask_to_nothing_keeps_what_it_shows](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/engine/src/cmd/path_edit_cmds.rs)
  - `cargo test -p photocraft-engine --locked set_operation_changes_existing_components -- --nocapture` — [crates/engine/src/cmd/path_edit_cmds.rs / set_operation_changes_existing_components](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/engine/src/cmd/path_edit_cmds.rs)
  - `cargo test -p photocraft-engine --locked set_operation_on_a_joined_subpath_changes_its_component -- --nocapture` — [crates/engine/src/cmd/path_edit_cmds.rs / set_operation_on_a_joined_subpath_changes_its_component](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/engine/src/cmd/path_edit_cmds.rs)
  - `cargo test -p photocraft-engine --locked set_operation_leaves_the_first_component_combining -- --nocapture` — [crates/engine/src/cmd/path_edit_cmds.rs / set_operation_leaves_the_first_component_combining](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/engine/src/cmd/path_edit_cmds.rs)
  - `cargo test -p photocraft-engine --locked bad_set_operation_params_change_nothing -- --nocapture` — [crates/engine/src/cmd/path_edit_cmds.rs / bad_set_operation_params_change_nothing](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/engine/src/cmd/path_edit_cmds.rs)
  - `cargo test -p photocraft-engine --locked bad_merge_params_change_nothing -- --nocapture` — [crates/engine/src/cmd/path_edit_cmds.rs / bad_merge_params_change_nothing](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/engine/src/cmd/path_edit_cmds.rs)
  - `cargo test -p photocraft-engine --locked merge_refuses_open_subpaths -- --nocapture` — [crates/engine/src/cmd/path_edit_cmds.rs / merge_refuses_open_subpaths](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/engine/src/cmd/path_edit_cmds.rs)
  - `cargo test -p photocraft-ui-egui --locked ui_set_drives_the_pen_path_operation -- --nocapture` — [crates/ui-egui/src/control/tests.rs / ui_set_drives_the_pen_path_operation](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/control/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked a_selected_component_takes_the_picked_operation -- --nocapture` — [crates/ui-egui/src/tools/path_components/tests.rs / a_selected_component_takes_the_picked_operation](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/path_components/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked plus_and_minus_pick_combine_and_subtract -- --nocapture` — [crates/ui-egui/src/tools/path_components/tests.rs / plus_and_minus_pick_combine_and_subtract](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/path_components/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked path_selection_selects_and_moves_components -- --nocapture` — [crates/ui-egui/src/tools/path_components/tests.rs / path_selection_selects_and_moves_components](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/path_components/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked a_stale_selection_changes_nothing -- --nocapture` — [crates/ui-egui/src/tools/path_components/tests.rs / a_stale_selection_changes_nothing](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/path_components/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked the_first_component_keeps_combining -- --nocapture` — [crates/ui-egui/src/tools/path_components/tests.rs / the_first_component_keeps_combining](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/path_components/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked the_pen_does_not_edit_a_hidden_component_selection -- --nocapture` — [crates/ui-egui/src/tools/path_components/tests.rs / the_pen_does_not_edit_a_hidden_component_selection](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/path_components/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked dragging_a_live_shape_keeps_it_live -- --nocapture` — [crates/ui-egui/src/tools/path_components/tests.rs / dragging_a_live_shape_keeps_it_live](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/path_components/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked the_pen_carries_on_an_open_shape_path -- --nocapture` — [crates/ui-egui/src/tools/path_components/tests.rs / the_pen_carries_on_an_open_shape_path](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/path_components/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked an_open_component_of_a_filled_shape_keeps_its_paint -- --nocapture` — [crates/ui-egui/src/tools/path_components/tests.rs / an_open_component_of_a_filled_shape_keeps_its_paint](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/path_components/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked the_pen_carries_on_a_vector_masks_open_subpath -- --nocapture` — [crates/ui-egui/src/tools/path_components/tests.rs / the_pen_carries_on_a_vector_masks_open_subpath](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/path_components/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked subtract_front_shape_cuts_a_hole_in_the_selected_shape_layer -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs / subtract_front_shape_cuts_a_hole_in_the_selected_shape_layer](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked a_locked_shape_layer_keeps_the_drawn_path -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs / a_locked_shape_layer_keeps_the_drawn_path](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked switching_tools_ends_a_refused_draft_once -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs / switching_tools_ends_a_refused_draft_once](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked esc_ends_a_refused_draft_and_return_keeps_it -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs / esc_ends_a_refused_draft_and_return_keeps_it](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked an_operation_without_a_selected_shape_starts_a_new_shape_layer -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs / an_operation_without_a_selected_shape_starts_a_new_shape_layer](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked path_mode_adds_subpaths_with_the_operation -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs / path_mode_adds_subpaths_with_the_operation](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked the_options_bar_menu_sets_the_operation_and_merges -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs / the_options_bar_menu_sets_the_operation_and_merges](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked merge_is_disabled_with_fewer_than_two_components -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs / merge_is_disabled_with_fewer_than_two_components](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked an_unknown_operation_falls_back_to_new_layer -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs / an_unknown_operation_falls_back_to_new_layer](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked freeform_refuses_a_locked_shape_before_starting_a_stroke -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs / freeform_refuses_a_locked_shape_before_starting_a_stroke](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/pen_path_ops_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked failed_stale_pen_commit_preserves_the_draft_until_retry -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/tests.rs / failed_stale_pen_commit_preserves_the_draft_until_retry](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked stale_pen_commits_to_its_owner_across_document_changes -- --nocapture` — [crates/ui-egui/src/tools/vector_ui/tests.rs / stale_pen_commits_to_its_owner_across_document_changes](https://github.com/storytold/photocraft/blob/d24669d412f64ab079a668b0b8520ddbc607410c/crates/ui-egui/src/tools/vector_ui/tests.rs)

- Expected behavior: merge shape components bakes a paths operations; merge shape components on a shape layer rerenders it; merging a vector mask to nothing keeps what it shows; set operation changes existing components; set operation on a joined subpath changes its component; set operation leaves the first component combining; bad set operation params change nothing; bad merge params change nothing; merge refuses open subpaths; ui set drives the pen path operation; a selected component takes the picked operation; plus and minus pick combine and subtract; path selection selects and moves components; a stale selection changes nothing; the first component keeps combining; the pen does not edit a hidden component selection; dragging a live shape keeps it live; the pen carries on an open shape path; an open component of a filled shape keeps its paint; the pen carries on a vector masks open subpath; subtract front shape cuts a hole in the selected shape layer; a locked shape layer keeps the drawn path; switching tools ends a refused draft once; esc ends a refused draft and return keeps it; an operation without a selected shape starts a new shape layer; path mode adds subpaths with the operation; the options bar menu sets the operation and merges; merge is disabled with fewer than two components; an unknown operation falls back to new layer; freeform refuses a locked shape before starting a stroke; failed stale pen commit preserves the draft until retry; stale pen commits to its owner across document changes. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`merge_shape_components_bakes_a_paths_operations` setup/action excerpt:

```rust
let mut s = session();
        s.execute("path.set", json!({"path": square()})).unwrap();
        let hole = json!({"subpaths": [{"closed": true, "knots": [[25, 25], [45, 25], [45, 45], [25, 45]]}]});
        s.execute("path.set", json!({"path": hole, "op": "subtract"})).unwrap();
        let before = work(&s);
        let r = s.execute("path.mergeShapeComponents", json!({})).unwrap();
        assert_eq!(r["name"], json!("work"));
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Merge Shape Components"));
        let merged = work(&s);
        assert_eq!(merged.subpaths.len(), 2, "the outline and its hole");
        for (x, y) in [(15, 15), (35, 35), (55, 50), (5, 5)] {
            assert!((covered(&merged, x, y) - covered(&before, x, y)).abs() < 0.01, "({x}, {y})");
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(r["name"], json!("work"));
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Merge Shape Components"));
        let merged = work(&s);
        assert_eq!(merged.subpaths.len(), 2, "the outline and its hole");
        for (x, y) in [(15, 15), (35, 35), (55, 50), (5, 5)] {
            assert!((covered(&merged, x, y) - covered(&before, x, y)).abs() < 0.01, "({x}, {y})");
        }
        assert_eq!((covered(&merged, 35, 35), covered(&merged, 15, 15)), (0.0, 1.0), "the hole stays a hole");
        assert!(s.undo());
        assert_eq!(work(&s), before);
        // A saved path and a vector mask merge the same way; an inverted mask stays inverted.
        s.execute("path.set", json!({"name": "Mug", "path": square()})).unwrap();
        s.execute("path.mergeShapeComponents", json!({"name": "Mug"})).unwrap();
        assert!(vm.path.inverted && !vm.path.subpaths.is_empty());
    }
```

`merge_shape_components_on_a_shape_layer_rerenders_it` setup/action excerpt:

```rust
let mut s = session();
        let id = s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 50, 50], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("shape.create", json!({"kind": "rect", "rect": [25, 25, 20, 20], "addTo": id, "op": "subtract"})).unwrap();
        s.execute("path.mergeShapeComponents", json!({"name": "layer", "layer": id})).unwrap();
        let st = s.active().unwrap();
        let photocraft_doc::LayerContent::Shape(ShapeLayer { path, cache, .. }) = &st.doc.layer(LayerId(id)).unwrap().content else { panic!("shape") };
        assert_eq!(covered(path, 35, 35), 0.0);
        assert!(cache.is_some());
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(covered(path, 35, 35), 0.0);
        assert!(cache.is_some());
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-8074: Transform: remember selected interpolation (#8067)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked tool_options_survive_a_restart -- --nocapture` — [crates/ui-egui/src/prefs_ui.rs / tool_options_survive_a_restart](https://github.com/storytold/photocraft/blob/290f924d0fecc3bf6bea159c0d9e86255d326a96/crates/ui-egui/src/prefs_ui.rs)
  - `cargo test -p photocraft-ui-egui --locked interpolation_selection_is_reused_for_the_next_transform -- --nocapture` — [crates/ui-egui/src/transform_tool.rs / interpolation_selection_is_reused_for_the_next_transform](https://github.com/storytold/photocraft/blob/290f924d0fecc3bf6bea159c0d9e86255d326a96/crates/ui-egui/src/transform_tool.rs)

- Expected behavior: tool options survive a restart; interpolation selection is reused for the next transform. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`tool_options_survive_a_restart` setup/action excerpt:

```rust
let (mut app, store) = app_with_store();
        let ctx = egui::Context::default();
        tick(&mut app, &ctx);
        app.ui.tool_options.move_auto_select = false;
        app.ui.tool_options.feather = 12.0;
        app.ui.tool_options.transform_interpolation = Some(photocraft_engine::prefs::Interpolation::Nearest);
        crate::tool_memory::persist(&mut app, &ctx);
        tick(&mut app, &ctx);
        // Only what differs from the defaults is stored.
        assert_eq!(stored(&store)["toolOptions"], json!({"move_auto_select": false, "feather": 12.0, "transform_interpolation": "nearestNeighbor"}));
        let saved = store.lock().unwrap().clone().unwrap();
        let (mut app2, _) = app_with_saved(Some(saved));
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(stored(&store)["toolOptions"], json!({"move_auto_select": false, "feather": 12.0, "transform_interpolation": "nearestNeighbor"}));
        let saved = store.lock().unwrap().clone().unwrap();
        let (mut app2, _) = app_with_saved(Some(saved));
        tick(&mut app2, &ctx);
        assert!(!app2.ui.tool_options.move_auto_select);
        assert_eq!(app2.ui.tool_options.feather, 12.0);
        assert_eq!(app2.ui.tool_options.transform_interpolation, Some(photocraft_engine::prefs::Interpolation::Nearest));
        assert_eq!(app2.ui.tool_options.tolerance, crate::state::ToolOptions::default().tolerance);
        // A hand-edited file: unreadable fields keep their defaults, the good ones still load.
        for bad in [json!("junk"), json!({"move_auto_select": "no", "feather": 1e300, "tolerance": 5.0, "unknown": 1})] {
            let (mut app3, _) = app_with_saved(Some(json!({"toolOptions": bad}).to_string()));
            assert!(o.move_auto_select && o.feather == 0.0, "{bad}");
            assert_eq!(o.tolerance, if bad.is_object() { 5.0 } else { 32.0 });
        }
    }
```

`interpolation_selection_is_reused_for_the_next_transform` setup/action excerpt:

```rust
use egui::accesskit::Role;
        use egui_kittest::{Harness, kittest::Queryable};

        let app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
        let mut h = Harness::builder().with_size(vec2(2400.0, 360.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    ui.horizontal_centered(|ui| options_bar(app, ui));
                }
            },
            app,
        );
```

Expected assertion context (overlapping windows merged):

```rust
assert!(h.state().ui.transform.is_some(), "transform stays open while the bar renders");

        h.get_by_role(Role::ComboBox).click();
        h.run();
        assert!(h.state().ui.transform.is_some(), "opening interpolation dropdown keeps the transform open");
        h.key_press(egui::Key::ArrowDown);
        h.run();
        assert!(h.state().ui.transform.is_some(), "first interpolation step keeps the transform open");
        h.key_press(egui::Key::ArrowDown);
        h.run();
        assert!(h.state().ui.transform.is_some(), "second interpolation step keeps the transform open");
        assert_eq!(h.state().ui.transform.as_ref().unwrap().interpolation, "nearest");
        assert_eq!(h.state().ui.tool_options.transform_interpolation, Some(photocraft_engine::prefs::Interpolation::Nearest));
        assert_eq!(h.state().session.prefs().general.image_interpolation, photocraft_engine::prefs::Interpolation::BicubicAutomatic);

        let transform = h.state_mut().ui.transform.as_mut().unwrap();
        transform.quad = transform.quad.map(|[x, y]| [x + 1.0, y]);
        assert!(h.state().ui.transform.is_none());
        begin(h.state_mut(), &ctx).unwrap();
        assert_eq!(h.state().ui.transform.as_ref().unwrap().interpolation, "nearest");
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-8896: Select and Mask: Photoshop's full-window workspace (#7069, #8148)

- Fixture/setup: Selected document/layer-mask fixtures in dialogs/select_mask_ui/tests.rs plus two-document controls; record viewport/theme/scale.
- Action: run each command separately:

  - `cargo test -p photocraft-ui-egui --locked opens_like_photoshop -- --nocapture`
  - `cargo test -p photocraft-ui-egui --locked opening_needs_a_selection_or_a_layer_mask -- --nocapture`
  - `cargo test -p photocraft-ui-egui --locked tools_make_edits_and_undo_and_redo_them -- --nocapture`
  - `cargo test -p photocraft-ui-egui --locked ok_is_one_step_and_matches_the_command -- --nocapture`
  - `cargo test -p photocraft-ui-egui --locked cancel_changes_nothing -- --nocapture`
  - `cargo test -p photocraft-ui-egui --locked decontaminate_outputs_to_a_new_layer -- --nocapture`
  - `cargo test -p photocraft-ui-egui --locked hostile_control_params_are_errors -- --nocapture`
  - `cargo test -p photocraft-ui-egui --locked ok_after_a_document_switch_changes_neither_document -- --nocapture`
  - `cargo test -p photocraft-ui-egui --locked ok_after_a_layer_switch_leaves_the_new_layer_alone -- --nocapture`
  - `cargo test -p photocraft-ui-egui --locked renders_photoshops_layout -- --nocapture`

- Expected: Workspace needs valid selection/mask; tools/preview affect intended target. OK matches engine output in one history step; Cancel changes nothing; decontamination outputs a new layer. Hostile controls refuse; stale document/layer targets edit neither new target.
- Separate coverage / prerequisites: Native refinement → undo/redo → Cancel and commit are separate NOT RUN subcases. Capture the full workspace with reachable controls, not only an overlay helper.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9493: fix(vector): a mask feather continues the mask past the canvas edge

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-compose --locked feather_at_the_canvas_edge_continues_the_mask_past_it -- --nocapture` — [crates/compose/src/masks.rs / feather_at_the_canvas_edge_continues_the_mask_past_it](https://github.com/storytold/photocraft/blob/61a413d8df9e90ee545eb08e57c5615aa2b29cea/crates/compose/src/masks.rs)
  - `cargo test -p photocraft-compose --locked soft_shape_matches_an_equal_vector_mask -- --nocapture` — [crates/compose/src/soft_shape_tests.rs / soft_shape_matches_an_equal_vector_mask](https://github.com/storytold/photocraft/blob/61a413d8df9e90ee545eb08e57c5615aa2b29cea/crates/compose/src/soft_shape_tests.rs)
  - `cargo test -p photocraft-vector --locked parallel_gaussian_matches_the_sequential_reference -- --nocapture` — [crates/vector/src/feather.rs / parallel_gaussian_matches_the_sequential_reference](https://github.com/storytold/photocraft/blob/61a413d8df9e90ee545eb08e57c5615aa2b29cea/crates/vector/src/feather.rs)
  - `cargo test -p photocraft-vector --locked padding_each_line_matches_padding_the_plane -- --nocapture` — [crates/vector/src/feather.rs / padding_each_line_matches_padding_the_plane](https://github.com/storytold/photocraft/blob/61a413d8df9e90ee545eb08e57c5615aa2b29cea/crates/vector/src/feather.rs)
  - `cargo test -p photocraft-vector --locked benchmark_50mp_mask_blur -- --nocapture --ignored` — [crates/vector/src/feather.rs / benchmark_50mp_mask_blur](https://github.com/storytold/photocraft/blob/61a413d8df9e90ee545eb08e57c5615aa2b29cea/crates/vector/src/feather.rs)
  - `cargo test -p photocraft-vector --locked a_maximum_feather_does_not_pad_the_whole_plane -- --nocapture` — [crates/vector/src/feather.rs / a_maximum_feather_does_not_pad_the_whole_plane](https://github.com/storytold/photocraft/blob/61a413d8df9e90ee545eb08e57c5615aa2b29cea/crates/vector/src/feather.rs)
  - `cargo test -p photocraft-vector --locked a_failed_line_buffer_never_leaves_a_half_blurred_mask -- --nocapture` — [crates/vector/src/feather.rs / a_failed_line_buffer_never_leaves_a_half_blurred_mask](https://github.com/storytold/photocraft/blob/61a413d8df9e90ee545eb08e57c5615aa2b29cea/crates/vector/src/feather.rs)

- Expected behavior: feather at the canvas edge continues the mask past it; soft shape matches an equal vector mask; parallel gaussian matches the sequential reference; padding each line matches padding the plane; benchmark 50mp mask blur; a maximum feather does not pad the whole plane; a failed line buffer never leaves a half blurred mask. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`feather_at_the_canvas_edge_continues_the_mask_past_it` setup/action excerpt:

```rust
let mut layer = Layer::raster("edge", PixelFormat::RGBA8);
        let mut mask = photocraft_doc::LayerMask::hide_all();
        mask.surface.fill_rect(Rect::new(40, 20, 120, 60), &[1.0]);
        mask.feather = 32.0;
        layer.mask = Some(mask);
        let canvas = Rect::new(0, 0, 160, 80);
        let tight = combined_mask(&layer, canvas).unwrap().read_region(canvas);
        let tall = combined_mask(&layer, Rect::new(0, -200, 160, 280)).unwrap().read_region(canvas);
        let worst = tight.iter().zip(&tall).map(|(a, b)| (a - b).abs()).fold(0.0_f32, f32::max);
        assert!(worst < 1e-4, "the canvas edge changed the feather by {worst}");

        let mut half = photocraft_doc::LayerMask::hide_all();
```

Expected assertion context (overlapping windows merged):

```rust
assert!(worst < 1e-4, "the canvas edge changed the feather by {worst}");

        let mut half = photocraft_doc::LayerMask::hide_all();
        half.surface.fill_rect(Rect::new(0, 0, 80, 80), &[1.0]);
            assert!(row.iter().zip(middle).all(|(a, b)| (a - b).abs() < 1e-5), "row {y} differs from row 40");
        }
        assert!(middle[0] > 0.99, "the left edge stays white: {}", middle[0]);
    }
```

`soft_shape_matches_an_equal_vector_mask` setup/action excerpt:

```rust
// 40 px reaches past the canvas edge: both feather it the same way there (#9475).
    for (density, feather) in [(1.0, 10.0), (0.45, 0.0), (0.3, 6.5), (0.0, 0.0), (1.0, 0.0), (0.5, 3.0), (1.0, 40.0)] {
        for depth in [SampleType::U8, SampleType::U16] {
            let (s, f) = (shape_doc(depth, density, feather, None), fill_doc(depth, density, feather));
            for y in [0, 45, 50, 100, 151, 199] {
                // An 8-bit shape keeps its rendering in 8-bit pixels (the fill layer's mask is
                // 16-bit), which can round its alpha one level apart, as at any anti-aliased edge.
                let d8 = max_diff(&row(&s, y), &row(&f, y));
                assert!(d8 <= i32::from(depth == SampleType::U8), "{depth:?} density {density} feather {feather}, row {y}: {d8} levels");
                // The compositor keeps masks at 16 bits (an 8-bit shape's pixels are 8-bit).
                if depth == SampleType::U16 {
                    let d16 = max_diff(&row16(&s, y), &row16(&f, y));
```

Expected assertion context (overlapping windows merged):

```rust
assert!(d8 <= i32::from(depth == SampleType::U8), "{depth:?} density {density} feather {feather}, row {y}: {d8} levels");
                // The compositor keeps masks at 16 bits (an 8-bit shape's pixels are 8-bit).
                if depth == SampleType::U16 {
                    let d16 = max_diff(&row16(&s, y), &row16(&f, y));
                    assert!(d16 <= 2, "{depth:?} density {density} feather {feather}, row {y}: {d16} 16-bit levels");
                }
            }
        }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9457: raw: faster tone stage (Shadows, Highlights, Clarity, Texture, Dehaze), same bytes

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-raw --locked gauss_gives_the_bits_of_the_tap_loop -- --nocapture` — [crates/raw/src/tone.rs / gauss_gives_the_bits_of_the_tap_loop](https://github.com/storytold/photocraft/blob/2863c3281c2a22dade0ce92d42bf8900074f76c6/crates/raw/src/tone.rs)
  - `cargo test -p photocraft-raw --locked box_mean_gives_the_bits_of_the_column_loop -- --nocapture` — [crates/raw/src/tone.rs / box_mean_gives_the_bits_of_the_column_loop](https://github.com/storytold/photocraft/blob/2863c3281c2a22dade0ce92d42bf8900074f76c6/crates/raw/src/tone.rs)
  - `cargo test -p photocraft-raw --locked the_tap_tables_give_the_bits_of_the_per_pixel_lookup -- --nocapture` — [crates/raw/src/tone.rs / the_tap_tables_give_the_bits_of_the_per_pixel_lookup](https://github.com/storytold/photocraft/blob/2863c3281c2a22dade0ce92d42bf8900074f76c6/crates/raw/src/tone.rs)
  - `cargo test -p photocraft-raw --locked reduce_pair_gives_the_bits_of_two_reductions -- --nocapture` — [crates/raw/src/tone.rs / reduce_pair_gives_the_bits_of_two_reductions](https://github.com/storytold/photocraft/blob/2863c3281c2a22dade0ce92d42bf8900074f76c6/crates/raw/src/tone.rs)

- Expected behavior: gauss gives the bits of the tap loop; box mean gives the bits of the column loop; the tap tables give the bits of the per pixel lookup; reduce pair gives the bits of two reductions. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`gauss_gives_the_bits_of_the_tap_loop` setup/action excerpt:

```rust
// Sizes below, around and above the kernel (edges only, a few interior samples, mostly interior).
        for (w, h) in [(1, 1), (5, 3), (13, 40), (64, 7), (65, 70), (131, 97), (300, 41)] {
            let v = noise(w * h, (w * 31 + h) as u64);
            for sigma in [0.04, 0.42, 1.0, 2.1, 4.2, 8.4, 13.44] {
                assert!(same(&gauss(&v, w, h, sigma), &gauss_ref(&v, w, h, sigma)), "{w}x{h} sigma {sigma}");
            }
        }
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!(same(&gauss(&v, w, h, sigma), &gauss_ref(&v, w, h, sigma)), "{w}x{h} sigma {sigma}");
            }
        }
    }
```

`box_mean_gives_the_bits_of_the_column_loop` setup/action excerpt:

```rust
for (w, h) in [(1, 1), (5, 3), (13, 40), (64, 7), (65, 70), (131, 97), (300, 41)] {
            let v = noise(w * h, (w * 17 + h) as u64);
            for r in [1, 2, 8, 32, 128] {
                assert!(same(&box_mean(&v, w, h, r), &box_mean_ref(&v, w, h, r)), "{w}x{h} r {r}");
            }
        }
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!(same(&box_mean(&v, w, h, r), &box_mean_ref(&v, w, h, r)), "{w}x{h} r {r}");
            }
        }
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9478: fix(text): vertical type sets roman letters upright, one per em, as Photoshop does (#9470)

- Fixture/setup: Roman, ligature and half-width CJK fixtures in text/vertical_tests.rs; record fonts.
- Action: run each command separately:

  - `cargo test -p photocraft-text --locked latin_stands_upright_one_per_em_by_default -- --nocapture`
  - `cargo test -p photocraft-text --locked upright_roman_letters_stay_apart_without_ligatures -- --nocapture`
  - `cargo test -p photocraft-text --locked half_width_cjk_stays_rotated_beside_upright_roman -- --nocapture`
  - `cargo test -p photocraft-text --locked baseline_direction_round_trips -- --nocapture`

- Expected: Roman defaults upright at one-em advances without ligature collapse; half-width CJK retains specified rotation. Run BaselineDirection overrides sheet defaults and survives PSD persistence.
- Separate coverage / prerequisites: Capture upright/rotated text at 1×/2× and save/reopen PSD separately. Font-unavailable geometry is BLOCKED; the flag round-trip alone is insufficient.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9523: Fix Type on a Path wrapping and caret positioning across closed-path seams

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-text --locked closed_rectangle_left_edge_continues_past_the_top_left_seam -- --nocapture` — [crates/text/src/closed_path_tests.rs / closed_rectangle_left_edge_continues_past_the_top_left_seam](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/text/src/closed_path_tests.rs)
  - `cargo test -p photocraft-text --locked closed_ellipse_upper_left_and_apex_cross_the_seam_in_both_directions -- --nocapture` — [crates/text/src/closed_path_tests.rs / closed_ellipse_upper_left_and_apex_cross_the_seam_in_both_directions](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/text/src/closed_path_tests.rs)
  - `cargo test -p photocraft-text --locked closed_path_shows_one_lap_without_repeating_glyph_centres -- --nocapture` — [crates/text/src/closed_path_tests.rs / closed_path_shows_one_lap_without_repeating_glyph_centres](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/text/src/closed_path_tests.rs)
  - `cargo test -p photocraft-text --locked open_path_clipping_offsets_and_end_tangents_stay_unchanged -- --nocapture` — [crates/text/src/closed_path_tests.rs / open_path_clipping_offsets_and_end_tangents_stay_unchanged](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/text/src/closed_path_tests.rs)
  - `cargo test -p photocraft-text --locked closed_path_alignment_uses_one_lap_from_the_text_left_edge -- --nocapture` — [crates/text/src/closed_path_tests.rs / closed_path_alignment_uses_one_lap_from_the_text_left_edge](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/text/src/closed_path_tests.rs)
  - `cargo test -p photocraft-text --locked closed_path_start_offsets_wrap_and_invalid_coordinates_are_hidden -- --nocapture` — [crates/text/src/closed_path_tests.rs / closed_path_start_offsets_wrap_and_invalid_coordinates_are_hidden](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/text/src/closed_path_tests.rs)
  - `cargo test -p photocraft-text --locked caret_projection_roundtrips_closed_seams_and_open_offsets -- --nocapture` — [crates/text/src/closed_path_tests.rs / caret_projection_roundtrips_closed_seams_and_open_offsets](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/text/src/closed_path_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked closed_path_forward_hit_after_last_caret -- --nocapture` — [crates/ui-egui/src/type_tool.rs / closed_path_forward_hit_after_last_caret](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/ui-egui/src/type_tool.rs)
  - `cargo test -p photocraft-ui-egui --locked open_path_hits_keep_endpoint_clamping -- --nocapture` — [crates/ui-egui/src/type_tool.rs / open_path_hits_keep_endpoint_clamping](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/ui-egui/src/type_tool.rs)
  - `cargo test -p photocraft-ui-egui --locked closed_path_backward_hit_before_first_caret -- --nocapture` — [crates/ui-egui/src/type_tool.rs / closed_path_backward_hit_before_first_caret](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/ui-egui/src/type_tool.rs)
  - `cargo test -p photocraft-ui-egui --locked closed_path_upstream_caret_rectangle_left_edge_roundtrip -- --nocapture` — [crates/ui-egui/src/type_tool.rs / closed_path_upstream_caret_rectangle_left_edge_roundtrip](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/ui-egui/src/type_tool.rs)
  - `cargo test -p photocraft-ui-egui --locked closed_path_aligned_selection_highlights_every_visible_cluster -- --nocapture` — [crates/ui-egui/src/type_tool.rs / closed_path_aligned_selection_highlights_every_visible_cluster](https://github.com/storytold/photocraft/blob/728782c595e768bdb6febc7f30e4d1456d13fd36/crates/ui-egui/src/type_tool.rs)

- Expected behavior: closed rectangle left edge continues past the top left seam; closed ellipse upper left and apex cross the seam in both directions; closed path shows one lap without repeating glyph centres; open path clipping offsets and end tangents stay unchanged; closed path alignment uses one lap from the text left edge; closed path start offsets wrap and invalid coordinates are hidden; caret projection roundtrips closed seams and open offsets; closed path forward hit after last caret; open path hits keep endpoint clamping; closed path backward hit before first caret; closed path upstream caret rectangle left edge roundtrip; closed path aligned selection highlights every visible cluster. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`closed_rectangle_left_edge_continues_past_the_top_left_seam` setup/action excerpt:

```rust
let p = Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    let start = arc_length_at(&p, false, 0.0, 50.0).unwrap();
    assert!((start - 350.0).abs() < 1e-6);
    let f = PathFrame::new(&p, start as f32, false).unwrap();
    let m = f.frame(75.0).expect("glyph after the seam remains on the closed path");
    let q = centre(m, 75.0);
    assert!((q.0 - 25.0).abs() < 1e-6 && q.1.abs() < 1e-6, "{q:?}");
    assert_eq!(&m[..4], &[1.0, 0.0, -0.0, 1.0], "text now travels right on the top side");
    assert_eq!(f.apply(75.0, 0.0), q, "caret and glyph use the same wrapped parameter");
}
```

Expected assertion context (overlapping windows merged):

```rust
assert!((start - 350.0).abs() < 1e-6);
    let f = PathFrame::new(&p, start as f32, false).unwrap();
    let m = f.frame(75.0).expect("glyph after the seam remains on the closed path");
    let q = centre(m, 75.0);
    assert!((q.0 - 25.0).abs() < 1e-6 && q.1.abs() < 1e-6, "{q:?}");
    assert_eq!(&m[..4], &[1.0, 0.0, -0.0, 1.0], "text now travels right on the top side");
    assert_eq!(f.apply(75.0, 0.0), q, "caret and glyph use the same wrapped parameter");
}
```

`closed_ellipse_upper_left_and_apex_cross_the_seam_in_both_directions` setup/action excerpt:

```rust
for reverse in [false, true] {
        let mut p = ellipse();
        if reverse {
            p.knots.reverse();
            for k in &mut p.knots {
                std::mem::swap(&mut k.in_ctrl, &mut k.out_ctrl);
            }
        }
        for flip in [false, true] {
            let whole = PathFrame::new(&p, 0.0, flip).unwrap();
            let length = whole.length();
            for remaining in [0.001, 0.2, 30.0] {
```

Expected assertion context (overlapping windows merged):

```rust
assert!((actual.0 - point.0).hypot(actual.1 - point.1) < 1e-6);
                assert!((m[0] - tangent.0).abs() < 1e-6 && (m[1] - tangent.1).abs() < 1e-6);
                assert!((m[0] * m[3] - m[1] * m[2] - 1.0).abs() < 1e-6, "no reflection or scale jump");
                let before = f.apply(remaining - 0.001, 0.0);
                let after = f.apply(remaining + 0.001, 0.0);
                assert!((before.0 - after.0).hypot(before.1 - after.1) < 0.01, "seam is continuous");
            }
        }
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9594: fix(engine): copied type keeps its size across resolutions after an edit

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked copied_type_keeps_its_pixel_size_across_resolutions_after_an_edit -- --nocapture` — [crates/engine/src/layer_copy_cmds/tests.rs / copied_type_keeps_its_pixel_size_across_resolutions_after_an_edit](https://github.com/storytold/photocraft/blob/46512521fec2b4b462c29d9fefd16398cb81212c/crates/engine/src/layer_copy_cmds/tests.rs)

- Expected behavior: copied type keeps its pixel size across resolutions after an edit. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`copied_type_keeps_its_pixel_size_across_resolutions_after_an_edit` setup/action excerpt:

```rust
for (from, to) in [(72, 300), (300, 72)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 200, "resolution": from})).unwrap();
        let t = LayerId(s.execute("type.create", json!({"text": "H", "size": 24, "x": 128, "y": 100})).unwrap()["layer"].as_u64().unwrap());
        let original = bounds(&s, 0, t);
        s.execute("file.new", json!({"width": 400, "height": 200, "resolution": to})).unwrap();
        s.set_active(0);
        s.execute("layer.select", json!({"layer": t.0})).unwrap();
        let id = copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap());
        assert_eq!(bounds(&s, 1, id).size(), original.size(), "{from}→{to}: the copy");
        s.execute("type.edit", json!({"layer": id.0, "text": "M"})).unwrap();
        s.execute("type.edit", json!({"layer": id.0, "text": "H"})).unwrap();
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(bounds(&s, 1, id).size(), original.size(), "{from}→{to}: the copy");
        s.execute("type.edit", json!({"layer": id.0, "text": "M"})).unwrap();
        s.execute("type.edit", json!({"layer": id.0, "text": "H"})).unwrap();
        let after = bounds(&s, 1, id);
        assert_eq!(after.size(), original.size(), "{from}→{to}: after an edit, {after:?} vs {original:?}");
    }
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9596: fix(io): drop a kept Txt2 once no type layers are left

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-io --locked a_kept_txt2_goes_with_the_last_type_layer -- --nocapture` — [crates/io/src/psd_export.rs / a_kept_txt2_goes_with_the_last_type_layer](https://github.com/storytold/photocraft/blob/e62013cfd652fa60470a0f7a185fbc8f3d2a2c92/crates/io/src/psd_export.rs)

- Expected behavior: a kept txt2 goes with the last type layer. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`a_kept_txt2_goes_with_the_last_type_layer` setup/action excerpt:

```rust
let mut doc = Document::new("t", photocraft_geom::Size::new(8, 6), ColorMode::Rgb, SampleType::U8);
        doc.layers.push(Layer::new("Layer 1", LayerContent::Raster(Surface::new(doc.pixel_format()))));
        doc.metadata.psd_global_blocks = vec![(*b"8BIM", *b"Txt2", std::sync::Arc::new(b"<< /0 << /1 (stale) >> >>".to_vec()))];
        let file = document_to_psd(&doc);
        assert!(!file.global_blocks.iter().any(|b| b.key == *b"Txt2"), "no type layers: no Txt2");

        doc.layers.push(Layer::new("T", LayerContent::Text(photocraft_doc::TextLayer::default())));
        let file = document_to_psd(&doc);
        assert!(file.global_blocks.iter().any(|b| b.key == *b"Txt2"), "a type layer keeps its Txt2");
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!(!file.global_blocks.iter().any(|b| b.key == *b"Txt2"), "no type layers: no Txt2");

        doc.layers.push(Layer::new("T", LayerContent::Text(photocraft_doc::TextLayer::default())));
        let file = document_to_psd(&doc);
        assert!(file.global_blocks.iter().any(|b| b.key == *b"Txt2"), "a type layer keeps its Txt2");
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-3461: Replay recorded layer visibility on the intended named layer

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked recorded_eye_visibility_replays_on_another_document_and_duplicate_names -- --nocapture` — [crates/engine/src/actions_cmds.rs / recorded_eye_visibility_replays_on_another_document_and_duplicate_names](https://github.com/storytold/photocraft/blob/5d8cb2a39ad70e31260f1f3fc2010ca132daff65/crates/engine/src/actions_cmds.rs)
  - `cargo test -p photocraft-ui-egui --locked shell_playback_resolves_recorded_eye_target_on_new_document -- --nocapture` — [crates/ui-egui/src/actions.rs / shell_playback_resolves_recorded_eye_target_on_new_document](https://github.com/storytold/photocraft/blob/5d8cb2a39ad70e31260f1f3fc2010ca132daff65/crates/ui-egui/src/actions.rs)

- Expected behavior: recorded eye visibility replays on another document and duplicate names; shell playback resolves recorded eye target on new document. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`recorded_eye_visibility_replays_on_another_document_and_duplicate_names` setup/action excerpt:

```rust
let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Same"})).unwrap();
        let hidden = s.execute("layer.new.layer", json!({"name": "Same"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("actions.record", json!({"name": "Hide"})).unwrap();
        s.execute("layer.setProps", json!({"layer": hidden, "visible": false, "coalesce": "eye-sweep"})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let keep = s.execute("layer.new.layer", json!({"name": "Same"})).unwrap()["layer"].as_u64().unwrap();
        let target = s.execute("layer.new.layer", json!({"name": "Same"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("select.deselectLayers", json!({})).unwrap();
        let result = s.execute("actions.play", json!({"action": "Hide"})).unwrap();
```

Expected assertion context (overlapping windows merged):

```rust
assert!(result.get("failed").is_none(), "{result}");
        assert!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(keep)).unwrap().visible);
        assert!(!s.active().unwrap().doc.layer(photocraft_doc::LayerId(target)).unwrap().visible);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(target)).unwrap().visible);
    }
```

`shell_playback_resolves_recorded_eye_target_on_new_document` setup/action excerpt:

```rust
let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.run("layer.new.layer", json!({"name": "Target"})).unwrap();
        app.run("actions.record", json!({"name": "Hide"})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
        app.run("actions.stop", json!({})).unwrap();
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.run("layer.new.layer", json!({"name": "Target"})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        let result = play(&mut app, &json!({"action": "Hide"})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
```

Expected assertion context (overlapping windows merged):

```rust
assert!(result.get("failed").is_none(), "{result}");
        assert!(!app.session.active().unwrap().doc.layer(id).unwrap().visible);
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-3520: fix(ui): restore square color chips in Studio Dark theme

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked studio_dark_keeps_square_colour_chips -- --nocapture` — [crates/ui-egui/src/theme.rs / studio_dark_keeps_square_colour_chips](https://github.com/storytold/photocraft/blob/d36ec9a21e9c1ed70de2e0d185efd7e084644157/crates/ui-egui/src/theme.rs)

- Expected behavior: studio dark keeps square colour chips. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`studio_dark_keeps_square_colour_chips` setup/action excerpt:

```rust
assert!(!Tokens::for_kind(ThemeKind::Studio).round_chips);
        // Only the dark variant changes; the light palette retains its layout.
        assert!(Tokens::for_kind(ThemeKind::StudioLight).round_chips);
        assert!(!Tokens::for_kind(ThemeKind::Pro).round_chips);
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!(!Tokens::for_kind(ThemeKind::Studio).round_chips);
        // Only the dark variant changes; the light palette retains its layout.
        assert!(Tokens::for_kind(ThemeKind::StudioLight).round_chips);
        assert!(!Tokens::for_kind(ThemeKind::Pro).round_chips);
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-4035: fix(cmyk): Flatten Image and merges keep the inks of a tagged CMYK document (#3778)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked flatten_keeps_the_inks_of_a_single_layer_exactly -- --nocapture` — [crates/engine/tests/cmyk_flatten_profile.rs / flatten_keeps_the_inks_of_a_single_layer_exactly](https://github.com/storytold/photocraft/blob/15ebf8fc940031abdf772e5fdd687c8b55938639/crates/engine/tests/cmyk_flatten_profile.rs)
  - `cargo test -p photocraft-engine --locked stamp_visible_keeps_the_inks_of_a_single_layer_exactly -- --nocapture` — [crates/engine/tests/cmyk_flatten_profile.rs / stamp_visible_keeps_the_inks_of_a_single_layer_exactly](https://github.com/storytold/photocraft/blob/15ebf8fc940031abdf772e5fdd687c8b55938639/crates/engine/tests/cmyk_flatten_profile.rs)
  - `cargo test -p photocraft-engine --locked background_from_layer_keeps_the_inks_exactly -- --nocapture` — [crates/engine/tests/cmyk_flatten_profile.rs / background_from_layer_keeps_the_inks_exactly](https://github.com/storytold/photocraft/blob/15ebf8fc940031abdf772e5fdd687c8b55938639/crates/engine/tests/cmyk_flatten_profile.rs)
  - `cargo test -p photocraft-engine --locked flatten_of_a_partly_transparent_layer_still_composites_over_white -- --nocapture` — [crates/engine/tests/cmyk_flatten_profile.rs / flatten_of_a_partly_transparent_layer_still_composites_over_white](https://github.com/storytold/photocraft/blob/15ebf8fc940031abdf772e5fdd687c8b55938639/crates/engine/tests/cmyk_flatten_profile.rs)
  - `cargo test -p photocraft-engine --locked merge_down_separates_through_the_document_profile -- --nocapture` — [crates/engine/tests/cmyk_flatten_profile.rs / merge_down_separates_through_the_document_profile](https://github.com/storytold/photocraft/blob/15ebf8fc940031abdf772e5fdd687c8b55938639/crates/engine/tests/cmyk_flatten_profile.rs)
  - `cargo test -p photocraft-engine --locked merge_visible_separates_through_the_document_profile -- --nocapture` — [crates/engine/tests/cmyk_flatten_profile.rs / merge_visible_separates_through_the_document_profile](https://github.com/storytold/photocraft/blob/15ebf8fc940031abdf772e5fdd687c8b55938639/crates/engine/tests/cmyk_flatten_profile.rs)
  - `cargo test -p photocraft-io --locked cmyk_merged_composite_reads_through_the_embedded_profile -- --nocapture` — [crates/io/tests/color.rs / cmyk_merged_composite_reads_through_the_embedded_profile](https://github.com/storytold/photocraft/blob/15ebf8fc940031abdf772e5fdd687c8b55938639/crates/io/tests/color.rs)

- Expected behavior: flatten keeps the inks of a single layer exactly; stamp visible keeps the inks of a single layer exactly; background from layer keeps the inks exactly; flatten of a partly transparent layer still composites over white; merge down separates through the document profile; merge visible separates through the document profile; cmyk merged composite reads through the embedded profile. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`flatten_keeps_the_inks_of_a_single_layer_exactly` setup/action excerpt:

```rust
exact("Flatten Image", |s| {
        s.execute("layer.flattenImage", json!({})).unwrap();
    });
}
```

Expected assertion context (overlapping windows merged):

```rust

```

`stamp_visible_keeps_the_inks_of_a_single_layer_exactly` setup/action excerpt:

```rust
exact("Stamp Visible", |s| {
        s.execute("layer.stampVisible", json!({})).unwrap();
    });
}
```

Expected assertion context (overlapping windows merged):

```rust

```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-4547: fix(icons): PhotoCraft icon corner radius matches other Craft apps (#4478)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft --locked windows_and_linux_icons_keep_the_round_corners -- --nocapture` — [apps/photocraft/src/app_icon.rs / windows_and_linux_icons_keep_the_round_corners](https://github.com/storytold/photocraft/blob/a08f7a2295488c2692f68902835a693322017b64/apps/photocraft/src/app_icon.rs)

- Expected behavior: windows and linux icons keep the round corners. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`windows_and_linux_icons_keep_the_round_corners` setup/action excerpt:

```rust
// rx=112 of 512 leaves (10, 10) of a 256 px render transparent; the old crop filled it (#4478).
        let png = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.photocraft.png");
        let icon = eframe::icon_data::from_png_bytes(png).unwrap();
        assert_eq!(icon.width, 256);
        assert_eq!(icon.rgba[(10 * 256 + 10) * 4 + 3], 0, "corner is transparent");
        assert_eq!(icon.rgba[(128 * 256 + 2) * 4 + 3], 255, "the tile is full bleed");
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(icon.width, 256);
        assert_eq!(icon.rgba[(10 * 256 + 10) * 4 + 3], 0, "corner is transparent");
        assert_eq!(icon.rgba[(128 * 256 + 2) * 4 + 3], 255, "the tile is full bleed");
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-4938: Layer Style: one dialog size on every page, and it fits short screens

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked dragging_the_title_bar_moves_the_dialog -- --nocapture` — [crates/ui-egui/src/dialogs.rs / dragging_the_title_bar_moves_the_dialog](https://github.com/storytold/photocraft/blob/63b7c921634835259139e4d3e7b9d3c396998b4d/crates/ui-egui/src/dialogs.rs)
  - `cargo test -p photocraft-ui-egui --locked layer_style_keeps_one_size_on_every_page -- --nocapture` — [crates/ui-egui/src/dialogs.rs / layer_style_keeps_one_size_on_every_page](https://github.com/storytold/photocraft/blob/63b7c921634835259139e4d3e7b9d3c396998b4d/crates/ui-egui/src/dialogs.rs)
  - `cargo test -p photocraft-ui-egui --locked layer_style_fits_a_short_screen -- --nocapture` — [crates/ui-egui/src/dialogs.rs / layer_style_fits_a_short_screen](https://github.com/storytold/photocraft/blob/63b7c921634835259139e4d3e7b9d3c396998b4d/crates/ui-egui/src/dialogs.rs)

- Expected behavior: dragging the title bar moves the dialog; layer style keeps one size on every page; layer style fits a short screen. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`dragging_the_title_bar_moves_the_dialog` setup/action excerpt:

```rust
use egui_kittest::{Harness, kittest::Queryable};

        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(1400.0, 1100.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
        harness.state_mut().ui.open_dialog(DialogKind::LayerStyle, serde_json::Map::new());
        harness.run_steps(3);
        let before = harness.get_by_label("Layer Style").rect();

        // Grab the title text itself: it must move the dialog, not select the text.
        let from = before.center();
        harness.hover_at(from);
```

Expected assertion context (overlapping windows merged):

```rust
assert!((moved - egui::vec2(-120.0, 80.0)).length() < 1.0, "dialog moved by {moved:?}");
        assert_eq!(harness.state().ui.dialogs.len(), 1, "dragging must not close the dialog");
    }
```

`layer_style_keeps_one_size_on_every_page` setup/action excerpt:

```rust
let mut h = layer_style_on_bevel(egui::vec2(1400.0, 900.0));
        let size = frame(&h).size();
        let dialog = h.state().ui.dialogs[0].id;
        for page in ["blendingOptions", "styles"] {
            h.state_mut().ui.dialog_mut(dialog).expect("still open").fields.insert("selected".into(), json!(page));
            h.run_steps(3);
            assert_eq!(frame(&h).size(), size, "{page}");
        }
        let grip = egui::Id::new(("dialog", dialog)).with("resize").with("__resize_corner");
        assert!(h.ctx.read_response(grip).is_none(), "no resize grip");
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(frame(&h).size(), size, "{page}");
        }
        let grip = egui::Id::new(("dialog", dialog)).with("resize").with("__resize_corner");
        assert!(h.ctx.read_response(grip).is_none(), "no resize grip");
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-5270: Layer Style: Gradient Overlay takes any number of colour stops (#5177)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked gradient_overlay_takes_explicit_stops -- --nocapture` — [crates/engine/src/layer_style.rs / gradient_overlay_takes_explicit_stops](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/engine/src/layer_style.rs)
  - `cargo test -p photocraft-ui-egui --locked layer_style_picker_control_binds_previews_confirms_and_cancels -- --nocapture` — [crates/ui-egui/src/control.rs / layer_style_picker_control_binds_previews_confirms_and_cancels](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/control.rs)
  - `cargo test -p photocraft-ui-egui --locked every_exposed_color_field_uses_the_shared_picker_and_only_updates_its_target -- --nocapture` — [crates/ui-egui/src/layer_style/color_picker/tests.rs / every_exposed_color_field_uses_the_shared_picker_and_only_updates_its_target](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/color_picker/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked preview_cancel_ok_parent_cancel_and_apply_are_transactional_at_every_depth -- --nocapture` — [crates/ui-egui/src/layer_style/color_picker/tests.rs / preview_cancel_ok_parent_cancel_and_apply_are_transactional_at_every_depth](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/color_picker/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked unchanged_ok_preserves_missing_parameters_and_imported_effect_snapshot -- --nocapture` — [crates/ui-egui/src/layer_style/color_picker/tests.rs / unchanged_ok_preserves_missing_parameters_and_imported_effect_snapshot](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/color_picker/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked stale_targets_and_invalid_colors_never_apply_or_leave_preview -- --nocapture` — [crates/ui-egui/src/layer_style/color_picker/tests.rs / stale_targets_and_invalid_colors_never_apply_or_leave_preview](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/color_picker/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked preview_off_and_parent_confirmation_and_cancellation_keep_their_contract -- --nocapture` — [crates/ui-egui/src/layer_style/color_picker/tests.rs / preview_off_and_parent_confirmation_and_cancellation_keep_their_contract](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/color_picker/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked eyedropper_edits_only_the_picker_and_invalid_samples_leave_it_unchanged -- --nocapture` — [crates/ui-egui/src/layer_style/color_picker/tests.rs / eyedropper_edits_only_the_picker_and_invalid_samples_leave_it_unchanged](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/color_picker/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked gradient_stop_colors_go_through_the_picker -- --nocapture` — [crates/ui-egui/src/layer_style/color_picker/tests.rs / gradient_stop_colors_go_through_the_picker](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/color_picker/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked ramp_adds_moves_and_deletes_stops -- --nocapture` — [crates/ui-egui/src/layer_style/gradient_field.rs / ramp_adds_moves_and_deletes_stops](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/gradient_field.rs)
  - `cargo test -p photocraft-ui-egui --locked presets_delete_and_location -- --nocapture` — [crates/ui-egui/src/layer_style/gradient_field.rs / presets_delete_and_location](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/gradient_field.rs)
  - `cargo test -p photocraft-ui-egui --locked stops_round_trip_through_the_field_grammar -- --nocapture` — [crates/ui-egui/src/layer_style/gradient_field.rs / stops_round_trip_through_the_field_grammar](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/gradient_field.rs)
  - `cargo test -p photocraft-ui-egui --locked factory_from_to_become_two_stops -- --nocapture` — [crates/ui-egui/src/layer_style/gradient_field.rs / factory_from_to_become_two_stops](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/gradient_field.rs)
  - `cargo test -p photocraft-ui-egui --locked the_fill_of_a_page_never_fails -- --nocapture` — [crates/ui-egui/src/layer_style/gradient_field.rs / the_fill_of_a_page_never_fails](https://github.com/storytold/photocraft/blob/9db1c7ae1d48ce4f31622b5f9029c70c590842f8/crates/ui-egui/src/layer_style/gradient_field.rs)

- Expected behavior: gradient overlay takes explicit stops; layer style picker control binds previews confirms and cancels; every exposed color field uses the shared picker and only updates its target; preview cancel ok parent cancel and apply are transactional at every depth; unchanged ok preserves missing parameters and imported effect snapshot; stale targets and invalid colors never apply or leave preview; preview off and parent confirmation and cancellation keep their contract; eyedropper edits only the picker and invalid samples leave it unchanged; gradient stop colors go through the picker; ramp adds moves and deletes stops; presets delete and location; stops round trip through the field grammar; factory from to become two stops; the fill of a page never fails. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`gradient_overlay_takes_explicit_stops` setup/action excerpt:

```rust
let mut s = session();
        let stops = json!([[0.0, "#ff0000"], [0.5, "#00ff00"], [1.0, "#0000ff"]]);
        s.execute("layer.layerStyle.gradientOverlay", json!({"stops": stops, "opacityStops": [[0, 100], [1, 50]]})).unwrap();
        let Effect::GradientOverlay { gradient, .. } = &effects(&s)[0] else { panic!() };
        assert_eq!(gradient.stops.len(), 3);
        assert_eq!(gradient.stops[1], (0.5, photocraft_color::Color::rgb(0.0, 1.0, 0.0)));
        assert_eq!(gradient.opacity_stops, vec![(0.0, 1.0), (1.0, 0.5)]);
        // Editing the same overlay: new stops replace the list (sorted), unsorted input is sorted,
        // and a geometry-only edit keeps the three stops.
        s.execute("layer.layerStyle.gradientOverlay", json!({"stops": [[1.0, "#ffffff"], [0.25, [0.0, 0.0, 0.0]]], "opacityStops": []})).unwrap();
        let Effect::GradientOverlay { gradient, .. } = &effects(&s)[0] else { panic!() };
        assert_eq!(gradient.stops.iter().map(|s| s.0).collect::<Vec<_>>(), vec![0.25, 1.0]);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(gradient.stops.len(), 3);
        assert_eq!(gradient.stops[1], (0.5, photocraft_color::Color::rgb(0.0, 1.0, 0.0)));
        assert_eq!(gradient.opacity_stops, vec![(0.0, 1.0), (1.0, 0.5)]);
        // Editing the same overlay: new stops replace the list (sorted), unsorted input is sorted,
        // and a geometry-only edit keeps the three stops.
        s.execute("layer.layerStyle.gradientOverlay", json!({"stops": [[1.0, "#ffffff"], [0.25, [0.0, 0.0, 0.0]]], "opacityStops": []})).unwrap();
        assert_eq!(gradient.stops.iter().map(|s| s.0).collect::<Vec<_>>(), vec![0.25, 1.0]);
        assert!(gradient.opacity_stops.is_empty(), "[] means opaque");
        s.execute("layer.layerStyle.gradientOverlay", json!({"stops": stops})).unwrap();
        // The dialog's path: the snapshot plus a geometry edit keeps the three stops.
        let fx = serde_json::to_value(&effects(&s)[0]).unwrap();
        assert_eq!((gradient.stops.len(), gradient.angle), (3, 45.0));
        let e = s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "gradientOverlay", "params": {"stops": [[0, "#ff0000"]]}}]})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{e}");
        // A gradient stroke takes stops too.
        s.execute("layer.layerStyle.stroke", json!({"size": 4, "stops": [[0, "#ff0000"], [0.5, "#00ff00"], [1, "#0000ff"]]})).unwrap();
        assert!(effects(&s).iter().any(|e| matches!(e, Effect::Stroke(st) if matches!(&st.paint, FxPaint::Gradient(g) if g.stops.len() == 3))));
        // Malformed lists are errors and change nothing.
        for bad in [
            json!({"stops": "red"}),
            assert!(matches!(e, EngineError::BadParams { .. }), "{bad}: {e}");
        }
        let Effect::GradientOverlay { gradient, .. } = &effects(&s)[0] else { panic!() };
        assert_eq!(gradient.stops.len(), 3, "a refused edit keeps the gradient");
        // Positions outside 0..1 are clamped.
        s.execute("layer.layerStyle.gradientOverlay", json!({"stops": [[-1, "#000000"], [2, "#ffffff"]]})).unwrap();
        let Effect::GradientOverlay { gradient, .. } = &effects(&s)[0] else { panic!() };
        assert_eq!(gradient.stops.iter().map(|s| s.0).collect::<Vec<_>>(), vec![0.0, 1.0]);
    }
```

`layer_style_picker_control_binds_previews_confirms_and_cancels` setup/action excerpt:

```rust
let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        let parent = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"layerStyle","effect":"gradientOverlay"}))["result"]["dialog"].as_u64();
        let parent = parent.unwrap();
        let before = app.ui.dialogs[0].fields.clone();
        // The Gradient Overlay's colours are its stops (#5177): `stop` picks one.
        for stop in [0, 1] {
            let reply = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":stop}));
            assert_eq!(reply["ok"], true, "{reply}");
            let picker = app.ui.dialogs.last().unwrap().id;
            let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(reply["ok"], true, "{reply}");
            let picker = app.ui.dialogs.last().unwrap().id;
            let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
            assert!(inspected.to_string().contains("__layerStyleColor"));
            assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":parent}))["ok"], false);
            assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog":picker,"field":"color","value":"#123456"}))["ok"], true);
            assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":picker}))["ok"], true);
            assert_eq!(app.ui.dialogs[0].fields["effects"][0]["params"]["stops"][stop][1], "#123456");
        }
        assert_ne!(app.ui.dialogs[0].fields, before);
        let bad = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":"x"}));
        assert_eq!(bad["ok"], false, "{bad}");
        let bad = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":9}));
        assert_eq!(bad["ok"], false, "{bad}");
        call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":1}));
        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog":parent}))["ok"], true);
        assert!(app.ui.dialogs.is_empty());
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-5744: fix(ui): keep detached documents in one interactive window

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked torn_off_document_is_not_drawn_again_in_the_main_window -- --nocapture` — [crates/ui-egui/src/canvas.rs / torn_off_document_is_not_drawn_again_in_the_main_window](https://github.com/storytold/photocraft/blob/50b1f6734799cf7b5d45dd84868976afd31aa711/crates/ui-egui/src/canvas.rs)
  - `cargo test -p photocraft-ui-egui --locked tab_reorder_keeps_floating_documents_in_place -- --nocapture` — [crates/ui-egui/src/canvas.rs / tab_reorder_keeps_floating_documents_in_place](https://github.com/storytold/photocraft/blob/50b1f6734799cf7b5d45dd84868976afd31aa711/crates/ui-egui/src/canvas.rs)
  - `cargo test -p photocraft-ui-egui --locked arrange_layout_only_includes_docked_documents -- --nocapture` — [crates/ui-egui/src/canvas.rs / arrange_layout_only_includes_docked_documents](https://github.com/storytold/photocraft/blob/50b1f6734799cf7b5d45dd84868976afd31aa711/crates/ui-egui/src/canvas.rs)
  - `cargo test -p photocraft-ui-egui --locked dragging_a_tab_off_the_strip_floats_it_in_a_window -- --nocapture` — [crates/ui-egui/src/canvas.rs / dragging_a_tab_off_the_strip_floats_it_in_a_window](https://github.com/storytold/photocraft/blob/50b1f6734799cf7b5d45dd84868976afd31aa711/crates/ui-egui/src/canvas.rs)
  - `cargo test -p photocraft-ui-egui --locked views_and_windows_stay_with_their_documents -- --nocapture` — [crates/ui-egui/src/file_open/tests.rs / views_and_windows_stay_with_their_documents](https://github.com/storytold/photocraft/blob/50b1f6734799cf7b5d45dd84868976afd31aa711/crates/ui-egui/src/file_open/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked float_in_window_takes_a_document_and_position -- --nocapture` — [crates/ui-egui/src/view_cmds/tests.rs / float_in_window_takes_a_document_and_position](https://github.com/storytold/photocraft/blob/50b1f6734799cf7b5d45dd84868976afd31aa711/crates/ui-egui/src/view_cmds/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked float_in_window_defaults_to_the_active_document -- --nocapture` — [crates/ui-egui/src/view_cmds/tests.rs / float_in_window_defaults_to_the_active_document](https://github.com/storytold/photocraft/blob/50b1f6734799cf7b5d45dd84868976afd31aa711/crates/ui-egui/src/view_cmds/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked float_all_in_windows_gives_each_document_its_own_view -- --nocapture` — [crates/ui-egui/src/view_cmds/tests.rs / float_all_in_windows_gives_each_document_its_own_view](https://github.com/storytold/photocraft/blob/50b1f6734799cf7b5d45dd84868976afd31aa711/crates/ui-egui/src/view_cmds/tests.rs)

- Expected behavior: torn off document is not drawn again in the main window; tab reorder keeps floating documents in place; arrange layout only includes docked documents; dragging a tab off the strip floats it in a window; views and windows stay with their documents; float in window takes a document and position; float in window defaults to the active document; float all in windows gives each document its own view. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`torn_off_document_is_not_drawn_again_in_the_main_window` setup/action excerpt:

```rust
for kind in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::Studio] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 80, "height": 60, "name": "floating"})).unwrap();
            app.run("file.new", json!({"width": 80, "height": 60, "name": "docked"})).unwrap();
            app.sync_views();
            let ctx = egui::Context::default();
            crate::menus::invoke(&mut app, &ctx, "window.arrange.floatInWindow", json!({"document": 0})).unwrap();
            assert!(app.ui.windows[0].floating);
            assert_eq!(app.session.active_index(), Some(0), "{kind:?}: the floated page remains active");
            assert_eq!(main_canvas_document(&app), Some(1), "{kind:?}: the main canvas picks a docked page instead");
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(680.0, 480.0)).build_ui_state(
                |ui, (app, ready): &mut (PhotocraftApp, bool)| {
```

Expected assertion context (overlapping windows merged):

```rust
assert!(app.ui.windows[0].floating);
            assert_eq!(app.session.active_index(), Some(0), "{kind:?}: the floated page remains active");
            assert_eq!(main_canvas_document(&app), Some(1), "{kind:?}: the main canvas picks a docked page instead");
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(680.0, 480.0)).build_ui_state(
                |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                    if *ready {
            assert_eq!(tabs, Some(vec![1]), "{kind:?}: only the docked document stays on the main tab strip");
        }
    }
```

`tab_reorder_keeps_floating_documents_in_place` setup/action excerpt:

```rust
let visible = [0, 2];
        assert_eq!(move_target_after_filter(&visible, 0, 1, 3), 2);
        assert_eq!(move_target_after_filter(&visible, 2, 0, 3), 0);
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(move_target_after_filter(&visible, 0, 1, 3), 2);
        assert_eq!(move_target_after_filter(&visible, 2, 0, 3), 0);
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-6387: Video Frames to Layers: every page of a multi-page TIFF (part of #6379)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked video_frames_to_layers_takes_every_tiff_page -- --nocapture` — [crates/engine/src/video_cmds/tests.rs / video_frames_to_layers_takes_every_tiff_page](https://github.com/storytold/photocraft/blob/8bfbbf816025942b74130adf16fb07265817b691/crates/engine/src/video_cmds/tests.rs)
  - `cargo test -p photocraft-engine --locked video_frames_to_layers_refuses_a_tiff_with_an_incomplete_page_list -- --nocapture` — [crates/engine/src/video_cmds/tests.rs / video_frames_to_layers_refuses_a_tiff_with_an_incomplete_page_list](https://github.com/storytold/photocraft/blob/8bfbbf816025942b74130adf16fb07265817b691/crates/engine/src/video_cmds/tests.rs)

- Expected behavior: video frames to layers takes every tiff page; video frames to layers refuses a tiff with an incomplete page list. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`video_frames_to_layers_takes_every_tiff_page` setup/action excerpt:

```rust
let dir = tmpdir("tiff-pages");
    let path = format!("{dir}/pages.tif");
    write_tiff(&path, &[(2, 2, 9, 1), (6, 4, 51, 0), (3, 5, 204, 0)]);
    let mut s = session();
    let r = s.execute("file.import.videoFramesToLayers", json!({"path": path})).unwrap();
    assert_eq!(r["layers"], 2, "two pages, the thumbnail left out");
    let doc = &s.active().unwrap().doc;
    for (name, (w, h), level) in [("Frame 1", (6, 4), 51.0), ("Frame 2", (3, 5), 204.0)] {
        let l = doc.layers.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("{name}"));
        let surf = l.surface().unwrap();
        let b = surf.content_bounds();
        assert_eq!((b.width(), b.height()), (w, h), "{name}");
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(r["layers"], 2, "two pages, the thumbnail left out");
    let doc = &s.active().unwrap().doc;
    for (name, (w, h), level) in [("Frame 1", (6, 4), 51.0), ("Frame 2", (3, 5), 204.0)] {
        let l = doc.layers.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!((b.width(), b.height()), (w, h), "{name}");
        let px = surf.read_region(photocraft_geom::Rect::new(b.x0, b.y0, b.x0 + 1, b.y0 + 1));
        assert!((px[0] * 255.0 - level).abs() < 0.6 && px[3] == 1.0, "{name}: {px:?}");
    }
    let single = format!("{dir}/single.tif");
    write_tiff(&single, &[(4, 4, 120, 0)]);
    assert_eq!(s.execute("file.import.videoFramesToLayers", json!({"path": single})).unwrap()["layers"], 1);
}
```

`video_frames_to_layers_refuses_a_tiff_with_an_incomplete_page_list` setup/action excerpt:

```rust
let dir = tmpdir("tiff-loop");
    let path = format!("{dir}/loop.tif");
    write_tiff(&path, &[(2, 2, 51, 0), (2, 2, 204, 0)]);
    // The last directory's next-IFD offset points back to the first directory.
    let mut f = std::fs::read(&path).unwrap();
    let first: [u8; 4] = f[4..8].try_into().unwrap();
    let end = f.len();
    f[end - 4..].copy_from_slice(&first);
    std::fs::write(&path, f).unwrap();
    let mut s = session();
    let before = s.active().unwrap().doc.layers.len();
    let err = s.execute("file.import.videoFramesToLayers", json!({"path": path})).unwrap_err().to_string();
```

Expected assertion context (overlapping windows merged):

```rust
assert!(err.contains("only the first 2 pages"), "{err}");
    assert_eq!(s.active().unwrap().doc.layers.len(), before, "nothing imported");
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-6449: Hand tool: Cmd held zooms in and Alt zooms out, as in Photoshop

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked cmd_is_not_the_move_tool_for_selection_hand_zoom_pen_shape_or_type_tools -- --nocapture` — [crates/ui-egui/src/hold_keys/tests.rs / cmd_is_not_the_move_tool_for_selection_hand_zoom_pen_shape_or_type_tools](https://github.com/storytold/photocraft/blob/25c38a0b69b7bff31badcdbe2ef9f2ff6aba1c63/crates/ui-egui/src/hold_keys/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked cmd_and_alt_with_the_hand_tool_zoom_in_and_out_while_held -- --nocapture` — [crates/ui-egui/src/hold_keys/tests.rs / cmd_and_alt_with_the_hand_tool_zoom_in_and_out_while_held](https://github.com/storytold/photocraft/blob/25c38a0b69b7bff31badcdbe2ef9f2ff6aba1c63/crates/ui-egui/src/hold_keys/tests.rs)
  - `cargo test -p photocraft-ui-egui --locked cmd_mid_pan_or_during_free_transform_is_not_the_hand_tools_zoom -- --nocapture` — [crates/ui-egui/src/hold_keys/tests.rs / cmd_mid_pan_or_during_free_transform_is_not_the_hand_tools_zoom](https://github.com/storytold/photocraft/blob/25c38a0b69b7bff31badcdbe2ef9f2ff6aba1c63/crates/ui-egui/src/hold_keys/tests.rs)

- Expected behavior: cmd is not the move tool for selection hand zoom pen shape or type tools; cmd and alt with the hand tool zoom in and out while held; cmd mid pan or during free transform is not the hand tools zoom. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`cmd_is_not_the_move_tool_for_selection_hand_zoom_pen_shape_or_type_tools` setup/action excerpt:

```rust
for tool in [
        Tool::Hand,
        Tool::RotateView,
        Tool::Zoom,
        Tool::Pen,
        Tool::Rectangle,
        Tool::Type,
        Tool::VerticalType,
        Tool::Crop,
        Tool::PathSelection,
        Tool::DirectSelection,
        Tool::Move,
```

Expected assertion context (overlapping windows merged):

```rust
assert!(!cmd_moves(tool), "{tool:?}");
        let mut h = harness(tool);
        h.event(Event::ModifiersChanged(platform(Modifiers::COMMAND)));
        h.run_steps(1);
        assert_eq!(held_tool(h.state(), &h.ctx), (tool == Tool::Hand).then_some(Temporary::ZoomIn), "{tool:?}");
    }
    for tool in [Tool::Brush, Tool::Eraser, Tool::Gradient, Tool::CloneStamp, Tool::Eyedropper, Tool::MagneticLasso, Tool::QuickSelection] {
        assert!(cmd_moves(tool), "{tool:?}");
    }
}
```

`cmd_and_alt_with_the_hand_tool_zoom_in_and_out_while_held` setup/action excerpt:

```rust
let mut h = harness(Tool::Hand);
    let z0 = h.state().ui.views[0].zoom;
    hold_mods(&mut h, Modifiers::COMMAND);
    assert_eq!(held_tool(h.state(), &h.ctx), Some(Temporary::ZoomIn));
    click(&mut h, 200.0, 150.0, Modifiers::COMMAND);
    let z1 = h.state().ui.views[0].zoom;
    assert!(z1 > z0, "{z0} -> {z1}");
    hold_mods(&mut h, Modifiers::ALT);
    assert_eq!(held_tool(h.state(), &h.ctx), Some(Temporary::ZoomOut));
    click(&mut h, 200.0, 150.0, Modifiers::ALT);
    let z2 = h.state().ui.views[0].zoom;
    assert!(z2 < z1, "{z1} -> {z2}");
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(held_tool(h.state(), &h.ctx), Some(Temporary::ZoomIn));
    click(&mut h, 200.0, 150.0, Modifiers::COMMAND);
    let z1 = h.state().ui.views[0].zoom;
    assert!(z1 > z0, "{z0} -> {z1}");
    hold_mods(&mut h, Modifiers::ALT);
    assert_eq!(held_tool(h.state(), &h.ctx), Some(Temporary::ZoomOut));
    click(&mut h, 200.0, 150.0, Modifiers::ALT);
    let z2 = h.state().ui.views[0].zoom;
    assert!(z2 < z1, "{z1} -> {z2}");
    hold_mods(&mut h, Modifiers::COMMAND | Modifiers::ALT);
    assert_eq!(held_tool(h.state(), &h.ctx), Some(Temporary::ZoomOut), "⌥ turns ⌘'s Zoom In around");
    hold_mods(&mut h, Modifiers::NONE);
    assert_eq!(held_tool(h.state(), &h.ctx), None);
    // Released: a drag pans again.
    let (center, zoom) = (h.state().ui.views[0].center, h.state().ui.views[0].zoom);
    drag_with(&mut h, Modifiers::NONE, 200.0, 150.0, 260.0, 190.0);
    assert_ne!(h.state().ui.views[0].center, center, "the Hand pans");
    assert_eq!(h.state().ui.views[0].zoom, zoom);
    assert_eq!(h.state().ui.tool, Tool::Hand);
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-7255: fix(engine): guard trim and duplicate merged against documents exceeding render limits (#7223)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-engine --locked trim_and_duplicate_merged_on_too_large_document_fail_gracefully_without_panic -- --nocapture` — [crates/engine/src/image_cmds.rs / trim_and_duplicate_merged_on_too_large_document_fail_gracefully_without_panic](https://github.com/storytold/photocraft/blob/1b810052bc3a876c1eaf1a8766870ffdba9f8a80/crates/engine/src/image_cmds.rs)

- Expected behavior: trim and duplicate merged on too large document fail gracefully without panic. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`trim_and_duplicate_merged_on_too_large_document_fail_gracefully_without_panic` setup/action excerpt:

```rust
let mut s = Session::new();
        s.execute("file.new", json!({"width": 40000, "height": 30000, "background": "transparent"})).unwrap();
        let initial_layers = doc(&s).layers.len();
        let initial_bounds = doc(&s).bounds();
        let initial_rev = s.active().unwrap().revision;
        let initial_docs = s.documents().len();

        let trim_res = s.execute("image.trim", json!({"basedOn": "topLeft"}));
        assert!(trim_res.is_err(), "trim on too-large document must return error");
        let err = trim_res.unwrap_err().to_string();
        assert!(err.contains("too large to trim"), "error should explain document is too large: {err}");
        assert_eq!(doc(&s).layers.len(), initial_layers, "layer count unchanged after failed trim");
```

Expected assertion context (overlapping windows merged):

```rust
assert!(trim_res.is_err(), "trim on too-large document must return error");
        let err = trim_res.unwrap_err().to_string();
        assert!(err.contains("too large to trim"), "error should explain document is too large: {err}");
        assert_eq!(doc(&s).layers.len(), initial_layers, "layer count unchanged after failed trim");
        assert_eq!(doc(&s).bounds(), initial_bounds, "canvas size unchanged after failed trim");
        assert_eq!(s.active().unwrap().revision, initial_rev, "revision unchanged after failed trim");

        let dup_res = s.execute("image.duplicate", json!({"mergedOnly": true}));
        assert!(dup_res.is_err(), "duplicate merged on too-large document must return error");
        let dup_err = dup_res.unwrap_err().to_string();
        assert!(dup_err.contains("too large to duplicate merged"), "error should explain document is too large: {dup_err}");
        assert_eq!(s.documents().len(), initial_docs, "document count unchanged after failed duplicate");
        assert_eq!(doc(&s).layers.len(), initial_layers, "layer count unchanged after failed duplicate");
        assert_eq!(doc(&s).bounds(), initial_bounds, "canvas size unchanged after failed duplicate");
        assert_eq!(s.active().unwrap().revision, initial_rev, "revision unchanged after failed duplicate");
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-8931: feat(algo): describe the Gaussian blur as data for other devices

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-algo --locked the_description_is_the_cpus_blur -- --nocapture` — [crates/algo/src/separable.rs / the_description_is_the_cpus_blur](https://github.com/storytold/photocraft/blob/d42dd4c6302c77c1f9dc46b7b53ffa5eb0047a13/crates/algo/src/separable.rs)
  - `cargo test -p photocraft-algo --locked an_impulse_spreads_no_further_than_the_reach -- --nocapture` — [crates/algo/src/separable.rs / an_impulse_spreads_no_further_than_the_reach](https://github.com/storytold/photocraft/blob/d42dd4c6302c77c1f9dc46b7b53ffa5eb0047a13/crates/algo/src/separable.rs)
  - `cargo test -p photocraft-algo --locked output_depends_only_on_the_input_region -- --nocapture` — [crates/algo/src/separable.rs / output_depends_only_on_the_input_region](https://github.com/storytold/photocraft/blob/d42dd4c6302c77c1f9dc46b7b53ffa5eb0047a13/crates/algo/src/separable.rs)

- Expected behavior: the description is the cpus blur; an impulse spreads no further than the reach; output depends only on the input region. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`the_description_is_the_cpus_blur` setup/action excerpt:

```rust
for radius in RADII {
            let params = FilterParams::GaussianBlur { radius };
            let blur = Separable::of(&params).unwrap();
            match &blur {
                Separable::Kernel(k) => {
                    assert!(radius <= 4.0, "radius {radius}");
                    assert_eq!(k, &blur::gaussian_kernel(radius));
                    assert_eq!(k.len() % 2, 1);
                    assert!((k.iter().sum::<f32>() - 1.0).abs() < 1e-5);
                }
                Separable::Boxes(b) => {
                    assert!(radius > 4.0, "radius {radius}");
```

Expected assertion context (overlapping windows merged):

```rust
assert!(radius <= 4.0, "radius {radius}");
                    assert_eq!(k, &blur::gaussian_kernel(radius));
                    assert_eq!(k.len() % 2, 1);
                    assert!((k.iter().sum::<f32>() - 1.0).abs() < 1e-5);
                }
                Separable::Boxes(b) => {
                    assert!(radius > 4.0, "radius {radius}");
                    assert_eq!(Some(b.clone()), blur::box_widths(&params));
                    assert!(b.iter().all(|w| w % 2 == 1));
                }
            }
            let Halo::Radius(halo) = params.halo() else { panic!("a Gaussian has a radius halo") };
            assert!(blur.reach() <= halo, "radius {radius}: reach {} past the halo {halo}", blur.reach());
        }
        assert_eq!(Separable::of(&FilterParams::BoxBlur { radius: 9.0 }), None, "not a Gaussian (yet)");
        assert_eq!(Separable::Boxes(vec![usize::MAX, 3]).reach(), i32::MAX, "saturates");
    }
```

`an_impulse_spreads_no_further_than_the_reach` setup/action excerpt:

```rust
for radius in RADII {
            let params = FilterParams::GaussianBlur { radius };
            let reach = Separable::of(&params).unwrap().reach();
            let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
            let size = 2 * reach + 41;
            let canvas = Rect::new(0, 0, size, size);
            let mut s = Surface::new(fmt);
            let dot = Rect::new(size / 2, size / 2, size / 2 + 1, size / 2 + 1);
            s.fill_rect(dot, &[1.0, 1.0, 1.0, 1.0]);
            let out = crate::apply_in_with(&s, &params, canvas, canvas, None, canvas, &Interrupt::NONE).unwrap();
            let mut px = Vec::new();
            out.read_region_into(canvas, &mut px);
```

Expected assertion context (overlapping windows merged):

```rust
assert!(dot.inflate(reach).contains_rect(&touched), "radius {radius}: {touched:?} past the reach {reach}");
            if matches!(Separable::of(&params), Some(Separable::Kernel(_))) {
                assert_eq!(touched, dot.inflate(reach), "radius {radius}");
            }
        }
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9004: fix(ui): enlarge Move tool alignment icons for legibility

- Fixture/setup: Studio Dark and Studio Light at UI scales 100% and 200%, 1280x800 and 900x600 logical-point windows; synthetic document with two selected layers and Move tool. Capture disabled state with fewer selected layers too.
- Action: Capture the six Move Align controls and Align and distribute ellipsis at each theme/scale/window combination. Measure icon artwork/hit rectangles from the captured scale and invoke Left Edges then Undo.
- Expected: Artwork is 20 logical points within 28-point hit targets (40/56 physical pixels at 2x), without clipping or overlap. Enabled-state and command dispatch are unchanged; Left Edges aligns the selected layers and Undo restores their positions.
- Status: NOT RUN
- Actual:
- Evidence:
- Tester:

## PR-9015: fix(xtask): reject leading zeroes in numeric prerelease identifiers

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p xtask --locked rejects_numeric_prerelease_identifiers_with_leading_zeroes -- --nocapture` — [xtask/src/version.rs / rejects_numeric_prerelease_identifiers_with_leading_zeroes](https://github.com/storytold/photocraft/blob/afb4364a8795c42e6353762891b4890d878f9a68/xtask/src/version.rs)

- Expected behavior: rejects numeric prerelease identifiers with leading zeroes. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`rejects_numeric_prerelease_identifiers_with_leading_zeroes` setup/action excerpt:

```rust
for version in ["1.0.0-01", "1.0.0-rc.01", "1.0.0-00.1"] {
            assert!(validate(version).is_err(), "validator accepted invalid version {version}");
            assert!(replace(MANIFEST, version).is_err(), "accepted invalid version {version}");
        }
        for version in ["1.0.0-0", "1.0.0-rc.0", "1.0.0-01a", "1.0.0-01-2"] {
            let updated = replace(MANIFEST, version).expect("valid prerelease");
            assert_eq!(read(&updated).unwrap(), version);
        }
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!(validate(version).is_err(), "validator accepted invalid version {version}");
            assert!(replace(MANIFEST, version).is_err(), "accepted invalid version {version}");
        }
        for version in ["1.0.0-0", "1.0.0-rc.0", "1.0.0-01a", "1.0.0-01-2"] {
            let updated = replace(MANIFEST, version).expect("valid prerelease");
            assert_eq!(read(&updated).unwrap(), version);
        }
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9354: fix(ui): Full Screen Mode needs a document (#9349)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked screen_mode_button_is_inert_without_a_document -- --nocapture` — [crates/ui-egui/src/panels.rs / screen_mode_button_is_inert_without_a_document](https://github.com/storytold/photocraft/blob/41eb1de5cff17f5f5b9c22913e48b71565b56a2a/crates/ui-egui/src/panels.rs)
  - `cargo test -p photocraft-ui-egui --locked long_press_screen_mode_opens_the_modes_without_cycling -- --nocapture` — [crates/ui-egui/src/panels.rs / long_press_screen_mode_opens_the_modes_without_cycling](https://github.com/storytold/photocraft/blob/41eb1de5cff17f5f5b9c22913e48b71565b56a2a/crates/ui-egui/src/panels.rs)
  - `cargo test -p photocraft-ui-egui --locked full_screen_modes_need_a_document -- --nocapture` — [crates/ui-egui/src/view_cmds/tests.rs / full_screen_modes_need_a_document](https://github.com/storytold/photocraft/blob/41eb1de5cff17f5f5b9c22913e48b71565b56a2a/crates/ui-egui/src/view_cmds/tests.rs)

- Expected behavior: screen mode button is inert without a document; long press screen mode opens the modes without cycling; full screen modes need a document. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`screen_mode_button_is_inert_without_a_document` setup/action excerpt:

```rust
let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", serde_json::json!({"width": 64, "height": 48})).unwrap();
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        frame(&mut app, &ctx, 0.0, vec![]);
        frame(&mut app, &ctx, 0.1, vec![]);
        let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
        // Found while a document makes it clickable: the toolbar's last button.
        let at = ctx
            .viewport(|v| {
                v.prev_pass
                    .widgets
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(crate::view_cmds::is_enabled(&app, "view.screenMode.cycle"), Some(false));
        let pointer = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut sent = Vec::new();
        for (t, events) in [(2.0, vec![egui::Event::PointerMoved(at), pointer(true)]), (2.1, vec![pointer(false)]), (2.15, vec![])] {
        assert!(!sent.iter().any(|c| matches!(c, egui::ViewportCommand::Fullscreen(_))), "no screen mode change was sent: {sent:?}");
        assert_eq!(app.ui.view.screen_mode, "standard");
    }
```

`long_press_screen_mode_opens_the_modes_without_cycling` setup/action excerpt:

```rust
let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        // A document open: Full Screen Mode needs one (#9349), so without it a click can't cycle.
        app.run("file.new", serde_json::json!({"width": 64, "height": 48})).unwrap();
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        frame(&mut app, &ctx, 0.0, vec![]);
        frame(&mut app, &ctx, 0.1, vec![]);
        let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
        // The toolbar's last button: Change Screen Mode, after Quick Mask.
        let at = ctx
            .viewport(|v| {
                v.prev_pass
```

Expected assertion context (overlapping windows merged):

```rust
assert!(standard(&app));
        // Hold, then release: the list opens and stays open, and the mode is unchanged.
        frame(&mut app, &ctx, 1.0, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
        frame(&mut app, &ctx, 1.2, vec![]);
        assert!(!menu_open(&ctx), "not before the hold time");
        frame(&mut app, &ctx, 1.4, vec![]);
        assert!(menu_open(&ctx), "open while still held");
        frame(&mut app, &ctx, 1.5, vec![pointer(at, false)]);
        frame(&mut app, &ctx, 1.55, vec![]);
        assert!(menu_open(&ctx), "the release keeps it open");
        assert!(standard(&app), "the release doesn't cycle the mode");
        // Escape closes it; a plain click cycles the mode and opens nothing.
        frame(
            &mut app,
        assert!(!menu_open(&ctx));
        frame(&mut app, &ctx, 3.0, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
        frame(&mut app, &ctx, 3.1, vec![pointer(at, false)]);
        frame(&mut app, &ctx, 3.15, vec![]);
        assert!(!standard(&app), "a click cycles");
        assert!(!menu_open(&ctx));
        // Right-click opens it too.
        let secondary = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, 4.0, vec![secondary(true)]);
        assert!(menu_open(&ctx));
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9418: Color Lookup: honour the .3dl Mesh header's cube edge and output depth (#9078, #9073)

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-cms --locked three_dl_without_a_header_reads_low_nodes_as_12_bit -- --nocapture` — [crates/cms/src/lutfile.rs / three_dl_without_a_header_reads_low_nodes_as_12_bit](https://github.com/storytold/photocraft/blob/da4b838e54f6073cda4798fa7b63b3b7214aaf93/crates/cms/src/lutfile.rs)
  - `cargo test -p photocraft-cms --locked three_dl_mesh_three_point_shaper_is_not_a_node -- --nocapture` — [crates/cms/src/lutfile.rs / three_dl_mesh_three_point_shaper_is_not_a_node](https://github.com/storytold/photocraft/blob/da4b838e54f6073cda4798fa7b63b3b7214aaf93/crates/cms/src/lutfile.rs)
  - `cargo test -p photocraft-cms --locked three_dl_mesh_declares_the_output_depth -- --nocapture` — [crates/cms/src/lutfile.rs / three_dl_mesh_declares_the_output_depth](https://github.com/storytold/photocraft/blob/da4b838e54f6073cda4798fa7b63b3b7214aaf93/crates/cms/src/lutfile.rs)
  - `cargo test -p photocraft-cms --locked three_dl_mesh_header_mismatch_is_rejected -- --nocapture` — [crates/cms/src/lutfile.rs / three_dl_mesh_header_mismatch_is_rejected](https://github.com/storytold/photocraft/blob/da4b838e54f6073cda4798fa7b63b3b7214aaf93/crates/cms/src/lutfile.rs)
  - `cargo test -p photocraft-cms --locked three_dl_malformed_mesh_header_falls_back_to_inference -- --nocapture` — [crates/cms/src/lutfile.rs / three_dl_malformed_mesh_header_falls_back_to_inference](https://github.com/storytold/photocraft/blob/da4b838e54f6073cda4798fa7b63b3b7214aaf93/crates/cms/src/lutfile.rs)
  - `cargo test -p photocraft-cms --locked three_dl_mesh_uses_declared_range_for_small_integer_codes -- --nocapture` — [crates/cms/src/lutfile.rs / three_dl_mesh_uses_declared_range_for_small_integer_codes](https://github.com/storytold/photocraft/blob/da4b838e54f6073cda4798fa7b63b3b7214aaf93/crates/cms/src/lutfile.rs)
  - `cargo test -p photocraft-engine --locked color_lookup_mesh_3dl_honours_the_header -- --nocapture` — [crates/engine/src/adjust_cmds/tests.rs / color_lookup_mesh_3dl_honours_the_header](https://github.com/storytold/photocraft/blob/da4b838e54f6073cda4798fa7b63b3b7214aaf93/crates/engine/src/adjust_cmds/tests.rs)

- Expected behavior: three dl without a header reads low nodes as 12 bit; three dl mesh three point shaper is not a node; three dl mesh declares the output depth; three dl mesh header mismatch is rejected; three dl malformed mesh header falls back to inference; three dl mesh uses declared range for small integer codes; color lookup mesh 3dl honours the header. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`three_dl_without_a_header_reads_low_nodes_as_12_bit` setup/action excerpt:

```rust
let table = |peak: u32| -> String {
            let mut text = String::from("0 1023\n");
            for i in 0..8 {
                let on = |bit: u32| if i & bit != 0 { peak } else { 0 };
                text.push_str(&format!("{} {} {}\n", on(4), on(2), on(1)));
            }
            text
        };
        for peak in [255, 1023, 4095] {
            let white = parse_3dl(&table(peak)).unwrap().data[21..24].to_vec();
            assert_eq!(white, [peak as f32 / 4095.0; 3], "peak {peak}");
        }
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(white, [peak as f32 / 4095.0; 3], "peak {peak}");
        }
        // A header keeps the inferred 10-bit depth. The bare `3DMESH` magic is a header without a
        // cube declaration; `Mesh 1 10` would declare a 3³ cube and refuse this 2³ table (#9078).
        assert_eq!(white, [1.0; 3]);
    }
```

`three_dl_mesh_three_point_shaper_is_not_a_node` setup/action excerpt:

```rust
// #9078: Photoshop loads a 3³ cube whose shaper line has three values; counting it as a
        // 28th node rejected the whole file.
        let l = parse_3dl(&mesh_3dl(1, 12, 3, 4095.0, true)).unwrap();
        assert_eq!(l.size, 3);
        assert_eq!(l.data.len(), 3 * 3 * 3 * 3);
        // The inverse identity: node (0,0,0) is full, node (2,2,2) black.
        assert!((l.data[0] - 1.0).abs() < 1e-6 && (l.data[1] - 1.0).abs() < 1e-6 && (l.data[2] - 1.0).abs() < 1e-6);
        let last = &l.data[l.data.len() - 3..];
        assert_eq!(last, [0.0, 0.0, 0.0]);
        // The same 27 nodes without the shaper row still load (the node-only control).
        assert_eq!(parse_3dl(&mesh_3dl(1, 12, 3, 4095.0, false)).unwrap().data, l.data);
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(l.size, 3);
        assert_eq!(l.data.len(), 3 * 3 * 3 * 3);
        // The inverse identity: node (0,0,0) is full, node (2,2,2) black.
        assert!((l.data[0] - 1.0).abs() < 1e-6 && (l.data[1] - 1.0).abs() < 1e-6 && (l.data[2] - 1.0).abs() < 1e-6);
        let last = &l.data[l.data.len() - 3..];
        assert_eq!(last, [0.0, 0.0, 0.0]);
        // The same 27 nodes without the shaper row still load (the node-only control).
        assert_eq!(parse_3dl(&mesh_3dl(1, 12, 3, 4095.0, false)).unwrap().data, l.data);
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9481: fix(ui): new layer effects start from the Make Default values

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked new_effects_start_from_the_made_default -- --nocapture` — [crates/ui-egui/src/layer_style.rs / new_effects_start_from_the_made_default](https://github.com/storytold/photocraft/blob/7937adff961027154d795b8905e821244ae4f929/crates/ui-egui/src/layer_style.rs)

- Expected behavior: new effects start from the made default. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`new_effects_start_from_the_made_default` setup/action excerpt:

```rust
let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("layer.layerStyle.makeDefault", json!({"kind": "stroke", "params": {"size": 7, "position": "outside"}})).unwrap();

        let id = open(&mut app, Some("stroke")).unwrap();
        let mut f = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
        add_instance(&mut f, "stroke");
        add_instance(&mut f, "dropShadow");
        let fx = effects_of(&f);
        let strokes: Vec<_> = fx.iter().filter(|e| e["kind"] == json!("stroke")).collect();
        assert_eq!(strokes.len(), 2);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(strokes.len(), 2);
        for e in strokes {
            assert_eq!((&e["params"]["size"], &e["params"]["position"]), (&json!(7), &json!("outside")));
            assert_eq!(e["params"]["opacity"], defaults("stroke")["opacity"], "unsaved keys keep the factory value");
        }
        let shadow = fx.iter().find(|e| e["kind"] == json!("dropShadow")).unwrap();
        assert_eq!(shadow["params"]["size"], defaults("dropShadow")["size"]);
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9504: fix(ui): Keyboard Shortcuts records ⌘Z instead of undoing the capture

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked cmd_z_is_recorded_while_capturing_and_undoes_otherwise -- --nocapture` — [crates/ui-egui/src/shortcut_capture_tests.rs / cmd_z_is_recorded_while_capturing_and_undoes_otherwise](https://github.com/storytold/photocraft/blob/993c9ed167c6acd9bc8f87c6cff6e2c7abba17a8/crates/ui-egui/src/shortcut_capture_tests.rs)

- Expected behavior: cmd z is recorded while capturing and undoes otherwise. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`cmd_z_is_recorded_while_capturing_and_undoes_otherwise` setup/action excerpt:

```rust
let (mut h, id) = dialog(1.0, "edit.search", "search…");
    capture(&mut h, &crate::shortcuts::pretty("Cmd+F"), Key::ControlLeft, Modifiers::COMMAND, Key::Z);
    assert_eq!(field(&h, id, "overrides")["edit.search"], json!("Cmd+Z"));
    assert_eq!(field(&h, id, "capture"), json!(false));
    key(&mut h, Key::Z, true, Modifiers::COMMAND);
    h.run_steps(2);
    assert_ne!(field(&h, id, "overrides")["edit.search"], json!("Cmd+Z"), "⌘Z outside a capture undoes the change");
    assert_eq!(field(&h, id, "capture"), json!(false), "and doesn't wait for keys again");
    // CodeRabbit on #9504: clearing the restored capture isn't a new edit, so Redo still works.
    h.run_steps(2);
    key(&mut h, Key::Z, true, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run_steps(2);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(field(&h, id, "overrides")["edit.search"], json!("Cmd+Z"));
    assert_eq!(field(&h, id, "capture"), json!(false));
    key(&mut h, Key::Z, true, Modifiers::COMMAND);
    h.run_steps(2);
    assert_ne!(field(&h, id, "overrides")["edit.search"], json!("Cmd+Z"), "⌘Z outside a capture undoes the change");
    assert_eq!(field(&h, id, "capture"), json!(false), "and doesn't wait for keys again");
    // CodeRabbit on #9504: clearing the restored capture isn't a new edit, so Redo still works.
    h.run_steps(2);
    key(&mut h, Key::Z, true, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(field(&h, id, "overrides")["edit.search"], json!("Cmd+Z"), "⇧⌘Z redoes the recorded shortcut");
    assert_eq!(field(&h, id, "capture"), json!(false));
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9517: fix(ui): Layer Style scrolls on a short window so OK stays on screen

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked layer_style_buttons_stay_on_a_short_window -- --nocapture` — [crates/ui-egui/src/dialogs.rs / layer_style_buttons_stay_on_a_short_window](https://github.com/storytold/photocraft/blob/81b434c6149732eadc94e0465fea81bd075facd4/crates/ui-egui/src/dialogs.rs)

- Expected behavior: layer style buttons stay on a short window. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`layer_style_buttons_stay_on_a_short_window` setup/action excerpt:

```rust
use egui_kittest::{Harness, kittest::Queryable};

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
        app.run("layer.new.layer", serde_json::json!({})).unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1435.0, 858.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        crate::layer_style::open(h.state_mut(), Some("bevelEmboss")).unwrap();
        h.run_steps(4);
        for label in ["OK", "Cancel"] {
            let r = h.get_by_label(label).rect();
            assert!(r.bottom() <= 858.0, "{label} is off the window: {r:?}");
```

Expected assertion context (overlapping windows merged):

```rust
assert!(r.bottom() <= 858.0, "{label} is off the window: {r:?}");
        }
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9597: fix(ui): the layer list doesn't move with no layer selected

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked layer_rows_do_not_move_when_no_layer_is_selected -- --nocapture` — [crates/ui-egui/src/layer_pct_slider_tests.rs / layer_rows_do_not_move_when_no_layer_is_selected](https://github.com/storytold/photocraft/blob/9429df26b371c04c2afa65228a60ded739b5fbc2/crates/ui-egui/src/layer_pct_slider_tests.rs)

- Expected behavior: layer rows do not move when no layer is selected. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`layer_rows_do_not_move_when_no_layer_is_selected` setup/action excerpt:

```rust
for theme in [crate::theme::ThemeKind::Studio, crate::theme::ThemeKind::Classic, crate::theme::ThemeKind::ProMedium] {
        let mut h = harness();
        PhotocraftApp::setup_context(&h.ctx, theme);
        h.run_steps(4);
        let row_top = |h: &Harness<'_, PhotocraftApp>| by_role(h, "Paint", Role::Button).map(|r| r.top());
        let selected = row_top(&h);
        h.state_mut().session.execute("select.deselectLayers", json!({})).unwrap();
        h.run_steps(4);
        let none = row_top(&h);
        assert!(selected.is_some(), "{theme:?}: the layer row");
        assert_eq!(none, selected, "{theme:?}: the row moved with no layer selected");
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert!(selected.is_some(), "{theme:?}: the layer row");
        assert_eq!(none, selected, "{theme:?}: the row moved with no layer selected");
    }
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9598: fix(ui): Liquify's Twirl Counterclockwise gets a mirrored twirl icon

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked twirl_counterclockwise_has_its_own_mirrored_icon -- --nocapture` — [crates/ui-egui/src/liquify_ui.rs / twirl_counterclockwise_has_its_own_mirrored_icon](https://github.com/storytold/photocraft/blob/47aea89e9220b996f0d5a0d2bb8b5938100b4ad1/crates/ui-egui/src/liquify_ui.rs)
  - `cargo test -p photocraft-ui-egui --locked alt_reversed_twirl_survives_replay_undo_and_redo -- --nocapture` — [crates/ui-egui/src/liquify_ui.rs / alt_reversed_twirl_survives_replay_undo_and_redo](https://github.com/storytold/photocraft/blob/47aea89e9220b996f0d5a0d2bb8b5938100b4ad1/crates/ui-egui/src/liquify_ui.rs)

- Expected behavior: twirl counterclockwise has its own mirrored icon; alt reversed twirl survives replay undo and redo. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`twirl_counterclockwise_has_its_own_mirrored_icon` setup/action excerpt:

```rust
assert_eq!(tool_icon(LiquifyTool::TwirlCcw), "rotate-ccw");
        let svg = |name: &str| crate::icon_data::ICONS.iter().find(|(n, _)| *n == name).map(|(_, b)| String::from_utf8_lossy(b).into_owned());
        let (cw, ccw) = (svg("rotate-cw").unwrap(), svg("rotate-ccw").expect("the rotate-ccw icon"));
        assert!(ccw.contains("M3 12a9 9 0 1 0 9-9") && cw.contains("M21 12a9 9 0 1 1-9-9"), "mirror images");
        for t in [LiquifyTool::ForwardWarp, LiquifyTool::TwirlCw, LiquifyTool::TwirlCcw, LiquifyTool::Pucker, LiquifyTool::Bloat] {
            assert!(svg(tool_icon(t)).is_some(), "{t:?}: {}", tool_icon(t));
        }
    }
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(tool_icon(LiquifyTool::TwirlCcw), "rotate-ccw");
        let svg = |name: &str| crate::icon_data::ICONS.iter().find(|(n, _)| *n == name).map(|(_, b)| String::from_utf8_lossy(b).into_owned());
        let (cw, ccw) = (svg("rotate-cw").unwrap(), svg("rotate-ccw").expect("the rotate-ccw icon"));
        assert!(ccw.contains("M3 12a9 9 0 1 0 9-9") && cw.contains("M21 12a9 9 0 1 1-9-9"), "mirror images");
        for t in [LiquifyTool::ForwardWarp, LiquifyTool::TwirlCw, LiquifyTool::TwirlCcw, LiquifyTool::Pucker, LiquifyTool::Bloat] {
            assert!(svg(tool_icon(t)).is_some(), "{t:?}: {}", tool_icon(t));
        }
    }
```

`alt_reversed_twirl_survives_replay_undo_and_redo` setup/action excerpt:

```rust
let ctx = egui::Context::default();
        let mut app = app_with_layer();
        open(&mut app, &ctx).unwrap();
        control(&mut app, &json!({"tool": "twirlCw", "size": 36, "rate": 80})).unwrap();

        // Releasing Alt while dragging must not change the tool recorded at pointer-down.
        pointer(&mut app, ToolEvent::Down { x: 53.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::ALT);
        pointer(&mut app, ToolEvent::Move { x: 58.0, y: 41.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Up { x: 58.0, y: 41.0 }, egui::Modifiers::NONE);
        let d = app.distort.liquify.as_ref().unwrap();
        assert_eq!(d.opts.tool, LiquifyTool::TwirlCw, "Alt must not change the selected tool");
        assert_eq!(d.strokes.len(), 1);
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(d.opts.tool, LiquifyTool::TwirlCw, "Alt must not change the selected tool");
        assert_eq!(d.strokes.len(), 1);
        assert_eq!(d.strokes[0].tool, LiquifyTool::TwirlCcw);
        assert_eq!(LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes), d.field);

        // A following stroke without Alt records the original tool.
        pointer(&mut app, ToolEvent::Down { x: 70.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert_eq!(d.strokes.len(), 2);
        assert_eq!(d.strokes[1].tool, LiquifyTool::TwirlCw);
        assert_eq!(LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes), d.field);

        control(&mut app, &json!({"undo": true})).unwrap();
        let d = app.distort.liquify.as_ref().unwrap();
        assert_eq!(d.strokes.len(), 1);
        assert_eq!(d.strokes[0].tool, LiquifyTool::TwirlCcw);
        assert_eq!(d.redo[0].tool, LiquifyTool::TwirlCw);
        assert_eq!(LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes), d.field);

        control(&mut app, &json!({"redo": true})).unwrap();
        let d = app.distort.liquify.as_ref().unwrap();
        assert_eq!(d.strokes.len(), 2);
        assert!(d.redo.is_empty());
        assert_eq!(d.strokes[0].tool, LiquifyTool::TwirlCcw);
        assert_eq!(d.strokes[1].tool, LiquifyTool::TwirlCw);
        assert_eq!(LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes), d.field);
    }
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:

## PR-9606: fix(ui): the Color panel remembers its options across restarts

- Fixture/setup and action: execute **all** identified changed regression functions below on the final frozen tree. Their checked-in setup constructs the concrete inputs; preserve those inputs, capability prerequisites and assertions.

  - `cargo test -p photocraft-ui-egui --locked color_panel_options_survive_a_restart -- --nocapture` — [crates/ui-egui/src/color_panel_tests.rs / color_panel_options_survive_a_restart](https://github.com/storytold/photocraft/blob/6cc516a3d20120b678040050d95ccadc8d1dfd3f/crates/ui-egui/src/color_panel_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked an_outside_preference_change_reaches_the_live_panel -- --nocapture` — [crates/ui-egui/src/color_panel_tests.rs / an_outside_preference_change_reaches_the_live_panel](https://github.com/storytold/photocraft/blob/6cc516a3d20120b678040050d95ccadc8d1dfd3f/crates/ui-egui/src/color_panel_tests.rs)
  - `cargo test -p photocraft-ui-egui --locked dragging_the_splitter_resizes_and_survives_a_ui_state_round_trip -- --nocapture` — [crates/ui-egui/src/dock_tests.rs / dragging_the_splitter_resizes_and_survives_a_ui_state_round_trip](https://github.com/storytold/photocraft/blob/6cc516a3d20120b678040050d95ccadc8d1dfd3f/crates/ui-egui/src/dock_tests.rs)

- Expected behavior: color panel options survive a restart; an outside preference change reaches the live panel; dragging the splitter resizes and survives a ui state round trip. Each listed test must actually execute its assertions; these are distinct behavior-specific oracles, not a single interchangeable guard.

<details><summary>Historical source context (deduplicated excerpts; not additional cases or execution evidence)</summary>

`color_panel_options_survive_a_restart` setup/action excerpt:

```rust
let mut first = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    first.ui.color_panel.mode = ColorPanelMode::HsbSliders;
    first.ui.color_panel.ramp = ColorRamp::GrayscaleRamp;
    crate::color_panel_ui::persist(&mut first);
    let saved = first.session.prefs().clone();

    let mut next = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    next.session.prefs.edit(|p| *p = saved);
    crate::color_panel_ui::restore(&mut next);
    assert_eq!((next.ui.color_panel.mode, next.ui.color_panel.ramp), (ColorPanelMode::HsbSliders, ColorRamp::GrayscaleRamp));

    let mut fresh = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!((next.ui.color_panel.mode, next.ui.color_panel.ramp), (ColorPanelMode::HsbSliders, ColorRamp::GrayscaleRamp));

    let mut fresh = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    crate::color_panel_ui::restore(&mut fresh);
    assert_eq!(fresh.ui.color_panel.mode, ColorPanelMode::HueCube, "no saved options: the default");
}
```

`an_outside_preference_change_reaches_the_live_panel` setup/action excerpt:

```rust
let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.color_panel.mode = ColorPanelMode::HsbSliders;
    crate::color_panel_ui::persist(&mut app);
    // As prefs.set or a reset to defaults would: the stored options change under the panel.
    let mut other = app.ui.color_panel;
    other.mode = ColorPanelMode::GrayscaleSlider;
    app.session.prefs.edit(|p| p.color_panel = serde_json::to_value(other).unwrap());
    crate::color_panel_ui::persist(&mut app);
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::GrayscaleSlider, "the live panel follows the preference");
    let stored: crate::state::ColorPanelState = serde_json::from_value(app.session.prefs().color_panel.clone()).unwrap();
    assert_eq!(stored.mode, ColorPanelMode::GrayscaleSlider, "and the preference isn't overwritten");
    // A reset to defaults clears the stored options: the panel goes back to its defaults too.
```

Expected assertion context (overlapping windows merged):

```rust
assert_eq!(app.ui.color_panel.mode, ColorPanelMode::GrayscaleSlider, "the live panel follows the preference");
    let stored: crate::state::ColorPanelState = serde_json::from_value(app.session.prefs().color_panel.clone()).unwrap();
    assert_eq!(stored.mode, ColorPanelMode::GrayscaleSlider, "and the preference isn't overwritten");
    // A reset to defaults clears the stored options: the panel goes back to its defaults too.
    app.session.prefs.edit(|p| p.color_panel = serde_json::Value::Null);
    crate::color_panel_ui::persist(&mut app);
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::HueCube, "reset: the default mode");
    crate::color_panel_ui::persist(&mut app);
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::HueCube, "and it stays reset");
}
```

</details>

- Status: NOT RUN
- Actual:
- Evidence:
- Tester/date:
- Tested SHA:
