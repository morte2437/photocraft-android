//! Exercise the real directory save, including cached-object reuse and garbage collection.
use super::*;
use crate::atomic::{AtomicFs, RealFs};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Layer, Size};
use std::cell::{Cell, RefCell};
use std::io;

/// The object or manifest directory barrier at which to inject one publication failure.
#[derive(Clone, Copy, Debug)]
enum FailAt {
    Tiles,
    Blobs,
    Manifest,
}

/// Real filesystem operations with recorded barriers and a single failure at the chosen stage.
struct PublicationFault {
    root: PathBuf,
    old_manifest: Vec<u8>,
    stage: FailAt,
    armed: Cell<bool>,
    synced: RefCell<Vec<PathBuf>>,
}

impl AtomicFs for PublicationFault {
    /// Keep temporary-file writes and file flushes real; only directory barriers are injected.
    fn write_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        RealFs.write_new(path, bytes)
    }
    /// Publish through the real rename so a barrier error occurs after replacement.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.rename(from, to)
    }
    /// Use real cleanup so the test can detect premature removal of old objects.
    fn remove(&self, path: &Path) -> io::Result<()> {
        RealFs.remove(path)
    }
    /// Preserve normal replacement metadata behavior during failure injection.
    fn copy_metadata(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.copy_metadata(from, to)
    }
    /// Record every barrier and fail the selected stage once, allowing a same-writer retry.
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        self.synced.borrow_mut().push(dir.to_path_buf());
        let fail = match self.stage {
            FailAt::Tiles => dir == self.root.join("tiles"),
            FailAt::Blobs => dir == self.root.join("blobs"),
            FailAt::Manifest => dir == self.root && std::fs::read(dir.join(MANIFEST))? != self.old_manifest,
        };
        if fail && self.armed.replace(false) {
            return Err(io::Error::other("injected publication flush failure"));
        }
        RealFs.sync_dir(dir)
    }
}

/// Failed object or manifest publication retains old objects; a retry flushes reused objects
/// before the manifest and resumes garbage collection only after durable publication.
#[test]
fn failed_bundle_publication_retains_old_objects_and_same_writer_retry_is_durable() {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    for stage in [FailAt::Tiles, FailAt::Blobs, FailAt::Manifest] {
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pcraft-durability-{}-{id}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        let mut old = Document::new("old", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let mut layer = Layer::raster("pixels", old.pixel_format());
        layer.surface_mut().unwrap().write_pixel(0, 0, &[1.0, 0.0, 0.0, 1.0]);
        old.layers.push(layer);
        old.metadata.exif = Some(Arc::new(vec![1; 100]));
        let mut new = old.clone();
        new.name = "new".into();
        new.layers[0].surface_mut().unwrap().write_pixel(0, 0, &[0.0, 0.0, 1.0, 1.0]);
        new.metadata.exif = Some(Arc::new(vec![2; 100]));

        let mut writer = PcraftWriter::new();
        writer.save_dir(&old, &dir, &SaveOptions::default()).unwrap();
        let old_objects: Vec<_> = list_objects(&dir).unwrap().into_iter().map(|p| (p.clone(), std::fs::read(dir.join(p)).unwrap())).collect();
        assert!(old_objects.len() >= 2);
        let fs = PublicationFault {
            old_manifest: std::fs::read(dir.join(MANIFEST)).unwrap(),
            root: dir.clone(),
            stage,
            armed: Cell::new(true),
            synced: RefCell::new(Vec::new()),
        };

        let error = writer.save_dir_with(&new, &dir, &SaveOptions::default(), &fs).unwrap_err();
        assert!(!fs.armed.get(), "the {stage:?} failure fired");
        assert!(error.to_string().contains("may not survive"), "{error}");
        let expected = if matches!(stage, FailAt::Manifest) { &new } else { &old };
        assert_eq!(&crate::load_path(&dir).unwrap(), expected);
        for (path, bytes) in &old_objects {
            assert_eq!(&std::fs::read(dir.join(path)).unwrap(), bytes, "old object removed before durable publication: {path}");
        }

        fs.synced.borrow_mut().clear();
        writer.save_dir_with(&new, &dir, &SaveOptions::default(), &fs).unwrap();
        assert_eq!(crate::load_path(&dir).unwrap(), new);
        let synced = fs.synced.borrow();
        let publication = synced.iter().rposition(|p| p == &dir).unwrap();
        for sub in ["tiles", "blobs"] {
            assert!(
                synced.iter().position(|p| p == &dir.join(sub)).is_some_and(|position| position < publication),
                "retry must flush reused {sub} before its manifest"
            );
        }
        assert!(old_objects.iter().all(|(p, _)| !dir.join(p).exists()), "cleanup resumes after the durable retry");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
