use super::*;

/// The flags each pinned corpus is fetched with by `fetch_all(local, update)`.
fn calls(local: bool, update: bool) -> Vec<(&'static str, bool)> {
    let mut seen = Vec::new();
    let r = fetch_pinned(local, update, |c, _, u| {
        seen.push((c.name, u));
        Ok(())
    });
    assert_eq!(r, Ok(()));
    seen
}

#[test]
fn all_with_update_manifest_updates_every_pinned_corpus() {
    for local in [false, true] {
        let seen = calls(local, true);
        assert_eq!(seen.len(), corpus_pins::ALL.len());
        assert!(seen.iter().all(|(_, u)| *u), "--all --update-manifest dropped the flag: {seen:?}");
    }
}

#[test]
fn all_without_update_manifest_only_verifies() {
    assert!(calls(false, false).iter().all(|(_, u)| !*u));
}

const FIXTURE_FILES: &[(&str, &[u8])] = &[("a.png", b"alpha"), ("b.png", b"beta"), ("PngSuite.LICENSE", b"fixture licence")];
const FIXTURE_ARCHIVE: &[u8] = b"owned archive fixture";

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let id = PNGSUITE_STAGE_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("photocraft-pngsuite-test-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn cache(&self) -> PathBuf {
        self.0.join("pngsuite")
    }

    fn old_cache(&self) {
        std::fs::create_dir(self.cache()).unwrap();
        std::fs::write(self.cache().join("old.png"), b"old cache bytes").unwrap();
    }

    fn assert_old_cache(&self) {
        assert_eq!(std::fs::read(self.cache().join("old.png")).unwrap(), b"old cache bytes");
        assert_eq!(std::fs::read_dir(&self.0).unwrap().count(), 1, "staging was not cleaned");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_manifest() -> String {
    FIXTURE_FILES.iter().map(|(name, bytes)| format!("{}  {name}\n", crate::sha256::hex(bytes))).collect()
}

fn fixture_extract(_: &Path, files: &Path) -> Result<(), String> {
    for (name, bytes) in FIXTURE_FILES {
        std::fs::write(files.join(name), bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn fixture_download(archive: &Path) -> Result<(), String> {
    std::fs::write(archive, FIXTURE_ARCHIVE).map_err(|e| e.to_string())
}

fn fixture_fetch(corpus: &Path) -> Result<(), String> {
    fetch_pngsuite_in(corpus, &crate::sha256::hex(FIXTURE_ARCHIVE), &fixture_manifest(), fixture_download, fixture_extract, |from, to| {
        std::fs::rename(from, to)
    })
}

#[test]
fn pngsuite_cache_checks_bytes_names_and_regular_files() {
    let f = Fixture::new();
    std::fs::create_dir(f.cache()).unwrap();
    fixture_extract(Path::new("unused"), &f.cache()).unwrap();
    assert!(verify_pngsuite(&f.cache(), &fixture_manifest()).is_ok());
    std::fs::write(f.cache().join("a.png"), b"changed").unwrap();
    assert!(verify_pngsuite(&f.cache(), &fixture_manifest()).unwrap_err().contains("sha256 mismatch"));
    fixture_extract(Path::new("unused"), &f.cache()).unwrap();
    std::fs::remove_file(f.cache().join("a.png")).unwrap();
    assert!(verify_pngsuite(&f.cache(), &fixture_manifest()).unwrap_err().contains("missing files"));
    fixture_extract(Path::new("unused"), &f.cache()).unwrap();
    std::fs::write(f.cache().join("extra.png"), b"extra").unwrap();
    assert!(verify_pngsuite(&f.cache(), &fixture_manifest()).unwrap_err().contains("unexpected file"));
    std::fs::remove_file(f.cache().join("extra.png")).unwrap();
    std::fs::remove_file(f.cache().join("a.png")).unwrap();
    std::fs::create_dir(f.cache().join("a.png")).unwrap();
    assert!(verify_pngsuite(&f.cache(), &fixture_manifest()).unwrap_err().contains("regular file"));
}

#[test]
fn pngsuite_valid_cache_needs_no_download_or_extraction() {
    let f = Fixture::new();
    fixture_fetch(&f.0).unwrap();
    fetch_pngsuite_in(
        &f.0,
        "unused",
        &fixture_manifest(),
        |_| panic!("valid cache downloaded"),
        |_, _| panic!("valid cache extracted"),
        |_, _| panic!("valid cache renamed"),
    )
    .unwrap();
}

#[test]
fn pngsuite_repairs_unverified_175_file_cache_and_removes_extras() {
    let f = Fixture::new();
    std::fs::create_dir(f.cache()).unwrap();
    for i in 0..175 {
        std::fs::write(f.cache().join(format!("fake-{i}.png")), b"not a PNG").unwrap();
    }
    fixture_fetch(&f.0).unwrap();
    assert!(verify_pngsuite(&f.cache(), &fixture_manifest()).is_ok());
    assert_eq!(std::fs::read_dir(f.cache()).unwrap().count(), FIXTURE_FILES.len());
    assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 1);
}

#[test]
fn pngsuite_unexpected_or_truncated_archive_never_reaches_extractor() {
    for bad in [b"unexpected".as_slice(), &FIXTURE_ARCHIVE[..4]] {
        let f = Fixture::new();
        f.old_cache();
        let e = fetch_pngsuite_in(
            &f.0,
            &crate::sha256::hex(FIXTURE_ARCHIVE),
            &fixture_manifest(),
            |a| std::fs::write(a, bad).map_err(|e| e.to_string()),
            |_, _| panic!("unverified archive reached extractor"),
            |_, _| panic!("unverified archive reached installation"),
        )
        .unwrap_err();
        assert!(e.contains("archive sha256 mismatch"));
        f.assert_old_cache();
    }
}

#[test]
fn pngsuite_download_or_extraction_failure_keeps_old_cache() {
    let f = Fixture::new();
    f.old_cache();
    assert!(
        fetch_pngsuite_in(
            &f.0,
            "unused",
            &fixture_manifest(),
            |_| Err("offline".into()),
            |_, _| panic!("failed download extracted"),
            |_, _| panic!("failed download installed")
        )
        .unwrap_err()
        .contains("offline")
    );
    f.assert_old_cache();
    assert!(
        fetch_pngsuite_in(
            &f.0,
            &crate::sha256::hex(FIXTURE_ARCHIVE),
            &fixture_manifest(),
            fixture_download,
            |_, files| {
                std::fs::write(files.join("partial.png"), b"partial").unwrap();
                Err("extract failed".into())
            },
            |_, _| panic!("failed extraction installed")
        )
        .unwrap_err()
        .contains("extract failed")
    );
    f.assert_old_cache();
    for extra in [false, true] {
        let e = fetch_pngsuite_in(
            &f.0,
            &crate::sha256::hex(FIXTURE_ARCHIVE),
            &fixture_manifest(),
            fixture_download,
            |a, files| {
                fixture_extract(a, files)?;
                if extra {
                    std::fs::write(files.join("extra.png"), b"extra").unwrap();
                } else {
                    std::fs::remove_file(files.join("a.png")).unwrap();
                }
                Ok(())
            },
            |_, _| panic!("unverified extraction installed"),
        )
        .unwrap_err();
        assert!(e.contains(if extra { "unexpected file" } else { "missing files" }));
        f.assert_old_cache();
    }
}

#[test]
fn pngsuite_failed_installation_restores_old_cache() {
    for fail_backup in [false, true] {
        let f = Fixture::new();
        f.old_cache();
        let e = fetch_pngsuite_in(&f.0, &crate::sha256::hex(FIXTURE_ARCHIVE), &fixture_manifest(), fixture_download, fixture_extract, |from, to| {
            if (fail_backup && from == f.cache()) || (!fail_backup && from.file_name().is_some_and(|n| n == "files")) {
                Err(std::io::Error::other("injected rename failure"))
            } else {
                std::fs::rename(from, to)
            }
        })
        .unwrap_err();
        assert!(e.contains("injected rename failure"));
        f.assert_old_cache();
    }
}

#[test]
fn pngsuite_failed_rollback_retains_and_names_the_backup() {
    let f = Fixture::new();
    f.old_cache();
    let e = fetch_pngsuite_in(&f.0, &crate::sha256::hex(FIXTURE_ARCHIVE), &fixture_manifest(), fixture_download, fixture_extract, |from, to| {
        if from == f.cache() { std::fs::rename(from, to) } else { Err(std::io::Error::other("injected install and rollback failure")) }
    })
    .unwrap_err();
    let stage = std::fs::read_dir(&f.0).unwrap().next().unwrap().unwrap().path();
    let backup = stage.join("previous");
    assert!(e.contains(&backup.display().to_string()), "{e}");
    assert_eq!(std::fs::read(backup.join("old.png")).unwrap(), b"old cache bytes");
    std::fs::rename(backup, f.cache()).unwrap();
}

#[test]
fn pngsuite_reads_are_bounded_and_malformed_manifests_are_rejected() {
    let f = Fixture::new();
    let file = f.0.join("large");
    std::fs::File::create(&file).unwrap().set_len(PNGSUITE_MAX_ARCHIVE_BYTES + 1).unwrap();
    assert!(pngsuite_bytes(&file, PNGSUITE_MAX_ARCHIVE_BYTES).unwrap_err().contains("regular file"));
    for malformed in ["", "invalid", "00  a.png", &format!("{}  ../outside\n", crate::sha256::hex(b""))] {
        assert!(verify_pngsuite(&f.0, malformed).is_err());
    }
}

#[test]
fn pngsuite_download_copy_accepts_boundary_and_never_writes_excess() {
    let limit = PNGSUITE_MAX_ARCHIVE_BYTES as usize;
    for len in [limit - 1, limit, limit + 1, limit * 2] {
        let data = vec![0xa5; len];
        let mut out = Vec::new();
        let result = copy_pngsuite_download(&mut std::io::Cursor::new(&data), &mut out);
        assert_eq!(result.is_ok(), len <= limit);
        assert_eq!(out, data[..len.min(limit)]);
    }
}

// The ordinary test invocation returns immediately. Only this test's child command
// enables emission; it stays alive so cleanup must kill and reap that particular child.
#[test]
fn pngsuite_stream_fixture() {
    if std::env::var_os("PHOTOCRAFT_PNGSUITE_STREAM_TEST").is_some() {
        let mut out = std::io::stdout().lock();
        for _ in 0..256 {
            let _ = out.write_all(&[0xa5; 8192]);
        }
        std::thread::park();
    }
}

fn stream_child() -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "corpus::tests::pngsuite_stream_fixture", "--nocapture"])
        .env("PHOTOCRAFT_PNGSUITE_STREAM_TEST", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
fn pngsuite_oversize_and_io_failures_kill_and_reap_only_the_download_child() {
    struct FailedRead;
    impl Read for FailedRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected read failure"))
        }
    }
    struct FailedWrite;
    impl Write for FailedWrite {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected write failure"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut child = stream_child();
    let stdout = child.stdout.take().unwrap();
    let mut out = Vec::new();
    assert!(finish_pngsuite_download(&mut child, stdout, &mut out).unwrap_err().contains("archive exceeds"));
    assert_eq!(out.len() as u64, PNGSUITE_MAX_ARCHIVE_BYTES);
    assert!(child.try_wait().unwrap().is_some());

    let mut child = stream_child();
    assert!(child.try_wait().unwrap().is_none());
    assert!(finish_pngsuite_download(&mut child, FailedRead, &mut Vec::new()).unwrap_err().contains("injected read failure"));
    assert!(child.try_wait().unwrap().is_some());

    let mut child = stream_child();
    let stdout = child.stdout.take().unwrap();
    assert!(finish_pngsuite_download(&mut child, stdout, &mut FailedWrite).unwrap_err().contains("injected write failure"));
    assert!(child.try_wait().unwrap().is_some());
}

#[cfg(unix)]
#[test]
fn pngsuite_cache_rejects_symlinks_even_when_target_bytes_match() {
    let f = Fixture::new();
    std::fs::create_dir(f.cache()).unwrap();
    fixture_extract(Path::new("unused"), &f.cache()).unwrap();
    std::fs::write(f.0.join("target"), b"alpha").unwrap();
    std::fs::remove_file(f.cache().join("a.png")).unwrap();
    std::os::unix::fs::symlink(f.0.join("target"), f.cache().join("a.png")).unwrap();
    assert!(verify_pngsuite(&f.cache(), &fixture_manifest()).unwrap_err().contains("regular file"));
}
