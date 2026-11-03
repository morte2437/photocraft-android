//! #7366: a background Remove on a Red-only target keeps G/B/alpha intact when a Type click
//! selects another layer while the job is pending.

use photocraft_doc::LayerId;
use photocraft_engine::Session;
use photocraft_engine::jobs::Started;
use photocraft_ui_egui::{PhotocraftApp, Services, control, type_tool};

const REMOVE: &str = r#"{"points":[[12,10],[16,12]],"size":6}"#;

fn run(s: &mut Session, id: &str, p: &str) {
    s.execute(id, p.parse().unwrap()).unwrap();
}

fn pixels(s: &Session, doc: usize, id: LayerId) -> Vec<f32> {
    let d = &s.documents()[doc].doc;
    d.layer(id).unwrap().surface().unwrap().read_region(d.bounds())
}

#[derive(Clone, Copy)]
enum Input {
    None,
    TypeTool,
    ControlPointer,
}

fn probe(input: Input) {
    let mut a = PhotocraftApp::new(Session::new(), Services::default());
    run(&mut a.session, "file.new", r#"{"name":"A","width":32,"height":24,"background":"transparent"}"#);
    run(&mut a.session, "type.create", r#"{"text":"X","x":2,"y":18,"size":14}"#);
    let text = a.session.active().unwrap().active_layer.unwrap();
    run(&mut a.session, "layer.new.layer", r#"{"name":"Target"}"#);
    let target = a.session.active().unwrap().active_layer.unwrap();
    a.session
        .edit("fixture", |d, l| {
            let q = d.layer_mut(l.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..24 {
                for x in 0..32 {
                    q.write_pixel(x, y, &[((x * 7 + y * 13) % 31) as f32 / 30., (x % 5) as f32 / 4., (y % 7) as f32 / 6., 1.]);
                }
            }
            Ok(())
        })
        .unwrap();
    run(&mut a.session, "file.new", r##"{"name":"B","width":32,"height":24,"background":"#6a219c"}"##);
    a.session.set_active(0);
    a.sync_views();
    run(&mut a.session, "channel.target", r#"{"channel":"red"}"#);
    let b = a.session.documents()[1].doc.clone();
    let text_before = a.session.documents()[0].doc.layer(text).unwrap().clone();

    // The inline Red-only Remove is the oracle.
    let mut inline = Session::new();
    inline.add_document(a.session.documents()[0].doc.as_ref().clone(), None);
    inline.select_layer(target).unwrap();
    run(&mut inline, "channel.target", r#"{"channel":"red"}"#);
    run(&mut inline, "paint.remove", REMOVE);
    let expected = pixels(&inline, 0, target);

    let job = match a.session.start("paint.remove", REMOVE.parse().unwrap()).unwrap() {
        Started::Job(id) => id,
        v => panic!("must start a background job: {v:?}"),
    };
    match input {
        Input::None => {}
        Input::TypeTool => assert!(type_tool::pointer_down(&mut a, 4., 14., Default::default()), "the click must hit the text"),
        Input::ControlPointer => {
            let (req, _rx) = control::ControlRequest::new("ui.pointer", r#"{"tool":"type","events":[{"kind":"down","x":4,"y":14}]}"#.parse().unwrap());
            let _ = control::handle(&mut a, &Default::default(), &req);
        }
    }
    let selected = if matches!(input, Input::None) { target } else { text };
    assert_eq!(a.session.active().unwrap().active_layer, Some(selected), "the input selects the text layer");

    a.session.set_active(1);
    a.session.wait_job(job).unwrap();
    assert_eq!(a.session.documents()[1].doc, b, "B is unchanged");
    let st = &a.session.documents()[0];
    assert_eq!(st.doc.layer(text).unwrap(), &text_before, "the text layer is unchanged");
    assert_eq!(st.active_layer, Some(selected), "the user's selection survives the job");
    let actual = pixels(&a.session, 0, target);
    let wrong = expected.as_chunks::<4>().0.iter().zip(actual.as_chunks::<4>().0).filter(|(e, r)| e[1..] != r[1..]).count();
    assert_eq!(wrong, 0, "pixels with G/B/alpha changed outside the Red target");
    assert_eq!(actual, expected);
}

#[test]
fn no_input_keeps_red_restriction() {
    probe(Input::None);
}

#[test]
fn type_tool_click_keeps_red_restriction() {
    probe(Input::TypeTool);
}

#[test]
fn control_pointer_click_keeps_red_restriction() {
    probe(Input::ControlPointer);
}
