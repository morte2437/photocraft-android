//! Notes and Image › Analysis data ⇄ PSD.
//!
//! * Notes: the global additional-info block `Anno` ("Annotations (Photoshop 6.0)" in the Adobe
//!   PSD spec): `u16` major version 2, `u16` minor version 1, `u32` count, then per annotation a
//!   `u32` length (including itself), type `txtA` (text) or `sndA` (sound), open flag `u8`,
//!   flags `u8`, optional blocks `u16`, icon rect and popup rect (four `i32` each: top, left,
//!   bottom, right), a colour (`u16` space + four `u16`), author, name and modification date as
//!   even-padded Pascal strings, then a `u32` length (including itself) of the data block:
//!   `txtC`/`sndM`, `u32` size and the data. Text is UTF-16BE with a byte-order mark, lines
//!   separated by CR. Layout checked against `corpus/psd/ag-psd/read-write/annotations`.
//!   Sound annotations are not modelled; they are carried over verbatim when notes are rewritten.
//! * Measurement scale: image resource 1074 (a version-16 descriptor). The key names
//!   (`pixelLength`, `logicalLength`, `logicalUnits`) follow Photoshop's scripting names for the
//!   measurement scale; no sample file was available, so parsing is lenient and the raw resource
//!   is kept byte-exact while the scale is unchanged.
//! * Count information: image resource 1080 (a version-16 `Cnt ` descriptor) maps to the
//!   document's ordered Count groups. Unreadable resources are kept while there are no groups.

use std::sync::Arc;

use craft_psd::ImageResource;
use craft_psd::descriptor::{Descriptor, UnicodeString, Value, VersionedDescriptor};
use photocraft_doc::{Color, CountGroup, Document, MeasurementScale, Note};

pub const MEASUREMENT_SCALE: u16 = 1074;
pub const COUNT_INFO: u16 = 1080;
pub const ANNO: [u8; 4] = *b"Anno";

fn pascal_bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| if (c as u32) < 256 { c as u32 as u8 } else { b'?' }).take(255).collect()
}

/// PSD colour struct (space + 4 × u16) → RGB.
fn color_from(space: u16, c: [u16; 4]) -> Color {
    let f = |v: u16| f32::from(v) / 65535.0;
    match space {
        // HSB: hue 0–65535 maps to 0–360°.
        1 => {
            let (h, s, v) = (f(c[0]) * 6.0, f(c[1]), f(c[2]));
            let i = (h.floor() as i32).rem_euclid(6);
            let fr = h - h.floor();
            let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * fr), v * (1.0 - s * (1.0 - fr)));
            let (r, g, b) = match i {
                0 => (v, t, p),
                1 => (q, v, p),
                2 => (p, v, t),
                3 => (p, q, v),
                4 => (t, p, v),
                _ => (v, p, q),
            };
            Color::rgb(r, g, b)
        }
        // CMYK (inverted: 0 = 100 % ink), naive.
        2 => {
            let k = 1.0 - f(c[3]);
            Color::rgb(f(c[0]) * (1.0 - k), f(c[1]) * (1.0 - k), f(c[2]) * (1.0 - k))
        }
        // Grayscale 0–10000.
        8 => {
            let g = 1.0 - (f32::from(c[0]) / 10000.0).clamp(0.0, 1.0);
            Color::rgb(g, g, g)
        }
        _ => Color::rgb(f(c[0]), f(c[1]), f(c[2])),
    }
}

fn decode_text(data: &[u8]) -> String {
    let s = if data.len() >= 2 && data[0] == 0xFE && data[1] == 0xFF {
        let units: Vec<u16> = data[2..].as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        data.iter().map(|&c| c as char).collect()
    };
    s.trim_end_matches('\0').replace("\r\n", "\n").replace('\r', "\n")
}

fn encode_text(s: &str) -> Vec<u8> {
    let mut out = vec![0xFE, 0xFF];
    for u in s.replace("\r\n", "\n").replace('\n', "\r").encode_utf16() {
        out.extend_from_slice(&u.to_be_bytes());
    }
    out
}

/// One parsed `Anno` entry: a note, or a raw entry we keep (sound annotations).
enum Entry {
    Text(Note),
    Raw(Vec<u8>),
}

fn parse_entries(data: &[u8]) -> Option<Vec<Entry>> {
    let records = craft_psd::annotations::read(data, Default::default()).ok()?;
    records
        .into_iter()
        .map(|r| {
            let Some(t) = r.text().ok()? else {
                return Some(Entry::Raw(r.raw().to_vec()));
            };
            Some(Entry::Text(Note {
                author: t.author.iter().map(|&c| c as char).collect(),
                color: color_from(t.color_space, t.color),
                text: decode_text(t.data),
                position: [f64::from(t.icon[1]), f64::from(t.icon[0])],
                popup: [f64::from(t.popup[1]), f64::from(t.popup[0]), f64::from(t.popup[3]), f64::from(t.popup[2])],
                open: t.open != 0,
                modified: t.modified.iter().map(|&c| c as char).collect(),
            }))
        })
        .collect()
}

/// Notes in an `Anno` block (None when it doesn't parse).
pub fn parse_anno(data: &[u8]) -> Option<Vec<Note>> {
    Some(parse_entries(data)?.into_iter().filter_map(|e| if let Entry::Text(n) = e { Some(n) } else { None }).collect())
}

fn write_note(n: &Note) -> craft_psd::Result<Vec<u8>> {
    let limits = craft_psd::annotations::Limits::default();
    // Bound temporary text normalization before the shared fallible writer allocates.
    if n.text.len().checked_mul(2).and_then(|n| n.checked_add(2)).is_none_or(|n| n > limits.bytes) {
        return Err(craft_psd::PsdError::LimitExceeded("annotation text"));
    }
    let (x, y) = (n.position[0].round() as i32, n.position[1].round() as i32);
    let pp = if n.popup == [0.0; 4] { [n.position[0] + 20.0, n.position[1] + 20.0, n.position[0] + 260.0, n.position[1] + 160.0] } else { n.popup };
    let rgb = n.color.to_rgb();
    let author = pascal_bytes(&n.author);
    let modified = pascal_bytes(&n.modified);
    let text = encode_text(&n.text);
    craft_psd::annotations::write_text(
        &craft_psd::annotations::Text {
            open: u8::from(n.open),
            flags: 0x1C,
            optional_blocks: 1,
            icon: [y, x, y.saturating_add(20), x.saturating_add(16)],
            popup: [pp[1].round() as i32, pp[0].round() as i32, pp[3].round() as i32, pp[2].round() as i32],
            color_space: 0,
            color: [
                (rgb[0].clamp(0.0, 1.0) * 65535.0).round() as u16,
                (rgb[1].clamp(0.0, 1.0) * 65535.0).round() as u16,
                (rgb[2].clamp(0.0, 1.0) * 65535.0).round() as u16,
                0,
            ],
            author: &author,
            name: &[],
            modified: &modified,
            data_length: 0,
            data_key: *b"txtC",
            data: &text,
        },
        limits,
    )
}

/// An Anno block holding notes and byte-exact unknown records.
/// Errors propagate to export instead of silently dropping annotations.
pub fn write_anno(notes: &[Note], raw_extra: &[Vec<u8>]) -> craft_psd::Result<Vec<u8>> {
    let limits = craft_psd::annotations::Limits::default();
    if notes.len().checked_add(raw_extra.len()).is_none_or(|n| n > limits.records) {
        return Err(craft_psd::PsdError::LimitExceeded("annotation count"));
    }
    let mut owned = Vec::new();
    owned.try_reserve_exact(notes.len()).map_err(|_| craft_psd::PsdError::OutOfMemory("annotation records"))?;
    let mut bytes = 8usize;
    for n in notes {
        let record = write_note(n)?;
        bytes = bytes.checked_add(record.len()).filter(|&n| n <= limits.bytes).ok_or(craft_psd::PsdError::LimitExceeded("annotation bytes"))?;
        owned.push(record);
    }
    let refs: Vec<&[u8]> = owned.iter().chain(raw_extra).map(Vec::as_slice).collect();
    craft_psd::annotations::write(&refs, limits)
}

/// The document's notes from its raw global blocks (empty when there is no `Anno` block).
pub fn notes_from_blocks(doc: &Document) -> Vec<Note> {
    doc.metadata.psd_global_blocks.iter().find(|(_, k, _)| *k == ANNO).and_then(|(_, _, d)| parse_anno(d)).unwrap_or_default()
}

/// Whether the document holds an `Anno` block that doesn't parse (its notes can't be shown).
pub fn anno_unreadable(doc: &Document) -> bool {
    doc.metadata.psd_global_blocks.iter().find(|(_, k, _)| *k == ANNO).is_some_and(|(_, _, d)| parse_entries(d).is_none())
}

/// Global blocks to write: the raw `Anno` block while it still describes `doc.notes` (or doesn't
/// parse and no note was added), else a regenerated one in its place (appended when new, dropped
/// when the last note is deleted).
pub fn export_blocks(doc: &Document, mut blocks: Vec<photocraft_doc::PsdGlobalBlock>) -> Result<Vec<photocraft_doc::PsdGlobalBlock>, crate::IoError> {
    let at = blocks.iter().position(|(_, k, _)| *k == ANNO);
    let raw = at.map(|i| blocks[i].2.clone());
    let entries = raw.as_deref().and_then(|d| parse_entries(d));
    let parsed: Option<Vec<Note>> = entries.as_ref().map(|e| e.iter().filter_map(|e| if let Entry::Text(n) = e { Some(n.clone()) } else { None }).collect());
    // Compare in stored precision (colours are 16-bit, positions whole pixels, empty popups get a
    // default rectangle).
    let want = parse_anno(&write_anno(&doc.notes, &[]).map_err(|e| crate::IoError::Unsupported(e.to_string()))?);
    let same = |a: &[Note], b: &[Note]| {
        a.len() == b.len()
            && a.iter().zip(b).all(|(x, y)| {
                let (cx, cy) = (x.color.to_rgb(), y.color.to_rgb());
                Note { color: y.color, ..x.clone() } == *y && cx.iter().zip(cy).all(|(p, q)| (p - q).abs() < 1.0 / 512.0)
            })
    };
    // With no notes, a block we can't read is kept as is: it isn't "the last note deleted", and
    // dropping it would delete the annotations it holds (issue #944).
    if parsed.as_deref().zip(want.as_deref()).is_some_and(|(p, w)| same(p, w)) || (entries.is_none() && doc.notes.is_empty()) {
        return Ok(blocks);
    }
    let extra: Vec<Vec<u8>> = entries.unwrap_or_default().into_iter().filter_map(|e| if let Entry::Raw(r) = e { Some(r) } else { None }).collect();
    let fresh = if !doc.notes.is_empty() || !extra.is_empty() {
        Some(Arc::new(write_anno(&doc.notes, &extra).map_err(|e| crate::IoError::Unsupported(e.to_string()))?))
    } else {
        None
    };
    match (at, fresh) {
        (Some(i), Some(f)) => blocks[i].2 = f,
        (Some(i), None) => {
            blocks.remove(i);
        }
        (None, Some(f)) => blocks.push((*b"8BIM", ANNO, f)),
        (None, None) => {}
    }
    Ok(blocks)
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Double(d) => Some(*d),
        Value::Integer(i) => Some(f64::from(*i)),
        Value::LargeInteger(i) => Some(*i as f64),
        Value::UnitFloat { value, .. } => Some(*value),
        _ => None,
    }
}

fn find<'a>(d: &'a Descriptor, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| d.get(k)).or_else(|| {
        // The scale may be nested one object deep (`measurementScale` object).
        d.items.iter().find_map(|(_, v)| if let Value::Descriptor(o) = v { keys.iter().find_map(|k| o.get(k)) } else { None })
    })
}

/// Measurement scale described by resource 1074 (None when it doesn't parse).
pub fn scale_from_resource(data: &[u8]) -> Option<MeasurementScale> {
    let d = VersionedDescriptor::parse_prefix(data).ok()?.0.descriptor;
    let pixel_length = find(&d, &["pixelLength", "PxlL"]).and_then(num)?;
    let logical_length = find(&d, &["logicalLength", "LglL"]).and_then(num)?;
    let units = match find(&d, &["logicalUnits", "LglU"]) {
        Some(Value::Text(t)) => t.to_string_lossy(),
        _ => "pixels".into(),
    };
    Some(MeasurementScale { pixel_length, logical_length, units })
}

/// Resource 1074 data for a scale.
pub fn write_scale_resource(s: &MeasurementScale) -> Vec<u8> {
    let d = Descriptor::new("null")
        .with("pixelLength", Value::Double(s.pixel_length))
        .with("logicalLength", Value::Double(s.logical_length))
        .with("logicalUnits", Value::Text(craft_psd::descriptor::UnicodeString::new(&s.units)));
    let mut v = VersionedDescriptor::new(d).to_bytes();
    if v.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// The measurement scale a document's preserved resource 1074 holds (the default when absent).
pub fn raw_scale(doc: &Document) -> Option<MeasurementScale> {
    let raw = doc.metadata.psd_resources.iter().find(|(id, _, _)| *id == MEASUREMENT_SCALE)?;
    scale_from_resource(&raw.2)
}

/// Whether the preserved resource 1074 may be written as is: it still describes the document's
/// scale (or doesn't parse and the scale was never changed from the default).
pub fn keep_raw_scale(doc: &Document) -> bool {
    match raw_scale(doc) {
        Some(s) => s == doc.measurement.scale,
        None => doc.measurement.scale.is_default(),
    }
}

/// Resource 1074 to write when the preserved one can't be used (None = write nothing).
pub fn fresh_scale_resource(doc: &Document) -> Option<Vec<u8>> {
    let has_raw = doc.metadata.psd_resources.iter().any(|(id, _, _)| *id == MEASUREMENT_SCALE);
    if has_raw && keep_raw_scale(doc) {
        return None;
    }
    (!doc.measurement.scale.is_default()).then(|| write_scale_resource(&doc.measurement.scale))
}

/// Decode the measured resource as a whole so unknown or damaged groups stay opaque.
fn count_groups_from_resource(data: &[u8]) -> Option<Vec<CountGroup>> {
    let d = VersionedDescriptor::from_bytes(data).ok()?.descriptor;
    if !d.class_id.is("Cnt ") || d.items.len() != 2 || d.get("Vrsn") != Some(&Value::Integer(1)) {
        return None;
    }
    let Value::List(groups) = d.get("countGroupList")? else { return None };
    groups.iter().map(count_group_from_descriptor).collect()
}

/// Decode the fields the Count tool can use without coercing unrecognized data.
fn count_group_from_descriptor(value: &Value) -> Option<CountGroup> {
    let Value::Descriptor(d) = value else { return None };
    if !d.class_id.is("countGroup") || d.items.len() != 8 {
        return None;
    }
    let integer = |key| match d.get(key)? {
        Value::Integer(n) => u32::try_from(*n).ok(),
        _ => None,
    };
    let channel = |key| u8::try_from(integer(key)?).ok().map(|n| f32::from(n) / 255.0);
    let color = Color::rgb(channel("Rd  ")?, channel("Grn ")?, channel("Bl  ")?);
    let Value::Text(name) = d.get("Nm  ")? else { return None };
    let Value::Boolean(visible) = d.get("Vsbl")? else { return None };
    let Value::List(objects) = d.get("countObjectList")? else { return None };
    let marker_size = integer("Rds ")?;
    let label_size = integer("fontSize")?;
    // Keep out-of-range sizes opaque instead of passing them to the marker and text renderers.
    if !(1..=10).contains(&marker_size) || !(8..=72).contains(&label_size) {
        return None;
    }
    let name = String::from_utf16(name.0.strip_suffix(&[0]).unwrap_or(&name.0)).ok()?;
    let points = objects.iter().map(count_point_from_descriptor).collect::<Option<_>>()?;
    Some(CountGroup { name, color, marker_size, label_size, visible: *visible, points })
}

/// Keep document coordinates as doubles and reject nonfinite values.
fn count_point_from_descriptor(value: &Value) -> Option<[f64; 2]> {
    let Value::Descriptor(d) = value else { return None };
    if !d.class_id.is("countObject") || d.items.len() != 2 {
        return None;
    }
    let (Value::Double(x), Value::Double(y)) = (d.get("X   ")?, d.get("Y   ")?) else { return None };
    (x.is_finite() && y.is_finite()).then_some([*x, *y])
}

/// Consume readable Count data. Keep only opaque bytes when no resource can be read.
pub(crate) fn import_count_resource(doc: &mut Document) {
    if let Some(groups) = doc.metadata.psd_resources.iter().filter(|(id, _, _)| *id == COUNT_INFO).find_map(|(_, _, data)| count_groups_from_resource(data)) {
        doc.measurement.count_groups = groups;
        doc.metadata.psd_resources.retain(|(id, _, _)| *id != COUNT_INFO);
    }
}

/// Resource 1080 from the current groups, warning when PSD changes sizes or omits points.
pub(crate) fn fresh_count_resource(doc: &Document, warnings: &mut Vec<String>) -> Option<ImageResource> {
    if doc.measurement.count_groups.is_empty() {
        return None;
    }
    let groups = doc.measurement.count_groups.iter().map(|group| count_group_descriptor(group, warnings)).collect();
    let d = Descriptor::new("Cnt ").with("Vrsn", Value::Integer(1)).with("countGroupList", Value::List(groups));
    Some(ImageResource::new(COUNT_INFO, VersionedDescriptor::new(d).to_bytes()))
}

/// Write the measured group layout and preserve marker order.
fn count_group_descriptor(group: &CountGroup, warnings: &mut Vec<String>) -> Value {
    let marker_size = group.marker_size.clamp(1, 10);
    let label_size = group.label_size.clamp(8, 72);
    if marker_size != group.marker_size || label_size != group.label_size {
        warnings.push(format!(
            "Count group {:?} sizes adjusted for PSD (marker {} to {marker_size}, label {} to {label_size})",
            group.name, group.marker_size, group.label_size
        ));
    }
    let [r, g, b, _] = group.color.to_rgba8();
    let objects: Vec<_> = group
        .points
        .iter()
        .filter(|[x, y]| x.is_finite() && y.is_finite())
        .map(|&[x, y]| Value::Descriptor(Descriptor::new("countObject").with("X   ", Value::Double(x)).with("Y   ", Value::Double(y))))
        .collect();
    let omitted = group.points.len() - objects.len();
    if omitted != 0 {
        warnings.push(format!("Count group {:?} had nonfinite points omitted from PSD export ({omitted} total)", group.name));
    }
    Value::Descriptor(
        Descriptor::new("countGroup")
            .with("Rd  ", Value::Integer(i32::from(r)))
            .with("Grn ", Value::Integer(i32::from(g)))
            .with("Bl  ", Value::Integer(i32::from(b)))
            .with("Nm  ", Value::Text(UnicodeString::new_nul(&group.name)))
            .with("Rds ", Value::Integer(marker_size as i32))
            .with("fontSize", Value::Integer(label_size as i32))
            .with("Vsbl", Value::Boolean(group.visible))
            .with("countObjectList", Value::List(objects)),
    )
}

/// Whether to keep a preserved resource with this id on export.
pub fn keep_resource(doc: &Document, id: u16) -> bool {
    match id {
        MEASUREMENT_SCALE => keep_raw_scale(doc),
        // An opaque copy is only used when there are no document groups to write.
        COUNT_INFO => doc.measurement.count_groups.is_empty(),
        _ => true,
    }
}

#[cfg(test)]
#[path = "count_resource_tests.rs"]
mod count_resource_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "corpus")]
    const SAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/psd/ag-psd/read-write/annotations/src.psd");

    #[cfg(feature = "corpus")]
    fn anno_of(bytes: &[u8]) -> Option<Vec<u8>> {
        let i = bytes.windows(8).position(|w| w == b"8BIMAnno")?;
        let n = u32::from_be_bytes(bytes[i + 8..i + 12].try_into().ok()?) as usize;
        Some(bytes[i + 12..i + 12 + n].to_vec())
    }

    #[cfg(feature = "corpus")]
    #[test]
    fn parses_and_rewrites_the_sample_byte_exact() {
        let file = std::fs::read(SAMPLE).unwrap_or_else(|e| panic!("{SAMPLE}: {e}: run `cargo xtask corpus --all`"));
        let raw = anno_of(&file).unwrap();
        let notes = parse_anno(&raw).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].author, "someone");
        assert_eq!(notes[0].text, "Example annotation\nis here\n\n~ annotator\n");
        assert_eq!(notes[0].position, [42.0, 28.0]);
        assert_eq!(notes[0].popup, [92.0, 74.0, 332.0, 214.0]);
        assert_eq!(notes[0].modified, "D:20210521231029+01'00'");
        assert!(notes[0].open);
        // Second note's colour is stored as HSB.
        assert_eq!(notes[1].author, "some author");
        assert_eq!(notes[1].text, "open note");
        let rgb = notes[1].color.to_rgb();
        // Hue 343°, saturation 0.49, brightness 1: pink.
        assert!(rgb[0] > 0.99 && rgb[1] < rgb[2] && rgb[2] < 0.7, "{rgb:?}");
        // Text, positions, flags and RGB colours are rewritten exactly.
        let again = write_anno(&notes[..1], &[]).unwrap();
        assert_eq!(parse_anno(&again).unwrap()[..], notes[..1]);
        let n0 = 8 + u32::from_be_bytes(raw[8..12].try_into().unwrap()) as usize;
        assert_eq!(&again[8..], &raw[8..n0]);
    }

    #[test]
    fn export_blocks_keeps_replaces_and_drops() {
        let mut d = Document::new("n", photocraft_doc::Size::new(10, 10), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        assert!(export_blocks(&d, vec![]).unwrap().is_empty());
        d.notes.push(Note { text: "héllo ✓".into(), author: "me".into(), position: [3.0, 4.0], ..Default::default() });
        let out = export_blocks(&d, vec![]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(parse_anno(&out[0].2).unwrap()[0].text, "héllo ✓");
        // Unchanged → the very same bytes.
        d.metadata.psd_global_blocks = out.clone();
        assert!(Arc::ptr_eq(&export_blocks(&d, out.clone()).unwrap()[0].2, &out[0].2));
        d.notes.clear();
        assert!(export_blocks(&d, out).unwrap().is_empty());
        assert!(parse_anno(&[0, 2, 0, 1, 0, 0, 0, 9]).is_none());
    }

    /// Issue #944: an `Anno` block that doesn't parse was taken for "no notes" and dropped on save.
    #[test]
    fn export_blocks_keeps_an_unreadable_block() {
        let mut d = Document::new("n", photocraft_doc::Size::new(10, 10), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let bad: Vec<photocraft_doc::PsdGlobalBlock> = vec![(*b"8BIM", ANNO, Arc::new(vec![0, 2, 0, 1, 0, 0, 0, 9]))];
        d.metadata.psd_global_blocks = bad.clone();
        assert!(anno_unreadable(&d) && notes_from_blocks(&d).is_empty());
        let out = export_blocks(&d, bad.clone()).unwrap();
        assert_eq!(out.len(), 1);
        assert!(Arc::ptr_eq(&out[0].2, &bad[0].2));
        // A note added by the user still writes a fresh block in its place.
        d.notes.push(Note { text: "new".into(), ..Default::default() });
        let out = export_blocks(&d, bad).unwrap();
        assert_eq!(parse_anno(&out[0].2).unwrap()[0].text, "new");
        // A readable block is not reported.
        d.metadata.psd_global_blocks = out;
        assert!(!anno_unreadable(&d));
    }

    /// An entry length below 8 (its own `u32` plus the type) put the cursor back on the same
    /// entry, so a count of 0xFFFFFFFF looped ~4.3e9 times and ran out of memory.
    #[test]
    fn short_entry_length_is_unreadable_not_a_loop() {
        for len in [0u8, 4, 7] {
            for kind in [b"sndA", b"txtA"] {
                let mut b = vec![0, 2, 0, 1, 0, 0, 0, 3, 0, 0, 0, len];
                b.extend_from_slice(kind);
                b.extend_from_slice(&[0; 64]);
                assert!(parse_entries(&b).is_none(), "len {len} {kind:?}");
            }
        }
        let hostile = [0, 2, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, b's', b'n', b'd', b'A'];
        assert!(parse_anno(&hostile).is_none());
    }

    #[test]
    fn scale_resource_roundtrip_and_keep_rules() {
        let s = MeasurementScale { pixel_length: 150.0, logical_length: 2.0, units: "mm".into() };
        assert_eq!(scale_from_resource(&write_scale_resource(&s)), Some(s.clone()));
        let mut d = Document::new("n", photocraft_doc::Size::new(10, 10), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        assert!(fresh_scale_resource(&d).is_none());
        // An unparseable resource stays while the scale is untouched.
        d.metadata.psd_resources.push((MEASUREMENT_SCALE, String::new(), Arc::new(vec![0, 0, 0, 16, 1])));
        assert!(keep_resource(&d, MEASUREMENT_SCALE) && fresh_scale_resource(&d).is_none());
        d.measurement.scale = s.clone();
        assert!(!keep_resource(&d, MEASUREMENT_SCALE));
        assert_eq!(scale_from_resource(&fresh_scale_resource(&d).unwrap()), Some(s));
        assert!(keep_resource(&d, COUNT_INFO));
        d.measurement.count_groups.push(photocraft_doc::CountGroup { points: vec![[1.0, 1.0]], ..Default::default() });
        assert!(!keep_resource(&d, COUNT_INFO));
    }
}

#[cfg(test)]
#[path = "annotation_shared_tests.rs"]
mod shared_tests;
