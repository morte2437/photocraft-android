//! `import_file` parses a PSD/PSB from the open file (channel data read as it is decoded) and must
//! open the same document, with the same warnings, as importing the file's bytes.

use photocraft_io::{ImportResult, import, import_file};
use photocraft_psd::testgen;
use photocraft_raster::Interrupt;

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("photocraft-import-file-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The document's debug form with every `…Id(n)` number blanked.
fn without_ids(doc: &photocraft_doc::Document) -> String {
    let s = format!("{doc:?}");
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(at) = rest.find("Id(") {
        out.push_str(&rest[..at + 3]);
        rest = rest[at + 3..].trim_start_matches(|c: char| c.is_ascii_digit());
    }
    out.push_str(rest);
    out
}

fn open(name: &str, path: &std::path::Path) -> Result<ImportResult, photocraft_io::IoError> {
    import_file(name, path, &photocraft_io::ImportOptions::default(), &Interrupt::NONE)
}

#[test]
fn import_file_opens_what_importing_the_bytes_opens() {
    let dir = TempDir::new("same");
    let mut checked = 0;
    for case in testgen::all_cases() {
        let ext = if case.file.header.version.is_psb() { "psb" } else { "psd" };
        let name = format!("case.{ext}");
        let bytes = case.file.to_bytes().unwrap();
        let path = dir.write(&name, &bytes);
        let from_bytes = import(&name, &bytes);
        let from_file = open(&name, &path);
        match (from_bytes, from_file) {
            (Ok(a), Ok(b)) => {
                // Ids come from a process-wide counter, so the two opens differ only there.
                assert!(without_ids(&b.document) == without_ids(&a.document), "{}", case.name);
                assert_eq!(b.warnings, a.warnings, "{}", case.name);
                checked += 1;
            }
            (Err(_), Err(_)) => {}
            (a, b) => panic!("{}: bytes {:?}, file {:?}", case.name, a.map(|r| r.warnings), b.map(|r| r.warnings)),
        }
    }
    assert!(checked > 50, "only {checked} cases opened");
}

#[test]
fn import_file_reads_other_formats_whole() {
    let dir = TempDir::new("other");
    let doc = photocraft_doc::Document::new("x", photocraft_geom::Size::new(3, 2), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let png = photocraft_io::export(&doc, "x.png", &Default::default()).unwrap().bytes;
    let path = dir.write("x.png", &png);
    let r = open("x.png", &path).unwrap();
    assert_eq!(r.document.size, doc.size);
}

#[test]
fn import_file_fails_cleanly() {
    let dir = TempDir::new("fail");
    assert!(open("missing.psd", &dir.0.join("missing.psd")).is_err());
    let bytes = testgen::small(photocraft_psd::Version::Psd, photocraft_psd::Compression::Rle).to_bytes().unwrap();
    // Cut inside the layer records, inside the channel data, and inside the merged image.
    for cut in [10, 60, bytes.len() / 2, bytes.len() - 5] {
        let name = format!("cut{cut}.psd");
        let path = dir.write(&name, &bytes[..cut]);
        let from_bytes = import(&name, &bytes[..cut]);
        let from_file = open(&name, &path);
        // The merged image is not read when the layers are what opens, so a file cut inside it
        // can open from the file where the strict parse of its bytes refused it.
        if from_bytes.is_ok() {
            assert!(from_file.is_ok(), "cut at {cut}");
        }
    }
}

#[test]
fn a_cancelled_import_file_reports_cancelled() {
    let dir = TempDir::new("cancel");
    let bytes = testgen::small(photocraft_psd::Version::Psd, photocraft_psd::Compression::Zip).to_bytes().unwrap();
    let path = dir.write("c.psd", &bytes);
    let cancel = || true;
    let ctl = Interrupt::cancel_only(&cancel);
    assert!(matches!(import_file("c.psd", &path, &photocraft_io::ImportOptions::default(), &ctl), Err(photocraft_io::IoError::Cancelled)));
}

/// A read that fails while the layers are decoded from the file (it is truncated or rewritten,
/// its drive goes away) fails the open: no layer is left empty for a save to lose.
#[test]
fn a_read_failing_while_layers_decode_fails_the_open() {
    use photocraft_psd::ReadAt;
    /// The file's bytes, failing every read of channel data (as a dropped network share or a
    /// truncated file would) except the 2-byte compression fields the parse reads.
    struct Failing {
        bytes: Vec<u8>,
        data: Vec<(u64, u64)>,
    }
    impl ReadAt for Failing {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> photocraft_psd::Result<()> {
            let end = offset + buf.len() as u64;
            if self.data.iter().any(|&(a, b)| offset < b && end > a) {
                return Err(photocraft_psd::PsdError::Read("the drive went away".into()));
            }
            self.bytes.as_slice().read_at(offset, buf)
        }
    }
    for c in [photocraft_psd::Compression::Rle, photocraft_psd::Compression::Raw, photocraft_psd::Compression::Zip] {
        let file = testgen::layered(photocraft_psd::Version::Psd, photocraft_psd::ColorMode::Rgb, 8, c);
        let bytes = file.to_bytes().unwrap();
        let lazy = photocraft_psd::PsdFile::from_source(photocraft_psd::Source::new(bytes.clone())).unwrap();
        let data: Vec<(u64, u64)> = lazy
            .layers()
            .iter()
            .flat_map(|l| &l.channels)
            .filter_map(|ch| ch.stored.as_ref())
            .filter(|s| s.len > 0)
            .map(|s| (s.offset, s.offset + s.len))
            .collect();
        assert!(!data.is_empty());
        // Opening the same file whole succeeds: only the failed reads fail the open.
        assert!(import("x.psd", &bytes).is_ok());
        let r = photocraft_io::import_source_for_open_with("x.psd", photocraft_psd::Source::new(Failing { bytes, data }), &Interrupt::NONE);
        match r {
            Err(photocraft_io::IoError::Read(msg)) => assert!(msg.contains("the drive went away"), "{c:?}: {msg}"),
            r => panic!("{c:?}: the open didn't fail: {:?}", r.map(|(r, damaged)| (r.warnings, damaged))),
        }
    }
}
