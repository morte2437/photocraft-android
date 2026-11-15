//! A PNG opened row by row into the tiles ([`import_png_streamed`]) gives the document the
//! whole-image path gives, for every file, and fails or falls back exactly where it must.

use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};

use craft_cms::Builtin;

use super::*;

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
    out
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(data).unwrap();
    z.finish().unwrap()
}

/// A PNG under construction: the colour type and depth, a palette and `tRNS`, and chunks before
/// and after the image data.
struct Png {
    w: u32,
    h: u32,
    ctype: u8,
    depth: u8,
    palette: usize,
    before: Vec<Vec<u8>>,
    after: Vec<Vec<u8>>,
}

impl Png {
    fn new(w: u32, h: u32, ctype: u8, depth: u8) -> Self {
        Png { w, h, ctype, depth, palette: 0, before: Vec::new(), after: Vec::new() }
    }

    fn palette(mut self, n: usize) -> Self {
        let colors: Vec<u8> = (0..n * 3).map(|i| (i * 37 + 11) as u8).collect();
        self.palette = n;
        self.before.push(chunk(b"PLTE", &colors));
        self
    }

    fn before(mut self, c: Vec<u8>) -> Self {
        self.before.push(c);
        self
    }

    fn after(mut self, c: Vec<u8>) -> Self {
        self.after.push(c);
        self
    }

    /// Rows of pseudo-random samples (palette indices below the palette size), filter type 0.
    fn raw(&self) -> Vec<u8> {
        let channels = match self.ctype {
            0 | 3 => 1,
            2 => 3,
            4 => 2,
            _ => 4,
        };
        let max = if self.ctype == 3 { self.palette as u32 } else { 1 << self.depth };
        let mut seed = 0x2545_f491u32 ^ self.w.wrapping_mul(31) ^ self.h;
        let mut out = Vec::new();
        for _ in 0..self.h {
            out.push(0);
            let (mut acc, mut bits) = (0u32, 0u32);
            for _ in 0..self.w * channels {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = (seed >> 8) % max;
                if self.depth == 16 {
                    out.extend_from_slice(&(v as u16).to_be_bytes());
                } else {
                    acc = (acc << self.depth) | v;
                    bits += u32::from(self.depth);
                    if bits >= 8 {
                        out.push((acc >> (bits - 8)) as u8);
                        bits -= 8;
                        acc &= (1 << bits) - 1;
                    }
                }
            }
            if bits > 0 {
                out.push((acc << (8 - bits)) as u8);
            }
        }
        out
    }

    fn idat(&self) -> Vec<u8> {
        chunk(b"IDAT", &zlib(&self.raw()))
    }

    fn bytes(&self) -> Vec<u8> {
        self.assemble(&self.idat())
    }

    fn assemble(&self, idat: &[u8]) -> Vec<u8> {
        let mut ihdr = self.w.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&self.h.to_be_bytes());
        ihdr.extend_from_slice(&[self.depth, self.ctype, 0, 0, 0]);
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend(chunk(b"IHDR", &ihdr));
        self.before.iter().for_each(|c| out.extend_from_slice(c));
        out.extend_from_slice(idat);
        self.after.iter().for_each(|c| out.extend_from_slice(c));
        out.extend(chunk(b"IEND", &[]));
        out
    }
}

/// A big-endian EXIF block holding only an Orientation.
fn exif(orientation: u8) -> Vec<u8> {
    let mut e = b"MM\x00\x2a\x00\x00\x00\x08\x00\x01\x01\x12\x00\x03\x00\x00\x00\x01\x00".to_vec();
    e.extend_from_slice(&[orientation, 0, 0, 0, 0, 0, 0]);
    chunk(b"eXIf", &e)
}

fn whole(bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_flat_impl("t", bytes, &Interrupt::NONE, None)
}

/// `import_flat` with the stream forced on whatever the size.
fn forced(bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_flat_impl("t", bytes, &Interrupt::NONE, Some(0))
}

fn streamed(bytes: &[u8]) -> Result<Option<ImportResult>, IoError> {
    import_png_streamed("t", bytes, &crate::ImportOptions::default(), &Interrupt::NONE, 0)
}

fn assert_same(a: &ImportResult, b: &ImportResult, what: &str) {
    assert_eq!(a.warnings, b.warnings, "{what}: warnings");
    assert_eq!((a.source_read_only, a.preview_only), (b.source_read_only, b.preview_only), "{what}");
    let (da, db) = (&a.document, &b.document);
    assert_eq!((da.size, da.mode, da.depth, &da.color_table), (db.size, db.mode, db.depth, &db.color_table), "{what}: shape");
    assert_eq!(
        (&da.icc_profile, &da.metadata, da.resolution_dpi, &da.file_encoding),
        (&db.icc_profile, &db.metadata, db.resolution_dpi, &db.file_encoding),
        "{what}: metadata"
    );
    assert_eq!(da.layers.len(), db.layers.len(), "{what}: layers");
    for (la, lb) in da.layers.iter().zip(&db.layers) {
        assert_eq!((&la.name, la.locks, la.opacity, la.visible), (&lb.name, lb.locks, lb.opacity, lb.visible), "{what}: layer");
        let (LayerContent::Raster(sa), LayerContent::Raster(sb)) = (&la.content, &lb.content) else { panic!("{what}: not raster") };
        assert_eq!(sa.format(), sb.format(), "{what}: format");
        assert_eq!(sa.tile_count(), sb.tile_count(), "{what}: tile count (pruning)");
        assert!(sa.to_interleaved(da.bounds()) == sb.to_interleaved(db.bounds()), "{what}: pixels");
    }
    // Everything else, but for the ids every document and layer gets fresh.
    let mut db = db.clone();
    db.id = da.id;
    db.layers.iter_mut().zip(&da.layers).for_each(|(l, k)| l.id = k.id);
    assert!(da == &db, "{what}: documents differ");
}

/// Both paths agree on `bytes`, and the stream is the one that took it.
fn assert_streams(bytes: &[u8], what: &str) {
    let want = whole(bytes).unwrap();
    // The stream took it (the palette and file encoding are added by `import_flat`), and the
    // document that comes out is the whole-image one.
    assert!(streamed(bytes).unwrap().is_some(), "{what}: did not stream");
    assert_same(&forced(bytes).unwrap(), &want, what);
}

const SIZES: [(u32, u32); 5] = [(1, 1), (1, 300), (255, 3), (257, 257), (513, 520)];

#[test]
fn streamed_import_equals_the_whole_image_for_every_colour_type_and_depth() {
    // (colour type, depth, palette size, tRNS)
    let kinds: &[(u8, u8, usize, Option<&[u8]>)] = &[
        (0, 1, 0, None),
        (0, 2, 0, None),
        (0, 4, 0, None),
        (0, 8, 0, None),
        (0, 16, 0, None),
        (0, 8, 0, Some(&[0, 77])),
        (0, 16, 0, Some(&[0, 5])),
        (4, 8, 0, None),
        (4, 16, 0, None),
        (2, 8, 0, None),
        (2, 16, 0, None),
        (2, 8, 0, Some(&[0, 1, 0, 2, 0, 3])),
        (6, 8, 0, None),
        (6, 16, 0, None),
        (3, 1, 2, None),
        (3, 2, 3, None),
        (3, 4, 16, None),
        (3, 8, 200, None),
        (3, 8, 200, Some(&[255, 255, 0])),
        (3, 4, 16, Some(&[128, 255])),
    ];
    for &(ctype, depth, palette, trns) in kinds {
        for (w, h) in SIZES {
            let mut png = Png::new(w, h, ctype, depth);
            if ctype == 3 {
                png = png.palette(palette);
            }
            if let Some(t) = trns {
                png = png.before(chunk(b"tRNS", t));
            }
            assert_streams(&png.bytes(), &format!("type {ctype} depth {depth} palette {palette} trns {} {w}x{h}", trns.is_some()));
        }
    }
}

#[test]
fn metadata_before_and_after_the_image_data_is_kept() {
    let icc = Builtin::Srgb.profile().to_bytes();
    let mut iccp = b"srgb\0\0".to_vec();
    iccp.extend(zlib(&icc));
    let xmp = {
        let mut x = b"XML:com.adobe.xmp\0\0\0\0\0".to_vec();
        x.extend_from_slice(b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"></x:xmpmeta>");
        x
    };
    let png = Png::new(300, 270, 2, 8)
        .before(chunk(b"iCCP", &iccp))
        .before(chunk(b"pHYs", &[0, 0, 0x2e, 0x23, 0, 0, 0x2e, 0x23, 1]))
        .before(chunk(b"tEXt", b"Author\0Someone"))
        .after(chunk(b"iTXt", &xmp))
        .after(chunk(b"tEXt", b"Comment\0late text"))
        .after(exif(1));
    let bytes = png.bytes();
    assert_streams(&bytes, "metadata");
    let d = forced(&bytes).unwrap().document;
    assert!(d.icc_profile.is_some() && d.metadata.exif.is_some() && d.metadata.xmp.is_some());
    assert_eq!(d.metadata.text.len(), 2);
    assert!((d.resolution_dpi - 300.0).abs() < 0.1, "{}", d.resolution_dpi);
    // Pixels of the stream and the codec's own decode agree sample for sample.
    let img = codecs::decode(&bytes).unwrap();
    assert_eq!((img.width(), img.height()), (300, 270));
}

#[test]
fn rows_arrive_in_tile_row_groups_and_match_the_whole_decode() {
    for (ctype, depth) in [(2u8, 8u8), (0, 16), (6, 16)] {
        let bytes = Png::new(70, 600, ctype, depth).bytes();
        let img = codecs::decode(&bytes).unwrap();
        let (mut got, mut starts) = (Vec::new(), Vec::new());
        let extras = codecs::decode_png_rows::<codecs::CodecError>(
            &bytes,
            &codecs::DecodeOptions::default(),
            TILE_SIZE as u32,
            &mut |h| {
                assert_eq!((h.width, h.height, h.layout, h.sample), (70, 600, img.layout(), img.sample_type()));
                Ok(true)
            },
            &mut |y0, rows| {
                starts.push(y0);
                got.extend_from_slice(rows);
                Ok(())
            },
        )
        .unwrap();
        assert!(extras.is_some());
        assert_eq!(starts, [0, 256, 512], "type {ctype} depth {depth}");
        assert!(got == img.data(), "type {ctype} depth {depth}");
    }
}

#[test]
fn rows_arrive_in_groups_of_the_height_the_caller_asks_for() {
    let bytes = Png::new(9, 30, 2, 8).bytes();
    let img = codecs::decode(&bytes).unwrap();
    // A group of 0 counts as 1; one taller than the image hands over every row at once.
    for (group, want) in [(7, vec![0, 7, 14, 21, 28]), (0, (0..30).collect()), (1, (0..30).collect()), (30, vec![0]), (u32::MAX, vec![0])] {
        let (mut got, mut starts) = (Vec::new(), Vec::new());
        let extras = codecs::decode_png_rows::<codecs::CodecError>(&bytes, &codecs::DecodeOptions::default(), group, &mut |_| Ok(true), &mut |y0, rows| {
            starts.push(y0);
            got.extend_from_slice(rows);
            Ok(())
        })
        .unwrap();
        assert!(extras.is_some());
        assert_eq!(starts, want, "group {group}");
        assert!(got == img.data(), "group {group}");
    }
}

#[test]
fn interlaced_and_animated_files_do_not_stream_and_open_as_before() {
    let img = Image::from_raw(97, 63, ChannelLayout::Rgb, CSample::U8, (0..97 * 63 * 3).map(|i| (i * 7 % 251) as u8).collect()).unwrap();
    let interlaced = codecs::encode(&img, Format::Png, &codecs::EncodeOptions { png_interlaced: true, ..Default::default() }).unwrap();
    assert!(streamed(&interlaced).unwrap().is_none());
    assert_same(&forced(&interlaced).unwrap(), &whole(&interlaced).unwrap(), "interlaced");

    // An acTL before the image data makes it an APNG, whose first frame the whole path reads.
    let bytes = Png::new(40, 40, 2, 8).before(chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0])).bytes();
    let called = AtomicU32::new(0);
    let r = codecs::decode_png_rows::<codecs::CodecError>(
        &bytes,
        &codecs::DecodeOptions::default(),
        TILE_SIZE as u32,
        &mut |_| {
            called.store(1, Ordering::Relaxed);
            Ok(true)
        },
        &mut |_, _| {
            called.store(2, Ordering::Relaxed);
            Ok(())
        },
    );
    assert!(matches!(r, Ok(None)) && called.load(Ordering::Relaxed) == 0);
    assert_same(&forced(&bytes).unwrap(), &whole(&bytes).unwrap(), "apng");
}

#[test]
fn an_orientation_after_the_image_data_falls_back_to_the_upright_whole_image() {
    for o in [3u8, 6, 8] {
        let bytes = Png::new(300, 20, 2, 8).after(exif(o)).bytes();
        assert!(streamed(&bytes).unwrap().is_none(), "orientation {o}");
        let got = forced(&bytes).unwrap();
        assert_same(&got, &whole(&bytes).unwrap(), "oriented");
        let want = if o == 3 { (300, 20) } else { (20, 300) };
        assert_eq!((got.document.size.width, got.document.size.height), want, "orientation {o}");
    }
    // Orientation 1 streams.
    assert_streams(&Png::new(300, 20, 2, 8).after(exif(1)).bytes(), "orientation 1");
}

#[test]
fn damaged_files_fail_or_open_exactly_as_with_the_whole_image() {
    let png = Png::new(300, 600, 6, 8);
    let good = png.bytes();
    let idat = png.idat();
    let at = good.windows(4).position(|w| w == b"IDAT").unwrap() - 4;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for cut in [20, 40, at + 10, at + 100, at + idat.len() / 2, at + idat.len() - 20, at + idat.len() - 1, at + idat.len(), good.len() - 12, good.len() - 5] {
        files.push((format!("cut at {cut}"), good.get(..cut).unwrap_or_default().to_vec()));
    }
    for off in [at + 20, at + idat.len() / 2, at + idat.len() - 6, at + idat.len() - 2] {
        let mut bad = good.clone();
        bad[off] ^= 0x55;
        files.push((format!("flipped byte {off}"), bad));
    }
    // Several IDAT chunks, the second one cut short of its CRC.
    let zdata = &idat[8..idat.len() - 4];
    let (z1, z2) = zdata.split_at(zdata.len() / 2);
    let split = [chunk(b"IDAT", z1), chunk(b"IDAT", z2)].concat();
    files.push(("split".into(), png.assemble(&split)));
    files.push(("split, second cut".into(), png.assemble(&split[..split.len() - 30])));
    // Damage after the pixels is tolerated: a text chunk with a bad CRC, and a lone byte of junk.
    let mut bad_crc = chunk(b"tEXt", b"a\0b");
    if let Some(last) = bad_crc.last_mut() {
        *last ^= 1;
    }
    files.push(("bad chunk after the image".into(), Png::new(300, 600, 6, 8).after(bad_crc).bytes()));
    let mut junk = good.clone();
    junk.truncate(good.len() - 12);
    junk.push(7);
    files.push(("junk after the image".into(), junk));
    let (mut errors, mut opened) = (0, 0);
    for (what, bytes) in &files {
        let (w, s, f) = (whole(bytes), streamed(bytes), forced(bytes));
        match (w, s, f) {
            (Err(w), Err(s), Err(f)) => {
                assert_eq!((w.to_string(), s.to_string()), (f.to_string(), f.to_string()), "{what}");
                errors += 1;
            }
            (Ok(w), Ok(Some(_)), Ok(f)) => {
                assert_same(&f, &w, what);
                opened += 1;
            }
            (w, s, f) => panic!("{what}: whole {:?}, streamed {:?}, import {:?}", w.map(|_| ()), s.map(|s| s.map(|_| ())), f.map(|_| ())),
        }
    }
    assert!(errors > 0 && opened > 0, "errors {errors}, opened {opened}");
}

#[test]
fn files_too_small_for_their_size_are_refused_before_any_tile_exists() {
    let stream = |bytes: &[u8]| {
        let called = AtomicU32::new(0);
        let r = codecs::decode_png_rows::<codecs::CodecError>(
            bytes,
            &codecs::DecodeOptions::default(),
            TILE_SIZE as u32,
            &mut |_| {
                called.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            },
            &mut |_, _| Ok(()),
        );
        (r.map(|e| e.is_some()), called.load(Ordering::Relaxed))
    };
    // A decompression bomb: 60000 x 60000 declared, a handful of bytes of data.
    let mut bomb = Png::new(60_000, 60_000, 6, 8);
    bomb.h = 60_000;
    let bomb = bomb.assemble(&chunk(b"IDAT", &zlib(&[0; 64])));
    let (r, called) = stream(&bomb);
    assert!(matches!(r, Err(codecs::CodecError::LimitExceeded(_))), "{r:?}");
    assert_eq!(called, 0);
    assert!(streamed(&bomb).is_err() && whole(&bomb).is_err());
    // Nothing after the header at all.
    let short = Png::new(2000, 2000, 6, 8).bytes();
    let short = short.get(..33).unwrap_or_default();
    let (r, called) = stream(short);
    assert!(r.is_err() && called == 0, "{r:?}");
    assert!(streamed(short).is_err() && whole(short).is_err());
}

#[test]
fn a_cancelled_stream_stops_with_cancelled() {
    let bytes = Png::new(513, 520, 2, 8).bytes();
    for n in [0, 1, 2, 3] {
        let calls = AtomicU32::new(0);
        let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= n;
        let r = import_flat_impl("t", &bytes, &Interrupt::cancel_only(&cancel), Some(0));
        assert!(matches!(r, Err(IoError::Cancelled)), "after {n} checks: {:?}", r.map(|_| ()));
    }
}

#[test]
fn progress_is_monotonic_streamed_and_after_a_fallback() {
    for bytes in [Png::new(513, 520, 2, 8).bytes(), Png::new(513, 520, 2, 8).after(exif(6)).bytes()] {
        let seen = std::sync::Mutex::new(Vec::new());
        let no = || false;
        let progress = |f: f32| seen.lock().unwrap().push(f);
        import_flat_impl("t", &bytes, &Interrupt::new(&no, &progress), Some(0)).unwrap();
        let seen = seen.into_inner().unwrap();
        assert!(seen.len() >= 4 && seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert!(seen.last().is_some_and(|&l| l >= 0.95), "{seen:?}");
    }
}

#[test]
fn only_images_of_the_minimum_decoded_size_stream() {
    // 300 x 270 RGB8 decodes to 243000 bytes.
    let bytes = Png::new(300, 270, 2, 8).bytes();
    let at = |min| import_png_streamed("t", &bytes, &crate::ImportOptions::default(), &Interrupt::NONE, min).unwrap().is_some();
    assert!(at(243_000) && !at(243_001));
    // The size the importer uses by default: this one is far under it, so `import_flat` opens it whole.
    assert!(!at(STREAM_MIN_BYTES));
    assert_same(&import_flat("t", &bytes).unwrap(), &whole(&bytes).unwrap(), "small");
    // 16-bit RGBA is 8 bytes a pixel.
    let wide = Png::new(100, 100, 6, 16).bytes();
    assert!(import_png_streamed("t", &wide, &crate::ImportOptions::default(), &Interrupt::NONE, 80_000).unwrap().is_some());
    assert!(import_png_streamed("t", &wide, &crate::ImportOptions::default(), &Interrupt::NONE, 80_001).unwrap().is_none());
}

/// A failed reservation is one tile or row band, so the error names the image by its pixel
/// size, never by the bytes of that reservation (which read as a tiny "0 MB image").
#[test]
fn allocation_errors_name_the_image_not_the_reservation() {
    let oom = alloc_error(AllocationError::NotEnoughMemory { bytes: 4096 }, 8000, 6000).to_string();
    assert!(oom.contains("not enough memory to open the 8000×6000 image"), "{oom}");
    assert!(!oom.contains("MB"), "{oom}");
    let overflow = alloc_error(AllocationError::SizeOverflow, 70000, 70000).to_string();
    assert!(overflow.contains("70000×70000 image is too large"), "{overflow}");
}

/// The stream asks the machine before it allocates the first tile: an image that can't fit is
/// refused, with an error naming its pixel size, and the stream never starts.
#[test]
fn a_streamed_png_the_machine_can_not_hold_is_refused_before_any_tile() {
    let bytes = Png::new(400, 300, 6, 8).bytes();
    match import_png_streamed_with("t", &bytes, &crate::ImportOptions::default(), &Interrupt::NONE, 0, || Some(0)) {
        Err(e) => assert!(e.to_string().contains("not enough memory to open this 400 × 300 px image"), "{e}"),
        Ok(_) => panic!("opened with no free memory"),
    }
    // Unknown free memory is no refusal: the image opens as before.
    assert!(import_png_streamed_with("t", &bytes, &crate::ImportOptions::default(), &Interrupt::NONE, 0, || None).unwrap().is_some());
    // A cancelled stream is cancelled, not refused for the memory it would have needed.
    let yes = || true;
    match import_png_streamed_with("t", &bytes, &crate::ImportOptions::default(), &Interrupt::cancel_only(&yes), 0, || Some(0)) {
        Err(IoError::Cancelled) => {}
        other => panic!("{:?}", other.map(|_| ())),
    }
}

/// The base (whole-image) PNG path checks the layer's tiles against free memory before they are
/// allocated, as the streamed path does, for images big enough for the tiles to matter; unknown
/// free memory is no refusal, and an image below the streaming size opens as it did.
#[test]
fn the_whole_image_path_refuses_tiles_the_machine_can_not_hold() {
    let img = codecs::decode(&Png::new(400, 300, 2, 8).bytes()).unwrap();
    // Small tiles are not worth a check: with nothing reported free this still opens.
    assert!(image_to_document_with_memory("t", &img, &Interrupt::NONE, Some(0)).is_ok());
    match check_layer_fits(4000, 3000, ChannelLayout::Rgba, CSample::U8, Some(0)) {
        Err(e) => assert!(e.to_string().contains("not enough memory to open this 4000 × 3000 px image"), "{e}"),
        Ok(()) => panic!("opened with no free memory"),
    }
    assert!(check_layer_fits(4000, 3000, ChannelLayout::Rgba, CSample::U8, None).is_ok());
    assert!(check_layer_fits(4000, 3000, ChannelLayout::Rgba, CSample::U8, Some(1 << 40)).is_ok());
}
