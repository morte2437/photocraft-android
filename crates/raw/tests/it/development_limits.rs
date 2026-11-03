//! Development limits and hostile public sensor metadata.
use photocraft_raw::testgen::DngSpec;
use photocraft_raw::*;

fn small_sensor() -> Sensor {
    decode(&DngSpec::cfa(64, 48, vec![1000; 64 * 48]).build(), &Limits::default()).unwrap()
}

#[test]
fn checks_peak_live_buffers_instead_of_summing_disjoint_phases() {
    let (w, h) = (128u64, 96u64);
    let sensor = decode(&DngSpec::cfa(w as usize, h as usize, vec![1000; (w * h) as usize]).build(), &Limits::default()).unwrap();
    let table = (4096 + 2) * 4;
    // Exactly `max_alloc` must work and one byte less must fail.
    let check = |opts: &mut DevelopOptions, max_alloc: u64| {
        let expected = develop_sensor(&sensor, opts).unwrap();
        opts.limits.max_alloc = max_alloc;
        assert_eq!(develop_sensor(&sensor, opts).unwrap().rgb, expected.rgb);
        opts.limits.max_alloc -= 1;
        assert!(matches!(develop_sensor(&sensor, opts), Err(RawError::LimitExceeded(_))));
    };
    // A tone slider needs the f32 RGB frame next to the output (18 bytes a pixel) ...
    let mut opts = DevelopOptions { demosaic: Demosaic::Mhc, tone: Tone { shadows: 20.0, ..Tone::NEUTRAL }, ..Default::default() };
    check(&mut opts, w * h * 18 + table);
    // ... the default rendering the u16 output and, per worker, one band (32 rows) of f32 RGB with
    // the 12 rows more of padded CFA it reads, but no whole plane.
    let mut opts = DevelopOptions { demosaic: Demosaic::Mhc, ..Default::default() };
    let band = 32 * w * 12 + (32 + 12) * (w + 12) * 4;
    rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| check(&mut opts, w * h * 6 + table + band));
}

#[test]
fn monochrome_and_rgb_linear_raw_preserve_output_and_memory_limits() {
    let (w, h) = (32, 24);
    for samples in [1, 3] {
        let data = (0..w * h).flat_map(|i| (0..samples).map(move |c| (i % w * 1000 + c * 5000) as u16)).collect();
        let spec = DngSpec { samples, monochrome: samples == 1, ..DngSpec::cfa(w, h, data) };
        let sensor = decode(&spec.build(), &Limits::default()).unwrap();
        assert!(sensor.cfa.is_none());
        assert_eq!(sensor.samples, samples);
        let expected = develop_sensor(&sensor, &DevelopOptions::default()).unwrap();
        let mut opts = DevelopOptions::default();
        opts.limits.max_alloc = 32 * 1024;
        let got = develop_sensor(&sensor, &opts).unwrap();
        assert_eq!(got.rgb, expected.rgb);
        assert_eq!((got.width, got.height), (w as u32, h as u32));
        if samples == 1 {
            assert!(got.rgb.as_chunks::<3>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2]));
        }
        // Even the encoded RGB output alone cannot fit this budget.
        opts.limits.max_alloc = (expected.rgb.len() * 2 - 1) as u64;
        assert!(matches!(develop_sensor(&sensor, &opts), Err(RawError::LimitExceeded(_))));
    }
}

#[test]
fn invalid_public_sensor_metadata_returns_errors() {
    let sensor = small_sensor();
    let mut cases = Vec::new();
    let mut s = sensor.clone();
    s.crop.x = usize::MAX;
    cases.push(s);
    let mut s = sensor.clone();
    s.width = usize::MAX;
    cases.push(s);
    let mut s = sensor.clone();
    s.samples = usize::MAX;
    cases.push(s);
    let mut s = sensor.clone();
    s.cfa.as_mut().unwrap().width = 0;
    cases.push(s);
    let mut s = sensor.clone();
    s.cfa.as_mut().unwrap().colors.clear();
    cases.push(s);
    let mut s = sensor.clone();
    s.cfa.as_mut().unwrap().colors[0] = 255;
    cases.push(s);
    let mut s = sensor.clone();
    s.cfa = None;
    s.samples = 2;
    s.data.resize(64 * 48 * 2, 1000);
    cases.push(s);
    let mut s = sensor.clone();
    s.samples = 3;
    s.data.resize(64 * 48 * 3, 1000);
    cases.push(s);
    for s in cases {
        assert!(develop_sensor(&s, &DevelopOptions::default()).is_err());
    }
}

#[test]
fn extreme_cfa_and_black_origins_do_not_overflow() {
    let mut sensor = small_sensor();
    let cfa = sensor.cfa.as_mut().unwrap();
    cfa.origin_x = usize::MAX;
    cfa.origin_y = usize::MAX;
    assert!(cfa.is_bayer());
    sensor.black.rows = usize::MAX;
    sensor.black.cols = usize::MAX;
    assert!(develop_sensor(&sensor, &DevelopOptions::default()).is_ok());
    assert_eq!(Cfa { width: 0, height: 0, colors: vec![], origin_x: 0, origin_y: 0 }.color(1, 2), 1);
}
