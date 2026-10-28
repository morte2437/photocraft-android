use super::{
    PhotocraftApp, analysis_ui, brush_panel, brush_picker, camera_raw_ui, canvas, dds_options_ui, dialogs, discard_ui, distort_ui, field_wheel, file_open,
    gpu_canvas, gpu_status, gradient_ui, i18n, jobs_ui, jpeg_options_ui, kys_import, last_brush, layers_panel_ui, monitor_status, native_menu, notices,
    paint_mouse, palette, panels, prefs_ui, preset_panels, rasterize_prompt, screen_picker, shape_stroke_ui, shortcuts, targa_options_ui, text_field_menu,
    theme, tiff_options_ui, timeline_ui, titlebar, tool_cursor, tool_memory, transform_tool, type_panels_ui, type_tool, updates_ui, vector_ui, view_cmds,
    wide_angle_ui, work_area, workspace_ui,
};

impl PhotocraftApp {
    /// Wait out the rest of [`Self::min_frame_ms`] since the last frame started.
    fn cap_frame_rate(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(min) = self.min_frame_ms.filter(|m| m.is_finite() && *m > 0.0) {
            let wait = min.min(100.0) - (gpu_canvas::now_ms() - self.last_frame_start_ms);
            if wait > 0.0 {
                std::thread::sleep(std::time::Duration::from_secs_f64(wait / 1000.0));
            }
        }
        self.last_frame_start_ms = gpu_canvas::now_ms();
    }
}

impl PhotocraftApp {
    /// Shared logic body. A wrapping host owns the final reset tail after its own work.
    pub fn logic_frame(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.cap_frame_rate();
        i18n::set_current(i18n::Lang::from_pref(&self.session.prefs().interface.language));
        // Caps Lock state read from the OS once per frame (see `Services::caps_lock`): the canvas
        // cursor block needs it before the frame renders. `None` (Wayland/web) keeps `false`.
        self.caps_lock = self.services.caps_lock.as_mut().is_some_and(|f| f());
        tool_cursor::set_wayland(ctx, self.services.is_wayland);
        if !self.styled {
            Self::setup_context(ctx, self.ui.theme);
            self.styled = true;
        } else {
            self.fonts_ready = true;
        }
        self.frame += 1;
        let now = ctx.input(|i| i.time);
        let dt = (now - self.last_frame_time) as f32;
        if dt > 0.0 {
            self.fps = self.fps * 0.9 + (1.0 / dt).min(240.0) * 0.1;
        }
        self.last_frame_time = now;
        self.sync_views();
        crate::type_font_preview::follow_hover(self, ctx);
        self.check_gpu(ctx);
        #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
        if self.live_tokens.poll(ctx, self.ui.theme) {
            self.checker = None;
        }
        // Finish or expire outstanding captures first, so a new request admitted in this pass
        // sees their released budget.
        self.drain_captures(ctx);
        self.drain_control(ctx);
        native_menu::run(self, ctx);
        // Switching tools commits, also between the Type and Type Mask tools.
        if self.ui.text_edit.as_ref().is_some_and(|ed| !self.ui.tool.is_type() || ed.mask != self.ui.tool.is_type_mask()) {
            type_tool::commit(self);
        }
        // So does selecting another layer or document by any route (#6483).
        type_tool::end_if_left(self);
        if self.ui.pen.as_ref().is_some_and(|p| !p.drawn_with(self.ui.tool)) {
            vector_ui::pen_commit(self, false);
        }
        transform_tool::end_if_left(self);
        shape_stroke_ui::sync_editor(self, ctx);
        prefs_ui::tick(self, ctx);
        updates_ui::tick(self, ctx);
        monitor_status::poll_frame(self, ctx, frame);
        // Control requests and persisted preferences can change the language in this frame.
        i18n::sync_context(ctx, &self.session.prefs().interface.language);
        type_tool::sync_font_names(ctx, self.session.prefs().type_.show_font_names_in_english);
        // A window bigger than its display (1440 × 900 on 1366 × 768) runs under the taskbar:
        // maximize it into the work area once (#315).
        work_area::fit_window(ctx);
        discard_ui::guard_window_close(self, ctx);
        // Background jobs: apply finished ones, keep frames coming, Esc cancels (before the
        // shortcuts see Esc).
        if !screen_picker::tick(self, ctx) {
            jobs_ui::tick(self, ctx);
            #[cfg(not(target_arch = "wasm32"))]
            crate::dialogs::filter_preview_worker::discard_closed(self);
            if type_tool::in_ui_pass(ctx) {
                if self.fonts_ready {
                    shortcuts::handle(self, ctx);
                } else {
                    type_tool::controls_only(self, ctx);
                }
            }
        }
        let arrived: Vec<(String, Vec<u8>)> =
            self.services.inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for (name, bytes) in arrived {
            if let Err(e) = self.open_bytes(&name, &bytes) {
                self.open_failed(&name, &e);
            }
        }
        // Web drops onto the canvas, read since the last frame: placed below, one at a time.
        let to_place: Vec<(String, Vec<u8>)> =
            self.services.place_inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        self.drop_places.extend(to_place.into_iter().map(|(name, bytes)| file_open::DropPlace::Bytes(name, bytes)));
        // Finder double-click / Open With / Dock drops (macOS open-documents events).
        self.drain_os_events(ctx);
        // While files are dragged over the window it gets no pointer events: keep frames coming so
        // the tab strip can follow the pointer.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            ctx.request_repaint();
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            let at = self.services.cursor_pos.as_mut().and_then(|f| f(ctx));
            self.open_dropped(ctx, dropped, at);
            // Files dragged in from another app's window: PhotoCraft comes to the front and takes
            // the keyboard, as Photoshop does, instead of staying behind the source (#5687).
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.place_next_dropped(ctx);
        // The control transport wakes the UI on arrival (ctx.request_repaint); only poll while a
        // screenshot is pending. (Polling every 50 ms here made idle apps render at 20 fps.)
        if self.captures.any_pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        // A hidden host runs no UI pass: a queued (or re-queued) capture asks for one. An issued
        // capture's own `Screenshot` command forces its paint; asking again would spin a hidden
        // window while a slow readback is in flight. Synthetic input queued during hidden ticks
        // needs the same wake: its events sit in eframe's preserved input until a UI pass.
        if self.captures.any_unissued() || self.synthetic_batch.is_some() || !self.synthetic.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        }
        self.poll_file_dialog(ctx, Some(frame));
    }

    /// Shared UI body. A wrapping host must call `finish_type_reset_pass` after return,
    /// including when this body returns early without drawing an editor.
    pub fn ui_frame(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // One control owner even when this callback cannot draw the custom editor.
        if !self.fonts_ready
            || !self.ui.dialogs.is_empty()
            || screen_picker::busy(&ctx)
            || self.ui.text_edit.is_none()
            || ctx.text_edit_focused()
            || ctx.egui_wants_keyboard_input()
        {
            type_tool::controls_only(self, &ctx);
        }
        // 0.36 saved panel sizes under `Id::unique(name)`; 0.37 reads parent-scoped keys. Data-only,
        // so it runs even before fonts are ready; the old keys stay for a 0.36 rollback.
        panels::migrate_panel_state(&ctx);
        // A real UI pass, not a hidden `run_logic` tick: record the capture geometry (the
        // document identity included) and count one settling pass. `run_logic` leaves these
        // input values stale; only this real UI phase counts toward settling.
        let geometry = crate::screenshot::CaptureGeometry::current(&ctx, self.document_identity());
        self.captures.note_geometry(geometry);
        self.captures.note_pass();
        // A real UI pass consumes the one in-flight synthetic batch (see `inject_synthetic`).
        self.begin_synthetic_pass(&ctx);
        self.hover_layer = None;
        i18n::set_current(i18n::Lang::from_pref(&self.session.prefs().interface.language));
        // Fonts registered via set_fonts only take effect next frame; named families would panic now.
        if !self.fonts_ready {
            // egui consumed this pass's input without PhotoCraft handling it: keep the synthetic
            // batch for the next pass instead of losing the step.
            self.defer_synthetic_pass();
            ctx.request_repaint();
            self.automation_input = false;
            // No editor was drawn this pass: end its IME context rather than keep a stale owner.
            type_tool::sync_ime(self, &ctx);
            return;
        }
        let t0 = gpu_canvas::now_ms();
        self.perf.frame_refresh_ms = 0.0;
        // A software adapter or CPU mode chosen at startup is every document's reason.
        crate::render_path::frame(self);
        self.stroke_trace.cpu_prev(frame.info().cpu_usage);
        // A text field's right-click menu, before any field is drawn (text_field_menu.rs).
        text_field_menu::show(&ctx);
        // The wheel goes to a focused value field before the canvas or a panel sees it (#5473).
        field_wheel::begin_frame(&ctx);
        // Each painting tool keeps its own brush (#218), so switch the active tool's brush in
        // before anything this frame reads it (the cursor, the options bar, a stroke).
        // Preferences ▸ Type ▸ Recent Fonts: a smaller cap (or 0) trims the existing list at
        // once, not only after the next pick.
        self.sync_recent_fonts();
        paint_mouse::sync_tool_brush(self);
        // A unit typed into a value field (`1in` in a px field) converts at the active document's
        // resolution, as in Photoshop.
        let ppi = self.session.prefs().units_and_rulers.point_size.per_inch();
        let dpi = self.session.active().map_or(72.0, |d| f64::from(d.doc.resolution_dpi.max(1.0)));
        crate::widgets::set_field_resolution(&ctx, dpi, ppi);
        // View › Screen Mode › Full Screen Mode: only the image, on black (F or Esc returns).
        screen_picker::show(&ctx);
        if screen_picker::busy(&ctx) && screen_picker::showing(&ctx) {
            egui::Modal::new(egui::Id::unique("screen-color-wait")).show(&ctx, |ui| {
                ui.spinner();
                ui.label(tl!("Pick a screen pixel or press Esc to cancel"));
                if ui.button(tl!("Cancel")).clicked() {
                    screen_picker::cancel(&ctx);
                }
            });
            // This pass handled its input (the modal above ran): deliver the synthetic batch.
            self.finish_synthetic_pass();
            self.automation_input = false;
            type_tool::sync_ime(self, &ctx);
            gpu_status::check_window(self, &ctx);
            return;
        }
        if screen_picker::busy(&ctx) {
            ui.disable();
            ui.set_opacity(1.0);
        }
        // The OS title bar (and the taskbar / Alt-Tab entry) follows the active file; with the
        // system title bar this is where the document name lives, as the in-app title is hidden.
        panels::sync_window_title(self, &ctx);
        let chrome = !self.ui.view.hides_chrome();
        if chrome {
            self.ui.panels.full_screen_chrome = false;
        }
        // In Full Screen Mode, Tab shows the menu bar, options bar, toolbar and dock (#6760).
        let tabbed_in = !chrome && self.ui.panels.full_screen_chrome;
        if (chrome || tabbed_in) && self.ui.panels.menu_bar {
            panels::title_bar(self, ui);
        }
        if (chrome && self.ui.panels.options_bar) || tabbed_in {
            panels::options_bar(self, ui);
        }
        if chrome && self.ui.panels.status_bar {
            panels::status_bar(self, ui);
        }
        if (chrome && self.ui.panels.toolbar) || tabbed_in {
            panels::toolbar(self, ui);
        }
        // craft-ui's layout holds the canvas as its protected `canvas` panel.
        let docking = (chrome && self.ui.panels.dock) || tabbed_in;
        if docking {
            panels::dock_area(self, ui);
        } else {
            let t = theme::Tokens::get(&ctx);
            let backdrop = if chrome { prefs_ui::pasteboard_color(self).unwrap_or(t.canvas) } else { egui::Color32::BLACK };
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(backdrop)).show(ui, |ui| {
                canvas::document_area(self, ui);
            });
            panels::properties_window(self, &ctx);
        }
        brush_panel::window(self, &ctx);
        preset_panels::windows(self, &ctx);
        gradient_ui::editor_window(self, &ctx);
        type_panels_ui::windows(self, &ctx);
        analysis_ui::windows(self, &ctx);
        timeline_ui::windows(self, &ctx);
        workspace_ui::windows(self, &ctx);
        palette::show(self, &ctx);
        dialogs::show(self, &ctx);
        layers_panel_ui::show_dialog(self, &ctx);
        rasterize_prompt::run_pending(self, &ctx);
        jobs_ui::dialog(self, &ctx);
        discard_ui::show(self, &ctx);
        tiff_options_ui::show(self, &ctx);
        targa_options_ui::show(self, &ctx);
        jpeg_options_ui::show(self, &ctx);
        dds_options_ui::show(self, &ctx);
        distort_ui::show(self, &ctx);
        camera_raw_ui::show(self, &ctx);
        wide_angle_ui::show(self, &ctx);
        canvas::extra_windows(self, &ctx);
        notices::show(self, &ctx);
        updates_ui::show(self, &ctx);
        gpu_status::show_fallback(self, &ctx);
        kys_import::show_offer(self, &ctx);
        if self.titlebar_modal_drag(chrome) {
            titlebar::modal_drag_overlay(&ctx);
        }
        if self.custom_titlebar {
            titlebar::resize_zones(ui);
        }
        // A device lost while drawing this frame: switch to the CPU canvas before the next one.
        gpu_status::check(self, &ctx);
        // The Brush Preset picker's view (its gear's card parts and the footer scale) is
        // remembered once the pointer is up. Here, after every panel, rather than in the
        // picker's own code: the control channel can set it while the picker is closed.
        brush_picker::persist(self, &ctx);
        // The tool's brush (settings and tip references) is remembered the same way. Like the
        // picker's view, in the frame loop, so the control channel can set it with the panels
        // closed; the engine's revision counter keeps an unchanged brush free.
        last_brush::persist(self, &ctx);
        // Tool options are remembered between launches the same way (#2814).
        tool_memory::persist(self, &ctx);
        // So are View › Extras, Show and Snap To (#3762).
        view_cmds::persist(self, &ctx);
        // Remember the 0.37 panel sizes so a later 0.36 rollback's newer old-key values win on
        // the next upgrade (see `panels::migrate_panel_state`).
        panels::remember_panel_state(&ctx);
        self.automation_input = false;
        shape_stroke_ui::finish_frame(self, &ctx);
        native_menu::sync(self, &ctx);
        if screen_picker::busy(&ctx) {
            // Block input without a backdrop: the worker may still be capturing the screen.
            egui::Modal::new(egui::Id::unique("screen-color-wait")).frame(egui::Frame::NONE).backdrop_color(egui::Color32::TRANSPARENT).show(&ctx, |_| {});
            ctx.set_cursor_icon(egui::CursorIcon::Wait);
        }
        let ui_ms = gpu_canvas::now_ms() - t0;
        // A GPU measurement lands a frame or two after the refresh it belongs to. Refreshes
        // carry them out, but the last refresh before the app settles has none following it, so
        // drain the timer here too or its number is never reported. Drain it either way: a
        // measurement left sitting would surface later against an unrelated refresh. Report it
        // only while the canvas is still on the GPU, since a CPU refresh clears `gpu_ms`
        // precisely so a GPU number is never shown as that refresh's.
        if let Some(gpu) = &self.gpu
            && let Some(t) = gpu.take_gpu_timing()
            && self.perf.last_refresh.starts_with("gpu")
        {
            self.perf.record_gpu(t);
        }
        self.perf.frame(ui_ms);
        if self.stroke_trace.enabled() {
            self.stroke_trace.canvas(self.perf.frame_refresh_ms);
            let info = &self.perf.gpu_info;
            self.stroke_trace.end_frame(ui_ms, self.live_stroke.is_some(), || {
                let gpu = format!("{} {}", info.backend, info.adapter).trim().to_string();
                if info.present.is_empty() { gpu } else { format!("{gpu}, {}", info.present) }
            });
        }
        // This pass handled its input: the in-flight synthetic batch is delivered once and the
        // next step may be injected on the next frame.
        self.finish_synthetic_pass();
        // Synthetic input is injected one press/release step per frame: keep frames coming until
        // the queue is empty, then release control replies waiting on it.
        // Only a pass that reached the end of the UI and delivered the in-flight batch counts as
        // settled input; an early-returning pass keeps the batch (and the waiting replies).
        if !self.synthetic.is_empty() || self.synthetic_batch.is_some() {
            ctx.request_repaint();
        } else if !self.input_waiters.is_empty() {
            for w in self.input_waiters.drain(..) {
                let _ = w.send(serde_json::json!({"ok": true, "result": null}));
            }
        }
        // A menu or dialog can start a filter after `jobs_ui::tick` ran in this frame. Keep
        // polling it even when the platform would otherwise put the event loop to sleep.
        jobs_ui::keep_polling(self, &ctx);
        // Menus and buttons in this frame may have asked for a file dialog.
        self.poll_file_dialog(&ctx, Some(frame));
        // The Type tool's IME descriptor is the last writer of this pass: every panel and the
        // canvas have finished mutating the model, and an absent/focus-moved editor ends here.
        type_tool::sync_ime(self, &ctx);
        // Issue any due `ui.screenshot` now that this pass's state is settled: a queued request
        // that has waited its passes and animation delay captures exactly this frame.
        self.issue_captures(&ctx);
        // Last, after everything drawn this frame: a lost window device can't draw it (#3438).
        gpu_status::check_window(self, &ctx);
    }
}
