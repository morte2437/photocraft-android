//! How long each piece of a stepped Place takes (#10031): the numbers behind the web build's
//! promise that a large file does not freeze the window.
//!
//! Not part of the default suite. Run it in release on one thread, as the web build has:
//!
//! ```sh
//! RAYON_NUM_THREADS=1 cargo test --release -p photocraft-engine --test place_steps_measure -- --ignored --nocapture
//! ```
//!
//! A synthetic 4096×4096 RGBA PNG is placed into a 3600×4200 document three ways: inline as one
//! call (the cost without steps), in steps of one unit each (the cost of every unit of work, so the
//! ones that cannot be cut up show), and in steps of the 8 ms a frame gives it (the longest frame
//! a user would see).

use std::time::{Duration, Instant};

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
use photocraft_engine::jobs::{JobOutcome, Started};
use photocraft_engine::{Session, file_cmds};
use serde_json::json;

const SIZE: u32 = 4096;

/// A 4096×4096 RGBA PNG of smooth colour and four bits of noise a channel, which makes a file of
/// a few tens of MB as a photo does (no personal photo).
fn synthetic_png() -> Vec<u8> {
    let mut data = Vec::with_capacity(SIZE as usize * SIZE as usize * 4);
    let mut r = 0x9E37_79B9u32;
    let mut noise = || {
        r = r.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (r >> 28) as u8
    };
    for y in 0..SIZE {
        for x in 0..SIZE {
            data.extend_from_slice(&[((x / 24) as u8) << 4 | noise(), ((y / 24) as u8) << 4 | noise(), (((x + y) / 48) as u8) << 4 | noise(), 0xF0 | noise()]);
        }
    }
    let img = Image::from_raw(SIZE, SIZE, ChannelLayout::Rgba, SampleType::U8, data).unwrap();
    photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap()
}

fn document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 3600, "height": 4200})).unwrap();
    s
}

/// A place's result without its layer id, which counts up across sessions.
fn strip_layer(v: &serde_json::Value) -> serde_json::Value {
    let mut v = v.clone();
    v.as_object_mut().map(|o| o.remove("layer"));
    v
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Polls a started place to the end, timing every poll; returns (stage label, time) per poll. The
/// label is the status the poll left (the message is set as a unit starts, so a poll that ends in
/// a new one began a stage's last unit).
fn timed_polls(s: &mut Session, id: photocraft_engine::jobs::JobId) -> (Vec<(String, String, Duration)>, serde_json::Value) {
    let mut polls = Vec::new();
    loop {
        let before = s.job(id).map(|j| j.message).unwrap_or_default();
        let t = Instant::now();
        let events = s.poll_jobs();
        let took = t.elapsed();
        let after = s.job(id).map_or_else(|| "finish and apply".to_string(), |j| j.message);
        polls.push((before, after, took));
        if let Some(e) = events.into_iter().find(|e| e.id == id) {
            let JobOutcome::Done(result) = e.outcome else { panic!("{:?}", e.outcome) };
            return (polls, result);
        }
        assert!(polls.len() < 1_000_000);
    }
}

fn stage_of(before: &str, after: &str) -> &'static str {
    match (before, after) {
        ("", "") => "open the PNG (header, buffer)",
        ("" | "Decoding", "Decoding") => "decode rows",
        ("Decoding", "Flattening") => "image to document (one unit)",
        ("Flattening", "Flattening") => "flatten a band",
        ("Flattening", "Resizing") => "resize: plan the first pass",
        (_, "Resizing") => "resize: bands and passes",
        (_, "finish and apply") => "translate, layer and apply (one unit)",
        _ => "other",
    }
}

#[test]
#[ignore = "a measurement, run by hand (see the module docs)"]
fn place_steps_timings() {
    let t = Instant::now();
    let png = synthetic_png();
    println!("synthetic {SIZE}x{SIZE} RGBA PNG: {:.1} MB, made in {:.0} ms", png.len() as f64 / 1e6, ms(t.elapsed()));
    println!("rayon threads: {}", rayon::current_num_threads());

    // Inline, as one call: no steps. The first run warms the allocator; the second is the time.
    let mut want = serde_json::Value::Null;
    let mut inline_ms = 0.0;
    for run in 1..=2 {
        let mut inline = document();
        let t = Instant::now();
        want = file_cmds::place_bytes(&mut inline, "big.png", png.clone(), None, &json!({})).unwrap();
        inline_ms = ms(t.elapsed());
        println!("\ninline place {run} (one call, as a desktop worker does it): {inline_ms:.0} ms");
    }

    // Cooperative, one unit per poll: the time of every unit of work.
    let mut s = document();
    s.set_cooperative_jobs(true);
    s.set_job_step_budget(Duration::ZERO);
    let t = Instant::now();
    let Started::Job(id) = s.start_place_bytes("big.png", png.clone(), None, json!({})).unwrap() else { panic!("expected a job") };
    let (polls, got) = timed_polls(&mut s, id);
    let total = ms(t.elapsed());
    assert_eq!(strip_layer(&got), strip_layer(&want), "the stepped place is the inline place");
    println!("\nstepped, one unit per poll: {} units, {total:.0} ms in all", polls.len());
    println!("{:<42} {:>7} {:>10} {:>10}", "stage", "units", "total ms", "longest ms");
    let mut stages: Vec<&str> = Vec::new();
    for (b, a, _) in &polls {
        let st = stage_of(b, a);
        if !stages.contains(&st) {
            stages.push(st);
        }
    }
    for st in stages {
        let times: Vec<f64> = polls.iter().filter(|(b, a, _)| stage_of(b, a) == st).map(|(_, _, d)| ms(*d)).collect();
        println!("{st:<42} {:>7} {:>10.1} {:>10.1}", times.len(), times.iter().sum::<f64>(), times.iter().copied().fold(0.0, f64::max));
    }
    let longest = polls.iter().map(|(_, _, d)| ms(*d)).fold(0.0, f64::max);
    println!("longest single unit: {longest:.1} ms");

    // Cooperative with the frame budget: the frames a user would see.
    let mut s = document();
    s.set_cooperative_jobs(true);
    let t = Instant::now();
    let Started::Job(id) = s.start_place_bytes("big.png", png, None, json!({})).unwrap() else { panic!("expected a job") };
    let (polls, _) = timed_polls(&mut s, id);
    let total = ms(t.elapsed());
    let mut times: Vec<f64> = polls.iter().map(|(_, _, d)| ms(*d)).collect();
    times.sort_by(f64::total_cmp);
    println!(
        "\nstepped, 8 ms a poll: {} polls, {total:.0} ms in all ({:+.0}% on inline); median poll {:.1} ms, 95th percentile {:.1} ms, longest {:.1} ms",
        times.len(),
        (total / inline_ms - 1.0) * 100.0,
        times[times.len() / 2],
        times[times.len() * 95 / 100],
        times[times.len() - 1]
    );
    let slow: Vec<_> = polls.iter().filter(|(_, _, d)| ms(*d) > 16.0).map(|(b, a, d)| format!("{} ({:.0} ms)", stage_of(b, a), ms(*d))).collect();
    println!("polls over 16 ms: {}", if slow.is_empty() { "none".to_string() } else { slow.join(", ") });
}
