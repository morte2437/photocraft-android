use super::*;

use std::collections::{HashMap, HashSet};

#[test]
fn packed_tile_coordinates_round_trip_without_collisions() {
    let values = [i32::MIN, i32::MIN + 1, -1_000_003, -1, 0, 1, 1_000_003, i32::MAX - 1, i32::MAX];
    let coords: Vec<_> = values.iter().flat_map(|&x| values.iter().map(move |&y| (x, y))).collect();
    let keys: HashSet<_> = coords.iter().copied().map(tile_key).collect();
    assert_eq!(keys.len(), coords.len());
    for coord in coords {
        assert_eq!(tile_coords(tile_key(coord)), coord);
    }
}

struct DabInput {
    rect: Rect,
    vals: Vec<f32>,
    ceil: f32,
    color: Option<Vec<f32>>,
    depth: Option<Vec<f32>>,
}

fn serial(map: &mut CoverageMap, dab: &DabInput) {
    if dab.rect.is_empty() {
        return;
    }
    map.bounds = map.bounds.union(&dab.rect);
    let (x0, y0) = (dab.rect.x0.div_euclid(COV_TILE), dab.rect.y0.div_euclid(COV_TILE));
    let (x1, y1) = ((dab.rect.x1 - 1).div_euclid(COV_TILE), (dab.rect.y1 - 1).div_euclid(COV_TILE));
    let w = dab.rect.width() as usize;
    let values = DabValues { vals: &dab.vals, rect: dab.rect, w, ceil: dab.ceil, color: dab.color.as_deref(), depth: dab.depth.as_deref(), nc: map.nc };
    let keys = (y0..=y1).flat_map(|ty| {
        (x0..=x1).map(move |tx| {
            let tr = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE).intersect(&dab.rect);
            ((tx, ty), tr)
        })
    });
    map.accumulate_serial(keys.filter(|(_, tr)| values.touches(*tr)), values);
}

fn pixel_index(rect: Rect, x: i32, y: i32) -> usize {
    (y - rect.y0) as usize * rect.width() as usize + (x - rect.x0) as usize
}

fn make_dab(rect: Rect, seed: usize, color: bool, depth: bool) -> DabInput {
    let w = rect.width() as usize;
    let h = rect.height() as usize;
    let n = w * h;
    let vals = (0..n).map(|i| if (i + seed).is_multiple_of(11) { 0.0 } else { (((i * 7 + seed * 3) % 13) as f32 + 1.0) / 17.0 }).collect();
    let colors = color.then(|| vec![seed as f32 / 10.0, 0.75]);
    let depths = depth.then(|| (0..n).map(|i| ((i * 3 + seed * 2) % 17) as f32 / 16.0).collect());
    DabInput { rect, vals, ceil: if seed.is_multiple_of(2) { 0.72 } else { 0.93 }, color: colors, depth: depths }
}

#[derive(Default)]
struct OraclePixel {
    cov: f32,
    color: [f32; 2],
    has_color: bool,
    depth: Option<f32>,
}

fn apply_oracle(oracle: &mut HashMap<(i32, i32), OraclePixel>, dab: &DabInput) {
    for y in dab.rect.y0..dab.rect.y1 {
        for x in dab.rect.x0..dab.rect.x1 {
            let i = pixel_index(dab.rect, x, y);
            let v = dab.vals[i];
            if v <= 0.0 {
                continue;
            }
            let p = oracle.entry((x, y)).or_default();
            if let Some(depth) = dab.depth.as_ref().and_then(|d| d.get(i)) {
                p.depth = Some(p.depth.map_or(*depth, |old| old.max(*depth)));
            }
            let old = p.cov;
            let new = if dab.ceil > old { old + v * (dab.ceil - old) } else { old };
            if new > old {
                if let Some(color) = &dab.color {
                    let k = (new - old) / (1.0 - old);
                    for (channel, out) in p.color.iter_mut().enumerate() {
                        *out = color[channel] * k + *out * (1.0 - k);
                    }
                    p.has_color = true;
                }
                p.cov = new;
            }
        }
    }
}

#[test]
fn overlapping_accumulation_matches_dense_pixel_oracle_and_serial_path() {
    let mut zero_only = make_dab(Rect::new(384, 0, 386, 2), 6, true, true);
    zero_only.vals.fill(0.0);
    let dabs = [
        make_dab(Rect::new(-67, -3, 68, 67), 1, false, false),   // crosses negative and positive tile edges
        make_dab(Rect::new(-70, -130, 260, 195), 2, true, true), // lazily enables colour and depth across more tiles
        make_dab(Rect::new(62, 60, 70, 68), 3, true, true),
        make_dab(Rect::new(-70, 64, -63, 70), 4, false, true), // depth continues after coverage ceiling
        make_dab(Rect::new(192, 0, 193, 1), 5, true, true),    // revisit an existing tile
        zero_only,                                             // all-zero tile must be skipped
    ];
    let mut actual = CoverageMap::new(2);
    let mut expected_serial = CoverageMap::new(2);
    let mut oracle = HashMap::new();
    let mut bounds = Rect::default();
    for dab in &dabs {
        actual.accumulate(dab.rect, &dab.vals, dab.ceil, dab.color.as_deref(), dab.depth.as_deref());
        serial(&mut expected_serial, dab);
        apply_oracle(&mut oracle, dab);
        bounds = bounds.union(&dab.rect);
    }

    assert_eq!(actual.bounds(), bounds);
    assert_eq!(actual.bounds(), expected_serial.bounds());
    let expected_dirty: HashSet<_> = expected_serial.dirty.iter().copied().collect();
    assert_eq!(actual.take_dirty().into_iter().collect::<HashSet<_>>(), expected_dirty);
    assert_eq!(expected_serial.take_dirty().into_iter().collect::<HashSet<_>>(), expected_dirty);

    let tile_coords_actual: HashSet<_> = actual.tiles.keys().copied().map(tile_coords).collect();
    assert_eq!(tile_coords_actual, expected_dirty);
    assert!(!tile_coords_actual.contains(&(6, 0))); // the zero-only tile was never allocated
    assert!(tile_coords_actual.len() >= 16);

    for y in -130..195 {
        for x in -70..260 {
            let i = pixel_index_for_tile(x, y);
            let tx = x.div_euclid(COV_TILE);
            let ty = y.div_euclid(COV_TILE);
            let a = actual.tiles.get(&tile_key((tx, ty)));
            let b = expected_serial.tiles.get(&tile_key((tx, ty)));
            if let (Some(a), Some(b)) = (a, b) {
                assert_eq!(a.cov[i].to_bits(), b.cov[i].to_bits(), "coverage at ({x}, {y})");
                assert_eq!(a.depth.get(i).map(|v| v.to_bits()), b.depth.get(i).map(|v| v.to_bits()), "depth at ({x}, {y})");
                assert_eq!(
                    a.col.get(i * 2..i * 2 + 2).map(|s| [s[0].to_bits(), s[1].to_bits()]),
                    b.col.get(i * 2..i * 2 + 2).map(|s| [s[0].to_bits(), s[1].to_bits()]),
                    "color at ({x}, {y})"
                );
            } else {
                assert!(a.is_none() && b.is_none());
            }
            let p = oracle.get(&(x, y));
            let expected_cov = p.map_or(0.0, |p| p.cov);
            assert_eq!(actual.get(x, y).to_bits(), expected_cov.to_bits(), "dense oracle coverage at ({x}, {y})");
            let expected_depth = p.and_then(|p| p.depth).unwrap_or(0.0);
            assert_eq!(actual.depth(x, y).unwrap_or(0.0).to_bits(), expected_depth.to_bits(), "dense oracle depth at ({x}, {y})");
            if let Some(p) = p.filter(|p| p.has_color) {
                let color = &actual.tiles[&tile_key((tx, ty))].col;
                assert_eq!(color[i * 2].to_bits(), p.color[0].to_bits(), "dense oracle color at ({x}, {y})");
                assert_eq!(color[i * 2 + 1].to_bits(), p.color[1].to_bits(), "dense oracle color at ({x}, {y})");
            }
        }
    }

    // Depth still records a later dab even where coverage already reached its ceiling.
    let (x, y) = (-67, 64);
    let mut repeated = CoverageMap::new(0);
    let values = [1.0];
    repeated.accumulate(Rect::new(x, y, x + 1, y + 1), &values, 0.5, None, None);
    repeated.accumulate(Rect::new(x, y, x + 1, y + 1), &values, 0.5, None, Some(&[0.2]));
    repeated.accumulate(Rect::new(x, y, x + 1, y + 1), &values, 0.5, None, Some(&[0.8]));
    assert_eq!(repeated.get(x, y), 0.5);
    assert_eq!(repeated.depth(x, y), Some(0.8));
}

fn pixel_index_for_tile(x: i32, y: i32) -> usize {
    (y.rem_euclid(COV_TILE) * COV_TILE + x.rem_euclid(COV_TILE)) as usize
}

#[test]
fn sparse_large_footprint_only_allocates_and_dirties_nonzero_tiles() {
    let rect = Rect::new(-128, -128, 256, 256);
    let mut vals = vec![0.0; rect.width() as usize * rect.height() as usize];
    vals[pixel_index(rect, -1, 64)] = 0.5;
    let mut map = CoverageMap::new(0);
    map.accumulate(rect, &vals, 0.8, Some(&[1.0]), None);
    assert_eq!(map.tiles.len(), 1);
    assert_eq!(map.take_dirty(), vec![(-1, 1)]);
    assert_eq!(map.get(-1, 64), 0.4);
    assert!(map.tiles.values().all(|tile| tile.col.is_empty() && tile.depth.is_empty()));
    vals.fill(0.0);
    map.accumulate(rect, &vals, 0.8, None, None);
    assert!(map.take_dirty().is_empty());
    assert_eq!(map.tiles.len(), 1);
    let bounds = map.bounds();
    map.accumulate(Rect::new(0, 0, 0, 0), &[], 0.8, None, None);
    assert_eq!(map.bounds(), bounds);
}
