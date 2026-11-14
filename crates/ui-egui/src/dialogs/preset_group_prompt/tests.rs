//! The documented dialog API must reject malformed answers without losing the pending import.
use super::*;
use crate::control::{ControlRequest, Outcome, handle};

#[derive(Clone, Copy, Debug)]
enum Kind {
    Brushes,
    Gradients,
}

struct Fixture {
    app: Box<PhotocraftApp>,
    ctx: egui::Context,
    kind: Kind,
    name: &'static str,
    stem: &'static str,
    bytes: Vec<u8>,
    original: Value,
    edited: Value,
    unrelated: Value,
}

fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (request, _reply) = ControlRequest::new(method, params);
    match handle(app, ctx, &request) {
        Outcome::Done(value) => value,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

fn group(app: &PhotocraftApp, kind: Kind, name: &str) -> Value {
    match kind {
        Kind::Brushes => json!(app.session.tools.presets.iter().filter(|p| p.group == name).collect::<Vec<_>>()),
        Kind::Gradients => json!(app.session.presets.gradients.iter().filter(|g| g.name == name).collect::<Vec<_>>()),
    }
}

fn library(app: &PhotocraftApp) -> Value {
    json!({"brushes": app.session.tools.presets, "gradients": app.session.presets.gradients, "selectedBrush": app.session.tools.brush})
}

fn held(app: &PhotocraftApp) -> Value {
    app.held_preset_import.as_ref().map(|h| json!({"cmd":h.cmd,"what":h.what,"stem":h.stem,"params":h.params})).unwrap_or(Value::Null)
}

impl Fixture {
    /// Import two distinct presets, edit one, and seed a neighboring group with the same file.
    fn new(kind: Kind) -> Self {
        let (name, stem, bytes) = match kind {
            Kind::Brushes => {
                use craft_psd::abr::{LegacyBrush, LegacyTip, write_v12};
                let brush = |diameter| LegacyBrush {
                    name: String::new(),
                    spacing: 25,
                    anti_alias: true,
                    tip: LegacyTip::Computed { diameter, hardness: 100, angle: 0, roundness: 100 },
                };
                ("Kit.abr", "Kit", write_v12(2, &[brush(9), brush(17)], true).unwrap())
            }
            Kind::Gradients => {
                use craft_psd::grd::{GrdColor, GrdGradient, GrdStop, write};
                let gradient = |name: &str, rgb| GrdGradient {
                    name: name.into(),
                    stops: vec![
                        GrdStop { location: 0.0, midpoint: 0.5, color: GrdColor::Foreground },
                        GrdStop { location: 1.0, midpoint: 0.5, color: GrdColor::Rgb(rgb) },
                    ],
                    ..Default::default()
                };
                ("Fire.grd", "Fire", write(&[gradient("Ember", [1.0, 0.0, 0.0]), gradient("Tide", [0.0, 0.0, 1.0])]))
            }
        };
        let mut app = Box::new(PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()));
        app.open_bytes(name, &bytes).unwrap();
        let original = group(&app, kind, stem);
        match kind {
            Kind::Brushes => {
                assert_eq!(original.as_array().unwrap().len(), 2);
                app.session.tools.presets.iter_mut().find(|p| p.group == stem).unwrap().brush.size = 77.0;
            }
            Kind::Gradients => {
                assert_eq!(original[0]["items"].as_array().unwrap().len(), 2);
                app.session.presets.gradients.iter_mut().find(|g| g.name == stem).unwrap().items[0].name = "Edited gradient".into();
            }
        }
        let edited = group(&app, kind, stem);
        assert_ne!(edited, original, "the fixture contains a user's edit");
        app.open_bytes(
            match kind {
                Kind::Brushes => "Unrelated.abr",
                Kind::Gradients => "Unrelated.grd",
            },
            &bytes,
        )
        .unwrap();
        let unrelated = group(&app, kind, "Unrelated");
        Self { app, ctx: egui::Context::default(), kind, name, stem, bytes, original, edited, unrelated }
    }

    fn open(&mut self) -> u64 {
        self.app.open_bytes(self.name, &self.bytes).unwrap();
        let dialog = self.app.ui.dialogs.iter().find(|d| owns(&d.fields)).unwrap();
        let id = dialog.id;
        let response = call(&mut self.app, &self.ctx, "ui.inspect", json!({}));
        assert_eq!(response["ok"], true);
        assert!(response["result"]["dialogs"].as_array().unwrap().iter().any(|d| d["id"] == id));
        id
    }

    fn answer(&mut self, id: u64, choice: &str) {
        let set = call(&mut self.app, &self.ctx, "ui.dialog.set", json!({"dialog":id,"field":"choice","value":choice}));
        assert_eq!(set["ok"], true, "{set}");
        let reply = call(&mut self.app, &self.ctx, "ui.dialog.confirm", json!({"dialog":id}));
        assert_eq!(reply["ok"], true, "{reply}");
        self.check_import(id, choice == "replace");
    }

    /// Compare every imported entry and check that a second confirmation cannot replay the import.
    fn check_import(&mut self, id: u64, replace: bool) {
        assert!(self.app.held_preset_import.is_none());
        assert!(self.app.ui.dialogs.iter().all(|d| d.id != id));
        assert_eq!(group(&self.app, self.kind, "Unrelated"), self.unrelated);
        if replace {
            let mut expected = self.original.clone();
            if matches!(self.kind, Kind::Gradients) {
                // The fixture renamed Fire's Ember before importing Unrelated, which now owns
                // Ember. Replace frees only Fire's names, so the new gradient must be Ember 2.
                assert_eq!(self.unrelated[0]["items"][0]["name"], "Ember");
                expected[0]["items"][0]["name"] = json!("Ember 2");
            }
            assert_eq!(group(&self.app, self.kind, self.stem), expected, "only the requested group is replaced with the file's values");
            assert_eq!(group(&self.app, self.kind, &format!("{} (2)", self.stem)), json!([]));
        } else {
            assert_eq!(group(&self.app, self.kind, self.stem), self.edited, "a new group keeps the existing edits");
            let sibling = format!("{} (2)", self.stem);
            let mut expected = self.original.clone();
            match self.kind {
                Kind::Brushes => {
                    // Computed ABR tips get names from their diameters; two groups already own them.
                    expected[0]["name"] = json!("9 px Round 3");
                    expected[1]["name"] = json!("17 px Round 3");
                    expected[0]["group"] = json!(sibling);
                    expected[1]["group"] = json!(sibling);
                }
                Kind::Gradients => {
                    // Fire's renamed Ember leaves its base name to Unrelated; both groups still own a Tide.
                    expected[0]["name"] = json!(sibling);
                    expected[0]["items"][0]["name"] = json!("Ember 2");
                    expected[0]["items"][1]["name"] = json!("Tide 3");
                }
            }
            assert_eq!(group(&self.app, self.kind, &sibling), expected, "every file entry is imported once into the sibling");
            assert_eq!(group(&self.app, self.kind, &format!("{} (3)", self.stem)), json!([]));
        }
        let after = library(&self.app);
        let again = call(&mut self.app, &self.ctx, "ui.dialog.confirm", json!({"dialog":id}));
        assert_eq!(again["ok"], false, "the consumed answer cannot import twice");
        assert_eq!(library(&self.app), after);
    }
}

/// A malformed answer must preserve the pending bytes and allow either valid answer on retry.
fn malformed_choice(kind: Kind) {
    for invalid in [json!("replcae"), json!(""), Value::Null, json!(false), json!(17), json!([]), json!({})] {
        for corrected in ["new", "replace"] {
            let mut f = Fixture::new(kind);
            let id = f.open();
            let before = library(&f.app);
            let pending = held(&f.app);
            let set = call(&mut f.app, &f.ctx, "ui.dialog.set", json!({"dialog":id,"field":"choice","value":invalid}));
            assert_eq!(set["ok"], true, "{set}");
            let dialog = f.app.ui.dialogs.iter().find(|d| d.id == id).unwrap().clone();
            let reply = call(&mut f.app, &f.ctx, "ui.dialog.confirm", json!({"dialog":id}));
            assert_eq!(reply["ok"], false, "{kind:?} {invalid}: malformed choice must reject, {reply}");
            assert!(reply["error"].is_string());
            assert_eq!(library(&f.app), before, "rejection must not change either library or the selected brush");
            assert_eq!(held(&f.app), pending, "keep the exact pending command and input for retry");
            assert_eq!(f.app.ui.dialogs.iter().find(|d| d.id == id), Some(&dialog), "preserve the same dialog and fields");
            let inspect = call(&mut f.app, &f.ctx, "ui.inspect", json!({}));
            assert!(inspect["result"]["dialogs"].as_array().unwrap().iter().any(|d| d["id"] == id && d["fields"]["choice"] == invalid));
            f.answer(id, corrected);
        }
    }
}

#[test]
fn malformed_brush_choice_preserves_api_retry() {
    malformed_choice(Kind::Brushes);
}

#[test]
fn malformed_gradient_choice_preserves_api_retry() {
    malformed_choice(Kind::Gradients);
}

/// The visible default, explicit answers and Cancel all use the dialog API.
#[test]
fn legitimate_preset_choices_use_the_dialog_api() {
    for kind in [Kind::Brushes, Kind::Gradients] {
        for choice in ["default", "new", "replace", "cancel"] {
            let mut f = Fixture::new(kind);
            let id = f.open();
            match choice {
                "default" => {
                    let reply = call(&mut f.app, &f.ctx, "ui.dialog.confirm", json!({"dialog":id}));
                    assert_eq!(reply["ok"], true, "{reply}");
                    f.check_import(id, false);
                }
                "cancel" => {
                    let before = library(&f.app);
                    let reply = call(&mut f.app, &f.ctx, "ui.dialog.cancel", json!({"dialog":id}));
                    assert_eq!(reply["ok"], true, "{reply}");
                    assert_eq!(library(&f.app), before);
                    assert!(f.app.held_preset_import.is_none());
                    assert!(f.app.ui.dialogs.iter().all(|d| d.id != id));
                }
                _ => f.answer(id, choice),
            }
        }
    }
}

/// A legacy prompt with no choice field still imports all entries into a new group.
#[test]
fn absent_preset_choice_keeps_the_new_group_default() {
    for kind in [Kind::Brushes, Kind::Gradients] {
        let mut f = Fixture::new(kind);
        let id = f.open();
        // The API's omitted `value` means present Null, not removal. Set up truly absent
        // legacy state directly, then confirm it through the real dialog API.
        f.app.ui.dialog_mut(id).unwrap().fields.remove("choice");
        let reply = call(&mut f.app, &f.ctx, "ui.dialog.confirm", json!({"dialog":id}));
        assert_eq!(reply["ok"], true, "{reply}");
        f.check_import(id, false);
    }
}
