//! File > Automate groups its rows with separators as the reference application does (#13704).

use crate::menu_catalog::CATALOG;

#[test]
fn automate_rows_fall_into_the_measured_separator_groups() {
    let mut groups: Vec<Vec<&str>> = vec![Vec::new()];
    for row in CATALOG.iter().filter(|r| r.0 == ["File", "Automate"]) {
        if row.3 == "---" {
            groups.push(Vec::new());
        } else if let Some(last) = groups.last_mut() {
            last.push(row.3.trim_start_matches("file.automate."));
        }
    }
    let expected: Vec<Vec<&str>> = vec![
        vec!["batch", "createDroplet"],
        vec!["cropAndStraightenPhotos"],
        vec!["contactSheetII"],
        vec!["conditionalModeChange", "fitImage", "lensCorrection", "mergeToHdrPro", "photomerge"],
    ];
    assert_eq!(groups, expected);
}
