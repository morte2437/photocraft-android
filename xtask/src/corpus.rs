//! `cargo xtask corpus [...]` (fetch test corpora) and `cargo xtask test-corpus [...]` (fetch,
//! then run the corpus tests). Pins: `corpus_pins.rs`. Docs: `docs/development.md` › Test corpora.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::corpus_pins::{
    self, AG_PSD_COMMIT, HEIC_RS_COMMIT, OPENEXR_COMMIT, PHOTOCRAFT_CORPUS_COMMIT, PHOTOCRAFT_CORPUS_GIMP_COMMIT, PILLOW_HEIF_COMMIT, PNGSUITE_SHA256,
    PNGSUITE_URL, PSD_TOOLS_COMMIT,
};
use crate::pinned::{PinnedCorpus, USER_AGENT};
use crate::{cargo, root, run};

/// Crates with corpus tests (behind their `corpus` feature).
pub const CORPUS_CRATES: &[&str] = &["photocraft-psd", "photocraft-codecs", "photocraft-io", "photocraft-engine"];
/// Opt-in corpus crates: fetched and tested only with `test-corpus --pixls`.
pub const PIXLS_CRATES: &[&str] = &["photocraft-raw"];

/// Corpus crates with a `heif` feature: test-corpus enables it so the HEIF corpus tests run.
const HEIF_CRATES: &[&str] = &["photocraft-codecs", "photocraft-io"];

/// Paths whose changes make `test-corpus --changed` run (the file-format and rendering crates,
/// and the engine's `.atn` import and export).
const CRITICAL: &[&str] = &[
    "crates/psd/",
    "crates/io/",
    "crates/codecs/",
    "crates/heif/",
    "crates/compose/",
    "crates/gpu/",
    "crates/text/",
    "crates/format/",
    "crates/affinity/",
    "crates/raw/",
    "crates/engine/src/atn_cmds",
];
/// `cargo xtask corpus [--all | --pngsuite | --download | --psd | --psd-tools | --heif | --exr | --pixls | --affinity | --atn | --photoshop [--local] | --gimp [--local]] [--update-manifest]`
pub fn cmd(args: &[&str]) -> Result<(), String> {
    let update = args.contains(&"--update-manifest");
    let mut did = false;
    if args.contains(&"--all") {
        return fetch_all(args.contains(&"--local"), update);
    }
    if args.contains(&"--pngsuite") || args.contains(&"--download") {
        fetch_pngsuite()?;
        did = true;
    }
    if args.contains(&"--psd") {
        corpus_pins::PSD_MIXED.fetch(update)?;
        did = true;
    }
    if args.contains(&"--psd-tools") {
        corpus_pins::PSD_TOOLS.fetch(update)?;
        did = true;
    }
    if args.contains(&"--heif") {
        corpus_pins::HEIF.fetch(update)?;
        did = true;
    }
    if args.contains(&"--pixls") {
        crate::pinned::fetch_pixls(update)?;
        did = true;
    }
    if args.contains(&"--exr") {
        corpus_pins::EXR.fetch(update)?;
        did = true;
    }
    if args.contains(&"--affinity") {
        corpus_pins::AFFINITY.fetch(update)?;
        did = true;
    }
    if args.contains(&"--atn") {
        corpus_pins::ATN.fetch(update)?;
        did = true;
    }
    if args.contains(&"--photoshop") {
        if args.contains(&"--local") {
            corpus_pins::PHOTOSHOP.fetch_local(&local_clone()?, update)?;
        } else {
            corpus_pins::PHOTOSHOP.fetch(update)?;
        }
        did = true;
    }
    if args.contains(&"--gimp") {
        if args.contains(&"--local") {
            corpus_pins::GIMP.fetch_local(&local_clone()?, update)?;
        } else {
            corpus_pins::GIMP.fetch(update)?;
        }
        did = true;
    }
    if !did {
        list();
    }
    Ok(())
}

/// The authoring clone of photocraft-corpus: `$PHOTOCRAFT_CORPUS_REPO`, else `../photocraft-corpus`.
fn local_clone() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("PHOTOCRAFT_CORPUS_REPO").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let sibling = root().parent().map(|p| p.join("photocraft-corpus"));
    sibling.filter(|s| s.join("photoshop").is_dir()).ok_or_else(|| {
        "no local photocraft-corpus clone: expected ../photocraft-corpus next to this checkout, or set PHOTOCRAFT_CORPUS_REPO=<path>".to_string()
    })
}

/// Fetches every corpus that is missing or stale (verified ones are left alone). With `local`,
/// `corpus/photoshop` and `corpus/gimp` are copied from the authoring clone of photocraft-corpus instead. With
/// `update`, every pinned corpus is refetched and its manifest rewritten (`--update-manifest`).
pub fn fetch_all(local: bool, update: bool) -> Result<(), String> {
    fetch_pngsuite()?;
    fetch_pinned(local, update, fetch_one)?;
    println!("all corpora present and verified under {}", root().join("corpus").display());
    Ok(())
}

/// Calls `fetch(corpus, from_clone, update)` for every pinned corpus; `from_clone` is set for
/// the photocraft-corpus sets (`corpus/photoshop`, `corpus/gimp`) when `local`.
fn fetch_pinned(local: bool, update: bool, mut fetch: impl FnMut(&PinnedCorpus, bool, bool) -> Result<(), String>) -> Result<(), String> {
    for c in corpus_pins::ALL {
        let ours = std::ptr::eq(*c, &corpus_pins::PHOTOSHOP) || std::ptr::eq(*c, &corpus_pins::GIMP);
        fetch(c, local && ours, update)?;
    }
    Ok(())
}

fn fetch_one(c: &PinnedCorpus, from_clone: bool, update: bool) -> Result<(), String> {
    if from_clone { c.fetch_local(&local_clone()?, update) } else { c.fetch(update) }
}

fn status(present: bool) -> &'static str {
    if present { "present" } else { "missing" }
}

fn list() {
    let corpus = root().join("corpus");
    let png = corpus.join("pngsuite");
    let png_ok = verify_pngsuite(&png, PNGSUITE_MANIFEST).is_ok();
    println!(
        "Test corpora live under {} (gitignored, never committed). The corpus tests are opt-in
(cargo feature `corpus`) and fail, not skip, when a corpus is missing. CI always runs them.

  cargo xtask corpus --all      fetch every corpus below that is missing or stale (sha256-verified)
  cargo xtask test-corpus       fetch, then run every corpus test (--release --features corpus)

  corpus/photoshop/  [{}] our Photoshop-authored oracles (MIT OR Apache-2.0), from
                     https://github.com/storytold/photocraft-corpus at {PHOTOCRAFT_CORPUS_COMMIT}
                     manifest xtask/photoshop-corpus.sha256. Fetch: --photoshop
                     (--photoshop --local copies from ../photocraft-corpus or $PHOTOCRAFT_CORPUS_REPO)
  corpus/gimp/       [{}] our GIMP-authored XCF oracles with GIMP's PNG renderings (MIT OR Apache-2.0),
                     from the same repository at {PHOTOCRAFT_CORPUS_GIMP_COMMIT}
                     manifest xtask/gimp-corpus.sha256. Fetch: --gimp (--gimp --local copies from the clone)
  corpus/psd/        [{}] 170 small psd-tools + ag-psd files (MIT), psd-tools@{} and
                     ag-psd@{}, manifest xtask/psd-corpus.sha256. Fetch: --psd
  corpus/psd-tools/  [{}] the full psd-tools test set (MIT) at {PSD_TOOLS_COMMIT}
                     manifest xtask/psd-tools-corpus.sha256. Fetch: --psd-tools
  corpus/heif/       [{}] a few HEIC/HEIF files from heic-rs@{} (MIT OR Apache-2.0) and
                     pillow-heif@{} (BSD-3-Clause), manifest xtask/heif-corpus.sha256. Fetch: --heif
  corpus/exr/        [{}] the deep OpenEXR test images (BSD-3-Clause), openexr@{}
                     manifest xtask/exr-corpus.sha256. Fetch: --exr
  corpus/affinity/   [{}] public Affinity documents and #1606 samples (CC0, MIT), with PNG references;
                     manifest xtask/affinity-corpus.sha256. Fetch: --affinity
  corpus/atn/        [{}] Photoshop Actions sets from seven public repositories (MIT, BSD, Unlicense);
                     manifest xtask/atn-corpus.sha256. Fetch: --atn
  corpus/pngsuite/   [{}] PngSuite (public domain), {PNGSUITE_URL}. Fetch: --pngsuite
  corpus/pixls/      [opt-in] real camera raws from raw.pixls.us (public domain), one per decode
                     path plus the known-unsupported packed ORF. Fetch: --pixls (opt-in)

Pins: xtask/src/corpus_pins.rs. Moving one: change it, then --<name> --update-manifest.",
        corpus.display(),
        status(corpus_pins::PHOTOSHOP.is_current()),
        status(corpus_pins::GIMP.is_current()),
        status(corpus_pins::PSD_MIXED.is_current()),
        &PSD_TOOLS_COMMIT[..12],
        &AG_PSD_COMMIT[..12],
        status(corpus_pins::PSD_TOOLS.is_current()),
        status(corpus_pins::HEIF.is_current()),
        &HEIC_RS_COMMIT[..12],
        &PILLOW_HEIF_COMMIT[..12],
        status(corpus_pins::EXR.is_current()),
        &OPENEXR_COMMIT[..12],
        status(corpus_pins::AFFINITY.is_current()),
        status(corpus_pins::ATN.is_current()),
        status(png_ok),
    );
}

const PNGSUITE_MANIFEST: &str = include_str!("../pngsuite.sha256");
const PNGSUITE_MAX_ARCHIVE_BYTES: u64 = 1024 * 1024;
const PNGSUITE_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
static PNGSUITE_STAGE_ID: AtomicU64 = AtomicU64::new(0);

/// Do not trust cached filenames, symlinks, or a file's size remaining unchanged during a read.
fn pngsuite_bytes(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() > limit {
        return Err(format!("{}: expected a regular file of at most {limit} bytes", path.display()));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{}: exceeds {limit} bytes", path.display()));
    }
    Ok(bytes)
}

fn verify_pngsuite(dir: &Path, manifest: &str) -> Result<(), String> {
    let mut expected = BTreeMap::new();
    for line in manifest.lines() {
        let Some((hash, name)) = line.split_once("  ") else { return Err("pngsuite: invalid manifest line".into()) };
        if hash.len() != 64
            || !hash.bytes().all(|c| c.is_ascii_hexdigit())
            || name.is_empty()
            || name.contains("..")
            || !name.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || expected.insert(name, hash).is_some()
        {
            return Err("pngsuite: invalid manifest entry".into());
        }
    }
    if expected.is_empty() {
        return Err("pngsuite: empty manifest".into());
    }
    let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !meta.is_dir() {
        return Err(format!("{}: expected a regular directory", dir.display()));
    }
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut seen = 0;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry.file_name();
        let hash = name.to_str().and_then(|n| expected.get(n)).ok_or_else(|| format!("pngsuite: unexpected file {}", entry.path().display()))?;
        let bytes = pngsuite_bytes(&entry.path(), PNGSUITE_MAX_FILE_BYTES)?;
        if crate::sha256::hex(&bytes) != **hash {
            return Err(format!("pngsuite: {}: sha256 mismatch", entry.path().display()));
        }
        seen += 1;
    }
    if seen != expected.len() {
        return Err(format!("pngsuite: missing files ({seen} of {})", expected.len()));
    }
    Ok(())
}

struct PngSuiteStage {
    path: PathBuf,
    keep: bool,
}

impl PngSuiteStage {
    fn new(corpus: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(corpus).map_err(|e| format!("create {}: {e}", corpus.display()))?;
        for _ in 0..100 {
            let id = PNGSUITE_STAGE_ID.fetch_add(1, Ordering::Relaxed);
            let path = corpus.join(format!(".pngsuite-{}-{id}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path, keep: false }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(format!("create {}: {e}", path.display())),
            }
        }
        Err("pngsuite: could not create a staging directory".into())
    }
}

impl Drop for PngSuiteStage {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Keep the old cache until the verified replacement is in place. On a failed rollback its
/// backup remains in staging and the error names it, rather than deleting the only copy.
fn install_pngsuite(
    dest: &Path,
    files: &Path,
    backup: &Path,
    backup_retained: &mut bool,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), String> {
    let had_cache = match std::fs::symlink_metadata(dest) {
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(format!("{}: {e}", dest.display())),
    };
    if had_cache {
        rename(dest, backup).map_err(|e| format!("pngsuite: preserve {}: {e}", dest.display()))?;
        *backup_retained = true;
    }
    if let Err(e) = rename(files, dest) {
        if had_cache && let Err(restore) = rename(backup, dest) {
            return Err(format!("pngsuite: install failed ({e}); restore failed ({restore}); old cache retained at {}", backup.display()));
        }
        *backup_retained = false;
        return Err(format!("pngsuite: install failed: {e}"));
    }
    *backup_retained = false;
    Ok(())
}

fn fetch_pngsuite_in(
    corpus: &Path,
    archive_hash: &str,
    manifest: &str,
    download: impl FnOnce(&Path) -> Result<(), String>,
    extract: impl FnOnce(&Path, &Path) -> Result<(), String>,
    rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), String> {
    let dest = corpus.join("pngsuite");
    if verify_pngsuite(&dest, manifest).is_ok() {
        println!("pngsuite: {} already matches its manifest", dest.display());
        return Ok(());
    }
    let mut stage = PngSuiteStage::new(corpus)?;
    let archive = stage.path.join("archive.tgz");
    download(&archive)?;
    let bytes = pngsuite_bytes(&archive, PNGSUITE_MAX_ARCHIVE_BYTES)?;
    if crate::sha256::hex(&bytes) != archive_hash {
        return Err("pngsuite: archive sha256 mismatch; nothing extracted".into());
    }
    let files = stage.path.join("files");
    std::fs::create_dir(&files).map_err(|e| format!("create {}: {e}", files.display()))?;
    extract(&archive, &files)?;
    verify_pngsuite(&files, manifest)?;
    let backup = stage.path.join("previous");
    install_pngsuite(&dest, &files, &backup, &mut stage.keep, rename)?;
    println!("pngsuite: verified {}", dest.display());
    Ok(())
}

/// Older curl versions cannot enforce --max-filesize for unknown-length responses. Write
/// only the allowed prefix, then probe for excess without spooling that byte to disk.
fn copy_pngsuite_download(reader: &mut impl Read, writer: &mut impl Write) -> std::io::Result<()> {
    std::io::copy(&mut reader.take(PNGSUITE_MAX_ARCHIVE_BYTES), writer)?;
    let mut extra = [0; 1];
    loop {
        match reader.read(&mut extra) {
            Ok(0) => return Ok(()),
            Ok(_) => return Err(std::io::Error::other(format!("pngsuite: archive exceeds {PNGSUITE_MAX_ARCHIVE_BYTES} bytes"))),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

fn finish_pngsuite_download(child: &mut Child, mut reader: impl Read, writer: &mut impl Write) -> Result<(), String> {
    let result = copy_pngsuite_download(&mut reader, writer).map_err(|e| format!("pngsuite: download: {e}")).and_then(|()| {
        let status = child.wait().map_err(|e| format!("pngsuite: wait for curl: {e}"))?;
        if status.success() { Ok(()) } else { Err(format!("pngsuite: curl failed: {status}")) }
    });
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn download_pngsuite(mut curl: Command, archive: &Path) -> Result<(), String> {
    let mut file = std::fs::File::create(archive).map_err(|e| format!("create {}: {e}", archive.display()))?;
    let mut child = curl.stdout(Stdio::piped()).spawn().map_err(|e| format!("pngsuite: start curl: {e}"))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("pngsuite: curl stdout unavailable".into());
    };
    finish_pngsuite_download(&mut child, stdout, &mut file)
}

/// PngSuite: fixed archive checked before tar; every cached file checked on each invocation.
fn fetch_pngsuite() -> Result<(), String> {
    fetch_pngsuite_in(
        &root().join("corpus"),
        PNGSUITE_SHA256,
        PNGSUITE_MANIFEST,
        |archive| {
            let mut curl = Command::new("curl");
            curl.args(["-fsSL", "--retry", "3", "--max-filesize", &PNGSUITE_MAX_ARCHIVE_BYTES.to_string(), "-A", USER_AGENT, "-o", "-", PNGSUITE_URL]);
            download_pngsuite(curl, archive)
        },
        |archive, files| {
            let mut tar = Command::new("tar");
            tar.arg("-xzf").arg(archive).arg("-C").arg(files);
            run(tar, "tar -xzf PngSuite-2017jul19.tgz")
        },
        |from, to| std::fs::rename(from, to),
    )
}

/// Files changed relative to `origin/main` (committed, staged or not).
fn changed_files() -> Result<Vec<String>, String> {
    let out = Command::new("git").current_dir(root()).args(["diff", "--name-only", "origin/main"]).output().map_err(|e| format!("git diff: {e}"))?;
    if !out.status.success() {
        return Err(format!("git diff origin/main failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect())
}

/// `cargo xtask test-corpus [-p <crate>]... [--changed] [--local] [-- <test args>]`
pub fn test_cmd(args: &[&str]) -> Result<(), String> {
    let (ours, passthrough) = match args.iter().position(|a| *a == "--") {
        Some(i) => (&args[..i], &args[i + 1..]),
        None => (args, &[][..]),
    };
    let mut raw_changed = false;
    if ours.contains(&"--changed") {
        let changed = changed_files()?;
        let hits: Vec<&String> = changed.iter().filter(|f| CRITICAL.iter().any(|c| f.starts_with(c))).collect();
        if hits.is_empty() {
            println!("test-corpus --changed: nothing under {} changed since origin/main; skipping", CRITICAL.join(", "));
            return Ok(());
        }
        println!("test-corpus --changed: {} critical files changed (e.g. {}); running the corpus tests", hits.len(), hits[0]);
        raw_changed = hits.iter().any(|f| f.starts_with("crates/raw/"));
    }
    let mut crates: Vec<String> = Vec::new();
    let mut it = ours.iter();
    while let Some(a) = it.next() {
        match *a {
            "-p" | "--package" => {
                let name = it.next().ok_or("-p needs a crate name")?;
                let full = if name.starts_with("photocraft-") { (*name).to_string() } else { format!("photocraft-{name}") };
                if !CORPUS_CRATES.contains(&full.as_str()) && !PIXLS_CRATES.contains(&full.as_str()) {
                    return Err(format!("{full} has no corpus tests (crates: {} or the opt-in {})", CORPUS_CRATES.join(", "), PIXLS_CRATES.join(", ")));
                }
                crates.push(full);
            }
            "--changed" | "--local" | "--pixls" => {}
            other => return Err(format!("test-corpus: unknown argument `{other}`")),
        }
    }
    if crates.is_empty() {
        crates = CORPUS_CRATES.iter().map(|s| (*s).to_string()).collect();
    }
    // `--pixls` opts in (and fetches), and a raw change under --changed does too: the raw
    // corpus tests are not part of the default set until a maintainer decides otherwise.
    let wants_pixls = ours.contains(&"--pixls") || crates.iter().any(|c| PIXLS_CRATES.contains(&c.as_str())) || raw_changed;
    if wants_pixls && !crates.iter().any(|c| c == "photocraft-raw") {
        crates.push("photocraft-raw".to_string());
    }
    if wants_pixls {
        crate::pinned::fetch_pixls(false)?;
    }
    fetch_all(ours.contains(&"--local"), false)?;
    let mut c = cargo();
    c.args(["test", "--release", "--lib", "--tests"]);
    for k in &crates {
        c.args(["-p", k]);
    }
    let mut features: Vec<String> = crates.iter().map(|k| format!("{k}/corpus")).collect();
    features.extend(crates.iter().filter(|k| HEIF_CRATES.contains(&k.as_str())).map(|k| format!("{k}/heif")));
    c.args(["--features", &features.join(",")]);
    if !passthrough.is_empty() {
        c.arg("--").args(passthrough);
    }
    run(c, &format!("cargo test --release --features corpus,heif ({})", crates.join(", ")))
}

#[cfg(test)]
mod tests;
