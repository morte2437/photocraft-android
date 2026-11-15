//! The exported CUBE file's comment layout and 1/32768 steps (#13750).

use super::*;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    s
}

fn cube(s: &mut Session, size: u32) -> String {
    s.execute("file.export.colorLookupTables", json!({"size": size, "title": "My Look"})).unwrap()["cube"].as_str().unwrap().to_string()
}

fn data_lines(cube: &str) -> Vec<&str> {
    cube.lines().filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_digit())).collect()
}

fn values(cube: &str) -> Vec<String> {
    data_lines(cube).iter().flat_map(|l| l.split(' ').map(str::to_string)).collect()
}

#[test]
fn header_follows_the_reference_comment_layout() {
    let mut s = session(8);
    let text = cube(&mut s, 3);
    let head: Vec<&str> = text.lines().take(12).collect();
    assert_eq!(
        head,
        [
            "#Created by: PhotoCraft",
            "TITLE \"My Look\"",
            "",
            "#LUT size",
            "LUT_3D_SIZE 3",
            "",
            "#data domain",
            "DOMAIN_MIN 0.0 0.0 0.0",
            "DOMAIN_MAX 1.0 1.0 1.0",
            "",
            "#LUT data points",
            data_lines(&text)[0],
        ]
    );
    assert_eq!(text.lines().count(), 11 + 27);
}

#[test]
fn eight_and_sixteen_bit_values_sit_on_1_32768_steps() {
    for depth in [8, 16] {
        let mut s = session(depth);
        // Inverted size-7 lattice: 1 - k/6 gives 5/6 and 1/6.
        let v = values(&cube(&mut s, 7));
        assert!(v.contains(&"0.833344".to_string()), "depth {depth}");
        assert!(v.contains(&"0.166656".to_string()), "depth {depth}");
        assert!(!v.contains(&"0.833333".to_string()), "depth {depth}");
        // Inverted size-52 lattice: 1 - 19/51 = 32/51 = 160/255.
        let v = values(&cube(&mut s, 52));
        assert!(v.contains(&"0.627441".to_string()), "depth {depth}");
        for x in v {
            let f: f64 = x.parse().unwrap();
            assert!((f * 32768.0 - (f * 32768.0).round()).abs() < 32768.0 * 1e-6 + 1e-9, "{x}");
        }
    }
}

#[test]
fn thirty_two_bit_values_are_not_quantised() {
    let mut s = session(32);
    let v = values(&cube(&mut s, 7));
    assert!(v.contains(&"0.833333".to_string()));
    assert!(v.contains(&"0.166667".to_string()));
    assert!(!v.contains(&"0.833344".to_string()));
}

#[test]
fn exported_text_parses_back_with_the_same_values() {
    for depth in [8, 32] {
        let mut s = session(depth);
        let text = cube(&mut s, 5);
        let lut = craft_cms::lutfile::parse_cube(&text).unwrap();
        assert_eq!(lut.size, 5);
        assert_eq!(lut.title, "My Look");
        let written: Vec<f32> = values(&text).iter().map(|x| x.parse().unwrap()).collect();
        assert_eq!(lut.data.len(), written.len());
        for (a, b) in lut.data.iter().zip(&written) {
            assert!((a - b).abs() < 1e-6, "{a} vs {b}");
        }
    }
}
