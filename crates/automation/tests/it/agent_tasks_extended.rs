//! M11 acceptance, part two: ten more scripted edit tasks (21–30) over MCP (roadmap M11 DoD).
//!
//! Same contract as `agent_tasks.rs`: each task uses only MCP tools (`doc_*`, `command_run`) the
//! way an agent would, and checks the outcome through `doc_inspect`, `document.pixel` or the
//! exported files, never engine internals.

use photocraft_automation::{AuthorizedWorkspace, PhotocraftMcp};
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

type Conn = RunningService<RoleClient, Client>;
type Server = tokio::task::JoinHandle<()>;

async fn connect(root: &std::path::Path) -> (Conn, Server) {
    let (s, c) = tokio::io::duplex(1 << 20);
    let workspace = AuthorizedWorkspace::new(Some(root), Some(root)).expect("test workspace");
    let server = tokio::spawn(async move {
        if let Ok(running) = PhotocraftMcp::headless_with_workspace(workspace).serve(s).await {
            let _ = running.waiting().await;
        }
    });
    (Client.serve(c).await.expect("client init"), server)
}

async fn shutdown(c: Conn, server: Server) {
    c.cancel().await.unwrap();
    server.await.unwrap();
}

fn text(r: &CallToolResult) -> String {
    r.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n")
}

/// Call a tool and parse its JSON result, failing the test on a tool error.
async fn tool(c: &Conn, name: &str, args: Value) -> Value {
    let mut p = CallToolRequestParams::new(name.to_owned());
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    let r = c.call_tool(p).await.expect("call_tool transport");
    assert_ne!(r.is_error, Some(true), "{name} failed: {}", text(&r));
    serde_json::from_str(&text(&r)).unwrap_or(Value::String(text(&r)))
}

async fn run(c: &Conn, id: &str, params: Value) -> Value {
    tool(c, "command_run", json!({"id": id, "params": params})).await
}

async fn inspect(c: &Conn) -> Value {
    tool(c, "doc_inspect", json!({})).await
}

async fn pixel(c: &Conn, x: i32, y: i32) -> Vec<f64> {
    let v = run(c, "document.pixel", json!({"x": x, "y": y})).await;
    let arr = if v.is_array() { v } else { v.get("result").cloned().unwrap_or(v) };
    arr.as_array().unwrap_or_else(|| panic!("pixel: {arr}")).iter().map(|x| x.as_f64().unwrap()).collect()
}

fn top_layer(doc: &Value) -> &Value {
    &doc["layers"][0]
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pc-agent-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn close(a: &[f64], b: &[f64], tol: f64) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

/// The next ten acceptance tasks (21–30): fill layers, adjustment layers with masks, grouping,
/// merging, gradients of edits, channel round trips, JPEG export and a native save round trip
/// that preserves a group.
#[tokio::test(flavor = "multi_thread")]
async fn agent_completes_ten_extended_tasks() {
    let dir = tmp("tasks3");
    let (c, server) = connect(&dir).await;

    // 21. Paint a red background and cover it with a solid-colour fill layer.
    tool(&c, "doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;
    run(&c, "select.all", json!({})).await;
    run(&c, "edit.fill", json!({"color": "#ff0000"})).await;
    run(&c, "select.deselect", json!({})).await;
    run(&c, "layer.newFillLayer.solidColor", json!({"color": "#00ffff"})).await;
    assert_eq!(top_layer(&inspect(&c).await)["kind"], "Fill", "task 21");
    let cyan = pixel(&c, 32, 32).await;
    assert!(cyan[1] > 0.99 && cyan[2] > 0.99 && cyan[0] < 0.01, "task 21: the fill covers red {cyan:?}");

    // 22. An Invert adjustment flips it to red; deleting the adjustment restores cyan.
    run(&c, "layer.newAdjustmentLayer.invert", json!({})).await;
    assert_eq!(top_layer(&inspect(&c).await)["kind"], "Adjustment", "task 22");
    let inverted = pixel(&c, 32, 32).await;
    assert!(inverted[0] > 0.99 && inverted[1] < 0.01 && inverted[2] < 0.01, "task 22: cyan inverts to red {inverted:?}");
    run(&c, "layer.delete", json!({})).await;
    assert!(close(&pixel(&c, 32, 32).await, &cyan, 1e-4), "task 22: deleting the adjustment restores cyan");

    // 23. Hide a rectangular selection behind the layer's mask.
    run(&c, "layer.delete", json!({})).await; // drop the fill so the red background shows
    run(&c, "select.rect", json!({"x": 16, "y": 16, "width": 32, "height": 32})).await;
    run(&c, "layer.layerMask.hideSelection", json!({})).await;
    let doc = inspect(&c).await;
    assert_eq!(top_layer(&doc)["hasMask"], true, "task 23");
    let hidden = pixel(&c, 32, 32).await;
    let kept = pixel(&c, 4, 4).await;
    assert!(hidden[3] < 0.01, "task 23: the selection is masked away {hidden:?}");
    assert!(kept[3] > 0.99 && kept[0] > 0.99, "task 23: outside keeps the red {kept:?}");
    run(&c, "layer.layerMask.delete", json!({})).await;

    // 24. Group every layer; the pixels are unchanged.
    let before = pixel(&c, 32, 32).await;
    run(&c, "select.allLayers", json!({})).await;
    run(&c, "layer.groupLayers", json!({})).await;
    let doc = inspect(&c).await;
    assert_eq!(top_layer(&doc)["kind"], "Group", "task 24: {doc}");
    assert!(close(&pixel(&c, 32, 32).await, &before, 1e-4), "task 24: grouping is visual only");

    // 25. Flatten the grouped document to one layer.
    run(&c, "layer.flattenImage", json!({})).await;
    assert_eq!(inspect(&c).await["layers"].as_array().unwrap().len(), 1, "task 25");

    // 26. Duplicate the layer and delete the copy.
    run(&c, "layer.duplicate", json!({})).await;
    assert_eq!(inspect(&c).await["layers"].as_array().unwrap().len(), 2, "task 26");
    run(&c, "layer.delete", json!({})).await;
    assert_eq!(inspect(&c).await["layers"].as_array().unwrap().len(), 1, "task 26");

    // 27. A Curves adjustment darkens the red; deleting it restores the original.
    run(&c, "layer.newAdjustmentLayer.curves", json!({"points": [[0, 0], [128, 32], [255, 128]]})).await;
    let dark = pixel(&c, 32, 32).await;
    assert!(dark[0] < 0.9, "task 27: the red channel deepens {dark:?}");
    run(&c, "layer.delete", json!({})).await;
    assert!(close(&pixel(&c, 32, 32).await, &before, 1e-4), "task 27: deleting restores the red");

    // 28. Hue/Saturation at -100 desaturates the red to gray.
    run(&c, "layer.newAdjustmentLayer.hueSaturation", json!({"saturation": -100})).await;
    let gray = pixel(&c, 32, 32).await;
    assert!((gray[0] - gray[1]).abs() < 0.05 && (gray[1] - gray[2]).abs() < 0.05, "task 28: saturated red goes gray {gray:?}");
    run(&c, "layer.delete", json!({})).await;

    // 29. Save a selection to a channel, load it back and paint through it.
    run(&c, "select.rect", json!({"x": 20, "y": 20, "width": 24, "height": 24})).await;
    run(&c, "select.saveSelection", json!({"name": "Box"})).await;
    run(&c, "select.deselect", json!({})).await;
    run(&c, "select.loadSelection", json!({"channel": "Box"})).await;
    run(&c, "edit.fill", json!({"color": "#0000ff"})).await;
    let painted = pixel(&c, 32, 32).await;
    let outside = pixel(&c, 4, 4).await;
    assert!(painted[2] > 0.99 && painted[0] < 0.01, "task 29: the loaded channel fills blue {painted:?}");
    assert!(outside[0] > 0.99 && outside[2] < 0.01, "task 29: outside keeps red {outside:?}");
    run(&c, "select.deselect", json!({})).await;

    // 30. Save natively with a group, reopen and find the group and its content intact.
    run(&c, "layer.new.layer", json!({"name": "Badge"})).await;
    run(&c, "select.rect", json!({"x": 40, "y": 40, "width": 16, "height": 16})).await;
    run(&c, "edit.fill", json!({"color": "#00ff00"})).await;
    run(&c, "select.deselect", json!({})).await;
    run(&c, "select.allLayers", json!({})).await;
    run(&c, "layer.groupLayers", json!({"name": "Set"})).await;
    tool(&c, "doc_save", json!({"path": "grouped.pcraft"})).await;
    tool(&c, "doc_close", json!({})).await;
    tool(&c, "doc_open", json!({"path": "grouped.pcraft"})).await;
    let doc = inspect(&c).await;
    assert_eq!(top_layer(&doc)["kind"], "Group", "task 30: {doc}");
    assert_eq!(top_layer(&doc)["name"], "Set", "task 30: {doc}");
    let green = pixel(&c, 48, 48).await;
    assert!(green[1] > 0.99 && green[0] < 0.01, "task 30: the badge inside the group survives {green:?}");
    tool(&c, "doc_close", json!({})).await;

    shutdown(c, server).await;
    std::fs::remove_dir_all(dir).unwrap();
}
