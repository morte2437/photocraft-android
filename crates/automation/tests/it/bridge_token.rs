//! NV-6: the bridge never reveals the control token to whatever holds the port. It connects to
//! the literal loopback address the app binds (never a `localhost` lookup that may reach a
//! `[::1]` listener), and the server must prove it holds the token before the client sends its
//! own proof; the token itself never crosses the wire.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use photocraft_automation::{BridgeClient, Headless};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

const CONTROL_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// How a listener that is not the app answers the frames it receives.
#[derive(Clone, Copy)]
enum Impostor {
    /// Accepts and reads, never answers.
    Silent,
    /// Refuses every frame, as a server without challenge authentication does.
    Refuses,
    /// Answers the challenge with a made-up nonce and proof.
    FakesTheChallenge,
}

/// Accept one connection on `listener` and return every line it received (until the client
/// closes or goes quiet), answering as `kind`.
async fn impostor(listener: TcpListener, kind: Impostor) -> Vec<String> {
    let mut lines = Vec::new();
    let Ok(Ok((socket, _))) = tokio::time::timeout(Duration::from_secs(3), listener.accept()).await else {
        return lines;
    };
    let (read, mut write) = socket.into_split();
    let mut reader = BufReader::new(read);
    loop {
        let mut line = String::new();
        match tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut line)).await {
            Ok(Ok(n)) if n > 0 => {}
            _ => return lines,
        }
        let id = serde_json::from_str::<Value>(&line).map(|v| v["id"].clone()).unwrap_or(Value::Null);
        lines.push(line);
        let reply = match kind {
            Impostor::Silent => continue,
            Impostor::Refuses => json!({"id": id, "ok": false, "error": "authentication required"}),
            Impostor::FakesTheChallenge => json!({"id": id, "ok": true, "result": {"serverNonce": "ab".repeat(32), "serverProof": "cd".repeat(32)}}),
        };
        if write.write_all(format!("{reply}\n").as_bytes()).await.is_err() {
            return lines;
        }
    }
}

/// Call through a bridge to `addr` while `listener` plays `kind`; the lines it got, and the
/// call's error.
async fn call_impostor(addr: &str, listener: TcpListener, kind: Impostor) -> (Vec<String>, String) {
    let bridge = BridgeClient::new(addr, CONTROL_TOKEN).unwrap().with_timeout(Duration::from_secs(2));
    let call = tokio::spawn(async move { bridge.call("methods", json!({})).await });
    let lines = impostor(listener, kind).await;
    let error = match tokio::time::timeout(Duration::from_secs(10), call).await {
        Ok(Ok(Err(e))) => e.to_string(),
        other => format!("no error: {other:?}"),
    };
    (lines, error)
}

/// Fail if any line a listener received contains the control token.
fn assert_token_withheld(lines: &[String]) {
    for line in lines {
        assert!(!line.contains(CONTROL_TOKEN), "the control token reached a listener that is not the app: {line}");
    }
}

/// `localhost` is the 127.0.0.1 the app binds: a process listening on `[::1]` at the app's port
/// gets no connection, and the app does.
#[tokio::test(flavor = "multi_thread")]
async fn localhost_reaches_the_ipv4_app_not_an_ipv6_listener() {
    let app = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    app.set_nonblocking(true).unwrap();
    let port = app.local_addr().unwrap().port();
    let Ok(other) = TcpListener::bind(format!("[::1]:{port}")).await else {
        eprintln!("skipped: IPv6 loopback unavailable");
        return;
    };
    let bridge = BridgeClient::new(format!("localhost:{port}"), CONTROL_TOKEN).unwrap().with_timeout(Duration::from_secs(1));
    let call = tokio::spawn(async move { bridge.call("methods", json!({})).await });
    let lines = impostor(other, Impostor::Silent).await;
    call.abort();
    assert!(lines.is_empty(), "the [::1] listener was reached: {lines:?}");
    assert!(app.accept().is_ok(), "the app on 127.0.0.1 got the connection");
}

/// Whoever holds the port, on either loopback, and however it answers, never receives the
/// token: the client sends only its challenge and stops when the listener cannot prove it holds
/// the token.
#[tokio::test(flavor = "multi_thread")]
async fn a_listener_that_is_not_the_app_never_receives_the_token() {
    for (kind, expected) in [
        (Impostor::Silent, "the request was not sent"),
        (Impostor::Refuses, "refused challenge authentication"),
        (Impostor::FakesTheChallenge, "did not prove it holds this control token"),
    ] {
        let mut hosts = vec!["127.0.0.1"];
        if TcpListener::bind("[::1]:0").await.is_ok() {
            hosts.push("[::1]");
        }
        for host in hosts {
            let listener = TcpListener::bind(format!("{host}:0")).await.unwrap();
            let addr = listener.local_addr().unwrap().to_string();
            let (lines, error) = call_impostor(&addr, listener, kind).await;
            assert_token_withheld(&lines);
            assert_eq!(lines.len(), 1, "{addr}: only the challenge was sent: {lines:?}");
            assert!(lines[0].contains("auth.challenge"), "{addr}: {lines:?}");
            assert!(error.contains(expected), "{addr}: {error}");
            if !matches!(kind, Impostor::Silent) {
                assert!(error.contains("the control token was not sent"), "{addr}: {error}");
            }
        }
    }
}

/// The bridge authenticates to the headless `photocraft-cli serve` server by challenge.
#[tokio::test(flavor = "multi_thread")]
async fn the_bridge_authenticates_to_the_headless_server_by_challenge() {
    let headless = Arc::new(Mutex::new(Headless::new()));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = photocraft_automation::rpc::serve_tcp("127.0.0.1:0", headless, CONTROL_TOKEN.to_owned(), move |addr| {
            let _ = ready_tx.send(addr);
        });
    });
    let addr = ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let bridge = BridgeClient::new(addr.to_string(), CONTROL_TOKEN).unwrap().with_timeout(Duration::from_secs(5));
    let methods = bridge.call("methods", json!({})).await.unwrap();
    assert!(methods.as_array().unwrap().iter().any(|m| m == "batch"), "{methods}");
    // A second bridge with another token is refused before anything derived from its token is sent.
    let other = BridgeClient::new(addr.to_string(), "f".repeat(64)).unwrap().with_timeout(Duration::from_secs(5));
    let error = other.call("methods", json!({})).await.unwrap_err().to_string();
    assert!(error.contains("did not prove it holds this control token"), "{error}");
}
