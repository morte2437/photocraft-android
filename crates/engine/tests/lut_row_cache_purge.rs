//! One integration-test process keeps its global adjustment-table cache isolated from parallel
//! unit tests.
use photocraft_compose::lut_rows::{self, ROW_LEN};
use photocraft_engine::Session;
use serde_json::json;

#[test]
fn purge_all_is_enabled_by_cached_adjustment_tables_and_releases_them() {
    let mut session = Session::new();
    assert!(!session.is_enabled("edit.purge.all"));
    let rows = lut_rows::cached(7, || vec![[0.5; ROW_LEN]; 3]);
    let accounted = lut_rows::cache_bytes();
    assert_eq!(accounted, 3 * ROW_LEN * 4);
    assert!(session.is_enabled("edit.purge.all"), "only the adjustment table cache enables purge");

    let result = session.execute("edit.purge.all", json!({})).unwrap();
    assert_eq!(result["purged"], json!(["adjustment table cache"]));
    assert_eq!(result["bytes"].as_u64(), Some(accounted as u64));
    assert_eq!(lut_rows::cache_bytes(), 0);
    assert!(!session.is_enabled("edit.purge.all"));

    // The next render builds the tables again.
    let mut built = false;
    let again = lut_rows::cached(7, || {
        built = true;
        vec![[0.5; ROW_LEN]; 3]
    });
    assert!(built);
    assert_eq!(rows, again);
}
