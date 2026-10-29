# PhotoCraft for Android
### A desktop-class image editor, rebuilt for Android tablets and phones.

[![Download APK](https://img.shields.io/badge/Download-APK%200.16.0--4-3DDC84?style=for-the-badge&logo=android&logoColor=white)](https://github.com/morte2437/photocraft-android/releases/download/android-v0.16.0-4/photocraft-0.16.0-4-arm64-v8a.apk)
![Android 8+](https://img.shields.io/badge/Android-8.0%2B-555?style=for-the-badge)
![Free & open source](https://img.shields.io/badge/Free-Open%20source-555?style=for-the-badge)

<a href="https://apps.obtainium.imranr.dev/redirect?r=obtainium://app/%7B%22id%22%3A%22ai.storyteller.photocraft%22%2C%22url%22%3A%22https%3A%2F%2Fgithub.com%2Fmorte2437%2Fphotocraft-android%22%2C%22author%22%3A%22morte2437%22%2C%22name%22%3A%22PhotoCraft%20for%20Android%22%2C%22additionalSettings%22%3A%22%7B%5C%22includePrereleases%5C%22%3A%20true%7D%22%7D"><img src="docs/images/badge-obtainium.png" alt="Get it on Obtainium" height="56"></a>

<sub>[All releases and release notes](https://github.com/morte2437/photocraft-android/releases) · [How-to wiki](https://github.com/morte2437/photocraft-android/wiki) · arm64 tablets and phones · a prototype, signed with a debug key for sideloading · Obtainium installs and updates it straight from these releases</sub>

<img src="docs/images/android-features.gif" width="720" alt="On The Great Wave off Kanagawa: a smooth red brush spiral, a two-finger pinch and twist, holding a finger on a layer for its right-click menu, the Shortcut Pad's full screen key, then the phone layout's panels sliding out and the top bar swiped sideways">

| 🎛️ **Shortcut Pad** | ✒️ **Real pen support** | ⌨️ **Keyboards that work** |
|---|---|---|
| ExpressKeys-style touch keys and a spin wheel for zoom, rotate, brush size and undo. Hold Alt while you draw. | Pressure, tilt and the eraser end on Wacom EMR, S Pen and USI pens, drawn through every sample the pen reports for smooth curves. | Ctrl, Alt and Shift shortcuts on hardware keyboards, plus an on-screen keyboard key for shortcuts without one. |
| 📂 **Real PSD files** | 🔄 **Updates itself** | 🖥️ **The full engine** |
| Open, edit and save layered PSDs from any folder or app, through Android's file picker or Share. | New builds are offered inside the app. One tap to install. | Same layers, masks, adjustments, brushes and type as the desktop app, running on your tablet's GPU. |
| 🤏 **Multi-touch gestures** | 🖐️ **Palm rejection** | 👆 **Touch-friendly lists** |
| Pinch to zoom, twist to rotate and drag with two fingers to pan, all at once. Two-finger tap undoes, three-finger tap redoes, and each tap can run any shortcut. | Rest your hand on the screen while you draw: fingers are ignored while the pen is on or near it. One finger can draw, move the canvas, or do nothing. | Swipe to scroll menus, brush and layer lists, with momentum. Hold a brush or layer, then drag, to reorder it. |
| 📱 **Phone layout** | 🖱️ **Right-click anywhere** | 🎨 **Make it yours** |
| On a phone the panels fold into an icon rail and slide out over the canvas, dialogs fit the screen, and the top bar swipes sideways. | Hold a finger still, press a pen button while hovering, or use a mouse's right button. Each pen button can erase, right-click or run a shortcut. | Colour each Shortcut Pad key, and step through full screen with or without the tool bars from the pad. |

> [!NOTE]
> Unofficial community port of [PhotoCraft](https://github.com/storytold/photocraft); not made or supported by its developers or ArtCraft.

What the port adds:

- The full PhotoCraft engine and interface on Android (wgpu on Vulkan, OpenGL ES fallback).
- **Pen input:** pressure, tilt and the eraser end of Wacom EMR, S Pen and USI styluses, every
  sample drawn for smooth curves, and pen buttons that erase, right-click or run a shortcut
  (Window › Touch Input › Touch Input Settings…).
- **Files** through Android's document picker (Open, Save, Save As, Open with, Share).
- **Keyboards:** hardware shortcuts with Ctrl/Alt/Shift, and typing on the on-screen keyboard.
- **Touch gestures:** pinch to zoom, twist to rotate, two fingers to pan, two-finger tap to
  undo and three-finger tap to redo, palm rejection, and smooth flick scrolling in menus
  (Window › Touch Input). Hold a finger still to right-click.
- **Shortcut Pad:** 2 to 10 touch keys and a wheel on the screen edge, like a pen tablet's
  ExpressKeys, for working without a keyboard, with a colour per key and a full screen key
  (Window › Shortcut Pad).
- **Phone layout:** panels as an icon rail that slides out over the canvas, dialogs that fit
  the screen, and a swipeable options bar; on by itself on phones (Window › Layout).

**How to use it:** the [wiki](https://github.com/morte2437/photocraft-android/wiki) covers every feature, step by step.

Found a bug? [Report it here](https://github.com/morte2437/photocraft-android/issues/new/choose).

Details, build steps and what isn't supported yet:
[apps/photocraft-android/README.md](apps/photocraft-android/README.md). The Android work lives on
the `android-port` branch; `main` mirrors the original project.

---

## About PhotoCraft

[PhotoCraft](https://github.com/storytold/photocraft) is an open-source image editor written in
Rust by the ArtCraft Team and the PhotoCraft contributors. This fork adds Android support on top
of it; for the official desktop and web app, see the original repository.

## License and credits

Based on PhotoCraft, Copyright (c) 2026 ArtCraft Team and the PhotoCraft contributors, licensed
under the MIT License or the Apache License 2.0, at your option ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)). The Android port's changes are under the same terms.

Required notices are in [NOTICE](NOTICE); third-party assets and their licenses are listed in
[ATTRIBUTION.md](ATTRIBUTION.md).

The ArtCraft name, wordmark and logos are trademarks of the ArtCraft Team and are not covered by
these licenses ([`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt)).
