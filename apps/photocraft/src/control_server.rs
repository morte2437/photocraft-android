//! Localhost JSON-lines control server (one request per line, one reply per line).
//! Loopback only. This is the transport the MCP server will wrap.

use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use photocraft_automation::budgets::write_reply;
use photocraft_automation::security::{
    AUTH_DEADLINE, AuthDeadlineStream, ConnectionLimiter, LineRead, MAX_CONNECTIONS, MAX_REQUEST_BYTES, authentication_reply, configure_stream,
    read_bounded_line, repeat_auth_reply,
};
use photocraft_ui_egui::ControlRequest;
use photocraft_ui_egui::control::REPLY_DEADLINE;
use serde_json::{Value, json};

/// Start serving a listener already bound during startup, before a window is opened.
pub fn start(listener: TcpListener, token: String, ctx: egui::Context) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let port = listener.local_addr().map_or(0, |a| a.port());
    eprintln!("photocraft: control server listening on {}", listening_addr(&listener, port));
    std::thread::spawn(move || {
        let limiter = ConnectionLimiter::new(MAX_CONNECTIONS);
        let token = Arc::new(token);
        for mut stream in listener.incoming().flatten() {
            let Some(permit) = limiter.try_acquire() else {
                let _ = configure_stream(&stream);
                let _ = writeln!(stream, "{}", json!({"id": null, "ok": false, "error": "connection limit reached"}));
                continue;
            };
            let tx = tx.clone();
            let ctx = ctx.clone();
            let token = Arc::clone(&token);
            std::thread::spawn(move || {
                let _permit = permit;
                serve(stream, &token, tx, ctx);
            });
        }
    });
    rx
}

/// The address `listener` is bound to: with `--control 0` the OS picks the port, and a client
/// needs that one, not the requested 0 (#6427). Falls back to the requested port if the OS
/// can't report it.
fn listening_addr(listener: &TcpListener, requested_port: u16) -> String {
    match listener.local_addr() {
        Ok(addr) => addr.to_string(),
        Err(_) => format!("127.0.0.1:{requested_port}"),
    }
}

fn serve(stream: TcpStream, token: &str, tx: Sender<ControlRequest>, ctx: egui::Context) {
    serve_with_auth_deadline(stream, token, tx, ctx, AUTH_DEADLINE);
}

/// [`serve`] for a connection that must complete `auth` within `auth_deadline` of connecting,
/// however much it sends meanwhile (#4965); after that only the idle timeout applies.
fn serve_with_auth_deadline(stream: TcpStream, token: &str, tx: Sender<ControlRequest>, ctx: egui::Context, auth_deadline: Duration) {
    if configure_stream(&stream).is_err() {
        return;
    }
    let Ok(read) = stream.try_clone() else { return };
    let mut reader = BufReader::new(AuthDeadlineStream::new(read, auth_deadline));
    let mut out = stream;
    let mut line = String::new();
    let mut authenticated = false;
    loop {
        match read_bounded_line(&mut reader, &mut line) {
            Ok(LineRead::Eof) | Err(_) => break,
            Ok(LineRead::TooLong) => {
                let reply = json!({
                    "id": null,
                    "ok": false,
                    "error": format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
                });
                let _ = write_reply(&mut out, &reply);
                let _ = out.flush();
                break;
            }
            Ok(LineRead::Line) if line.trim().is_empty() => continue,
            Ok(LineRead::Line) => {}
        }
        if !authenticated {
            let (reply, ok) = authentication_reply(&line, token);
            authenticated = ok;
            if ok && reader.get_mut().authenticated().is_err() {
                break;
            }
            if write_reply(&mut out, &reply).is_err() || out.flush().is_err() || !authenticated {
                break;
            }
            continue;
        }
        if let Some(reply) = repeat_auth_reply(&line) {
            if write_reply(&mut out, &reply).is_err() || out.flush().is_err() {
                break;
            }
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => {
                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let deadline = Instant::now() + REPLY_DEADLINE;
                let (mut req, rrx) = ControlRequest::new(method, params);
                req.deadline = Some(deadline);
                let dispatch = req.dispatch.clone();
                if tx.send(req).is_err() {
                    break;
                }
                ctx.request_repaint();
                let mut r =
                    rrx.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_else(|_| json!({"ok": false, "error": dispatch.give_up()}));
                if let Some(o) = r.as_object_mut() {
                    o.insert("id".into(), id);
                }
                r
            }
            Err(e) => json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}),
        };
        if write_reply(&mut out, &reply).is_err() {
            break;
        }
        if out.flush().is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    /// #6427: with port 0 the log names the port the OS bound, which a client can connect to.
    #[test]
    fn port_zero_logs_the_bound_port() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let bound = listener.local_addr().unwrap();
        assert_ne!(bound.port(), 0);
        let logged = listening_addr(&listener, 0);
        assert_eq!(logged, format!("127.0.0.1:{}", bound.port()));
        TcpStream::connect(&logged).unwrap();
    }

    #[test]
    fn a_prebound_listener_authenticates_and_dispatches_requests() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = start(listener, TOKEN.into(), egui::Context::default());
        let mut stream = TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut replies = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).unwrap();
        let mut line = String::new();
        replies.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["result"]["authenticated"], true);
        writeln!(stream, "{}", json!({"id": 2, "method": "test.bound"})).unwrap();
        let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(request.method, "test.bound");
        request.reply.send(json!({"ok": true, "result": "served"})).unwrap();
        line.clear();
        replies.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], 2);
        assert_eq!(reply["result"], "served");
    }

    /// #4965: a connection that keeps sending without authenticating is closed at the deadline
    /// (releasing its connection permit), and nothing it sent is dispatched.
    #[test]
    fn an_unauthenticated_connection_is_closed_at_the_deadline_whatever_it_sends() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let started = Instant::now();
            serve_with_auth_deadline(stream, "unused", tx, egui::Context::default(), Duration::from_millis(300));
            started.elapsed()
        });
        // Blank lines and partial frames, one byte every 20 ms: each read succeeds long before
        // the idle timeout.
        let mut stream = TcpStream::connect(addr).unwrap();
        for i in 0..150 {
            let byte: &[u8] = if i % 5 == 0 { b"\n" } else { b" " };
            if stream.write_all(byte).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let served = server.join().unwrap();
        assert!(served < Duration::from_secs(3), "the trickle did not extend the deadline: {served:?}");
        assert!(rx.try_recv().is_err(), "nothing was dispatched");
    }

    #[test]
    fn oversized_reply_preserves_framing_id_and_the_next_control_request() {
        use photocraft_automation::budgets::MAX_RESPONSE_BYTES;
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
        });
        let handler = std::thread::spawn(move || {
            let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(first.method, "test.large");
            assert!(first.deadline.is_some_and(|deadline| deadline > std::time::Instant::now()), "native requests carry their reply deadline");
            first.reply.send(json!({"ok": true, "result": "x".repeat(MAX_RESPONSE_BYTES)})).unwrap();
            let second = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(second.method, "test.small");
            second.reply.send(json!({"ok": true, "result": "still serving"})).unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], true);
        writeln!(stream, "{}", json!({"id": 2, "method": "test.large"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let rejected: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(rejected["id"], 2);
        assert_eq!(rejected["ok"], false);
        assert!(rejected["error"].as_str().unwrap().contains("operation may have completed"));
        writeln!(stream, "{}", json!({"id": 3, "method": "test.small"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let accepted: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(accepted["id"], 3);
        assert_eq!(accepted["result"], "still serving");
        drop(reader);
        drop(stream);
        handler.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn bad_json_reply_carries_a_null_id_and_the_connection_keeps_serving() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
        });
        let handler = std::thread::spawn(move || {
            let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(req.method, "test.after");
            req.reply.send(json!({"ok": true, "result": "still serving"})).unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream, "{}", json!({"id": "auth", "method": "auth", "params": {"token": TOKEN}})).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], true);
        writeln!(stream, "{{not json}}").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let bad: Value = serde_json::from_str(&line).unwrap();
        let bad = bad.as_object().unwrap();
        assert_eq!(bad.get("id"), Some(&Value::Null), "bad JSON reply must carry `id: null`: {line}");
        assert_eq!(bad.get("ok"), Some(&Value::Bool(false)));
        assert!(bad.get("error").and_then(Value::as_str).unwrap().starts_with("bad JSON: "));
        writeln!(stream, "{}", json!({"id": 7, "method": "test.after"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let after: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(after["id"], 7);
        assert_eq!(after["result"], "still serving");
        drop(reader);
        drop(stream);
        handler.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn repeat_auth_reports_already_authenticated_without_dispatching() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
        });
        let handler = std::thread::spawn(move || {
            let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(req.method, "test.after", "a repeat auth must not reach the UI dispatcher");
            req.reply.send(json!({"ok": true, "result": "still serving"})).unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        for id in ["auth", "again"] {
            writeln!(stream, "{}", json!({"id": id, "method": "auth", "params": {"token": TOKEN}})).unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
        }
        let again: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(again["id"], "again");
        assert_eq!(again["ok"], false);
        assert_eq!(again["error"], "already authenticated");
        writeln!(stream, "{}", json!({"id": 9, "method": "test.after"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let after: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(after["id"], 9);
        assert_eq!(after["result"], "still serving");
        drop(reader);
        drop(stream);
        handler.join().unwrap();
        server.join().unwrap();
    }
}
