use super::*;

/// A run joins `LayerBlend` passes that only feed each other, at most `BATCH` of them, and
/// never ends in the slot it reads from.
fn check_runs(p: &Plan<'_>) -> Vec<usize> {
    let runs = batch_runs(p);
    assert_eq!(runs.len(), p.passes.len());
    for (i, &k) in runs.iter().enumerate() {
        assert!((1..=BATCH).contains(&k), "pass {i}: run {k}");
        if k > 1 {
            assert_ne!(Some(p.passes[i + k - 1].dst), p.passes[i].a, "run at {i} renders into its input");
            for j in i + 1..i + k {
                assert_eq!(p.passes[j].kernel, Kernel::LayerBlend);
                assert_eq!(p.passes[j].a, Some(p.passes[j - 1].dst), "run at {i}: pass {j} doesn't read the one before");
                assert!(p.passes[j].mask.is_none() && p.passes[j].mode != photocraft_color::BlendMode::Dissolve, "run at {i}: pass {j}");
            }
        }
    }
    runs
}

#[test]
fn plain_layers_batch_and_runs_stop_where_a_pass_must_stand_alone() {
    use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
    use photocraft_doc::{Document, Layer, LayerMask};
    let mut d = Document::with_background("t", photocraft_geom::Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    for i in 0..12 {
        let mut l = Layer::raster(format!("l{i}"), d.pixel_format());
        l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 8, 8), &[0.2, 0.4, 0.6, 0.5]);
        d.layers.push(l);
    }
    let p = plan::plan(&d).unwrap();
    let runs = check_runs(&p);
    assert!(runs.iter().any(|k| *k >= 3), "{runs:?}");
    // The cache resumes only where a run starts, so a cached render draws the passes above
    // its checkpoint in the same runs as an uncached one; checkpoints inside a run are
    // skipped, but a stack of plain layers keeps some to resume at.
    let starts = run_starts(&runs);
    assert_eq!(starts.len(), runs.len() + 1);
    assert!(starts[0] && starts[runs.len()]);
    let (mut at, mut i) = (Vec::new(), 0);
    while i < runs.len() {
        at.push(i);
        i += runs[i];
    }
    assert_eq!(starts.iter().enumerate().filter(|(_, s)| **s).map(|(i, _)| i).collect::<Vec<_>>(), [at, vec![runs.len()]].concat());
    let resumable = p.checkpoints.iter().filter(|c| c.pass_end > 1 && starts[c.pass_end]).count();
    assert!(resumable >= 2 && resumable < p.checkpoints.len(), "{resumable} of {:?}", p.checkpoints);
    // A masked or Dissolve layer, or a clipped one, never joins a run as an extra layer.
    d.layers[3].mask = Some(LayerMask::hide_all());
    d.layers[6].blend = BlendMode::Dissolve;
    d.layers[9].clipped = true;
    check_runs(&plan::plan(&d).unwrap());
    // Nothing to batch: a lone layer.
    d.layers.truncate(1);
    assert_eq!(check_runs(&plan::plan(&d).unwrap()), vec![1; plan::plan(&d).unwrap().passes.len()]);
}

#[test]
fn batch_records_hold_three_extra_layers_in_the_spare_words() {
    let lead = plan::Pass::new(Kernel::LayerBlend, 0);
    let mut x = plan::Pass::new(Kernel::LayerBlend, 1);
    x.opacity = 0.25;
    x.fill = 0.5;
    x.color = [0.1, 0.2, 0.3, 0.4];
    let w = batch_op_words(&lead, None, None, &[(&x, Some([1, 2, 3, 4])), (&x, None), (&x, None), (&x, None)]);
    assert_eq!(w.len() as u64, OP_UNIFORM);
    let word = |i: usize| <[u8; 4]>::try_from(&w[i * 4..i * 4 + 4]).unwrap();
    let f = |i: usize| f32::from_le_bytes(word(i));
    // At most three extras; the first's geometry, colour and parameters in p0, p1, p2.
    assert_eq!(i32::from_le_bytes(word(1)), 3);
    assert_eq!([f(20), f(21), f(22), f(23)], [1.0, 2.0, 3.0, 4.0]);
    assert_eq!([f(24), f(25), f(26), f(27)], [0.1, 0.2, 0.3, 0.4]);
    assert_eq!([f(28), f(29), f(31)], [0.25, 0.5, 1.0]);
    // The third's parameters in q4; one without pixels says so.
    assert_eq!([f(60), f(61), f(63)], [0.25, 0.5, 0.0]);
    // Other passes leave the batch words zero.
    let plain = op_words(&x, None, None, None);
    assert!(plain[44 * 4..].iter().all(|b| *b == 0));
}

#[test]
fn runs_of_normal_layers_draw_with_normal_code_alone() {
    use photocraft_color::BlendMode;
    let pass = |mode: BlendMode, flags: u32| {
        let mut p = plan::Pass::new(Kernel::LayerBlend, 0);
        p.mode = mode;
        p.flags = flags;
        p
    };
    let normal = pass(BlendMode::Normal, plan::F_HDR);
    assert_eq!(batch_kernel(&[normal.clone(), pass(BlendMode::Normal, 0)]), Kernel::LayerBatchNormal);
    // Any other mode, text gamma or a Lab document anywhere in the run takes the full shader.
    for other in [pass(BlendMode::Multiply, 0), pass(BlendMode::Dissolve, 0), pass(BlendMode::Normal, plan::F_TEXT_GAMMA), pass(BlendMode::Normal, plan::F_LAB)]
    {
        assert_eq!(batch_kernel(&[normal.clone(), other.clone()]), Kernel::LayerBatch);
        assert_eq!(batch_kernel(&[other, normal.clone()]), Kernel::LayerBatch);
    }
}

#[test]
fn layer_pages_are_admitted_by_evicting_least_recently_used_first() {
    let pages = || vec![(5, 'c', 40), (1, 'a', 30), (3, 'b', 20)];
    // Fits already: nothing dropped.
    assert!(lru_victims(pages(), 90, 10, 100).is_empty());
    // 90 held + 40 incoming over 100: the oldest page goes first, then the next.
    assert_eq!(lru_victims(pages(), 90, 40, 100), vec!['a']);
    assert_eq!(lru_victims(pages(), 90, 60, 100), vec!['a', 'b']);
    // Out of memory: a zero budget drops every candidate, and an impossible request stops
    // when none are left instead of looping or overflowing.
    assert_eq!(lru_victims(pages(), 90, 0, 0), vec!['a', 'b', 'c']);
    assert_eq!(lru_victims(pages(), u64::MAX, u64::MAX, 1), vec!['a', 'b', 'c']);
    assert!(lru_victims(Vec::<(u64, char, u64)>::new(), 90, 40, 100).is_empty());
}

#[test]
fn uniform_buffer_sizes_are_checked_before_allocation() {
    assert_eq!(uniform_buffer_len(1, 255, 65_536).unwrap(), 65_536);
    assert!(uniform_buffer_len(1, 255, 65_535).is_err());
    assert!(uniform_buffer_len(usize::MAX, 1, u64::MAX).is_err());
    assert!(uniform_buffer_len(usize::MAX, 0, u64::MAX).is_err());
    assert!(uniform_buffer_len(1, 16_777_216, u64::MAX).is_err(), "the last record offset cannot be represented by u32");
    #[cfg(target_pointer_width = "64")]
    assert_eq!(uniform_buffer_len(1, 16_777_215, u64::MAX).unwrap(), 4_294_967_296, "the last valid u32 dynamic offset is accepted");
}

#[test]
fn half_roundtrip() {
    for v in [0.0f32, 1.0, 0.5, 0.25, 1.0 / 255.0, 0.333, 65504.0, -2.5, 1e-6] {
        let r = f16_to_f32(f32_to_f16(v));
        assert!((r - v).abs() <= v.abs() / 1024.0 + 1e-7, "{v} -> {r}");
    }
}

#[test]
fn shader_validates() {
    let module = wgpu::naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
    wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{e:?}"));
    for k in Kernel::DRAWN {
        let e = k.entry().unwrap();
        assert!(module.entry_points.iter().any(|ep| ep.name == e), "missing entry point {e}");
    }
}

/// The decimal float literals in `src` (comments skipped; hex literals are exact, so skipped),
/// with their line numbers.
fn decimal_float_literals(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (n, line) in src.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("").as_bytes();
        let mut i = 0;
        while i < code.len() {
            let digit_or_dot = code[i].is_ascii_digit() || (code[i] == b'.' && code.get(i + 1).is_some_and(u8::is_ascii_digit));
            let word_before = i > 0 && (code[i - 1].is_ascii_alphanumeric() || code[i - 1] == b'_' || code[i - 1] == b'.');
            if !digit_or_dot || word_before {
                i += 1;
                continue;
            }
            if code[i] == b'0' && matches!(code.get(i + 1), Some(b'x' | b'X')) {
                i += 2;
                while i < code.len() && (code[i].is_ascii_hexdigit() || matches!(code[i], b'.' | b'p' | b'P' | b'+' | b'-')) {
                    i += 1;
                }
                continue;
            }
            let start = i;
            while i < code.len() && (code[i].is_ascii_digit() || code[i] == b'.') {
                i += 1;
            }
            if i < code.len() && matches!(code[i], b'e' | b'E') {
                let mut j = i + 1;
                if j < code.len() && matches!(code[j], b'+' | b'-') {
                    j += 1;
                }
                if j < code.len() && code[j].is_ascii_digit() {
                    i = j;
                    while i < code.len() && code[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            out.push((n + 1, String::from_utf8_lossy(&code[start..i]).into_owned()));
        }
    }
    out
}

/// Browsers' WebGPU shader compiler (Tint) rejects a float literal beyond f32's finite range
/// even when it would round to `f32::MAX` (naga, used natively, rounds it), and then no
/// compositor pipeline builds and the web app falls back to the CPU renderer. `3.40282347e38`
/// was such a literal.
#[test]
fn shader_float_literals_fit_f32() {
    let lits = decimal_float_literals(SHADER);
    assert!(lits.iter().any(|(_, l)| l == "1.0"), "the scanner finds literals: {lits:?}");
    let bad: Vec<_> = lits.iter().filter(|(_, l)| l.parse::<f64>().map_or(true, |v| !v.is_finite() || v.abs() > f64::from(f32::MAX))).collect();
    assert!(bad.is_empty(), "float literals outside f32's finite range (line, literal): {bad:?}");
}

#[test]
fn op_record_fits_the_uniform() {
    let p = plan::Pass::new(Kernel::FxPaint, 0);
    assert_eq!(op_words(&p, None, None, None).len() as u64, OP_UNIFORM);
}

/// #6534: a 16-bit mask (16-bit documents, feathered or vector combined masks) costs two
/// bytes per pixel on the GPU, twice an 8-bit mask, not four; 32-bit masks stay R32Float.
#[test]
fn sixteen_bit_masks_upload_at_two_bytes_per_pixel() {
    let gray = |sample| PixelFormat { sample, ..PixelFormat::GRAY8 };
    assert_eq!(TexKind::for_surface(Role::Mask, PixelFormat::GRAY8).bytes_per_pixel(), 1);
    let k16 = TexKind::for_surface(Role::Mask, gray(SampleType::U16));
    assert_eq!(k16.bytes_per_pixel(), 2, "{k16:?}");
    assert_eq!(k16.format().block_copy_size(None), Some(2), "{k16:?}");
    let k32 = TexKind::for_surface(Role::Mask, gray(SampleType::F32));
    assert_eq!(k32.format(), wgpu::TextureFormat::R32Float);
    // The texels hold the mask values (a half each), the default pixel included.
    let mut s = Surface::with_default(gray(SampleType::U16), &[1.0]);
    s.fill_rect(Rect::new(0, 0, 4, 1), &[0.25]);
    let c = TileCoord::new(0, 0);
    let bytes = convert_tile(&s, s.tile(c), k16, c);
    assert_eq!(bytes.len(), (TILE_SIZE * TILE_SIZE) as usize * 2);
    assert_eq!(bytes.get(..2), Some(f32_to_f16(0.25).to_le_bytes().as_slice()));
    assert_eq!(bytes.get(8..10), Some(f32_to_f16(1.0).to_le_bytes().as_slice()));
    assert_eq!(convert_tile(&s, None, k16, c).get(..2), Some(f32_to_f16(1.0).to_le_bytes().as_slice()));
}

/// #1774: a solid fill shares one tile across every coordinate; it is converted once, and
/// every coordinate (shared or not) still gets exactly the bytes `convert_tile` gives it, at
/// every depth and texture kind.
#[test]
fn convert_tiles_converts_each_distinct_tile_once() {
    let area = Rect::new(0, 0, TILE_SIZE * 9, TILE_SIZE * 9);
    for (fmt, kind) in [
        (PixelFormat::RGBA8, TexKind::Rgba8Direct),
        (PixelFormat { alpha: false, ..PixelFormat::RGBA8 }, TexKind::Rgba8),
        (PixelFormat::RGBA16, TexKind::Rgba16F),
        (PixelFormat::RGBA32F, TexKind::Rgba16F),
        (PixelFormat::GRAY8, TexKind::R8Direct),
        (PixelFormat { sample: SampleType::U16, ..PixelFormat::GRAY8 }, TexKind::R16F),
        (PixelFormat { sample: SampleType::F32, ..PixelFormat::GRAY8 }, TexKind::R32F),
    ] {
        let mut s = Surface::new(fmt);
        let px = [0.2, 0.4, 0.6, 1.0];
        let n = fmt.channels();
        // Fully covered tiles share one `Arc`; then two tiles get their own pixels.
        s.fill_rect(area, &px[..n]);
        s.fill_rect(Rect::new(3, 5, 40, 60), &[0.9, 0.1, 0.3, 0.5][..n]);
        s.fill_rect(Rect::new(TILE_SIZE * 4 + 7, TILE_SIZE * 2, TILE_SIZE * 4 + 9, TILE_SIZE * 2 + 3), &[0.0; 4][..n]);
        let tiles: Vec<_> = area.tiles().filter_map(|c| s.tile(c).map(|t| (c, t.clone()))).collect();
        assert_eq!(tiles.len(), 81, "{fmt:?}");
        let mut seen = Vec::new();
        let converted = convert_tiles(&s, tiles, kind, |t, cs, bytes| {
            for &c in cs {
                assert!(s.tile(c).is_some_and(|own| Arc::ptr_eq(own, t)), "{fmt:?} {c:?}");
                assert_eq!(bytes, convert_tile(&s, Some(t), kind, c).as_slice(), "{fmt:?} {c:?}");
                seen.push(c);
            }
        });
        assert_eq!(converted, 3, "{fmt:?}: the shared tile once, plus the two edited ones");
        seen.sort_by_key(|c| (c.ty, c.tx));
        let mut want: Vec<_> = area.tiles().collect();
        want.sort_by_key(|c| (c.ty, c.tx));
        assert_eq!(seen, want, "{fmt:?}: every coordinate once");
    }
    // More distinct tiles than one batch: all converted, each coordinate once.
    let mut s = Surface::new(PixelFormat::RGBA16);
    let wide = Rect::new(0, 0, TILE_SIZE * (CONVERT_BATCH as i32 + 5), TILE_SIZE);
    for c in wide.tiles() {
        s.fill_rect(Rect::new(c.rect().x0, 0, c.rect().x0 + 1, 1), &[c.tx as f32 / 100.0, 0.0, 0.0, 1.0]);
    }
    let tiles: Vec<_> = wide.tiles().filter_map(|c| s.tile(c).map(|t| (c, t.clone()))).collect();
    let mut n = 0;
    let converted = convert_tiles(&s, tiles, TexKind::Rgba16F, |t, cs, bytes| {
        for &c in cs {
            assert_eq!(bytes, convert_tile(&s, Some(t), TexKind::Rgba16F, c).as_slice());
            n += 1;
        }
    });
    assert_eq!((converted, n), (CONVERT_BATCH + 5, CONVERT_BATCH + 5));
}

/// The 16-bit and 32-bit float RGB/gray fast paths give exactly the general path's half
/// floats (32-bit samples above 1 and below 0 included).
#[test]
fn sixteen_and_thirty_two_bit_fast_paths_match_the_general_one() {
    for fmt in [
        PixelFormat::RGBA16,
        PixelFormat { alpha: false, ..PixelFormat::RGBA16 },
        PixelFormat { sample: SampleType::U16, ..PixelFormat::GRAY8 },
        PixelFormat { sample: SampleType::U16, alpha: true, ..PixelFormat::GRAY8 },
        PixelFormat::RGBA32F,
        PixelFormat { alpha: false, ..PixelFormat::RGBA32F },
        PixelFormat { sample: SampleType::F32, ..PixelFormat::GRAY8 },
        PixelFormat { sample: SampleType::F32, alpha: true, ..PixelFormat::GRAY8 },
    ] {
        let mut s = Surface::new(fmt);
        let r = Rect::new(0, 0, TILE_SIZE, TILE_SIZE);
        let n = (TILE_SIZE * TILE_SIZE) as usize * fmt.channels();
        let hdr = if fmt.sample == SampleType::F32 { 4.0 } else { 1.0 };
        let data: Vec<f32> = (0..n).map(|i| (i as u32).wrapping_mul(2_654_435_761) as f32 / u32::MAX as f32 * hdr - (hdr - 1.0) / 4.0).collect();
        s.write_region(r, &data);
        let t = s.tile(TileCoord::new(0, 0)).expect("written").clone();
        let fast = rgb_gray16_to_f16(&fmt, t.bytes()).or_else(|| rgb_gray32_to_f16(&fmt, t.bytes())).expect("fast path");
        let raw = s.read_region(r);
        let general: Vec<u8> =
            raw.chunks_exact(fmt.channels()).flat_map(|px| photocraft_raster::to_rgba(&fmt, px).map(|x| f32_to_f16(x).to_le_bytes())).flatten().collect();
        assert_eq!(fast, general, "{fmt:?}");
        assert_eq!(convert_tile(&s, Some(&t), TexKind::Rgba16F, TileCoord::new(0, 0)), general, "{fmt:?}");
    }
    assert!(rgb_gray16_to_f16(&PixelFormat::RGBA8, &[]).is_none());
    assert!(rgb_gray16_to_f16(&PixelFormat { sample: SampleType::U16, ..PixelFormat::CMYKA8 }, &[]).is_none());
    assert!(rgb_gray32_to_f16(&PixelFormat::RGBA16, &[]).is_none());
    assert!(rgb_gray32_to_f16(&PixelFormat { sample: SampleType::F32, ..PixelFormat::CMYKA8 }, &[]).is_none());
}

/// CMYK and Lab tiles convert in parallel bands of rows, each run of equal pixels once
/// (#5108): the texels are still exactly the per-pixel conversion's, at 8 and 16 bits, with
/// the built-in CMYK profile and with a document's own (which the worker threads enter).
#[test]
fn colour_managed_tiles_match_the_per_pixel_conversion() {
    use photocraft_cms::synth::{CmykParams, cmyk_profile};
    use photocraft_color::ColorMode;
    use photocraft_color::convert::{CmykSpace, with_cmyk_space};
    let icc = Arc::new(
        cmyk_profile(&CmykParams { description: "GPU test CMYK".into(), tvi: [0.26, 0.26, 0.26, 0.3], grid_a2b: 5, grid_b2a: 9, ..Default::default() })
            .to_bytes(),
    );
    let own = CmykSpace::for_profile(Some(&icc));
    assert!(own.is_some(), "the synthetic profile is not the built-in one");
    let area = Rect::new(0, 0, TILE_SIZE * 3, TILE_SIZE);
    for (mode, sample, space) in [
        (ColorMode::Cmyk, SampleType::U8, None),
        (ColorMode::Cmyk, SampleType::U16, None),
        (ColorMode::Cmyk, SampleType::U8, own.clone()),
        (ColorMode::Cmyk, SampleType::U16, own.clone()),
        (ColorMode::Lab, SampleType::U8, None),
        (ColorMode::Lab, SampleType::U16, None),
    ] {
        let fmt = PixelFormat { mode, sample, alpha: true };
        let kind = TexKind::for_surface(Role::Content, fmt);
        assert!(colour_managed(&fmt, kind), "{fmt:?}");
        let ch = fmt.channels();
        let mut s = Surface::new(fmt);
        // Tile 0: noise. Tile 1: transparent with a soft dab (runs of equal pixels). Tile 2:
        // a flat colour with a few lines of noise.
        let n = (TILE_SIZE * TILE_SIZE) as usize * ch;
        let noise: Vec<f32> = (0..n).map(|i| (i as u32).wrapping_mul(2_654_435_761) as f32 / u32::MAX as f32).collect();
        s.write_region(Rect::new(0, 0, TILE_SIZE, TILE_SIZE), &noise);
        let mut dab = [0.8, 0.6, 0.1, 0.05, 0.0];
        for r in (1..30).rev() {
            dab[ch - 1] = 1.0 - r as f32 / 30.0;
            s.fill_rect(Rect::new(TILE_SIZE + 100 - r, 90 - r, TILE_SIZE + 100 + r, 90 + r), &dab[..ch]);
        }
        s.fill_rect(Rect::new(TILE_SIZE * 2, 0, TILE_SIZE * 3, TILE_SIZE), &[0.3, 0.2, 0.7, 0.1, 1.0][..ch]);
        s.write_region(Rect::new(TILE_SIZE * 2, 40, TILE_SIZE * 3, 43), &noise[..TILE_SIZE as usize * 3 * ch]);
        let tiles: Vec<_> = area.tiles().filter_map(|c| s.tile(c).map(|t| (c, t.clone()))).collect();
        assert_eq!(tiles.len(), 3, "{fmt:?}");
        let mut seen = 0;
        with_cmyk_space(space.as_ref(), || {
            convert_tiles(&s, tiles, kind, |_, cs, bytes| {
                for &c in cs {
                    // The old path: every pixel through `to_rgba` on this thread, in this scope.
                    let want: Vec<u8> = s
                        .read_region(c.rect())
                        .chunks_exact(ch)
                        .flat_map(|px| {
                            let v = photocraft_raster::to_rgba(&fmt, px);
                            match kind {
                                TexKind::Rgba16F => v.iter().flat_map(|x| f32_to_f16(*x).to_le_bytes()).collect::<Vec<u8>>(),
                                _ => v.map(|x| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).to_vec(),
                            }
                        })
                        .collect();
                    assert!(bytes == want.as_slice(), "{fmt:?} {c:?} with {}", if space.is_some() { "the document's profile" } else { "the built-in profile" });
                    assert_eq!(convert_tile(&s, s.tile(c), kind, c), want, "{fmt:?} {c:?}");
                    seen += 1;
                }
            });
        });
        assert_eq!(seen, 3, "{fmt:?}");
    }
}

#[test]
fn grid_rounds_out() {
    assert_eq!(tile_grid(Rect::new(0, 0, 300, 256)), Rect::new(0, 0, 512, 256));
}
