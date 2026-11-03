//! The smart filter authorization tests' fixture: a workspace with a striped image to place and
//! maps and LUTs outside it (`#[path]`-included by `it/smart_filter_authorization.rs` and
//! `smart_filter_stack_cache.rs`).
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use photocraft_automation::{AuthorizedWorkspace, Headless};
use photocraft_doc::{Document, LayerContent, LayerId};
use serde_json::{Value, json};

pub struct Fixture(PathBuf);

impl Fixture {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("pc-filter-auth-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&base).unwrap();
        std::fs::create_dir(base.join("workspace")).unwrap();
        std::fs::create_dir(base.join("outside")).unwrap();
        let f = Self(base);
        std::fs::write(f.root().join("stripes.png"), png(|x| if (x / 2) % 2 == 0 { 0 } else { 255 })).unwrap();
        std::fs::write(f.outside("black.png"), png(|_| 0)).unwrap();
        std::fs::write(f.outside("white.png"), png(|_| 255)).unwrap();
        std::fs::write(f.outside("black.cube"), cube(0)).unwrap();
        std::fs::write(f.outside("white.cube"), cube(1)).unwrap();
        f
    }

    pub fn root(&self) -> PathBuf {
        self.0.join("workspace")
    }
    pub fn outside(&self, name: &str) -> PathBuf {
        self.0.join("outside").join(name)
    }
    pub fn headless(&self) -> Headless {
        Headless::with_workspace(AuthorizedWorkspace::new(Some(&self.root()), Some(&self.root())).unwrap())
    }
    pub fn setup(&self, lookup: bool) -> (Headless, LayerId) {
        let mut h = self.headless();
        h.handle("doc.new", json!({"width": 32, "height": 32, "background": "#808080"})).unwrap();
        h.handle("doc.new", json!({"width": 32, "height": 32})).unwrap();
        let layer = LayerId(h.handle("doc.place", json!({"path": "stripes.png"})).unwrap()["layer"].as_u64().unwrap());
        let (id, params) = if lookup {
            ("image.adjustments.colorLookup", json!({"layer": layer.0, "lut": "none"}))
        } else {
            ("filter.distort.displace", json!({"layer": layer.0, "mapDocument": 0, "horizontal": 50, "vertical": 0}))
        };
        h.session.execute(id, params).unwrap();
        (h, layer)
    }
    pub fn saved_filter(&self, lookup: bool) -> (Document, LayerId) {
        let (h, layer) = self.setup(lookup);
        let mut doc = (*h.session.active().unwrap().doc).clone();
        let LayerContent::Smart(sm) = &mut doc.layer_mut(layer).unwrap().content else { panic!("smart object") };
        sm.smart_filters[0].params = path_params(lookup, &self.outside(if lookup { "white.cube" } else { "white.png" }));
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        std::fs::write(self.root().join("stored.pcraft"), bytes).unwrap();
        (doc, layer)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn png(value: impl Fn(u32) -> u8) -> Vec<u8> {
    let mut pixels = Vec::new();
    for _ in 0..32 {
        for x in 0..32 {
            pixels.extend([value(x); 3]);
        }
    }
    let image = photocraft_codecs::Image::from_u8(32, 32, photocraft_codecs::ChannelLayout::Rgb, pixels).unwrap();
    photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap()
}
pub fn cube(value: u8) -> String {
    format!("LUT_3D_SIZE 2\n{}", format!("{value} {value} {value}\n").repeat(8))
}
pub fn path_params(lookup: bool, path: &Path) -> Value {
    if lookup { json!({"file": path}) } else { json!({"mapPath": path, "mapDocument": null}) }
}
pub fn assert_denied<T: std::fmt::Debug>(result: Result<T, impl std::fmt::Display>) {
    match result {
        Err(error) => assert!(error.to_string().contains("ambient filesystem"), "{error}"),
        Ok(value) => panic!("ambient file access was accepted: {value:?}"),
    }
}
