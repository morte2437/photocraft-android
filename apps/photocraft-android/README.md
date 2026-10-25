# PhotoCraft for Android tablets (prototype)

The same PhotoCraft app (engine, compositor, brushes, file formats, egui UI) running on Android
tablets: an arm64-v8a APK, minimum Android 8.0 (API 26), built for Android 15 (API 35).

Status: **functional prototype.** It builds, installs and starts; editing, file access and pen
input are wired up. It is not tuned for touch-first use yet (see [Not supported](#not-supported-yet)).

## How it fits together

```text
PhotoCraftActivity.java (extends GameActivity)       libphotocraft_android.so (this crate)
 ├─ dispatchTouchEvent / dispatchGenericMotionEvent ──JNI──▶ photocraft-tablet::android ─▶ StylusFeed
 │    (pressure, AXIS_TILT, AXIS_ORIENTATION, tool type)        (pure mapping: tablet::motion)
 ├─ Storage Access Framework pickers   ◀──files──▶  bridge.rs (request.json / answer.json / sync/)
 ├─ Open with / Share  ──────────────────files──▶  bridge/incoming ─▶ app inbox
 └─ GameActivity ──▶ android-activity ──▶ winit ──▶ eframe (wgpu: Vulkan, GLES fallback)
                                                     └─▶ PhotocraftApp (photocraft-ui-egui, unchanged)
```

- **Rendering:** eframe's native runner on winit's GameActivity backend. wgpu uses Vulkan, or
  OpenGL ES 3 where Vulkan is missing; the GPU canvas is used whenever the adapter is a real GPU.
- **Pen (Wacom EMR, S Pen, USI):** winit forwards only pressure (as touch force). The Activity
  reads every `MotionEvent`'s pressure, tilt, orientation and tool type (including historical
  samples) and passes them to Rust through one JNI method with primitive arguments.
  `photocraft-tablet`'s `motion` module turns Android's polar tilt (angle from the normal plus
  direction) into the W3C `tiltX`/`tiltY` pair the brush engine uses, and flags the eraser end,
  which switches to the Eraser tool as on the desktop. Samples feed the same `StylusFeed` as
  the desktop tablet monitors, so Pen Pressure/Tilt brush dynamics, Preferences › Tools › Use
  Tablet Pressure and the pressure curve all apply.
- **Files:** File › Open and Save As show Android's document pickers. Picked documents are copied
  into app-private storage and opened from there. Saving writes the private copy, then the
  Activity copies it back to the document (when the provider grants write access; otherwise a
  message suggests Save As). Documents sent from other apps ("Open with", "Share") open in a new
  tab. Preferences and brush presets live in the app's private storage.
- **On-screen keyboard:** winit 0.30 opens the soft keyboard but drops GameActivity's text
  events, so the shell (`src/app.rs`, `src/ime.rs`) polls the keyboard's text state each frame
  while a text field has focus and replays the changes as egui text/Backspace/Enter events. The
  keyboard is set to multi-line so its Enter key reaches the app, and it is put away when focus
  leaves the field.
- **Hardware keyboard shortcuts:** winit 0.30 never reports modifier state on Android, so the
  shell (`src/keys.rs`) tracks Ctrl, Alt and Shift from their own key presses, in event order
  (Android often delivers Ctrl down, S down and Ctrl up within one frame), and puts them on
  every key and pointer event: Ctrl+S, Ctrl+Z, Ctrl+Shift+S, Alt-click and the rest work.
- **Leaving:** File › Exit (Ctrl+Q) asks, as on the desktop, about unsaved work, then has the
  Activity finish and end the process once saves have been copied back. winit's event loop can't
  be created twice in one process (a relaunch in the same process closed at once) and
  GameActivity's `onDestroy` would wait forever for the stopped Rust loop. Back doesn't close the
  app (it would skip the unsaved-work prompt).
- **Fingers** paint and click like a mouse (pressure 1): winit reports a finger's contact size as
  touch force, so the app turns off `Stylus::touch_force` and takes pressure from the pen only.
- **GPU limits:** device limits are clamped to what the adapter reports
  (`gpu_canvas::limits_for`), since mobile and software Vulkan drivers fall below wgpu's desktop
  defaults (the emulator's SwiftShader: 16 KiB uniform buffers).
- **The bridge** (`src/bridge.rs`) is plain files in `files/bridge`, so it needs no JNI calls
  from Rust and no `unsafe`; the protocol is documented in that module.
- **Settings backup:** updates keep everything in the app's storage. For a reinstall or a new
  tablet, Android's app backup (the Google account, and device-to-device transfer) keeps the
  preferences, presets (brushes, adjustments, tools), Shortcut Pad, Touch Input and layout
  settings, and restores them when the app is installed again (`res/xml/data_extraction_rules.xml`,
  `backup_rules.xml`; #17). Documents, crash-recovery files and the bridge are never backed up;
  imported fonts move with a device transfer but stay out of the cloud backup (its 25 MB limit).
  Test with `adb shell bmgr backupnow ai.storyteller.photocraft`, `pm clear`, then
  `bmgr restore 1 ai.storyteller.photocraft` on the local transport.

`unsafe`: the workspace forbids it. Two attributes need it here: `#[unsafe(no_mangle)]` on
`android_main` (android-activity looks it up by name, `src/app.rs`) and on the JNI pen hook
(`crates/tablet/src/android.rs`). Neither dereferences a pointer.

## Shortcut Pad

A slim strip on the left or right edge of the window with 2 to 10 keys (4 by default) split
above and below a wheel, modelled on a pen tablet's ExpressKeys and Touch Ring, for working
without a keyboard.

![PhotoCraft on an Android tablet with the Shortcut Pad on the right edge](../../docs/images/android-shortcut-pad.png)

<img src="../../docs/images/android-shortcut-pad-closeup.png" alt="The Shortcut Pad: Undo and Redo keys, the wheel set to Zoom, Alt and Pan keys" width="210">

- **Keys** press a shortcut while touched. `Ctrl+Z`, `B` or `Tab` act once; a modifier on its
  own (`Alt`, `Shift`, `Ctrl`) or `Space` is held for as long as the finger stays on the key, so
  the pen can Alt-click (Eyedropper, Clone Stamp source) or pan while a key is held.
  Defaults: Undo, Redo, Alt, Pan (Space). With an odd number of keys the extra one sits above
  the wheel. Keys are 44 points tall where they fit and shrink evenly (to 26 points) on a
  shorter screen, so ten fit any tablet.
- **The wheel**: press the ring and a large ring opens beside the pad;
  slide your finger around it. Clockwise zooms in, rotates the view, grows or hardens the brush,
  or redoes; counter-clockwise does the opposite. Zoom, rotation, size and hardness change
  smoothly (at sensitivity 1×, a quarter turn doubles the zoom, a third of a turn doubles the
  brush size, a full turn runs hardness 0–100%); undo/redo steps every 30°. The ring shows the
  function and its value (zoom %, angle, size, hardness). The centre is a button: a tap
  switches function without opening the large ring; the lights on the small wheel show which
  one is active.
- **Full screen key** (always there, just above the keyboard key): steps through full screen
  keeping the tool bar and options bar, full screen with the canvas alone, and back, so full
  screen (which has no menu bar) never needs a keyboard to get out of. It is drawn like a video
  player's button and shows what a tap does: corners pointing out (lit, with a bar along its
  edge, while the bars are kept), or pointing in to come back.
- **Keyboard key** (always at the very bottom): opens the on-screen keyboard with no text field
  needed, and closes it when tapped again (or put the keyboard away with its own hide key; the
  pad's key, lit while the keyboard is open, goes dark). With no field focused, letters and
  numbers act as their shortcut keys (`E` picks the Eraser; with a pad key set to `Ctrl` held,
  `S` saves); in a text field it types as usual.
- **Window › Shortcut Pad**: Show Shortcut Pad, On the Left / On the Right, and Shortcut Pad
  Settings… (the number of keys; each key's label, colour (nine presets or any colour) and
  shortcut, from presets or typed; which
  functions the wheel's centre cycles through; whether the large ring opens, its size
  (140–400 points) and the wheel's sensitivity (0.25×–4×); Restore Defaults). Adding keys fills them with the
  presets no key uses yet; removing takes them from the bottom. Settings are saved in the app's files directory
  (`shortcut-pad.json`).

How it works (`src/pad/`): the pad is an egui side panel drawn before the app, so the app lays
out beside it. egui turns only the first touch into the pointer, so a finger on a key would
have kept the pen from drawing; while the pad is shown, `router.rs` takes over touch-to-pointer
emulation: touches that start on the pad go to the pad (never to the canvas), the first touch
elsewhere drives the pointer, and a window or menu drawn over the pad keeps its touches. Keys
become key events before the modifier tracker (`keys.rs`) runs, so they combine with a
hardware keyboard; wheel steps run app commands (`view.zoomIn`, `view.rotateView`,
`tools.increaseBrushSize`, `edit.undo`, …). The Window menu rows come through a small hook in
`photocraft-ui-egui` (`platform_menu.rs`, `Services::platform_menu`).

## Touch Input

Multi-touch gestures, set up in **Window › Touch Input** (finger mode, palm rejection) and
**Touch Input Settings…** (everything, saved in `touch-input.json`):

| Gesture | Default | Options |
|---|---|---|
| Two fingers on the canvas | Pan, pinch to zoom and twist to rotate, all at once (the image stays under the fingers) | Each can be turned off |
| Two-finger tap | Undo (Ctrl+Z) | Any shortcut, a preset or Nothing |
| Three-finger tap | Redo (Ctrl+Shift+Z) | Any shortcut, a preset or Nothing |
| One finger on the canvas | Draws and clicks like the pen | Move the canvas, or nothing (gestures only) |
| Fingers while the pen is on or near the screen | Ignored (palm rejection); a pen touching down drops resting fingers | Off |
| Two fingers over menus, panels and lists | Scroll them | Off |
| One finger on a list row (Brushes, Layers, presets) | Swiping up or down scrolls the list; hold still for a moment, then drag, to reorder; a sideways drag moves a slider | |
| One finger on the options bar, when it's wider than the screen | Swiping sideways scrolls it; hold still for a moment, then drag, to work a slider or number field | |
| A finger held still on a button, tab, panel or row (about 0.8 s) | Right-clicks it (context menus, closing panels) | Off |
| A mouse's right button | Right-clicks | |

**Trackpad Cursor** (Window › Touch Input, off by default; `pad/trackpad.rs`, #4): fingers move
an arrow cursor that stays on screen like a mouse pointer, so the fingertip never covers what it
points at. Slide to move it (speed 0.5–3× in the settings), tap to click where it is, double tap
to double-click, hold still to right-click, tap and then touch and drag to hold the button
(painting, dragging). Over the canvas a tap's click waits 0.25 s so a stroke doesn't start with a
dot; over the interface it clicks at once. Two fingers moving together pan, zoom and rotate the
canvas when the cursor is over it and scroll the panel under it otherwise. The pen draws where it
touches and moves the cursor there. Optional, all off by default: acceleration (quick swipes move
the cursor up to 2.5× as far), resting one finger while another moves to hold the button, and a
two-finger tap that right-clicks instead of running its shortcut. Ideas from how Winlator's
touchpad behaves; no code from it.

**Finger pressure** (experimental, Touch Input Settings): a finger's contact size stands in for
pressure, between a light touch and a firm press calibrated on a box in the settings. The
settings show what the screen reports for each touch (pressure, size, contact length), which
varies a lot between devices; some report no size at all, and then it can't work.

**Pen buttons** (also in Touch Input Settings…): Pen button 1 (nearest the tip), 2 and 3 and
the eraser end each **erase**, **right-click**, **run a shortcut** or do **nothing**. Defaults,
as a Wacom pen on a PC: button 1 right-clicks, button 2 erases, button 3 does nothing, the
eraser end erases. Right-click works as on a PC: press the button while the pen hovers to
right-click under it, or hold it while touching down. Each row has a light that turns green
while that button is pressed (or the eraser end is near), and the raw button state Android
reports is shown beneath, for pens that number their buttons unusually.

A flicked menu or list keeps gliding, whether it scrolls itself or a swipe scrolls it. A lone finger waits a tenth of a second (or until it moves)
before it becomes the pointer, so a second finger landing with it starts a gesture instead of a
stroke or a click.

How it works: the touch router (`src/pad/router.rs`) owns every touch. The Activity reports
each pointer's tool type as it goes down (`nativePointer`, `photocraft-tablet`'s
`android.rs`), so fingers are told apart from the pen, eraser and mouse; pens drive the pointer
at once. Canvas gestures become view moves applied to the active document's camera
(`src/touch/mod.rs`: zoom, centre and rotation, keeping the document point under the fingers),
taps press their shortcut like a pad key, and two fingers over the interface become wheel events.

## Phone layout

**Window › Layout** picks **Automatic** (the default), **Phone** or **Tablet**. Automatic uses the
phone layout when the screen's smallest side is under 600 dp, Android's own phone/tablet line,
so a phone stays a phone on its side; it is checked every frame, so rotating, a foldable opening
or split screen switches at once. The phone layout:

- **Panels:** the right-hand dock is only its icon rail (Properties, Navigator, Color &
  Swatches, Layers, History, Brushes, with finger-sized buttons). A rail button opens its group
  as a sheet over the canvas, one at a time, without squeezing the canvas; the same button,
  Esc, or closing or collapsing the group puts it away.
- **Interface scale:** Preferences › Interface › UI Scale › Auto is 75% of the phone's own
  scale, so a desktop-class interface has room. A fixed scale is left as chosen.
- **Dialogs** take the screen's width, stay centred and scroll when taller than the screen, so
  OK and Cancel can always be reached. Preferences puts each label above its setting.
- **Shortcut Pad:** slimmer (56 points instead of 72).

On every screen, no dialog's minimum size is bigger than the window.

## Fonts

The Type tool lists PhotoCraft's bundled fonts, the tablet's own fonts (`/system/fonts`,
`/product/fonts`: Roboto, the Noto families and the device maker's), and fonts you import.
**Type › Import Font…** opens Android's file picker for `.ttf`, `.otf`, `.ttc` or `.otc` files
(several at once): each is checked, added to the font list straight away, and kept in the app's
private `fonts` folder, which loads on every start (`src/fonts.rs`). Files that aren't fonts are
skipped with a message in the status bar.

## Updates

The app updates itself from this fork's GitHub releases (`Updater.java`). At launch (Help ›
Check for Updates at Startup, on by default) and from Help › Check for Updates…, it reads the
release list (pre-releases included) and takes the newest release tagged
`android-v<version>[-<build>]` with an APK attached. When that is newer than the installed
version (`0.8.2-3` beats `0.8.2-2` and `0.8.2`), a dialog shows the release notes' "New in this
build" section and offers **Install**, **Later** or **Skip This Version** (a skipped version is
offered again only by Help › Check for Updates…). Install downloads the APK and hands it to
Android's package installer. The first time, Android asks to allow PhotoCraft to install apps;
the install carries on when you come back.

Publishing an update: build with the next build number, then tag and attach the APK.

```bash
RELEASE_BUILD=4 apps/photocraft-android/build.sh          # photocraft-0.8.2-4-arm64-v8a.apk
gh release create android-v0.8.2-4 --prerelease --notes-file notes.md target/android/photocraft-0.8.2-4-arm64-v8a.apk
```

The build number goes into the version code (0.8.2-4 → 80204), so each release installs over
the last. Reset it to 1 when the workspace version changes. Android installs an update only when
it is signed with the same key as the installed copy: build every release with the same
keystore (here the build machine's debug key, `~/.android/debug.keystore`; keep a backup).

## Build

Requirements: Rust (see `rust-version` in the workspace `Cargo.toml`) with
`rustup target add aarch64-linux-android`, `cargo install cargo-ndk`, JDK 17, and an Android SDK
with platform 35, build-tools 35 and NDK r27 or newer.

```bash
export ANDROID_HOME=/path/to/android-sdk
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/27.3.13750724
apps/photocraft-android/build.sh
adb install -r target/android/photocraft-<version>-arm64-v8a.apk
```

`build.sh` builds the library with `cargo ndk` (copying it into `android/app/src/main/jniLibs`),
then runs Gradle (`android/`, wrapper included). `PROFILE=dev` builds an unoptimised library
faster; `FEATURES=heif` adds HEIC/HEIF decoding. Logs: `adb logcat -s PhotoCraft`.

The release APK is signed with the Android debug key so it installs as is. To distribute it,
add a `signingConfig` with your own keystore in `android/app/build.gradle`.

On a Windows host with the `x86_64-pc-windows-gnu` Rust toolchain: build scripts need a
`dlltool` that works without a GNU assembler (the NDK's `llvm-dlltool.exe`, copied to a folder on
`PATH` as `dlltool.exe`, does), and this crate's host tests can't link (a `cdylib` exceeds the
65,535 DLL export limit of GNU ld); run host tests on Linux, macOS or the MSVC toolchain.

## Tests

```bash
cargo test -p photocraft-android -p photocraft-tablet   # bridge, services, MotionEvent mapping
cargo test -p photocraft-ui-egui --lib stylus           # feed/touch-force precedence
```

The Java side has no unit tests; it was exercised on an emulator:

### Validated on the Android Emulator (2026-10-10)

Android 14 x86_64 Google APIs image, Pixel Tablet profile (2560×1600), running the arm64 APK
through the image's ARM translation; SwiftShader (software Vulkan).

| Check | Result |
|---|---|
| Install, launch, `android_main`, logging to logcat | works |
| Rendering (wgpu on Vulkan; full UI, home screen, panels, canvas) | works |
| Touch: menus, buttons, dialogs | works |
| Soft keyboard: typing into the New Document name, Enter confirms, keyboard hides | works |
| Stylus stroke (`adb shell input stylus swipe`): Java → JNI → `photocraft-tablet` → `StylusFeed` (logcat: `first pen sample`) → brush stroke | works |
| Finger stroke | works (full width, like a mouse) |
| Shortcut Pad: shows on the right, moves left (Window menu), keys (Undo, a preset changed to Eraser), wheel zoom and rotate, centre switches function, settings persist across restarts | works |
| Shortcut Pad keyboard key: opens the on-screen keyboard with no field focused, `e` picks the Eraser, hiding the keyboard turns the key off | works |
| Keyboard shortcuts with modifiers (`adb shell input keycombination`): Ctrl+N opens New Document, Ctrl+Z undoes a stroke, Ctrl+Shift+S opens the save picker | works |
| Ctrl+Q: unsaved-work prompt, then the app closes at once and reopens normally | works |
| File › Save As → system picker → PSD written to the picked document (valid `8BPS` header, 1920×1080 RGB/8) | works |
| File › Open → system picker → that PSD opens in a new tab | works |

Not covered by the emulator: real pressure curves, tilt and the eraser end (adb injects
pressure 1.0 and no tilt; the mapping is unit-tested in `photocraft-tablet`), a hardware GPU
driver, real-device performance, and holding a Shortcut Pad key while the pen draws (adb
injects one touch at a time; the routing is unit-tested in `src/pad/router.rs`).

## Supported

Everything in the shared engine and UI: layers, masks, adjustments, filters, brushes, type,
vectors, history, PSD/PSB, .pcraft, PNG/JPEG/WebP/TIFF/GIF/BMP/EXR/HDR, raw files, Affinity and
OpenRaster import, export settings, preferences, brush presets.

## Not supported yet

| Area | State on Android |
|---|---|
| On-screen keyboard | Typing works (see above), but the keyboard covers the lower half of the screen without resizing the UI (including the pad's keyboard key: use the keyboard's own hide key). Shortcuts with modifiers need a hardware keyboard or a pad key holding the modifier. Typing on a Type layer on the canvas is untested. |
| Touch gestures | See [Touch Input](#touch-input). Palm rejection relies on the pen being seen (touching, or hovering on pens that report hover): a palm landing well before the pen comes near counts as a finger until the pen touches down. |
| Pen hover | Hover moves are not forwarded by winit 0.30: no brush cursor preview while hovering. The eraser end switches tools on contact, not on proximity, so the first eraser-end touch after flipping still uses the previous tool for that stroke. |
| Barrel rotation | Android has no standard axis for it: rotation stays 0 (Wacom Art Pen rotation is not read). |
| UI density | Tablets use the desktop (Photoshop-like) layout; on small tablets panels are dense. Phones get the [phone layout](#phone-layout), but tools and the options bar are still the desktop ones. |
| Clipboard images, screen color picker, display ICC profiles | Not available (no Android services wired). |
| Crash-recovery autosave | Works (Preferences › File Handling). Besides the timer, unsaved work is saved for recovery a few seconds after your last change, because Android can close the app in the background without warning; it comes back the next time you open the app. |
| TCP control server / MCP | Not available. |
| Photoshop settings import | Not available. |
| Names from document providers | Some providers (Downloads on some devices) report a saved document's name without its extension, or no name. The app keeps the extension it asked for, so the format is right, but the file in that folder may show without `.psd`. |
| Choosing a folder | Commands that ask for a folder (such as loading video frames as layers) report that it isn't supported on Android; pick the files themselves. |
| Saving back to read-only documents | A document whose provider grants no write access is saved to the app's private copy only; a message says to use Save As. |
| Other ABIs | arm64-v8a only (no 32-bit ARM, no x86_64 build target configured). |
