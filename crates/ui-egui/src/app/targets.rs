use super::*;

/// The active layer is an adjustment layer. It has no pixels, so painting and filling it edit its
/// mask even when its icon thumbnail, not the mask, is the Layers panel's target, as in
/// Photoshop (#4298).
pub(crate) fn adjustment_layer_active(st: &photocraft_engine::DocState) -> bool {
    st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Adjustment(_)))
}

impl PhotocraftApp {
    /// `params` aimed at the active layer's mask (`"target":"mask"`) when the Layers panel targets
    /// it and command `id` edits the target (adjustments, filters, fills) without naming one:
    /// ⌘I then inverts the mask, as in Photoshop (#780). A targeted alpha channel or Quick Mask
    /// mode wins, as the engine routes those itself.
    pub fn with_mask_target(&self, id: &str, params: Value) -> Value {
        if !photocraft_engine::channel_cmds::follows_target(id) || params.get("target").is_some() {
            return params;
        }
        // A filter mask is painted, filled and filtered in place; it doesn't float or move, and
        // Edit › Transform, Apply Image and the dialog filters act on the smart object.
        let target = if self.filter_mask_targeted() && photocraft_engine::channel_cmds::follows_filter_mask(id) {
            "filterMask"
        } else if self.layer_mask_targeted() {
            "mask"
        } else {
            return params;
        };
        match params {
            Value::Object(mut m) => {
                m.insert("target".into(), Value::from(target));
                Value::Object(m)
            }
            _ => serde_json::json!({ "target": target }),
        }
    }

    /// The Layers panel targets the active smart object's filter mask (FILE-215-3), and no alpha
    /// channel or Quick Mask takes over.
    pub fn filter_mask_targeted(&self) -> bool {
        let Some(st) = self.session.active() else { return false };
        let Some(id) = st.active_layer.filter(|id| self.ui.filter_mask_target == Some(id.0)) else { return false };
        let composite = st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::Composite && st.doc.quick_mask.is_none();
        composite && st.doc.layer(id).is_some_and(|l| matches!(&l.content, photocraft_doc::LayerContent::Smart(sm) if !sm.smart_filters.is_empty()))
    }

    /// The active layer's mask is what pixel commands edit: the Layers panel targets it, the layer
    /// is an adjustment layer (no pixels of its own), or every colour channel is off and that mask
    /// is the picture. An alpha channel or Quick Mask wins, as the engine routes those itself.
    pub fn layer_mask_targeted(&self) -> bool {
        let Some(st) = self.session.active() else { return false };
        let layer_mask = st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::LayerMask;
        let composite = (st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::Composite || layer_mask) && st.doc.quick_mask.is_none();
        let has_mask = st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.mask.is_some());
        if !composite || !has_mask {
            return false;
        }
        if self.ui.mask_target || layer_mask || adjustment_layer_active(st) {
            return true;
        }
        let colors_off = st.channel_view.visible_colors(photocraft_engine::channel_cmds::color_count(&st.doc)) == 0;
        let alpha_on = (0..st.doc.channels.len()).any(|i| st.channel_view.alpha_shown(i));
        colors_off && !alpha_on
    }

    /// Targeting the pixels, the layer mask or the vector mask ends a shown filter mask (⌥-click
    /// on its thumbnail): while it shows, [`Self::sync_mask_targets`] keeps the filter mask
    /// targeted, so strokes would keep landing there.
    pub(crate) fn end_filter_mask_view(&mut self) {
        let Some(v) = self.session.active().and_then(photocraft_engine::mask_view_cmds::current).filter(|v| v.filter) else { return };
        let off = serde_json::json!({"layer": v.layer.0, "mode": "off", "mask": "filter"});
        if let Err(err) = self.session.execute(photocraft_engine::mask_view_cmds::ID, off) {
            log::warn!("could not end the filter mask view: {err}");
        }
    }

    /// Explicitly targeting a filter mask ends any conflicting mask view; untargeting it ends its
    /// own view too.
    pub(crate) fn select_filter_mask_target(&mut self, layer: Option<u64>) {
        if layer.is_none() {
            self.end_filter_mask_view();
        }
        if layer.is_some() {
            if let Some(st) = self.session.active_mut() {
                if st.channel_view.layer_mask.is_some_and(|v| !v.filter || Some(v.layer.0) != layer) {
                    st.channel_view.layer_mask = None;
                }
                if matches!(
                    st.channel_view.target,
                    photocraft_engine::channel_cmds::ChannelTarget::LayerMask | photocraft_engine::channel_cmds::ChannelTarget::VectorMask
                ) {
                    st.channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Composite;
                }
            }
            self.ui.mask_target = false;
            self.ui.vector_mask_target = false;
        }
        self.ui.filter_mask_target = layer;
        self.sync_mask_targets();
    }

    /// Viewing a layer mask (#196) targets it; a vector-mask target needs a vector mask on the
    /// active layer (a shape layer's path is its content, not a mask). Targeting a mask or the
    /// pixels brings back that target's foreground/background pair, as in Photoshop (#2166).
    pub(crate) fn sync_mask_targets(&mut self) {
        if let Some(st) = self.session.active_mut() {
            use photocraft_engine::channel_cmds::ChannelTarget;
            let channel = st.channel_view.target;
            match photocraft_engine::mask_view_cmds::current(st) {
                Some(v) if v.filter => {
                    self.ui.filter_mask_target = Some(v.layer.0);
                    self.ui.mask_target = false;
                    self.ui.vector_mask_target = false;
                }
                Some(_) => {
                    self.ui.mask_target = true;
                    self.ui.vector_mask_target = false;
                    self.ui.filter_mask_target = None;
                }
                None if channel == ChannelTarget::LayerMask => {
                    self.ui.mask_target = true;
                    self.ui.vector_mask_target = false;
                }
                None if channel == ChannelTarget::VectorMask => {
                    self.ui.vector_mask_target = true;
                    self.ui.mask_target = false;
                }
                None => {}
            }
            if self.ui.mask_target || self.ui.vector_mask_target {
                self.ui.filter_mask_target = None;
            }
            if self.ui.vector_mask_target && !mask_thumbs_ui::has_vector_mask(st) {
                self.ui.vector_mask_target = false;
            }
            if self.ui.mask_target && st.active_layer.and_then(|id| st.doc.layer(id)).is_none_or(|l| l.mask.is_none()) {
                self.ui.mask_target = false;
            }
            if !self.ui.mask_target && st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::LayerMask {
                st.channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Composite;
            }
            if !self.ui.mask_target || self.ui.properties_layer != st.active_layer.map(|l| l.0) {
                self.ui.properties_layer = None;
            }
        }
        if self.ui.filter_mask_target.is_some() && !self.filter_mask_targeted() {
            self.ui.filter_mask_target = None;
        }
        let mask = self.layer_mask_targeted() || self.filter_mask_targeted();
        self.session.tools.target_mask(mask);
    }
}
