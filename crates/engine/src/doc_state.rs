use super::*;

/// Per-document editing state.
#[derive(Clone, Debug)]
pub struct DocState {
    pub doc: Arc<Document>,
    pub history: History,
    /// The primary ("key") layer: what single-layer commands act on. Always a member of
    /// `selected_layers` when set.
    pub active_layer: Option<LayerId>,
    /// Every selected layer in the Layers panel (⌘/⇧-click), in selection order. Like
    /// `active_layer`, selecting is not a history step, but undo and redo restore the layers each
    /// state targeted when the next step was taken; see [`DocState::selected_layers`].
    pub selected_layers: Vec<LayerId>,
    /// Anchor of ⇧-click range selection (the last plainly or ⌘-clicked layer).
    pub layer_anchor: Option<LayerId>,
    pub path: Option<String>,
    /// An Affinity document (or its preview) must not acquire the native source as its Save path.
    pub source_read_only: bool,
    /// Increments on every change; UIs re-render when it moves.
    pub revision: u64,
    pub saved_revision: u64,
    /// Native save cache shared with this document's workers, released with the editing state.
    pub pcraft_cache: photocraft_io::PcraftExportCache,
    /// How much of `revision` came from view-only changes (channel eyes, targeting): pixel caches can ignore those.
    pub view_revision: u64,
    /// Area changed by the latest revision (None = assume everything changed).
    pub last_damage: Option<photocraft_geom::Rect>,
    /// Coalescing key of the latest history step (see [`Session::execute`]'s `coalesce` param).
    pub coalesce: Option<String>,
    /// Channels panel: targeted channel and eye toggles (view state, not history).
    pub channel_view: channel_cmds::ChannelView,
    /// Select › Isolate Layers: the Layers panel lists only these layers (empty = off; view state).
    pub isolated_layers: Vec<LayerId>,
    /// Painting symmetry preset or sampled path (tool state, not document pixels).
    pub symmetry: Option<symmetry_cmds::PaintingSymmetry>,
    /// Layers panel: layers whose effects list is collapsed under their row (the fx triangle;
    /// view state, not history). Effects lists start open.
    pub fx_collapsed: Vec<LayerId>,
    /// ⌥-click on a layer's eye (`layer.showOnly`): the layer shown alone and every layer's
    /// visibility before, so the next ⌥-click restores it (view state, not history).
    pub show_only: Option<(LayerId, Vec<(LayerId, bool)>)>,
    /// A floating selection (`select.float`): the cut piece and where it floats, until dropped
    /// (view state: the document is unchanged until `select.drop`).
    pub floating: Option<float_cmds::Floating>,
    /// Color Sampler points (`view.colorSamplers.*`, #1046): numbered sample positions in document
    /// pixel coordinates. Per-document view state: discarded when the document closes, never saved
    /// to `.pcraft` or PSD, and not a history step.
    pub color_samplers: Vec<[f64; 2]>,
    /// History panel snapshots, oldest first (`history.newSnapshot`, #2966): the first (id 0) is
    /// the document as opened. Kept for the session only, as in Photoshop: not history states,
    /// and not saved with the document.
    pub snapshots: Vec<history_cmds::Snapshot>,
    /// The id the next snapshot gets; ids are never reused, so deleting one keeps the others.
    pub(crate) next_snapshot: u64,
    /// The snapshot set as the History Brush's source (`history.setSource {snapshot}`); `None`:
    /// the source is a state, kept by the history ([`photocraft_ops::History::brush_source`]).
    pub(crate) source_snapshot: Option<u64>,
}

impl DocState {
    /// Trim the history to its budget, counting the tiles only History panel snapshots hold
    /// against it too (#4530). Returns how many states were dropped.
    pub fn trim_history(&mut self) -> usize {
        let snaps: Vec<&Document> = self.snapshots.iter().map(|s| &*s.doc).collect();
        self.history.trim_with(&self.doc, &snaps)
    }

    /// Pixel bytes of the document, the tiles only its history holds and the tiles only its
    /// History panel snapshots hold.
    pub fn memory_bytes(&self) -> usize {
        let snaps: Vec<&Document> = self.snapshots.iter().map(|s| &*s.doc).collect();
        self.history.pixel_bytes(&self.doc).saturating_add(self.history.extra_bytes(&self.doc, &snaps))
    }

    pub fn new(doc: Document, path: Option<String>) -> Self {
        let active_layer = doc.top_layer();
        let mut history = History::default();
        let target = LayerTarget { active: active_layer, selected: active_layer.into_iter().collect() };
        history.set_current_layers(target.clone());
        let doc = Arc::new(doc);
        // Photoshop's first snapshot: the document as opened, named after it, so the History
        // panel's top row can always bring it back (#2966).
        let first = history_cmds::Snapshot { id: 0, name: doc.name.clone(), doc: doc.clone(), layers: target };
        Self {
            doc,
            history,
            active_layer,
            selected_layers: active_layer.into_iter().collect(),
            layer_anchor: active_layer,
            path,
            source_read_only: false,
            revision: 1,
            saved_revision: 1,
            pcraft_cache: Default::default(),
            view_revision: 0,
            last_damage: None,
            coalesce: None,
            channel_view: Default::default(),
            isolated_layers: Vec::new(),
            symmetry: None,
            fx_collapsed: Vec::new(),
            show_only: None,
            floating: None,
            color_samplers: Vec::new(),
            snapshots: vec![first],
            next_snapshot: 1,
            source_snapshot: None,
        }
    }
    /// The selected layers in bottom-to-top document order, always including the active layer.
    /// Robust against stale state: ids no longer in the document are skipped, and if the active
    /// layer was changed without updating the set, the selection is just the active layer.
    pub fn selected_layers(&self) -> Vec<LayerId> {
        let Some(active) = self.active_layer else { return Vec::new() };
        let set: &[LayerId] = if self.selected_layers.contains(&active) { &self.selected_layers } else { std::slice::from_ref(&active) };
        self.doc.walk().into_iter().map(|(_, _, l)| l.id).filter(|id| set.contains(id)).collect()
    }
    pub fn is_layer_selected(&self, id: LayerId) -> bool {
        self.active_layer == Some(id) || (self.active_layer.is_some_and(|a| self.selected_layers.contains(&a)) && self.selected_layers.contains(&id))
    }
    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }
    /// Whether the step from `before` to the current document leaves a clean document clean:
    /// it changed nothing but the selection (#6865). Making, changing or dropping a selection
    /// (marquee, lasso, Magic Wand, Quick / Object Selection, Select › …) is a history step but
    /// not an edit, so it neither marks the file modified nor makes closing ask to save.
    pub(crate) fn keeps_clean(&self, before: &Document) -> bool {
        self.saved_revision == self.revision && layer_multi_cmds::selection_only_change(before, &self.doc)
    }
    /// Move to the next revision; `keep_clean` (from [`Self::keeps_clean`]) moves the saved
    /// revision with it.
    pub(crate) fn bump_revision(&mut self, keep_clean: bool) {
        self.revision += 1;
        if keep_clean {
            self.saved_revision = self.revision;
        }
    }
    /// The targeted layers, as history stores them with each state.
    pub(crate) fn layer_target(&self) -> LayerTarget {
        LayerTarget { active: self.active_layer, selected: self.selected_layers.clone() }
    }
}

/// App-wide tool state that commands read (foreground colour, brush…).
#[derive(Clone, Debug)]
pub struct ToolState {
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    pub brush: photocraft_paint::BrushSettings,
    /// Brush presets (built-ins plus user presets; see `brush.presets.*`).
    pub presets: Vec<photocraft_paint::BrushPreset>,
    /// Bumped whenever `presets` changes ([`Session::brush_presets_changed`]); the preset store
    /// syncs when it moves.
    pub presets_rev: u64,
    /// Mixer Brush paint carried between strokes.
    pub mixer: photocraft_paint::mixer::MixerState,
    /// The coalescing key of the running `tools.setBrush` gesture and the brush before it, so the
    /// gesture journals as one call ([`brush_cmds::coalesce_journal`]).
    pub brush_gesture: Option<(String, photocraft_paint::BrushSettings)>,
    /// Name of the preset the current brush was last picked from. Kept across later edits (the
    /// preset stays "the current brush" so `brush.presets.update` can overwrite it); cleared when
    /// the brush is reset or the preset is gone.
    pub current_preset: Option<String>,
    /// Bumped whenever `brush` or `current_preset` changes ([`ToolState::brush_changed`]). The
    /// last-brush persistence watches it instead of comparing the brush itself, so an unchanged
    /// brush costs a counter compare per frame, not a clone and serialise.
    pub brush_rev: u64,
    /// The revision whose value is in `Preferences::last_brush` (a restored one counts as saved):
    /// [`brush_rev`](Self::brush_rev) == `brush_saved_rev` means there is nothing new to write.
    pub brush_saved_rev: u64,
    /// A layer mask is the edit target ([`ToolState::target_mask`]).
    pub mask_targeted: bool,
    /// The foreground/background pair of the edit target that isn't current: the layer pixels'
    /// colours while a mask is targeted, the mask's while the pixels are (#2166).
    pub other_colors: [[f32; 4]; 2],
    /// Colours picked as CMYK, newest last: the sRGB colour that stands for them and the ink
    /// values chosen. The tool colours are stored as sRGB, and an sRGB round trip does not give
    /// back what was typed (100/0/0/0 cyan is outside sRGB), so a CMYK document is filled with
    /// the inks the person picked (#4196). Matched by colour, so it follows a swap, a reset or a
    /// mask pair without bookkeeping.
    pub cmyk_picks: Vec<([f32; 3], [f32; 4])>,
}

impl ToolState {
    /// How many CMYK picks are remembered.
    const CMYK_PICKS: usize = 8;

    /// Remember that sRGB colour `rgb` was picked as the inks `cmyk` (each 0..1).
    pub fn remember_cmyk(&mut self, rgb: [f32; 3], cmyk: [f32; 4]) {
        self.cmyk_picks.retain(|(c, _)| *c != rgb);
        self.cmyk_picks.push((rgb, cmyk));
        let extra = self.cmyk_picks.len().saturating_sub(Self::CMYK_PICKS);
        self.cmyk_picks.drain(..extra);
    }

    /// The inks `rgb` was picked as, if it was picked as CMYK.
    pub fn picked_cmyk(&self, rgb: [f32; 3]) -> Option<[f32; 4]> {
        self.cmyk_picks.iter().rev().find(|(c, _)| *c == rgb).map(|(_, k)| *k)
    }

    /// Photoshop's colour pair for a layer mask that has none yet: white foreground, black
    /// background (reveal-paint first).
    pub const MASK_COLORS: [[f32; 4]; 2] = [[1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0]];

    /// Note that `brush` or `current_preset` changed. Every setter of either (commands and the
    /// shell's tool-switch paths) calls this; the last-brush persistence only serialises when the
    /// revision has moved since the stored value ([`Self::brush_saved_rev`]).
    pub fn brush_changed(&mut self) {
        self.brush_rev = self.brush_rev.wrapping_add(1);
    }

    /// Switch the edit target between layer pixels and a layer mask. Like Photoshop, the mask and
    /// the pixels keep separate app-wide foreground/background pairs: changing the target puts
    /// the current pair aside and brings back the other one, so colours picked while a mask is
    /// targeted are remembered for masks (#2166). The mask pair starts at [`Self::MASK_COLORS`].
    /// Returns whether the target changed.
    pub fn target_mask(&mut self, mask: bool) -> bool {
        if self.mask_targeted == mask {
            return false;
        }
        self.mask_targeted = mask;
        let [fg, bg] = std::mem::replace(&mut self.other_colors, [self.foreground, self.background]);
        self.foreground = fg;
        self.background = bg;
        true
    }
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            foreground: [0.0, 0.0, 0.0, 1.0],
            background: [1.0, 1.0, 1.0, 1.0],
            // Photoshop's Brush tool starts at 10 % Smoothing.
            brush: photocraft_paint::BrushSettings {
                smoothing: photocraft_paint::brush::Smoothing { amount: 0.1, ..Default::default() },
                ..Default::default()
            },
            presets: photocraft_paint::presets::builtin(),
            presets_rev: 0,
            mixer: Default::default(),
            brush_gesture: None,
            current_preset: None,
            brush_rev: 0,
            brush_saved_rev: 0,
            mask_targeted: false,
            other_colors: Self::MASK_COLORS,
            cmyk_picks: Vec::new(),
        }
    }
}

/// Layer › Layer Style › Copy Layer Style: the effects and the Blending Options, as Photoshop
/// copies them (Paste Layer Style puts them all on each target).
#[derive(Clone, Debug)]
pub struct CopiedStyle {
    pub effects: photocraft_doc::Effects,
    pub blend: photocraft_color::BlendMode,
    pub fill: f32,
    pub advanced: photocraft_doc::AdvancedBlending,
    pub blend_if: photocraft_doc::BlendIf,
    pub excluded_channels: u32,
    /// The colour mode it was copied in: Blend If's channel ranges and the channel restrictions
    /// only carry over to a document in the same mode.
    pub mode: photocraft_color::ColorMode,
}

/// The commands whose [`Session::precheck`] is complete: it accepts what `execute` does (Free
/// Transform applies its box before these, and only when they will run), but for the group
/// nesting limit ([`photocraft_doc::MAX_GROUP_DEPTH`]): grouping layers already that deep is
/// refused only as the edit is made. Layer via Copy and
/// Cut aren't: whether they have anything to copy (a selection over transparent pixels, a group
/// or fill layer) is known only when they run.
pub const PRECHECKED: &[&str] = &["layer.new.layer", "layer.new.group", "layer.new.groupFromLayers", "layer.new.layerFromBackground"];
