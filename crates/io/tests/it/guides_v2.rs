//! Image resources 1092 (guides with colours) and 1097 (guide GUIDs) next to 1032 (#5966).

use photocraft_io::guides_map::{DEFAULT_GUIDE_RGB, GUIDE_IDS, GUIDES_V2};
use photocraft_io::*;
use photocraft_psd::testgen;
use photocraft_psd::{Compression, ImageResource, PsdFile, Version};

const RED: [u8; 3] = [0xff, 0x00, 0x00];

type Edit = fn(&mut photocraft_doc::Document);

fn guides_v1(guides: &[(i32, u8)]) -> Vec<u8> {
    let mut v = Vec::new();
    for n in [1u32, 576, 576, guides.len() as u32] {
        v.extend_from_slice(&n.to_be_bytes());
    }
    for (loc, dir) in guides {
        v.extend_from_slice(&loc.to_be_bytes());
        v.push(*dir);
    }
    v
}

fn guides_v2(guides: &[(i32, u8, [u8; 3])]) -> Vec<u8> {
    let mut v = Vec::new();
    for n in [2u32, 576, 576, guides.len() as u32] {
        v.extend_from_slice(&n.to_be_bytes());
    }
    for (loc, dir, rgb) in guides {
        v.extend_from_slice(&loc.to_be_bytes());
        v.extend_from_slice(&[*dir, 0]);
        v.extend_from_slice(rgb);
    }
    v
}

fn guide_ids(n: u32) -> Vec<u8> {
    let mut v = n.to_be_bytes().to_vec();
    for i in 0..n {
        let id = format!("0000000{i}-1111-2222-3333-444444444444");
        let zero = "00000000-0000-0000-0000-000000000000";
        v.extend_from_slice(&36u32.to_be_bytes());
        v.extend_from_slice(id.as_bytes());
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&36u32.to_be_bytes());
        v.extend_from_slice(zero.as_bytes());
    }
    v
}

/// A file as Photoshop writes it: a vertical guide at 16 px in the default colour and a red
/// horizontal one at 10 px, in 1032, 1092 and 1097.
fn photoshop_file() -> PsdFile {
    let mut f = testgen::small(Version::Psd, Compression::Raw);
    f.resources.push(ImageResource::new(1032, guides_v1(&[(512, 0), (320, 1)])));
    f.resources.push(ImageResource::new(GUIDES_V2, guides_v2(&[(512, 0, DEFAULT_GUIDE_RGB), (320, 1, RED)])));
    f.resources.push(ImageResource::new(GUIDE_IDS, guide_ids(2)));
    f
}

fn resource(f: &PsdFile, id: u16) -> Option<&[u8]> {
    f.resources.iter().find(|r| r.id == id).map(|r| r.data.as_slice())
}

#[test]
fn colours_come_from_guides_v2() {
    let (d, _) = psd_to_document(&photoshop_file());
    assert_eq!(d.guides.vertical, [16.0]);
    assert_eq!(d.guides.horizontal, [10.0]);
    assert_eq!(d.guides.color(false, 10.0), Some(RED));
    // The default colour stays with the preference.
    assert_eq!(d.guides.color(true, 16.0), None);
}

#[test]
fn unchanged_guides_keep_both_resources_byte_exact() {
    let f = photoshop_file();
    let (d, _) = psd_to_document(&f);
    let out = document_to_psd(&d);
    assert_eq!(resource(&out, GUIDES_V2), resource(&f, GUIDES_V2));
    assert_eq!(resource(&out, GUIDE_IDS), resource(&f, GUIDE_IDS));
}

#[test]
fn edited_guides_drop_both_resources() {
    let (base, _) = psd_to_document(&photoshop_file());
    let edits: [(&str, Edit); 4] = [
        ("clear", |d| d.guides = Default::default()),
        ("add", |d| d.guides.horizontal.push(40.0)),
        ("move", |d| {
            d.guides.vertical[0] = 20.0;
        }),
        ("recolour", |d| d.guides.set_color(true, 16.0, Some(RED))),
    ];
    for (name, edit) in edits {
        let mut d = base.clone();
        edit(&mut d);
        let out = document_to_psd(&d);
        assert!(resource(&out, GUIDES_V2).is_none(), "{name}: stale 1092 written");
        assert!(resource(&out, GUIDE_IDS).is_none(), "{name}: stale 1097 written");
        let (back, _) = psd_to_document(&out);
        assert_eq!(back.guides.vertical, d.guides.vertical, "{name}");
        assert_eq!(back.guides.horizontal, d.guides.horizontal, "{name}");
    }
}

#[test]
fn mismatched_ids_or_malformed_v2_are_not_kept() {
    let mut f = photoshop_file();
    f.resources.retain(|r| r.id != GUIDE_IDS);
    f.resources.push(ImageResource::new(GUIDE_IDS, guide_ids(3)));
    let (d, _) = psd_to_document(&f);
    let out = document_to_psd(&d);
    assert!(resource(&out, GUIDES_V2).is_some());
    assert!(resource(&out, GUIDE_IDS).is_none());

    let mut f = photoshop_file();
    for r in &mut f.resources {
        if r.id == GUIDES_V2 {
            r.data.truncate(20);
        }
    }
    let (d, _) = psd_to_document(&f);
    assert_eq!(d.guides.color(false, 10.0), None);
    let out = document_to_psd(&d);
    assert!(resource(&out, GUIDES_V2).is_none());
    assert!(resource(&out, GUIDE_IDS).is_none());
}
