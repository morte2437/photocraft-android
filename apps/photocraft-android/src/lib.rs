//! PhotoCraft on Android tablets.
//!
//! Runs the same [`photocraft_ui_egui::PhotocraftApp`] as the desktop and web apps through
//! eframe's native runner on winit's GameActivity backend (wgpu: Vulkan, or GLES where Vulkan is
//! missing). The Rust library is `libphotocraft_android.so`; the Gradle project in `android/`
//! packages it with a small Activity (`PhotoCraftActivity.java`) that adds what winit doesn't
//! offer: document pickers, documents sent from other apps, and the pen's tilt and eraser end.
//! Build with `cargo xtask`-free scripts: see `README.md` in this directory.
//!
//! Differences from the desktop app:
//! - files are opened and saved through Android's document pickers ([`bridge`]); the app works
//!   on private copies and writes them back to the picked documents when saving;
//! - no TCP control server, MCP, clipboard images, screen color picking or Photoshop settings
//!   import;
//! - crash-recovery autosave also runs a few seconds after the last change, since Android
//!   closes apps in the background without warning (`app.rs`);
//! - pen pressure, tilt and the eraser end come from `MotionEvent`s (`photocraft-tablet`).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bridge;
pub mod fonts;
pub mod ime;
pub mod keys;
pub mod layout;
pub mod pad;
pub mod pen_path;
pub mod present;
pub mod services;
pub mod touch;
pub mod updates;

#[cfg(target_os = "android")]
mod app;
