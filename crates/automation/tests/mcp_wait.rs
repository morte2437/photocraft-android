//! Calls that wait on a long command: headless (#8129), with progress, other calls served
//! meanwhile and cancellation that leaves the document unchanged; and over the bridge (#8130),
//! polling the app's job past the reply timeout. Driven over the real line transport.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use photocraft_automation::security::authentication_reply;
use photocraft_automation::{Backend, BridgeClient, PhotocraftMcp};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};

struct Wire {
    input: DuplexStream,
    lines: Lines<BufReader<DuplexStream>>,
    server: tokio::task::JoinHandle<Result<(), photocraft_automation::AutomationError>>,
}

impl Wire {
    async fn start(mcp: PhotocraftMcp) -> Wire {
        let (input, server_in) = tokio::io::duplex(1 << 20);
        let (server_out, output) = tokio::io::duplex(1 << 20);
        let server = tokio::spawn(mcp.serve_io(server_in, server_out));
        let mut wire = Wire { input, lines: BufReader::new(output).lines(), server };
        wire.send(json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{"protocolVersion":"2025-06-18", "capabilities":{}, "clientInfo":{"name":"test", "version":"1"}}}))
            .await;
        wire.next().await;
        wire.send(json!({"jsonrpc":"2.0", "method":"notifications/initialized"})).await;
        wire
    }

    async fn send(&mut self, value: Value) {
        self.input.write_all(format!("{value}\n").as_bytes()).await.unwrap();
    }

    async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(60), self.lines.next_line()).await.unwrap().unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    }

    async fn call(&mut self, id: u64, name: &str, args: Value, token: Option<&str>) {
        let mut request = json!({"jsonrpc":"2.0", "id":id, "method":"tools/call", "params":{"name":name, "arguments":args}});
        if let Some(token) = token {
            request["params"]["_meta"] = json!({"progressToken": token});
        }
        self.send(request).await;
    }

    /// The reply to `id`, skipping progress notes. `pending` must not answer meanwhile.
    async fn reply(&mut self, id: u64, pending: Option<u64>) -> Value {
        let (text, error) = self.outcome(id, pending).await;
        assert!(!error, "{text}");
        serde_json::from_str(&text).unwrap()
    }

    /// The text of the tool result for `id` and whether it is an error.
    async fn outcome(&mut self, id: u64, pending: Option<u64>) -> (String, bool) {
        loop {
            let message = self.next().await;
            if pending.is_some_and(|p| message["id"] == p) {
                panic!("request {} answered while it should still run: {message}", pending.unwrap_or(0));
            }
            if message["id"] == id {
                let text = message["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{message}")).to_owned();
                return (text, message["result"]["isError"] == true);
            }
        }
    }

    async fn cancel(&mut self, id: u64) {
        self.send(json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":id}})).await;
    }

    /// Poll `jobs_list` until no job runs; `cancelled` must never be answered.
    async fn settle(&mut self, mut id: u64, cancelled: u64) -> Value {
        loop {
            self.call(id, "jobs_list", json!({}), None).await;
            let jobs = self.reply(id, Some(cancelled)).await;
            if !jobs["jobs"].as_array().unwrap().iter().any(|j| j["state"] == "running") {
                return jobs;
            }
            id += 1;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn history(&mut self, id: u64) -> Vec<Value> {
        self.call(id, "doc_inspect", json!({}), None).await;
        self.reply(id, None).await["history"].as_array().unwrap().clone()
    }

    /// A document whose blur takes seconds even in an optimized build (about 2s here).
    async fn big_document(&mut self) {
        self.call(2, "doc_new", json!({"width": 6000, "height": 6000}), None).await;
        self.reply(2, None).await;
        self.call(3, "command_run", json!({"id": "filter.noise.addNoise", "params": {"amount": 50}}), None).await;
        self.reply(3, None).await;
    }

    /// Wait for the first progress note of request `id` (so its job is running).
    async fn first_progress(&mut self, id: u64, token: &str, total: f64) -> f64 {
        loop {
            let note = self.next().await;
            assert_ne!(note["id"], id, "request {id} finished before it could be cancelled: {note}");
            if note["method"] == "notifications/progress" {
                assert_eq!(note["params"]["progressToken"], token, "{note}");
                assert_eq!(note["params"]["total"].as_f64(), Some(total), "{note}");
                return note["params"]["progress"].as_f64().unwrap();
            }
        }
    }

    async fn finish(self) {
        drop(self.input);
        tokio::time::timeout(Duration::from_secs(10), self.server).await.unwrap().unwrap().unwrap();
    }
}

const BLUR: fn() -> Value = || json!({"id": "filter.blur.gaussianBlur", "params": {"radius": 1000}});

#[tokio::test(flavor = "multi_thread")]
async fn a_waited_command_reports_progress_serves_other_calls_and_cancels_cleanly() {
    let mut wire = Wire::start(PhotocraftMcp::headless()).await;
    wire.big_document().await;
    let before = wire.history(4).await;

    wire.call(5, "command_run", BLUR(), Some("blur")).await;
    let progress = wire.first_progress(5, "blur", 1.0).await;
    assert!((0.0..1.0).contains(&progress), "{progress}");
    // Served while the blur runs: before #8129 both waited for it behind the session lock.
    wire.call(6, "jobs_list", json!({}), None).await;
    let jobs = wire.reply(6, Some(5)).await;
    assert!(jobs["jobs"].as_array().unwrap().iter().any(|j| j["state"] == "running" && j["command"] == "filter.blur.gaussianBlur"), "{jobs}");
    assert_eq!(wire.history(7).await, before);

    wire.cancel(5).await;
    let jobs = wire.settle(8, 5).await;
    assert!(jobs["jobs"].as_array().unwrap().iter().any(|j| j["state"] == "cancelled"), "{jobs}");
    // Before #8129 the cancelled blur still ran to the end and added its history step.
    assert_eq!(wire.history(1000).await, before, "a cancelled command leaves the document unchanged");

    // The session keeps working, and a waited command that finishes returns its result.
    wire.call(1001, "doc_new", json!({"width": 64, "height": 64}), None).await;
    wire.reply(1001, None).await;
    wire.call(1002, "command_run", json!({"id": "filter.blur.gaussianBlur", "params": {"radius": 2}}), Some("short")).await;
    wire.reply(1002, None).await;
    assert_eq!(wire.history(1003).await.last(), Some(&json!("Gaussian Blur")));
    wire.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_batch_cancels_its_waited_step_and_runs_no_later_step() {
    let mut wire = Wire::start(PhotocraftMcp::headless()).await;
    wire.big_document().await;
    let before = wire.history(4).await;

    let steps = json!([BLUR(), {"id": "layer.new.layer", "params": {"name": "after"}}]);
    wire.call(5, "command_batch", json!({"steps": steps, "stop_on_error": false}), Some("batch")).await;
    assert!(wire.first_progress(5, "batch", 2.0).await < 1.0, "still in the first step");
    wire.cancel(5).await;
    wire.settle(6, 5).await;
    // Give a (wrongly) continuing batch time to start its next step.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(wire.history(1000).await, before, "neither the blur nor the later step ran");
    wire.finish().await;
}

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A scripted desktop app for bridge mode. A blur runs as job 7 for `runs`: `engine.execute`
/// with `wait: false` answers `{job, pending}` at once, without it only when the job ends, like
/// the app. `jobs.list` reports the job and `jobs.cancel` stops it. Other commands answer at
/// once. Every request is logged.
struct FakeApp {
    runs: Duration,
    /// Close the connection instead of answering the start (its reply is lost).
    lose_start: bool,
    /// Lose the first cancellation request before applying it; a later request can succeed.
    lose_cancel: bool,
    /// How long to hold each `jobs.list` reply (an app slow to answer).
    hold_polls: Duration,
    started: Option<Instant>,
    cancelled: bool,
    log: Vec<Value>,
}

impl FakeApp {
    async fn serve(runs: Duration, lose_start: bool) -> (String, Arc<Mutex<FakeApp>>) {
        let app = Arc::new(Mutex::new(FakeApp {
            runs,
            lose_start,
            lose_cancel: false,
            hold_polls: Duration::ZERO,
            started: None,
            cancelled: false,
            log: Vec::new(),
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let shared = app.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let app = shared.clone();
                tokio::spawn(async move {
                    let (read, mut write) = socket.into_split();
                    let mut lines = BufReader::new(read).lines();
                    let (auth, ok) = authentication_reply(&lines.next_line().await.unwrap().unwrap(), TOKEN);
                    assert!(ok, "each connection must authenticate");
                    write.write_all(format!("{auth}\n").as_bytes()).await.unwrap();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let request: Value = serde_json::from_str(&line).unwrap();
                        let Some((result, after)) = app.lock().unwrap().answer(&request) else { return };
                        tokio::time::sleep(after).await;
                        let reply = json!({"id": request["id"], "ok": true, "result": result});
                        if write.write_all(format!("{reply}\n").as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (addr, app)
    }

    /// The result for `request` and how long to hold it; `None` closes the connection.
    fn answer(&mut self, request: &Value) -> Option<(Value, Duration)> {
        self.log.push(request.clone());
        let p = &request["params"];
        let result = match request["method"].as_str().unwrap() {
            "engine.execute" if p["wait"] == false && p["command"] == "filter.blur.gaussianBlur" => {
                if self.lose_start {
                    return None;
                }
                self.started = Some(Instant::now());
                json!({"job": 7, "pending": true})
            }
            "engine.execute" if p["command"] == "filter.blur.gaussianBlur" => {
                self.started = Some(Instant::now());
                return Some((json!({"blurred": true}), self.runs));
            }
            "engine.execute" => json!({"ran": p["command"]}),
            "jobs.list" => {
                let Some(started) = self.started else { return Some((json!({"jobs": []}), Duration::ZERO)) };
                let progress = started.elapsed().as_secs_f64() / self.runs.as_secs_f64();
                let job = match () {
                    () if self.cancelled => json!({"id": 7, "state": "cancelled", "progress": progress.min(1.0), "message": ""}),
                    () if progress >= 1.0 => json!({"id": 7, "state": "done", "progress": 1.0, "message": "", "result": {"blurred": true}}),
                    () => json!({"id": 7, "state": "running", "progress": progress, "message": "Blurring"}),
                };
                return Some((json!({"jobs": [job]}), self.hold_polls));
            }
            "jobs.cancel" => {
                if self.lose_cancel {
                    self.lose_cancel = false;
                    return None;
                }
                self.cancelled = true;
                json!({"cancelled": 1})
            }
            method => panic!("unexpected `{method}`"),
        };
        Some((result, Duration::ZERO))
    }

    /// The logged requests for `method`.
    fn sent(app: &Mutex<FakeApp>, method: &str) -> Vec<Value> {
        app.lock().unwrap().log.iter().filter(|r| r["method"] == method).map(|r| r["params"].clone()).collect()
    }
}

/// An MCP server bridged to `addr` whose every call times out after 300ms.
fn bridged(addr: &str) -> PhotocraftMcp {
    PhotocraftMcp::with_backend(Backend::Bridge(Arc::new(BridgeClient::new(addr, TOKEN).unwrap().with_timeout(Duration::from_millis(300)))))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bridged_job_that_outlives_the_reply_timeout_returns_its_result_with_progress() {
    // The job runs five times as long as any one reply may take.
    let (addr, app) = FakeApp::serve(Duration::from_millis(1500), false).await;
    let mut wire = Wire::start(bridged(&addr)).await;
    wire.call(5, "command_run", json!({"id": "filter.blur.gaussianBlur", "params": {"radius": 9}}), Some("blur")).await;
    let progress = wire.first_progress(5, "blur", 1.0).await;
    assert!((0.0..1.0).contains(&progress), "{progress}");
    assert_eq!(wire.reply(5, None).await, json!({"blurred": true}));
    // Before #8130 the edit was sent waiting, and the call failed after the timeout.
    assert_eq!(FakeApp::sent(&app, "engine.execute"), [json!({"command": "filter.blur.gaussianBlur", "params": {"radius": 9}, "wait": false})]);
    assert!(FakeApp::sent(&app, "jobs.list").len() > 1);
    assert!(FakeApp::sent(&app, "jobs.cancel").is_empty());

    // A command the engine doesn't know (a UI command) still lets the app wait.
    assert!(photocraft_engine::commands::find("view.zoomIn").is_none());
    wire.call(6, "command_run", json!({"id": "view.zoomIn"}), None).await;
    wire.reply(6, None).await;
    assert_eq!(FakeApp::sent(&app, "engine.execute").last(), Some(&json!({"command": "view.zoomIn", "params": {}, "wait": true})));

    // So does every waited step of a batch, which reports the steps around the job's progress.
    let steps = json!([{"id": "filter.blur.gaussianBlur", "params": {"radius": 9}}, {"id": "layer.new.layer"}]);
    app.lock().unwrap().started = None;
    wire.call(7, "command_batch", json!({"steps": steps}), Some("batch")).await;
    assert!(wire.first_progress(7, "batch", 2.0).await < 1.0, "still in the first step");
    let batch = wire.reply(7, None).await;
    assert_eq!(batch["completed"], 2, "{batch}");
    assert_eq!(batch["results"][0]["result"], json!({"blurred": true}), "{batch}");
    wire.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_bridged_wait_cancels_the_apps_job() {
    let (addr, app) = FakeApp::serve(Duration::from_secs(600), false).await;
    let mut wire = Wire::start(bridged(&addr)).await;
    wire.call(5, "command_run", BLUR(), Some("blur")).await;
    wire.first_progress(5, "blur", 1.0).await;
    wire.cancel(5).await;
    let t = Instant::now();
    while FakeApp::sent(&app, "jobs.cancel").is_empty() {
        assert!(t.elapsed() < Duration::from_secs(10), "the job was never cancelled");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(FakeApp::sent(&app, "jobs.cancel"), [json!({"job": 7})]);
    assert_eq!(FakeApp::sent(&app, "engine.execute").len(), 1, "the edit is sent once");
    wire.finish().await;
}

/// A failed cancellation keeps the drop guard armed, so its cleanup can reach the app.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_bridge_cancel_keeps_cleanup_armed() {
    let (addr, app) = FakeApp::serve(Duration::from_secs(600), false).await;
    app.lock().unwrap().lose_cancel = true;
    let mut wire = Wire::start(bridged(&addr)).await;
    wire.call(5, "command_run", BLUR(), Some("blur")).await;
    wire.first_progress(5, "blur", 1.0).await;
    wire.cancel(5).await;

    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.lock().unwrap().cancelled {
        assert!(Instant::now() < deadline, "the failed jobs.cancel disabled cleanup and left the job running");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(FakeApp::sent(&app, "jobs.cancel"), [json!({"job": 7}), json!({"job": 7})]);
    assert_eq!(FakeApp::sent(&app, "engine.execute").len(), 1, "the edit is never resent");
    wire.finish().await;
}

/// A cancel during a poll the app is slow to answer, or that then fails (review of #8134), reaches
/// the app at once over a fresh connection.
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_during_a_slow_poll_cancels_without_waiting_for_it() {
    let (addr, app) = FakeApp::serve(Duration::from_secs(600), false).await;
    app.lock().unwrap().hold_polls = Duration::from_secs(600);
    // The poll would hold the cancel for this long if it waited for it.
    let bridge = BridgeClient::new(&addr, TOKEN).unwrap().with_timeout(Duration::from_secs(30));
    let mut wire = Wire::start(PhotocraftMcp::with_backend(Backend::Bridge(Arc::new(bridge)))).await;
    wire.call(5, "command_run", BLUR(), Some("blur")).await;
    let t = Instant::now();
    while FakeApp::sent(&app, "jobs.list").is_empty() {
        assert!(t.elapsed() < Duration::from_secs(10), "the job was never polled");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    wire.cancel(5).await;
    let t = Instant::now();
    while FakeApp::sent(&app, "jobs.cancel").is_empty() {
        assert!(t.elapsed() < Duration::from_secs(10), "the cancel waited for the poll");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(FakeApp::sent(&app, "jobs.cancel"), [json!({"job": 7})]);
    assert_eq!(FakeApp::sent(&app, "jobs.list").len(), 1, "the slow poll was not retried");
    wire.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_start_reply_is_reported_and_never_resent() {
    let (addr, app) = FakeApp::serve(Duration::from_millis(100), true).await;
    let mut wire = Wire::start(bridged(&addr)).await;
    wire.call(5, "command_run", BLUR(), None).await;
    let (error, is_error) = wire.outcome(5, None).await;
    assert!(is_error && error.contains("may have completed"), "{error}");
    // No resend of the edit, and no polling for a job whose id never arrived.
    assert_eq!(FakeApp::sent(&app, "engine.execute").len(), 1);
    assert!(FakeApp::sent(&app, "jobs.list").is_empty());
    wire.finish().await;
}

#[test]
fn polling_a_job_directly_returns_its_result_once_it_ends() {
    use photocraft_automation::headless::JobPoll;
    use photocraft_engine::jobs::Started;

    let mut h = photocraft_automation::Headless::trusted_local();
    h.command_run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let Started::Job(job) = h.command_begin("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap() else {
        panic!("Gaussian Blur runs as a job");
    };
    // No `sync_jobs` between polls: `job_poll` must apply the finished job itself.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        match h.job_poll(job) {
            JobPoll::Running(..) => assert!(std::time::Instant::now() < deadline, "job_poll never saw the job end"),
            JobPoll::Ended(result) => {
                result.unwrap();
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn abandoning_a_job_that_already_ended_returns_its_result_not_cancelled() {
    use photocraft_engine::jobs::Started;

    let mut h = photocraft_automation::Headless::trusted_local();
    h.command_run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let Started::Job(job) = h.command_begin("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap() else {
        panic!("Gaussian Blur runs as a job");
    };
    // Another request applies the finished job before the waiter's cancel arrives.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while h.session.job(job).is_some() {
        assert!(std::time::Instant::now() < deadline, "the job never ended");
        std::thread::sleep(Duration::from_millis(1));
        h.sync_jobs();
    }
    // The blur is in the document, so the waiter must not hear "cancelled".
    h.job_abandon(job).unwrap();
}
