//! #11764: the Count tool's options-bar readout: "N" for one group (or none), "V (C)" for two or
//! more, V the visible groups' markers and C the current group's (0 while it is hidden).

use serde_json::json;

use super::count_readout;

fn session(groups: &[usize]) -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    for (g, n) in groups.iter().enumerate() {
        s.execute("count.newGroup", json!({"name": format!("G{g}")})).unwrap();
        for i in 0..*n {
            s.execute("count.add", json!({"x": 10 + 5 * i, "y": 10 + 10 * g})).unwrap();
        }
    }
    s
}

fn readout(s: &photocraft_engine::Session) -> String {
    count_readout(&s.active().unwrap().doc.measurement)
}

#[test]
fn no_group_or_one_group_reads_its_markers() {
    assert_eq!(readout(&session(&[])), "0");
    let mut s = session(&[3]);
    assert_eq!(readout(&s), "3");
    s.execute("count.setGroup", json!({"visible": false})).unwrap();
    assert_eq!(readout(&s), "3", "a single hidden group keeps today's count");
}

#[test]
fn two_groups_read_the_visible_total_and_the_current_group() {
    let mut s = session(&[3, 2]);
    // The 2-marker group (made last) is current.
    assert_eq!(readout(&s), "5 (2)");
    s.execute("count.setGroup", json!({"visible": false})).unwrap();
    assert_eq!(readout(&s), "3 (0)");
    // count_total() keeps its meaning: every group.
    assert_eq!(s.active().unwrap().doc.measurement.count_total(), 5);
}

#[test]
fn hiding_another_group_or_switching_the_current_one() {
    let mut s = session(&[3, 2]);
    s.execute("count.setGroup", json!({"group": 0, "visible": false})).unwrap();
    assert_eq!(readout(&s), "2 (2)", "a hidden other group leaves the total only");
    s.execute("count.setGroup", json!({"group": 0, "visible": true, "active": true})).unwrap();
    assert_eq!(readout(&s), "5 (3)");
    assert_eq!(readout(&session(&[3, 2, 4])), "9 (4)");
    assert_eq!(readout(&session(&[0, 0])), "0 (0)");
}

#[test]
fn an_out_of_range_current_group_reads_the_last_one() {
    // The Group dropdown and `count.add` treat a stale index as the last group; the readout too.
    let mut m = session(&[3, 2]).active().unwrap().doc.measurement.clone();
    m.active_count_group = 9;
    assert_eq!(count_readout(&m), "5 (2)");
}
