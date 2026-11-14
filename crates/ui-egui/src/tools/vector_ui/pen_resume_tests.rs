//! Undo back to where a Pen path was finished gives the path back to the Pen, and Redo finishes
//! it again (`pen_resume`).

use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// Edit › Undo or Redo as the menu, the shortcut and the control channel run it.
fn menu(app: &mut PhotocraftApp, id: &str) {
    crate::menus::invoke(app, &egui::Context::default(), id, json!({})).unwrap();
}

/// A triangle drawn with the Pen and closed by clicking its first anchor; the knots it had
/// before the close.
fn close_triangle(app: &mut PhotocraftApp) -> Vec<[[f64; 2]; 3]> {
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        pen_down(app, x, y, false);
        pen_up(app);
    }
    let knots = app.ui.pen.as_ref().unwrap().knots.clone();
    pen_down(app, 20.0, 20.0, false);
    pen_up(app);
    assert!(app.ui.pen.is_none(), "clicking the first anchor commits the path");
    knots
}

/// [`close_triangle`] in Path mode: the closed triangle is the work path.
fn closed_triangle(app: &mut PhotocraftApp) -> Vec<[[f64; 2]; 3]> {
    let knots = close_triangle(app);
    assert!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].closed);
    knots
}

#[test]
fn pen_undo_after_closing_reopens_the_path_instead_of_deleting_it() {
    let mut app = app();
    let before = app.session.active().unwrap().history.past_len();
    let knots = closed_triangle(&mut app);

    // Undo takes back only the closing segment: the path is open again, still being drawn.
    menu(&mut app, "edit.undo");
    let pen = app.ui.pen.as_ref().expect("the open path is back on the Pen");
    assert_eq!(pen.knots, knots);
    assert_eq!(app.session.active().unwrap().history.past_len(), before);
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "the closed path is no longer committed");

    // The next Undo removes the last anchor, as for any path being drawn.
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().unwrap().knots, knots[..2].to_vec());
}

#[test]
fn pen_command_z_after_closing_reopens_the_path() {
    let mut app = app();
    let knots = closed_triangle(&mut app);
    press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots));
    assert!(app.session.active().unwrap().doc.work_path.is_none());
}

/// A key press through the keyboard shortcuts, as the app handles it.
fn press(app: &mut PhotocraftApp, key: egui::Key, modifiers: egui::Modifiers) {
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    });
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

#[test]
fn pen_undo_after_enter_or_esc_gives_the_open_path_back() {
    for key in [egui::Key::Enter, egui::Key::Escape] {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
            pen_down(&mut app, x, y, false);
            pen_up(&mut app);
        }
        let knots = app.ui.pen.as_ref().unwrap().knots.clone();
        press(&mut app, key, egui::Modifiers::NONE);
        assert!(app.ui.pen.is_none(), "{key:?} ends the path");
        assert!(!app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].closed);

        // Undo takes back only the ending: the Pen carries on from the last anchor.
        press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots.clone()), "{key:?}");
        assert!(app.session.active().unwrap().doc.work_path.is_none());
        assert_eq!(app.ui.pen_doc, Some(app.session.active().unwrap().doc.id), "the path given back belongs to its document");
        press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(app.ui.pen.as_ref().unwrap().knots, knots[..2].to_vec(), "then Undo removes the last anchor");

        // Ending it again, then Undo and Redo, works as for a closed path.
        press(&mut app, key, egui::Modifiers::NONE);
        menu(&mut app, "edit.undo");
        menu(&mut app, "edit.redo");
        assert!(app.ui.pen.is_none() && app.ui.pen_doc.is_none());
        assert_eq!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots.len(), 2);
    }
}

#[test]
fn pen_undo_after_a_close_drag_gives_back_the_first_anchor_unchanged() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    let knots = app.ui.pen.as_ref().unwrap().knots.clone();
    // Dragging from the first anchor pulls out its handles in the closed path only.
    pen_down(&mut app, 20.0, 20.0, false);
    pen_move(&mut app, 20.0, 60.0, false);
    pen_up(&mut app);
    let first = &app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots[0];
    assert!(first.smooth && first.out_ctrl.y > 20.0, "the close-drag shaped the closed path");

    menu(&mut app, "edit.undo");
    let pen = app.ui.pen.as_ref().unwrap();
    assert_eq!(pen.knots, knots, "no stand-in knot and no pulled handles");
    assert!(!pen.dragging && !pen.adjusting_last);
}

#[test]
fn curvature_pen_undo_after_closing_reopens_the_path() {
    let mut app = app();
    app.ui.tool = Tool::CurvaturePen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        curvature_down(&mut app, x, y);
        curvature_up(&mut app);
    }
    let knots = app.ui.pen.as_ref().unwrap().knots.clone();
    curvature_down(&mut app, 20.0, 20.0);
    curvature_up(&mut app);
    assert!(app.ui.pen.is_none());
    assert!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].closed);

    menu(&mut app, "edit.undo");
    let pen = app.ui.pen.as_ref().unwrap();
    assert!(pen.curvature);
    assert_eq!(pen.knots, knots);
    assert!(app.session.active().unwrap().doc.work_path.is_none());
}

#[test]
fn redo_closes_a_reopened_pen_path_again() {
    let mut app = app();
    let knots = closed_triangle(&mut app);
    let closed = app.session.active().unwrap().doc.work_path.clone();
    for _ in 0..2 {
        menu(&mut app, "edit.undo");
        assert_eq!(app.ui.pen.as_ref().unwrap().knots, knots);
        menu(&mut app, "edit.redo");
        assert!(app.ui.pen.is_none(), "Redo takes the draft back into the closed path");
        assert_eq!(app.session.active().unwrap().doc.work_path, closed);
    }
}

#[test]
fn redo_after_editing_a_reopened_path_leaves_the_draft_alone() {
    let mut app = app();
    closed_triangle(&mut app);
    menu(&mut app, "edit.undo");
    pen_down(&mut app, 60.0, 160.0, false);
    pen_up(&mut app);
    let draft = app.ui.pen.clone();
    // The draft changed since the reopen, so the closed path is not the one Redo brings back.
    menu(&mut app, "edit.redo");
    assert_eq!(app.ui.pen, draft);
}

#[test]
fn undo_after_a_later_step_takes_that_step_back_first() {
    let mut app = app();
    let knots = closed_triangle(&mut app);
    app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#ff0000"})).unwrap();
    let after_close = app.session.active().unwrap().history.past_len() - 1;
    menu(&mut app, "edit.undo");
    assert!(app.ui.pen.is_none(), "Undo takes back the rectangle only");
    assert_eq!(app.session.active().unwrap().history.past_len(), after_close);
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    // Back at the close, the next Undo reopens the path.
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots));
    assert!(app.session.active().unwrap().doc.work_path.is_none());
}

#[test]
fn undo_gives_every_finished_path_back_to_the_pen_not_just_the_last() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    let mut drafts = Vec::new();
    // Three paths, finished by closing, by Esc and by choosing another tool (↩ would deselect the
    // Work Path, and the next path would replace it, #12892).
    for (n, dy) in [0.0, 50.0, 100.0].into_iter().enumerate() {
        crate::tool_memory::select(&mut app, Tool::Pen);
        for (x, y) in [(20.0, 20.0 + dy), (120.0, 20.0 + dy), (120.0, 60.0 + dy)] {
            pen_down(&mut app, x, y, false);
            pen_up(&mut app);
        }
        drafts.push(app.ui.pen.as_ref().unwrap().knots.clone());
        match n {
            0 => {
                pen_down(&mut app, 20.0, 20.0 + dy, false);
                pen_up(&mut app);
            }
            1 => press(&mut app, egui::Key::Escape, egui::Modifiers::NONE),
            _ => {
                crate::tool_memory::select(&mut app, Tool::Move);
                pen_commit(&mut app, false);
            }
        }
        assert!(app.ui.pen.is_none());
    }
    let subpaths = |app: &PhotocraftApp| app.session.active().unwrap().doc.work_path.as_ref().map_or(0, |p| p.subpaths.len());
    assert_eq!(subpaths(&app), 3, "the paths build up in the work path");

    // Undo walks back through each path: given back to the Pen, then one anchor at a time.
    for (n, knots) in drafts.iter().enumerate().rev() {
        press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(app.ui.tool, Tool::Pen);
        assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots.clone()), "path {n} is being drawn again");
        assert_eq!(subpaths(&app), n, "path {n} is no longer in the work path");
        for left in (0..knots.len()).rev() {
            press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
            assert_eq!(app.ui.pen.as_ref().map_or(0, |p| p.knots.len()), left);
        }
        assert_eq!(subpaths(&app), n, "taking back the anchors leaves the earlier paths alone");
    }
}

#[test]
fn undo_steps_back_from_make_selection_to_the_path_being_drawn() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    let knots = app.ui.pen.as_ref().unwrap().knots.clone();
    // The canvas menu's Make Selection ends the path, then makes the selection from it.
    pen_commit(&mut app, false);
    app.run("path.toSelection", json!({"name": "work"})).unwrap();
    let doc = |app: &PhotocraftApp| app.session.active().unwrap().doc.clone();
    assert!(doc(&app).selection.is_some());

    menu(&mut app, "edit.undo");
    assert!(doc(&app).selection.is_none(), "Undo takes back the selection first");
    assert!(doc(&app).work_path.is_some() && app.ui.pen.is_none());
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots.clone()), "then the path is being drawn again");
    assert!(doc(&app).work_path.is_none());

    // Redo walks forward again, and Undo back still finds the path being drawn.
    menu(&mut app, "edit.redo");
    menu(&mut app, "edit.redo");
    assert!(doc(&app).selection.is_some() && app.ui.pen.is_none());
    menu(&mut app, "edit.undo");
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots));
}

#[test]
fn undo_after_switching_tools_gives_the_path_back_to_the_pen() {
    for (pen_tool, curvature) in [(Tool::Pen, false), (Tool::CurvaturePen, true)] {
        let mut app = app();
        app.ui.tool = pen_tool;
        for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
            if curvature {
                curvature_down(&mut app, x, y);
                curvature_up(&mut app);
            } else {
                pen_down(&mut app, x, y, false);
                pen_up(&mut app);
            }
        }
        let knots = app.ui.pen.as_ref().unwrap().knots.clone();
        // Choosing another tool ends the path, as the frame loop does (`PhotocraftApp::logic`).
        crate::tool_memory::select(&mut app, Tool::Move);
        pen_commit(&mut app, false);
        assert!(app.session.active().unwrap().doc.work_path.is_some());

        press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(app.ui.tool, pen_tool, "the tool that drew the path comes back");
        assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots));
        assert!(app.session.active().unwrap().doc.work_path.is_none());
    }
}

#[test]
fn a_free_transform_box_keeps_its_own_undo_and_redo_over_a_finished_pen_path() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    let knots = app.ui.pen.as_ref().unwrap().knots.clone();
    crate::tool_memory::select(&mut app, Tool::Move);
    pen_commit(&mut app, false);

    // Undo under the box steps the box: the path stays finished, and comes back once it closes.
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    menu(&mut app, "edit.undo");
    assert!(app.ui.transform.is_some() && app.ui.pen.is_none(), "the box took the Undo");
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    crate::transform_tool::cancel(&mut app);
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots));

    // Redo under the box steps the box too, leaving the Pen its path.
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    menu(&mut app, "edit.redo");
    assert!(app.ui.transform.is_some() && app.ui.pen.is_some(), "the box took the Redo");
    assert!(app.session.active().unwrap().doc.work_path.is_none());

    // Nor do ⌘/Ctrl+Z or Edit › Undo take an anchor off the draft under the box.
    let knots = app.ui.pen.as_ref().unwrap().knots.clone();
    press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    menu(&mut app, "edit.undo");
    assert!(app.ui.transform.is_some());
    assert_eq!(app.ui.pen.as_ref().unwrap().knots, knots, "the box took the Undo");
    crate::transform_tool::cancel(&mut app);
    press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots, knots[..knots.len() - 1].to_vec(), "then Undo takes the last anchor");
}

#[test]
fn redo_does_not_put_a_finished_path_beside_its_edited_draft() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    press(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
    menu(&mut app, "edit.undo");
    pen_down(&mut app, 20.0, 120.0, false);
    pen_up(&mut app);
    let knots = app.ui.pen.as_ref().unwrap().knots.clone();
    assert!(!crate::menus::is_enabled(&app, "edit.redo"), "Redo greys out over the edited draft");
    crate::menus::invoke(&mut app, &ctx, "edit.redo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "no finished path beside the draft");
    assert_eq!(app.ui.pen.as_ref().unwrap().knots, knots);

    // Back to what Undo gave back, Redo finishes it again.
    press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    assert!(crate::menus::is_enabled(&app, "edit.redo"));
    menu(&mut app, "edit.redo");
    assert!(app.ui.pen.is_none());
    assert_eq!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots.len(), 3);
}

#[test]
fn redo_of_a_step_made_and_undone_over_a_resumed_path_does_not_finish_it() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    press(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
    menu(&mut app, "edit.undo");
    // Another step over the resumed path, undone: it, not the finish, is next to redo.
    app.run("select.all", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert!(crate::menus::is_enabled(&app, "edit.redo"));
    menu(&mut app, "edit.redo");
    let st = app.session.active().unwrap();
    assert!(st.doc.selection.is_some(), "Redo made the selection again");
    assert!(st.doc.work_path.is_none(), "and didn't finish the path");
    // The steps since changed the document under the draft, which drops it (`PenOwner::valid`).
    assert!(app.ui.pen.is_none());
}

#[test]
fn a_refused_pen_redo_keeps_the_path_claimed_by_its_document() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    press(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
    menu(&mut app, "edit.undo");
    let owner = app.ui.pen_doc;
    assert!(app.ui.pen.is_some() && owner.is_some());
    app.automation_input = true;
    app.services.automation_command = Some(Box::new(|id, _| if id == "edit.redo" { Err("denied".into()) } else { Ok(()) }));
    assert!(pen_finish_on_redo(&mut app).is_err());
    assert!(app.ui.pen.is_some());
    assert_eq!(app.ui.pen_doc, owner, "the path given back still belongs to its document");
}

#[test]
fn pen_undo_after_closing_a_shape_reopens_the_path() {
    let mut app = app();
    app.ui.tool_options.vector_mode = "shape".into();
    let layers = app.session.active().unwrap().doc.layers.len();
    let knots = close_triangle(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
    menu(&mut app, "edit.undo");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "the shape layer is gone");
    assert_eq!(app.ui.pen.as_ref().unwrap().knots, knots);
}

fn work_path(app: &PhotocraftApp) -> Option<Path> {
    app.session.active().unwrap().doc.work_path.clone()
}

type Knots = Vec<[[f64; 2]; 3]>;

/// An open path of two anchors ended with Esc (which keeps it selected, #12892), then carried on from its last anchor (#6593) by one
/// more anchor and ended again. The open path and the carried-on draft's knots.
fn carry_on_a_path(app: &mut PhotocraftApp) -> (Option<Path>, Knots, Knots) {
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0)] {
        pen_down(app, x, y, false);
        pen_up(app);
    }
    let first = app.ui.pen.as_ref().unwrap().knots.clone();
    press(app, egui::Key::Escape, egui::Modifiers::NONE);
    let open = work_path(app);
    pen_down(app, 120.0, 20.0, false);
    pen_up(app);
    assert!(app.ui.pen.as_ref().unwrap().resumes.is_some(), "a click on the endpoint carries the path on");
    pen_down(app, 120.0, 120.0, false);
    pen_up(app);
    let carried = app.ui.pen.as_ref().unwrap().knots.clone();
    assert_eq!(carried.len(), 3);
    press(app, egui::Key::Escape, egui::Modifiers::NONE);
    let work = work_path(app).unwrap();
    assert_eq!((work.subpaths.len(), work.subpaths[0].knots.len()), (1, 3), "the path grew in place");
    (open, first, carried)
}

/// #6413 review: a path carried on from its endpoint (#6593) comes back from Undo still carried
/// on. Undo then takes back the added anchor, then drops the draft and leaves the path as it was
/// (`pen_undo_last_point`); one more Undo gives back the path's own first finish.
#[test]
fn undo_after_finishing_a_carried_on_path_gives_it_back_still_carried_on() {
    let mut app = app();
    let (open, first, carried) = carry_on_a_path(&mut app);

    menu(&mut app, "edit.undo");
    let pen = app.ui.pen.as_ref().expect("the carried-on path is back on the Pen");
    assert_eq!(pen.knots, carried);
    assert!(pen.resumes.is_some(), "still carrying the path on");
    assert_eq!(work_path(&app), open, "the path is as it was before the second finish");

    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(2), "the added anchor goes");
    menu(&mut app, "edit.undo");
    assert!(app.ui.pen.is_none(), "past the added anchors the draft goes");
    assert_eq!(work_path(&app), open, "and the path stays as it was");

    menu(&mut app, "edit.undo");
    let pen = app.ui.pen.as_ref().expect("the first finish is taken back too");
    assert_eq!(pen.knots, first);
    assert!(pen.resumes.is_none());
    assert!(work_path(&app).is_none());
}

#[test]
fn redo_finishes_a_carried_on_path_again_in_place() {
    let mut app = app();
    let (_, _, carried) = carry_on_a_path(&mut app);
    let finished = work_path(&app);
    menu(&mut app, "edit.undo");
    menu(&mut app, "edit.redo");
    assert!(app.ui.pen.is_none());
    assert_eq!(work_path(&app), finished, "one subpath again, not a second one beside it");

    // With the draft dropped, Redo brings the finish back, and Undo gives the path back again.
    menu(&mut app, "edit.undo");
    menu(&mut app, "edit.undo");
    menu(&mut app, "edit.undo");
    assert!(app.ui.pen.is_none());
    menu(&mut app, "edit.redo");
    assert_eq!(work_path(&app), finished);
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(carried));
}

/// The draft Undo gives back belongs to the document state Undo went back to: the next frame
/// doesn't take it for a stale draft and finish it, and ↩ commits it.
#[test]
fn a_path_given_back_by_undo_is_owned_by_the_current_document_state() {
    let mut app = app();
    let knots = closed_triangle(&mut app);
    menu(&mut app, "edit.undo");
    let owner_valid = |app: &PhotocraftApp| app.ui.pen.as_ref().and_then(|p| p.owner.as_ref()).is_some_and(|o| o.valid(app));
    assert!(owner_valid(&app));
    assert!(!cancel_stale_pen(&mut app));
    finish_pen_if_left(&mut app);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots), "the draft is still being drawn");
    assert!(work_path(&app).is_none());
    press(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
    assert!(app.ui.pen.is_none() && !app.ui.status_error, "{}", app.ui.status);
    assert_eq!(work_path(&app).unwrap().subpaths[0].knots.len(), 3);
}

/// Three anchors drawn with the Pen and ended with ↩; their knots.
fn enter_path(app: &mut PhotocraftApp) -> Knots {
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        pen_down(app, x, y, false);
        pen_up(app);
    }
    let knots = app.ui.pen.as_ref().unwrap().knots.clone();
    press(app, egui::Key::Enter, egui::Modifiers::NONE);
    assert!(app.ui.pen.is_none() && work_path(app).is_some());
    knots
}

/// #6413 review: Edit › Step Backward and Step Forward (the History panel's too) step a Pen path
/// as Undo and Redo do, instead of deleting the finished path whole.
#[test]
fn step_backward_and_forward_step_a_pen_path_as_undo_and_redo_do() {
    let mut app = app();
    let knots = closed_triangle(&mut app);
    menu(&mut app, "history.stepBackward");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots.clone()), "the open path is back on the Pen");
    assert!(work_path(&app).is_none());
    menu(&mut app, "history.stepForward");
    assert!(app.ui.pen.is_none());
    assert!(work_path(&app).unwrap().subpaths[0].closed, "Step Forward closes it again");
    menu(&mut app, "history.stepBackward");
    menu(&mut app, "history.stepBackward");
    assert_eq!(app.ui.pen.as_ref().unwrap().knots, knots[..2].to_vec(), "then the last anchor goes");

    // A first path's anchors step back with no History step to go back to.
    let mut app = self::app();
    app.ui.tool = Tool::Pen;
    pen_down(&mut app, 20.0, 20.0, false);
    pen_up(&mut app);
    assert!(crate::menus::is_enabled(&app, "history.stepBackward"));
    menu(&mut app, "history.stepBackward");
    assert!(app.ui.pen.is_none());
}

/// #6413 review: a path finished in another document leaves this document's Redo of a path given
/// back by Undo alone; the next Undo gives the path back again rather than deleting it.
#[test]
fn a_path_finished_in_another_document_keeps_this_documents_pen_redo() {
    let mut app = app();
    let knots = enter_path(&mut app);
    let finished = work_path(&app);
    menu(&mut app, "edit.undo");
    for _ in 0..knots.len() {
        menu(&mut app, "edit.undo");
    }
    assert!(app.ui.pen.is_none() && work_path(&app).is_none(), "every anchor taken back");

    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    enter_path(&mut app);
    assert!(app.session.set_active(0));

    menu(&mut app, "edit.redo");
    assert_eq!(work_path(&app), finished);
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots), "Undo gives the path back to the Pen");
    assert!(work_path(&app).is_none());
}

/// #6413 review: the same anchors redrawn in Shape mode after Undo took the path given back apart
/// are a new draft: Redo doesn't throw it away to bring back the Work Path.
#[test]
fn redo_leaves_the_same_anchors_redrawn_in_shape_mode_alone() {
    let mut app = app();
    let knots = enter_path(&mut app);
    menu(&mut app, "edit.undo");
    for _ in 0..knots.len() {
        menu(&mut app, "edit.undo");
    }
    assert!(app.ui.pen.is_none());
    app.ui.tool_options.vector_mode = "shape".into();
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    assert_eq!(app.ui.pen.as_ref().unwrap().knots, knots);
    assert!(!crate::menus::is_enabled(&app, "edit.redo"), "Redo greys out over the new draft");
    menu(&mut app, "edit.redo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots), "the Shape draft is still being drawn");
    assert!(work_path(&app).is_none(), "no Work Path brought back beside it");
}

/// #6413 review: a draft the document refuses (a locked shape layer) and Esc keeps as the Work
/// Path comes back to the Pen on Undo too.
#[test]
fn undo_gives_back_a_refused_draft_esc_kept_as_the_work_path() {
    let mut app = app();
    app.ui.tool_options.vector_mode = "shape".into();
    close_triangle(&mut app);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    app.ui.tool_options.pen_path_op = "subtract".into();
    for (x, y) in [(40.0, 40.0), (100.0, 40.0), (100.0, 100.0)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    let knots = app.ui.pen.as_ref().unwrap().knots.clone();
    press(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(app.ui.pen.is_none() && work_path(&app).is_some(), "kept as the Work Path: {}", app.ui.status);
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.clone()), Some(knots), "the draft is back on the Pen");
    assert!(work_path(&app).is_none());
}
