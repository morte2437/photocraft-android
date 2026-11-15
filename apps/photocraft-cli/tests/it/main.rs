//! The crate's integration tests, linked into one test binary.

mod cli;
mod gpu;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod malloc_tuning;

mod man_and_completions;
mod orientation;
mod psd_write_oracle;

/// Nextest archives relocate binaries; plain Cargo keeps the compile-time path valid.
fn binary() -> std::ffi::OsString {
    std::env::var_os("NEXTEST_BIN_EXE_photocraft_cli").unwrap_or_else(|| env!("CARGO_BIN_EXE_photocraft-cli").into())
}

/// The CLI, kept away from this machine's own PhotoCraft settings, ArtCraft Launcher and feature
/// flags: plain `mcp` works in the open app when the launcher's switch lets agents use it, and
/// would otherwise find (or start) the developer's own PhotoCraft. A test that needs a settings
/// folder of its own sets `PHOTOCRAFT_CONFIG_DIR` again; the last value wins.
fn command() -> std::process::Command {
    let base = std::env::temp_dir().join(format!("pc-cli-it-isolated-{}", std::process::id()));
    let settings = base.join("settings");
    let _ = std::fs::create_dir_all(&settings);
    let mut cmd = std::process::Command::new(binary());
    cmd.env("PHOTOCRAFT_CONFIG_DIR", &settings)
        .env("CRAFT_MCP_DIR", base.join("launcher"))
        .env("CRAFT_FLAGS_FILE", base.join("flags.toml"))
        .env_remove("CRAFT_FLAGS");
    cmd
}
