//! File > Save a Copy: the suggested name (#14169).

use super::dialogs::suggested_copy_name;
use photocraft_engine::prefs::FileHandling;

fn suffix(do_not_append_copy: bool) -> &'static str {
    let handling = FileHandling { do_not_append_copy, ..FileHandling::default() };
    handling.copy_suffix()
}

#[test]
fn unsaved_document_suggests_its_own_name_without_copy() {
    assert_eq!(suggested_copy_name("sacopy-test", false, suffix(false)), "sacopy-test.psd");
}

#[test]
fn unsaved_document_suggests_its_own_name_with_the_preference_on_too() {
    assert_eq!(suggested_copy_name("sacopy-test", false, suffix(true)), "sacopy-test.psd");
}

#[test]
fn document_with_a_file_keeps_the_copy_suffix() {
    assert_eq!(suggested_copy_name("base.psd", true, suffix(false)), "base copy.psd");
}

#[test]
fn do_not_append_copy_removes_the_suffix_for_a_document_with_a_file() {
    assert_eq!(suggested_copy_name("base.psd", true, suffix(true)), "base.psd");
}
