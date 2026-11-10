//! A scratch folder that stops answering (a dead hard-mounted network share) must not freeze
//! anything: status reads stay prompt, opening and measuring wait a bounded time, retries do not
//! pile up threads, leaving the folder works, and a late answer applies only to the folder it
//! was about.
//!
//! One test function: the spill manager is process-wide, so its steps must not interleave.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Surface, spill};

const TILE_BYTES: usize = (TILE_SIZE * TILE_SIZE) as usize * 4;

/// Folders whose name starts with "hung" stall their queries while this is on.
static STALL: AtomicBool = AtomicBool::new(false);
/// Queries that reached the stall since the last reset.
static ENTERED: AtomicUsize = AtomicUsize::new(0);
/// `true` once the stalled queries may go on.
static GATE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

fn stall(dir: &Path) {
    let hung = dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("hung"));
    if !STALL.load(Ordering::SeqCst) || !hung {
        return;
    }
    ENTERED.fetch_add(1, Ordering::SeqCst);
    let mut open = GATE.0.lock().unwrap();
    while !*open {
        open = GATE.1.wait(open).unwrap();
    }
}

fn arm() {
    ENTERED.store(0, Ordering::SeqCst);
    *GATE.0.lock().unwrap() = false;
    STALL.store(true, Ordering::SeqCst);
}

fn release() {
    STALL.store(false, Ordering::SeqCst);
    *GATE.0.lock().unwrap() = true;
    GATE.1.notify_all();
}

fn poll_until(what: &str, cond: impl Fn() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}: {:?}", spill::stats());
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Run `f` and require that it returns well before a query's wait.
fn prompt<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    assert!(start.elapsed() < Duration::from_millis(1500), "{what} took {:?}", start.elapsed());
    out
}

/// A query's 2 s wait plus a scheduling allowance.
const QUERY_DEADLINE: Duration = Duration::from_secs(6);

/// Run `f` and require that it returns within [`QUERY_DEADLINE`].
fn bounded<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    assert!(start.elapsed() < QUERY_DEADLINE, "{what} took {:?}", start.elapsed());
    out
}

fn rgba8() -> PixelFormat {
    PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U8, alpha: true }
}

/// Incompressible bytes, different per seed.
fn noise(seed: u32, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

fn surface(seed: u32, n: i32) -> Surface {
    let mut s = Surface::new(rgba8());
    for i in 0..n {
        let r = Rect::from_xywh(i * TILE_SIZE, 0, TILE_SIZE as u32, TILE_SIZE as u32);
        s.write_interleaved(r, &noise(seed + i as u32, TILE_BYTES));
    }
    s
}

#[test]
fn a_folder_that_stops_answering_never_freezes_the_app() {
    let root = std::env::temp_dir().join(format!("photocraft-scratch-slow-{}", std::process::id()));
    let good = root.join("good");
    let good2 = root.join("good2");
    let hung1 = root.join("hung1");
    let hung2 = root.join("hung2");
    let hung3 = root.join("hung3");
    spill::set_query_hook_for_tests(Some(stall));

    // A. A healthy folder.
    spill::configure(Some(&good), 1 << 20).unwrap();
    assert!(spill::enabled() && !spill::paused());
    let measurable = spill::scratch_capacity().is_some();

    // B. Configuring a folder that never answers: status reads stay prompt meanwhile (they
    //    blocked on the config lock before), and configure gives up after its wait.
    arm();
    let began = Instant::now();
    let configuring = {
        let hung1 = hung1.clone();
        std::thread::spawn(move || spill::configure(Some(&hung1), 1 << 20))
    };
    poll_until("the first query to start", || ENTERED.load(Ordering::SeqCst) == 1);
    prompt("last_error", spill::last_error);
    prompt("check_integrity", spill::check_integrity).ok();
    prompt("stats", spill::stats);
    prompt("free_space_unmeasured", spill::free_space_unmeasured);
    prompt("paused", spill::paused);
    let result = configuring.join().unwrap();
    assert!(began.elapsed() < QUERY_DEADLINE, "configure of a hung folder took {:?}", began.elapsed());
    assert!(result.as_ref().err().is_some_and(|e| e.contains("not responding")), "{result:?}");
    assert!(spill::paused() && !spill::enabled());
    assert_eq!(spill::stats().dir.as_deref(), Some(hung1.as_path()));

    // C. Retries neither wait nor start more threads: the question is already being asked.
    for _ in 0..5 {
        spill::retry_now_for_tests();
        prompt("maintain_budget", spill::maintain_budget);
    }
    assert_eq!(ENTERED.load(Ordering::SeqCst), 1, "a retry started a second query");
    assert!(spill::paused());

    // D. Leaving the dead folder works while its query is stuck.
    prompt("configure away from a hung folder", || spill::configure(Some(&good2), 1 << 20)).unwrap();
    assert!(spill::enabled() && !spill::paused());

    // E. The stuck query's late answer is about a folder that is no longer current: discarded.
    release();
    poll_until("the stuck query to settle", || spill::query_threads_for_tests() == 0);
    let st = spill::stats();
    assert_eq!(st.dir.as_deref(), Some(good2.as_path()), "{st:?}");
    assert!(spill::enabled() && !spill::paused() && st.last_error.is_none(), "{st:?}");

    // F. A late answer for the current folder applies: spilling resumes by itself.
    arm();
    let result = bounded("configure of a second hung folder", || spill::configure(Some(&hung2), 1 << 20));
    assert!(result.is_err());
    assert!(spill::paused());
    release();
    poll_until("the late answer to resume spilling", || spill::enabled() && !spill::paused() && spill::last_error().is_none());
    assert_eq!(spill::stats().dir.as_deref(), Some(hung2.as_path()));

    // G. The open folder stops answering when measured: no answer counts as unknown, spilling
    //    goes on, and asking again costs nothing.
    spill::configure(Some(&hung3), 1 << 20).unwrap();
    arm();
    assert_eq!(bounded("scratch_capacity", spill::scratch_capacity), None);
    assert!(spill::free_space_unmeasured());
    assert!(spill::stats().free_space_unmeasured);
    assert!(spill::enabled() && !spill::paused());
    for _ in 0..5 {
        assert_eq!(prompt("scratch_capacity", spill::scratch_capacity), None);
    }
    assert_eq!(ENTERED.load(Ordering::SeqCst), 1, "asking again started a second query");
    release();
    if measurable {
        poll_until("the late answer to measure the volume", || !spill::free_space_unmeasured());
    }

    poll_until("the previous query to finish", || spill::query_threads_for_tests() == 0);

    // H. The growth check never stalls eviction: tiles still spill while it is unanswered.
    spill::set_budget(8 * TILE_BYTES);
    arm();
    spill::set_size_cap(Some(1 << 50));
    let trigger_tiles = surface(1, 32);
    poll_until("the growth query to reach the stall", || ENTERED.load(Ordering::SeqCst) >= 1);
    let evictions = spill::stats().evictions;
    let tiles = surface(33, 32);
    prompt("settle", spill::settle);
    assert!(spill::stats().evictions > evictions, "{:?}", spill::stats());
    poll_until("the unanswered growth check to count as unknown", spill::free_space_unmeasured);
    release();
    if measurable {
        poll_until("the growth check to be answered", || !spill::free_space_unmeasured());
    }
    spill::set_size_cap(None);

    // Cleanup.
    spill::set_query_hook_for_tests(None);
    drop(tiles);
    drop(trigger_tiles);
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(&root);
}
