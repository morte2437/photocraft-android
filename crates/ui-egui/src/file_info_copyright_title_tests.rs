//! #13821: a document File Info marks Copyrighted shows "© " before its name in the document tab
//! and the floating document window's title; Public Domain and Unknown show no prefix.

use serde_json::json;

use crate::PhotocraftApp;

use super::{pro_tab_title, titled_name};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "name": "oracle"})).unwrap();
    app
}

fn title(app: &PhotocraftApp) -> String {
    pro_tab_title(app, 0)
}

fn status(app: &mut PhotocraftApp, status: &str) {
    app.run("file.fileInfo", json!({"copyrightStatus": status})).unwrap();
}

#[test]
fn copyrighted_puts_the_copyright_sign_before_the_name() {
    let mut app = app();
    assert_eq!(title(&app), "oracle @ 100% (RGB/8)");
    status(&mut app, "copyrighted");
    assert_eq!(title(&app), "\u{a9} oracle @ 100% (RGB/8)*");
    assert_eq!(titled_name(&app.session.documents()[0]), "\u{a9} oracle", "the floating window's title has it too");
}

#[test]
fn public_domain_and_unknown_show_no_prefix() {
    let mut app = app();
    status(&mut app, "copyrighted");
    status(&mut app, "publicDomain");
    assert_eq!(title(&app), "oracle @ 100% (RGB/8)*");
    status(&mut app, "copyrighted");
    status(&mut app, "unknown");
    assert_eq!(title(&app), "oracle @ 100% (RGB/8)*");
    assert_eq!(titled_name(&app.session.documents()[0]), "oracle");
}

#[test]
fn undo_and_redo_take_the_prefix_with_them() {
    let mut app = app();
    status(&mut app, "copyrighted");
    assert!(app.session.undo());
    assert!(title(&app).starts_with("oracle @ "), "{}", title(&app));
    assert!(app.session.redo());
    assert!(title(&app).starts_with("\u{a9} oracle @ "), "{}", title(&app));
}

#[test]
fn a_file_opened_with_the_mark_already_set_shows_the_prefix() {
    let packets = [
        // Element form, as Photoshop writes it.
        (
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/"><xmpRights:Marked> True </xmpRights:Marked></rdf:Description></rdf:RDF></x:xmpmeta>"#,
            true,
        ),
        // Attribute form.
        (r#"<rdf:Description xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/" xmpRights:Marked="True"/>"#, true),
        (r#"<rdf:Description xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/" xmpRights:Marked="False"/>"#, false),
        (r#"<rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/"/>"#, false),
        // Single quotes, and the name on its own line or after a tab, with space around the '='.
        ("<rdf:Description xmlns:xmpRights='http://ns.adobe.com/xap/1.0/rights/' xmpRights:Marked='True'/>", true),
        ("<rdf:Description\n   xmpRights:Marked=\"True\"/>", true),
        ("<rdf:Description\txmpRights:Marked = 'True'/>", true),
        ("<rdf:Description\n   xmpRights:Marked='False'/>", false),
        // Not an attribute: a longer name, or the name with no '='.
        ("<rdf:Description xmpRights:MarkedX=\"True\"/>", false),
        ("<rdf:Description xmpRights:Marked/>", false),
    ];
    for (xmp, marked) in packets {
        let mut app = app();
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).metadata.xmp = Some(xmp.into());
        let want = if marked { "\u{a9} oracle @ 100% (RGB/8)" } else { "oracle @ 100% (RGB/8)" };
        assert_eq!(title(&app), want, "{xmp}");
    }
}

#[test]
fn changing_the_status_of_an_opened_document_clears_every_attribute_form() {
    for attr in ["\n   xmpRights:Marked='True'", "\txmpRights:Marked = \"True\"", " xmpRights:Marked=\"True\""] {
        let xmp = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/"{attr}/></rdf:RDF></x:xmpmeta>"#
        );
        for steps in [&["unknown"][..], &["publicDomain", "unknown"]] {
            let mut app = app();
            std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).metadata.xmp = Some(xmp.clone().into());
            assert!(title(&app).starts_with("\u{a9} "), "{attr:?}");
            for &s in steps {
                status(&mut app, s);
            }
            assert!(title(&app).starts_with("oracle @ "), "{attr:?} then {steps:?}: {}", title(&app));
            let packet = app.session.documents()[0].doc.metadata.xmp.as_deref().unwrap_or_default().to_string();
            assert!(!packet.contains("xmpRights:Marked"), "{attr:?} then {steps:?} leaves {packet}");
            assert!(packet.contains("rdf:about=\"\""), "the rest of the description stays: {packet}");
        }
    }
}

#[test]
fn the_name_inside_other_text_is_neither_read_nor_removed() {
    const RDF: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">"#;
    const END: &str = "</rdf:RDF></x:xmpmeta>";
    let headline = r#"photoshop:Headline="Example xmpRights:Marked='True' text""#;
    let quoted = [
        format!("<rdf:Description rdf:about=\"\" {headline}/>"),
        "<rdf:Description rdf:about=\"\"><photoshop:Credit>a xmpRights:Marked=\"True\" b</photoshop:Credit></rdf:Description>".to_string(),
        "<rdf:Description rdf:about=\"\"><!-- <x xmpRights:Marked='True'/> --></rdf:Description>".to_string(),
        "<rdf:Description rdf:about=\"\"><![CDATA[<x xmpRights:Marked='True'/>]]></rdf:Description>".to_string(),
    ];
    for desc in quoted {
        let mut app = app();
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).metadata.xmp = Some(format!("{RDF}{desc}{END}").into());
        assert!(title(&app).starts_with("oracle @ "), "text is not the mark: {desc}");
    }
    // Beside a real attribute: the status change removes the attribute and keeps the headline.
    let mut app = app();
    let xmp = format!("{RDF}<rdf:Description rdf:about=\"\" {headline}\n   xmpRights:Marked='True'/>{END}");
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).metadata.xmp = Some(xmp.into());
    assert!(title(&app).starts_with("\u{a9} "));
    status(&mut app, "unknown");
    assert!(title(&app).starts_with("oracle @ "), "{}", title(&app));
    let packet = app.session.documents()[0].doc.metadata.xmp.as_deref().unwrap_or_default().to_string();
    assert!(packet.contains(headline), "the headline is unchanged: {packet}");
    assert_eq!(packet.matches("xmpRights:Marked").count(), 1, "only the headline's text is left: {packet}");
}
