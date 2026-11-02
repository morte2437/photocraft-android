use super::services::{ExportSettings, FormatChoices, SAVE_OPTIONS_OPEN, SaveKind, Services, plain_raster, saved_revision_after_write};
use super::{
    ControlRequest, DocId, Document, HashMap, PhotocraftApp, Receiver, Session, SyntheticTarget, Tool, UiState, Value, actions, analysis_ui, brush_panel,
    brush_picker, camera_raw_ui, canvas, dds_options_ui, delete_layer_prompt, dialogs, discard_ui, distort_ui, dock, field_wheel, file_open, gpu_canvas,
    gpu_status, gradient_ui, histogram_panel, i18n, jobs_ui, jpeg_options_ui, json, kys_import, last_brush, layers_panel_ui, mask_thumbs_ui, mask_trash_prompt,
    monitor_status, native_menu, notices, open_prompt, paint_mouse, palette, panel_docking, panels, prefs_ui, preset_files_ui, preset_panels, rasterize_prompt,
    screen_picker, shape_stroke_ui, shortcuts, single_line_drag, smart_svg_ui, state, stroke_trace, stylus, targa_options_ui, text_field_menu, theme,
    tiff_options_ui, timeline_ui, titlebar, tool_cursor, tool_memory, transform_tool, type_panels_ui, type_tool, type_transform, updates_ui, vector_ui,
    view_cmds, wheel_nav, wide_angle_ui, widgets, work_area, workspace_ui,
};

mod clipboard;
mod commands;
mod control;
mod files;
mod frame;
mod gpu;
mod ime;
mod input;
mod prefs;
mod screenshot;
mod targets;
mod thumbs;
mod view;
mod window;

pub(crate) use files::remember_new_document;
pub(crate) use input::read_dropped;
pub(crate) use targets::adjustment_layer_active;
#[cfg(test)]
pub(crate) use thumbs::layer_thumb_frame;
pub use thumbs::surface_fingerprint;

impl PhotocraftApp {
    pub fn new(mut session: Session, mut services: Services) -> Self {
        session.updates.check = services.check_updates.take();
        let mut app = Self {
            session,
            ui: UiState::default(),
            services,
            canvases: HashMap::new(),
            navigator_textures: HashMap::new(),
            display_memo: HashMap::new(),
            navigator_gpu: HashMap::new(),
            navigator_plain: std::collections::HashSet::new(),
            info_samplers: None,
            monitors: Default::default(),
            checker: None,
            drag: None,
            cancelled_vector_drag: false,
            move_blocked: false,
            tool_override: None,
            live_stroke: None,
            committed_stroke: None,
            retouch_from_live: false,
            trail: None,
            remove_pending: Vec::new(),
            remove_pending_target: None,
            remove_trail: None,
            menu_cache: None,
            move_preview: None,
            move_hover: None,
            move_distance: None,
            object_finder: None,
            blend_preview: None,
            reorder_preview: None,
            reorder_preview_hit: false,
            patch_preview: None,
            cam_preview: None,
            magnetic: Default::default(),
            secondary_erase: false,
            held_erase: false,
            defer_live_stroke: false,
            press_stroke: false,
            last_stroke_end: None,
            brush_resize: None,
            quick_pick: false,
            brush_resize_armed: false,
            alt_sampling: false,
            mixer_sampling: false,
            caps_lock: false,
            opacity_keys: None,
            control_rx: None,
            captures: crate::screenshot::Captures::default(),
            type_ime: type_tool::ImeState::default(),
            input_waiters: Vec::new(),
            live_adjust: None,
            frame: 0,
            notices_hovered: false,
            styled: false,
            integrated_titlebar: false,
            custom_titlebar: false,
            last_window_title: String::new(),
            fonts_ready: false,
            last_canvas_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            arrange_start: 0,
            ppp: 1.0,
            drop_canvas_rect: None,
            tab_strip: None,
            drop_places: Default::default(),
            view_docs: Vec::new(),
            path_document: None,
            fps: 0.0,
            last_frame_time: 0.0,
            thumbs: HashMap::new(),
            thumb_frames: HashMap::new(),
            snapshot_thumbs: HashMap::new(),
            thumb_documents: Vec::new(),
            bounds_cache: HashMap::new(),
            proxy: None,
            proxy_uploaded: None,
            outline_cache: None,
            filter_preview: None,
            smart_filter_targets: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            filter_preview_worker: Default::default(),
            redraws: Default::default(),
            kept: Default::default(),
            color_range: None,
            adjust_preview: None,
            live_adjust_preview: None,
            synthetic: Vec::new(),
            synthetic_batch: None,
            synthetic_reinject: false,
            synthetic_snapshot: None,
            automation_input: false,
            run_depth: 0,
            paused_playbacks: Default::default(),
            channel_thumbs: None,
            channel_views: HashMap::new(),
            type_layout: None,
            type_clicks: None,
            type_transform_preview: None,
            type_font_preview: None,
            color_picker_preview: None,
            guide_drag: None,
            layer_box: None,
            crop: Default::default(),
            pcrop: Default::default(),
            hover_doc: None,
            hover_layer: None,
            canvas_hover_layer: None,
            clone_preview: None,
            info_sample: None,
            os_clip_sig: None,
            clip_unmirrored: true,
            clip_external: false,
            clip_read_for_paste: false,
            smart_svg: Default::default(),
            transform_preview: None,
            move_mods: Default::default(),
            nudge_bundle: Default::default(),
            style_preview: None,
            text_style_preview: None,
            solid_fill_preview: None,
            pattern_fill_preview: None,
            shape_fill_preview: None,
            widget_colors: Vec::new(),
            distort: Default::default(),
            gradient: Default::default(),
            camera_raw: None,
            pending_raw_open: None,
            raw_redevelop: Vec::new(),
            wide_angle: None,
            dialog_edits: Default::default(),
            pending_after_prompt: None,
            tone_hist: None,
            doc_hist: None,
            doc_hist_preview: None,
            hist_job: None,
            hist_uncached: None,
            gpu: None,
            window_device: None,
            window_lost: false,
            window_recovery_retry: None,
            gpu_render_state: None,
            gpu_fault: None,
            gpu_oom_retries: 0,
            started: None,
            perf: Default::default(),
            stroke_trace: stroke_trace::StrokeTrace::from_env(),
            render_path: Default::default(),
            slow_frames: Default::default(),
            prefs_rt: Default::default(),
            updates_rt: Default::default(),
            discard: None,
            file_dialog: None,
            tiff_options: None,
            save_dirs: HashMap::new(),
            disk_stamps: HashMap::new(),
            targa_options: None,
            jpeg_options: None,
            dds_options: None,
            held_opens: Default::default(),
            held_preset_import: None,
            allow_close: false,
            stylus: Default::default(),
            control_click: Default::default(),
            background_jobs: false,
            min_frame_ms: None,
            last_frame_start_ms: 0.0,
            jobs: Default::default(),
            #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
            live_tokens: theme::live::LiveTokens::from_env(),
        };
        // Saved preferences are in place before the first frame; recovery starts in upkeep.
        prefs_ui::load(&mut app);
        layers_panel_ui::restore_options(&mut app);
        histogram_panel::restore_options(&mut app);
        // After the saved preferences, which say whether the set was already offered.
        kys_import::offer_import(&mut app);
        notices::wayland_file_drop_guidance(&mut app);
        // File › Scripts › Script Events Manager: "Start Application".
        photocraft_engine::automate_cmds::fire_event(&mut app.session, "startApplication");
        app
    }
}

impl eframe::App for PhotocraftApp {
    fn on_exit(&mut self) {
        if let Err(err) = tool_memory::save_on_exit(self) {
            log::warn!("Preferences: could not save the final tool selection: {err}");
        }
        // Release a pending screenshot's image budget and answer its control request; the
        // callback is inert afterwards (it holds only a weak slot).
        let finished = self.captures.cancel_all("the application is closing");
        self.finish_captures(finished);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.logic_frame(ctx, frame);
        self.finish_type_reset_logic(ctx);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let frame_start = gpu_canvas::now_ms();
        // An undecorated window gets its caption back when the toolkit changes its flags
        // (maximize, restore, …): take it off again before the frame is laid out (#2246).
        if let Some(window_frame) = self.services.window_frame.as_mut() {
            window_frame();
        }
        // Native menu key equivalents become the key presses they were (see `native_menu`).
        if let Some(menu) = self.services.native_menu.as_mut() {
            menu.raw_input(raw_input);
        }
        // While a text field of a floating panel's own window has the keyboard, the dock area
        // hands that window the text keys in this input and converts them there.
        if craft_ui::docking::typing_window(ctx).is_none() {
            shortcuts::clipboard_keys(ctx, ctx.text_edit_focused() || self.ui.text_edit.is_some(), raw_input);
        }
        // Windows precision touchpads and some Linux Wayland compositors send pinches
        // as fractional Ctrl + wheel rather than a native Zoom event (#4566).
        // Leave Ctrl+whole-notch mouse scrolling alone, and honour the pinch preference.
        if cfg!(any(target_os = "windows", target_os = "linux")) && self.session.prefs().enhanced_controls.zoom_with_trackpad_pinch {
            wheel_nav::fold_legacy_pinch(ctx, raw_input);
        }
        self.inject_synthetic(ctx, raw_input);
        // After the control channel's own events (`ui.type`) join the OS's, before any widget.
        widgets::cap_text_events(&mut raw_input.events);
        self.control_click.normalize(&mut raw_input.events, cfg!(target_os = "macos"));
        // A pen held still is no touch screen's press-and-hold right click (stylus.rs).
        let pen_down = self.stylus.pen_contacts.observe(&raw_input.events);
        stylus::pen_hold_is_no_right_click(ctx, pen_down);
        if self.custom_titlebar {
            titlebar::release_after_os_resize(ctx, raw_input);
        }
        // The OS's moves and the control channel's (injected just above).
        let moves = raw_input.events.iter().filter(|e| matches!(e, egui::Event::PointerMoved(_))).count();
        self.perf.stage.start(frame_start, moves);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.ui_frame(ui, frame);
        self.finish_type_reset_pass(ui.ctx());
    }
}

#[cfg(test)]
mod clipboard_tests;
