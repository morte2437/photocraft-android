//! Each kind of place that follows a path stored in a document goes through [`super::resolve`]: a
//! link to another computer is never followed, and a relative link next to the document is.

use std::path::{Path, PathBuf};

use photocraft_doc::{LayerContent, LayerId, SmartSource};
use photocraft_geom::Rect;
use serde_json::json;

use crate::Session;

/// A link to a share on a TEST-NET-1 address, which never answers.
const REMOTE: &str = r"\\192.0.2.1\share\x.png";

/// An empty folder of its own (tests run in parallel).
fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("photocraft-doc-path-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A 32×32 PNG whose pixel at column `x` is `rgb(x)`.
fn png(path: &Path, rgb: impl Fn(u32) -> [u8; 3]) {
    let mut px = Vec::new();
    for _ in 0..32 {
        for x in 0..32 {
            px.extend(rgb(x));
        }
    }
    let image = photocraft_codecs::Image::from_u8(32, 32, photocraft_codecs::ChannelLayout::Rgb, px).unwrap();
    std::fs::write(path, photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
}

/// On Unix `//` followed by an absolute path names that same local file, while the resolver sees a
/// path on another computer: a place that skipped the resolver would read the file.
#[cfg(unix)]
fn network_alias(p: &Path) -> String {
    format!("/{}", p.display())
}

/// A 32×32 white document saved (as far as its links know) at `d/doc.pcraft`, with `file` placed
/// linked → (session, layer id).
fn document(d: &Path, file: &Path) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s.active_mut().unwrap().path = Some(d.join("doc.pcraft").to_string_lossy().into_owned());
    s.execute("file.placeLinked", json!({"path": file.to_string_lossy()})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    (s, id)
}

/// Point smart object `id` at `path` without re-rendering it.
fn relink(s: &mut Session, id: LayerId, path: &str) {
    s.edit("Relink", |doc, _| {
        if let Some(LayerContent::Smart(sm)) = doc.layer_mut(id).map(|l| &mut l.content) {
            sm.source = SmartSource::Linked { path: path.into() };
        }
        Ok(())
    })
    .unwrap();
}

fn centre(s: &Session) -> [f32; 3] {
    let p = photocraft_compose::render(&s.active().unwrap().doc, Rect::new(16, 16, 17, 17)).px[0];
    [p[0], p[1], p[2]]
}

const RED: [f32; 3] = [1.0, 0.0, 0.0];
const BLUE: [f32; 3] = [0.0, 0.0, 1.0];

fn update(s: &mut Session) -> Result<serde_json::Value, String> {
    s.execute("layer.smartObjects.updateModifiedContent", json!({})).map_err(|e| e.to_string())
}

#[test]
fn linked_smart_objects_follow_only_local_files() {
    let d = dir("links");
    png(&d.join("red.png"), |_| [255, 0, 0]);
    png(&d.join("blue.png"), |_| [0, 0, 255]);
    let (mut s, id) = document(&d, &d.join("red.png"));
    assert_eq!(centre(&s), RED);

    relink(&mut s, id, REMOTE);
    s.start_link_scan();
    assert!(s.link_scan.is_none(), "no look at a file on another computer");
    assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated": []}));
    let err = update(&mut s).unwrap_err();
    assert!(err.contains("is not opened: the path is on another computer"), "{err}");
    assert_eq!(centre(&s), RED, "the layer keeps its rendering");
    #[cfg(unix)]
    {
        relink(&mut s, id, &network_alias(&d.join("blue.png")));
        assert!(update(&mut s).is_err());
        assert_eq!(centre(&s), RED, "the file was not read");
    }

    // A relative link is taken from the document's folder.
    relink(&mut s, id, "blue.png");
    update(&mut s).unwrap();
    assert_eq!(centre(&s), BLUE);
    s.start_link_scan();
    assert!(s.link_scan.is_some(), "a followed link is looked at");
}

#[test]
fn package_copies_only_followed_links() {
    let d = dir("package");
    png(&d.join("red.png"), |_| [255, 0, 0]);
    let (mut s, id) = document(&d, &d.join("red.png"));
    relink(&mut s, id, "red.png");
    #[allow(unused_mut)]
    let mut remote = vec![REMOTE.to_string()];
    #[cfg(unix)]
    remote.push(network_alias(&d.join("red.png")));
    for path in &remote {
        s.execute("file.placeLinked", json!({"path": d.join("red.png").to_string_lossy()})).unwrap();
        let other = s.active().unwrap().active_layer.unwrap();
        relink(&mut s, other, path);
    }
    let r = s.execute("file.package", json!({"dir": d.join("pkg").to_string_lossy()})).unwrap();
    let mut missing: Vec<String> = serde_json::from_value(r["missing"].clone()).unwrap();
    missing.sort();
    remote.sort();
    assert_eq!(missing, remote);
    let links = r["links"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert!(links[0].as_str().unwrap().ends_with("red.png"), "{links:?}");
}

#[test]
fn psd_saves_embed_only_followed_links() {
    let d = dir("psd");
    png(&d.join("red.png"), |_| [255, 0, 0]);
    let (mut s, id) = document(&d, &d.join("red.png"));
    relink(&mut s, id, "red.png");
    s.execute("file.placeLinked", json!({"path": d.join("red.png").to_string_lossy()})).unwrap();
    let other = s.active().unwrap().active_layer.unwrap();
    relink(&mut s, other, REMOTE);
    let warnings = |r: serde_json::Value| r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect::<Vec<_>>();
    let saved = warnings(s.execute("file.saveACopy", json!({"path": d.join("copy.psd").to_string_lossy()})).unwrap());
    assert!(saved.iter().any(|w| w.contains("red.png was embedded")), "{saved:?}");
    assert!(saved.iter().any(|w| w.contains("is not opened: the path is on another computer")), "{saved:?}");
    // Package as PSD writes the copy of the document, whose remote link stays as it was.
    let packaged = warnings(s.execute("file.package", json!({"dir": d.join("pkg").to_string_lossy(), "format": "psd"})).unwrap());
    assert!(packaged.iter().any(|w| w.contains("is not opened: the path is on another computer")), "{packaged:?}");
}

/// The pixels of the active document after its smart object `id` re-renders with its first smart
/// filter's params set to `params`.
fn replayed(s: &mut Session, id: LayerId, params: serde_json::Value) -> Vec<[f32; 4]> {
    s.edit("Params", |doc, _| {
        if let Some(LayerContent::Smart(sm)) = doc.layer_mut(id).map(|l| &mut l.content) {
            sm.smart_filters[0].params = params;
        }
        crate::smart_cmds::refresh(doc, id).map(|_| ())
    })
    .unwrap();
    let doc = &s.active().unwrap().doc;
    photocraft_compose::render(doc, doc.bounds()).px
}

#[test]
fn smart_filters_reread_only_followed_files() {
    let d = dir("filters");
    png(&d.join("ramp.png"), |x| [(x * 8) as u8, 0, 0]);
    png(&d.join("map.png"), |_| [255, 255, 255]);
    // An inverting look: corner (r, g, b) of the cube maps to (1-r, 1-g, 1-b), red fastest.
    let corners = (0..8).map(|i| format!("{} {} {}\n", 1 - (i & 1), 1 - ((i >> 1) & 1), 1 - ((i >> 2) & 1))).collect::<String>();
    std::fs::write(d.join("invert.cube"), format!("LUT_3D_SIZE 2\n{corners}")).unwrap();
    let path = |name: &str| d.join(name).to_string_lossy().into_owned();

    // Displace re-reads its `mapPath`.
    let (mut s, id) = document(&d, &d.join("ramp.png"));
    s.execute("filter.distort.displace", json!({"layer": id.0, "mapPath": path("map.png"), "horizontal": 50, "vertical": 0})).unwrap();
    let displaced = replayed(&mut s, id, json!({"mapPath": path("map.png"), "horizontal": 50, "vertical": 0}));
    let unfiltered = replayed(&mut s, id, json!({"mapPath": REMOTE, "horizontal": 50, "vertical": 0}));
    assert_ne!(displaced, unfiltered, "the map moves the pixels");
    assert_eq!(replayed(&mut s, id, json!({"mapPath": "map.png", "horizontal": 50, "vertical": 0})), displaced, "a relative map next to the document");
    #[cfg(unix)]
    assert_eq!(replayed(&mut s, id, json!({"mapPath": network_alias(&d.join("map.png")), "horizontal": 50, "vertical": 0})), unfiltered);

    // Color Lookup re-reads its LUT `file`.
    let (mut s, id) = document(&d, &d.join("ramp.png"));
    s.execute("image.adjustments.colorLookup", json!({"layer": id.0, "lut": "none"})).unwrap();
    let inverted = replayed(&mut s, id, json!({"file": path("invert.cube")}));
    let unfiltered = replayed(&mut s, id, json!({"file": REMOTE}));
    assert_ne!(inverted, unfiltered, "the look changes the pixels");
    assert_eq!(replayed(&mut s, id, json!({"file": "invert.cube"})), inverted, "a relative look next to the document");
    #[cfg(unix)]
    assert_eq!(replayed(&mut s, id, json!({"file": network_alias(&d.join("invert.cube"))})), unfiltered);
}

/// #7423 (review on #7799): Displace on a Smart Object keeps a `mapPath` as a path, as before
/// (only a map from an open document is embedded). A path relative to the saved document, set
/// through the filter's own edit, displaces as the absolute one did, and the dialog's "None"
/// (`mapDocument: -1`) names no map instead of failing.
#[test]
fn smart_displace_keeps_a_relative_map_path() {
    let d = dir("displace-relative");
    png(&d.join("ramp.png"), |x| [(x * 8) as u8, 0, 0]);
    png(&d.join("map.png"), |_| [255, 255, 255]);
    let (mut s, id) = document(&d, &d.join("ramp.png"));
    let path = d.join("map.png").to_string_lossy().into_owned();
    let unfiltered = photocraft_compose::render(&s.active().unwrap().doc, s.active().unwrap().doc.bounds()).px;
    s.execute("filter.distort.displace", json!({"layer": id.0, "mapPath": path, "horizontal": 50, "vertical": 0})).unwrap();
    let state = |s: &Session| {
        let doc = &s.active().unwrap().doc;
        let Some(LayerContent::Smart(sm)) = doc.layer(id).map(|l| &l.content) else { panic!("a smart object") };
        (sm.smart_filters[0].params.clone(), photocraft_compose::render(doc, doc.bounds()).px)
    };
    let (params, displaced) = state(&s);
    assert_eq!(params["mapPath"], json!(path));
    assert!(photocraft_io::smart_map::displace_embedded_map(&params).is_none(), "a path, not an embedded map");
    assert_ne!(displaced, unfiltered, "the map moves the pixels");
    s.execute("layer.smartFilter.setParams", json!({"layer": id.0, "index": 0, "params": {"mapPath": "map.png"}})).unwrap();
    let (params, relative) = state(&s);
    assert_eq!(params["mapPath"], json!("map.png"));
    assert_eq!(relative, displaced, "a relative map next to the document");
    s.execute("layer.smartFilter.setParams", json!({"layer": id.0, "index": 0, "params": {"mapDocument": -1}})).unwrap();
    assert_eq!(state(&s).1, displaced, "\"None\" changes nothing");
}

#[test]
fn data_sets_and_footage_read_only_followed_files() {
    let d = dir("datasets");
    png(&d.join("blue.png"), |_| [0, 0, 255]);
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "#ff0000"})).unwrap();
    s.active_mut().unwrap().path = Some(d.join("doc.pcraft").to_string_lossy().into_owned());
    let layer = s.active().unwrap().active_layer.unwrap();
    s.execute("image.variables.define", json!({"defs": [{"name": "img", "layer": layer.0, "type": "pixelReplacement", "method": "conform"}]})).unwrap();
    let set = |name: &str, path: &str| json!({"name": name, "values": [{"variable": "img", "kind": "pixels", "value": path}]});
    s.execute("image.variables.dataSets", json!({"dataSets": [set("remote", REMOTE), set("relative", "blue.png")]})).unwrap();
    let err = s.execute("image.applyDataSet", json!({"name": "remote"})).unwrap_err().to_string();
    assert!(err.contains("is not opened: the path is on another computer"), "{err}");
    assert_eq!(centre(&s), RED);
    s.execute("image.applyDataSet", json!({"name": "relative"})).unwrap();
    assert_eq!(centre(&s), BLUE);

    // Reload Frame re-reads the video layer's footage.
    s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": d.join("blue.png").to_string_lossy()})).unwrap();
    let footage = |s: &mut Session, path: &str| {
        let id = s.active().unwrap().active_layer.unwrap();
        s.edit("Footage", |doc, _| {
            if let Some(v) = doc.layer_mut(id).and_then(|l| l.video.as_mut()) {
                v.source = photocraft_doc::VideoSource::File { path: path.into() };
            }
            Ok(())
        })
        .unwrap();
        s.execute("layer.videoLayers.reloadFrame", json!({})).map_err(|e| e.to_string())
    };
    let err = footage(&mut s, REMOTE).unwrap_err();
    assert!(err.contains("is not opened: the path is on another computer"), "{err}");
    footage(&mut s, "blue.png").unwrap();
}

/// A link to a file PhotoCraft can't place (a text or settings file) is never embedded: a saved
/// PSD would otherwise carry bytes no reader can show as the layer's contents. The layer is
/// written as pixels; Convert to Embedded and Relink refuse.
#[test]
fn links_to_files_that_are_not_images_are_never_embedded() {
    const TEXT: &[u8] = b"plain text, not an image
photocraft-text-marker
";
    let d = dir("not-image");
    std::fs::create_dir_all(d.join("notes")).unwrap();
    std::fs::create_dir_all(d.join("Downloads")).unwrap();
    std::fs::write(d.join("notes").join("readme"), TEXT).unwrap();
    std::fs::write(d.join("Downloads").join(".settings"), TEXT).unwrap();
    png(&d.join("red.png"), |_| [255, 0, 0]);
    let (mut s, id) = document(&d, &d.join("red.png"));
    s.active_mut().unwrap().path = Some(d.join("Downloads").join("doc.pcraft").to_string_lossy().into_owned());
    let has_text = |bytes: &[u8]| bytes.windows(TEXT.len()).any(|w| w == TEXT);
    for (n, path) in ["../notes/readme", ".settings"].into_iter().enumerate() {
        relink(&mut s, id, path);
        for ext in ["psd", "psb", "tif"] {
            let out = d.join(format!("copy{n}.{ext}"));
            let r = s.execute("file.saveACopy", json!({"path": out.to_string_lossy(), "tiffLayers": true})).unwrap();
            assert!(!has_text(&std::fs::read(&out).unwrap()), "{path} embedded in the {ext}: {r}");
            let warnings = r["warnings"].to_string();
            assert!(warnings.contains("written as pixels"), "{path} {ext}: {warnings}");
        }
        assert!(s.execute("layer.smartObjects.convertToEmbedded", json!({"layer": id.0})).is_err(), "{path}");
        let abs = d.join("notes").join("readme");
        assert!(s.execute("layer.smartObjects.relinkToFile", json!({"layer": id.0, "path": abs.to_string_lossy()})).is_err(), "{path}");
    }
}

/// Duplicates share their contents identity, so a linked and an embedded copy of the same bytes
/// meet in the export's conversion cache: each keeps its own rule, whichever is written first.
#[test]
fn a_linked_and_an_embedded_copy_of_the_same_bytes_keep_their_own_rule() {
    const TEXT: &[u8] = b"plain text, not an image
photocraft-shared-marker
";
    let d = dir("shared-contents");
    std::fs::write(d.join("readme"), TEXT).unwrap();
    png(&d.join("red.png"), |_| [255, 0, 0]);
    let (mut s, linked) = document(&d, &d.join("red.png"));
    s.execute("layer.duplicate", json!({})).unwrap();
    let embedded = s.active().unwrap().active_layer.unwrap();
    assert_ne!(linked, embedded);
    relink(&mut s, linked, "readme");
    s.edit("Embed", |doc, _| {
        if let Some(LayerContent::Smart(sm)) = doc.layer_mut(embedded).map(|l| &mut l.content) {
            sm.source = SmartSource::Embedded { file_name: "readme".into(), bytes: std::sync::Arc::new(TEXT.to_vec()) };
        }
        Ok(())
    })
    .unwrap();
    for ext in ["psd", "psb", "tif"] {
        let out = d.join(format!("copy.{ext}"));
        let r = s.execute("file.saveACopy", json!({"path": out.to_string_lossy(), "tiffLayers": true})).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        let copies = bytes.windows(TEXT.len()).filter(|w| *w == TEXT).count();
        assert_eq!(copies, 1, "{ext}: only the embedded copy is kept: {r}");
        assert!(r["warnings"].to_string().contains("written as pixels"), "{ext}: {r}");
    }
}

/// Package copies only linked files PhotoCraft can open: a link to a text file is listed as
/// missing and no copy of it is written into the package's Links folder.
#[test]
fn package_copies_only_links_to_images() {
    const TEXT: &[u8] = b"plain text, not an image
photocraft-package-marker
";
    let d = dir("package-not-image");
    std::fs::write(d.join("readme"), TEXT).unwrap();
    png(&d.join("red.png"), |_| [255, 0, 0]);
    let (mut s, id) = document(&d, &d.join("red.png"));
    relink(&mut s, id, "readme");
    let pkg = d.join("pkg");
    let r = s.execute("file.package", json!({"dir": pkg.to_string_lossy()}));
    let copies: Vec<_> = walkdir(&pkg).into_iter().filter(|f| std::fs::read(f).is_ok_and(|b| b == TEXT)).collect();
    assert!(copies.is_empty(), "copied into the package: {copies:?} ({r:?})");
    let r = r.unwrap();
    assert_eq!(r["missing"], json!(["readme"]), "{r}");
}

fn walkdir(d: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() { out.extend(walkdir(&p)) } else { out.push(p) }
    }
    out
}
