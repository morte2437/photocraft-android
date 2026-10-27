/// Manually compare allocating reads with the previous implementation.
#[test]
#[ignore = "manual 24 MP release benchmark"]
fn benchmark_sparse_reads() {
    use std::{hint::black_box, time::Instant};
    let rect = Rect::new(0, 0, 6000, 4000);
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for default in [0.0, 0.25] {
            let mut s = Surface::with_default(PixelFormat::GRAY8.with_sample(depth), &[default]);
            s.fill_rect(Rect::new(257, 257, 266, 266), &[0.75]);
            assert_eq!(s.read_region(rect), previous_read_region(&s, rect));
            for trial in 0..6 {
                let mut paths = [("main", previous_read_region as fn(&Surface, Rect) -> Vec<f32>), ("split", Surface::read_region)];
                if trial % 2 == 1 {
                    paths.reverse();
                }
                for (name, path) in paths {
                    let t = Instant::now();
                    let out = black_box(path)(&s, rect);
                    let elapsed = t.elapsed().as_secs_f64() * 1000.0;
                    black_box(&out);
                    println!("sparse,{depth:?}-{default},{trial},{name},{elapsed:.6}");
                }
            }
        }
    }
}
/// Previous allocating read path, retained as the bitwise-equivalence oracle.
fn previous_read_region(surface: &Surface, r: Rect) -> Vec<f32> {
    let n = surface.channels();
    let w = r.width() as usize;
    let mut out = vec![0.0f32; w * r.height() as usize * n];
    let dp = surface.default_pixel();
    for tc in r.tiles() {
        let tr = tc.rect().intersect(&r);
        let tile = surface.tiles.get(&tc);
        for y in tr.y0..tr.y1 {
            for x in tr.x0..tr.x1 {
                let o = (((y - r.y0) as usize) * w + (x - r.x0) as usize) * n;
                match tile {
                    Some(t) => {
                        let base = (((y - tc.ty * TILE_SIZE) as usize) * TILE_SIZE as usize + (x - tc.tx * TILE_SIZE) as usize) * n;
                        for i in 0..n {
                            out[o + i] = read_sample(&t.data, surface.format.sample, base + i);
                        }
                    }
                    None => out[o..o + n].copy_from_slice(&dp),
                }
            }
        }
    }
    out
}
/// Distinct tile allocations in `s`.
fn distinct_tiles(s: &Surface) -> usize {
    let mut seen: Vec<*const Tile> = s.tiles().map(|(_, t)| Arc::as_ptr(t)).collect();
    seen.sort();
    seen.dedup();
    seen.len()
}

/// Converting or tone-mapping a surface keeps a shared tile shared (#4531) and gives the
/// same pixels as converting each pixel: a filled background is one tile across the grid,
/// plus some distinct tiles.
#[test]
fn conversion_keeps_shared_tiles_shared() {
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.fill_rect(Rect::new(0, 0, 2048, 1024), &[0.9, 0.5, 0.2, 1.0]);
    s.fill_rect(Rect::new(10, 10, 20, 20), &[0.1, 0.2, 0.3, 0.5]);
    let shared_before = distinct_tiles(&s);
    assert!(shared_before <= 2 && s.tile_count() == 32, "{shared_before} distinct of {}", s.tile_count());
    let r = Rect::new(0, 0, 2048, 1024);
    for to in [PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        for out in [s.convert(to), s.try_convert(to).expect("memory")] {
            assert_eq!(distinct_tiles(&out), shared_before, "{to:?}: still shared");
            assert_eq!(out.tile_count(), 32);
            let (a, b) = (out.read_region(r), s.read_region(r));
            for (pa, pb) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
                let want = convert_pixel(&s.format(), &to, pb);
                assert!(pa.iter().zip(&want).all(|(x, y)| (x - y).abs() < 1e-6), "{to:?}: {pa:?} vs {want:?}");
            }
        }
    }
    let mut m = s.clone();
    m.map_color(|v| v * 0.5);
    assert_eq!(distinct_tiles(&m), shared_before, "map_color keeps it shared");
    assert!((m.pixel(1000, 500)[0] - s.pixel(1000, 500)[0] * 0.5).abs() < 1.0 / 255.0);
    assert_eq!(s.pixel(1000, 500), vec![s.pixel(1000, 500)[0], s.pixel(1000, 500)[1], s.pixel(1000, 500)[2], 1.0], "the source is untouched");
}

use super::*;
use photocraft_color::PixelFormat;
use proptest::prelude::*;

fn write_region_reference(surface: &mut Surface, r: Rect, data: &[f32]) {
    let n = surface.channels();
    let w = r.width() as usize;
    assert_eq!(data.len(), w * r.height() as usize * n, "region data length mismatch");
    let sample = surface.format.sample;
    for tc in r.tiles() {
        let tr = tc.rect().intersect(&r);
        let t = surface.tile_mut(tc);
        for y in tr.y0..tr.y1 {
            for x in tr.x0..tr.x1 {
                let o = (((y - r.y0) as usize) * w + (x - r.x0) as usize) * n;
                let base = (((y - tc.ty * TILE_SIZE) as usize) * TILE_SIZE as usize + (x - tc.tx * TILE_SIZE) as usize) * n;
                for i in 0..n {
                    write_sample(&mut t.data, sample, base + i, data[o + i]);
                }
            }
        }
    }
}

#[test]
fn region_writes_match_scalar_bytes_for_every_format() {
    let special = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        2.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        f32::from_bits(0x7fc12345),
        f32::from_bits(0xffc12345),
        f32::MIN_POSITIVE,
        f32::from_bits(1),
        0.5 / 255.0,
        0.5 / 65535.0,
    ];
    for mode in [
        ColorMode::Bitmap,
        ColorMode::Grayscale,
        ColorMode::Indexed,
        ColorMode::Rgb,
        ColorMode::Cmyk,
        ColorMode::Lab,
        ColorMode::Multichannel,
        ColorMode::Duotone,
    ] {
        for sample in SampleType::ALL {
            for alpha in [false, true] {
                let fmt = PixelFormat::new(mode, sample, alpha);
                let mut actual = Surface::with_default(fmt, &vec![0.25; fmt.channels()]);
                actual.write_pixel(900, 900, &vec![0.75; fmt.channels()]);
                let mut expected = actual.clone();
                for rect in [Rect::EMPTY, Rect::new(-270, -2, 270, 3), Rect::new(256, 256, 512, 257), Rect::new(-64, -64, 0, 0), Rect::new(255, 250, 258, 270)]
                {
                    let data: Vec<_> = (0..rect.width() as usize * rect.height() as usize * fmt.channels())
                        .map(|i| {
                            if i % 3 == 0 {
                                special[i / 3 % special.len()]
                            } else {
                                // Integer rounding boundaries and deterministic arbitrary f32 bit patterns.
                                let bits = (i as u32).wrapping_mul(2654435761);
                                if i % 3 == 1 { ((bits % 65536) as f32 + 0.5) / 65535.0 } else { f32::from_bits(bits) }
                            }
                        })
                        .collect();
                    let snapshot = actual.clone();
                    let snapshot_bytes = snapshot.to_interleaved(rect.inflate(1));
                    let mut par = actual.clone();
                    actual.write_region(rect, &data);
                    par.write_region_par(rect, &data);
                    write_region_reference(&mut expected, rect, &data);
                    assert_eq!(actual, expected, "{fmt:?}, {rect:?}");
                    assert_eq!(par, expected, "parallel {fmt:?}, {rect:?}");
                    assert_eq!(snapshot.to_interleaved(rect.inflate(1)), snapshot_bytes, "COW snapshot changed");
                    // Repeat against already materialised, unshared tiles too.
                    actual.write_region(rect, &data);
                    par.write_region_par(rect, &data);
                    assert_eq!(actual, expected);
                    assert_eq!(par, expected);
                }
            }
        }
    }
}

/// Mask builds call `write_region` inside a `OnceLock` initializer, where entering Rayon can
/// run another tile's job that waits on the same lock. With every global worker held, a write
/// that queued work on Rayon could not finish.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn write_region_does_not_enter_rayon() {
    use std::sync::{Barrier, mpsc};
    let workers = rayon::current_num_threads();
    let held = Arc::new(Barrier::new(workers + 1));
    let release = Arc::new(Barrier::new(workers + 1));
    let (h, rel) = (held.clone(), release.clone());
    rayon::spawn_broadcast(move |_| {
        h.wait();
        rel.wait();
    });
    held.wait();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = Rect::new(0, 0, 2 * TILE_SIZE, 2 * TILE_SIZE);
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.write_region(r, &vec![0.5; (r.width() * r.height() * 4) as usize]);
        tx.send(()).ok();
    });
    let done = rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok();
    release.wait();
    assert!(done, "write_region waited on the Rayon pool");
}

#[test]
fn from_rgba_into_matches_from_rgba() {
    for fmt in [PixelFormat::RGBA8, PixelFormat::GRAY8, PixelFormat::GRAYA8, PixelFormat::CMYKA8, PixelFormat::new(ColorMode::Lab, SampleType::F32, true)] {
        let mut buf = [0.0f32; 8];
        let n = from_rgba_into(&fmt, [0.2, 0.5, 0.7, 0.4], &mut buf);
        assert_eq!(&buf[..n], &from_rgba(&fmt, [0.2, 0.5, 0.7, 0.4])[..]);
    }
}

/// Sample counts above the stack capacity use writable storage without truncating channels.
#[test]
fn sample_buffer_keeps_stack_fast_path_and_supports_dynamic_channels() {
    for channels in [1, 8, 9, 32] {
        let mut stack = [0.0; 8];
        let mut heap = Vec::new();
        let samples = sample_buffer(channels, &mut stack, &mut heap);
        assert_eq!(samples.len(), channels);
        samples.fill(0.25);
        if channels <= stack.len() {
            assert!(heap.is_empty());
            assert_eq!(&stack[..channels], vec![0.25; channels]);
        } else {
            assert_eq!(heap, vec![0.25; channels]);
        }
    }
}

/// Compare sparse reads and converted accessors with the previous allocating path.
#[test]
fn zero_alloc_accessors_match_read_region() {
    for (fmt, default) in [
        PixelFormat::RGBA8,
        PixelFormat::RGBA16,
        PixelFormat::RGBA32F,
        PixelFormat::GRAYA8,
        PixelFormat::CMYKA8,
        PixelFormat::GRAY8,
        PixelFormat::GRAY8.with_sample(SampleType::F32),
        PixelFormat::new(ColorMode::Lab, SampleType::U16, true),
    ]
    .into_iter()
    .flat_map(|fmt| [0.0, 0.25, -0.0].map(|default| (fmt, default)))
    {
        let mut s = Surface::with_default(fmt, &vec![default; fmt.channels()]);
        let mut stack = [0.0; 8];
        let mut heap = Vec::new();
        for capacity in [0, stack.len()] {
            let decoded = s.default_samples(&mut stack[..capacity], &mut heap);
            assert!(decoded.iter().map(|v| v.to_bits()).eq(s.default_pixel().into_iter().map(f32::to_bits)));
        }
        let r = Rect::new(-300, -20, 300, 40);
        for (i, (x, y)) in [(-299, -19), (0, 0), (255, 39), (256, 10), (299, 0)].into_iter().enumerate() {
            let px: Vec<f32> = (0..fmt.channels()).map(|c| ((i * 3 + c) % 5) as f32 / 4.0).collect();
            s.write_pixel(x, y, &px);
        }
        let region = s.read_region(r);
        assert!(region.iter().map(|v| v.to_bits()).eq(previous_read_region(&s, r).into_iter().map(f32::to_bits)));
        let mut into = vec![9.0; 3];
        s.read_region_into(r, &mut into);
        assert_eq!(region, into);
        let mut rgba = vec![[0.0; 4]; (r.width() * r.height()) as usize];
        s.read_rgba_into(r, &mut rgba);
        let mut rgba8 = vec![[0; 4]; rgba.len()];
        s.read_rgba8_into(r, &mut rgba8);
        let n = fmt.channels();
        for (i, p) in region.chunks_exact(n).enumerate() {
            let want = to_rgba(&fmt, p);
            assert_eq!(rgba8[i], want.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
            for c in 0..4 {
                assert!((rgba[i][c] - want[c]).abs() < 1e-6, "{fmt:?} px {i}");
            }
            let (x, y) = (r.x0 + (i % r.width() as usize) as i32, r.y0 + (i / r.width() as usize) as i32);
            for c in 0..n {
                let bits = s.sample_channel(x, y, c).to_bits();
                assert_eq!(bits, p[c].to_bits());
                assert_eq!(bits, into[i * n + c].to_bits());
            }
        }
        assert!(s.has_tiles_in(Rect::new(0, 0, 1, 1)));
        assert!(!s.has_tiles_in(Rect::new(1000, 1000, 1001, 1001)));
    }
}

#[test]
fn fallible_region_reads_report_size_overflow() {
    let surface = Surface::new(PixelFormat::RGBA8);
    let huge = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    assert_eq!(surface.try_read_region(huge), Err(AllocationError::SizeOverflow));
    assert_eq!(surface.try_to_interleaved(huge), Err(AllocationError::SizeOverflow));
}

#[test]
fn fallible_region_write_rejects_bad_input_without_mutating_surface() {
    let mut surface = Surface::new(PixelFormat::RGBA8);
    let before = surface.clone();
    assert_eq!(surface.try_write_region(Rect::new(0, 0, 2, 2), &[1.0]), Err(AllocationError::SizeOverflow));
    assert_eq!(surface, before);
}

#[test]
fn fallible_tile_edits_move_weakly_cached_pixels_and_copy_shared_pixels() {
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let mut surface = Surface::new(fmt);
        let coord = TileCoord::new(0, 0);
        let pixels = surface.try_tile_mut(coord).unwrap().bytes().as_ptr();
        let weak = Arc::downgrade(surface.tile(coord).unwrap());
        let tile = surface.try_tile_mut(coord).unwrap();
        assert_eq!(tile.bytes().as_ptr(), pixels);
        assert!(weak.upgrade().is_none());
        let snapshot = surface.clone();
        surface.try_write_region(Rect::new(0, 0, 1, 1), &[1.0; 4]).unwrap();
        assert_eq!(snapshot.pixel(0, 0), vec![0.0; 4]);
        assert_eq!(surface.pixel(0, 0), vec![1.0; 4]);
    }
}

#[test]
fn fill_rect_shares_covered_tiles_and_edits_stay_local() {
    // A 300000² background used to materialize ~360 GB of tiles (#705).
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.fill_rect(Rect::new(0, 0, 300_000, 300_000), &[1.0, 0.5, 0.0, 1.0]);
    let t = |s: &Surface, x, y| s.tile(TileCoord::new(x, y)).unwrap().clone();
    assert!(Arc::ptr_eq(&t(&s, 0, 0), &t(&s, 1000, 900)));
    // 300000 is not a multiple of 256: the edge tile is partial and stays its own.
    let edge = 300_000 / TILE_SIZE;
    assert!(!Arc::ptr_eq(&t(&s, 0, 0), &t(&s, edge, 0)));
    assert_eq!(s.pixel(299_999, 299_999), vec![1.0, 128.0 / 255.0, 0.0, 1.0]);
    assert_eq!(s.pixel(300_000, 0), vec![0.0; 4]);
    s.write_pixel(5, 5, &[0.0, 0.0, 1.0, 1.0]);
    assert_eq!(s.pixel(5, 5), vec![0.0, 0.0, 1.0, 1.0]);
    assert_eq!(s.pixel(300, 5), vec![1.0, 128.0 / 255.0, 0.0, 1.0]);
    assert!(!Arc::ptr_eq(&t(&s, 0, 0), &t(&s, 1, 0)));
}

#[test]
fn empty_surface_reads_default() {
    let s = Surface::new(PixelFormat::RGBA8);
    assert_eq!(s.pixel(10, -500), vec![0.0; 4]);
    let m = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
    assert_eq!(m.pixel(123_456, -9), vec![1.0]);
    assert_eq!(m.tile_count(), 0);
}

#[test]
fn write_read_pixel_every_depth() {
    for sample in SampleType::ALL {
        let f = PixelFormat::RGBA8.with_sample(sample);
        let mut s = Surface::new(f);
        s.write_pixel(-3, 700, &[0.2, 0.4, 0.6, 1.0]);
        let p = s.pixel(-3, 700);
        for (a, b) in p.iter().zip([0.2, 0.4, 0.6, 1.0]) {
            assert!((a - b).abs() <= 1.0 / 255.0, "{sample:?}");
        }
        assert_eq!(s.tile_count(), 1);
        assert_eq!(s.tile_bounds(), TileCoord::containing(-3, 700).rect());
    }
}

#[test]
fn copy_on_write_shares_untouched_tiles() {
    let mut a = Surface::new(PixelFormat::RGBA8);
    a.fill_rect(Rect::new(0, 0, 512, 512), &[1.0, 0.0, 0.0, 1.0]);
    let mut b = a.clone();
    b.write_pixel(10, 10, &[0.0, 1.0, 0.0, 1.0]);
    // a unchanged
    assert_eq!(a.pixel(10, 10), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(b.pixel(10, 10), vec![0.0, 1.0, 0.0, 1.0]);
    // only tile (0,0) was copied
    let shared = a.tiles().filter(|(c, t)| Arc::ptr_eq(t, b.tile(**c).unwrap())).count();
    assert_eq!(shared, 3);
}

#[test]
fn content_bounds_and_prune() {
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.fill_rect(Rect::new(300, 10, 310, 20), &[1.0, 1.0, 1.0, 1.0]);
    s.write_pixel(-5, -5, &[0.0, 0.0, 0.0, 1.0]);
    assert_eq!(s.content_bounds(), Rect::new(-5, -5, 310, 20));
    // erase the single pixel -> its tile becomes default and is pruned
    s.write_pixel(-5, -5, &[0.0, 0.0, 0.0, 0.0]);
    s.prune();
    assert_eq!(s.tile_count(), 1);
    assert_eq!(s.content_bounds(), Rect::new(300, 10, 310, 20));
}

#[test]
fn interleaved_roundtrip_crossing_tiles() {
    let r = Rect::new(-20, 250, 280, 262);
    let bpp = PixelFormat::RGBA16.bytes_per_pixel();
    let bytes: Vec<u8> = (0..r.width() as usize * r.height() as usize * bpp).map(|i| (i * 31 % 251) as u8).collect();
    let s = Surface::from_interleaved(PixelFormat::RGBA16, r, &bytes);
    assert_eq!(s.to_interleaved(r), bytes);
    assert_eq!(s.tile_count(), 6); // x tiles -1..=1, y tiles 0..=1
}

#[test]
fn region_outside_reads_default() {
    let s = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
    let v = s.read_region(Rect::new(0, 0, 3, 3));
    assert_eq!(v, vec![1.0; 9]);
}

#[test]
fn convert_depths_and_models() {
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.write_pixel(1, 1, &[1.0, 1.0, 1.0, 1.0]);
    let g = s.convert(PixelFormat::GRAYA8);
    assert!((g.pixel(1, 1)[0] - 1.0).abs() < 1e-2);
    let f = s.convert(PixelFormat::RGBA32F);
    assert_eq!(f.pixel(1, 1), vec![1.0, 1.0, 1.0, 1.0]);
    let c = s.convert(PixelFormat::CMYKA8);
    let p = c.pixel(1, 1);
    assert!(p[3] < 0.01 && (p[4] - 1.0).abs() < 1e-6);
    let back = c.convert(PixelFormat::RGBA8);
    assert_eq!(back.pixel(1, 1), vec![1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn rgba_from_gray_and_cmyk() {
    let mut g = Surface::new(PixelFormat::GRAYA8);
    g.write_pixel(0, 0, &[0.5, 1.0]);
    let v = g.rgba(0, 0);
    assert!((v[0] - v[2]).abs() < 1e-6 && (v[3] - 1.0).abs() < 1e-6);
}

#[test]
fn repeated_cmyk_reads_match_individual_pixels_bitwise() {
    let rect = Rect::new(-2, -1, 515, 2);
    for depth in SampleType::ALL {
        for alpha in [false, true] {
            let fmt = PixelFormat::new(ColorMode::Cmyk, depth, alpha);
            let mut s = Surface::with_default(fmt, &[0.2, 0.4, 0.6, 0.3, 0.7]);
            for (i, x) in (-1..514).enumerate() {
                let c = if i / 2 % 256 == 0 { -0.0 } else { (i / 2 % 256) as f32 / 255.0 };
                s.write_pixel(x, 0, &[c, 0.4, 0.6, 0.3, (i % 9) as f32 / 8.0]);
            }
            let mut out = vec![[0.0; 4]; (rect.width() * rect.height()) as usize];
            s.read_rgba_into(rect, &mut out);
            for (i, actual) in out.iter().enumerate() {
                let expected = s.rgba(rect.x0 + (i % rect.width() as usize) as i32, rect.y0 + (i / rect.width() as usize) as i32);
                assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits), "{depth:?}, alpha={alpha}, pixel={i}");
            }
        }
    }
}

#[test]
fn rgba_reads_match_individual_pixels_bitwise_in_every_model_and_depth() {
    // Across tile edges and a missing tile; float samples include -0, out-of-range and NaN.
    let rect = Rect::new(-3, -1, 300, 2);
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Lab, ColorMode::Cmyk] {
        for depth in SampleType::ALL {
            for alpha in [false, true] {
                let fmt = PixelFormat::new(mode, depth, alpha);
                let mut s = Surface::with_default(fmt, &vec![0.25; fmt.channels()]);
                let values = [-0.0, 0.1, 0.5, 1.0, 1.5, -0.25, f32::NAN, 0.75];
                for (i, x) in (-2..260).enumerate() {
                    let px: Vec<f32> = (0..fmt.channels()).map(|c| values[(i + c * 3) % values.len()]).collect();
                    s.write_pixel(x, 0, &px);
                }
                let mut out = vec![[0.0; 4]; (rect.width() * rect.height()) as usize];
                s.read_rgba_into(rect, &mut out);
                for (i, actual) in out.iter().enumerate() {
                    let expected = s.rgba(rect.x0 + (i % rect.width() as usize) as i32, rect.y0 + (i / rect.width() as usize) as i32);
                    assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits), "{fmt:?}, pixel={i}");
                }
            }
        }
    }
}

#[test]
fn tile_content_bounds_scans_against_the_given_pixel() {
    let mut s = Surface::new(PixelFormat::GRAY8);
    s.write_pixel(3, 4, &[1.0]);
    s.write_pixel(200, 9, &[1.0]);
    let t = s.tile(TileCoord::new(0, 0)).expect("written tile");
    assert_eq!(t.content_bounds(&[0]), Some((3, 4, 201, 10)));
    assert_eq!(t.content_bounds(&[255]).map(|b| (b.0, b.1)), Some((0, 0)));
    // A malformed pixel is no content, not a panic.
    assert_eq!(t.content_bounds(&[]), None);
}

#[test]
fn tile_content_bounds_match_a_per_pixel_scan() {
    // One differing byte at each position of a pixel, at and around the compare-block
    // boundaries, in every pixel size; and a pixel that differs from `dp` in every byte.
    fn per_pixel(t: &Tile, dp: &[u8]) -> Option<(u16, u16, u16, u16)> {
        let (bpp, ts) = (dp.len(), TILE_SIZE as usize);
        let mut b: Option<(usize, usize, usize, usize)> = None;
        for (i, px) in t.bytes().chunks_exact(bpp).enumerate() {
            if px != dp {
                let (x, y) = (i % ts, i / ts);
                let (x0, y0, x1, y1) = b.unwrap_or((x, y, x, y));
                b = Some((x0.min(x), y0.min(y), x1.max(x), y1.max(y)));
            }
        }
        b.map(|(x0, y0, x1, y1)| (x0 as u16, y0 as u16, x1 as u16 + 1, y1 as u16 + 1))
    }
    for bpp in [1, 2, 3, 4, 6, 8, 16] {
        let dp: Vec<u8> = (0..bpp).map(|i| (i * 37 + 5) as u8).collect();
        let row = TILE_SIZE as usize * bpp;
        for spots in [vec![], vec![0], vec![row - 1], vec![63, 64], vec![row * 7 + 127, row * 7 + 128, row * 200 + 5], vec![row * 255 + row / 2]] {
            for byte in 0..bpp {
                let mut data = dp.repeat(TILE_SIZE as usize * TILE_SIZE as usize);
                for &s in &spots {
                    data[s - s % bpp + byte] ^= 0x5a;
                }
                let t = Tile { data: data.into_boxed_slice() };
                assert_eq!(t.content_bounds(&dp), per_pixel(&t, &dp), "bpp {bpp}, spots {spots:?}, byte {byte}");
            }
        }
        let mut data = dp.repeat(TILE_SIZE as usize * TILE_SIZE as usize);
        data[row * 3 + 9 * bpp..row * 3 + 10 * bpp].iter_mut().for_each(|b| *b = !*b);
        let t = Tile { data: data.into_boxed_slice() };
        assert_eq!(t.content_bounds(&dp), Some((9, 3, 10, 4)), "bpp {bpp}");
    }
}

#[test]
fn translated_matches_reading_the_moved_pixels() {
    // Any offset (whole tiles, negative, across tile edges), every depth, a non-zero default.
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAY8] {
        let mut s = Surface::with_default(fmt, &vec![0.25; fmt.channels()]);
        let r = Rect::new(-37, 200, 530, 300);
        let v: Vec<f32> = (0..r.width() * r.height() * fmt.channels() as u32).map(|i| (i % 251) as f32 / 250.0).collect();
        s.write_region(r, &v);
        for (dx, dy) in [(0, 0), (256, -512), (1, 0), (-3, 7), (300, -257), (-1000, 999)] {
            let t = s.translated(dx, dy, s.content_bounds());
            let moved = r.translate(dx, dy).inflate(3);
            assert_eq!(t.read_region(moved), s.read_region(moved.translate(-dx, -dy)), "{fmt:?} by ({dx}, {dy})");
            assert_eq!(t.default_pixel(), s.default_pixel());
            assert_eq!(t.to_interleaved(r.translate(dx, dy)), s.to_interleaved(r), "{fmt:?} by ({dx}, {dy}): not an exact copy");
        }
        // Whole tiles: the tiles are shared, not copied.
        let t = s.translated(512, -256, s.content_bounds());
        assert!(s.tiles().zip(t.tiles()).all(|((_, a), (_, b))| Arc::ptr_eq(a, b)));
        // Tiles without content (allocated, default pixels) are left behind.
        let mut padded = s.clone();
        padded.fill_rect(Rect::new(2000, 2000, 2100, 2100), &s.default_pixel());
        assert_eq!(padded.translated(5, 5, s.content_bounds()).tile_count(), s.translated(5, 5, s.content_bounds()).tile_count());
    }
    // A solid fill moved off the tile grid keeps sharing its one tile inside, at every depth
    // and over a non-zero default pixel, and reads exactly like the copied pixels.
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAY8] {
        let mut s = Surface::with_default(fmt, &vec![0.25; fmt.channels()]);
        let r = Rect::new(-300, -10, 1300, 1030);
        s.fill_rect(r, &vec![0.75; fmt.channels()]);
        let fill = s.tile(TileCoord::new(1, 1)).cloned().expect("an inner fill tile");
        for (dx, dy) in [(10, 7), (-1, 0), (0, 300), (-257, 513)] {
            let t = s.translated(dx, dy, s.content_bounds());
            let moved = r.translate(dx, dy);
            assert_eq!(t.read_region(moved.inflate(3)), s.read_region(moved.inflate(3).translate(-dx, -dy)), "{fmt:?} by ({dx}, {dy})");
            // Targets read only from the fill's whole tiles (x -256..1280, y 0..1024).
            let whole = Rect::new(-256, 0, 1280, 1024).translate(dx, dy);
            let inner: Vec<_> = t.tiles().filter(|(c, _)| whole.contains_rect(&c.rect())).collect();
            assert!(!inner.is_empty() && inner.iter().all(|(_, a)| Arc::ptr_eq(a, &fill)), "{fmt:?} by ({dx}, {dy}): inner tiles copied");
        }
        // Nudged back and forth, the inside keeps sharing one tile (copies of uniform edge
        // tiles count as that value too) and the pixels stay exact.
        let mut t = s.clone();
        for k in 0..8 {
            let d = if k % 2 == 0 { 10 } else { -10 };
            t = t.translated(d, d / 2, t.content_bounds());
        }
        // (Targets whose last window read uniform tiles only: one tile in from the fill's whole
        // tiles on the sides the last move came from.)
        let inner: Vec<_> = t.tiles().filter(|(c, _)| Rect::new(-256, 0, 1024, 768).contains_rect(&c.rect())).map(|(_, a)| a).collect();
        assert!(inner.len() >= 4 && inner.iter().all(|a| Arc::ptr_eq(a, inner[0])), "{fmt:?}: repeated moves copied the inside");
        assert_eq!(t.read_region(r.inflate(3)), s.read_region(r.inflate(3)), "{fmt:?}: back where it started");
    }
    // Two solid fills of different values side by side: windows across the seam are
    // copied (each tile is uniform, but not with the same value), the rest shared.
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA32F] {
        let mut s = Surface::new(fmt);
        s.fill_rect(Rect::new(0, 0, 1024, 768), &vec![0.2; fmt.channels()]);
        s.fill_rect(Rect::new(1024, 0, 2048, 768), &vec![0.9; fmt.channels()]);
        let t = s.translated(10, 7, s.content_bounds());
        let all = Rect::new(-20, -20, 2100, 800);
        assert_eq!(t.read_region(all), s.read_region(all.translate(-10, -7)), "{fmt:?}: two fills");
        let seam = t.tile(TileCoord::new(4, 1)).expect("seam tile");
        let (left, right) = (t.tile(TileCoord::new(2, 1)).expect("left"), t.tile(TileCoord::new(6, 1)).expect("right"));
        assert!(!Arc::ptr_eq(seam, left) && !Arc::ptr_eq(seam, right) && !Arc::ptr_eq(left, right), "{fmt:?}: seam aliased");
    }
    // An offset of i32::MIN saturates its negation, so the source window isn't the exact
    // preimage: no shortcut may apply (a zero pre-fill would leak into the output).
    let mut s = Surface::with_default(PixelFormat::GRAY8, &[0.5]);
    s.fill_rect(Rect::new(i32::MAX - 1024, 0, i32::MAX - 512, 512), &[0.25]);
    let (fill, default) = (s.sample_channel(i32::MAX - 600, 10, 0), s.default_pixel()[0]);
    for (dx, dy) in [(i32::MIN, 1), (1, i32::MIN), (i32::MIN, 0)] {
        let t = s.translated(dx, dy, s.content_bounds());
        for (c, _) in t.tiles() {
            assert!(t.read_region(c.rect()).iter().all(|v| *v == fill || *v == default), "({dx}, {dy}): tile {c:?} holds a pre-fill");
        }
    }
    // One tile shared but not uniform: each target is copied, not aliased.
    let mut s = Surface::new(PixelFormat::GRAY8);
    s.write_pixel(3, 4, &[1.0]);
    let odd = s.tile(TileCoord::new(0, 0)).cloned().expect("written tile");
    let grid: Vec<_> = (0..9).map(|i| (TileCoord::new(i % 3, i / 3), odd.clone())).collect();
    s.put_tiles(grid);
    let t = s.translated(10, 7, s.content_bounds());
    let all = Rect::new(-20, -20, 800, 800);
    assert_eq!(t.read_region(all), s.read_region(all.translate(-10, -7)));
    assert!(t.tiles().all(|(_, a)| !Arc::ptr_eq(a, &odd)));
    // Offsets at the limits don't panic.
    let mut s = Surface::new(PixelFormat::GRAY8);
    s.write_pixel(5, 5, &[1.0]);
    let all = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    let _ = s.translated(i32::MAX, i32::MIN, all);
    let _ = s.translated(i32::MIN + 1, 3, all);
}

/// Many references to one payload, moved off the tile grid: the rebuilt tiles that read the
/// same source tiles at the same offset are one tile, not a copy each.
#[test]
fn translated_shares_tiles_rebuilt_from_shared_sources() {
    let fmt = PixelFormat::RGBA8;
    let mut one = Surface::new(fmt);
    let r = Rect::new(0, 0, TILE_SIZE, TILE_SIZE);
    let px: Vec<f32> = (0..r.width() * r.height()).flat_map(|i| [(i % 251) as f32 / 255.0, (i % 7) as f32 / 7.0, 0.5, 1.0]).collect();
    one.write_region(r, &px);
    let payload = Arc::clone(one.tile(TileCoord::new(0, 0)).unwrap());
    let mut s = Surface::new(fmt);
    s.put_tiles((0..16).flat_map(|ty| (0..16).map(move |tx| TileCoord::new(tx, ty))).map(|c| (c, Arc::clone(&payload))));
    let moved = s.translated(7, 5, s.content_bounds());
    let distinct: std::collections::HashSet<_> = moved.tiles().map(|(_, t)| Arc::as_ptr(t)).collect();
    // The 15 × 15 interior tiles read the same four source tiles at the same offset, so they are
    // one tile, however many threads build them; only the 64 border tiles (cut by the
    // content's edge) are built one by one.
    assert!(moved.tile_count() == 289 && distinct.len() == 64 + 1, "{} tiles, {} distinct", moved.tile_count(), distinct.len());
    let all = Rect::new(-50, -50, 16 * TILE_SIZE + 50, 16 * TILE_SIZE + 50);
    assert_eq!(moved.read_region(all), s.read_region(all.translate(-7, -5)));
}

#[test]
fn a_surface_bounds_tile_count_is_counted_not_walked() {
    let mut s = Surface::new(PixelFormat::RGBA8);
    let far = Rect::new(0, 0, i32::MAX, i32::MAX);
    assert_eq!(far.tile_count(), u64::from(i32::MAX as u32 / TILE_SIZE as u32 + 1).pow(2));
    // Not allocated: 2^46 entries is far past any reservation.
    assert!(s.try_take_tiles(far).is_err());
}

#[test]
fn translated_stores_no_tiles_of_only_the_default_pixel() {
    let only_default = |t: &Surface| {
        let dp = t.default_pixel();
        t.tiles().filter(|(c, _)| t.read_region(c.rect()).chunks_exact(dp.len()).all(|p| p == dp.as_slice())).count()
    };
    // Two blocks a few tiles apart, moved off the tile grid and back: the default-pixel edges
    // of their tiles land on tiles of their own, which aren't stored.
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAY8] {
        for default in [0.0, 0.25] {
            let mut s = Surface::with_default(fmt, &vec![default; fmt.channels()]);
            s.fill_rect(Rect::new(0, 0, 300, 300), &vec![1.0; fmt.channels()]);
            s.fill_rect(Rect::new(2000, 1000, 2300, 1300), &vec![0.5; fmt.channels()]);
            let all = Rect::new(-600, -600, 3000, 2000);
            let mut t = s.clone();
            for (dx, dy) in [(100, 56), (-100, -56), (10, 7), (-3, 40), (130, -90), (-137, 43)] {
                let moved = t.translated(dx, dy, t.content_bounds());
                assert_eq!(moved.read_region(all), t.read_region(all.translate(-dx, -dy)), "{fmt:?} over {default} by ({dx}, {dy})");
                assert_eq!(only_default(&moved), 0, "{fmt:?} over {default} by ({dx}, {dy})");
                t = moved;
            }
            assert_eq!(t.read_region(all), s.read_region(all), "{fmt:?} over {default}: back where it started");
            assert_eq!(t.tile_count(), s.tile_count(), "{fmt:?} over {default}: back where it started");
        }
    }
    // Stored tiles of the default pixel inside `content`: the targets that would share one of
    // them (#4414's uniform windows) are left out too.
    let mut s = Surface::with_default(PixelFormat::GRAY8, &[0.25]);
    s.fill_rect(Rect::new(0, 0, 1024, 1024), &[0.25]);
    s.write_pixel(600, 600, &[1.0]);
    let content = Rect::new(0, 0, 1024, 1024);
    let t = s.translated(10, 7, content);
    assert_eq!(only_default(&t), 0);
    assert_eq!(t.tile_count(), 1);
    assert_eq!(t.read_region(Rect::new(-20, -20, 1100, 1100)), s.read_region(Rect::new(-30, -27, 1090, 1093)));
}

proptest! {
    #[test]
    fn region_write_read_roundtrip(x in -600i32..600, y in -600i32..600, w in 1u32..90, h in 1u32..90, seed in any::<u32>()) {
        let r = Rect::from_xywh(x, y, w, h);
        let n = 4usize;
        let data: Vec<f32> = (0..(w * h) as usize * n).map(|i| ((i as u32).wrapping_mul(2654435761).wrapping_add(seed) % 256) as f32 / 255.0).collect();
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.write_region(r, &data);
        let back = s.read_region(r);
        for (a, b) in back.iter().zip(&data) {
            prop_assert!((a - b).abs() < 1e-6);
        }
        prop_assert_eq!(s.content_bounds().union(&r), r);
    }

    #[test]
    fn clone_mutation_isolated(x in -300i32..300, y in -300i32..300) {
        let mut a = Surface::new(PixelFormat::RGBA8);
        a.fill_rect(Rect::new(-256, -256, 256, 256), &[0.5, 0.5, 0.5, 1.0]);
        let snapshot = a.clone();
        a.write_pixel(x, y, &[1.0, 0.0, 0.0, 1.0]);
        let expect = if Rect::new(-256, -256, 256, 256).contains(x, y) { vec![0.5f32, 0.5, 0.5, 1.0] } else { vec![0.0; 4] };
        let got = snapshot.pixel(x, y);
        for (g, e) in got.iter().zip(&expect) {
            prop_assert!((g - e).abs() <= 1.0 / 255.0);
        }
    }
}

#[test]
fn strided_row_sampling_handles_extreme_coordinates_and_steps() {
    for sample in SampleType::ALL {
        let mut s = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, sample, false), &[0.25]);
        for (x, value) in [(i32::MIN, 0.1), (-1, 0.3), (0, 0.4), (i32::MAX - 1, 0.6), (i32::MAX, 0.9)] {
            s.write_pixel(x, 0, &[value]);
        }
        for y in [0, 256] {
            for (x0, step, len) in [(i32::MAX, 1, 1), (i32::MIN, i32::MAX, 3), (0, i32::MAX, 2), (i32::MAX - 1, 1, 2), (i32::MAX, i32::MAX, 0)] {
                let mut out = vec![0.0; len];
                s.sample_row_strided(y, x0, step, 0, &mut out);
                for (i, &got) in out.iter().enumerate() {
                    let x = i32::try_from(i64::from(x0) + i as i64 * i64::from(step)).unwrap();
                    assert_eq!(got, s.sample_channel(x, y, 0), "{sample:?} y={y} x0={x0} step={step} i={i}");
                }
            }
        }
    }
}

#[test]
fn strided_row_sampling_uses_default_past_coordinate_range() {
    for sample in SampleType::ALL {
        let mut s = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, sample, false), &[0.25]);
        s.write_pixel(i32::MAX, 0, &[0.75]);
        s.write_pixel(i32::MIN, 0, &[1.0]);
        for step in [1, i32::MAX] {
            let mut out = [0.0; 4];
            s.sample_row_strided(0, i32::MAX, step, 0, &mut out);
            assert_eq!(out, [s.sample_channel(i32::MAX, 0, 0), s.default_pixel()[0], s.default_pixel()[0], s.default_pixel()[0]]);
        }
    }
}

#[test]
fn strided_row_sampling_matches_sample_channel() {
    let mut s = Surface::with_default(PixelFormat::GRAY8, &[0.25]);
    s.fill_rect(Rect::new(-300, 10, 200, 40), &[1.0]);
    s.fill_rect(Rect::new(250, 10, 700, 20), &[0.5]);
    for (y, x0, step) in [(15, -520, 7), (15, -1, 1), (30, 3, 64), (500, 0, 9)] {
        let mut out = vec![0.0; 120];
        s.sample_row_strided(y, x0, step, 0, &mut out);
        for (i, v) in out.iter().enumerate() {
            assert_eq!(*v, s.sample_channel(x0 + i as i32 * step, y, 0), "y={y} x0={x0} step={step} i={i}");
        }
    }
}

#[test]
fn content_bounds_with_tile_skipping() {
    // Interior content only; ring tiles present but all-default (unpruned).
    let mut s = Surface::new(PixelFormat::GRAY8);
    s.fill_rect(Rect::new(0, 0, 1024, 1024), &[0.0]);
    s.fill_rect(Rect::new(300, 310, 700, 720), &[1.0]);
    assert_eq!(s.content_bounds(), Rect::new(300, 310, 700, 720));
    // Full-canvas content plus a far-away speck.
    let mut s = Surface::new(PixelFormat::GRAY8);
    s.fill_rect(Rect::new(0, 0, 2000, 1500), &[1.0]);
    s.fill_rect(Rect::new(-700, 4000, -699, 4001), &[0.5]);
    assert_eq!(s.content_bounds(), Rect::new(-700, 0, 2000, 4001));
}
