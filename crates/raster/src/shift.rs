//! [`Surface::translated`] in pieces: the target tiles of a shift by a whole number of pixels
//! that is not a whole number of tiles, each built from the source tiles it reads. The one-shot
//! call and [`TranslateSteps`] build every tile with the same code, so their results are equal.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use photocraft_geom::{Rect, TILE_SIZE, TileCoord};

use super::{Surface, Tile, TileBytes, UniformTiles, scratch};

/// What the target tiles of one shift share: the offset, the source's content bounds, rows of the
/// default pixel, which source tiles are one solid value, and the tiles already built.
pub(super) struct ShiftBuilder {
    dx: i32,
    dy: i32,
    content: Rect,
    bpp: usize,
    default_pixel: Box<[u8]>,
    /// One row of the default pixel, so the regions a shift vacates fill with bulk copies (empty
    /// when the default is zero: the tile starts zeroed).
    fill_row: Vec<u8>,
    /// One tile row of the default pixel, to compare a finished tile with a row at a time.
    default_row: Vec<u8>,
    pub(super) uniform: UniformTiles,
    /// Target tiles rebuilt from the same source tiles at the same offset are the same tile: a
    /// bundle can point thousands of tile references at one payload, and a move would otherwise
    /// copy it into a separate buffer for each.
    built: Mutex<HashMap<Vec<usize>, Option<Arc<Tile>>>>,
}

impl ShiftBuilder {
    /// A builder for shifting `src` by (`dx`, `dy`); `content` must hold every pixel of `src`
    /// that isn't the default one.
    pub(super) fn new(src: &Surface, dx: i32, dy: i32, content: Rect) -> Self {
        let bpp = src.format.bytes_per_pixel();
        let fill_row: Vec<u8> = if src.default_pixel.iter().any(|&b| b != 0) {
            let mut row = Vec::with_capacity(TILE_SIZE as usize * bpp);
            for _ in 0..TILE_SIZE {
                row.extend_from_slice(&src.default_pixel);
            }
            row
        } else {
            Vec::new()
        };
        Self {
            dx,
            dy,
            content,
            bpp,
            default_pixel: src.default_pixel.clone(),
            fill_row,
            default_row: src.default_pixel.repeat(TILE_SIZE.unsigned_abs() as usize),
            uniform: UniformTiles::default(),
            built: Mutex::default(),
        }
    }

    /// The source tiles that can hold content, in coordinate order.
    pub(super) fn sources(src: &Surface, content: Rect) -> impl Iterator<Item = (&TileCoord, &Arc<Tile>)> {
        src.tiles.iter().filter(move |(c, _)| !c.rect().intersect(&content).is_empty())
    }

    /// Notes whether source tile `t` is one solid value, so targets reading only such tiles share
    /// one tile.
    pub(super) fn scan(&mut self, t: &Arc<Tile>) {
        self.uniform.value_of(t, self.bpp);
    }

    /// The target tiles: every tile a source tile's pixels land on.
    pub(super) fn targets(&self, src: &Surface) -> BTreeSet<TileCoord> {
        let (dx, dy) = (self.dx, self.dy);
        let moved = self.content.translate(dx, dy);
        Self::sources(src, self.content).flat_map(|(c, _)| c.rect().translate(dx, dy).intersect(&moved).tiles()).collect()
    }

    /// Builds target tile `dc` of `src` shifted; `None` when it would hold only the default pixel.
    pub(super) fn build(&self, src: &Surface, dc: TileCoord) -> Option<(TileCoord, Arc<Tile>)> {
        let (dx, dy, content, bpp) = (self.dx, self.dy, self.content, self.bpp);
        let (fill_row, default_row, built, uniform) = (&self.fill_row, &self.default_row, &self.built, &self.uniform);
        let dr = dc.rect();
        let sr = dr.translate(dx.saturating_neg(), dy.saturating_neg());
        // Every pixel of the target comes from a source tile (none from the default pixel).
        // `sr` must be the exact preimage: an offset of `i32::MIN` saturates its negation.
        let covered = sr.width() == TILE_SIZE.unsigned_abs()
            && sr.height() == TILE_SIZE.unsigned_abs()
            && sr.translate(dx, dy) == dr
            && content.contains_rect(&sr)
            && sr.tiles().all(|sc| src.tiles.contains_key(&sc));
        if covered && let Some(shared) = uniform.window(src, sr) {
            // A tile of only the default pixel is left out, as `prune` would.
            let keep = shared.bytes().get(..bpp) != Some(&*self.default_pixel);
            return keep.then_some((dc, shared));
        }
        // The identities of the source tiles under a covered window (the offset is the same
        // for every target of this call).
        let memo_key: Option<Vec<usize>> = covered.then(|| sr.tiles().map(|sc| src.tiles.get(&sc).map_or(0, |t| Arc::as_ptr(t) as usize)).collect());
        if let Some(k) = &memo_key
            && let Some(hit) = built.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(k)
        {
            return hit.clone().map(|t| (dc, t));
        }

        let mut data = scratch::take_exact((TILE_SIZE * TILE_SIZE) as usize * bpp, 0u8);
        let (tx0, ty0) = (i64::from(dc.tx) * i64::from(TILE_SIZE), i64::from(dc.ty) * i64::from(TILE_SIZE));
        let row_len = TILE_SIZE as usize * bpp;
        // Per stored row: copy the source span that falls inside the content in as few
        // memcpys as the tile grid allows, and fill the vacated edges from the default
        // row — no whole-tile pre-fill, which rewrote nearly every byte twice.
        for local_y in 0..i64::from(TILE_SIZE) {
            let y = ty0 + local_y;
            let Some(row) = data.get_mut(local_y as usize * row_len..(local_y as usize + 1) * row_len) else { continue };
            let in_content_y = y - i64::from(dy) >= i64::from(content.y0) && y - i64::from(dy) < i64::from(content.y1);
            // The row's target columns whose SOURCE lies inside the content are copied
            // (the copy loop crosses source tiles as needed); the rest stays default —
            // exactly the pixels the shift vacated.
            let cx0 = (i64::from(content.x0) + i64::from(dx)).max(tx0);
            let cx1 = (i64::from(content.x1) + i64::from(dx)).min(tx0 + i64::from(TILE_SIZE));
            if !in_content_y || cx1 <= cx0 {
                if !fill_row.is_empty() {
                    row.copy_from_slice(fill_row);
                }
                continue;
            }
            let left = (cx0.max(tx0) - tx0) as usize * bpp;
            let copied_end = (cx1.min(tx0 + i64::from(TILE_SIZE)) - tx0) as usize * bpp;
            if !fill_row.is_empty() {
                if left > 0 {
                    row[..left].copy_from_slice(&fill_row[..left]);
                }
                if copied_end < row_len {
                    row[copied_end..].copy_from_slice(&fill_row[copied_end..]);
                }
            }
            let mut x = cx0.max(tx0);
            let x_end = cx1.min(tx0 + i64::from(TILE_SIZE));
            // Rows run over the same one or two source tiles: cache the last lookup.
            let mut cached: Option<(i64, i64, TileBytes<'_>)> = None;
            while x < x_end {
                // The source document position this target column reads, and the source
                // tile it lives in (the shift crosses source tiles as x runs).
                let (sx, sy) = (x - i64::from(dx), y - i64::from(dy));
                let (tx, ty) = (sx.div_euclid(i64::from(TILE_SIZE)), sy.div_euclid(i64::from(TILE_SIZE)));
                // A missing source tile inside the content holds the default pixel, not
                // zeros: fill the span from the default row instead of skipping it
                // (echelon's white-mask-with-sparse-tiles case).
                if !matches!(&cached, Some((ctx, cty, _)) if *ctx == tx && *cty == ty) {
                    match src.tiles.get(&TileCoord::new(tx as i32, ty as i32)) {
                        Some(t) => cached = Some((tx, ty, t.bytes())),
                        None => {
                            let span_end = ((tx + 1) * i64::from(TILE_SIZE) + i64::from(dx)).min(x_end).max(x + 1);
                            if !fill_row.is_empty() {
                                let at = (x - tx0) as usize * bpp;
                                let len = (span_end - x) as usize * bpp;
                                if let Some(d) = row.get_mut(at..at + len) {
                                    d.copy_from_slice(&fill_row[at..at + len]);
                                }
                            }
                            x = span_end;
                            continue;
                        }
                    }
                }
                let Some((_, _, st)) = &cached else { break };
                let span_end = (((tx + 1) * i64::from(TILE_SIZE) + i64::from(dx)).min(x_end)).max(x + 1);
                let span = (span_end - x) as usize * bpp;
                let src = ((sy - ty * i64::from(TILE_SIZE)) * row_len as i64 + (sx - tx * i64::from(TILE_SIZE)) * bpp as i64) as usize;
                let dst = (x - tx0) as usize * bpp;
                if let (Some(d), Some(sr)) = (row.get_mut(dst..dst + span), st.get(src..src + span)) {
                    d.copy_from_slice(sr);
                }
                x = span_end;
            }
        }
        // A target that got only default pixels isn't stored, as `prune` would drop it. Checked
        // after the copy, a row at a time: a tile with content stops at its first such row.
        let finished = if data.chunks(default_row.len()).any(|row| row != default_row) {
            Some(Arc::new(Tile::from_vec(data)))
        } else {
            scratch::give(data);
            None
        };
        // Workers that missed the memo at the same time each built a copy: all of them take
        // the one stored first, so the targets share it however many threads raced.
        let finished = match memo_key {
            Some(k) => built.lock().unwrap_or_else(std::sync::PoisonError::into_inner).entry(k).or_insert(finished).clone(),
            None => finished,
        };
        finished.map(|t| (dc, t))
    }
}

/// A surface shifted by whole pixels, a bounded piece of work at a time: the same surface
/// [`Surface::translated`] returns, for a caller that must not block for the whole shift (the web
/// build has no threads). A unit of work is the scan of one source tile or the build of one
/// target tile, so a call of [`TranslateSteps::step`] that is asked to stop does no more than one.
pub struct TranslateSteps {
    src: Surface,
    builder: ShiftBuilder,
    /// The source tiles still to scan, then the target tiles still to build, both in coordinate
    /// order: the targets are the tiles the shifted source tiles land on, found as they are scanned.
    scan: std::vec::IntoIter<(TileCoord, Arc<Tile>)>,
    sources: usize,
    found: BTreeSet<TileCoord>,
    targets: Option<std::vec::IntoIter<TileCoord>>,
    built: usize,
    total_targets: usize,
    out: Surface,
    /// Set when the shift is by whole tiles, which needs no building.
    whole_tiles: bool,
    dx: i32,
    dy: i32,
    content: Rect,
}

impl TranslateSteps {
    /// Prepares to shift `src` by (`dx`, `dy`); `content` must hold every pixel of `src` that
    /// isn't the default one (`src.content_bounds()`, say), as for [`Surface::translated`].
    pub fn new(src: Surface, dx: i32, dy: i32, content: Rect) -> Self {
        let whole_tiles = dx % TILE_SIZE == 0 && dy % TILE_SIZE == 0;
        let builder = ShiftBuilder::new(&src, dx, dy, content);
        let tiles: Vec<(TileCoord, Arc<Tile>)> =
            if whole_tiles { Vec::new() } else { ShiftBuilder::sources(&src, content).map(|(c, t)| (*c, Arc::clone(t))).collect() };
        let out = Surface { format: src.format, default_pixel: src.default_pixel.clone(), tiles: Default::default() };
        Self {
            sources: tiles.len(),
            scan: tiles.into_iter(),
            builder,
            found: BTreeSet::new(),
            targets: None,
            built: 0,
            total_targets: 0,
            out,
            whole_tiles,
            dx,
            dy,
            content,
            src,
        }
    }

    /// How much of the shift is done, `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        let todo = self.scan.len() + self.targets.as_ref().map_or(0, ExactSizeIterator::len);
        let total = self.sources + self.total_targets;
        if total == 0 { 0.0 } else { (1.0 - todo as f32 / total as f32).clamp(0.0, 1.0) }
    }

    /// Does at least one unit of work, then goes on while `more()` returns true; `Some` with the
    /// shifted surface once the last tile is built.
    pub fn step(&mut self, more: &mut dyn FnMut() -> bool) -> Option<Surface> {
        if self.whole_tiles {
            return Some(self.src.translated(self.dx, self.dy, self.content));
        }
        loop {
            if self.targets.is_none() {
                match self.scan.next() {
                    Some((c, t)) => {
                        self.builder.scan(&t);
                        let moved = self.content.translate(self.dx, self.dy);
                        self.found.extend(c.rect().translate(self.dx, self.dy).intersect(&moved).tiles());
                    }
                    None => {
                        let list: Vec<TileCoord> = std::mem::take(&mut self.found).into_iter().collect();
                        self.total_targets = list.len();
                        self.targets = Some(list.into_iter());
                    }
                }
            } else if let Some(dc) = self.targets.as_mut().and_then(Iterator::next) {
                if let Some((c, t)) = self.builder.build(&self.src, dc) {
                    self.out.tiles.insert(c, t);
                }
                self.built += 1;
            } else {
                return Some(std::mem::replace(
                    &mut self.out,
                    Surface { format: self.src.format, default_pixel: self.src.default_pixel.clone(), tiles: Default::default() },
                ));
            }
            if !more() {
                return None;
            }
        }
    }
}
