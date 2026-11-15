//! JSONL adapter for interaction-check. Drives the shipped guide dialog and egui frame path.
//! See docs/guide-edit-interactions.md for the pinned runner and scenario command.
use std::io::{self, BufRead, Write};

use photocraft_ui_egui::{PhotocraftApp, Services, canvas, control, dialogs, guide_select, new_guide_ui};
use serde_json::{Value, json};

struct Driver {
    app: PhotocraftApp,
    ctx: egui::Context,
    time: f64,
}

impl Driver {
    /// Create a fresh synthetic document and two guides for each independent scenario.
    fn new() -> Result<Self, String> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.run("file.new", json!({"width":400,"height":200,"resolution":100}))?;
        app.run("prefs.set", json!({"path":"unitsAndRulers.rulers","value":"pixels"}))?;
        app.run("view.newGuide", json!({"orientation":"vertical","position":10}))?;
        app.run("view.newGuide", json!({"orientation":"vertical","position":20}))?;
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let mut driver = Self { app, ctx, time: 0.0 };
        driver.frames(false);
        Ok(driver)
    }

    /// Run real egui canvas and dialog frames, optionally delivering Escape on the first.
    fn frames(&mut self, escape: bool) {
        self.app.sync_views();
        for frame in 0..3 {
            self.time += 1.0 / 60.0;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
                time: Some(self.time),
                ..Default::default()
            };
            if escape && frame == 0 {
                input.events.push(egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut output = self.ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                egui::CentralPanel::default().show(ui, |ui| canvas::document_area(&mut self.app, ui));
                dialogs::show(&mut self.app, &ctx);
            });
            // Semantic checks do not upload textures; screenshot checks use the snapshot example.
            output.textures_delta.clear();
        }
    }

    /// Locate the current guide dialog, returning an error after it has closed.
    fn dialog(&self) -> Result<u64, String> {
        self.app.ui.dialogs.iter().find(|d| d.fields.contains_key("__edit")).map(|d| d.id).ok_or("no guide dialog".into())
    }

    /// Dispatch one scenario event through the shipped UI or engine paths.
    fn event(&mut self, event: &Value) -> Result<(), String> {
        match event.get("op").and_then(Value::as_str) {
            Some("begin") => {
                guide_select::select(&mut self.app, true, 0);
                new_guide_ui::open_edit(&mut self.app)?;
            }
            Some("set") => {
                let params = json!({"dialog":self.dialog()?,"field":event.get("field"),"value":event.get("value")});
                let (req, _rx) = control::ControlRequest::new("ui.dialog.set", params);
                match control::handle(&mut self.app, &self.ctx, &req) {
                    control::Outcome::Done(v) if v.get("ok") == Some(&json!(true)) => {}
                    _ => return Err("ui.dialog.set failed".into()),
                }
            }
            Some("confirm") => {
                let id = self.dialog()?;
                dialogs::confirm(&mut self.app, id)?;
            }
            Some("cancel") => {
                let id = self.dialog()?;
                dialogs::cancel(&mut self.app, id)?;
            }
            Some("escape") => self.frames(true),
            Some("command") => {
                let id = event.get("id").and_then(Value::as_str).ok_or("command needs id")?;
                self.app.run(id, event.get("params").cloned().unwrap_or(json!({})))?;
            }
            Some("undo") if self.app.session.undo() => {}
            Some("redo") if self.app.session.redo() => {}
            Some("other_document") => {
                self.app.run("file.new", json!({"width":400,"height":200}))?;
                self.app.run("view.newGuide", json!({"orientation":"vertical","position":10}))?;
            }
            _ => return Err("unknown or unavailable event".into()),
        }
        Ok(())
    }

    /// Report observable document, preview, dialog and history state after an event.
    fn state(&self, result: Result<(), String>) -> Value {
        let active = self.app.session.active();
        let preview = active.and_then(|st| new_guide_ui::edit_preview(&self.app, &st.doc));
        let dialog = self.app.ui.dialogs.iter().find(|d| d.fields.contains_key("__edit"));
        json!({
            "rejected": result.is_err(), "error": result.err(),
            "vertical": active.map(|st| &st.doc.guides.vertical),
            "horizontal": active.map(|st| &st.doc.guides.horizontal),
            "colors": active.map(|st| &st.doc.guides.colors),
            "preview": preview,
            "history": active.map(|st| st.history.past_len()),
            "redo": active.is_some_and(|st| st.history.can_redo()),
            "editing": dialog.is_some(),
            "invalid": dialog.is_some_and(|d| d.fields.contains_key("__error")),
            "editCommands": self.app.session.journal.iter().filter(|(id, _)| id == "view.editGuide").count(),
        })
    }
}

/// Exchange one JSONL response per validated protocol request until stdin closes.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut driver = Driver::new()?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        if request.get("protocol") != Some(&json!(1)) || request.get("id").and_then(Value::as_u64).is_none() {
            return Err("expected interaction-check protocol 1 and an unsigned id".into());
        }
        let event = request.get("event").ok_or("missing event")?;
        let result = driver.event(event);
        driver.frames(false);
        let response = json!({"protocol":1,"id":request.get("id"),"state":driver.state(result)});
        writeln!(out, "{response}")?;
        out.flush()?;
    }
    Ok(())
}
