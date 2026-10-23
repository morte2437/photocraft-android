//! `type.create` accepts an exact PostScript face name in its font field.

use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::Session;
use serde_json::json;

fn font_with_postscript_name(source: &[u8], postscript: &str) -> Vec<u8> {
    let mut bytes = source.to_vec();
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let name_record = (0..count).map(|i| 12 + i * 16).find(|r| &bytes[*r..r + 4] == b"name").unwrap();
    for i in 0..count {
        let r = 12 + i * 16;
        let offset = u32::from_be_bytes(bytes[r + 8..r + 12].try_into().unwrap()) as usize;
        if &bytes[r..r + 4] == b"OS/2" {
            bytes[offset + 4..offset + 6].copy_from_slice(&900u16.to_be_bytes());
        }
    }
    let names = [(1u16, "PhotoCraft Black Test"), (2, "Black"), (6, postscript), (16, "PhotoCraft Black Test"), (17, "Black")];
    let mut table = Vec::new();
    let mut strings = Vec::new();
    for value in [0u16, names.len() as u16, (6 + names.len() * 12) as u16] {
        table.extend(value.to_be_bytes());
    }
    for (id, value) in names {
        let encoded: Vec<_> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for n in [3u16, 1, 0x409, id, encoded.len() as u16, strings.len() as u16] {
            table.extend(n.to_be_bytes());
        }
        strings.extend(encoded);
    }
    table.extend(strings);
    let offset = bytes.len() as u32;
    bytes[name_record + 8..name_record + 12].copy_from_slice(&offset.to_be_bytes());
    bytes[name_record + 12..name_record + 16].copy_from_slice(&(table.len() as u32).to_be_bytes());
    bytes.extend(table);
    bytes
}

#[test]
fn type_create_resolves_exact_postscript_font_names() {
    let ps = "PhotoCraft-Black-Test";
    photocraft_text::shared()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .fonts
        .register_font_data(font_with_postscript_name(photocraft_text::fonts::INTER_REGULAR.as_slice(), ps));

    let mut session = Session::new();
    session.execute("file.new", json!({"width": 200, "height": 100, "background": "transparent"})).unwrap();
    let response = session.execute("type.create", json!({"text": "Test", "font": ps})).unwrap();
    assert!(response["warnings"].as_array().is_none_or(Vec::is_empty), "{response}");
    let id = LayerId(response["layer"].as_u64().unwrap());
    let LayerContent::Text(text) = &session.active().unwrap().doc.layer(id).unwrap().content else { panic!("not a text layer") };
    let style = &text.char_runs()[0].style;
    assert_eq!(style.font_family, "PhotoCraft Black Test");
    assert_eq!(style.postscript_name.as_deref(), Some(ps));
    assert_eq!(style.weight, 900);

    let regular = session.execute("type.create", json!({"text": "Test", "font": "Inter"})).unwrap();
    assert!(regular["warnings"].as_array().is_none_or(Vec::is_empty), "{regular}");
    let missing = session.execute("type.create", json!({"text": "Test", "font": "No Such Family 123"})).unwrap();
    assert_eq!(missing["warnings"], json!(["font 'No Such Family 123' is not installed; text may render using a fallback face"]));
}

#[test]
fn exact_postscript_face_clears_retained_weight_variation() {
    let ps = "PhotoCraft-Black-Test-Var";
    photocraft_text::shared()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .fonts
        .register_font_data(font_with_postscript_name(photocraft_text::fonts::INTER_REGULAR.as_slice(), ps));

    let mut session = Session::new();
    session.execute("file.new", json!({"width": 200, "height": 100, "background": "transparent"})).unwrap();
    let response = session.execute("type.create", json!({"text": "Test", "font": "Inter", "variations": {"wght": 300}})).unwrap();
    let id = LayerId(response["layer"].as_u64().unwrap());

    // Setting an exact PostScript face should clear the previously retained wght variation
    session.execute("type.setStyle", json!({"layer": id.0, "font": ps})).unwrap();
    let LayerContent::Text(text) = &session.active().unwrap().doc.layer(id).unwrap().content else { panic!("not a text layer") };
    let style = &text.char_runs()[0].style;
    assert_eq!(style.font_family, "PhotoCraft Black Test");
    assert_eq!(style.weight, 900);
    assert!(!style.variations.iter().any(|v| v.axis == "wght"));

    // Supplying a variations.wght in the same call should apply after clearing
    session.execute("type.setStyle", json!({"layer": id.0, "font": ps, "variations": {"wght": 700}})).unwrap();
    let LayerContent::Text(text2) = &session.active().unwrap().doc.layer(id).unwrap().content else { panic!("not a text layer") };
    let style2 = &text2.char_runs()[0].style;
    assert_eq!(style2.font_family, "PhotoCraft Black Test");
    assert_eq!(style2.weight, 900);
    assert_eq!(style2.variations.iter().find(|v| v.axis == "wght").map(|v| v.value), Some(700.0));
}
