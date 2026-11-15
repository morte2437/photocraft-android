use craft_psd::descriptor::{Descriptor, UnicodeString, Value, VersionedDescriptor};
use craft_psd::{ColorMode as PsdMode, ImageResource, PsdFile};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Color, ColorTable, CountGroup, Document};
use photocraft_geom::Size;

use super::{COUNT_INFO, count_groups_from_resource};

fn measured_resource(second_y: f64) -> Vec<u8> {
    let point = |x, y| Value::Descriptor(Descriptor::new("countObject").with("X   ", Value::Double(x)).with("Y   ", Value::Double(y)));
    let first = Descriptor::new("countGroup")
        .with("Rd  ", Value::Integer(51))
        .with("Grn ", Value::Integer(102))
        .with("Bl  ", Value::Integer(204))
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Cells")))
        .with("Rds ", Value::Integer(3))
        .with("fontSize", Value::Integer(12))
        .with("Vsbl", Value::Boolean(true))
        .with("countObjectList", Value::List(vec![point(9.5, 10.5), point(1.125, second_y), point(3.5, 4.5)]));
    let second = Descriptor::new("countGroup")
        .with("Rd  ", Value::Integer(255))
        .with("Grn ", Value::Integer(0))
        .with("Bl  ", Value::Integer(0))
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Café")))
        .with("Rds ", Value::Integer(10))
        .with("fontSize", Value::Integer(72))
        .with("Vsbl", Value::Boolean(false))
        .with("countObjectList", Value::List(vec![point(7.5, 8.5), point(5.5, 6.5)]));
    VersionedDescriptor::new(
        Descriptor::new("Cnt ").with("Vrsn", Value::Integer(1)).with("countGroupList", Value::List(vec![Value::Descriptor(first), Value::Descriptor(second)])),
    )
    .to_bytes()
}

fn groups() -> Vec<CountGroup> {
    vec![
        CountGroup {
            name: "Cells".into(),
            color: Color::rgb(51.0 / 255.0, 102.0 / 255.0, 204.0 / 255.0),
            marker_size: 3,
            label_size: 12,
            visible: true,
            points: vec![[9.5, 10.5], [1.125, 2.75], [3.5, 4.5]],
        },
        CountGroup {
            name: "Café".into(),
            color: Color::rgb(1.0, 0.0, 0.0),
            marker_size: 10,
            label_size: 72,
            visible: false,
            points: vec![[7.5, 8.5], [5.5, 6.5]],
        },
    ]
}

fn document(mode: ColorMode) -> Document {
    Document::new("Counts", Size::new(16, 16), mode, SampleType::U8)
}

fn to_psd(doc: &Document) -> Vec<u8> {
    crate::export(doc, "counts.psd", &crate::ExportOptions::default()).unwrap().bytes
}

fn import_counts(data: Vec<u8>) -> Document {
    let mut file = PsdFile::from_bytes(&to_psd(&document(ColorMode::Rgb))).unwrap();
    file.resources.push(ImageResource::new(COUNT_INFO, data));
    crate::import("counts.psd", &file.to_bytes().unwrap()).unwrap().document
}

/// Two independently constructed groups preserve every option and the order of all five points.
#[test]
fn parses_hand_built_count_descriptor() {
    assert_eq!(count_groups_from_resource(&measured_resource(2.75)), Some(groups()));
    assert!(count_groups_from_resource(&measured_resource(f64::INFINITY)).is_none());
    let mut trailing = measured_resource(2.75);
    trailing.push(1);
    assert!(count_groups_from_resource(&trailing).is_none());
}

/// Count groups round-trip through the real PSD reader and both export routes.
#[test]
fn count_groups_round_trip_through_psd() {
    for (mode, wire_mode) in [(ColorMode::Rgb, PsdMode::Rgb), (ColorMode::Bitmap, PsdMode::Bitmap), (ColorMode::Multichannel, PsdMode::Multichannel)] {
        let mut doc = document(mode);
        doc.measurement.count_groups = groups();
        let exported = crate::export(&doc, "counts.psd", &crate::ExportOptions::default()).unwrap();
        assert!(!exported.warnings.iter().any(|warning| warning.starts_with("Count group ")));
        let bytes = exported.bytes;
        let file = PsdFile::from_bytes(&bytes).unwrap();
        assert_eq!(file.header.color_mode, wire_mode);
        let resources: Vec<_> = file.resources.iter().filter(|r| r.id == COUNT_INFO).collect();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].data, measured_resource(2.75));
        let reopened = crate::import("counts.psd", &bytes).unwrap().document;
        assert_eq!(reopened.measurement.count_groups, doc.measurement.count_groups);
        assert!(!reopened.metadata.psd_resources.iter().any(|(id, _, _)| *id == COUNT_INFO));
    }
}

/// Both export routes report clamped sizes and preserve the original Count groups.
#[test]
fn count_size_clamps_warn_without_changing_source_groups() {
    for mode in [ColorMode::Rgb, ColorMode::Bitmap] {
        let mut doc = document(mode);
        let mut original = groups();
        original[0].marker_size = 0;
        original[0].label_size = 73;
        original[1].marker_size = 11;
        original[1].label_size = 7;
        doc.measurement.count_groups = original.clone();

        let exported = crate::export(&doc, "counts.psd", &crate::ExportOptions::default()).unwrap();
        let warnings: Vec<_> = exported.warnings.iter().filter(|warning| warning.starts_with("Count group ")).map(String::as_str).collect();
        assert_eq!(
            warnings,
            [
                "Count group \"Cells\" sizes adjusted for PSD (marker 0 to 1, label 73 to 72)",
                "Count group \"Café\" sizes adjusted for PSD (marker 11 to 10, label 7 to 8)",
            ]
        );
        assert_eq!(doc.measurement.count_groups, original);

        let reopened = crate::import("counts.psd", &exported.bytes).unwrap().document;
        let mut expected = original;
        expected[0].marker_size = 1;
        expected[0].label_size = 72;
        expected[1].marker_size = 10;
        expected[1].label_size = 8;
        assert_eq!(reopened.measurement.count_groups, expected);
    }
}

/// Opaque data survives an untouched save and an add-then-delete, but current groups win.
#[test]
fn unreadable_count_resource_is_preserved_without_groups() {
    let raw = vec![0, 0, 0, 16, 0];
    let mut doc = import_counts(raw.clone());
    assert!(doc.measurement.count_groups.is_empty());
    let check_raw = |doc: &Document| {
        let file = PsdFile::from_bytes(&to_psd(doc)).unwrap();
        assert_eq!(file.resource(COUNT_INFO).unwrap().data, raw);
    };
    check_raw(&doc);
    doc.measurement.count_groups.push(CountGroup::default());
    let file = PsdFile::from_bytes(&to_psd(&doc)).unwrap();
    let mut resources = file.resources.iter().filter(|r| r.id == COUNT_INFO);
    assert_eq!(count_groups_from_resource(&resources.next().unwrap().data), Some(doc.measurement.count_groups.clone()));
    assert!(resources.next().is_none());
    doc.measurement.count_groups.clear();
    check_raw(&doc);
}

/// Deleting the imported groups cannot bring back the readable resource.
#[test]
fn deleting_all_readable_count_groups_writes_no_resource() {
    let mut doc = import_counts(measured_resource(2.75));
    assert_eq!(doc.measurement.count_groups, groups());
    assert!(!doc.metadata.psd_resources.iter().any(|(id, _, _)| *id == COUNT_INFO));
    doc.measurement.count_groups.clear();
    let file = PsdFile::from_bytes(&to_psd(&doc)).unwrap();
    assert!(file.resource(COUNT_INFO).is_none());
}

/// Indexed export uses the shared layerless writer and keeps exactly one Count resource.
#[test]
fn indexed_count_groups_round_trip_through_psd() {
    let mut doc = document(ColorMode::Indexed);
    doc.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 255, 255]], transparent: None });
    let opaque = vec![0, 0, 0, 16, 0];
    doc.metadata.psd_resources.push((COUNT_INFO, String::new(), std::sync::Arc::new(opaque.clone())));
    let file = PsdFile::from_bytes(&to_psd(&doc)).unwrap();
    assert_eq!(file.header.color_mode, PsdMode::Indexed);
    assert_eq!(file.resource(COUNT_INFO).unwrap().data, opaque);

    doc.measurement.count_groups = groups();
    let bytes = to_psd(&doc);
    let file = PsdFile::from_bytes(&bytes).unwrap();
    assert_eq!(file.header.color_mode, PsdMode::Indexed);
    assert!(file.layer_info.is_none());
    let resources: Vec<_> = file.resources.iter().filter(|r| r.id == COUNT_INFO).collect();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].data, measured_resource(2.75));
    let reopened = crate::import("counts.psd", &bytes).unwrap().document;
    assert_eq!(reopened.color_table, doc.color_table);
    assert_eq!(reopened.measurement.count_groups, doc.measurement.count_groups);
}

/// Both PSD routes report omitted points, keeping the finite points in order.
#[test]
fn nonfinite_count_points_warn_on_psd_export() {
    for (mode, wire_mode) in [(ColorMode::Rgb, PsdMode::Rgb), (ColorMode::Bitmap, PsdMode::Bitmap)] {
        let mut doc = document(mode);
        doc.measurement.count_groups = groups();
        let options = crate::ExportOptions::default();
        let clean = crate::export(&doc, "counts.psd", &options).unwrap();
        assert!(!clean.warnings.iter().any(|warning| warning.starts_with("Count group ")));
        let reopened = crate::import("counts.psd", &clean.bytes).unwrap().document;
        assert_eq!(reopened.measurement.count_groups, doc.measurement.count_groups);

        let mut expected = groups();
        expected[0].points.remove(1);
        doc.measurement.count_groups[0].points[1][1] = f64::INFINITY;
        let original = doc.measurement.count_groups.clone();
        let exported = crate::export(&doc, "counts.psd", &options).unwrap();
        let warnings: Vec<_> = exported.warnings.iter().filter(|warning| warning.starts_with("Count group ")).map(String::as_str).collect();
        assert_eq!(warnings, ["Count group \"Cells\" had nonfinite points omitted from PSD export (1 total)"]);
        assert_eq!(doc.measurement.count_groups, original);
        let file = PsdFile::from_bytes(&exported.bytes).unwrap();
        assert_eq!(file.header.color_mode, wire_mode);
        let reopened = crate::import("counts.psd", &exported.bytes).unwrap().document;
        assert_eq!(reopened.measurement.count_groups, expected);
    }
}
