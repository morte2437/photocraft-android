//! PSD/PSB recovery regression tests using in-memory Rust-generated fixtures.
//!
//! No `.psd` file is checked into the repository and no Python generator runs.
//! Fixtures are created at the beginning of each test, so parallel test order
//! does not matter. See the ignored test for GUI-ready temporary files.

#[path = "../support/recovery_psd.rs"]
mod recovery_psd;

use photocraft_io::{ImportOptions, import, import_for_open};
use photocraft_psd::PsdFile;

/// Verifies a generated, two-layer PSD opens without source restrictions or damage prompts.
#[test]
fn valid_generated_psd_opens_normally() {
    let bytes = recovery_psd::make_psd(false);
    assert_eq!(bytes.len(), 74_986, "the fixture should retain the reference PSD layout");
    let result = import("valid-two-layers.psd", &bytes).expect("generated healthy PSD must import");
    assert!(!result.source_read_only);
    assert!(!result.preview_only);
    assert_eq!(result.document.layers.len(), 2);
    let (_, damaged) = import_for_open("valid-two-layers.psd", &bytes, &ImportOptions::default()).unwrap();
    assert!(!damaged);
}

/// Verifies a truncated green PackBits row leaves the PSD container and merged image
/// readable, while normal import reports a warning and interactive open reports damage.
#[test]
fn corrupt_generated_rle_channel_keeps_best_effort_import_and_reports_damage() {
    let bytes = recovery_psd::make_psd(true);
    assert_eq!(bytes.len(), 74_986, "the fixture should retain the reference PSD layout");
    let (parsed, _) = PsdFile::from_bytes_and_merged(&bytes, |_| false).expect("the PSD container must stay valid");
    let layer = parsed.layers().iter().find(|layer| layer.name() == "BROKEN orange RLE").expect("generated upper layer exists");

    assert!(layer.decode_channel(0, parsed.header.depth, parsed.header.version).is_ok());
    assert!(layer.decode_channel(1, parsed.header.depth, parsed.header.version).is_err());
    assert!(layer.decode_channel(2, parsed.header.depth, parsed.header.version).is_ok());
    assert!(parsed.decode_merged().is_ok(), "the composite must remain recoverable");

    let imported = import("corrupt-green-rle.psd", &bytes).expect("headless imports keep readable content");
    assert!(!imported.source_read_only);
    assert!(!imported.preview_only);
    assert_eq!(imported.document.layers.len(), 2);
    assert!(imported.warnings.iter().any(|w| w.contains("could not be decoded")));

    let (opened, damaged) = import_for_open("corrupt-green-rle.psd", &bytes, &ImportOptions::default()).unwrap();
    assert!(damaged, "interactive opening must ask whether to continue");
    assert!(!opened.source_read_only, "the flag does not introduce a save restriction");
    assert_eq!(opened.warnings, imported.warnings);
    assert_eq!(opened.document.layers.len(), imported.document.layers.len());
}

/// Verifies a truncated merged preview triggers confirmation without losing intact layers.
#[test]
fn damaged_composite_keeps_layers_and_requests_interactive_confirmation() {
    let mut file = PsdFile::from_bytes(&recovery_psd::make_psd(false)).unwrap();
    file.image_data.data.truncate(file.image_data.data.len() / 2);
    let bytes = file.to_bytes().unwrap();
    let imported = import("damaged-preview.psd", &bytes).expect("intact layers are still usable");
    assert_eq!(imported.document.layers.len(), 2);
    assert!(!imported.source_read_only);
    let (opened, damaged) = import_for_open("damaged-preview.psd", &bytes, &ImportOptions::default()).unwrap();
    assert!(damaged);
    assert_eq!(opened.warnings, imported.warnings);
    assert!(opened.warnings.iter().any(|w| w.contains("merged image")));
}

/// Verifies mismatched resolution metadata produces a warning but no damage prompt.
#[test]
fn ordinary_compatibility_warning_does_not_mean_damaged() {
    use photocraft_psd::{ImageResource, ResolutionInfo, resources::ids};
    let mut file = PsdFile::from_bytes(&recovery_psd::make_psd(false)).unwrap();
    let resolution = ResolutionInfo { v_res_fixed: ResolutionInfo::from_dpi(150.0).v_res_fixed, ..ResolutionInfo::from_dpi(300.0) };
    file.resources.push(ImageResource::new(ids::RESOLUTION_INFO, resolution.to_bytes()));
    let (opened, damaged) = import_for_open("resolution.psd", &file.to_bytes().unwrap(), &ImportOptions::default()).unwrap();
    assert!(!damaged);
    assert!(opened.warnings.iter().any(|w| w.contains("vertical resolution")));
}

/// Verifies a structurally unparseable PSD still fails instead of offering recovery.
#[test]
fn fatal_parse_errors_are_not_turned_into_confirmation_requests() {
    let bytes = b"8BPS\0";
    assert!(import("truncated.psd", bytes).is_err());
    assert!(import_for_open("truncated.psd", bytes, &ImportOptions::default()).is_err());
}

/// Generate the same two PSDs on demand for manual testing in the GUI.
///
/// Run with:
/// cargo test -p photocraft-io --test it \
///   psd_recovery_generated::write_psds_for_manual_gui_test -- --ignored --nocapture
///
/// The files go to the system's temp folder, not into the repository.
#[test]
#[ignore = "writes temporary PSD fixtures only when manual GUI testing is requested"]
fn write_psds_for_manual_gui_test() {
    let output = std::env::temp_dir().join(format!("photocraft-psd-recovery-{}", std::process::id()));
    std::fs::create_dir_all(&output).expect("create temporary fixture directory");

    for (name, corrupt) in [("valid-two-layers.psd", false), ("corrupt-green-rle.psd", true)] {
        let path = output.join(name);
        std::fs::write(&path, recovery_psd::make_psd(corrupt)).expect("write generated temporary PSD");
        println!("{}", path.display());
    }
}
