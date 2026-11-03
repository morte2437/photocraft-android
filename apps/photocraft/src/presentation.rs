//! Launch overrides for how frames reach the display, so testers can compare input-to-pixel
//! latency without a rebuild (#1343): `PHOTOCRAFT_PRESENT_MODE` (`fifo`, `autovsync`, or `novsync`,
//! also spelled `immediate`, `mailbox` or `autonovsync`: no vsync, wgpu's Immediate → Mailbox → Fifo
//! by what the driver offers) and `PHOTOCRAFT_FRAME_LATENCY` (`1` to `3`, frames the swap chain may
//! queue). Unset, the platform default applies. A variable's value that isn't one of these is ignored
//! with a warning in the log, never a failed start; the `--present-mode` and `--frame-latency` flags
//! take the same values, and a bad one is a usage error (#7071). A mode the driver may not offer is
//! never requested.
//! The result is logged at start and shown in the stroke trace.

use eframe::egui_wgpu::SurfaceConfig;
use eframe::wgpu::PresentMode;

/// The present mode and frame latency this launch asked for (`None`: the default).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Overrides {
    pub mode: Option<PresentMode>,
    pub latency: Option<u32>,
}

/// A `--present-mode` / `PHOTOCRAFT_PRESENT_MODE` value, or `None` when it isn't one.
pub fn present_mode(v: &str) -> Option<PresentMode> {
    match v.trim().to_ascii_lowercase().as_str() {
        // Only modes every surface takes: wgpu crashes on an explicit Immediate, Mailbox or
        // FifoRelaxed the driver doesn't offer (X11/NVIDIA has no Mailbox, #1343). Without
        // vsync, wgpu picks Immediate, else Mailbox, else Fifo.
        "fifo" | "vsync" => Some(PresentMode::Fifo),
        "autovsync" => Some(PresentMode::AutoVsync),
        "novsync" | "immediate" | "mailbox" | "autonovsync" => Some(PresentMode::AutoNoVsync),
        _ => None,
    }
}

/// A `--frame-latency` / `PHOTOCRAFT_FRAME_LATENCY` value (1 to 3), or `None` when it isn't one.
pub fn frame_latency(v: &str) -> Option<u32> {
    v.trim().parse().ok().filter(|n| (1..=3).contains(n))
}

/// Parse the two variables' values. Returns the overrides and a warning per rejected value.
pub fn parse(mode: Option<&str>, latency: Option<&str>) -> (Overrides, Vec<String>) {
    let mut warnings = Vec::new();
    let mode = mode.map(str::trim).filter(|v| !v.is_empty()).and_then(|v| {
        let m = present_mode(v);
        if m.is_none() {
            warnings.push(format!("PHOTOCRAFT_PRESENT_MODE={v:?} isn't fifo, autovsync or novsync (immediate, mailbox); ignored"));
        }
        m
    });
    let latency = latency.map(str::trim).filter(|v| !v.is_empty()).and_then(|v| {
        let n = frame_latency(v);
        if n.is_none() {
            warnings.push(format!("PHOTOCRAFT_FRAME_LATENCY={v:?} isn't 1, 2 or 3; ignored"));
        }
        n
    });
    (Overrides { mode, latency }, warnings)
}

impl Overrides {
    /// `--present-mode` / `--frame-latency` when given (checked by the command line), else
    /// `PHOTOCRAFT_PRESENT_MODE` / `PHOTOCRAFT_FRAME_LATENCY`, whose rejected values are logged.
    pub fn from_args_or_env(mode: Option<PresentMode>, latency: Option<u32>) -> Self {
        let env = |given: bool, name: &str| if given { None } else { std::env::var(name).ok() };
        let (o, warnings) = parse(env(mode.is_some(), "PHOTOCRAFT_PRESENT_MODE").as_deref(), env(latency.is_some(), "PHOTOCRAFT_FRAME_LATENCY").as_deref());
        for w in warnings {
            log::warn!("{w}");
        }
        Self { mode: mode.or(o.mode), latency: latency.or(o.latency) }
    }

    /// `surface` with the overrides applied.
    pub fn apply(&self, surface: SurfaceConfig) -> SurfaceConfig {
        SurfaceConfig {
            present_mode: self.mode.unwrap_or(surface.present_mode),
            desired_maximum_frame_latency: self.latency.or(surface.desired_maximum_frame_latency),
        }
    }
}

/// How `surface` presents, for the stroke trace and System Info: "fifo, latency 2", with
/// "(override)" after a value an environment variable set. wgpu's GL backend ignores the latency.
pub fn describe(surface: &SurfaceConfig, overrides: &Overrides) -> String {
    let mode = match surface.present_mode {
        PresentMode::AutoVsync => "autovsync",
        PresentMode::AutoNoVsync => "autonovsync",
        PresentMode::Fifo => "fifo",
        PresentMode::FifoRelaxed => "fiforelaxed",
        PresentMode::Immediate => "immediate",
        PresentMode::Mailbox => "mailbox",
    };
    let latency = surface.desired_maximum_frame_latency.map_or("default".to_string(), |n| n.to_string());
    let tag = |set: bool| if set { " (override)" } else { "" };
    format!("{mode}{}, latency {latency}{}", tag(overrides.mode.is_some()), tag(overrides.latency.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_parse_and_bad_ones_are_ignored_with_a_warning() {
        assert_eq!(parse(None, None), (Overrides::default(), vec![]));
        assert_eq!(parse(Some(""), Some(" ")), (Overrides::default(), vec![]));
        let (o, w) = parse(Some(" Mailbox "), Some("1"));
        assert_eq!((o.mode, o.latency, w.len()), (Some(PresentMode::AutoNoVsync), Some(1), 0));
        assert_eq!(parse(Some("vsync"), None).0.mode, Some(PresentMode::Fifo));
        // Never a mode a driver may lack (wgpu crashes on it): no vsync falls back by support.
        for v in ["IMMEDIATE", "mailbox", "novsync", "autonovsync"] {
            assert_eq!(parse(Some(v), None).0.mode, Some(PresentMode::AutoNoVsync), "{v}");
        }
        for (mode, latency) in
            [(Some("fast"), None), (None, Some("0")), (None, Some("4")), (None, Some("-1")), (None, Some("one")), (None, Some("99999999999"))]
        {
            let (o, w) = parse(mode, latency);
            assert_eq!((o, w.len()), (Overrides::default(), 1), "{mode:?} {latency:?}");
        }
    }

    #[test]
    fn overrides_replace_only_what_they_set_and_are_described() {
        let default = SurfaceConfig::HIGH_THROUGHPUT;
        assert_eq!(Overrides::default().apply(default), default);
        let latency_only = Overrides { mode: None, latency: Some(1) };
        let s = latency_only.apply(default);
        assert_eq!((s.present_mode, s.desired_maximum_frame_latency), (PresentMode::AutoVsync, Some(1)));
        assert_eq!(describe(&s, &latency_only), "autovsync, latency 1 (override)");
        let novsync = Overrides { mode: Some(PresentMode::AutoNoVsync), latency: None };
        assert_eq!(describe(&novsync.apply(default), &novsync), "autonovsync (override), latency 2");
        let none = SurfaceConfig { present_mode: PresentMode::Fifo, desired_maximum_frame_latency: None };
        assert_eq!(describe(&none, &Overrides::default()), "fifo, latency default");
    }
}
