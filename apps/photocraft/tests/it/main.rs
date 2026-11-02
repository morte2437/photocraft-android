//! The crate's integration tests, linked into one test binary.

mod command_line_usage;
mod control_exit_code;
mod startup_without_display;

/// Nextest archives relocate binaries; plain Cargo keeps the compile-time path valid.
fn binary() -> std::ffi::OsString {
    std::env::var_os("NEXTEST_BIN_EXE_photocraft").unwrap_or_else(|| env!("CARGO_BIN_EXE_photocraft").into())
}
