//! Agents in the open desktop app: while the person lets them in (ArtCraft Launcher › Settings ›
//! AI agents; without the `AI-MCP` flag, a Control Port saved before), the app serves the control
//! protocol by itself (no `--control`) and publishes the connection file `photocraft-cli mcp`
//! finds it by ([`photocraft_automation::agent_link`]). Turning it on or off, or changing the
//! port or folders in Preferences › AI Integrations, applies at once. The token stays in the settings
//! folder across launches (`agent-control.token`, as `--control-token-file` would keep it; delete
//! it for a new one). Agent requests get the policy and the folders chosen there, exactly as with
//! `--control` and the automation roots.
//!
//! Launch flags win: started with `--control`, the app doesn't install this service and the
//! dialog says the launch options manage the connection.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use photocraft_automation::agent_link::{self, Connection};
use photocraft_automation::security;
use photocraft_engine::prefs::Integrations;
use photocraft_ui_egui::{AgentLink, AgentLinkFn, ControlRequest};

use crate::control_server;

/// The connection while it runs; dropping it stops the server and removes the connection file.
struct Running {
    server: control_server::Listening,
    dir: PathBuf,
    /// The `controlPort` preference it was started for (0 = any).
    port_pref: u32,
}

impl Drop for Running {
    fn drop(&mut self) {
        agent_link::remove_connection(&self.dir, std::process::id());
        log::info!("agent connection on 127.0.0.1:{} stopped", self.server.port);
    }
}

/// Start serving agents on `port` (0 = any free port): the token from the settings folder
/// (created `0600` when missing), then the connection file. Returns the connection and the
/// channel its requests arrive on. A token file that can't be written leaves the connection off
/// (no server, so no token to leak) and says why.
fn start(dir: &Path, port: u16, ctx: &egui::Context, port_pref: u32) -> Result<(Running, Receiver<ControlRequest>), String> {
    let token_file = dir.join(agent_link::TOKEN_FILE);
    let token = security::server_token(None, Some(&token_file)).map_err(|e| format!("Agents can't connect: the token file could not be written: {e}"))?;
    let (requests, server) = control_server::listen(port, token, ctx.clone()).map_err(|e| format!("Couldn't listen on 127.0.0.1:{port}: {e}"))?;
    let running = Running { server, dir: dir.to_path_buf(), port_pref };
    agent_link::write_connection(dir, &Connection::new(running.server.port, token_file)).map_err(|e| format!("Couldn't publish the agent connection: {e}"))?;
    log::info!("agents can connect on 127.0.0.1:{}", running.server.port);
    Ok((running, requests))
}

/// The agent connection service (`settings_dir`: where the connection file and token go).
pub fn service(settings_dir: Option<PathBuf>) -> AgentLinkFn {
    let mut running: Option<Running> = None;
    Box::new(move |integrations: &Integrations, ctx: &egui::Context| apply(&mut running, settings_dir.as_deref(), integrations, ctx))
}

/// Bring the connection in line with `integrations`: start, restart on another port, stop, or only
/// change the folders agents may use.
fn apply(running: &mut Option<Running>, dir: Option<&Path>, integrations: &Integrations, ctx: &egui::Context) -> AgentLink {
    let mut out = AgentLink::default();
    if !integrations.connect_agents {
        out.stopped = running.take().is_some();
        return out;
    }
    // Allow Agent Control off refuses every control server, this one as `--control`.
    if !integrations.allow_agent_control {
        out.stopped = running.take().is_some();
        out.status.error = Some("Agents can't connect: Allow Agent Control is off in the preferences file.".into());
        return out;
    }
    let Some(dir) = dir else {
        out.stopped = running.take().is_some();
        out.status.error = Some("Agents can't connect: PhotoCraft has no settings folder (no home folder).".into());
        return out;
    };
    if running.as_ref().is_none_or(|r| r.port_pref != integrations.control_port) {
        out.stopped = running.take().is_some();
        // The preference's range is 0..=65535; anything else picks a free port.
        let port = u16::try_from(integrations.control_port).unwrap_or(0);
        match start(dir, port, ctx, integrations.control_port) {
            Ok((r, requests)) => {
                out.requests = Some(requests);
                *running = Some(r);
            }
            Err(e) => {
                log::warn!("{e}");
                out.status.error = Some(e);
                return out;
            }
        }
    }
    out.status.port = running.as_ref().map(|r| r.server.port);
    // Agent requests always go through the automation policy; without folders they get no file
    // access at all (as `--control` without automation roots). A folder that can't be opened, or
    // isn't an absolute path, is left out (that direction fails closed) and named, as at launch.
    let no_flags = photocraft_automation::args::AutomationArgs { automation_read_root: Vec::new(), automation_write_root: None };
    let workspace = match crate::automation_workspace(&no_flags, integrations) {
        Ok((workspace, warning)) => {
            if let Some(w) = warning {
                out.status.error = Some(format!("Agents can connect, but without file access there: {w}"));
            }
            workspace
        }
        Err(e) => {
            out.status.error = Some(format!("Agents can connect, but without file access: {e}"));
            photocraft_automation::AuthorizedWorkspace::default()
        }
    };
    out.automation = crate::services::automation(Some(workspace));
    out
}

/// How often [`launcher_watch`] looks at the ArtCraft Launcher's files.
const LAUNCHER_POLL: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether the ArtCraft Launcher turned agents on for every Craft app
/// ([`agent_link::launcher_allows_agents`]), watched on its own thread so the UI can ask every
/// frame without touching the disk. A change wakes the UI, which then starts or stops the
/// connection. Without a thread (one couldn't be started) the answer stays what it was at launch.
pub fn launcher_watch(ctx: &egui::Context) -> Box<dyn Fn() -> bool> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let on = Arc::new(AtomicBool::new(agent_link::launcher_allows_agents()));
    let (seen, ctx) = (on.clone(), ctx.clone());
    let started = std::thread::Builder::new().name("launcher-agents".into()).spawn(move || {
        loop {
            std::thread::sleep(LAUNCHER_POLL);
            let now = agent_link::launcher_allows_agents();
            if seen.swap(now, Ordering::Relaxed) != now {
                log::info!("ArtCraft Launcher: agents {}", if now { "on" } else { "off" });
                ctx.request_repaint();
            }
        }
    });
    if let Err(e) = started {
        log::warn!("couldn't watch the ArtCraft Launcher's agents switch: {e}");
    }
    Box::new(move || on.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("photocraft-agent-service-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn on(port: u32) -> Integrations {
        Integrations { connect_agents: true, control_port: port, ..Default::default() }
    }

    #[test]
    fn turning_agents_on_publishes_a_reachable_connection_and_off_removes_it() {
        let dir = temp();
        let ctx = egui::Context::default();
        let mut link = service(Some(dir.clone()));
        // Off (the default): nothing listens, nothing is published.
        let off = link(&Integrations::default(), &ctx);
        assert!(off.requests.is_none() && !off.stopped && off.status.port.is_none());
        assert!(off.automation.command.is_none(), "no agent policy without a connection");
        assert_eq!(agent_link::read_connection(&dir).unwrap(), None);

        let started = link(&on(0), &ctx);
        let port = started.status.port.unwrap();
        assert!(started.requests.is_some());
        assert!(started.automation.command.is_some() && started.automation.authorize.is_some(), "agent requests go through the policy");
        assert!(started.automation.read.is_some(), "the workspace answers (and refuses: no folder chosen)");
        let c = agent_link::read_connection(&dir).unwrap().unwrap();
        assert_eq!((c.port, c.pid), (port, std::process::id()));
        assert!(agent_link::probe(port, agent_link::PROBE_TIMEOUT));
        let token = security::client_token(None, Some(&c.token_file)).unwrap();

        // Folder changes keep the server and its token.
        let folders = Integrations { read_root: dir.to_string_lossy().into_owned(), ..on(0) };
        let changed = link(&folders, &ctx);
        assert!(changed.requests.is_none() && !changed.stopped);
        assert_eq!(changed.status.port, Some(port));
        assert!(changed.status.error.is_none(), "{:?}", changed.status.error);
        assert_eq!(security::client_token(None, Some(&c.token_file)).unwrap(), token);

        // A missing folder still connects, without file access, and says why.
        let missing = Integrations { write_root: dir.join("nope").to_string_lossy().into_owned(), ..on(0) };
        let partial = link(&missing, &ctx);
        assert_eq!(partial.status.port, Some(port));
        assert!(partial.status.error.as_deref().is_some_and(|e| e.contains("without file access")), "{:?}", partial.status.error);
        assert!(partial.automation.command.is_some());

        let stopped = link(&Integrations::default(), &ctx);
        assert!(stopped.stopped && stopped.status.port.is_none());
        assert_eq!(agent_link::read_connection(&dir).unwrap(), None, "the connection file goes with the server");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while agent_link::probe(port, agent_link::PROBE_TIMEOUT) {
            assert!(std::time::Instant::now() < deadline, "port {port} still open");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_port_restarts_with_the_same_token_and_a_taken_port_is_reported() {
        let dir = temp();
        let ctx = egui::Context::default();
        let mut link = service(Some(dir.clone()));
        let first = link(&on(0), &ctx);
        let first_port = first.status.port.unwrap();
        let token_file = dir.join(agent_link::TOKEN_FILE);
        let first_token = std::fs::read_to_string(&token_file).unwrap();
        // A taken port: the error names it and nothing is published for it.
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken_port = taken.local_addr().unwrap().port();
        let failed = link(&on(u32::from(taken_port)), &ctx);
        assert!(failed.stopped && failed.requests.is_none() && failed.status.port.is_none());
        assert!(failed.status.error.as_deref().is_some_and(|e| e.contains(&taken_port.to_string())), "{:?}", failed.status.error);
        assert_eq!(agent_link::read_connection(&dir).unwrap(), None);
        drop(taken);
        // Back to any port: the token in the settings folder is kept, so a client holding it
        // (a `--bridge` setup reading the token file) needn't be set up again.
        let again = link(&on(0), &ctx);
        assert!(again.requests.is_some());
        assert!(again.status.port.is_some_and(|p| p != 0));
        assert_eq!(std::fs::read_to_string(&token_file).unwrap(), first_token);
        let _ = first_port;
        drop(link);
        assert_eq!(agent_link::read_connection(&dir).unwrap(), None, "quitting removes the connection file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A token file that can't be written (here the "settings folder" is a file) leaves the
    /// connection off with the reason shown, and its port free.
    #[test]
    fn a_token_file_that_cannot_be_written_leaves_agents_off() {
        let dir = temp();
        let settings = dir.join("not-a-folder");
        std::fs::write(&settings, b"").unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut link = service(Some(settings));
        let r = link(&on(u32::from(port)), &egui::Context::default());
        assert!(r.requests.is_none() && r.status.port.is_none());
        assert!(r.status.error.as_deref().is_some_and(|e| e.contains("token file")), "{:?}", r.status.error);
        std::net::TcpListener::bind(("127.0.0.1", port)).expect("the port was let go");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A relative folder would depend on the folder the app was started from: left out and named.
    #[test]
    fn a_relative_folder_is_left_out_with_a_reason() {
        let dir = temp();
        let mut link = service(Some(dir.clone()));
        let r = link(&Integrations { read_root: ".".into(), ..on(0) }, &egui::Context::default());
        assert!(r.status.port.is_some());
        assert!(r.status.error.as_deref().is_some_and(|e| e.contains("not an absolute path")), "{:?}", r.status.error);
        drop(link);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Allow Agent Control off (in the file; the dialog doesn't show it) refuses this server too.
    #[test]
    fn allow_agent_control_off_keeps_agents_out() {
        let dir = temp();
        let mut link = service(Some(dir.clone()));
        let r = link(&Integrations { allow_agent_control: false, ..on(0) }, &egui::Context::default());
        assert!(r.requests.is_none() && r.status.port.is_none() && r.automation.command.is_none());
        assert!(r.status.error.as_deref().is_some_and(|e| e.contains("Allow Agent Control")), "{:?}", r.status.error);
        assert_eq!(agent_link::read_connection(&dir).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_settings_folder_is_an_error_not_a_panic() {
        let mut link = service(None);
        let r = link(&on(0), &egui::Context::default());
        assert!(r.requests.is_none() && r.status.error.is_some());
    }
}
