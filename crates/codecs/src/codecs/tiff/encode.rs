use super::bands::convert_ab;
use super::tags::{F, TAG_IMAGE_SOURCE_DATA, TAG_PHOTOSHOP, TAG_XMP, TEXT_TAGS};
use super::*;

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Bytes written with TIFF type UNDEFINED (7), as required for ICC.
struct Undefined<'a>(&'a [u8]);

impl TiffValue for Undefined<'_> {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: Type = Type::UNDEFINED;
    fn count(&self) -> usize {
        self.0.len()
    }
    fn data(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(self.0)
    }
}

// Colour types with 1..=MAX_EXTRA_SAMPLES trailing extra samples (spot channels, extra alpha).
// One generic type per (sample, photometric, bit depth, total sample count) instead of a
// hand-written type for every layout. `N` is the total sample count, not the extra count.
const fn extra_photo(v: u16) -> PhotometricInterpretation {
    match v {
        1 => PhotometricInterpretation::BlackIsZero,
        2 => PhotometricInterpretation::RGB,
        5 => PhotometricInterpretation::CMYK,
        8 => PhotometricInterpretation::CIELab,
        _ => PhotometricInterpretation::RGB,
    }
}

trait ExtraKind: Copy + TiffValue + 'static {
    const FORMAT: SampleFormat;
    fn predict(row: &[Self], out: &mut Vec<Self>, samples: usize);
}

impl ExtraKind for u8 {
    const FORMAT: SampleFormat = SampleFormat::Uint;
    fn predict(row: &[Self], out: &mut Vec<Self>, samples: usize) {
        let n = samples.min(row.len());
        out.extend_from_slice(&row[..n]);
        out.extend(row.iter().zip(&row[n..]).map(|(prev, current)| current.wrapping_sub(*prev)));
    }
}

impl ExtraKind for u16 {
    const FORMAT: SampleFormat = SampleFormat::Uint;
    fn predict(row: &[Self], out: &mut Vec<Self>, samples: usize) {
        let n = samples.min(row.len());
        out.extend_from_slice(&row[..n]);
        out.extend(row.iter().zip(&row[n..]).map(|(prev, current)| current.wrapping_sub(*prev)));
    }
}

impl ExtraKind for f32 {
    const FORMAT: SampleFormat = SampleFormat::IEEEFP;
    fn predict(row: &[Self], out: &mut Vec<Self>, _samples: usize) {
        out.extend_from_slice(row);
    }
}

pub(super) struct Extras<T, const PHOTO: u16, const BITS: u16, const N: usize>(pub(super) std::marker::PhantomData<T>);

impl<T, const PHOTO: u16, const BITS: u16, const N: usize> TiffColorType for Extras<T, PHOTO, BITS, N>
where
    T: ExtraKind,
{
    type Inner = T;
    const TIFF_VALUE: PhotometricInterpretation = extra_photo(PHOTO);
    const BITS_PER_SAMPLE: &'static [u16] = &[BITS; N];
    const SAMPLE_FORMAT: &'static [SampleFormat] = &[T::FORMAT; N];
    fn horizontal_predict(row: &[Self::Inner], result: &mut Vec<Self::Inner>) {
        T::predict(row, result, N);
    }
}

macro_rules! write_extras {
    ($n:expr, $enc:expr, $w:expr, $h:expr, $data:expr, $tags:expr, $sample:ty, $photo:literal, $bits:literal, $base:literal) => {
        match $n {
            1 => write_one::<Extras<$sample, $photo, $bits, { $base + 1 }>, K>($enc, $w, $h, $data, $tags),
            2 => write_one::<Extras<$sample, $photo, $bits, { $base + 2 }>, K>($enc, $w, $h, $data, $tags),
            3 => write_one::<Extras<$sample, $photo, $bits, { $base + 3 }>, K>($enc, $w, $h, $data, $tags),
            4 => write_one::<Extras<$sample, $photo, $bits, { $base + 4 }>, K>($enc, $w, $h, $data, $tags),
            5 => write_one::<Extras<$sample, $photo, $bits, { $base + 5 }>, K>($enc, $w, $h, $data, $tags),
            6 => write_one::<Extras<$sample, $photo, $bits, { $base + 6 }>, K>($enc, $w, $h, $data, $tags),
            7 => write_one::<Extras<$sample, $photo, $bits, { $base + 7 }>, K>($enc, $w, $h, $data, $tags),
            8 => write_one::<Extras<$sample, $photo, $bits, { $base + 8 }>, K>($enc, $w, $h, $data, $tags),
            9 => write_one::<Extras<$sample, $photo, $bits, { $base + 9 }>, K>($enc, $w, $h, $data, $tags),
            10 => write_one::<Extras<$sample, $photo, $bits, { $base + 10 }>, K>($enc, $w, $h, $data, $tags),
            11 => write_one::<Extras<$sample, $photo, $bits, { $base + 11 }>, K>($enc, $w, $h, $data, $tags),
            12 => write_one::<Extras<$sample, $photo, $bits, { $base + 12 }>, K>($enc, $w, $h, $data, $tags),
            13 => write_one::<Extras<$sample, $photo, $bits, { $base + 13 }>, K>($enc, $w, $h, $data, $tags),
            14 => write_one::<Extras<$sample, $photo, $bits, { $base + 14 }>, K>($enc, $w, $h, $data, $tags),
            15 => write_one::<Extras<$sample, $photo, $bits, { $base + 15 }>, K>($enc, $w, $h, $data, $tags),
            16 => write_one::<Extras<$sample, $photo, $bits, { $base + 16 }>, K>($enc, $w, $h, $data, $tags),
            other => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported extra samples ({other})")).into()),
        }
    };
}

/// An empty buffer with room for `len` elements, allocated fallibly: a save too large for memory
/// is an error, not an abort.
pub(super) fn try_with_capacity<T>(len: usize) -> io::Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, format!("cannot allocate {len} samples for the extra samples")))?;
    Ok(v)
}

fn bytes_as_u16(b: &[u8]) -> io::Result<Vec<u16>> {
    let chunks = b.as_chunks::<2>().0;
    let mut v = try_with_capacity(chunks.len())?;
    v.extend(chunks.iter().map(|c| u16::from_ne_bytes(*c)));
    Ok(v)
}

fn bytes_as_f32(b: &[u8]) -> io::Result<Vec<f32>> {
    let chunks = b.as_chunks::<4>().0;
    let mut v = try_with_capacity(chunks.len())?;
    v.extend(chunks.iter().map(|c| f32::from_ne_bytes(*c)));
    Ok(v)
}

/// Interleaves colour (and alpha) samples with [`Metadata::extra_samples`].
fn interleave_extras(img: &Image) -> Result<Vec<u8>, String> {
    let (w, h) = img.dimensions();
    let n = (w as usize).checked_mul(h as usize).ok_or_else(|| "image size overflows".to_string())?;
    let sb = img.sample_type().bytes();
    let ch = img.layout().channels();
    let extras = &img.meta.extra_samples;
    let total = ch.checked_add(extras.len()).ok_or_else(|| "too many samples".to_string())?;
    let px = total.checked_mul(sb).ok_or_else(|| "sample size overflows".to_string())?;
    let len = n.checked_mul(px).ok_or_else(|| "image size overflows".to_string())?;
    let mut out = try_with_capacity::<u8>(len).map_err(|e| e.to_string())?;
    out.resize(len, 0);
    let base = img.data();
    let base_px = ch.saturating_mul(sb);
    for i in 0..n {
        let src = i.saturating_mul(base_px);
        let dst = i.saturating_mul(px);
        let Some(s) = base.get(src..src.saturating_add(base_px)) else {
            return Err("image buffer is shorter than its dimensions".into());
        };
        let Some(d) = out.get_mut(dst..dst.saturating_add(base_px)) else {
            return Err("extra-sample buffer is short".into());
        };
        d.copy_from_slice(s);
        for (k, e) in extras.iter().enumerate() {
            let es = i.saturating_mul(sb);
            let ed = dst.saturating_add(base_px).saturating_add(k.saturating_mul(sb));
            let Some(s) = e.data.get(es..es.saturating_add(sb)) else {
                return Err(format!("extra sample {k} is shorter than the image"));
            };
            let Some(d) = out.get_mut(ed..ed.saturating_add(sb)) else {
                return Err("extra-sample buffer is short".into());
            };
            d.copy_from_slice(s);
        }
    }
    Ok(out)
}

fn write_with_extras<K: TiffKind>(enc: &mut TiffEncoder<&mut Cursor<Vec<u8>>, K>, img: &Image, tags: &TagSet<'_>) -> tiff::TiffResult<()> {
    use ChannelLayout as L;
    use SampleType as S;
    let (w, h) = img.dimensions();
    let n = img.meta.extra_samples.len();
    if img.layout().is_lab() {
        let file = lab_file(img);
        let bytes = interleave_extras(&file).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        return match (file.layout(), file.sample_type()) {
            (L::Lab, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 8, 8, 3),
            (L::Lab, _) => {
                let v = bytes_as_u16(&bytes)?;
                write_extras!(n, enc, w, h, &v, tags, u16, 8, 16, 3)
            }
            (_, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 8, 8, 4),
            _ => {
                let v = bytes_as_u16(&bytes)?;
                write_extras!(n, enc, w, h, &v, tags, u16, 8, 16, 4)
            }
        };
    }
    let bytes = interleave_extras(img).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    match (img.layout(), img.sample_type()) {
        (L::Gray, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 1, 8, 1),
        (L::Gray, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 1, 16, 1)
        }
        (L::Gray, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 1, 32, 1)
        }
        (L::GrayA, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 1, 8, 2),
        (L::GrayA, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 1, 16, 2)
        }
        (L::GrayA, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 1, 32, 2)
        }
        (L::Rgb, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 2, 8, 3),
        (L::Rgb, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 2, 16, 3)
        }
        (L::Rgb, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 2, 32, 3)
        }
        (L::Rgba, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 2, 8, 4),
        (L::Rgba, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 2, 16, 4)
        }
        (L::Rgba, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 2, 32, 4)
        }
        (L::Cmyk, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 5, 8, 4),
        (L::Cmyk, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 5, 16, 4)
        }
        (L::Cmyk, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 5, 32, 4)
        }
        (L::CmykA, S::U8) => write_extras!(n, enc, w, h, &bytes, tags, u8, 5, 8, 5),
        (L::CmykA, S::U16) => {
            let v = bytes_as_u16(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, u16, 5, 16, 5)
        }
        (L::CmykA, S::F32) => {
            let v = bytes_as_f32(&bytes)?;
            write_extras!(n, enc, w, h, &v, tags, f32, 5, 32, 5)
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported extra samples ({n})")).into()),
    }
}

struct TagSet<'a> {
    icc: Option<&'a [u8]>,
    /// The EXIF block's directories and IFD0 tags, written as the TIFF's own.
    exif: Option<ExifDirs>,
    xmp: Option<&'a [u8]>,
    dpi: Option<(f32, f32)>,
    text: Vec<(Tag, &'a str)>,
    /// ExtraSamples tag, including the unassociated-alpha marker when the image has alpha.
    extra_samples: &'a [u16],
    photoshop_resources: Option<&'a [u8]>,
    photoshop_layers: Option<&'a [u8]>,
}

/// Writes one image directory. (The `tiff` encoder only compresses through `write_data`, which
/// takes every sample at once.)
fn write_one<C, K>(enc: &mut TiffEncoder<&mut Cursor<Vec<u8>>, K>, w: u32, h: u32, data: &[C::Inner], tags: &TagSet<'_>) -> tiff::TiffResult<()>
where
    C: TiffColorType,
    [C::Inner]: TiffValue,
    K: TiffKind,
{
    // The Exif and GPS directories go before the image, outside its chain; IFD0 points to them.
    let mut sub = |e: &ExifDirs, fields: &[tiff_exif::Field]| (!fields.is_empty()).then(|| tiff_exif::write_directory(enc, fields, e.le)).transpose();
    let exif_at = match &tags.exif {
        Some(e) => sub(e, &e.exif)?,
        None => None,
    };
    let gps_at = match &tags.exif {
        Some(e) => sub(e, &e.gps)?,
        None => None,
    };
    let mut im = enc.new_image::<C>(w, h)?;
    if let Some((x, y)) = tags.dpi {
        im.resolution_unit(ResolutionUnit::Inch);
        im.x_resolution(Rational { n: (x * 1000.0).round() as u32, d: 1000 });
        im.y_resolution(Rational { n: (y * 1000.0).round() as u32, d: 1000 });
    }
    let d = im.encoder();
    if !tags.extra_samples.is_empty() {
        d.write_tag(Tag::ExtraSamples, tags.extra_samples)?;
    }
    if let Some(icc) = tags.icc {
        d.write_tag(Tag::IccProfile, Undefined(icc))?;
    }
    if let Some(xmp) = tags.xmp {
        d.write_tag(Tag::Unknown(TAG_XMP), xmp)?;
    }
    for (tag, s) in &tags.text {
        d.write_tag(*tag, *s)?;
    }
    if let Some(e) = &tags.exif {
        // The EXIF block's descriptive IFD0 tags, unless the text metadata already wrote them.
        let ifd0: Vec<tiff_exif::Field> = e.ifd0.iter().filter(|f| !tags.text.iter().any(|(t, _)| t.to_u16() == f.tag)).cloned().collect();
        tiff_exif::write_fields(d, &ifd0, e.le)?;
    }
    if let Some(at) = exif_at {
        d.write_tag(Tag::Unknown(tiff_exif::TAG_EXIF_IFD), at)?;
    }
    if let Some(at) = gps_at {
        d.write_tag(Tag::Unknown(tiff_exif::TAG_GPS_IFD), at)?;
    }
    if let Some(r) = tags.photoshop_resources {
        d.write_tag(Tag::Unknown(TAG_PHOTOSHOP), r)?;
    }
    if let Some(l) = tags.photoshop_layers {
        d.write_tag(Tag::Unknown(TAG_IMAGE_SOURCE_DATA), Undefined(l))?;
    }
    im.write_data(data)
}

/// A Lab image as CIELab stores it: a* and b* signed, 8 or 16 bits (CIELab has no float form:
/// 32-bit Lab is written with 16 bits). Extra samples follow the new sample type.
fn lab_file(img: &Image) -> Image {
    let sample = if img.sample_type() == SampleType::U8 { SampleType::U8 } else { SampleType::U16 };
    // `convert` brings the extra samples to the new sample type with the image.
    let mut file = img.convert(img.layout(), sample);
    let channels = file.layout().channels();
    convert_ab(file.data_mut(), channels, sample, true, true);
    file
}

/// Writes the image with the colour type matching its layout and sample type.
fn write_image<K: TiffKind>(enc: &mut TiffEncoder<&mut Cursor<Vec<u8>>, K>, img: &Image, tags: &TagSet<'_>) -> tiff::TiffResult<()> {
    use ChannelLayout as L;
    use SampleType as S;
    if !img.meta.extra_samples.is_empty() {
        return write_with_extras::<K>(enc, img, tags);
    }
    let (w, h) = img.dimensions();
    let u16s = || img.to_u16_samples().unwrap_or_default();
    let f32s = || img.to_f32_samples().unwrap_or_default();
    let d8 = img.data();
    match (img.layout(), img.sample_type()) {
        (L::Gray, S::U8) => write_one::<colortype::Gray8, K>(enc, w, h, d8, tags),
        (L::Gray, S::U16) => write_one::<colortype::Gray16, K>(enc, w, h, &u16s(), tags),
        (L::Gray, S::F32) => write_one::<colortype::Gray32Float, K>(enc, w, h, &f32s(), tags),
        (L::GrayA, S::U8) => write_one::<GrayA8, K>(enc, w, h, d8, tags),
        (L::GrayA, S::U16) => write_one::<GrayA16, K>(enc, w, h, &u16s(), tags),
        (L::GrayA, S::F32) => write_one::<GrayA32F, K>(enc, w, h, &f32s(), tags),
        (L::Rgb, S::U8) => write_one::<colortype::RGB8, K>(enc, w, h, d8, tags),
        (L::Rgb, S::U16) => write_one::<colortype::RGB16, K>(enc, w, h, &u16s(), tags),
        (L::Rgb, S::F32) => write_one::<colortype::RGB32Float, K>(enc, w, h, &f32s(), tags),
        (L::Rgba, S::U8) => write_one::<colortype::RGBA8, K>(enc, w, h, d8, tags),
        (L::Rgba, S::U16) => write_one::<colortype::RGBA16, K>(enc, w, h, &u16s(), tags),
        (L::Rgba, S::F32) => write_one::<colortype::RGBA32Float, K>(enc, w, h, &f32s(), tags),
        (L::Cmyk, S::U8) => write_one::<colortype::CMYK8, K>(enc, w, h, d8, tags),
        (L::Cmyk, S::U16) => write_one::<colortype::CMYK16, K>(enc, w, h, &u16s(), tags),
        (L::Cmyk, S::F32) => write_one::<colortype::CMYK32Float, K>(enc, w, h, &f32s(), tags),
        (L::CmykA, S::U8) => write_one::<colortype::CMYKA8, K>(enc, w, h, d8, tags),
        (L::CmykA, S::U16) => write_one::<CmykA16, K>(enc, w, h, &u16s(), tags),
        (L::CmykA, S::F32) => write_one::<CmykA32F, K>(enc, w, h, &f32s(), tags),
        (L::Lab | L::LabA, _) => {
            let file = lab_file(img);
            let u16s = || file.to_u16_samples().unwrap_or_default();
            match (file.layout(), file.sample_type()) {
                (L::Lab, S::U8) => write_one::<Lab8, K>(enc, w, h, file.data(), tags),
                (L::Lab, _) => write_one::<Lab16, K>(enc, w, h, &u16s(), tags),
                (_, S::U8) => write_one::<LabA8, K>(enc, w, h, file.data(), tags),
                _ => write_one::<LabA16, K>(enc, w, h, &u16s(), tags),
            }
        }
        (l, s) => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported {l:?} {s:?}")).into()),
    }
}

/// Classic TIFF offsets are 32-bit: a file that may grow past this is written as BigTIFF.
const CLASSIC_LIMIT: u64 = u32::MAX as u64;

/// An upper estimate of the file size: the uncompressed samples plus every tag payload plus
/// room for the directory and strip tables.
fn estimated_size(img: &Image, tags: &TagSet<'_>) -> u64 {
    let payloads = [tags.icc, tags.xmp, tags.photoshop_resources, tags.photoshop_layers].iter().flatten().map(|p| p.len() as u64).sum::<u64>();
    let text = tags.text.iter().map(|(_, s)| s.len() as u64).sum::<u64>()
        + tags.exif.iter().flat_map(|e| e.ifd0.iter().chain(&e.exif).chain(&e.gps)).map(|f| f.value.len() as u64 + 12).sum::<u64>();
    // Strip tables: one offset and one count per strip of about 1 MB.
    let tables = (img.data().len() as u64 / (1 << 20) + 1) * 16;
    let extras: u64 = img.meta.extra_samples.iter().map(|e| e.data.len() as u64).sum();
    (img.data().len() as u64).saturating_add(extras).saturating_add(payloads).saturating_add(text).saturating_add(tables).saturating_add(1 << 20)
}

/// The byte length of one extra-sample plane: one sample per pixel of `img`, in its sample type.
/// `None` when the pixel count overflows a `usize` (no such image buffer can exist).
fn extra_plane_len(img: &Image) -> Option<usize> {
    let (w, h) = img.dimensions();
    (w as usize).checked_mul(h as usize)?.checked_mul(img.sample_type().bytes())
}

/// Every extra plane must hold exactly one sample per pixel of `img`, in its sample type.
///
/// The writer copies one sample per pixel out of each plane, so a plane of any other length
/// would be silently truncated (long) or read past its end (short). A sample-type conversion
/// reinterprets the bytes in the new width instead, so an unchecked plane would also change
/// meaning rather than fail. Refuse the image instead of writing either.
fn check_extra_planes(img: &Image) -> Result<(), CodecError> {
    let (w, h) = img.dimensions();
    let sample = img.sample_type();
    let plane_len = extra_plane_len(img).ok_or_else(|| CodecError::encode(F, format!("the extra-sample size of a {w}x{h} image overflows")))?;
    for (k, e) in img.meta.extra_samples.iter().enumerate() {
        if e.data.len() != plane_len {
            return Err(CodecError::encode(
                F,
                format!("extra sample {k} is {} bytes; a {w}x{h} {sample:?} image needs exactly {plane_len} (one sample per pixel)", e.data.len()),
            ));
        }
    }
    Ok(())
}

pub(crate) fn encode(src: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    // Check the source planes before `converted`, which brings them to the plan's sample type
    // with the image but deliberately leaves a wrong-sized plane alone: past that point a bad
    // plane would be reinterpreted in the new width rather than rejected.
    check_extra_planes(src)?;
    // `converted` brings the extra samples to the plan's sample type with the image.
    encode_ready(&src.converted(plan.layout, plan.sample), opts)
}

fn encode_ready(img: &Image, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let compression = match opts.tiff_compression {
        TiffCompression::None => tiff::encoder::Compression::Uncompressed,
        TiffCompression::Lzw => tiff::encoder::Compression::Lzw,
        TiffCompression::Deflate => tiff::encoder::Compression::Deflate(tiff::encoder::DeflateLevel::Balanced),
        TiffCompression::PackBits => tiff::encoder::Compression::Packbits,
    };
    let predictor = if !img.sample_type().is_float() && matches!(opts.tiff_compression, TiffCompression::Lzw | TiffCompression::Deflate) {
        tiff::encoder::Predictor::Horizontal
    } else {
        tiff::encoder::Predictor::None
    };
    let text: Vec<(Tag, &str)> = if opts.embed_metadata {
        img.meta
            .text
            .iter()
            .filter_map(|(k, v)| TEXT_TAGS.iter().find(|(_, key)| key.eq_ignore_ascii_case(k)).map(|(t, _)| (*t, v.as_str())))
            .filter(|(_, v)| v.is_ascii() && !v.contains('\0'))
            .collect()
    } else {
        Vec::new()
    };
    // TIFF writes no Orientation tag (= 1): keep the XMP from contradicting it.
    let xmp = if opts.embed_metadata { img.meta.xmp.as_deref().map(|x| crate::resolution::export_xmp(x, img.meta.dpi)) } else { None };
    if img.meta.extra_samples.len() > crate::MAX_EXTRA_SAMPLES {
        return Err(CodecError::encode(F, format!("TIFF stores at most {} extra samples", crate::MAX_EXTRA_SAMPLES)));
    }
    // Validate again in the sample type the planes now hold: `Image::convert` takes a plane's
    // bytes before it rebuilds them, so one it could not rebuild would arrive here empty. The Lab
    // path's later re-type through `lab_file` converts a plane only when its length already
    // matches the image, which this guarantees.
    check_extra_planes(img)?;
    let mut kinds = Vec::with_capacity(img.meta.extra_samples.len().saturating_add(1));
    if img.layout().has_alpha() {
        kinds.push(2);
    }
    // TIFF 6.0 reserves ExtraSamples values above 2: those are written as 0 (unspecified).
    for e in &img.meta.extra_samples {
        kinds.push(if e.kind <= 2 { e.kind } else { 0 });
    }
    let tags = TagSet {
        icc: if opts.embed_icc { img.icc.as_deref() } else { None },
        exif: if opts.embed_metadata { img.meta.exif.as_deref().and_then(|e| tiff_exif::read(crate::orientation::tiff_body(e))) } else { None },
        xmp: xmp.as_deref().map(str::as_bytes),
        dpi: if opts.embed_metadata { img.meta.dpi.filter(|d| d.0 > 0.0 && d.1 > 0.0) } else { None },
        text,
        extra_samples: &kinds,
        // Document content, not metadata: written whenever present.
        photoshop_resources: img.meta.photoshop_resources.as_deref().filter(|r| !r.is_empty()),
        photoshop_layers: img.meta.photoshop_layers.as_deref().filter(|l| !l.is_empty()),
    };

    // BigTIFF when asked, or when the file could pass the 4 GiB a classic TIFF can address. A
    // classic write that still runs out of 32-bit offsets (compression grew the data) is redone
    // as BigTIFF.
    let write = |big: bool| -> tiff::TiffResult<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::new());
        if big {
            let mut enc = TiffEncoder::new_big(&mut cursor)?.with_compression(compression).with_predictor(predictor);
            write_image::<TiffKindBig>(&mut enc, img, &tags)?;
        } else {
            let mut enc = TiffEncoder::new(&mut cursor)?.with_compression(compression).with_predictor(predictor);
            write_image::<TiffKindStandard>(&mut enc, img, &tags)?;
        }
        Ok(cursor.into_inner())
    };
    let big = opts.tiff_bigtiff || estimated_size(img, &tags) > CLASSIC_LIMIT;
    match write(big) {
        Ok(v) => Ok(v),
        Err(tiff::TiffError::IntSizeError) if !big => write(true).map_err(|e| CodecError::encode(F, e)),
        Err(e) => Err(CodecError::encode(F, e)),
    }
}
