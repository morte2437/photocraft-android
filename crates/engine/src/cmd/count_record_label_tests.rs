//! #11775: Record Measurements labels a Count tool row "Count N"; selection and ruler rows keep
//! their labels, and N is the number the row gets today (the shared counter).

use serde_json::{Value, json};

use crate::Session;

fn five_counts() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    for i in 0..5 {
        s.execute("count.add", json!({"x": 10 + 10 * i, "y": 10})).unwrap();
    }
    s
}

fn record(s: &mut Session, source: &str) -> Value {
    let r = s.execute("image.analysis.recordMeasurements", json!({"source": source})).unwrap();
    let rows = r["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{source}: {r}");
    rows[0]["values"].clone()
}

#[test]
fn a_count_row_is_labelled_count_n() {
    for source in ["count", "auto"] {
        let mut s = five_counts();
        let row = record(&mut s, source);
        assert_eq!(row["label"], json!("Count 1"), "{source}: {row}");
        assert_eq!(row["count"], json!(5), "{source}: {row}");
        assert_eq!(row["source"], json!("Count Tool"), "{source}: {row}");
    }
}

#[test]
fn other_rows_keep_their_labels_and_the_counter_is_shared() {
    let mut s = five_counts();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    let selection = record(&mut s, "selection");
    assert_eq!(selection["label"], json!("Measurement 1"), "{selection}");
    let count = record(&mut s, "count");
    assert_eq!(count["label"], json!("Count 2"), "{count}");
    // `auto` picks the selection over the counts, and keeps its label.
    let auto = record(&mut s, "auto");
    assert_eq!(auto["source"], json!("Selection"), "{auto}");
    assert_eq!(auto["label"], json!("Measurement 3"), "{auto}");
    s.execute("image.analysis.rulerTool", json!({"start": [0, 0], "end": [3, 4]})).unwrap();
    let ruler = record(&mut s, "ruler");
    assert_eq!(ruler["label"], json!("Measurement 4"), "{ruler}");
    let count = record(&mut s, "count");
    assert_eq!(count["label"], json!("Count 5"), "{count}");
}
