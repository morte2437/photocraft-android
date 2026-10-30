use super::batch::write_tile;
use super::textures::{Resident, TexKind};
use super::{Arc, F, HashMap, I, PixelFormat, Rect, SampleType, Surface, TILE_SIZE, Tile, TileCoord, U, fx};

/// Distinct tiles converted per parallel batch: bounds the converted bytes held at once (64 tiles
/// of a 16-bit page are 32 MiB).
pub(crate) const CONVERT_BATCH: usize = 64;

/// Write the same tile `bytes` at every coordinate in `cs` of resident `r`; returns the bytes
/// staged. Several coordinates (a solid fill or a cleared area shares one tile) get one upload to
/// a scratch tile and GPU copies from it: writing each one from the CPU sent 387 MB per refresh
/// of a filled 50 MP 16-bit page (#1774).
pub(crate) fn put_tiles(device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, r: &Resident, cs: &[TileCoord], bytes: &[u8]) -> u64 {
    if let [c] = cs
        && !r.ordered
    {
        write_tile(queue, r, *c, bytes);
        return bytes.len() as u64;
    }
    if cs.is_empty() {
        return 0;
    }
    let extent = wgpu::Extent3d { width: TILE_SIZE as u32, height: TILE_SIZE as u32, depth_or_array_layers: 1 };
    // Its own texture per call: queued writes land before the encoder runs, so a shared scratch
    // would hold only the last tile by then.
    let scratch = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pc_tile_scratch"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: r.kind.format(),
        usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &scratch, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(TILE_SIZE as u32 * r.kind.bytes_per_pixel() as u32),
            rows_per_image: Some(TILE_SIZE as u32),
        },
        extent,
    );
    for c in cs {
        let tr = c.rect();
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &scratch, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo {
                texture: &r.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: (tr.x0 - r.region.x0) as u32, y: (tr.y0 - r.region.y0) as u32, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            extent,
        );
    }
    bytes.len() as u64
}

/// Copy the texels `old` holds inside the region of its replacement `new` (same texel kind,
/// format and default pixel; only the region moved) and carry over the tiles they show, so only
/// tiles outside the overlap or changed since need uploading. Returns the kept (tile-aligned)
/// rect; empty when nothing can be kept.
pub(crate) fn keep_overlap(device: &wgpu::Device, queue: &wgpu::Queue, old: &Resident, new: &mut Resident) -> Rect {
    let same = old.kind == new.kind && old.format == new.format && old.cmyk == new.cmyk && old.default_bits == new.default_bits;
    let k = old.region.intersect(&new.region);
    if !same || k.is_empty() {
        return Rect::EMPTY;
    }
    // Its own submission: tile writes go through the queue and land at the start of the next
    // submit, before anything in the frame's encoder, so a copy recorded there would overwrite
    // the tiles written after it with the old texels.
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_compose_keep") });
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &old.texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: (k.x0 - old.region.x0) as u32, y: (k.y0 - old.region.y0) as u32, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: &new.texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: (k.x0 - new.region.x0) as u32, y: (k.y0 - new.region.y0) as u32, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d { width: k.width(), height: k.height(), depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    new.tiles = old.tiles.iter().filter(|(c, _)| k.contains_rect(&c.rect())).map(|(c, t)| (*c, t.clone())).collect();
    k
}

/// Convert `tiles` of `surface` to `kind` texels and hand each distinct tile, the coordinates
/// sharing it and its bytes to `put`. Each distinct tile is converted once (a solid fill shares
/// one `Arc` across every coordinate it covers), in parallel batches. Converting every tile one
/// after another on the UI thread froze a 50 MP 16-bit document for about 1.4 s after a
/// full-canvas fill (#1774). Returns how many tiles were converted.
pub(crate) fn convert_tiles(
    surface: &Surface,
    tiles: Vec<(TileCoord, Arc<Tile>)>,
    kind: TexKind,
    mut put: impl FnMut(&Arc<Tile>, &[TileCoord], &[u8]),
) -> usize {
    let mut groups: Vec<(Arc<Tile>, Vec<TileCoord>)> = Vec::new();
    let mut index: HashMap<*const Tile, usize> = HashMap::new();
    for (c, t) in tiles {
        match index.get(&Arc::as_ptr(&t)).and_then(|&i| groups.get_mut(i)) {
            Some(g) => g.1.push(c),
            None => {
                index.insert(Arc::as_ptr(&t), groups.len());
                groups.push((t, vec![c]));
            }
        }
    }
    // CMYK and Lab tiles convert pixel by pixel through a colour transform (a CMYK tile took
    // ~5 ms on one thread, the whole of a brush dab's refresh on an A4 CMYK page, #5108): those
    // go in bands of rows, so even the one tile a dab touches spreads over the threads. Workers
    // don't inherit this thread's CMYK profile scope, so each band enters it.
    let bands = if colour_managed(&surface.format(), kind) { TILE_SIZE / MANAGED_BAND_ROWS } else { 1 };
    let space = photocraft_color::convert::active_cmyk_space();
    for batch in groups.chunks(CONVERT_BATCH) {
        // A tile's content is the same at every coordinate sharing it: convert it at the first.
        let jobs: Vec<_> = batch.iter().filter_map(|(t, cs)| cs.first().map(|c| (t, *c))).flat_map(|(t, c)| (0..bands).map(move |b| (t, c, b))).collect();
        let parts = fx::par_map(jobs, |(t, c, b)| {
            photocraft_color::convert::with_cmyk_space(space.as_ref(), || {
                if bands == 1 {
                    return convert_tile(surface, Some(t), kind, c);
                }
                let r = c.rect();
                let y0 = r.y0 + b * MANAGED_BAND_ROWS;
                convert_rows(surface, Some(t), kind, Rect::new(r.x0, y0, r.x1, y0 + MANAGED_BAND_ROWS))
            })
        });
        for ((t, cs), bytes) in batch.iter().zip(parts.chunks(bands as usize)) {
            // One part (every format but CMYK and Lab) goes through as is: no copy per tile.
            match bytes {
                [one] => put(t, cs, one),
                _ => put(t, cs, &bytes.concat()),
            }
        }
    }
    groups.len()
}

/// Rows per parallel band of a colour-managed (CMYK, Lab) tile conversion.
const MANAGED_BAND_ROWS: i32 = 16;

/// Whether `fmt` pixels reach `kind` texels through a per-pixel colour transform (CMYK, Lab).
pub(crate) fn colour_managed(fmt: &PixelFormat, kind: TexKind) -> bool {
    use photocraft_color::ColorMode;
    matches!(kind, TexKind::Rgba8 | TexKind::Rgba16F) && matches!(fmt.mode, ColorMode::Cmyk | ColorMode::Lab)
}

/// Tile pixels in the texture's format. `tile = None` gives the default pixel everywhere.
pub(crate) fn convert_tile(surface: &Surface, tile: Option<&Arc<Tile>>, kind: TexKind, c: TileCoord) -> Vec<u8> {
    if let (Some(t), TexKind::Rgba8Direct | TexKind::R8Direct) = (tile, kind) {
        return t.bytes().to_vec();
    }
    let fmt = surface.format();
    if let (Some(t), TexKind::Rgba16F) = (tile, kind)
        && let Some(out) = rgb_gray16_to_f16(&fmt, t.bytes()).or_else(|| rgb_gray32_to_f16(&fmt, t.bytes()))
    {
        return out;
    }
    convert_rows(surface, tile, kind, c.rect())
}

/// [`convert_tile`]'s general path for `rect` (rows of one tile): each pixel through
/// `photocraft_raster::to_rgba`. A run of identical pixels (the transparent or flat areas around a
/// brush dab) is converted once: the same samples give the same texels.
fn convert_rows(surface: &Surface, tile: Option<&Arc<Tile>>, kind: TexKind, rect: Rect) -> Vec<u8> {
    let n = rect.width() as usize * rect.height() as usize;
    let fmt = surface.format();
    let ch = fmt.channels();
    let raw: Vec<f32> = match tile {
        Some(_) => surface.read_region(rect),
        None => surface.default_pixel().repeat(n),
    };
    let mut out = Vec::with_capacity(n * kind.bytes_per_pixel());
    // The previous pixel and its colour.
    let mut last: Option<(usize, [f32; 4])> = None;
    for (i, px) in raw.chunks_exact(ch).enumerate() {
        let mut rgba = || -> [f32; 4] {
            if let Some((j, v)) = last
                && raw.get(j * ch..j * ch + ch).is_some_and(|p| p.iter().zip(px).all(|(a, b)| a.to_bits() == b.to_bits()))
            {
                return v;
            }
            let v = photocraft_raster::to_rgba(&fmt, px);
            last = Some((i, v));
            v
        };
        match kind {
            TexKind::Rgba8Direct | TexKind::Rgba8 => {
                let v = rgba();
                out.extend(v.map(|x| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
            }
            TexKind::Rgba16F => {
                let v = rgba();
                for x in v {
                    out.extend(f32_to_f16(x).to_le_bytes());
                }
            }
            TexKind::R8Direct => out.push((px[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8),
            TexKind::R16F => out.extend(f32_to_f16(px[0]).to_le_bytes()),
            TexKind::R32F => out.extend(px[0].to_le_bytes()),
        }
    }
    out
}

/// RGBA16F texels of a 16-bit RGB or grayscale tile, through a table of every 16-bit sample's
/// half: the same bytes as the general path at a fraction of its cost (a 50 MP 16-bit refresh
/// converts dozens of tiles). `None` for other formats.
pub(crate) fn rgb_gray16_to_f16(fmt: &PixelFormat, bytes: &[u8]) -> Option<Vec<u8>> {
    use photocraft_color::ColorMode;
    static HALF: std::sync::OnceLock<Box<[u16]>> = std::sync::OnceLock::new();
    if fmt.sample != SampleType::U16 {
        return None;
    }
    let gray = match fmt.mode {
        ColorMode::Rgb => false,
        ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone => true,
        _ => return None,
    };
    let half = HALF.get_or_init(|| (0..=u16::MAX).map(|v| f32_to_f16(v as f32 / 65535.0)).collect());
    let one = f32_to_f16(1.0).to_le_bytes();
    let ch = fmt.channels();
    let mut out = Vec::with_capacity((TILE_SIZE * TILE_SIZE) as usize * 8);
    for px in bytes.chunks_exact(ch * 2) {
        let h = |i: usize| -> [u8; 2] {
            let v = px.get(i * 2..i * 2 + 2).map_or(0, |b| u16::from_ne_bytes([b[0], b[1]]));
            half.get(v as usize).copied().unwrap_or(0).to_le_bytes()
        };
        let (r, g, b) = if gray { (h(0), h(0), h(0)) } else { (h(0), h(1), h(2)) };
        let a = if fmt.alpha { h(ch - 1) } else { one };
        out.extend_from_slice(&r);
        out.extend_from_slice(&g);
        out.extend_from_slice(&b);
        out.extend_from_slice(&a);
    }
    Some(out)
}

/// RGBA16F texels of a 32-bit float RGB or grayscale tile, straight from its samples: the same
/// bytes as the general path without decoding the tile into a float buffer and converting it
/// pixel by pixel (a brush dab on a 32-bit layer re-uploads the tiles it touched). `None` for
/// other formats.
pub(crate) fn rgb_gray32_to_f16(fmt: &PixelFormat, bytes: &[u8]) -> Option<Vec<u8>> {
    use photocraft_color::ColorMode;
    if fmt.sample != SampleType::F32 {
        return None;
    }
    let gray = match fmt.mode {
        ColorMode::Rgb => false,
        ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone => true,
        _ => return None,
    };
    let one = f32_to_f16(1.0).to_le_bytes();
    let ch = fmt.channels();
    let mut out = Vec::with_capacity((TILE_SIZE * TILE_SIZE) as usize * 8);
    for px in bytes.chunks_exact(ch * 4) {
        let h = |i: usize| -> [u8; 2] {
            let v = px.get(i * 4..i * 4 + 4).map_or(0.0, |b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]));
            f32_to_f16(v).to_le_bytes()
        };
        let (r, g, b) = if gray {
            let v = h(0);
            (v, v, v)
        } else {
            (h(0), h(1), h(2))
        };
        let a = if fmt.alpha { h(ch - 1) } else { one };
        out.extend_from_slice(&r);
        out.extend_from_slice(&g);
        out.extend_from_slice(&b);
        out.extend_from_slice(&a);
    }
    Some(out)
}

pub(crate) fn lut_texture(device: &wgpu::Device, queue: &wgpu::Queue, rows: &[[f32; 4096]]) -> (wgpu::Texture, wgpu::TextureView) {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pc_compose_lut"),
        size: wgpu::Extent3d { width: 4096, height: rows.len() as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes: Vec<u8> = rows.iter().flat_map(|r| r.iter().flat_map(|v| v.to_le_bytes())).collect();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4096 * 4), rows_per_image: Some(rows.len() as u32) },
        wgpu::Extent3d { width: 4096, height: rows.len() as u32, depth_or_array_layers: 1 },
    );
    let v = t.create_view(&Default::default());
    (t, v)
}

#[derive(Clone, Copy)]
pub(crate) enum W {
    I(i32),
    U(u32),
    F(f32),
}

pub(crate) fn words(v: &[W]) -> Vec<u8> {
    v.iter()
        .flat_map(|w| match w {
            I(i) => i.to_le_bytes(),
            U(u) => u.to_le_bytes(),
            F(f) => f.to_le_bytes(),
        })
        .collect()
}

/// IEEE half from f32 (round to nearest even; overflow → inf).
pub fn f32_to_f16(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rounded = (m + half - 1 + ((m >> shift) & 1)) >> shift;
        return sign | rounded as u16;
    }
    let mut h = ((e as u32) << 10) | (man >> 13);
    let rest = man & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && (h & 1) == 1) {
        h += 1;
    }
    sign | h as u16
}

/// f32 from IEEE half.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let man = (h & 0x3ff) as f32;
    match exp {
        0 => sign * man * 2f32.powi(-24),
        0x1f => {
            if man == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => sign * (1.0 + man / 1024.0) * 2f32.powi(e - 15),
    }
}
