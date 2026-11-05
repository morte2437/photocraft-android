//! Headless calls that wait on a long command (#8129): progress, other calls served meanwhile,
//! and cancellation that leaves the document unchanged. Driven over the real line transport.
use std::time::Duration;

use photocraft_automation::PhotocraftMcp;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};

struct Wire {
    input: DuplexStream,
    lines: Lines<BufReader<DuplexStream>>,
    server: tokio::task::JoinHandle<Result<(), photocraft_automation::AutomationError>>,
}

impl Wire {
    async fn start() -> Wire {
        let (input, server_in) = tokio::io::duplex(1 << 20);
        let (server_out, output) = tokio::io::duplex(1 << 20);
        let server = tokio::spawn(PhotocraftMcp::headless().serve_io(server_in, server_out));
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
        loop {
            let message = self.next().await;
            if pending.is_some_and(|p| message["id"] == p) {
                panic!("request {} answered while it should still run: {message}", pending.unwrap_or(0));
            }
            if message["id"] == id {
                assert_eq!(message["result"]["isError"], false, "{message}");
                return serde_json::from_str(message["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
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
    let mut wire = Wire::start().await;
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
    let mut wire = Wire::start().await;
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
