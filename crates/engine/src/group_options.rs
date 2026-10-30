//! The options of Photoshop's New Group dialog (Name, Color, Mode, Opacity), shared by
//! Layer › New › Group… (`layer.new.group`) and Group from Layers (`layer.new.groupFromLayers`,
//! `layer.groupLayers`), so the dialog's OK is a single history step.

use photocraft_color::BlendMode;
use photocraft_doc::{LabelColor, Layer};
use serde_json::Value;

use crate::{EngineError, Result};

/// New Group options parsed from command params; an absent (or `null`) key keeps the default.
#[derive(Debug, Default)]
pub(crate) struct GroupOptions {
    /// `None` (or an empty name) takes the next "Group N".
    pub name: Option<String>,
    color: Option<LabelColor>,
    blend: Option<BlendMode>,
    opacity: Option<f32>,
}

impl GroupOptions {
    /// Parses `name`, `color` (a layer colour id), `mode` (a blend mode, Pass Through included)
    /// and `opacity` (percent, clamped to 0..100). Validates everything before any edit runs.
    pub(crate) fn parse(cmd: &str, p: &Value) -> Result<Self> {
        let bad = |msg: &str| EngineError::BadParams { cmd: cmd.into(), msg: msg.into() };
        let get = |key: &str| p.get(key).filter(|v| !v.is_null());
        let name = match get("name") {
            None => None,
            Some(v) => Some(v.as_str().ok_or_else(|| bad("`name` must be a string"))?.to_string()).filter(|n| !n.trim().is_empty()),
        };
        let color = match get("color") {
            None => None,
            Some(v) => Some(v.as_str().and_then(LabelColor::from_id).ok_or_else(|| {
                bad("`color` must be a layer colour id (none, red, orange, yellow, green, seafoam, blue, indigo, magenta, fuchsia, violet, gray)")
            })?),
        };
        let blend = match get("mode") {
            None => None,
            Some(v) => Some(
                v.as_str()
                    .and_then(crate::commands::blend_from_str)
                    .ok_or_else(|| bad("`mode` must be a blend mode such as \"passThrough\" or \"multiply\""))?,
            ),
        };
        let opacity = match get("opacity") {
            None => None,
            Some(v) => {
                let o = v.as_f64().filter(|o| o.is_finite()).ok_or_else(|| bad("`opacity` must be a number (percent, 0..100)"))?;
                Some((o.clamp(0.0, 100.0) / 100.0) as f32)
            }
        };
        Ok(Self { name, color, blend, opacity })
    }

    /// Sets the mode, opacity and colour of a new `group`. A colour also reaches every layer inside
    /// it, as Layer › Layer Color does on a group; "No Color" leaves the layers' own colours alone.
    pub(crate) fn apply(&self, group: &mut Layer) {
        if let Some(blend) = self.blend {
            group.blend = blend;
        }
        if let Some(opacity) = self.opacity {
            group.opacity = opacity;
        }
        match self.color {
            Some(LabelColor::None) | None => {}
            Some(color) => {
                // Explicit stack: documents can nest groups deeply.
                let mut stack = vec![group];
                while let Some(l) = stack.pop() {
                    l.label = color;
                    if let Some(children) = l.children_mut() {
                        stack.extend(children.iter_mut());
                    }
                }
            }
        }
    }
}

/// `layer.new.group`'s `position`: `below` (the Layers panel's ⌘/Ctrl-click) or `above` (the default).
pub(crate) fn below(cmd: &str, p: &Value) -> Result<bool> {
    match p.get("position").filter(|v| !v.is_null()) {
        None => Ok(false),
        Some(v) => match v.as_str() {
            Some("above") => Ok(false),
            Some("below") => Ok(true),
            _ => Err(EngineError::BadParams { cmd: cmd.into(), msg: "`position` must be above or below".into() }),
        },
    }
}

#[cfg(test)]
mod tests {
    use photocraft_color::BlendMode;
    use photocraft_doc::{Document, LabelColor, Layer, LayerContent, LayerId};
    use serde_json::{Value, json};

    use crate::Session;

    /// Background, then `A` and `B` (`B` active).
    fn session() -> (Session, LayerId, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        let a = id(s.execute("layer.new.layer", json!({"name": "A"})).unwrap());
        let b = id(s.execute("layer.new.layer", json!({"name": "B"})).unwrap());
        (s, a, b)
    }

    fn id(v: Value) -> LayerId {
        LayerId(v["layer"].as_u64().unwrap())
    }

    fn doc(s: &Session) -> &Document {
        &s.active().unwrap().doc
    }

    fn layer(s: &Session, id: LayerId) -> &Layer {
        doc(s).layer(id).unwrap()
    }

    fn expanded(s: &Session, id: LayerId) -> bool {
        matches!(&layer(s, id).content, LayerContent::Group(g) if g.expanded)
    }

    fn steps(s: &Session) -> usize {
        s.active().unwrap().history.entries().len()
    }

    fn top(s: &Session) -> Vec<LayerId> {
        doc(s).layers.iter().map(|l| l.id).collect()
    }

    #[test]
    fn new_group_defaults_and_dialog_options() {
        let (mut s, _, _) = session();
        let g = id(s.execute("layer.new.group", json!({})).unwrap());
        let l = layer(&s, g);
        assert_eq!((l.name.as_str(), l.blend, l.opacity, l.label), ("Group 1", BlendMode::PassThrough, 1.0, LabelColor::None));
        assert!(expanded(&s, g), "an empty new group stays open");
        let before = steps(&s);
        let g = id(s.execute("layer.new.group", json!({"name": "Sky", "color": "red", "mode": "multiply", "opacity": 40})).unwrap());
        let l = layer(&s, g);
        assert_eq!((l.name.as_str(), l.blend, l.label), ("Sky", BlendMode::Multiply, LabelColor::Red));
        assert!((l.opacity - 0.4).abs() < 1e-6);
        assert_eq!(steps(&s), before + 1, "the dialog's OK is one history step");
        // Labels as shown in the dialog work too; opacity is clamped to 0..100 %, a blank name is "Group N".
        let g = id(s.execute("layer.new.group", json!({"name": "  ", "mode": "Linear Dodge (Add)", "opacity": 250})).unwrap());
        let l = layer(&s, g);
        assert_eq!((l.name.as_str(), l.blend, l.opacity), ("Group 2", BlendMode::LinearDodge, 1.0));
        let g = id(s.execute("layer.new.group", json!({"opacity": -5, "name": null, "color": null, "mode": null, "position": null})).unwrap());
        assert_eq!(layer(&s, g).opacity, 0.0);
    }

    #[test]
    fn group_from_layers_is_collapsed_and_takes_the_dialog_options() {
        let (mut s, a, b) = session();
        s.execute("layer.setLabelColor", json!({"layer": a.0, "color": "red"})).unwrap();
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
        let before = steps(&s);
        let g = id(s.execute("layer.new.groupFromLayers", json!({"name": "Pair", "color": "none", "mode": "normal", "opacity": 50})).unwrap());
        assert_eq!(steps(&s), before + 1, "one history step");
        let l = layer(&s, g);
        assert_eq!((l.name.as_str(), l.blend, l.opacity, l.label), ("Pair", BlendMode::Normal, 0.5, LabelColor::None));
        assert!(!expanded(&s, g), "Photoshop shows a group made from layers closed");
        assert_eq!(layer(&s, a).label, LabelColor::Red, "No Color leaves the layers' own colours alone");
        let inspect = s.execute("document.inspect", json!({})).unwrap();
        let row = inspect["layers"].as_array().unwrap().iter().find(|l| l["id"] == json!(g.0)).unwrap().clone();
        assert_eq!(row["expanded"], false, "document.inspect reports it");
        // Undo removes the group; redo brings it back closed.
        s.undo();
        assert!(doc(&s).layer(g).is_none());
        s.redo();
        assert!(!expanded(&s, g));
        // ⌘G with a colour labels the group and every layer in it, like Layer › Layer Color.
        s.undo();
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
        let g = id(s.execute("layer.groupLayers", json!({"color": "green"})).unwrap());
        assert!(!expanded(&s, g));
        for l in [g, a, b] {
            assert_eq!(layer(&s, l).label, LabelColor::Green);
        }
    }

    #[test]
    fn below_puts_the_new_group_under_the_active_layer() {
        let (mut s, a, b) = session();
        let bg = top(&s)[0];
        let g = id(s.execute("layer.new.group", json!({"position": "below"})).unwrap());
        assert_eq!(top(&s), vec![bg, a, g, b]);
        // Inside a group it stays inside, under the active layer.
        s.execute("layer.select", json!({"layer": b.0})).unwrap();
        let outer = id(s.execute("layer.groupLayers", json!({})).unwrap());
        s.execute("layer.select", json!({"layer": b.0})).unwrap();
        let inner = id(s.execute("layer.new.group", json!({"position": "below"})).unwrap());
        let children: Vec<LayerId> = layer(&s, outer).children().unwrap().iter().map(|l| l.id).collect();
        assert_eq!(children, vec![inner, b]);
        // Nothing goes under the Background: the group goes above it instead.
        s.execute("layer.select", json!({"layer": bg.0})).unwrap();
        let g2 = id(s.execute("layer.new.group", json!({"position": "below"})).unwrap());
        assert_eq!(top(&s)[..2], [bg, g2]);
        // With no active layer it goes to the top, like a plain New Group.
        s.execute("select.deselectLayers", json!({})).unwrap();
        let g3 = id(s.execute("layer.new.group", json!({"position": "below"})).unwrap());
        assert_eq!(top(&s).last(), Some(&g3));
    }

    /// A group added inside a clipping mask (⌘-click under a clipped layer, or a plain New Group
    /// on the base) joins the mask, as in Photoshop: the clipped layers keep their base and stay
    /// visible, and nothing needs recompositing.
    #[test]
    fn a_group_added_inside_a_clipping_mask_joins_it() {
        let (mut s, a, b) = session();
        s.execute("layer.setProps", json!({"layer": b.0, "clipped": true})).unwrap();
        s.execute("layer.select", json!({"layer": b.0})).unwrap();
        let g = id(s.execute("layer.new.group", json!({"position": "below"})).unwrap());
        assert_eq!(top(&s)[1..], [a, g, b]);
        assert!(layer(&s, g).clipped, "under a clipped layer the group is clipped too");
        assert_eq!(s.active().unwrap().last_damage, Some(photocraft_geom::Rect::EMPTY), "B still clips to A");
        // On the base itself: the new group goes between A and its clipped layers.
        s.undo();
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        let g = id(s.execute("layer.new.group", json!({})).unwrap());
        assert_eq!(top(&s)[1..], [a, g, b]);
        assert!(layer(&s, g).clipped);
        // Outside a clipping mask nothing changes.
        s.execute("layer.select", json!({"layer": b.0})).unwrap();
        let g = id(s.execute("layer.new.group", json!({})).unwrap());
        assert!(!layer(&s, g).clipped);
    }

    #[test]
    fn bad_options_fail_without_touching_the_document() {
        let (mut s, a, b) = session();
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
        for cmd in ["layer.new.group", "layer.new.groupFromLayers", "layer.groupLayers"] {
            let mut bad = vec![
                json!({"name": 7}),
                json!({"color": 5}),
                json!({"color": "pink"}),
                json!({"color": "Red"}),
                json!({"mode": ""}),
                json!({"mode": 3}),
                json!({"mode": "sparkle"}),
                json!({"opacity": "x"}),
                json!({"opacity": [50]}),
            ];
            if cmd == "layer.new.group" {
                bad.push(json!({"position": "yes"}));
            }
            for p in bad {
                let before = (doc(&s).clone(), s.active().unwrap().revision, steps(&s));
                assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p} must be refused");
                assert_eq!((doc(&s).clone(), s.active().unwrap().revision, steps(&s)), before, "{cmd} {p} changed nothing");
            }
            // A huge but finite opacity clamps to 100 % instead of failing.
            let g = id(s.execute(cmd, json!({"opacity": f64::MAX})).unwrap());
            assert_eq!(layer(&s, g).opacity, 1.0);
            s.undo();
        }
    }
}
