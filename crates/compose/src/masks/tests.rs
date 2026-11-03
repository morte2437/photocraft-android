use super::*;
use photocraft_doc::LayerMask;
use photocraft_doc::vector::{Knot, Path, Subpath, VectorMask};

fn pending(c: &Mutex<Cache>, k: u64) -> Arc<OnceLock<Surface>> {
    match lookup(c, k) {
        CachedMask::Pending(slot, _) => slot,
        CachedMask::Hit(_) => panic!("expected a cold mask"),
    }
}

/// A built mask (and no pixel-mask input), as `finish_mask`'s build returns it.
fn test_surface() -> (Surface, Option<Surface>) {
    let mut surface = Surface::new(FORMAT);
    surface.fill_rect(Rect::new(0, 0, 2, 2), &[0.5]);
    (surface, None)
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cold_callers_share_one_build_without_retaining_a_second_cache() {
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let c = Mutex::new(Cache::default());
    // Acquire all callers' slots before allowing any builder to finish.
    let slots: Vec<_> = (0..8).map(|_| pending(&c, 7)).collect();
    assert!(slots.iter().all(|slot| Arc::ptr_eq(slot, &slots[0])));
    let barrier = Barrier::new(slots.len());
    let builds = AtomicUsize::new(0);
    let outputs = std::thread::scope(|scope| {
        let handles: Vec<_> = slots
            .iter()
            .map(|slot| {
                let (c, barrier, builds) = (&c, &barrier, &builds);
                scope.spawn(move || {
                    barrier.wait();
                    finish_mask(c, 7, slot, 0, || {
                        builds.fetch_add(1, Ordering::SeqCst);
                        test_surface()
                    })
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert!(outputs.iter().all(|surface| Arc::ptr_eq(surface.tiles().next().unwrap().1, outputs[0].tiles().next().unwrap().1)));

    // Evicting the retained result cannot make existing callers build again.
    c.lock().unwrap().map.clear();
    let again = finish_mask(&c, 7, &slots[0], 0, || panic!("rebuilt an active slot"));
    assert!(Arc::ptr_eq(again.tiles().next().unwrap().1, outputs[0].tiles().next().unwrap().1));
    drop(slots);
    let next = pending(&c, 8);
    assert_eq!(c.lock().unwrap().in_flight.len(), 1);
    drop(next);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn distinct_keys_build_while_another_key_is_paused() {
    use std::sync::mpsc;
    use std::time::Duration;

    let c = Mutex::new(Cache::default());
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (second_tx, second_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let c = &c;
        let first = scope.spawn(move || {
            cached_mask(c, 1, || {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                test_surface()
            })
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let second = scope.spawn(move || {
            let surface = cached_mask(c, 2, test_surface);
            second_tx.send(surface.sample_channel(0, 0, 0)).unwrap();
        });
        assert!((second_rx.recv_timeout(Duration::from_secs(5)).unwrap() - 0.5).abs() <= 1.0 / f32::from(u16::MAX));
        release_tx.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();
    });
}

#[test]
fn obsolete_in_flight_build_does_not_repopulate_the_cache() {
    let c = Mutex::new(Cache::default());
    let old = pending(&c, 1);
    c.lock().unwrap().in_flight.clear();
    let current = pending(&c, 1);
    assert!(!Arc::ptr_eq(&old, &current));
    let old_surface = finish_mask(&c, 1, &old, 0, test_surface);
    assert!(c.lock().unwrap().map.is_empty());
    let current_surface = finish_mask(&c, 1, &current, 0, test_surface);
    assert!(!Arc::ptr_eq(old_surface.tiles().next().unwrap().1, current_surface.tiles().next().unwrap().1));
    assert_eq!(c.lock().unwrap().map.len(), 1);
}

#[test]
fn a_purge_during_a_shared_build_keeps_the_cache_empty() {
    let c = Mutex::new(Cache::default());
    let CachedMask::Pending(slot, generation) = lookup(&c, 1) else { panic!("expected a cold mask") };
    c.lock().unwrap().clear();
    finish_mask(&c, 1, &slot, generation, test_surface);
    assert!(c.lock().unwrap().map.is_empty());
    assert_eq!(c.lock().unwrap().bytes, 0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn parallel_feathered_pixel_mask_returns_the_same_tiles() {
    let canvas = Rect::new(0, 0, 600, 400);
    let mut layer = Layer::raster("cold shared mask", PixelFormat::RGBA8);
    let mut mask = photocraft_doc::LayerMask::reveal_all();
    mask.surface.fill_rect(canvas, &[0.5]);
    mask.feather = 8.0;
    layer.mask = Some(mask);
    let barrier = std::sync::Barrier::new(8);
    let outputs = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (layer, barrier) = (&layer, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    combined_mask(layer, canvas).unwrap()
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
    });
    assert!(outputs.iter().all(|surface| Arc::ptr_eq(surface.tiles().next().unwrap().1, outputs[0].tiles().next().unwrap().1)));
}

/// A feather reaching the canvas edge sees the mask continue past it with its edge values, as
/// in Photoshop (#9475): a rectangle near the edge feathers the same on a taller canvas holding
/// the same mask (clamping each blur pass at the edge brightened it by up to 12 levels at
/// 32 px), and a mask white up to the edge stays white along it.
#[test]
fn feather_at_the_canvas_edge_continues_the_mask_past_it() {
    let mut layer = Layer::raster("edge", PixelFormat::RGBA8);
    let mut mask = photocraft_doc::LayerMask::hide_all();
    mask.surface.fill_rect(Rect::new(40, 20, 120, 60), &[1.0]);
    mask.feather = 32.0;
    layer.mask = Some(mask);
    let canvas = Rect::new(0, 0, 160, 80);
    let tight = combined_mask(&layer, canvas).unwrap().read_region(canvas);
    let tall = combined_mask(&layer, Rect::new(0, -200, 160, 280)).unwrap().read_region(canvas);
    let worst = tight.iter().zip(&tall).map(|(a, b)| (a - b).abs()).fold(0.0_f32, f32::max);
    assert!(worst < 1e-4, "the canvas edge changed the feather by {worst}");

    let mut half = photocraft_doc::LayerMask::hide_all();
    half.surface.fill_rect(Rect::new(0, 0, 80, 80), &[1.0]);
    half.feather = 32.0;
    layer.mask = Some(half);
    let m = combined_mask(&layer, canvas).unwrap().read_region(canvas);
    // Photoshop: every row the same, no darkening towards the top or bottom edge.
    let middle = &m[40 * 160..41 * 160];
    for (y, row) in m.as_chunks::<160>().0.iter().enumerate() {
        assert!(row.iter().zip(middle).all(|(a, b)| (a - b).abs() < 1e-5), "row {y} differs from row 40");
    }
    assert!(middle[0] > 0.99, "the left edge stays white: {}", middle[0]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn vector_mask_build_inside_a_small_rayon_pool_completes() {
    use rayon::prelude::*;
    use std::sync::mpsc;
    use std::time::Duration;

    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let canvas = Rect::new(0, 0, 512, 512);
        let mut layer = Layer::raster("rayon vector mask", PixelFormat::RGBA8);
        let mut mask = VectorMask::new(photocraft_vector::shapes::ellipse(8.0, 8.0, 496.0, 496.0));
        mask.feather = 8.0;
        layer.vector_mask = Some(mask);
        let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
        let outputs: Vec<_> = pool.install(|| (0..8).into_par_iter().map(|_| combined_mask(&layer, canvas).unwrap()).collect());
        assert!(outputs.iter().all(|surface| Arc::ptr_eq(surface.tiles().next().unwrap().1, outputs[0].tiles().next().unwrap().1)));
        tx.send(()).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(10)).expect("mask initializer deadlocked inside Rayon");
    handle.join().unwrap();
}

fn one_tile(sample: SampleType) -> Surface {
    let mut surface = Surface::with_default(PixelFormat { mode: ColorMode::Grayscale, sample, alpha: false }, &[1.0]);
    surface.fill_rect(Rect::new(0, 0, 1, 1), &[0.0]);
    surface
}

#[test]
fn charges_input_and_output_at_every_depth_including_off_canvas_tiles() {
    let result = one_tile(SampleType::F32);
    for sample in SampleType::ALL {
        let mut input = one_tile(sample);
        input.fill_rect(Rect::new(1024, 1024, 1025, 1025), &[0.0]);
        let tile_pixels = (photocraft_geom::TILE_SIZE * photocraft_geom::TILE_SIZE) as usize;
        let expected = tile_pixels * (SampleType::F32.bytes() + 2 * sample.bytes()) + SampleType::F32.bytes() + sample.bytes();
        let bytes = entry_bytes(&result, Some(&input));
        assert_eq!(bytes, expected, "{sample:?}");
        let mut cache = Cache::default();
        cache.insert(1, result.clone(), None, bytes, bytes * 2, cache.generation);
        cache.insert(2, result.clone(), None, bytes, bytes * 2, cache.generation);
        assert_eq!(cache.bytes, bytes * 2, "shared tiles are conservatively charged per entry");
    }
}

#[test]
fn byte_budget_evicts_the_least_recently_used_entry() {
    let surface = one_tile(SampleType::F32);
    let bytes = entry_bytes(&surface, None);
    let mut cache = Cache::default();
    cache.insert(1, surface.clone(), None, bytes, bytes * 2, cache.generation);
    cache.insert(2, surface.clone(), None, bytes, bytes * 2, cache.generation);
    assert!(cache.get(1).is_some());
    cache.insert(3, surface, None, bytes, bytes * 2, cache.generation);
    assert!(cache.get(2).is_none());
    assert!(cache.get(1).is_some());
    assert!(cache.get(3).is_some());
    assert_eq!(cache.bytes, bytes * 2);
}

#[test]
fn entry_limit_also_bounds_masks_with_no_tiles() {
    let surface = Surface::new(FORMAT);
    let bytes = entry_bytes(&surface, None);
    let mut cache = Cache::default();
    for key in 0..CAPACITY as u64 {
        cache.insert(key, surface.clone(), None, bytes, CACHE_BUDGET, cache.generation);
    }
    assert!(cache.get(0).is_some());
    cache.insert(CAPACITY as u64, surface, None, bytes, CACHE_BUDGET, cache.generation);
    assert_eq!(cache.map.len(), CAPACITY);
    assert!(cache.get(1).is_none());
    assert!(cache.get(0).is_some());
    assert_eq!(cache.bytes, bytes * CAPACITY);
}

#[test]
fn oversized_results_are_usable_without_retention_or_other_eviction() {
    let surface = one_tile(SampleType::F32);
    let bytes = entry_bytes(&surface, None);
    let mut cache = Cache::default();
    let small = Surface::new(FORMAT);
    let small_bytes = entry_bytes(&small, None);
    cache.insert(1, small, None, small_bytes, bytes - 1, cache.generation);
    let tile = surface.tiles().next().unwrap().1.clone();
    let owners = Arc::strong_count(&tile);
    cache.insert(2, surface.clone(), None, bytes, bytes - 1, cache.generation);
    assert!(cache.get(2).is_none());
    assert!(cache.get(1).is_some());
    assert_eq!(cache.bytes, small_bytes);
    assert_eq!(Arc::strong_count(&tile), owners);
    assert_eq!(surface.pixel(0, 0), vec![0.0]);
}

#[test]
fn replacement_and_purge_release_accounted_bytes_and_tile_references() {
    let first = one_tile(SampleType::U8);
    let second = one_tile(SampleType::F32);
    let first_bytes = entry_bytes(&first, None);
    let second_bytes = entry_bytes(&second, None);
    let first_tile = first.tiles().next().unwrap().1.clone();
    let second_tile = second.tiles().next().unwrap().1.clone();
    let mut cache = Cache::default();
    cache.insert(1, first.clone(), None, first_bytes, CACHE_BUDGET, cache.generation);
    assert_eq!(Arc::strong_count(&first_tile), 3);
    cache.insert(1, second.clone(), None, second_bytes, CACHE_BUDGET, cache.generation);
    assert_eq!(cache.bytes, second_bytes);
    assert_eq!(cache.map.len(), 1);
    assert_eq!(Arc::strong_count(&first_tile), 2);
    assert_eq!(Arc::strong_count(&second_tile), 3);
    assert_eq!(cache.clear(), second_bytes);
    assert_eq!(cache.bytes, 0);
    assert!(cache.map.is_empty());
    assert_eq!(Arc::strong_count(&second_tile), 2);
    assert_eq!(cache.clear(), 0);
}

#[test]
fn a_build_started_before_purge_cannot_refill_the_cache() {
    let surface = one_tile(SampleType::F32);
    let bytes = entry_bytes(&surface, None);
    let mut cache = Cache::default();
    assert!(cache.get(1).is_none());
    let generation_before_purge = cache.generation;
    cache.clear();
    cache.insert(1, surface.clone(), None, bytes, CACHE_BUDGET, generation_before_purge);
    assert!(cache.map.is_empty());
    assert_eq!(cache.bytes, 0);
    cache.insert(1, surface, None, bytes, CACHE_BUDGET, cache.generation);
    assert!(cache.get(1).is_some());
}

#[test]
fn pixel_edits_refresh_cached_masks_at_every_depth() {
    let canvas = Rect::new(0, 0, 64, 64);
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut layer = Layer::raster("edited mask", PixelFormat::RGBA8);
        let mut mask = LayerMask::hide_all();
        mask.surface = Surface::new(PixelFormat { sample, ..FORMAT });
        mask.surface.fill_rect(canvas, &[1.0]);
        mask.feather = 4.0;
        layer.mask = Some(mask);

        let first = combined_mask(&layer, canvas).unwrap();
        assert!((first.sample_channel(32, 32, 0) - 1.0).abs() < 1e-6);
        drop(first);
        // No document/undo snapshot shares the input. Previously this write could
        // reuse the original tile address and return the old cached white mask.
        layer.mask.as_mut().unwrap().surface.fill_rect(canvas, &[0.0]);
        let edited = combined_mask(&layer, canvas).unwrap();
        assert_eq!(edited.sample_channel(32, 32, 0), 0.0, "{sample:?}");

        layer.mask.as_mut().unwrap().surface.fill_rect(canvas, &[0.5]);
        let repainted = combined_mask(&layer, canvas).unwrap();
        assert!((repainted.sample_channel(32, 32, 0) - 0.5).abs() < 0.005, "{sample:?}");
        let warm = combined_mask(&layer, canvas).unwrap();
        assert!(repainted.tiles().zip(warm.tiles()).all(|((_, a), (_, b))| Arc::ptr_eq(a, b)));
    }
}

#[test]
fn cache_budget_holds_five_50mp_u8_input_u16_result_masks() {
    // Account for whole 256×256 tiles, as entry_bytes does, rather than just logical pixels.
    let (w, h) = (6336usize, 7920usize);
    let tile = photocraft_geom::TILE_SIZE as usize;
    let tile_count = w.div_ceil(tile) * h.div_ceil(tile);
    let tile_pixels = tile * tile;
    let one_entry = tile_count * tile_pixels * (SampleType::U8.bytes() + FORMAT.sample.bytes());
    assert!(one_entry.saturating_mul(5) <= CACHE_BUDGET, "five reported-size masks must fit");

    let mut cache = Cache::default();
    let surface = Surface::new(FORMAT);
    for key in 0..5 {
        cache.insert(key, surface.clone(), None, one_entry, CACHE_BUDGET, cache.generation);
    }
    for key in 0..5 {
        assert!(cache.get(key).is_some(), "mask {key} was evicted before the next plan");
    }
}

#[test]
fn matches_the_cpu_mask_and_is_cached() {
    let mut l = Layer::raster("l", PixelFormat::RGBA8);
    let pts = [(5.0, 5.0), (30.0, 6.0), (20.0, 28.0)];
    let knots = pts.iter().map(|&(x, y)| Knot::corner(x, y)).collect();
    let mut path = Path::default();
    path.subpaths.push(Subpath { knots, closed: true, ..Default::default() });
    let mut vm = VectorMask::new(path);
    vm.density = 0.9;
    l.vector_mask = Some(vm);
    let canvas = Rect::new(0, 0, 40, 32);
    let s = combined_mask(&l, canvas).unwrap();
    let want = photocraft_vector::vector_mask_values(l.vector_mask.as_ref().unwrap(), canvas);
    let mut got = Vec::new();
    s.read_region_into(canvas, &mut got);
    for (a, b) in got.iter().zip(&want) {
        assert!((a - b).abs() <= 1.0 / f32::from(u16::MAX));
    }
    let again = combined_mask(&l, canvas).unwrap();
    assert!(s.tiles().zip(again.tiles()).all(|(a, b)| Arc::ptr_eq(a.1, b.1)));
    l.vector_mask = None;
    assert!(combined_mask(&l, canvas).is_none());
}

#[test]
fn hostile_path_coordinates_do_not_overflow() {
    let mut l = Layer::raster("l", PixelFormat::RGBA8);
    let knots = [(-1.0e30, -1.0e30), (1.0e30, 0.0), (0.0, 1.0e30)].iter().map(|&(x, y)| Knot::corner(x, y)).collect();
    let mut path = Path::default();
    path.subpaths.push(Subpath { knots, closed: true, ..Default::default() });
    l.vector_mask = Some(VectorMask::new(path));
    let _ = combined_mask(&l, Rect::new(0, 0, 8, 8));
}
