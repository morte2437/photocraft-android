//! Opening and re-saving a Photoshop 16/32-bit file whose merged transparency is marked only by
//! its name keeps the channel count: the "Transparency" channel is the composite's alpha, not a
//! user alpha channel that the next save writes a second time (#4654).
//!
//! Photoshop's 16-bit PSB in the report has four channels, a positive layer count in `Lr16`, no
//! `Mt16` block, and one alpha channel name, "Transparency". Re-saving it wrote five channels.

use crate::common;

use common::raster;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::{Rect, Size};
use photocraft_io::*;
use photocraft_psd::PsdFile;

/// The unicode alpha channel names (resource 1045), empty without it.
fn names(file: &PsdFile) -> Vec<String> {
    let Some(data) = file.resources.iter().find(|r| r.id == 1045).map(|r| &r.data) else { return Vec::new() };
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 <= data.len() {
        let n = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        let units: Vec<u16> = data[at + 4..at + 4 + n * 2].chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        out.push(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string());
        at += 4 + n * 2;
    }
    out
}

#[test]
fn resave_keeps_a_name_marked_merged_transparency() {
    for depth in [SampleType::U16, SampleType::F32] {
        for path in ["x.psd", "x.psb"] {
            let case = format!("{depth:?} {path}");
            let mut doc = Document::new("t", Size::new(8, 6), ColorMode::Rgb, depth);
            let fmt = doc.pixel_format();
            doc.layers.push(raster("shape", fmt, Rect::new(1, 1, 5, 4), 3, true));
            let ours = PsdFile::from_bytes(&export(&doc, path, &ExportOptions::default()).expect("export").bytes).expect("parse");
            assert_eq!(ours.header.channels, 4, "{case}: colour + merged transparency");

            // Photoshop's form: a positive layer count, no `Mt16`/`Mt32` marker, and the merged
            // transparency named in the alpha channel names.
            let mut ps = ours.clone();
            ps.layer_info.as_mut().expect("layers").merged_alpha = false;
            ps.global_blocks.retain(|b| !b.is_merged_transparency_marker());
            assert!(!ps.merged_has_alpha(), "{case}: nothing marks it yet");
            let name: Vec<u8> = [0u8, 0, 0, 13].into_iter().chain("Transparency\0".encode_utf16().flat_map(u16::to_be_bytes)).collect();
            ps.resources.retain(|r| !matches!(r.id, 1006 | 1045));
            ps.resources.push(photocraft_psd::ImageResource::new(1045, name));
            assert!(ps.merged_has_alpha(), "{case}: the name marks the merged transparency");

            let opened = import(path, &ps.to_bytes().expect("write")).expect("import").document;
            let user: Vec<_> = opened.channels.iter().map(|c| c.name.as_str()).collect();
            assert!(user.is_empty(), "{case}: no user alpha channel from the transparency: {user:?}");

            let resaved = PsdFile::from_bytes(&export(&opened, path, &ExportOptions::default()).expect("re-export").bytes).expect("reparse");
            assert_eq!(resaved.header.channels, 4, "{case}: a save with no edits keeps the channel count");
            let resaved_names = names(&resaved);
            assert!(resaved_names.iter().filter(|n| *n == "Transparency").count() <= 1, "{case}: no duplicate Transparency: {resaved_names:?}");
        }
    }
}
