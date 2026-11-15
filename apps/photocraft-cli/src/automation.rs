use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};

use serde_json::Value;

use photocraft_automation::{Headless, PhotocraftMcp, security};

use crate::{
    Result,
    args::{CommandsArgs, McpArgs, ServeArgs},
    output::{JsonStyle, print_json},
};

pub(crate) fn commands(a: &CommandsArgs, out: &mut dyn Write) -> Result {
    let h = Headless::new();
    let list = h.command_list();
    let needle = a.filter.as_deref().map(str::to_lowercase);
    let items: Vec<&Value> = list
        .as_array()
        .map(|v| v.iter().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|c| needle.as_ref().is_none_or(|n| photocraft_engine::commands::list_entry_matches(c, n)))
        .collect();
    if a.json {
        return print_json(out, &Value::Array(items.into_iter().cloned().collect()), JsonStyle::Pretty);
    }
    for c in items {
        let menu = c["menu"].as_array().map(|m| m.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" > ")).unwrap_or_default();
        writeln!(out, "{:<44} {:<32} {}", c["id"].as_str().unwrap_or(""), c["label"].as_str().unwrap_or(""), menu)?;
    }
    Ok(())
}

pub(crate) fn serve(a: &ServeArgs, err: &mut dyn Write) -> Result {
    use std::sync::{Arc, Mutex};
    let mut h = Headless::with_workspace(a.automation.workspace()?);
    let policy = crate::gpu::GpuPolicy::from_args(&a.gpu).map_err(crate::CliError::Other)?;
    policy.apply_to_headless(&mut h);
    for d in h.drain_diagnostics() {
        let _ = writeln!(err, "warning: {d}");
    }
    let h = Arc::new(Mutex::new(h));
    match a.port {
        Some(port) => {
            if a.port_from_env {
                // Name the mode switch: an exported variable changes what this command does, and
                // unsetting it (or setting it empty) is the way back to stdio.
                let _ = writeln!(err, "photocraft-cli: PHOTOCRAFT_CONTROL_PORT={port} is set; serving on TCP instead of stdio (unset it to serve stdio)");
            }
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port)).to_string();
            let (supplied, token_file) = (a.control.control_token.as_deref(), a.control.control_token_file.as_deref());
            let token = security::server_token(supplied, token_file)?;
            if let Some(path) = token_file {
                let _ = writeln!(err, "photocraft-cli control token file: {}", path.display());
            } else if supplied.is_none() {
                let _ = writeln!(err, "photocraft-cli control token: {token}");
            }
            photocraft_automation::rpc::serve_tcp(&addr, h, token, |local| {
                let _ = writeln!(err, "photocraft-cli serving on {local}");
            })?;
            Ok(())
        }
        None => {
            let stdin = std::io::stdin();
            Ok(photocraft_automation::rpc::serve_lines(&h, stdin.lock(), std::io::stdout())?)
        }
    }
}

/// `mcp`: with the suite's `AI-MCP` flag, the open window when agents may use it, otherwise
/// headless (`mcp_live`); without the flag headless, as before it; `--headless` always headless;
/// `--bridge` (or PHOTOCRAFT_CONTROL_PORT) a `photocraft --control` app.
pub(crate) fn mcp(a: &McpArgs, err: &mut dyn Write) -> Result {
    let server = match &a.bridge {
        None if !a.headless && photocraft_automation::agent_link::mcp_flag() => PhotocraftMcp::live(live_config(a)?),
        Some(addr) => {
            if a.bridge_from_env {
                // Name the mode switch, as `serve` does: the way back to headless is to unset it.
                let _ = writeln!(
                    err,
                    "photocraft-cli: PHOTOCRAFT_CONTROL_PORT is set; bridging to the app on {addr} instead of a headless engine (unset it to run headless)"
                );
            }
            let token = security::client_token(a.control.control_token.as_deref(), a.control.control_token_file.as_deref())?;
            PhotocraftMcp::bridge(&addr.to_string(), &token)?
        }
        None => {
            let mut h = Headless::with_workspace(a.automation.workspace()?);
            let policy = crate::gpu::GpuPolicy::from_args(&a.gpu).map_err(crate::CliError::Other)?;
            policy.apply_to_headless(&mut h);
            for d in h.drain_diagnostics() {
                let _ = writeln!(err, "warning: {d}");
            }
            PhotocraftMcp::with_headless(h)
        }
    };
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    Ok(rt.block_on(server.serve_stdio())?)
}

/// `mcp`: the app's settings folder, the closed-app override, the app launcher, and the headless
/// session's roots and rendering for when it works without the window.
fn live_config(a: &McpArgs) -> Result<photocraft_automation::live::LiveConfig> {
    let other = |m: &str| crate::CliError::Other(m.to_string());
    let dir = photocraft_automation::app_dirs::from_env()
        .dir
        .ok_or_else(|| other("can't find PhotoCraft's settings folder (no home folder; set PHOTOCRAFT_CONFIG_DIR)"))?;
    let mut config = photocraft_automation::live::LiveConfig::new(dir);
    config.when_closed = a.when_closed;
    // As a headless `mcp` always did: folders that can't be opened stop the start, before the
    // server answers anything, rather than at the first call that would fall back to them.
    a.automation.workspace()?;
    config.read_roots.clone_from(&a.automation.automation_read_root);
    config.write_root = a.automation.automation_write_root.clone();
    config.launch = std::env::current_exe().ok().and_then(|exe| photocraft_automation::live::launcher(&exe));
    let policy = crate::gpu::GpuPolicy::from_args(&a.gpu).map_err(crate::CliError::Other)?;
    config.prepare_headless = Some(Box::new(move |h| {
        policy.apply_to_headless(h);
        // stdout carries the protocol; the warnings a headless start has go to stderr.
        let mut err = std::io::stderr();
        for d in h.drain_diagnostics() {
            let _ = writeln!(err, "warning: {d}");
        }
    }));
    Ok(config)
}
