use super::{
    DocId, PhotocraftApp, Tool, Value, actions, canvas, delete_layer_prompt, dock, gpu_canvas, jobs_ui, json, mask_trash_prompt, notices, panel_docking, state,
    transform_tool, type_transform, vector_ui,
};

impl PhotocraftApp {
    /// Run an engine command, reporting errors in the status bar.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        if let Some(result) = delete_layer_prompt::intercept(self, id, &params) {
            return result;
        }
        if let Some(result) = mask_trash_prompt::intercept(self, id, &params) {
            return result;
        }
        // Automation input also gates every step a command runs on its behalf (`actions.play`).
        let gate = if self.automation_input && self.session.authorize.is_none() { self.services.automation_authorize } else { None };
        if gate.is_some() {
            self.session.authorize = gate;
        }
        let result = self.run_command(id, params);
        if gate.is_some() {
            self.session.authorize = None;
        }
        // An explicit channel target (⌘2, ⌘\, ⌘3…) decides what the Layers panel targets too:
        // ⌘2 after the mask was targeted goes back to the layer's pixels.
        if result.is_ok() && id.starts_with("channel.target") {
            if let Some(t) = self.session.active().map(|st| st.channel_view.target) {
                use photocraft_engine::channel_cmds::ChannelTarget;
                self.ui.mask_target = t == ChannelTarget::LayerMask;
                self.ui.vector_mask_target = t == ChannelTarget::VectorMask;
            }
            self.sync_mask_targets();
        }
        result
    }

    pub(crate) fn run_command(&mut self, id: &str, params: Value) -> Result<Value, String> {
        if !vector_ui::cancel_stale(self) {
            return Err(self.ui.status.clone());
        }
        if matches!(id, "paint.setSymmetry" | "paint.symmetryFromPath" | "paint.symmetryDisable") {
            self.ui.symmetry_transform = None;
        }
        let clip_read = std::mem::take(&mut self.clip_read_for_paste);
        if self.automation_input
            && let Some(authorize) = self.services.automation_command.as_ref()
        {
            authorize(id, &params)?;
        }
        // A new layer while Free Transform is open applies the transform first, as Photoshop does
        // (#5667): History reads the transform (or Place Embedded), then New Layer. Only for a
        // command that can run; a transform refused or still being applied keeps the box open and
        // the command doesn't run.
        if photocraft_engine::PRECHECKED.contains(&id) && self.ui.transform.is_some() {
            // Bad params or a command that can't run leave the box as it is.
            self.session.precheck(id, &params).map_err(|e| e.to_string())?;
            transform_tool::commit(self);
            if transform_tool::committing(self).is_some() {
                return Err(tl!("Free Transform is still being applied; try again when it has finished.").into());
            }
            if self.ui.transform.is_some() {
                return Err(if self.ui.status.is_empty() { tl!("The transform couldn't be applied.").into() } else { self.ui.status.clone() });
            }
        }
        actions::suspended_guard(self, id)?;
        if let Some(result) = panel_docking::command(self, id, &params) {
            return result;
        }
        if let Some(r) = transform_tool::intercept(self, id) {
            return r;
        }
        if let Some(r) = crate::crop_ui::intercept(self, id) {
            return r;
        }
        let t0 = gpu_canvas::now_ms();
        // The OS clipboard is read only on an explicit paste, never in the background (privacy, CPU).
        if matches!(id, "edit.paste" | "edit.pasteSpecial.pasteInPlace" | "file.newFromClipboard") {
            if !clip_read {
                self.import_os_clipboard()?;
            }
            if self.session.clipboard.is_none() && self.services.clipboard_get_image.is_some() {
                if self.services.clipboard_unreadable.as_ref().is_some_and(|unreadable| unreadable()) {
                    // The clipboard may well hold an image that can't be reached: say why.
                    notices::clipboard_unreadable(self);
                    return Ok(serde_json::json!({"pasted": false}));
                }
                // Enabled on the strength of the OS clipboard, which held no image: a quiet no-op.
                let message = "Nothing to paste: the clipboard holds no image";
                self.ui.status = tl!(message).into();
                self.ui.status_error = false;
                return Ok(serde_json::json!({"pasted": false}));
            }
        }
        let suppress_events = self.automation_input && self.session.prefs().script_events.enabled;
        if suppress_events {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = false);
        }
        // Long commands become background jobs when enabled (`jobs_ui`); the rest run inline.
        let params = self.with_mask_target(id, params);
        let params = vector_ui::with_active_path(self, id, params);
        let path_mask = vector_ui::takes_path_mask(id) && params.get("path").is_some_and(|v| !v.is_null());
        let on_active_layer = self
            .session
            .active()
            .is_some_and(|st| st.active_layer.is_some_and(|layer| params.get("layer").and_then(Value::as_u64).is_none_or(|target| target == layer.0)));
        let creates_active_mask =
            matches!(id, "layer.layerMask.revealAll" | "layer.layerMask.hideAll" | "layer.layerMask.revealSelection" | "layer.layerMask.hideSelection")
                && on_active_layer;
        let creates_active_vector_mask = matches!(
            id,
            "layer.vectorMask.add" | "layer.vectorMask.fromPath" | "layer.vectorMask.revealAll" | "layer.vectorMask.hideAll" | "layer.vectorMask.currentPath"
        ) && on_active_layer;
        let r = if id == "actions.play" {
            actions::play(self, &params)
        } else if photocraft_engine::actions_cmds::shell_view_command(id) {
            actions::run_view(self, id, params)
        } else {
            jobs_ui::run(self, id, params)
        };
        let creates_adjustment_or_fill = id.starts_with("layer.newAdjustmentLayer.") || id.starts_with("layer.newFillLayer.");
        self.sync_command_side_effects(id, &r, creates_active_mask, creates_active_vector_mask, creates_adjustment_or_fill, path_mask);
        self.perf.last_command = id.to_string();
        self.perf.command_ms = gpu_canvas::now_ms() - t0;
        match &r {
            Ok(_) => {
                if self.ui.status_error {
                    self.ui.status.clear();
                    self.ui.status_error = false;
                }
                self.sync_views();
                let removed = self.session.take_samplers_removed();
                if removed > 0 && !self.ui.status_error {
                    // A canvas-size change (crop, Canvas Size, Image Size, rotation) dropped Color
                    // Sampler points that fell outside the new canvas (#1046).
                    self.ui.status = crate::i18n::trn(crate::i18n::current(), removed as u64, "{n} sample point removed", "{n} sample points removed");
                    self.ui.status_error = false;
                }
            }
            Err(e) => {
                self.ui.status = e.clone();
                self.ui.status_error = true;
            }
        }
        if suppress_events {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
        }
        r
    }

    /// Keep one view per document, in tab order: a view and its windows stay with their document
    /// when tabs move (`document.move`) or close.
    /// The tool pointer events go to: a held temporary tool when there is one (⌘ is the Move
    /// tool, `hold_keys::cmd_moves`), else the selected tool.
    pub fn active_tool(&self) -> state::Tool {
        self.tool_override.unwrap_or(self.ui.tool)
    }

    fn sync_command_side_effects(
        &mut self,
        id: &str,
        result: &Result<Value, String>,
        creates_active_mask: bool,
        creates_active_vector_mask: bool,
        creates_adjustment_or_fill: bool,
        path_mask: bool,
    ) {
        if result.is_ok() && ADDS_LAYER_MASK.contains(&id) {
            // Adding an explicit mask targets it, as in Photoshop (#2166).
            self.ui.mask_target = true;
            self.ui.vector_mask_target = false;
            self.ui.properties_layer = None;
        } else if result.is_ok() && creates_adjustment_or_fill {
            // Panel Options may suppress a fill layer's default mask. Never target a
            // nonexistent mask; an active path creates a vector mask instead.
            let masks = self
                .session
                .active()
                .and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id)))
                .map(|layer| (layer.mask.is_some(), layer.vector_mask.is_some()))
                .unwrap_or((false, false));
            self.ui.mask_target = masks.0;
            self.ui.vector_mask_target = !masks.0 && masks.1;
            // Keep Properties on the new layer contents, as on main (#3369).
            self.ui.properties_layer = self.session.active().and_then(|st| st.active_layer).map(|l| l.0);
        }
        if result.is_ok() && creates_adjustment_or_fill {
            // Properties comes to the front, shown and expanded, so the new layer's settings can
            // be edited at once, as in Photoshop (#4028). The layer is made whatever happens here.
            if self.ui.docking.is_some() {
                let _ = panel_docking::command(self, "window.panel.activate", &json!({"panel": "properties"}));
            } else {
                self.ui.dock_tabs.properties = 0;
                dock::reveal(self, dock::Group::Properties);
            }
        }
        // Making a new selection reveals its outline after View > Extras was hidden (#5597).
        // These interactive selection commands complete inline; a failed command, deselection,
        // or painting inside an existing selection must leave the user's view choice alone.
        if result.as_ref().is_ok_and(|v| v.get("changed") != Some(&Value::Bool(false)) && v.get("pending") != Some(&Value::Bool(true)))
            && matches!(
                id,
                "select.rect"
                    | "select.lasso"
                    | "select.magneticLasso"
                    | "select.magicWand"
                    | "select.quick"
                    | "select.object"
                    | "select.loadSelection"
                    | "select.loadLuminosity"
                    | "path.toSelection"
            )
            && self.session.active().is_some_and(|st| st.doc.selection.is_some())
        {
            self.ui.view.extras = true;
        }
        if result.is_ok() && id == "select.toWorkPath" {
            // Make Work Path selects the new work path in the Paths panel, as in Photoshop.
            self.ui.selected_path = Some("work".into());
        }
        if result.is_ok() && path_mask {
            // The new layer's vector mask becomes the active path, as in Photoshop: the path it
            // was made from is no longer selected, so the next fill layer isn't masked by it too.
            self.ui.selected_path = Some("layer".into());
        }
        if result.is_ok() && creates_active_mask {
            // Adding a mask selects its thumbnail: the next brush or footer Delete targets it.
            self.ui.mask_target = true;
            self.ui.vector_mask_target = false;
        }
        if result.is_ok() && creates_active_vector_mask {
            // Likewise a new vector mask (#4728): Properties shows its Density and Feather, and a
            // mask made from a path becomes the active path.
            self.ui.vector_mask_target = true;
            self.ui.mask_target = false;
            if matches!(id, "layer.vectorMask.currentPath" | "layer.vectorMask.fromPath") {
                self.ui.selected_path = Some("layer".into());
            }
        }
        if result.is_ok() && matches!(id, "edit.copy" | "edit.cut" | "edit.copyMerged") {
            self.clip_external = false;
            self.export_os_clipboard();
        }
    }

    pub fn sync_views(&mut self) {
        type_transform::cancel_stale(self);
        crate::cam_transform::cancel_stale(self);
        crate::lasso_ui::cancel_stale(self);
        // A polygon under construction belongs to the Polygonal Lasso tool, not the next tool.
        if self.ui.tool != Tool::PolygonLasso {
            self.ui.polygon.clear();
            self.ui.polygon_mode.clear();
        }
        // ... and to the document it was started on.
        crate::canvas::polygon_cancel_stale(self);
        crate::crop_ui::cancel_stale(self);
        crate::distort_ui::cancel_stale(self);
        crate::perspective_crop_ui::cancel_stale(self);
        // A pen path under construction belongs to the document it was started on too.
        vector_ui::cancel_stale(self);
        let ids: Vec<DocId> = self.session.documents().iter().map(|d| d.doc.id).collect();
        // Where the document of view `i` is now. Views not tracked yet keep their index.
        let now = |i: usize| match self.view_docs.get(i) {
            Some(id) => ids.iter().position(|d| d == id),
            None => (i < ids.len()).then_some(i),
        };
        let mut views: Vec<Option<state::View>> = ids.iter().map(|_| None).collect();
        for (i, v) in std::mem::take(&mut self.ui.views).into_iter().enumerate() {
            if let Some(slot) = now(i).and_then(|to| views.get_mut(to)) {
                *slot = Some(v);
            }
        }
        self.ui.views = views.into_iter().map(Option::unwrap_or_default).collect();
        self.ui.windows.retain_mut(|w| now(w.document).map(|d| w.document = d).is_some());
        if !self.session.prefs().workspace.open_documents_as_tabs {
            let new_docs: Vec<usize> = ids.iter().enumerate().filter(|(_, id)| !self.view_docs.contains(id)).map(|(i, _)| i).collect();
            for &idx in &new_docs {
                if !self.ui.windows.iter().any(|w| w.document == idx) {
                    crate::view_cmds::float_window(self, idx, idx);
                }
            }
        }
        self.canvases.retain(|(doc, _), _| ids.contains(doc));
        self.navigator_textures.retain(|doc, _| ids.contains(doc));
        self.view_docs = ids;
        self.prune_thumbs();
        self.sync_mask_targets();
        // Channel-view textures outlive a hidden view (cheap re-show), not their document.
        if !self.channel_views.is_empty() {
            let docs = self.session.documents();
            self.channel_views.retain(|id, _| docs.iter().any(|d| d.doc.id.0 == *id));
        }
        // Commands may close and reopen a preserved-ID document before the next repaint.
        canvas::retain_gpu_documents(self);
    }
}

/// Layer › Layer Mask commands that add a layer mask: Photoshop targets the new mask (#2166).
const ADDS_LAYER_MASK: [&str; 5] = [
    "layer.layerMask.revealAll",
    "layer.layerMask.hideAll",
    "layer.layerMask.revealSelection",
    "layer.layerMask.hideSelection",
    "layer.layerMask.fromTransparency",
];
