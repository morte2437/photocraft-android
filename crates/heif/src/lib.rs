//! `photocraft-heif`: the optional HEIF / HEIC (iPhone and Mac photo) decoder.
//!
//! A thin wrapper around `heic-rs`, a pure-Rust HEVC still-picture decoder: single pictures and
//! grid-tiled photos, 8- and 10-bit (10-bit decodes to 16-bit), alpha auxiliary images, ICC,
//! EXIF and XMP. Its API is plain data ([`Info`], [`Decoded`], [`Error`]) so the crate knows
//! nothing about the rest of PhotoCraft; `photocraft-codecs` adapts it behind its `heif` feature.
//!
//! A grid's ICC profile is inherited from its tiles when the grid item itself carries no `colr`,
//! which is how phone cameras write it: see [`icc_for_item`].
//!
//! HEIF records orientation in the container (`irot`/`imir`, plus a `clap` crop), not in EXIF.
//! [`Options::apply_transforms`] applies them; with it off, the pixels come back as coded.
//!
//! Never panics: heic-rs is young, so every call into it runs under `catch_unwind` and a panic
//! inside it becomes [`Error::Malformed`].

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::borrow::Cow;

use heic_rs::PixelLayout;

/// What a HEIF file declares, read from the container alone (no pixel is decoded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    /// Size after the container transforms (rotation, mirror, crop): what a viewer shows.
    pub width: u32,
    pub height: u32,
    /// Size as coded, before the transforms.
    pub coded_width: u32,
    pub coded_height: u32,
    /// The primary image has an alpha auxiliary image.
    pub has_alpha: bool,
    /// More than 8 bits per sample: it decodes to 16-bit.
    pub sixteen_bit: bool,
}

/// Decode settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Refuse images with more pixels than this (checked before decoding).
    pub max_pixels: u64,
    /// Apply the container's rotation, mirror and crop (`irot`/`imir`/`clap`).
    pub apply_transforms: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { max_pixels: 1 << 28, apply_transforms: true }
    }
}

/// A decoded image: interleaved RGB or RGBA samples, row-major, no padding. 16-bit samples are
/// native-endian `u16`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// Four channels (RGBA) instead of three (RGB).
    pub has_alpha: bool,
    /// Two bytes per sample instead of one.
    pub sixteen_bit: bool,
    pub data: Vec<u8>,
    /// The primary image's ICC profile.
    pub icc: Option<Vec<u8>>,
    /// The primary image's `nclx` colour description (inherited from its tiles like [`Self::icc`]).
    /// The decoder has already applied its matrix and range; its primaries and transfer
    /// characteristics describe the RGB samples returned.
    pub nclx: Option<Nclx>,
    /// EXIF as a TIFF structure (without HEIF's offset header). Its Orientation tag only mirrors
    /// what the container says; it is returned verbatim.
    pub exif: Option<Vec<u8>>,
    /// The XMP packet.
    pub xmp: Option<String>,
}

/// A `colr` box of type `nclx`: ITU-T H.273 code points (CICP), as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nclx {
    /// Colour primaries: 1 BT.709, 9 BT.2020, 12 P3-D65…
    pub primaries: u16,
    /// Transfer characteristics: 13 sRGB, 1/6/14/15 BT.709, 16 PQ, 18 HLG…
    pub transfer: u16,
    /// Matrix coefficients (already applied by the decoder).
    pub matrix: u16,
    /// Full-range samples (already applied by the decoder).
    pub full_range: bool,
}

/// Why a file could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A valid file using something this decoder does not handle (image sequences, overlays…).
    Unsupported(String),
    /// The image is larger than [`Options::max_pixels`] or holds an oversized box.
    Limit(String),
    /// Broken or truncated data (or a decoder bug, reported the same way).
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unsupported(m) => write!(f, "unsupported HEIF: {m}"),
            Error::Limit(m) => write!(f, "limit exceeded: {m}"),
            Error::Malformed(m) => write!(f, "malformed HEIF: {m}"),
        }
    }
}

impl std::error::Error for Error {}

const SEQUENCE: &str = "this file holds an image sequence; only HEIF still images can be opened";

fn err(e: heic_rs::Error) -> Error {
    match e {
        heic_rs::Error::Unsupported(what) => Error::Unsupported(what.to_string()),
        // The primary item is neither an HEVC picture nor a grid of them.
        heic_rs::Error::MissingBox("hvcC") => Error::Unsupported("the image is an overlay, an identity derivation or not HEVC-coded".into()),
        heic_rs::Error::PixelLimit { .. } | heic_rs::Error::BoxTooLarge { .. } => Error::Limit(e.to_string()),
        e => Error::Malformed(e.to_string()),
    }
}

/// Runs `f`, turning a heic-rs panic into an error. Fuzzing found two out-of-range slices in
/// heic-rs 0.1.1 on malformed files: in its box parser (`boxes.rs:130`) and during reconstruction
/// (`hevc/decode/recon.rs:70`). The codecs fuzz target also calls heic-rs directly, so its panics
/// stay visible there.
fn guarded<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(Error::Malformed("the HEIF decoder failed on this file".into())))
}

/// Reads the container's declared size, depth and alpha without decoding pixels, so callers can
/// check their limits first.
pub fn probe(bytes: &[u8]) -> Result<Info, Error> {
    guarded(|| probe_unguarded(bytes))
}

fn probe_unguarded(bytes: &[u8]) -> Result<Info, Error> {
    let info = heic_rs::probe(bytes).map_err(|e| match e {
        // No `meta` box: no still image, only a `moov` image sequence (a video track).
        heic_rs::Error::MissingBox("meta") => Error::Unsupported(SEQUENCE.into()),
        e => err(e),
    })?;
    Ok(Info {
        width: info.width,
        height: info.height,
        coded_width: info.coded_width,
        coded_height: info.coded_height,
        has_alpha: info.has_alpha,
        sixteen_bit: info.bit_depth > 8,
    })
}

/// Decodes the primary image, with its metadata.
pub fn decode(bytes: &[u8], options: &Options) -> Result<Decoded, Error> {
    guarded(|| decode_unguarded(bytes, options))
}

fn decode_unguarded(bytes: &[u8], options: &Options) -> Result<Decoded, Error> {
    let info = probe_unguarded(bytes)?;
    let pixel_layout = match (info.has_alpha, info.sixteen_bit) {
        (true, true) => PixelLayout::Rgba16,
        (true, false) => PixelLayout::Rgba8,
        (false, true) => PixelLayout::Rgb16,
        (false, false) => PixelLayout::Rgb8,
    };
    let raw = heic_rs::DecodeOptions::default()
        .with_layout(pixel_layout)
        .with_max_pixels(Some(options.max_pixels))
        .with_transforms(options.apply_transforms)
        .with_alpha(info.has_alpha);
    let decoded = heic_rs::decode(bytes, &raw).map_err(err)?;
    let (icc, nclx, exif, xmp) = metadata(bytes);
    Ok(Decoded {
        width: decoded.width,
        height: decoded.height,
        has_alpha: info.has_alpha,
        sixteen_bit: info.sixteen_bit,
        data: decoded.data,
        icc,
        nclx,
        exif,
        xmp,
    })
}

type Metadata = (Option<Vec<u8>>, Option<Nclx>, Option<Vec<u8>>, Option<String>);

/// The primary image's ICC profile, `nclx`, EXIF (TIFF structure) and XMP packet. Metadata is a
/// courtesy: a broken metadata item never fails a decode whose pixels came out fine.
fn metadata(bytes: &[u8]) -> Metadata {
    let Ok(ctx) = heic_rs::context::Context::open(bytes) else {
        return (None, None, None, None);
    };
    let id = ctx.meta.primary;
    let icc = icc_for_item(&ctx, id);
    let nclx = nclx_for_item(&ctx, id);
    let exif = ctx.exif(id).ok().flatten().map(<[u8]>::to_vec);
    let xmp = ctx.xmp_item(id).and_then(|x| ctx.item_data(x).ok()).and_then(|d| match d {
        Cow::Borrowed(b) => String::from_utf8(b.to_vec()).ok(),
        Cow::Owned(b) => String::from_utf8(b).ok(),
    });
    (icc, nclx, exif, xmp.map(|x| x.trim_end_matches('\0').to_string()))
}

/// The ICC profile describing `item`, inherited from the coded tiles when the item itself carries
/// none.
///
/// `colr` is optional on a `grid` derivation, and phone cameras write it on the tiles instead: a
/// OnePlus 11 photograph carries one `prof` (Display P3) on each of its 48 `hvc1` tiles and
/// nothing on the grid. Asking the primary item alone therefore loses the profile and the photo
/// opens as if it were sRGB. The tiles are what the mosaic is composed from, so their profile is
/// the composed image's profile.
///
/// heic-rs resolves `nclx` from the primary item in the same way and has the same gap
/// (tbraun96/heic-rs#6). The sample range cannot be recovered here as well: `DecodeOptions` has no
/// override for it, so that half has to be fixed upstream.
fn icc_for_item(ctx: &heic_rs::context::Context<'_>, item: u32) -> Option<Vec<u8>> {
    inherited(ctx, item, |p| ctx.icc(p).map(<[u8]>::to_vec))
}

/// The `nclx` colour description of `item`, inherited from its tiles as [`icc_for_item`] does.
/// A 10-bit HDR HEIC declares its BT.2020 primaries and HLG or PQ transfer only here (#6752).
fn nclx_for_item(ctx: &heic_rs::context::Context<'_>, item: u32) -> Option<Nclx> {
    inherited(ctx, item, |p| {
        p.nclx.map(|n| Nclx { primaries: n.primaries, transfer: n.transfer, matrix: n.matrix_code, full_range: n.range == heic_rs::Range::Full })
    })
}

/// `get` of `item`'s properties, or of the first of its grid tiles that has it.
fn inherited<T>(ctx: &heic_rs::context::Context<'_>, item: u32, get: impl Fn(&heic_rs::props::ItemProps<'_>) -> Option<T>) -> Option<T> {
    let own = ctx.props(item).ok().and_then(|p| get(&p));
    if own.is_some() {
        return own;
    }
    // `Ok(None)` is a coded picture rather than a derivation: no tiles to inherit from.
    let (_, tiles) = ctx.grid(item).ok()??;
    tiles.iter().find_map(|&t| ctx.props(t).ok().and_then(|p| get(&p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Found by the `decode_heif` fuzz target: a malformed box makes heic-rs 0.1.1 slice out of
    /// range (`boxes.rs:130`). It must be an error, not a crash.
    const HEIC_RS_BOX_PANIC: [u8; 72] = [
        0x00, 0x00, 0x00, 0x24, 0x66, 0x74, 0x79, 0x70, 0x68, 0x65, 0x69, 0x63, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x69, 0x66, 0x31, 0x4d, 0x69, 0x50, 0x72, 0x6d,
        0x69, 0x61, 0x66, 0x4d, 0x69, 0x48, 0x42, 0x72, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x65, 0x74, 0x61, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x10, 0x75, 0x75, 0x69, 0x64, 0x00, 0x00, 0x00, 0x00, 0x03, 0x08, 0x08, 0x08, 0x00, 0x03, 0x01, 0x03, 0x70, 0x00, 0xa8, 0x00,
    ];

    #[test]
    fn a_heic_rs_panic_is_a_malformed_error() {
        for apply_transforms in [true, false] {
            let r = decode(&HEIC_RS_BOX_PANIC, &Options { apply_transforms, ..Default::default() });
            assert!(matches!(r, Err(Error::Malformed(_))), "{r:?}");
        }
    }

    #[test]
    fn image_sequence_is_unsupported_with_a_clear_message() {
        // An ftyp and nothing else: a HEIF image sequence would have a moov here instead of a meta.
        let bytes = b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc";
        for r in [probe(bytes).map(|_| ()), decode(bytes, &Options::default()).map(|_| ())] {
            assert!(matches!(&r, Err(Error::Unsupported(m)) if m.contains("sequence")), "{r:?}");
        }
    }

    #[test]
    fn garbage_is_an_error() {
        for bytes in [&b""[..], b"\0\0\0\x18ftypheic\0\0\0\0mif1heic", &[0xFF; 64]] {
            assert!(probe(bytes).is_err());
            assert!(decode(bytes, &Options::default()).is_err());
        }
    }

    #[test]
    fn errors_display_their_kind() {
        assert!(Error::Unsupported("x".into()).to_string().contains("unsupported"));
        assert!(Error::Limit("x".into()).to_string().contains("limit"));
        assert!(Error::Malformed("x".into()).to_string().contains("malformed"));
    }

    // A grid whose colour description lives on its tiles, not on the grid item.
    //
    // The fixtures below are synthetic containers with no coded data at all: resolving a profile
    // never decodes a pixel. Each test first asserts the fixture is the shape it claims to be, so
    // a mistake in the builder cannot look like a passing fix.

    /// Stands in for a profile. `colr` hands its payload back without parsing it.
    const TILE_ICC: &[u8] = b"tile-profile";
    const GRID_ICC: &[u8] = b"grid-profile";

    const GRID_ID: u16 = 1;
    const PICTURE_ID: u16 = 2;

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = u32::try_from(payload.len() + 8).unwrap().to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    /// A box whose first four payload bytes are a version and 24 bits of flags.
    fn full(kind: &[u8; 4], version: u8, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![version, 0, 0, 0];
        p.extend_from_slice(payload);
        bx(kind, &p)
    }

    fn hdlr() -> Vec<u8> {
        let mut p = vec![0; 4];
        p.extend_from_slice(b"pict");
        p.extend_from_slice(&[0; 12]);
        p.push(0);
        full(b"hdlr", 0, &p)
    }

    fn infe(id: u16, item_type: &[u8; 4]) -> Vec<u8> {
        let mut p = id.to_be_bytes().to_vec();
        p.extend_from_slice(&0u16.to_be_bytes());
        p.extend_from_slice(item_type);
        p.push(0);
        full(b"infe", 2, &p)
    }

    fn iref_dimg(from: u16, to: u16) -> Vec<u8> {
        let mut p = from.to_be_bytes().to_vec();
        p.extend_from_slice(&1u16.to_be_bytes());
        p.extend_from_slice(&to.to_be_bytes());
        full(b"iref", 0, &bx(b"dimg", &p))
    }

    fn colr_prof(icc: &[u8]) -> Vec<u8> {
        let mut p = b"prof".to_vec();
        p.extend_from_slice(icc);
        bx(b"colr", &p)
    }

    /// A `colr` box of type `nclx`, laid out as ISO/IEC 23008-12 says: three big-endian `u16`
    /// code points, then the full-range flag in the top bit of a byte.
    fn colr_nclx(primaries: u16, transfer: u16, matrix: u16, full_range: bool) -> Vec<u8> {
        let mut p = b"nclx".to_vec();
        for v in [primaries, transfer, matrix] {
            p.extend_from_slice(&v.to_be_bytes());
        }
        p.push(if full_range { 0x80 } else { 0 });
        bx(b"colr", &p)
    }

    fn ispe(w: u32, h: u32) -> Vec<u8> {
        let mut p = w.to_be_bytes().to_vec();
        p.extend_from_slice(&h.to_be_bytes());
        full(b"ispe", 0, &p)
    }

    fn ipma(entries: &[(u16, &[u8])]) -> Vec<u8> {
        let mut p = u32::try_from(entries.len()).unwrap().to_be_bytes().to_vec();
        for (item, assoc) in entries {
            p.extend_from_slice(&item.to_be_bytes());
            p.push(u8::try_from(assoc.len()).unwrap());
            p.extend_from_slice(assoc);
        }
        full(b"ipma", 0, &p)
    }

    /// `item`'s bytes live in `idat` (construction method 1) at offset 0.
    fn iloc_in_idat(item: u16, length: u32) -> Vec<u8> {
        let mut p = vec![0x44, 0x00];
        p.extend_from_slice(&1u16.to_be_bytes());
        p.extend_from_slice(&item.to_be_bytes());
        p.extend_from_slice(&1u16.to_be_bytes());
        p.extend_from_slice(&0u16.to_be_bytes());
        p.extend_from_slice(&1u16.to_be_bytes());
        p.extend_from_slice(&0u32.to_be_bytes());
        p.extend_from_slice(&length.to_be_bytes());
        full(b"iloc", 1, &p)
    }

    fn file(meta: &[u8]) -> Vec<u8> {
        let mut ftyp = b"heic".to_vec();
        ftyp.extend_from_slice(&0u32.to_be_bytes());
        ftyp.extend_from_slice(b"mif1heic");
        let mut out = bx(b"ftyp", &ftyp);
        out.extend_from_slice(&full(b"meta", 0, meta));
        out
    }

    /// A 1x1 64x64 grid, with `grid_icc` on the grid item and `tile_icc` on its single tile.
    ///
    /// `ipco` children are numbered from 1 in file order; an association byte's top bit marks the
    /// property essential, as macOS writes it.
    fn grid_file(grid_icc: Option<&[u8]>, tile_icc: Option<&[u8]>) -> Vec<u8> {
        grid_file_colr(grid_icc.map(colr_prof), tile_icc.map(colr_prof))
    }

    /// [`grid_file`] with whole `colr` boxes.
    fn grid_file_colr(grid_colr: Option<Vec<u8>>, tile_colr: Option<Vec<u8>>) -> Vec<u8> {
        let mut ipco = ispe(64, 64);
        let (mut grid_assoc, mut tile_assoc, mut index) = (vec![0x81], vec![0x81], 1u8);
        if let Some(colr) = grid_colr {
            index += 1;
            ipco.extend_from_slice(&colr);
            grid_assoc.push(0x80 | index);
        }
        if let Some(colr) = tile_colr {
            index += 1;
            ipco.extend_from_slice(&colr);
            tile_assoc.push(0x80 | index);
        }
        let mut iprp = bx(b"ipco", &ipco);
        iprp.extend_from_slice(&ipma(&[(GRID_ID, grid_assoc.as_slice()), (PICTURE_ID, tile_assoc.as_slice())]));

        let mut items = 2u16.to_be_bytes().to_vec();
        items.extend_from_slice(&infe(GRID_ID, b"grid"));
        items.extend_from_slice(&infe(PICTURE_ID, b"hvc1"));

        // Version 0, flags 0, one row, one column, 64x64 output.
        const DESCRIPTOR: &[u8] = &[0, 0, 0, 0, 0, 64, 0, 64];

        let mut meta = hdlr();
        meta.extend_from_slice(&full(b"pitm", 0, &GRID_ID.to_be_bytes()));
        meta.extend_from_slice(&full(b"iinf", 0, &items));
        meta.extend_from_slice(&iref_dimg(GRID_ID, PICTURE_ID));
        meta.extend_from_slice(&bx(b"iprp", &iprp));
        meta.extend_from_slice(&bx(b"idat", DESCRIPTOR));
        meta.extend_from_slice(&iloc_in_idat(GRID_ID, u32::try_from(DESCRIPTOR.len()).unwrap()));
        file(&meta)
    }

    /// One coded picture rather than a derivation, with `icc` on the item itself.
    fn picture_file(icc: Option<&[u8]>) -> Vec<u8> {
        picture_file_colr(icc.map(colr_prof))
    }

    /// [`picture_file`] with a whole `colr` box.
    fn picture_file_colr(colr: Option<Vec<u8>>) -> Vec<u8> {
        let mut ipco = ispe(64, 64);
        let mut assoc = vec![0x81];
        if let Some(colr) = colr {
            ipco.extend_from_slice(&colr);
            assoc.push(0x82);
        }
        let mut iprp = bx(b"ipco", &ipco);
        iprp.extend_from_slice(&ipma(&[(PICTURE_ID, assoc.as_slice())]));

        let mut items = 1u16.to_be_bytes().to_vec();
        items.extend_from_slice(&infe(PICTURE_ID, b"hvc1"));

        let mut meta = hdlr();
        meta.extend_from_slice(&full(b"pitm", 0, &PICTURE_ID.to_be_bytes()));
        meta.extend_from_slice(&full(b"iinf", 0, &items));
        meta.extend_from_slice(&bx(b"iprp", &iprp));
        file(&meta)
    }

    fn ctx_of(bytes: &[u8]) -> heic_rs::context::Context<'_> {
        heic_rs::context::Context::open(bytes).expect("the synthetic container must parse")
    }

    #[test]
    fn a_grid_inherits_its_tiles_icc_profile() {
        let bytes = grid_file(None, Some(TILE_ICC));
        let ctx = ctx_of(&bytes);
        let primary = ctx.meta.primary;
        assert_eq!(primary, u32::from(GRID_ID));
        let (grid, tiles) = ctx.grid(primary).unwrap().expect("a grid derivation");
        assert_eq!((grid.rows, grid.columns, tiles.len()), (1, 1, 1));
        assert!(ctx.icc(&ctx.props(primary).unwrap()).is_none(), "the grid item carries no colr of its own");
        assert_eq!(ctx.icc(&ctx.props(tiles[0]).unwrap()), Some(TILE_ICC), "the tile carries the profile");

        assert_eq!(icc_for_item(&ctx, primary).as_deref(), Some(TILE_ICC));
    }

    #[test]
    fn a_grids_own_profile_wins_over_its_tiles() {
        let bytes = grid_file(Some(GRID_ICC), Some(TILE_ICC));
        let ctx = ctx_of(&bytes);
        assert_eq!(icc_for_item(&ctx, ctx.meta.primary).as_deref(), Some(GRID_ICC));
    }

    #[test]
    fn an_unlabelled_grid_has_no_profile() {
        let bytes = grid_file(None, None);
        let ctx = ctx_of(&bytes);
        assert_eq!(icc_for_item(&ctx, ctx.meta.primary), None);
    }

    #[test]
    fn a_coded_picture_resolves_its_own_profile() {
        let labelled = picture_file(Some(GRID_ICC));
        let ctx = ctx_of(&labelled);
        assert!(ctx.grid(ctx.meta.primary).unwrap().is_none(), "not a derivation");
        assert_eq!(icc_for_item(&ctx, ctx.meta.primary).as_deref(), Some(GRID_ICC));

        let bare = picture_file(None);
        let ctx = ctx_of(&bare);
        assert_eq!(icc_for_item(&ctx, ctx.meta.primary), None);
    }
    /// An iPhone 10-bit HDR HEIC's description (#6752): BT.2020 primaries, ARIB STD-B67 (HLG)
    /// transfer, BT.2020 non-constant-luminance matrix, full range.
    const HLG: Nclx = Nclx { primaries: 9, transfer: 18, matrix: 9, full_range: true };

    #[test]
    fn a_coded_pictures_nclx_is_read() {
        let bytes = picture_file_colr(Some(colr_nclx(9, 18, 9, true)));
        let ctx = ctx_of(&bytes);
        assert_eq!(nclx_for_item(&ctx, ctx.meta.primary), Some(HLG));

        let sdr = picture_file_colr(Some(colr_nclx(1, 13, 6, false)));
        let ctx = ctx_of(&sdr);
        assert_eq!(nclx_for_item(&ctx, ctx.meta.primary), Some(Nclx { primaries: 1, transfer: 13, matrix: 6, full_range: false }));

        let bare = picture_file(Some(TILE_ICC));
        let ctx = ctx_of(&bare);
        assert_eq!(nclx_for_item(&ctx, ctx.meta.primary), None, "an ICC profile is not an nclx");
    }

    #[test]
    fn a_grid_inherits_its_tiles_nclx() {
        let bytes = grid_file_colr(None, Some(colr_nclx(9, 18, 9, true)));
        let ctx = ctx_of(&bytes);
        assert!(ctx.props(ctx.meta.primary).unwrap().nclx.is_none(), "the grid item carries no colr of its own");
        assert_eq!(nclx_for_item(&ctx, ctx.meta.primary), Some(HLG));
    }
}
