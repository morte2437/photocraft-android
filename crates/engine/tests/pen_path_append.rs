use photocraft_engine::Session;
use serde_json::json;

fn session() -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    session
}

#[test]
fn appended_work_and_saved_components_preserve_existing_geometry_and_undo() {
    for name in ["work", "Saved drawing"] {
        let mut session = session();
        let first = json!({"subpaths": [{"closed": false, "knots": [[20, 20], [80, 20]]}]});
        let second = json!({"subpaths": [{"closed": false, "knots": [[30, 60], [90, 60]]}]});
        session.execute("path.set", json!({"name": name, "path": first})).unwrap();
        let before = session.execute("path.info", json!({"name": name})).unwrap();
        session.execute("path.set", json!({"name": name, "path": second, "op": "combine"})).unwrap();
        let after = session.execute("path.info", json!({"name": name})).unwrap();
        assert_eq!(after["path"]["subpaths"].as_array().unwrap().len(), 2);
        assert_eq!(after["path"]["subpaths"][0], before["path"]["subpaths"][0]);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.execute("path.info", json!({"name": name})).unwrap(), before);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.execute("path.info", json!({"name": name})).unwrap(), after);
    }
}

#[test]
fn appending_valid_components_cannot_exceed_total_path_budget() {
    let mut session = session();
    let subpath = json!({"closed": false, "knots": []});
    session.execute("path.set", json!({"path": {"subpaths": vec![subpath.clone(); 2_049]}})).unwrap();
    let before = session.active().unwrap().doc.work_path.clone();
    let history = session.active().unwrap().history.entries().len();
    let error = session.execute("path.set", json!({"path": {"subpaths": vec![subpath; 2_048]}, "op": "combine"})).unwrap_err();
    assert!(error.to_string().contains("4096"));
    assert_eq!(session.active().unwrap().doc.work_path, before);
    assert_eq!(session.active().unwrap().history.entries().len(), history);
}

#[test]
fn invalid_path_coordinates_leave_work_and_saved_paths_unchanged() {
    for name in ["work", "Saved drawing"] {
        let mut session = session();
        session.execute("path.set", json!({"name": name, "path": {"subpaths": [{"knots": [[10, 10], [20, 20]]}]}})).unwrap();
        let before = session.execute("path.info", json!({"name": name})).unwrap();
        let history = session.active().unwrap().history.entries().len();
        assert!(
            session.execute("path.set", json!({"name": name, "op": "combine", "path": {"subpaths": [{"knots": [[-1.0e308, 10], [1.0e308, 20]]}]}})).is_err()
        );
        assert_eq!(session.execute("path.info", json!({"name": name})).unwrap(), before);
        assert_eq!(session.active().unwrap().history.entries().len(), history);
    }
}

#[test]
fn large_straight_work_and_saved_paths_append_and_undo_without_shape_coordinate_limits() {
    for name in ["work", "Saved drawing"] {
        let mut session = session();
        let large = json!({"subpaths": [{"closed": true, "knots": [[0.0, 0.0], [3.0e9, 0.0], [3.0e9, 1.0e9]]}]});
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.entries().len();
        session.execute("path.set", json!({"name":name,"path":large})).unwrap();
        let first = session.execute("path.info", json!({"name":name})).unwrap();
        assert_eq!(first["path"]["subpaths"][0]["knots"][1]["anchor"], json!([3.0e9, 0.0]));
        assert_eq!(session.active().unwrap().history.entries().len(), history + 1);
        session.execute("path.set", json!({"name":name,"op":"combine","path":{"subpaths":[{"knots":[[10,20],[80,20]]}]}})).unwrap();
        let appended = session.execute("path.info", json!({"name":name})).unwrap();
        assert_eq!(appended["path"]["subpaths"].as_array().unwrap().len(), 2);
        assert_eq!(appended["path"]["subpaths"][0], first["path"]["subpaths"][0]);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.execute("path.info", json!({"name":name})).unwrap(), first);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.work_path, before.work_path);
        assert_eq!(session.active().unwrap().doc.paths, before.paths);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.execute("path.info", json!({"name":name})).unwrap(), first);
    }
}
