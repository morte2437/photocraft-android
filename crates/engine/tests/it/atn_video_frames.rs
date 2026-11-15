//! #10723: recorded frame edits use the active video layer and the current playhead.
use craft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Descriptor, UnicodeString, Value as D},
};
use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use serde_json::json;
use std::sync::Arc;

const EVENTS: [(&str, &str); 3] = [
    ("insertBlankFrame", "layer.videoLayers.insertBlankFrame"),
    ("duplicateFrame", "layer.videoLayers.duplicateFrame"),
    ("deleteFrame", "layer.videoLayers.deleteFrame"),
];

/// Build typed source fixtures without asking the mapper to generate its own oracle.
fn file(item: AtnItem) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Video"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("Frame edit"), items: vec![item], ..Default::default() }],
        ..Default::default()
    })
    .unwrap()
}

fn item(event: &str, descriptor: Option<Descriptor>) -> AtnItem {
    AtnItem { event: AtnEvent::name(event), descriptor, ..Default::default() }
}

/// Exercise strict public export: skipping an unrepresentable step cannot pass this check.
fn saved(action: &Action) -> AtnItem {
    let (bytes, skipped) = atn_cmds::export("Video", &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    atn::parse(&bytes).unwrap().actions[0].items[0].clone()
}

/// Playback tests require a native mapping, rather than a successfully loaded kept step.
fn load(s: &mut Session, source: AtnItem) {
    let bytes = file(source);
    let report = s.execute("actions.load", json!({"data":photocraft_engine::paint::tile::b64_encode(&bytes)})).unwrap();
    assert_eq!(report["unsupported"], json!([]));
}

/// Compare actual pixel markers; VideoData equality checks only stack shape.
fn pixels(s: &Session, id: LayerId, original: bool) -> Vec<Vec<f32>> {
    let video = s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap();
    let frames = if original { video.original_frames.as_ref().unwrap() } else { &video.frames };
    frames.iter().map(|frame| frame.pixel(0, 0)).collect()
}

/// Distinguish both frame order and original footage from the altered display cache.
fn fixture(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":2,"height":2,"depth":depth,"background":"transparent"})).unwrap();
    s.execute("timeline.create", json!({"duration":3,"fps":24})).unwrap();
    let result = s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
    let id = LayerId(result["layer"].as_u64().unwrap());
    let state = s.active_mut().unwrap();
    let doc = Arc::make_mut(&mut state.doc);
    let mut frames = Vec::new();
    for red in [0.25, 0.5, 0.75] {
        let mut frame = photocraft_raster::Surface::new(doc.pixel_format());
        frame.write_pixel(0, 0, &[red, 0.0, 0.0, 1.0]);
        frames.push(frame);
    }
    let layer = doc.layer_mut(id).unwrap();
    layer.content = LayerContent::Raster(frames[1].clone());
    let video = layer.video.as_mut().unwrap();
    video.original_frames = Some(
        frames
            .iter()
            .rev()
            .map(|frame| {
                let mut original = frame.clone();
                let red = original.pixel(0, 0)[0];
                original.write_pixel(0, 0, &[red, 1.0, 0.0, 1.0]);
                original
            })
            .collect(),
    );
    video.frames = frames;
    let timeline = doc.timeline.as_mut().unwrap();
    timeline.current = 1;
    timeline.work_start = 1;
    timeline.work_end = 2;
    (s, id)
}

#[test]
/// Import's empty placeholder must not erase the wire's descriptor-presence distinction.
fn descriptors_and_step_flags_round_trip_including_an_absent_descriptor() {
    for (event, command) in EVENTS {
        for descriptor in [None, Some(Descriptor::default())] {
            for enabled in [false, true] {
                let mut source = item(event, descriptor.clone());
                source.enabled = enabled;
                source.with_dialog = true;
                let imported = atn_cmds::import(&file(source)).unwrap();
                assert!(imported.unsupported.is_empty());
                let action = &imported.actions[0];
                assert_eq!(action.steps[0].0, command);
                assert_eq!(action.disabled.contains(&0), !enabled);
                assert!(action.dialogs.contains(&0));
                let back = saved(action);
                assert_eq!(back.event, AtnEvent::name(event));
                assert_eq!(back.descriptor, descriptor);
                assert_eq!(back.enabled, enabled);
                assert!(back.with_dialog);
            }
        }
    }
}

#[test]
/// One recorded command must change the correct stack position and remain one undoable edit.
fn playback_edits_the_current_frame_and_original_stack_with_undo_redo_at_each_depth() {
    for (depth, has_descriptor) in [(8, true), (16, true), (32, true), (8, false)] {
        for (event, _) in EVENTS {
            let (mut s, id) = fixture(depth);
            let before = pixels(&s, id, false);
            let originals = pixels(&s, id, true);
            let timeline = s.active().unwrap().doc.timeline.clone();
            let expected = |frames: &[Vec<f32>]| match event {
                "insertBlankFrame" => vec![frames[0].clone(), frames[1].clone(), vec![0.0; 4], frames[2].clone()],
                "duplicateFrame" => vec![frames[0].clone(), frames[1].clone(), frames[1].clone(), frames[2].clone()],
                "deleteFrame" => vec![frames[0].clone(), frames[2].clone()],
                _ => panic!("fixture event"),
            };
            load(&mut s, item(event, has_descriptor.then(Descriptor::default)));
            let report = s.execute("actions.play", json!({"action":0})).unwrap();
            assert!(report.get("failed").is_none(), "{depth}/{event}: {report}");
            assert_eq!(report["ran"], 1);
            assert_eq!(pixels(&s, id, false), expected(&before), "{depth}/{event}");
            assert_eq!(pixels(&s, id, true), expected(&originals), "original {depth}/{event}");
            let t = s.active().unwrap().doc.timeline.as_ref().unwrap();
            assert_eq!((t.current, t.work_start, t.work_end), (1, 1, 2));
            assert_eq!(t.duration, if event == "deleteFrame" { 3 } else { 4 });
            let LayerContent::Raster(display) = &s.active().unwrap().doc.layer(id).unwrap().content else { panic!("video display") };
            assert_eq!(display.pixel(0, 0), expected(&before)[1]);
            s.execute("history.undo", json!({})).unwrap();
            assert_eq!(pixels(&s, id, false), before);
            assert_eq!(pixels(&s, id, true), originals);
            assert_eq!(s.active().unwrap().doc.timeline, timeline);
            s.execute("history.redo", json!({})).unwrap();
            assert_eq!(pixels(&s, id, false), expected(&before));
            assert_eq!(pixels(&s, id, true), expected(&originals));
        }
    }
}

#[test]
/// The native last-frame protection must also hold through recorded playback.
fn deleting_the_last_frame_keeps_its_pixels() {
    let (mut s, id) = fixture(8);
    let state = s.active_mut().unwrap();
    let doc = Arc::make_mut(&mut state.doc);
    let layer = doc.layer_mut(id).unwrap();
    let video = layer.video.as_mut().unwrap();
    video.frames.truncate(1);
    video.original_frames.as_mut().unwrap().truncate(1);
    layer.content = LayerContent::Raster(video.frames[0].clone());
    let t = doc.timeline.as_mut().unwrap();
    *t = photocraft_doc::video::Timeline::new(1, 24.0);
    let before = pixels(&s, id, false);
    let original = pixels(&s, id, true);
    load(&mut s, item("deleteFrame", Some(Descriptor::default())));
    let report = s.execute("actions.play", json!({"action":0})).unwrap();
    assert!(report.get("failed").is_none(), "{report}");
    assert_eq!(pixels(&s, id, false), before);
    assert_eq!(pixels(&s, id, true), original);
}

#[test]
/// Recognizing an event name must never discard settings or reinterpret its unknown header.
fn unrepresented_settings_and_descriptor_metadata_stay_complete_kept_steps() {
    let named = Descriptor { name: UnicodeString::new_nul("Unmeasured metadata"), ..Default::default() };
    for (event, _) in EVENTS {
        for descriptor in [Descriptor::default().with("frame", D::Integer(2)), Descriptor::new("other"), named.clone()] {
            let imported = atn_cmds::import(&file(item(event, Some(descriptor.clone())))).unwrap();
            assert_eq!(imported.unsupported.len(), 1);
            assert_eq!(imported.actions[0].steps[0].0, "actions.photoshopStep");
            assert_eq!(saved(&imported.actions[0]).descriptor, Some(descriptor));
        }
    }
}

#[test]
/// Current-frame recordings cannot serialize an explicit frame, layer or mask selector.
fn native_options_and_damaged_absence_metadata_refuse_export() {
    for (_, command) in EVENTS {
        for params in [
            json!({"frame":2}),
            json!({"layer":3}),
            json!({"target":"mask"}),
            json!({"_atnVideoFrameNoDescriptor":false}),
            json!({"_atnVideoFrameNoDescriptor":"true"}),
        ] {
            let action = Action::new("Invalid", vec![(command.into(), params)]);
            assert!(atn_cmds::export("Video", &[&action], &[], false).is_err(), "{command}");
        }
    }
}

#[test]
/// Mapping a known event cannot bypass the native video-target precondition.
fn playback_without_a_video_target_fails_without_editing_a_document() {
    for (event, _) in EVENTS {
        for document in [false, true] {
            let mut s = Session::new();
            if document {
                s.execute("file.new", json!({"width":2,"height":2})).unwrap();
            }
            let before = s.active().map(|state| state.doc.clone());
            load(&mut s, item(event, Some(Descriptor::default())));
            let report = s.execute("actions.play", json!({"action":0})).unwrap();
            assert!(report.get("failed").is_some(), "{event}: {report}");
            match before {
                Some(before) => assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc)),
                None => assert!(s.active().is_none()),
            }
        }
    }
}
