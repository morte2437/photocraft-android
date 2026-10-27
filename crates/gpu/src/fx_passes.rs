use super::batch::op_words;
use super::consts::{MAP16, MAP32};
use super::kit::MapDraw;
use super::paging::field_reaches;
use super::textures::{FxEntry, FxKey, ProgState, Stats, Tex};
use super::*;

impl Compositor {
    /// Build or update the effect maps of one layer over `region`: its whole effect region, or
    /// the window of one page cell (`key.1`) grown by the effects' reach.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sync_fx(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        doc: &Document,
        f: &plan::FxLayer<'_>,
        key: FxKey,
        region: Rect,
        stats: &mut Stats,
    ) -> Result<(), Unsupported> {
        let layer = f.layer;
        let canvas = doc.bounds();
        let is_group = matches!(layer.content, LayerContent::Group(_));
        let shape_key = if is_group { fx::group_key(layer, &doc.global_light, &mut |_| {}) } else { fx::shape_key(layer, canvas) };
        let (content_src, mask_src) = if is_group { (None, None) } else { fx::shape_sources(layer) };
        let frame = self.frame;
        let (rw, rh) = (region.width(), region.height());

        // What changed: everything (new settings / size), the position only, some tiles, or
        // nothing.
        let mut damage = Rect::EMPTY;
        let prev = self.fx.get(&key).filter(|e| e.shape_key == shape_key).map(|e| e.region);
        let mut rebuild = true;
        if prev == Some(region)
            && let Some(e) = self.fx.get(&key)
        {
            rebuild = false;
            for (old, cur) in e.tiles.iter().zip([content_src, mask_src]) {
                let d = match (old, cur) {
                    (Some(o), Some(s)) => fx::damage(o, s),
                    (None, None) => Rect::EMPTY,
                    // A source appeared or vanished (the key changes with it too).
                    _ => region,
                };
                damage = if damage.is_empty() { d } else { damage.union(&d) };
            }
            damage = damage.intersect(&region);
        } else if let Some(old) = prev
            && (old.width(), old.height()) == (rw, rh)
            && !region.is_empty()
        {
            // Moved by whole pixels: maps are computed relative to the region, so if the shape
            // moved unchanged with it, every map is still exact.
            let v = fx::shape(doc, layer, region);
            if let Some(e) = self.fx.get_mut(&key)
                && v == e.shape_cpu
            {
                e.region = region;
                rebuild = false;
            }
        }
        if rebuild {
            let shape = Tex::map(device, "pc_fx_shape", region, MAP32);
            let n = rw as usize * rh as usize;
            self.fx.insert(
                key,
                FxEntry {
                    doc: doc.id,
                    region,
                    shape_key,
                    tiles: [None, None],
                    _pin: if is_group {
                        let mut pin = Vec::new();
                        fx::group_key(layer, &doc.global_light, &mut |t| pin.push(Arc::downgrade(t)));
                        pin
                    } else {
                        Vec::new()
                    },
                    shape_cpu: vec![0.0; n],
                    shape,
                    fields: HashMap::new(),
                    progs: Vec::new(),
                    last_used: frame,
                    stamp: 0,
                },
            );
            damage = region;
        }
        // Present: kept or (re)inserted above.
        let stamp = self.stamp;
        let Some(e) = self.fx.get_mut(&key) else { return Ok(()) };
        e.last_used = frame;
        e.stamp = stamp;
        e.doc = doc.id;
        e.tiles = [content_src.map(fx::snapshot), mask_src.map(fx::snapshot)];
        if region.is_empty() {
            return Ok(());
        }

        let t_trace = web_time_now();
        let mut staged = 0u64;
        // Shape over the damage (compose's own alpha).
        if !damage.is_empty() {
            stats.fx_shapes += 1;
            let v = fx::shape(doc, layer, damage);
            fx::paste(&mut e.shape_cpu, region, damage, &v);
            e.shape.write_r32(queue, region, damage, &v);
            staged += v.len() as u64 * 4;
        }

        // Programs, and the distance fields they read (max reach per field).
        // Filled shapes stroke their outline (the effect shape); others estimate it.
        let vector_shape = matches!(layer.content, LayerContent::Shape(_)) && photocraft_compose::effect_outline(layer).is_none();
        let anchor = layer.effects.reference.unwrap_or((f64::from(f.bounds.x0), f64::from(f.bounds.y0)));
        let progs: Vec<fx::MapProgram> =
            layer.effects.items.iter().filter(|e| e.enabled()).map(|e| fx::program_with(e, &doc.global_light, vector_shape, &doc.patterns, anchor)).collect();
        let want = field_reaches(&progs);
        e.fields.retain(|k, _| want.contains_key(k));
        // Each field to recompute: over which rectangle, exact up to which reach.
        let jobs: Vec<(FieldKind, Rect, i32)> = want
            .iter()
            .filter_map(|(&kind, &reach)| match e.fields.get(&kind).map(|(r, _)| *r) {
                Some(r) if r >= reach => (!damage.is_empty()).then(|| (kind, damage.inflate(fx::field_radius(kind, r)).intersect(&region), r)),
                _ => Some((kind, region, reach)),
            })
            .collect();
        // A chiselled bevel's two fields are asked for and kept together (`fx::bevel_fields`), so they
        // are recomputed together, over one rectangle at one reach.
        let job = |k: FieldKind| jobs.iter().find(|j| j.0 == k).map(|j| (j.1, j.2));
        let (bevel_in, bevel_out) = (job(FieldKind::BevelInside), job(FieldKind::BevelOutside));
        if bevel_in != bevel_out {
            return Err(Unsupported(format!("bevel fields of `{}` out of step: inside {bevel_in:?}, outside {bevel_out:?}", layer.name)));
        }
        for (kind, out, reach) in jobs {
            let fields = match kind {
                FieldKind::BevelInside => {
                    let [inside, outside] = fx::bevel_fields(reach, &e.shape_cpu, region, out);
                    vec![(FieldKind::BevelInside, inside), (FieldKind::BevelOutside, outside)]
                }
                // Computed with BevelInside, over the same rectangle at the same reach (checked above).
                FieldKind::BevelOutside => continue,
                _ => vec![(kind, fx::field(kind, reach, &e.shape_cpu, region, out))],
            };
            for (kind, v) in fields {
                if e.fields.get(&kind).is_none_or(|(r, _)| *r < reach) {
                    e.fields.insert(kind, (reach, Tex::map(device, "pc_fx_field", region, MAP32)));
                }
                if let Some((_, t)) = e.fields.get(&kind) {
                    t.write_r32(queue, region, out, &v);
                }
                staged += v.len() as u64 * 4;
                stats.fx_pixels += out.width() as u64 * out.height() as u64;
            }
        }

        if let Some(t) = t_trace.filter(|_| !damage.is_empty()) {
            eprintln!("effect maps of `{}`: shape + distance fields over {damage:?} in {:.2} ms (CPU)", layer.name, t.elapsed().as_secs_f64() * 1000.0);
        }
        e.progs.truncate(progs.len());
        while e.progs.len() < progs.len() {
            e.progs.push(ProgState { key: 0, maps: Vec::new() });
        }
        self.staged += staged;
        for (i, prog) in progs.iter().enumerate() {
            if prog.maps == 0 {
                continue;
            }
            let Some(e) = self.fx.get_mut(&key) else { return Ok(()) };
            let same = e.progs[i].key == prog.key && e.progs[i].maps.len() == prog.maps;
            let d = if same { damage } else { region };
            if d.is_empty() {
                continue;
            }
            if !same {
                e.progs[i] = ProgState {
                    key: prog.key,
                    maps: (0..prog.maps).map(|k| prog.stages.iter().any(|s| s.out == Some(k)).then(|| Tex::map(device, "pc_fx_map", region, MAP16))).collect(),
                };
            }
            stats.fx_programs += 1;
            self.run_program(device, queue, encoder, key, i, prog, d, stats);
        }
        Ok(())
    }

    /// Record the passes of `prog` (maps of item `item` of `layer`) recomputing damage `d`.
    #[allow(clippy::too_many_arguments)]
    fn run_program(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        key: FxKey,
        item: usize,
        prog: &fx::MapProgram,
        d: Rect,
        stats: &mut Stats,
    ) {
        let pattern_views: Vec<Option<wgpu::TextureView>> =
            prog.stages.iter().map(|s| s.pattern.as_ref().map(|p| self.pattern_view(device, queue, p))).collect();
        let Some(e) = self.fx.get(&key) else { return };
        let region = e.region;
        let shape = e.shape.view.clone();
        let finals: Vec<Option<wgpu::TextureView>> = e.progs[item].maps.iter().map(|m| m.as_ref().map(|t| t.view.clone())).collect();
        let fields: HashMap<FieldKind, (i32, wgpu::TextureView)> = e.fields.iter().map(|(k, (r, t))| (*k, (*r, t.view.clone()))).collect();
        let windows = prog.windows(d, region, &|k| fields.get(&k).map_or(0, |f| f.0));
        let (assign, count) = prog.temps();
        let mut temps: Vec<wgpu::TextureView> = Vec::with_capacity(count);
        for _ in 0..count {
            let v = self.temp(device, region, &temps);
            temps.push(v);
        }
        // `None` only if a program's map or temporary wasn't allocated, which `sync_fx` and
        // `MapProgram::temps` rule out.
        let target_of = |i: usize| -> Option<(wgpu::TextureView, wgpu::TextureFormat)> {
            match prog.stages[i].out {
                Some(k) => Some((finals.get(k)?.clone()?, MAP16)),
                None => Some((temps.get((*assign.get(i)?)?)?.clone(), MAP32)),
            }
        };
        let view_of = |inp: Option<fx::In>| -> Option<wgpu::TextureView> {
            match inp? {
                fx::In::Shape => Some(shape.clone()),
                fx::In::Field(k) => fields.get(&k).map(|f| f.1.clone()),
                fx::In::Val(k) => target_of(k).map(|t| t.0),
            }
        };
        let mut draws = Vec::with_capacity(prog.stages.len());
        for (i, s) in prog.stages.iter().enumerate() {
            let w = windows[i];
            if w.is_empty() {
                continue;
            }
            stats.fx_pixels += w.width() as u64 * w.height() as u64;
            let Some((target, format)) = target_of(i) else { continue };
            let mut p = plan::Pass::new(s.kernel, 0);
            p.params[0] = s.p0;
            p.params[1] = s.p1;
            p.params[3] = s.p3;
            p.extra = s.p4;
            draws.push(MapDraw {
                kernel: s.kernel,
                format,
                target,
                scissor: [(w.x0 - region.x0) as u32, (w.y0 - region.y0) as u32, w.width(), w.height()],
                space: [region.x0, region.y0, region.width() as i32, region.height() as i32],
                op: op_words(&p, None, None, None),
                views: [view_of(s.a), view_of(s.b), view_of(s.s), None, None, None, None, pattern_views[i].clone()],
                lut: s.lut.clone(),
            });
        }
        self.kit.run(device, queue, encoder, &draws);
    }
}

/// A start time when `PHOTOCRAFT_FX_TRACE=1` (it prints the CPU time of effect shapes and
/// distance fields); never on wasm.
fn web_time_now() -> Option<std::time::Instant> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        T.get_or_init(|| std::env::var_os("PHOTOCRAFT_FX_TRACE").is_some()).then(std::time::Instant::now)
    }
    #[cfg(target_arch = "wasm32")]
    None
}
