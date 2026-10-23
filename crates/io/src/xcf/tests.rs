//! Synthetic XCF files (written here, from the specification) for the reader: structure,
//! every compression, precisions, groups, masks, channels, limits and malformed input.

use super::*;
use photocraft_raster::Interrupt;

/// A small XCF writer for tests: version 11+ layout (64-bit pointers) by default.
pub(super) struct W {
    pub out: Vec<u8>,
    pub wide: bool,
}

impl W {
    pub fn new(version: u32) -> Self {
        let mut w = W { out: Vec::new(), wide: version >= 11 };
        w.out.extend_from_slice(b"gimp xcf ");
        if version == 0 {
            w.out.extend_from_slice(b"file\0");
        } else {
            w.out.extend_from_slice(format!("v{version:03}\0").as_bytes());
        }
        w
    }
    pub fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    pub fn ptr(&mut self, v: usize) {
        if self.wide {
            self.out.extend_from_slice(&(v as u64).to_be_bytes());
        } else {
            self.u32(v as u32);
        }
    }
    pub fn string(&mut self, s: &str) {
        if s.is_empty() {
            self.u32(0);
        } else {
            self.u32(s.len() as u32 + 1);
            self.out.extend_from_slice(s.as_bytes());
            self.out.push(0);
        }
    }
    pub fn prop(&mut self, kind: u32, payload: &[u8]) {
        self.u32(kind);
        self.u32(payload.len() as u32);
        self.out.extend_from_slice(payload);
    }
    pub fn prop_u32(&mut self, kind: u32, v: u32) {
        self.prop(kind, &v.to_be_bytes());
    }
    pub fn end(&mut self) {
        self.u32(0);
        self.u32(0);
    }
    /// Reserves a pointer slot, returning where to patch it.
    pub fn slot(&mut self) -> usize {
        let at = self.out.len();
        self.ptr(0);
        at
    }
    pub fn patch(&mut self, slot: usize, target: usize) {
        if self.wide {
            self.out[slot..slot + 8].copy_from_slice(&(target as u64).to_be_bytes());
        } else {
            self.out[slot..slot + 4].copy_from_slice(&(target as u32).to_be_bytes());
        }
    }
}

/// RLE-encodes one tile of `bpp`-byte pixels, plane by plane, with runs and literals.
pub(super) fn rle(pixels: &[u8], bpp: usize) -> Vec<u8> {
    let n = pixels.len() / bpp;
    let mut out = Vec::new();
    for b in 0..bpp {
        let plane: Vec<u8> = (0..n).map(|i| pixels[i * bpp + b]).collect();
        let mut i = 0;
        while i < n {
            let v = plane[i];
            let mut run = 1;
            while i + run < n && plane[i + run] == v && run < 65535 {
                run += 1;
            }
            if run >= 2 {
                if run <= 127 {
                    out.push((run - 1) as u8);
                } else {
                    out.push(127);
                    out.push((run / 256) as u8);
                    out.push((run % 256) as u8);
                }
                out.push(v);
                i += run;
            } else {
                let start = i;
                while i < n && (i + 1 >= n || plane[i + 1] != plane[i]) && i - start < 127 {
                    i += 1;
                }
                let len = i - start;
                out.push((256 - len) as u8);
                out.extend_from_slice(&plane[start..i]);
            }
        }
    }
    out
}

/// Writes a hierarchy for `w`×`h` pixels of `bpp` bytes (`data` row-major), returning its offset.
pub(super) fn hierarchy(w: &mut W, width: u32, height: u32, bpp: usize, data: &[u8], compression: Compression) -> usize {
    let at = w.out.len();
    w.u32(width);
    w.u32(height);
    w.u32(bpp as u32);
    let level_slot = w.slot();
    w.ptr(0);
    let level = w.out.len();
    w.patch(level_slot, level);
    w.u32(width);
    w.u32(height);
    let across = width.div_ceil(TILE) as usize;
    let down = height.div_ceil(TILE) as usize;
    let slots: Vec<usize> = (0..across * down).map(|_| w.slot()).collect();
    w.ptr(0);
    for (i, slot) in slots.into_iter().enumerate() {
        let (tx, ty) = ((i % across) as u32, (i / across) as u32);
        let (tw, th) = ((width - tx * TILE).min(TILE), (height - ty * TILE).min(TILE));
        let mut tile = Vec::new();
        for py in 0..th {
            for px in 0..tw {
                let src = ((ty * TILE + py) as usize * width as usize + (tx * TILE + px) as usize) * bpp;
                tile.extend_from_slice(&data[src..src + bpp]);
            }
        }
        let encoded = match compression {
            Compression::None => tile,
            Compression::Rle => rle(&tile, bpp),
            Compression::Zlib => {
                use std::io::Write;
                let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&tile).unwrap();
                e.finish().unwrap()
            }
        };
        let here = w.out.len();
        w.patch(slot, here);
        w.out.extend_from_slice(&encoded);
    }
    at
}

/// A layer to write.
pub(super) struct TestLayer {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    pub kind: u32,
    pub x: i32,
    pub y: i32,
    pub opacity: f32,
    pub visible: bool,
    pub mode: u32,
    pub props: Vec<(u32, Vec<u8>)>,
    pub data: Vec<u8>,
    pub mask: Option<Vec<u8>>,
    pub apply_mask: bool,
}

impl TestLayer {
    pub fn rgba(name: &'static str, width: u32, height: u32, data: Vec<u8>) -> Self {
        TestLayer { name, width, height, kind: 1, x: 0, y: 0, opacity: 1.0, visible: true, mode: 28, props: Vec::new(), data, mask: None, apply_mask: true }
    }
}

pub(super) struct TestImage {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub base_type: u32,
    pub precision: u32,
    pub compression: Compression,
    pub image_props: Vec<(u32, Vec<u8>)>,
    pub layers: Vec<TestLayer>,
    pub channels: Vec<(&'static str, Vec<u8>, bool)>,
}

impl TestImage {
    pub fn rgb8(width: u32, height: u32) -> Self {
        TestImage {
            version: 11,
            width,
            height,
            base_type: 0,
            precision: 150,
            compression: Compression::Rle,
            image_props: Vec::new(),
            layers: Vec::new(),
            channels: Vec::new(),
        }
    }

    pub fn write(&self) -> Vec<u8> {
        let mut w = W::new(self.version);
        w.u32(self.width);
        w.u32(self.height);
        w.u32(self.base_type);
        if self.version >= 4 {
            w.u32(self.precision);
        }
        let comp = match self.compression {
            Compression::None => 0u8,
            Compression::Rle => 1,
            Compression::Zlib => 2,
        };
        w.prop(17, &[comp]);
        for (k, v) in &self.image_props {
            w.prop(*k, v);
        }
        w.end();
        let layer_slots: Vec<usize> = self.layers.iter().map(|_| w.slot()).collect();
        w.ptr(0);
        let channel_slots: Vec<usize> = self.channels.iter().map(|_| w.slot()).collect();
        w.ptr(0);
        if self.version >= 18 {
            w.ptr(0);
        }
        let bytes = match self.precision {
            100 | 150 => 1,
            200 | 250 | 500 | 550 => 2,
            300 | 350 | 600 | 650 => 4,
            _ => 8,
        };
        for (layer, slot) in self.layers.iter().zip(layer_slots) {
            let at = w.out.len();
            w.patch(slot, at);
            w.u32(layer.width);
            w.u32(layer.height);
            w.u32(layer.kind);
            w.string(layer.name);
            w.prop_u32(6, (layer.opacity * 255.0).round() as u32);
            w.prop(33, &layer.opacity.to_be_bytes());
            w.prop_u32(8, u32::from(layer.visible));
            w.prop_u32(7, layer.mode);
            let mut off = Vec::new();
            off.extend_from_slice(&layer.x.to_be_bytes());
            off.extend_from_slice(&layer.y.to_be_bytes());
            w.prop(15, &off);
            if layer.mask.is_some() {
                w.prop_u32(11, u32::from(layer.apply_mask));
            }
            for (k, v) in &layer.props {
                w.prop(*k, v);
            }
            w.end();
            let hslot = w.slot();
            let mslot = w.slot();
            if self.version >= 20 {
                w.ptr(0);
            }
            let channels = match layer.kind {
                0 => 3,
                1 => 4,
                2 | 4 => 1,
                _ => 2,
            };
            if !layer.data.is_empty() {
                let h = hierarchy(&mut w, layer.width, layer.height, channels * bytes, &layer.data, self.compression);
                w.patch(hslot, h);
            }
            if let Some(mask) = &layer.mask {
                let at = w.out.len();
                w.patch(mslot, at);
                w.u32(layer.width);
                w.u32(layer.height);
                w.string("mask");
                w.end();
                let hs = w.slot();
                let h = hierarchy(&mut w, layer.width, layer.height, bytes, mask, self.compression);
                w.patch(hs, h);
            }
        }
        for ((name, data, selection), slot) in self.channels.iter().zip(channel_slots) {
            let at = w.out.len();
            w.patch(slot, at);
            w.u32(self.width);
            w.u32(self.height);
            w.string(name);
            if *selection {
                w.prop(4, &[]);
            }
            w.end();
            let hs = w.slot();
            let h = hierarchy(&mut w, self.width, self.height, bytes, data, self.compression);
            w.patch(hs, h);
        }
        w.out
    }
}

fn open(bytes: &[u8]) -> Result<ImportResult> {
    import("test.xcf", bytes, &Interrupt::NONE)
}

fn rgba(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let f = &f;
    (0..h).flat_map(|y| (0..w).flat_map(move |x| f(x, y))).collect()
}

fn pixel(layer: &Layer, x: i32, y: i32) -> [f32; 4] {
    let LayerContent::Raster(s) = &layer.content else { panic!("not raster") };
    let px = s.read_region(Rect::new(x, y, x + 1, y + 1));
    [px[0], px[1], px[2], px[3]]
}

#[test]
fn a_layer_decodes_with_every_compression() {
    for compression in [Compression::None, Compression::Rle, Compression::Zlib] {
        let mut img = TestImage::rgb8(70, 66);
        img.compression = compression;
        img.layers.push(TestLayer::rgba("A", 70, 66, rgba(70, 66, |x, y| [x as u8 * 3, y as u8 * 2, (x + y) as u8, 255 - x as u8])));
        let r = open(&img.write()).unwrap();
        let d = &r.document;
        assert_eq!((d.size.width, d.size.height), (70, 66));
        assert_eq!(d.layers.len(), 1, "{compression:?}");
        for (x, y) in [(0, 0), (63, 0), (64, 0), (69, 65), (10, 64)] {
            let p = pixel(&d.layers[0], x, y);
            let want = [x as f32 * 3.0, y as f32 * 2.0, (x + y) as f32, 255.0 - x as f32].map(|v| v / 255.0);
            assert!((0..4).all(|c| (p[c] - want[c]).abs() < 1e-6), "{compression:?} at {x},{y}: {p:?} vs {want:?}");
        }
        assert!(r.source_read_only);
    }
}

#[test]
fn every_truncation_errors_never_panics() {
    let mut img = TestImage::rgb8(20, 20);
    img.layers.push(TestLayer::rgba("A", 20, 20, rgba(20, 20, |x, y| [x as u8, y as u8, 0, 255])));
    let bytes = img.write();
    for cut in 0..bytes.len() {
        let _ = open(&bytes[..cut]);
    }
    assert!(open(&bytes).is_ok());
}

fn layer_at(layers: &[Layer], name: &str) -> Layer {
    layers.iter().find(|l| l.name == name).cloned().unwrap_or_else(|| panic!("no layer {name}"))
}

#[test]
fn properties_offsets_opacity_mode_visibility_locks_and_tags() {
    let mut img = TestImage::rgb8(30, 20);
    let mut l = TestLayer::rgba("Top", 10, 8, rgba(10, 8, |x, y| [x as u8 * 20, y as u8 * 30, 0, 200]));
    l.x = -3;
    l.y = 15;
    l.opacity = 0.4;
    l.visible = false;
    l.mode = 30; // Multiply
    l.props = vec![(10, 1u32.to_be_bytes().to_vec()), (28, 1u32.to_be_bytes().to_vec()), (32, 1u32.to_be_bytes().to_vec()), (34, 2u32.to_be_bytes().to_vec())];
    img.layers.push(l);
    img.layers.push(TestLayer::rgba("Bottom", 30, 20, rgba(30, 20, |_, _| [9, 9, 9, 255])));
    let r = open(&img.write()).unwrap();
    let d = &r.document;
    assert_eq!(d.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Bottom", "Top"], "topmost in the file is last in the stack");
    let top = &d.layers[1];
    assert_eq!((top.blend, top.opacity, top.visible), (BlendMode::Multiply, 0.4, false));
    assert!(top.locks.transparency && top.locks.pixels && top.locks.position);
    assert_eq!(top.label, LabelColor::Green);
    // Offset (-3, 15): the layer's pixel (5, 2) sits at document (2, 17).
    let p = pixel(top, 2, 17);
    assert!((p[0] - 100.0 / 255.0).abs() < 1e-6 && (p[1] - 60.0 / 255.0).abs() < 1e-6 && (p[3] - 200.0 / 255.0).abs() < 1e-6, "{p:?}");
    assert_eq!(pixel(top, 2, 14)[3], 0.0, "above the layer");
}

#[test]
fn groups_follow_item_paths_and_pass_through() {
    // File order: Top, Group (0), child A [0,0], Nested group [0,1], nested child [0,1,0], Bottom.
    let mut img = TestImage::rgb8(16, 16);
    let px = |v: u8| rgba(4, 4, move |_, _| [v, v, v, 255]);
    img.layers.push(TestLayer::rgba("Top", 4, 4, px(1)));
    let mut group = TestLayer::rgba("Group", 1, 1, Vec::new());
    group.props = vec![(29, vec![]), (31, 0u32.to_be_bytes().to_vec())];
    group.mode = 28;
    img.layers.push(group);
    let mut a = TestLayer::rgba("A", 4, 4, px(2));
    a.props = vec![(30, [1u32, 0].iter().flat_map(|v| v.to_be_bytes()).collect())];
    img.layers.push(a);
    let mut nested = TestLayer::rgba("Nested", 1, 1, Vec::new());
    nested.props = vec![(29, vec![]), (30, [1u32, 1].iter().flat_map(|v| v.to_be_bytes()).collect())];
    nested.mode = 61; // Pass through
    img.layers.push(nested);
    let mut b = TestLayer::rgba("B", 4, 4, px(3));
    b.props = vec![(30, [1u32, 1, 0].iter().flat_map(|v| v.to_be_bytes()).collect())];
    img.layers.push(b);
    img.layers.push(TestLayer::rgba("Bottom", 16, 16, rgba(16, 16, |_, _| [0, 0, 0, 255])));
    let d = open(&img.write()).unwrap().document;
    assert_eq!(d.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Bottom", "Group", "Top"]);
    let g = &d.layers[1];
    assert!(g.is_group() && g.blend == BlendMode::Normal);
    if let LayerContent::Group(gr) = &g.content {
        assert!(!gr.expanded);
    }
    let children = g.children().unwrap();
    assert_eq!(children.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Nested", "A"], "bottom first inside the group too");
    assert_eq!(children[0].blend, BlendMode::PassThrough);
    assert_eq!(children[0].children().unwrap()[0].name, "B");
    assert_eq!(pixel(&layer_at(children, "A"), 0, 0)[0], 2.0 / 255.0);
}

#[test]
fn a_path_into_nothing_lands_at_the_top_level_with_a_note() {
    let mut img = TestImage::rgb8(8, 8);
    let mut stray = TestLayer::rgba("Stray", 2, 2, rgba(2, 2, |_, _| [5, 5, 5, 255]));
    stray.props = vec![(30, [7u32, 0].iter().flat_map(|v| v.to_be_bytes()).collect())];
    img.layers.push(stray);
    img.layers.push(TestLayer::rgba("Bottom", 8, 8, rgba(8, 8, |_, _| [0, 0, 0, 255])));
    let r = open(&img.write()).unwrap();
    assert_eq!(r.document.layers.len(), 2);
    assert!(r.warnings.iter().any(|w| w.contains("item path")), "{:?}", r.warnings);
}

#[test]
fn masks_are_read_at_the_layer_offset_applied_or_not() {
    for apply in [true, false] {
        let mut img = TestImage::rgb8(20, 20);
        let mut l = TestLayer::rgba("Masked", 6, 6, rgba(6, 6, |_, _| [255, 0, 0, 255]));
        l.x = 4;
        l.y = 5;
        l.mask = Some((0..36).map(|i| (i * 7) as u8).collect());
        l.apply_mask = apply;
        img.layers.push(l);
        let d = open(&img.write()).unwrap().document;
        let mask = d.layers[0].mask.as_ref().expect("mask");
        assert_eq!(mask.enabled, apply);
        assert_eq!(mask.surface.read_region(Rect::new(4 + 3, 5 + 2, 4 + 4, 5 + 3))[0], (2 * 6 + 3) as f32 * 7.0 / 255.0);
    }
}

#[test]
fn sixteen_bit_and_float_samples_are_big_endian() {
    // 16-bit gamma integer (250): one 2x1 gray+alpha layer.
    let mut img = TestImage::rgb8(2, 1);
    img.base_type = 1;
    img.precision = 250;
    let mut data = Vec::new();
    for v in [0x1234u16, 0xffff, 0x8000, 0x0001] {
        data.extend_from_slice(&v.to_be_bytes());
    }
    let mut l = TestLayer::rgba("G", 2, 1, data);
    l.kind = 3; // gray with alpha
    img.layers.push(l);
    let d = open(&img.write()).unwrap().document;
    assert_eq!((d.mode, d.depth), (ColorMode::Grayscale, SampleType::U16));
    let p = pixel_n(&d.layers[0], 0, 0, 2);
    assert!((p[0] - 0x1234 as f32 / 65535.0).abs() < 1e-6 && (p[1] - 1.0).abs() < 1e-6, "{p:?}");
    let p = pixel_n(&d.layers[0], 1, 0, 2);
    assert!((p[0] - 0x8000 as f32 / 65535.0).abs() < 1e-6 && (p[1] - 1.0 / 65535.0).abs() < 1e-6, "{p:?}");

    // 32-bit float linear (600): RGB without alpha, values above 1 kept, linear profile.
    let mut img = TestImage::rgb8(1, 1);
    img.precision = 600;
    let mut data = Vec::new();
    for v in [0.25f32, 1.5, -0.5] {
        data.extend_from_slice(&v.to_be_bytes());
    }
    let mut l = TestLayer::rgba("F", 1, 1, data);
    l.kind = 0;
    img.layers.push(l);
    let r = open(&img.write()).unwrap();
    let d = &r.document;
    assert_eq!(d.depth, SampleType::F32);
    assert_eq!(pixel(&d.layers[0], 0, 0), [0.25, 1.5, -0.5, 1.0]);
    assert!(d.icc_profile.is_some(), "linear light gets a linear profile");
    assert!(d.layers[0].locks.transparency, "no alpha channel in the file: the layer is opaque");

    // 16-bit float (500) and 64-bit float (700) decode too.
    for (precision, bytes) in [(500u32, f16::from_f32(0.75).to_bits().to_be_bytes().to_vec()), (700, 0.75f64.to_be_bytes().to_vec())] {
        let mut img = TestImage::rgb8(1, 1);
        img.base_type = 1;
        img.precision = precision;
        let mut l = TestLayer::rgba("F", 1, 1, bytes);
        l.kind = 2;
        img.layers.push(l);
        let d = open(&img.write()).unwrap().document;
        assert_eq!(pixel_n(&d.layers[0], 0, 0, 2)[0], 0.75, "precision {precision}");
    }
}

fn pixel_n(layer: &Layer, x: i32, y: i32, n: usize) -> Vec<f32> {
    let LayerContent::Raster(s) = &layer.content else { panic!("not raster") };
    let px = s.read_region(Rect::new(x, y, x + 1, y + 1));
    assert_eq!(px.len(), n);
    px
}

#[test]
fn indexed_images_use_the_colormap() {
    let mut img = TestImage::rgb8(2, 1);
    img.base_type = 2;
    let mut cmap = 3u32.to_be_bytes().to_vec();
    cmap.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255]);
    img.image_props.push((1, cmap));
    let mut l = TestLayer::rgba("I", 2, 1, vec![2, 255, 1, 128]);
    l.kind = 5; // indexed with alpha
    img.layers.push(l);
    let r = open(&img.write()).unwrap();
    assert_eq!(r.document.mode, ColorMode::Rgb);
    assert_eq!(pixel(&r.document.layers[0], 0, 0), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(pixel(&r.document.layers[0], 1, 0), [0.0, 1.0, 0.0, 128.0 / 255.0]);
    assert!(r.warnings.iter().any(|w| w.contains("indexed")));
}

#[test]
fn channels_become_alpha_channels_and_the_selection() {
    let mut img = TestImage::rgb8(4, 4);
    img.layers.push(TestLayer::rgba("L", 4, 4, rgba(4, 4, |_, _| [0, 0, 0, 255])));
    img.channels.push(("Extra", (0..16).map(|i| i as u8 * 16).collect(), false));
    img.channels.push(("Selection Mask", vec![255; 16], true));
    let d = open(&img.write()).unwrap().document;
    assert_eq!(d.channels.len(), 1);
    assert_eq!(d.channels[0].name, "Extra");
    assert_eq!(d.channels[0].surface.read_region(Rect::new(3, 1, 4, 2))[0], 7.0 * 16.0 / 255.0);
    assert_eq!(d.selection.as_ref().unwrap().read_region(Rect::new(0, 0, 1, 1))[0], 1.0);
}

#[test]
fn image_parasites_resolution_guides_and_comment() {
    let mut img = TestImage::rgb8(10, 10);
    let mut res = Vec::new();
    res.extend_from_slice(&300.0f32.to_be_bytes());
    res.extend_from_slice(&300.0f32.to_be_bytes());
    img.image_props.push((19, res));
    let mut guides = Vec::new();
    guides.extend_from_slice(&4i32.to_be_bytes());
    guides.push(1);
    guides.extend_from_slice(&7i32.to_be_bytes());
    guides.push(2);
    img.image_props.push((18, guides));
    let mut parasites = Vec::new();
    for (name, data) in [("gimp-comment", &b"hello\0"[..]), ("icc-profile", &[1u8, 2, 3][..])] {
        parasites.extend_from_slice(&(name.len() as u32 + 1).to_be_bytes());
        parasites.extend_from_slice(name.as_bytes());
        parasites.push(0);
        parasites.extend_from_slice(&0u32.to_be_bytes());
        parasites.extend_from_slice(&(data.len() as u32).to_be_bytes());
        parasites.extend_from_slice(data);
    }
    img.image_props.push((21, parasites));
    // An unknown property is skipped by its length.
    img.image_props.push((99, vec![1, 2, 3, 4, 5]));
    img.layers.push(TestLayer::rgba("L", 10, 10, rgba(10, 10, |_, _| [0, 0, 0, 255])));
    let d = open(&img.write()).unwrap().document;
    assert_eq!(d.resolution_dpi, 300.0);
    assert_eq!((d.guides.horizontal.as_slice(), d.guides.vertical.as_slice()), (&[4.0][..], &[7.0][..]));
    assert_eq!(d.metadata.text, vec![("Comment".to_string(), "hello".to_string())]);
    assert_eq!(d.icc_profile.as_deref().map(|p| p.as_slice()), Some(&[1u8, 2, 3][..]));
}

#[test]
fn clipping_text_vector_effects_and_floating_selections_are_reported() {
    let mut img = TestImage::rgb8(4, 4);
    img.version = 20;
    let mut clipped = TestLayer::rgba("Clipped", 4, 4, rgba(4, 4, |_, _| [1, 1, 1, 255]));
    let mut parasite = Vec::new();
    parasite.extend_from_slice(&("gimphoto-clipped".len() as u32 + 1).to_be_bytes());
    parasite.extend_from_slice(b"gimphoto-clipped\0");
    parasite.extend_from_slice(&0u32.to_be_bytes());
    parasite.extend_from_slice(&1u32.to_be_bytes());
    parasite.push(1);
    clipped.props = vec![(21, parasite), (26, 1u32.to_be_bytes().to_vec()), (47, vec![0; 8])];
    img.layers.push(clipped);
    let mut floating = TestLayer::rgba("Floating Selection", 2, 2, rgba(2, 2, |_, _| [2, 2, 2, 255]));
    floating.props = vec![(5, vec![0; 8])];
    img.layers.push(floating);
    img.layers.push(TestLayer::rgba("Bottom", 4, 4, rgba(4, 4, |_, _| [0, 0, 0, 255])));
    let r = open(&img.write()).unwrap();
    assert_eq!(r.document.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Bottom", "Clipped"], "the floating selection is left out");
    assert!(r.document.layers[1].clipped);
    let w = r.warnings.join("\n");
    assert!(w.contains("text layer") && w.contains("vector layer") && w.contains("floating selection"), "{w}");
}

#[test]
fn version_3_files_use_32_bit_pointers_and_legacy_modes() {
    let mut img = TestImage::rgb8(5, 5);
    img.version = 3;
    let mut l = TestLayer::rgba("Old", 5, 5, rgba(5, 5, |x, _| [x as u8, 0, 0, 255]));
    l.mode = 3; // Multiply (legacy)
    img.layers.push(l);
    let bytes = img.write();
    assert!(bytes.starts_with(b"gimp xcf v003\0"));
    let d = open(&bytes).unwrap().document;
    assert_eq!((d.depth, d.layers[0].blend), (SampleType::U8, BlendMode::Multiply));
    assert_eq!(pixel(&d.layers[0], 4, 0)[0], 4.0 / 255.0);
    // Version 0 ("file") as well.
    let mut img0 = TestImage::rgb8(2, 2);
    img0.version = 0;
    img0.layers.push(TestLayer::rgba("Oldest", 2, 2, rgba(2, 2, |_, _| [7, 7, 7, 255])));
    assert!(open(&img0.write()).unwrap().document.layers.len() == 1);
}

#[test]
fn rle_long_runs_and_literals_round_trip() {
    // A tile with runs over 127 and long literal stretches, in every byte plane.
    let w = 64;
    let data = rgba(w, 64, |x, y| if y < 40 { [200, 1, 2, 255] } else { [(x * 7 + y) as u8, (x * 3) as u8, y as u8, 255] });
    let mut img = TestImage::rgb8(w, 64);
    img.layers.push(TestLayer::rgba("R", w, 64, data.clone()));
    let d = open(&img.write()).unwrap().document;
    let got = pixel(&d.layers[0], 63, 63);
    assert!((got[0] - (63 * 7 + 63) as u8 as f32 / 255.0).abs() < 1e-6 && (got[1] - 189.0 / 255.0).abs() < 1e-6, "{got:?}");
    assert_eq!(pixel(&d.layers[0], 10, 10), [200.0 / 255.0, 1.0 / 255.0, 2.0 / 255.0, 1.0]);
}

#[test]
fn limits_and_malformed_structures_are_errors() {
    // A canvas past the side limit.
    let mut big = TestImage::rgb8(1, 1);
    big.width = MAX_SIDE + 1;
    assert!(open(&big.write()).is_err());
    // A layer whose declared size has no pixels behind it (hierarchy pointer to garbage).
    let mut img = TestImage::rgb8(4, 4);
    img.layers.push(TestLayer::rgba("L", 4, 4, rgba(4, 4, |_, _| [0; 4])));
    let mut bytes = img.write();
    let n = bytes.len();
    bytes[n - 10..].fill(0xff);
    assert!(open(&bytes).is_err());
    // Wrong signature, unknown version tag, unknown compression.
    assert!(matches!(open(b"gimp xcf vXYZ\0"), Err(IoError::Xcf(_))));
    assert!(open(b"not an xcf").is_err());
    let mut img = TestImage::rgb8(2, 2);
    img.layers.push(TestLayer::rgba("L", 2, 2, rgba(2, 2, |_, _| [0; 4])));
    let mut bytes = img.write();
    let at = 14 + 12 + 4 + 8;
    bytes[at] = 3; // compression byte: reserved
    assert!(open(&bytes).is_err());
}

#[test]
fn gzipped_files_are_recognised_by_name_and_inflated() {
    use std::io::Write;
    let mut img = TestImage::rgb8(3, 3);
    img.layers.push(TestLayer::rgba("L", 3, 3, rgba(3, 3, |_, _| [4, 5, 6, 255])));
    let plain = img.write();
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&plain).unwrap();
    let gz = e.finish().unwrap();
    assert!(is_gzipped_xcf("a/b/photo.XCF.GZ", &gz) && is_gzipped_xcf("x.xcfgz", &gz));
    assert!(!is_gzipped_xcf("photo.xcf", &gz) && !is_gzipped_xcf("photo.xcf.gz", &plain));
    let d = crate::import("photo.xcf.gz", &gz).unwrap().document;
    assert_eq!(pixel(&d.layers[0], 1, 1), [4.0 / 255.0, 5.0 / 255.0, 6.0 / 255.0, 1.0]);
    assert!(gunzip(&gz[..gz.len() / 2]).is_err() || crate::import("photo.xcf.gz", &gz[..gz.len() / 2]).is_err());
}

/// A recognized raster signature wins over a misleading `.xcf` suffix, as with `.kra` and `.svg`.
#[test]
fn a_png_with_an_xcf_suffix_opens_by_content() {
    let img =
        photocraft_codecs::Image::from_raw(12, 8, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, vec![90; 12 * 8 * 4]).unwrap();
    let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let r = crate::import("renamed.xcf", &png).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (12, 8));
    assert!(crate::import("broken.xcf", b"not an image at all").is_err());
}

/// `PROP_FLOAT_OPACITY` wins over `PROP_OPACITY` whichever comes first in the file.
#[test]
fn float_opacity_wins_over_opacity_in_either_order() {
    let mut img = TestImage::rgb8(4, 4);
    // The writer emits OPACITY (0.4 → 102) then FLOAT_OPACITY 0.4; an OPACITY 255 after them must not win.
    let mut byte_last = TestLayer::rgba("Float then byte", 4, 4, rgba(4, 4, |_, _| [1, 2, 3, 255]));
    byte_last.opacity = 0.4;
    byte_last.props = vec![(6, 255u32.to_be_bytes().to_vec())];
    // A later FLOAT_OPACITY replaces the earlier one.
    let mut float_last = TestLayer::rgba("Byte then float", 4, 4, rgba(4, 4, |_, _| [1, 2, 3, 255]));
    float_last.opacity = 0.4;
    float_last.props = vec![(33, 0.25f32.to_be_bytes().to_vec())];
    img.layers.push(byte_last);
    img.layers.push(float_last);
    let d = open(&img.write()).unwrap().document;
    assert_eq!(layer_at(&d.layers, "Float then byte").opacity, 0.4);
    assert_eq!(layer_at(&d.layers, "Byte then float").opacity, 0.25);
}

/// A path that fails below the top level (here at depth 2) still lands at the top level, so the
/// group it would have been pushed into keeps its indices for the siblings placed after it.
#[test]
fn a_path_failing_below_the_top_level_lands_at_the_top_level() {
    let mut img = TestImage::rgb8(8, 8);
    let px = |v: u8| rgba(2, 2, move |_, _| [v, v, v, 255]);
    let mut group = TestLayer::rgba("Group", 1, 1, Vec::new());
    group.props = vec![(29, vec![]), (31, 0u32.to_be_bytes().to_vec())];
    img.layers.push(group);
    let mut stray = TestLayer::rgba("Stray", 2, 2, px(5));
    stray.props = vec![(30, [0u32, 4, 0].iter().flat_map(|v| v.to_be_bytes()).collect())];
    img.layers.push(stray);
    let mut a = TestLayer::rgba("A", 2, 2, px(6));
    a.props = vec![(30, [0u32, 0].iter().flat_map(|v| v.to_be_bytes()).collect())];
    img.layers.push(a);
    img.layers.push(TestLayer::rgba("Bottom", 8, 8, rgba(8, 8, |_, _| [0, 0, 0, 255])));
    let r = open(&img.write()).unwrap();
    let d = &r.document;
    assert_eq!(d.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Bottom", "Stray", "Group"]);
    let children = layer_at(&d.layers, "Group").children().unwrap().iter().map(|l| l.name.clone()).collect::<Vec<_>>();
    assert_eq!(children, ["A"], "the valid sibling placed after the stray one still lands in the group");
    assert!(r.warnings.iter().any(|w| w.contains("Stray") && w.contains("item path")), "{:?}", r.warnings);
}

/// A parasite name field: u32 `len+1`, then the bytes and a trailing NUL.
fn parasite_name(out: &mut Vec<u8>, name: &str) {
    out.extend_from_slice(&((name.len() as u32) + 1).to_be_bytes());
    out.extend_from_slice(name.as_bytes());
    out.push(0);
}

/// One PROP_PARASITES record: name, u32 flags, u32 len, then the payload.
fn parasite_record(out: &mut Vec<u8>, name: &str, payload: &[u8]) {
    parasite_name(out, name);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
}

#[test]
fn duplicate_parasites_keep_order_and_last_value() {
    // Duplicate names must resolve to the last payload while keeping first-seen order.
    // This is the behavior the name->index map must preserve versus the old linear scan.
    let mut data = Vec::new();
    parasite_record(&mut data, "gimp-comment", b"first");
    parasite_record(&mut data, "gimp-image-grid", b"grid");
    parasite_record(&mut data, "gimp-comment", b"second");
    let out = parasites(&data);
    assert_eq!(out.len(), 2, "duplicate collapses to one slot");
    assert_eq!(out[0].0, "gimp-comment");
    assert_eq!(out[0].1, b"second", "last value wins");
    assert_eq!(out[1].0, "gimp-image-grid", "insertion order preserved");
}
