//! Minimal, deterministic PSD fixtures for the PSD recovery tests.
//!
//! Pure Rust / std: no binary fixtures, external assets, Python, or build script.
//! The damaged fixture has valid layer records and a valid merged composite;
//! only the green channel of the orange layer contains a truncated PackBits run.

const WIDTH: usize = 128;
const HEIGHT: usize = 96;
const PANEL: (usize, usize, usize, usize) = (20, 16, 108, 80);
const PANEL_RGB: [u8; 3] = [244, 156, 28];
const PANEL_ALPHA: u8 = 220;

struct Channel {
    id: i16,
    data: Vec<u8>,
}

struct Layer {
    record: Vec<u8>,
    pixels: Vec<u8>,
}

/// Writes the big-endian 32-bit length of a PSD section to the output buffer.
fn put_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&u32::try_from(len).expect("test PSD section fits u32").to_be_bytes());
}

/// Builds the three RGB planes of the deterministic blue checkerboard background.
fn background_rgb() -> [Vec<u8>; 3] {
    let mut planes: [Vec<u8>; 3] = std::array::from_fn(|_| Vec::with_capacity(WIDTH * HEIGHT));
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let checker = u8::try_from((x / 16 + y / 16) % 2).expect("checker value is 0 or 1");
            planes[0].push(34 + checker * 12);
            planes[1].push(102 + checker * 15);
            planes[2].push(184 + checker * 12);
        }
    }
    planes
}

/// Encodes a layer channel without compression, including its PSD compression marker.
fn raw_channel(id: i16, pixels: &[u8]) -> Channel {
    let mut data = Vec::with_capacity(2 + pixels.len());
    data.extend_from_slice(&0u16.to_be_bytes()); // 0 = uncompressed
    data.extend_from_slice(pixels);
    Channel { id, data }
}

/// Encodes constant-color PackBits rows, optionally truncating the middle row.
/// The corrupt variant keeps the row-length table structurally valid.
fn rle_channel(id: i16, sample: u8, width: usize, height: usize, corrupt: bool) -> Channel {
    let mut rows: Vec<Vec<u8>> = Vec::with_capacity(height);
    for y in 0..height {
        if corrupt && y == height / 2 {
            // 0 declares one literal byte, but that byte is missing.
            // Its row-length entry remains valid: this tests the decoder.
            rows.push(vec![0]);
            continue;
        }
        let mut row = Vec::new();
        let mut remaining = width;
        while remaining != 0 {
            let count = remaining.min(128);
            // PackBits negative repeat count -(count - 1), modulo 256.
            let header = u8::try_from(257 - count).expect("PackBits repeat header fits u8");
            row.extend_from_slice(&[header, sample]);
            remaining -= count;
        }
        rows.push(row);
    }
    let mut data = Vec::new();
    data.extend_from_slice(&1u16.to_be_bytes()); // 1 = PackBits/RLE
    for row in &rows {
        data.extend_from_slice(&u16::try_from(row.len()).expect("row fits PSD u16 length").to_be_bytes());
    }
    for row in rows {
        data.extend_from_slice(&row);
    }
    Channel { id, data }
}

/// Serializes a PSD layer record and its channel payloads separately, in file order.
fn layer(name: &str, rect: (i32, i32, i32, i32), channels: Vec<Channel>) -> Layer {
    let (left, top, right, bottom) = rect;
    let name_bytes = name.as_bytes();
    let name_len = u8::try_from(name_bytes.len()).expect("layer name fits Pascal string");

    let mut extra = Vec::new();
    extra.extend_from_slice(&0u32.to_be_bytes()); // no layer mask
    extra.extend_from_slice(&0u32.to_be_bytes()); // no blending ranges
    extra.push(name_len);
    extra.extend_from_slice(name_bytes);
    while extra.len() % 4 != 0 {
        extra.push(0);
    }

    let mut record = Vec::new();
    for coordinate in [top, left, bottom, right] {
        record.extend_from_slice(&coordinate.to_be_bytes());
    }
    record.extend_from_slice(&u16::try_from(channels.len()).expect("channel count fits u16").to_be_bytes());
    for channel in &channels {
        record.extend_from_slice(&channel.id.to_be_bytes());
        put_len(&mut record, channel.data.len());
    }
    record.extend_from_slice(b"8BIMnorm");
    record.extend_from_slice(&[255, 0, 0, 0]); // opacity, clipping, flags, filler
    put_len(&mut record, extra.len());
    record.extend_from_slice(&extra);

    let mut pixels = Vec::new();
    for channel in channels {
        pixels.extend_from_slice(&channel.data);
    }
    Layer { record, pixels }
}

/// Composites the orange rectangle over the checkerboard as an uncompressed
/// merged RGB image, independent of the deliberately damaged layer channel.
fn merged_rgb(background: &[Vec<u8>; 3]) -> Vec<u8> {
    let mut planes = background.clone();
    for y in PANEL.1..PANEL.3 {
        for x in PANEL.0..PANEL.2 {
            let pixel = y * WIDTH + x;
            for channel in 0..3 {
                let fg = u32::from(PANEL_RGB[channel]);
                let bg = u32::from(background[channel][pixel]);
                let alpha = u32::from(PANEL_ALPHA);
                let mixed = (fg * alpha + bg * (255 - alpha) + 127) / 255;
                planes[channel][pixel] = u8::try_from(mixed).expect("composite pixel fits u8");
            }
        }
    }
    let mut data = Vec::new();
    data.extend_from_slice(&0u16.to_be_bytes()); // merged composite: uncompressed
    for plane in &planes {
        data.extend_from_slice(plane);
    }
    data
}

/// Returns a complete PSD file in memory. `corrupt` changes *only* one RLE row
/// in the green channel of the upper layer; the file is otherwise well framed.
pub fn make_psd(corrupt: bool) -> Vec<u8> {
    let background = background_rgb();
    let background_layer =
        layer("SAFE blue background", (0, 0, WIDTH as i32, HEIGHT as i32), (0..3).map(|ch| raw_channel(ch, &background[ch as usize])).collect());

    let panel_width = PANEL.2 - PANEL.0;
    let panel_height = PANEL.3 - PANEL.1;
    let top_layer = layer(
        "BROKEN orange RLE",
        (PANEL.0 as i32, PANEL.1 as i32, PANEL.2 as i32, PANEL.3 as i32),
        vec![
            rle_channel(0, PANEL_RGB[0], panel_width, panel_height, false),
            rle_channel(1, PANEL_RGB[1], panel_width, panel_height, corrupt),
            rle_channel(2, PANEL_RGB[2], panel_width, panel_height, false),
            rle_channel(-1, PANEL_ALPHA, panel_width, panel_height, false),
        ],
    );
    // Photoshop stores the top layer's record first.
    let layers = [top_layer, background_layer];
    let mut layer_info = Vec::new();
    layer_info.extend_from_slice(&2i16.to_be_bytes());
    for layer in &layers {
        layer_info.extend_from_slice(&layer.record);
    }
    for layer in &layers {
        layer_info.extend_from_slice(&layer.pixels);
    }
    if layer_info.len() % 2 != 0 {
        layer_info.push(0); // layer info is padded to an even size
    }

    let mut layer_and_mask = Vec::new();
    put_len(&mut layer_and_mask, layer_info.len());
    layer_and_mask.extend_from_slice(&layer_info);
    layer_and_mask.extend_from_slice(&0u32.to_be_bytes()); // empty global layer mask

    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&1u16.to_be_bytes()); // PSD (not PSB)
    out.extend_from_slice(&[0u8; 6]);
    out.extend_from_slice(&3u16.to_be_bytes()); // RGB channels
    out.extend_from_slice(&(HEIGHT as u32).to_be_bytes());
    out.extend_from_slice(&(WIDTH as u32).to_be_bytes());
    out.extend_from_slice(&8u16.to_be_bytes()); // 8 bits per channel
    out.extend_from_slice(&3u16.to_be_bytes()); // RGB colour mode
    out.extend_from_slice(&0u32.to_be_bytes()); // no colour-mode data
    out.extend_from_slice(&0u32.to_be_bytes()); // no image resources
    put_len(&mut out, layer_and_mask.len());
    out.extend_from_slice(&layer_and_mask);
    out.extend_from_slice(&merged_rgb(&background));
    out
}
