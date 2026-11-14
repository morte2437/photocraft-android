//! Preferences › AI Integrations (`agents.rs`): the live agent connection and its buttons.

use super::tests::app_with_store;
use super::*;
use std::sync::{Arc, Mutex};

#[test]
fn ai_integrations_start_and_stop_the_agent_connection_through_the_service() {
    use std::sync::mpsc::channel;
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    let calls: Arc<Mutex<Vec<prefs::Integrations>>> = Arc::default();
    let seen = calls.clone();
    let (tx, rx) = channel::<crate::ControlRequest>();
    let mut rx = Some(rx);
    app.services.agent_link = Some(Box::new(move |p: &prefs::Integrations, _: &egui::Context| {
        seen.lock().unwrap().push(p.clone());
        let mut out = crate::AgentLink { stopped: !p.connect_agents, ..Default::default() };
        if p.connect_agents {
            out.requests = rx.take();
            out.status.port = Some(4321);
            out.automation.command = Some(Box::new(|_, _| Err("policy".into())));
        }
        out
    }));
    // The first frame applies the saved settings (off), once.
    tick(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(app.control_rx.is_none() && app.agent_link.port.is_none());
    // On: the request channel and the automation policy are installed, without a restart.
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": true, "integrations.readRoot": "/work"}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(calls.lock().unwrap().last().map(|p| (p.connect_agents, p.read_root.clone())), Some((true, "/work".into())));
    assert!(app.control_rx.is_some());
    assert!(app.services.automation_command.is_some());
    assert_eq!(app.agent_link.port, Some(4321));
    assert!(tx.send(crate::ControlRequest::new("ui.inspect", json!({})).0).is_ok(), "the app holds the receiver");
    // An unrelated preference doesn't touch the connection.
    app.run("prefs.set", json!({"values": {"performance.historyStates": 12}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(calls.lock().unwrap().len(), 2);
    // Off: the channel and the policy go.
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": false}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(calls.lock().unwrap().len(), 3);
    assert!(app.control_rx.is_none() && app.services.automation_command.is_none());
    assert_eq!(app.agent_link.port, None);
    // ui.inspect reports it.
    assert!(crate::control::inspect(&app, &ctx).get("agentLink").is_some());
}

/// With the AI-MCP flag the ArtCraft Launcher's switch is the one consent: it starts the
/// connection, turning it off there stops it again, and a Control Port saved before (the old,
/// flag-less way) doesn't keep it on.
#[test]
fn with_the_flag_the_launcher_switch_alone_turns_the_agent_connection_on_and_off() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (mut app, _) = app_with_store();
    app.agent_link.mcp_flag = true;
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": true}})).unwrap();
    let ctx = egui::Context::default();
    let calls: Arc<Mutex<Vec<bool>>> = Arc::default();
    let seen = calls.clone();
    app.services.agent_link = Some(Box::new(move |p: &prefs::Integrations, _: &egui::Context| {
        seen.lock().unwrap().push(p.connect_agents);
        crate::AgentLink {
            stopped: !p.connect_agents,
            status: crate::AgentLinkStatus { port: p.connect_agents.then_some(4321), ..Default::default() },
            ..Default::default()
        }
    }));
    let launcher = Arc::new(AtomicBool::new(false));
    let on = launcher.clone();
    app.services.launcher_agents = Some(Box::new(move || on.load(Ordering::Relaxed)));
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), [false]);
    launcher.store(true, Ordering::Relaxed);
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), [false, true], "the launcher's switch alone starts it");
    assert!(app.agent_link.launcher && app.agent_link.port == Some(4321));
    tick(&mut app, &ctx);
    assert_eq!(calls.lock().unwrap().len(), 2, "no change, no restart");
    launcher.store(false, Ordering::Relaxed);
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), [false, true, false], "off in the launcher stops it");
    assert!(!app.agent_link.launcher && app.agent_link.port.is_none());
}

/// Without the AI-MCP flag nothing changes from before: the launcher's switch means nothing, and a
/// Control Port saved in the preferences keeps its connection.
#[test]
fn without_the_flag_the_launcher_switch_means_nothing() {
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    let calls: Arc<Mutex<Vec<bool>>> = Arc::default();
    let seen = calls.clone();
    app.services.agent_link = Some(Box::new(move |p: &prefs::Integrations, _: &egui::Context| {
        seen.lock().unwrap().push(p.connect_agents);
        crate::AgentLink { stopped: !p.connect_agents, ..Default::default() }
    }));
    app.services.launcher_agents = Some(Box::new(|| true));
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), [false], "the launcher's switch needs the flag");
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": true}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), [false, true], "a saved Control Port still counts");
}

/// The connection's updates keep the AI-MCP flag the app was started with: losing it would put the
/// page back to the settings it had before the flag after the first frame.
#[test]
fn connection_updates_keep_the_mcp_flag() {
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    app.services.agent_link = Some(Box::new(|_: &prefs::Integrations, _: &egui::Context| crate::AgentLink::default()));
    app.agent_link.mcp_flag = true;
    tick(&mut app, &ctx);
    assert!(app.agent_link.mcp_flag, "first frame");
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": true}})).unwrap();
    tick(&mut app, &ctx);
    assert!(app.agent_link.mcp_flag, "after a change");
}

/// With the flag the page has no switch of its own and nothing that sets up an MCP client: it says
/// where agents are turned on (the ArtCraft Launcher), and keeps the port, folders and closed-app
/// choice.
#[test]
fn ai_integrations_point_to_the_launcher_and_have_no_switch_or_client_buttons() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, _) = app_with_store();
    app.agent_link.mcp_flag = true;
    open_preferences(&mut app, "integrations");
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    for label in ["Let AI agents connect (MCP)", "Copy Configuration", "Add to Claude Desktop", "Copy Prompt for an Agent"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
    h.get_by_label_contains("ArtCraft Launcher › Settings › AI agents › Let AI agents use Craft apps");
    h.get_by_label("Off: turn on AI agents in ArtCraft Launcher › Settings › AI agents.");
    assert!(h.query_by_label("Folder agents can open from").is_some(), "the folders stay");
}

/// Without the suite's AI-MCP feature flag the page shows only the settings it had before.
#[test]
fn ai_integrations_without_the_mcp_flag_show_only_the_earlier_settings() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, _) = app_with_store();
    app.agent_link.mcp_flag = false;
    open_preferences(&mut app, "integrations");
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    assert!(h.query_by_label("Let AI agents connect (MCP)").is_none(), "the new connection switch waits for the flag");
    assert!(h.query_by_label("Folder agents can open from").is_none());
    assert!(h.query_by_label("Allow agent control").is_some(), "the earlier setting stays");
}

#[test]
fn ai_integrations_choices_and_paths_validate_through_prefs_set() {
    let (mut app, _) = app_with_store();
    for v in ["launch", "headless", "error"] {
        app.run("prefs.set", json!({"values": {"integrations.whenAppClosed": v}})).unwrap();
        assert_eq!(app.session.prefs().integrations.when_app_closed.name(), v);
    }
    assert!(app.run("prefs.set", json!({"values": {"integrations.whenAppClosed": "sometimes"}})).is_err());
    assert!(app.run("prefs.set", json!({"values": {"integrations.controlPort": 70000}})).is_err());
    app.run("prefs.set", json!({"values": {"integrations.writeRoot": "/tmp/out"}})).unwrap();
    assert_eq!(app.session.prefs().integrations.write_root, "/tmp/out");
}

/// A connection that is on but failed to start (its port taken) is tried again by itself, with the
/// same settings, and the automation gate follows the live policy.
#[test]
fn a_failed_start_is_retried_without_a_setting_change() {
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    let calls: Arc<Mutex<u32>> = Arc::default();
    let seen = calls.clone();
    app.services.agent_link = Some(Box::new(move |p: &prefs::Integrations, _: &egui::Context| {
        let mut n = seen.lock().unwrap();
        *n += 1;
        let mut out = crate::AgentLink::default();
        if p.connect_agents && *n >= 3 {
            out.status.port = Some(4321);
            out.automation.authorize = Some(|_, _| Ok(()));
        } else if p.connect_agents {
            out.status.error = Some("Couldn't listen on 127.0.0.1:4321: in use".into());
        }
        out
    }));
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": true}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), 1);
    assert!(app.agent_link.port.is_none() && app.prefs_rt.agent_link_retry.is_some(), "a retry is scheduled");
    // Not yet due: nothing happens.
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), 1);
    // Due: tried again with the same settings, still failing, then started.
    app.prefs_rt.agent_link_retry = Some(0.0);
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), 2);
    app.prefs_rt.agent_link_retry = Some(0.0);
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), 3);
    assert_eq!(app.agent_link.port, Some(4321));
    assert!(app.prefs_rt.agent_link_retry.is_none(), "running: no more retries");
    assert!(app.session.automation_gate.is_some(), "actions agents record keep the policy");
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), 3);
}
