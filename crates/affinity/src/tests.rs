//! Unit tests on synthetic inputs; `tests/it/real_files.rs` covers public Affinity documents.

mod preview {
    use crate::container::header;
    use crate::*;
    use proptest::prelude::*;

    fn fixture() -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(13u32.to_be_bytes());
        png.extend(b"IHDR");
        png.extend(2u32.to_be_bytes());
        png.extend(1u32.to_be_bytes());
        png.extend([8, 6, 0, 0, 0]);
        png.extend(crc32fast::hash(&png[12..]).to_be_bytes());
        png.extend([0; 4]);
        png.extend(b"IDAT");
        png.extend(crc32fast::hash(b"IDAT").to_be_bytes());
        png.extend([0; 4]);
        png.extend(b"IEND");
        png.extend(crc32fast::hash(b"IEND").to_be_bytes());
        let mut b = vec![0; 72];
        b[..4].copy_from_slice(MAGIC);
        b[4..6].copy_from_slice(&12u16.to_le_bytes());
        b[8..12].copy_from_slice(b"nsrP");
        b[12..16].copy_from_slice(b"#Inf");
        b[24..32].copy_from_slice(&72u64.to_le_bytes());
        b[64..68].copy_from_slice(b"Prot");
        b.extend(b"\xff\xff\xff\xffThmb");
        b.extend(1u32.to_le_bytes());
        b.extend((png.len() as u32 + 13).to_le_bytes());
        b.extend(29u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend((png.len() as u32).to_le_bytes());
        b.push(1);
        b.extend(png);
        b
    }

    #[test]
    fn indexed_preview_and_header() {
        let b = fixture();
        assert_eq!(header(&b).unwrap().version, 12);
        let p = preview(&b).unwrap();
        assert_eq!((p.width, p.height), (2, 1));
        assert_eq!(p.png, &b[101..]);
    }

    #[test]
    fn every_truncation_is_rejected() {
        let b = fixture();
        for end in 0..b.len() {
            assert!(preview(&b[..end]).is_err(), "{end}");
        }
    }

    #[test]
    fn rejects_assets_versions_offsets_lengths_and_bombs() {
        let b = fixture();
        for (offset, value) in [
            (4, 13u64.to_le_bytes().to_vec()),
            (8, b"urBR".to_vec()),
            (24, u64::MAX.to_le_bytes().to_vec()),
            (24, 1u64.to_le_bytes().to_vec()),
            (24, 0u64.to_le_bytes().to_vec()),
            (84, u32::MAX.to_le_bytes().to_vec()),
            (88, 30u32.to_le_bytes().to_vec()),
            (96, u32::MAX.to_le_bytes().to_vec()),
            (117, 5000u32.to_be_bytes().to_vec()),
        ] {
            let mut broken = b.clone();
            broken[offset..offset + value.len()].copy_from_slice(&value);
            assert!(preview(&broken).is_err(), "offset {offset}");
        }
        // Versions 8 to 11 store the same record (checked on public Affinity 1 and 2 files).
        for version in 8..12 {
            let mut legacy = b.clone();
            legacy[4..6].copy_from_slice(&(version as u16).to_le_bytes());
            assert!(preview(&legacy).is_ok());
        }
        // Flags 4 and 8 occur in real files; the low two bits mark other container variants.
        for (flags, ok) in [(4u16, true), (8, true), (1, false), (2, false)] {
            let mut variant = b.clone();
            variant[6..8].copy_from_slice(&flags.to_le_bytes());
            assert_eq!(preview(&variant).is_ok(), ok, "flags {flags}");
        }
        let mut old = b.clone();
        old[4..6].copy_from_slice(&7u16.to_le_bytes());
        assert!(matches!(preview(&old), Err(Error::Unsupported(_))));
    }

    #[test]
    fn an_unindexed_png_or_resource_is_never_used() {
        let mut b = fixture();
        b[24..32].copy_from_slice(&0u64.to_le_bytes());
        assert!(preview(&b).is_err());
        b[24..32].copy_from_slice(&72u64.to_le_bytes());
        b[76..80].copy_from_slice(b"Meta");
        assert!(preview(&b).is_err());
    }

    #[test]
    fn every_chunk_checksum_including_the_end_is_checked() {
        let b = fixture();
        for at in [130, b.len() - 1] {
            let mut damaged = b.clone();
            damaged[at] ^= 1;
            assert_eq!(preview(&damaged).unwrap_err(), Error::Malformed("PNG chunk checksum"));
        }
        let mut ancillary = vec![0; 4];
        ancillary.extend(b"tEXt");
        ancillary.extend(crc32fast::hash(b"tEXt").to_be_bytes());
        let mut with_ancillary = b.clone();
        let at = b.len() - 12;
        with_ancillary.splice(at..at, ancillary);
        let png_length = (with_ancillary.len() - 101) as u32;
        with_ancillary[84..88].copy_from_slice(&(png_length + 13).to_le_bytes());
        with_ancillary[96..100].copy_from_slice(&png_length.to_le_bytes());
        assert!(preview(&with_ancillary).is_ok());
        with_ancillary[at + 8] ^= 1;
        assert_eq!(preview(&with_ancillary).unwrap_err(), Error::Malformed("PNG chunk checksum"));
    }

    #[test]
    fn compressed_metadata_and_animation_are_not_preview_support() {
        for kind in [b"zTXt", b"iTXt", b"iCCP", b"acTL", b"fcTL", b"fdAT"] {
            let mut extra = vec![0; 4];
            extra.extend(kind);
            extra.extend(crc32fast::hash(kind).to_be_bytes());
            let mut b = fixture();
            let at = b.len() - 12;
            b.splice(at..at, extra);
            let length = (b.len() - 101) as u32;
            b[84..88].copy_from_slice(&(length + 13).to_le_bytes());
            b[96..100].copy_from_slice(&length.to_le_bytes());
            assert!(matches!(preview(&b), Err(Error::Unsupported(_))));
        }
    }

    #[test]
    fn critical_chunks_follow_a_single_complete_png_layout() {
        fn with_chunks(kinds: &[&[u8; 4]]) -> Vec<u8> {
            let original = fixture();
            let mut b = original[..134].to_vec(); // File, record and complete first IHDR.
            for kind in kinds {
                b.extend([0; 4]);
                b.extend(*kind);
                b.extend(crc32fast::hash(*kind).to_be_bytes());
            }
            let length = (b.len() - 101) as u32;
            b[84..88].copy_from_slice(&(length + 13).to_le_bytes());
            b[96..100].copy_from_slice(&length.to_le_bytes());
            b
        }
        assert!(preview(&with_chunks(&[b"IDAT", b"IDAT", b"IEND"])).is_ok());
        // PNG readers must not reject an otherwise valid unknown ancillary chunk just
        // because its reserved (third-letter) bit is set; future versions can define it.
        assert!(preview(&with_chunks(&[b"abct", b"IDAT", b"IEND"])).is_ok());
        for kinds in [
            vec![b"IEND"],
            vec![b"IDAT", b"IHDR", b"IEND"],
            vec![b"PLTE", b"PLTE", b"IDAT", b"IEND"],
            vec![b"IDAT", b"PLTE", b"IEND"],
            vec![b"IDAT", b"tEXt", b"IDAT", b"IEND"],
            vec![b"ABCD", b"IDAT", b"IEND"],
            vec![b"IDAT", b"IEND", b"tEXt"],
        ] {
            assert!(preview(&with_chunks(&kinds)).is_err(), "{kinds:?}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        fn arbitrary_input_never_panics(b in prop::collection::vec(any::<u8>(), 0..2048)) {
            let _ = header(&b);
            let _ = preview(&b);
        }
        #[test]
        fn mutations_never_panic(edits in prop::collection::vec((0usize..200, any::<u8>()), 0..32)) {
            let mut b = fixture();
            for (i, value) in edits { if let Some(v) = b.get_mut(i) { *v = value; } }
            let _ = preview(&b);
        }
    }
}

mod archive {
    use crate::Error;
    use crate::container::{Archive, Limits};
    use crate::stream::{self, Kind, Tag, Value};
    use crate::synth::{self, F, Method, tag};
    use proptest::prelude::*;

    fn doc() -> Vec<u8> {
        synth::stream(&[
            (tag(b"Desc"), F::Str("Document".into())),
            (tag(b"Visi"), F::Bool(true)),
            (tag(b"Xfrm"), F::F64s(vec![1.0, 0.0, 5.0, 0.0, 1.0, -2.0])),
            (tag(b"Colr"), F::Def(7, vec![tag(b"RGBA")], vec![(tag(b"_col"), F::Struct([0u8; 16].to_vec()))])),
            (tag(b"Same"), F::Ref(7)),
            (tag(b"Null"), F::Null),
            (tag(b"Kids"), F::Shared(vec![F::Def(8, vec![tag(b"Grup"), tag(b"Node")], vec![(tag(b"Desc"), F::Str("g".into()))]), F::Ref(8)])),
            (tag(b"Crvs"), F::Obj(tag(b"PCvD"), vec![(tag(b"Data"), F::Pos(vec![F::U8(0), F::U32(1), F::Bool(true), F::Records(18, vec![vec![0; 18]; 4])]))])),
            (tag(b"Enum"), F::Enum(2, 1)),
            (tag(b"Tile"), F::Entry("d/3".into())),
            (tag(b"Stat"), F::U8s(vec![4, 0, 2])),
        ])
    }

    #[test]
    fn every_method_round_trips_with_checksums() {
        let d = doc();
        for method in [Method::Stored, Method::Zlib, Method::Zstd] {
            let file = synth::container(&[("doc.dat", &d, method), ("d/3", b"tile", method)], None);
            let mut a = Archive::open(&file, Limits::default()).unwrap();
            assert_eq!(a.header.version, 12);
            assert_eq!(a.read("doc.dat").unwrap(), d);
            assert_eq!(a.read("d/3").unwrap(), b"tile");
            assert_eq!(a.extracted(), d.len() + 4);
            assert!(a.read("missing").is_err());
        }
    }

    #[test]
    fn the_stream_parses_into_shared_objects_and_links() {
        let s = stream::parse(&doc()).unwrap();
        let root = &s.objects[s.root];
        assert_eq!(root.class, Tag::of(b"Pers"));
        assert_eq!(root.get(Tag::of(b"Desc")), Some(&Value::Str("Document".into())));
        assert_eq!(root.get(Tag::of(b"Xfrm")), Some(&Value::Floats(vec![1.0, 0.0, 5.0, 0.0, 1.0, -2.0])));
        let (Some(Value::Obj(Some(a))), Some(Value::Obj(Some(b)))) = (root.get(Tag::of(b"Colr")), root.get(Tag::of(b"Same"))) else { panic!() };
        assert_eq!(a, b);
        assert_eq!(s.objects[*a].class, Tag::of(b"RGBA"));
        assert_eq!(s.objects[*a].kind, Kind::Shared);
        let Some(Value::Array(kids)) = root.get(Tag::of(b"Kids")) else { panic!() };
        assert_eq!(kids[0], kids[1]);
        let Value::Obj(Some(g)) = kids[0] else { panic!() };
        assert_eq!(s.objects[g].chain, vec![Tag::of(b"Grup"), Tag::of(b"Node")]);
        assert_eq!(root.get(Tag::of(b"Enum")), Some(&Value::Enum { id: 2, version: 1 }));
        assert_eq!(root.get(Tag::of(b"Tile")), Some(&Value::Entry("d/3".into())));
        assert_eq!(root.get(Tag::of(b"Null")), Some(&Value::Obj(None)));
    }

    #[test]
    fn checksums_sizes_and_budgets_are_enforced() {
        let d = doc();
        let file = synth::container(&[("doc.dat", &d, Method::Zstd)], None);
        // Flip a byte inside the compressed payload, past the "#Fil" tag at offset 72.
        let mut broken = file.clone();
        broken[90] ^= 0x55;
        let mut a = Archive::open(&broken, Limits::default()).unwrap();
        assert!(a.read("doc.dat").is_err());
        let mut a = Archive::open(&file, Limits { max_entry: d.len() - 1, max_total: usize::MAX, ..Limits::default() }).unwrap();
        assert!(matches!(a.read("doc.dat"), Err(Error::Limit(_))));
        let mut a = Archive::open(&file, Limits { max_entry: usize::MAX, max_total: d.len() + 1, ..Limits::default() }).unwrap();
        a.read("doc.dat").unwrap();
        assert!(matches!(a.read("doc.dat"), Err(Error::Limit(_))));
    }

    #[test]
    fn a_failed_read_still_spends_the_extraction_allowance() {
        let d = doc();
        let file = synth::container(&[("doc.dat", &d, Method::Zstd)], None);
        let mut broken = file.clone();
        broken[90] ^= 0x55;
        // Room for exactly one read: the failed attempt must use it up, so a caller that
        // recovers from the checksum error cannot make the same entry decode again.
        let mut a = Archive::open(&broken, Limits { max_entry: usize::MAX, max_total: d.len(), ..Limits::default() }).unwrap();
        assert!(a.read("doc.dat").is_err());
        assert_eq!(a.extracted(), d.len());
        assert!(matches!(a.read("doc.dat"), Err(Error::Limit(_))));
    }

    #[test]
    fn a_declared_size_smaller_than_the_data_is_rejected_not_truncated() {
        let big = vec![7u8; 1 << 20];
        let mut file = synth::container(&[("doc.dat", &big, Method::Zlib)], None);
        // Rewrite the declared size in the table to 1 KiB: decoding must stop and fail.
        let fat = u64::from_le_bytes(file[16..24].try_into().unwrap()) as usize;
        let size_at = fat + 59 + 5 + 8;
        file[size_at..size_at + 8].copy_from_slice(&1024u64.to_le_bytes());
        let mut a = Archive::open(&file, Limits::default()).unwrap();
        assert_eq!(a.read("doc.dat").unwrap_err(), Error::Malformed("archive entry size mismatch"));
    }

    #[test]
    fn libraries_and_cyclic_tables_are_rejected() {
        let d = doc();
        let mut file = synth::container(&[("doc.dat", &d, Method::Stored)], None);
        let mut library = file.clone();
        library[8..12].copy_from_slice(b"rAsA");
        assert!(matches!(Archive::open(&library, Limits::default()), Err(Error::Unsupported(_))));
        let fat = u64::from_le_bytes(file[16..24].try_into().unwrap());
        let at = fat as usize + 4;
        file[at..at + 8].copy_from_slice(&fat.to_le_bytes());
        assert_eq!(Archive::open(&file, Limits::default()).unwrap_err(), Error::Malformed("allocation tables form a cycle"));
    }

    #[test]
    fn hostile_streams_fail_cleanly() {
        let deep = (0..2000).fold(Vec::new(), |inner, _| vec![(tag(b"Chld"), F::Obj(tag(b"Grup"), inner))]);
        assert_eq!(stream::parse(&synth::stream(&deep)).unwrap_err(), Error::Limit("objects nested too deeply"));
        // An array claiming four billion elements in a few bytes.
        let huge = synth::stream(&[(tag(b"Big_"), F::Raw(vec![0x81, b'_', b'g', b'i', b'B', 0xff, 0xff, 0xff, 0xff, 0]))]);
        assert_eq!(stream::parse(&huge).unwrap_err(), Error::Malformed("array longer than its data"));
        let forward = synth::stream(&[(tag(b"Link"), F::Ref(99))]);
        assert!(stream::parse(&forward).is_err());
        let twice = synth::stream(&[(tag(b"A___"), F::Def(1, vec![tag(b"RGBA")], vec![])), (tag(b"B___"), F::Def(1, vec![tag(b"RGBA")], vec![]))]);
        assert!(stream::parse(&twice).is_err());
        let unknown = synth::stream(&[(tag(b"X___"), F::Raw(vec![0x0b, 1, 2, 3, 4]))]);
        assert!(matches!(stream::parse(&unknown), Err(Error::Unsupported(_))));
        let d = doc();
        for end in 0..d.len() {
            assert!(stream::parse(&d[..end]).is_err(), "{end}");
        }
    }

    #[test]
    fn fixed_records_of_zero_width_are_rejected_before_any_value_is_made() {
        // Four million records of no bytes each pass a length check that multiplies by the width.
        let mut f = vec![0x80 | 0x2c];
        f.extend(tag(b"Big_").0.to_le_bytes());
        f.extend(4_000_000u32.to_le_bytes());
        f.extend(0u16.to_le_bytes());
        let d = synth::stream(&[(tag(b"Big_"), F::Raw(f))]);
        assert_eq!(stream::parse(&d).unwrap_err(), Error::Malformed("array longer than its data"));
    }

    #[test]
    fn a_small_compressed_entry_cannot_make_the_stream_allocate_gigabytes() {
        // Each decoded value takes about 40 bytes, and a bool array packs eight in a byte: 16 MiB
        // of zero bits, a few KiB once compressed, would ask for 2^27 values (about 4 GiB). An
        // array of one-byte values just past the budget fails the same way, before allocating.
        let field = |code: u8, n: u32, data: usize| {
            let mut f = vec![0x80 | code];
            f.extend(tag(b"Big_").0.to_le_bytes());
            f.extend(n.to_le_bytes());
            f.resize(f.len() + data, 0);
            F::Raw(f)
        };
        let bools = 1u32 << 27;
        let bytes = u32::try_from(stream::MAX_VALUES).unwrap() + 1;
        for f in [field(0x29, bools, (bools / 8) as usize), field(0x01, bytes, bytes as usize)] {
            let d = synth::stream(&[(tag(b"Big_"), f)]);
            let file = synth::container(&[("doc.dat", &d, Method::Zstd)], None);
            assert!(file.len() < d.len() / 100, "{} of {}", file.len(), d.len());
            let entry = Archive::open(&file, Limits::default()).unwrap().read("doc.dat").unwrap();
            assert_eq!(stream::parse(&entry).unwrap_err(), Error::Limit("too many values in the document stream"));
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn mutated_containers_and_streams_never_panic(edits in prop::collection::vec((0usize..4096, any::<u8>()), 1..24)) {
            let d = doc();
            for method in [Method::Stored, Method::Zlib, Method::Zstd] {
                let mut file = synth::container(&[("doc.dat", &d, method)], None);
                for (i, v) in &edits {
                    let n = file.len();
                    file[i % n] = *v;
                }
                if let Ok(bytes) = Archive::open(&file, Limits::default()).and_then(|mut a| a.read("doc.dat")) {
                    let _ = stream::parse(&bytes);
                }
            }
            let mut s = d.clone();
            for (i, v) in &edits {
                let n = s.len();
                s[i % n] = *v;
            }
            let _ = stream::parse(&s);
        }
    }
}

mod shapes {
    use crate::container::Limits;
    use crate::model::{Document, Kind, Path, Point, SubPath};
    use crate::stream::Tag;
    use crate::synth::{self, F, Method, tag};

    fn shape(id: u32, class: &[u8; 4], parameters: Vec<(Tag, F)>) -> F {
        F::Def(
            id,
            vec![tag(b"ShpN")],
            vec![(tag(b"ShpB"), F::F64s(vec![0.0, 0.0, 10.0, 20.0])), (tag(b"Shpe"), F::Def(id + 10_000, vec![tag(class)], parameters))],
        )
    }

    /// Reads a document whose one page holds the object `top` from `definitions`, on a thread of
    /// its own, and fails the test when reading takes more than 20 s.
    fn read(definitions: Vec<F>, top: u32) -> Document {
        let data = synth::stream(&[
            (tag(b"Defs"), F::Shared(definitions)),
            (
                tag(b"DocR"),
                F::Def(
                    1,
                    vec![tag(b"DocN")],
                    vec![(
                        tag(b"Chld"),
                        F::Shared(vec![F::Def(
                            2,
                            vec![tag(b"Sprd")],
                            vec![(tag(b"SprB"), F::F64s(vec![0.0, 0.0, 400.0, 300.0])), (tag(b"Chld"), F::Shared(vec![F::Ref(top)]))],
                        )]),
                    )],
                ),
            ),
        ]);
        let bytes = synth::container(&[("doc.dat", &data, Method::Zlib)], None);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(crate::model::read(&bytes, Limits::default())).ok());
        rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap_or_else(|_| panic!("reading the document did not finish")).unwrap()
    }

    fn path(document: &Document) -> &Path {
        match &document.spreads[0].nodes[0].kind {
            Kind::Shape { path, .. } => path,
            other => panic!("expected shape, got {other:?}"),
        }
    }

    #[test]
    fn artistic_text_reads_its_characters_runs_and_anchor() {
        let glyphs = F::Obj(tag(b"Glyp"), vec![(tag(b"Utf8"), F::Str("Hi".into()))]);
        let run = F::Obj(tag(b"Item"), vec![]);
        let attributes = F::Obj(tag(b"GAtt"), vec![(tag(b"Runs"), F::Objs(tag(b"Runs"), vec![vec![(tag(b"Indx"), F::I32(2)), (tag(b"Item"), run)]]))]);
        let block = vec![(tag(b"Glyp"), glyphs), (tag(b"GAtt"), attributes)];
        let text = F::Def(
            100,
            vec![tag(b"TxtA")],
            vec![
                (tag(b"TxtH"), F::Obj(tag(b"ArFr"), vec![(tag(b"FrmB"), F::F64s(vec![0.0, 0.0, 100.0, 50.0]))])),
                (tag(b"StSt"), F::Obj(tag(b"StSt"), vec![(tag(b"Blok"), F::Objs(tag(b"Blok"), vec![block]))])),
            ],
        );
        let document = read(vec![text], 100);
        let Kind::Text(t) = &document.spreads[0].nodes[0].kind else { panic!("expected text") };
        assert_eq!(t.runs.len(), 1);
        assert_eq!(t.runs[0].text, "Hi");
        assert_eq!(t.align, crate::model::Align::Left);
        assert_eq!(t.anchor, Point { x: 0.0, y: 0.0 });
        assert!(t.frame.is_none());
    }

    #[test]
    fn pie_angles_with_huge_finite_values_do_not_loop_or_emit_nonfinite_points() {
        let document = read(vec![shape(100, b"ShPi", vec![(tag(b"AngS"), F::F64(1.0e308)), (tag(b"AngE"), F::F64(-1.0e308))])], 100);
        for subpath in &path(&document).subpaths {
            assert!(subpath.start.x.is_finite() && subpath.start.y.is_finite());
            assert!(subpath.segments.len() <= 20);
            assert!(subpath.segments.iter().flatten().all(|point| point.x.is_finite() && point.y.is_finite()));
        }
    }

    #[test]
    fn wrapped_pie_angles_keep_the_same_geometry_as_the_positive_span() {
        let make = |end| read(vec![shape(100, b"ShPi", vec![(tag(b"AngS"), F::F64(0.0)), (tag(b"AngE"), F::F64(end))])], 100);
        assert_eq!(path(&make(-std::f64::consts::FRAC_PI_2)), path(&make(3.0 * std::f64::consts::FRAC_PI_2)));
    }

    /// A 10×20 rectangle (`ShNR`) with the given parameters, read back as its outline.
    fn rectangle(parameters: Vec<(Tag, F)>) -> (SubPath, Vec<String>) {
        let node =
            F::Def(3, vec![tag(b"ShpN")], vec![(tag(b"ShpB"), F::F64s(vec![0.0, 0.0, 10.0, 20.0])), (tag(b"Shpe"), F::Def(4, vec![tag(b"ShNR")], parameters))]);
        let spread = F::Def(2, vec![tag(b"Sprd")], vec![(tag(b"SprB"), F::F64s(vec![0.0, 0.0, 400.0, 300.0])), (tag(b"Chld"), F::Shared(vec![node]))]);
        let data = synth::stream(&[(tag(b"DocR"), F::Def(1, vec![tag(b"DocN")], vec![(tag(b"Chld"), F::Shared(vec![spread]))]))]);
        let doc = crate::read(&synth::container(&[("doc.dat", &data, Method::Zlib)], None), Limits::default()).unwrap();
        let Kind::Shape { path, .. } = &doc.spreads[0].nodes[0].kind else { panic!("expected a shape") };
        (path.subpaths[0].clone(), doc.warnings)
    }

    /// `CTyp`: an array of corner-type enums (top left, top right, bottom right, bottom left).
    fn corner_types(ids: [u16; 4]) -> F {
        let mut raw = vec![0xaa];
        raw.extend(tag(b"CTyp").0.to_le_bytes());
        raw.extend(4u32.to_le_bytes());
        raw.extend(0u16.to_le_bytes());
        raw.extend(ids.iter().flat_map(|id| id.to_le_bytes()));
        F::Raw(raw)
    }

    #[test]
    fn unlocked_rectangle_corners_keep_their_own_radius_and_type() {
        let p = |x: f64, y: f64| Point { x, y };
        // Relative radii scale by 10. Round 5, round 1, straight 3, none.
        let corners = |lock: Option<bool>| {
            let mut fields = vec![(tag(b"ShCR"), F::F64s(vec![0.5, 0.1, 0.3, 0.0])), (tag(b"CTyp"), corner_types([0, 0, 1, 4]))];
            fields.extend(lock.map(|l| (tag(b"Lock"), F::Bool(l))));
            rectangle(fields)
        };

        let (sub, warnings) = corners(Some(false));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(sub.start, p(5.0, 0.0));
        let ends: Vec<Point> = sub.segments.iter().map(|s| s[2]).collect();
        assert_eq!(ends, [p(9.0, 0.0), p(10.0, 1.0), p(10.0, 17.0), p(7.0, 20.0), p(0.0, 20.0), p(0.0, 5.0), p(5.0, 0.0)]);
        assert_eq!(sub.segments[3], [p(7.0, 20.0); 3], "the straight corner is a line");
        assert_ne!(sub.segments[1][0], p(9.0, 0.0), "the round corner is a curve");

        // Locked (the default): every corner follows the top left one.
        let (sub, _) = corners(None);
        assert_eq!(sub.segments.len(), 8);
        assert_eq!(sub.segments[3][2], p(5.0, 20.0));
    }
}

/// Aggregate materialized-pixel admission (#4960). The entry limits bound what the archive
/// extracts; these cover what the decoded layers and their working planes cost, which is declared
/// by each bitmap's `BmpW`/`BmpH` and needs no tile data at all.
mod pixels {
    use crate::Error;
    use crate::container::Limits;
    use crate::model::{Document, Kind};
    use crate::synth::{self, F, Method, tag};

    /// A `Rstr` pixel layer whose bitmap declares `w`×`h` pixels and holds no tiles.
    fn layer(id: u32, w: i32, h: i32) -> F {
        F::Def(
            id,
            vec![tag(b"Rstr")],
            vec![
                (tag(b"Desc"), F::Str(format!("Layer {id}"))),
                (tag(b"Bitm"), F::Def(id + 1, vec![tag(b"DyBm")], vec![(tag(b"BmpW"), F::I32(w)), (tag(b"BmpH"), F::I32(h)), (tag(b"Frmt"), F::Enum(0, 0))])),
            ],
        )
    }

    /// A document whose one page holds a pixel layer per `sizes` entry.
    fn document(sizes: &[(i32, i32)]) -> Vec<u8> {
        let children: Vec<F> = sizes.iter().enumerate().map(|(i, &(w, h))| layer(10 + 2 * i as u32, w, h)).collect();
        let data = synth::stream(&[(
            tag(b"DocR"),
            F::Def(
                1,
                vec![tag(b"DocN")],
                vec![(
                    tag(b"Chld"),
                    F::Shared(vec![F::Def(
                        2,
                        vec![tag(b"Sprd")],
                        vec![(tag(b"SprB"), F::F64s(vec![0.0, 0.0, 256.0, 256.0])), (tag(b"Chld"), F::Shared(children))],
                    )]),
                )],
            ),
        )]);
        synth::container(&[("doc.dat", &data, Method::Zlib)], None)
    }

    fn imported_layers(d: &Document) -> usize {
        d.spreads[0].nodes.iter().filter(|n| matches!(n.kind, Kind::Image(_))).count()
    }

    /// A 64×64 layer costs 16 KiB of RGBA and 16 KiB of planes, so 40 KiB admits one and not two.
    const ONE_LAYER: usize = 40 << 10;

    #[test]
    fn a_pixel_layer_within_the_budget_imports() {
        let limits = Limits { max_pixel_bytes: ONE_LAYER, ..Limits::default() };
        let d = crate::model::read(&document(&[(64, 64)]), limits).unwrap();
        assert_eq!(imported_layers(&d), 1, "{:?}", d.warnings);
    }

    #[test]
    fn layers_past_the_pixel_budget_are_rejected_before_they_are_allocated() {
        let limits = Limits { max_pixel_bytes: ONE_LAYER, ..Limits::default() };
        // The document is a few hundred bytes: the sizes are in the headers, not the entries.
        let bytes = document(&[(64, 64), (64, 64)]);
        assert!(bytes.len() < 4096, "the fixture is {} bytes", bytes.len());
        assert!(matches!(crate::model::read(&bytes, limits), Err(Error::Limit(_))), "the second layer materialized");
        // Two layers fit once the budget covers their pixels together.
        let room = Limits { max_pixel_bytes: ONE_LAYER + (32 << 10), ..Limits::default() };
        assert_eq!(imported_layers(&crate::model::read(&bytes, room).unwrap()), 2);
    }

    #[test]
    fn a_declared_size_larger_than_the_budget_is_rejected_on_its_own() {
        // 2048×2048 is 16 MiB of RGBA and 16 MiB of planes, past a 1 MiB budget.
        let limits = Limits { max_pixel_bytes: 1 << 20, ..Limits::default() };
        assert!(
            matches!(crate::model::read(&document(&[(2048, 2048)]), limits), Err(Error::Limit(_))),
            "16 MiB of declared pixels materialized under a 1 MiB budget"
        );
    }
}

/// Spot colours that link to themselves or to each other. Each document is read on a 1 MiB
/// stack, the smallest main-thread stack PhotoCraft runs on (Windows), first in a child process
/// that must finish within 20 s. A stack overflow aborts the process it happens in, so unbounded
/// recursion fails the one test that caused it instead of taking the other tests in the binary
/// down with it.
mod spot_colours {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use crate::container::Limits;
    use crate::model::{Document, Kind};
    use crate::paint::{Color, Paint};
    use crate::stream::Tag;
    use crate::synth::{self, F, Method, tag};

    /// A one-page document whose page holds `top`, read in a child process first and then here.
    fn read(top: Vec<F>) -> Document {
        let spread = F::Def(2, vec![tag(b"Sprd")], vec![(tag(b"SprB"), F::F64s(vec![0.0, 0.0, 400.0, 300.0])), (tag(b"Chld"), F::Shared(top))]);
        let data = synth::stream(&[(tag(b"DocR"), F::Def(1, vec![tag(b"DocN")], vec![(tag(b"Chld"), F::Shared(vec![spread]))]))]);
        let bytes = synth::container(&[("doc.dat", &data, Method::Zlib)], None);
        let test = std::thread::current().name().unwrap_or("reader").to_owned();
        read_in_child(&bytes, &test);
        read_on_small_stack(bytes, &test).unwrap()
    }

    /// Reads on a 1 MiB thread named after the test, so a stack overflow message names the test.
    fn read_on_small_stack(bytes: Vec<u8>, test: &str) -> Result<Document, crate::Error> {
        let reader = std::thread::Builder::new().name(test.to_owned()).stack_size(1 << 20).spawn(move || crate::read(&bytes, Limits::default())).unwrap();
        reader.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }

    /// Set in the child process to the name of the test it reads for.
    const CHILD: &str = "PHOTOCRAFT_AFFINITY_READ_CHILD";

    /// Runs `read_child` in a new process of this test binary on `bytes` and fails the test if
    /// that process crashes, panics or takes over 20 s, or never read the document.
    fn read_in_child(bytes: &[u8], test: &str) {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["tests::spot_colours::read_child", "--exact", "--ignored", "--nocapture", "--test-threads=1"])
            .env(CHILD, test)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let drain = |mut pipe: Box<dyn Read + Send>| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = pipe.read_to_string(&mut text);
                text
            })
        };
        let stdout = drain(Box::new(child.stdout.take().unwrap()));
        let stderr = drain(Box::new(child.stderr.take().unwrap()));
        // Fed from a thread, so a child that never reads cannot block the deadline below; a child
        // that dies first breaks the pipe and is reported by its exit status.
        let (mut stdin, input) = (child.stdin.take().unwrap(), bytes.to_vec());
        let feed = std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        feed.join().unwrap();
        let output = format!("{}{}", stdout.join().unwrap(), stderr.join().unwrap());
        match status {
            None => panic!("reading the document did not finish within 20 s:\n{output}"),
            Some(status) if !status.success() => panic!("reading the document failed in a child process ({status}):\n{output}"),
            // A filter that matched no test would also exit successfully, having read nothing.
            Some(_) => assert!(output.contains(&format!("read {} bytes", bytes.len())), "the child process did not read the document:\n{output}"),
        }
    }

    /// Not a test of its own: `read_in_child` runs it in a child process to read the document on
    /// its stdin. Without the variable, as when run with `--ignored`, it does nothing.
    #[test]
    #[ignore = "run by read_in_child in a child process"]
    fn read_child() {
        let Some(test) = std::env::var_os(CHILD) else { return };
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes).unwrap();
        let len = bytes.len();
        let _ = read_on_small_stack(bytes, &test.to_string_lossy());
        println!("read {len} bytes");
    }

    /// An ellipse filled with `colour`.
    fn filled(id: u32, colour: F) -> F {
        F::Def(
            id,
            vec![tag(b"ShpN")],
            vec![
                (tag(b"ShpB"), F::F64s(vec![0.0, 0.0, 1.0, 1.0])),
                (tag(b"Shpe"), F::Def(id + 1_000_000, vec![tag(b"ShpE")], vec![])),
                (tag(b"BFFl"), F::Shared(vec![F::Def(id + 2_000_000, vec![tag(b"FilS")], vec![(tag(b"Colr"), colour)])])),
            ],
        )
    }

    fn spot(id: u32, fields: Vec<(Tag, F)>) -> F {
        F::Def(id, vec![tag(b"Pant")], fields)
    }

    fn fills(doc: &Document) -> &[Paint] {
        match &doc.spreads[0].nodes[0].kind {
            Kind::Shape { fills, .. } => fills,
            other => panic!("expected a shape, got {other:?}"),
        }
    }

    fn warned(doc: &Document, what: &str) -> bool {
        doc.warnings.iter().any(|w| w.contains(what))
    }

    #[test]
    fn a_spot_colour_that_is_its_own_rgb_equivalent_is_refused() {
        let doc = read(vec![filled(10, spot(20, vec![(tag(b"srgb"), F::Ref(20))]))]);
        assert!(fills(&doc).is_empty(), "{:?}", fills(&doc));
        assert!(warned(&doc, "spot colours nested deeper than 4 levels"), "{:?}", doc.warnings);
    }

    #[test]
    fn spot_colours_that_are_each_others_base_and_rgb_equivalent_are_refused() {
        let doc = read(vec![filled(10, spot(20, vec![(tag(b"base"), spot(21, vec![(tag(b"srgb"), F::Ref(20))]))]))]);
        assert!(fills(&doc).is_empty(), "{:?}", fills(&doc));
        assert!(warned(&doc, "spot colours nested deeper than 4 levels"), "{:?}", doc.warnings);
    }

    #[test]
    fn spot_colours_of_spot_colours_do_not_take_exponential_time() {
        // Each level's RGB equivalent and base are the next level, and the last is in an unknown
        // model, so both are followed everywhere: 2^41 colour reads before the fix.
        fn level(id: u32, left: u32) -> F {
            if left == 0 {
                return F::Def(id, vec![tag(b"XXXX")], vec![]);
            }
            spot(id, vec![(tag(b"srgb"), level(id + 1, left - 1)), (tag(b"base"), F::Ref(id + 1))])
        }
        let doc = read(vec![filled(10, level(100, 40))]);
        assert!(fills(&doc).is_empty(), "{:?}", fills(&doc));
    }

    /// `spots` spot colours, each defined through the next, around an opaque RGB colour.
    fn spots(id: u32, spots: u32) -> F {
        if spots == 0 {
            let rgba: Vec<u8> = [1.0f32, 0.5, 0.25, 1.0].iter().flat_map(|c| c.to_le_bytes()).collect();
            return F::Def(id, vec![tag(b"RGBA")], vec![(tag(b"_col"), F::Struct(rgba))]);
        }
        spot(id, vec![(tag(b"srgb"), self::spots(id + 1, spots - 1))])
    }

    #[test]
    fn spot_colours_nest_four_levels_and_no_deeper() {
        let doc = read(vec![filled(10, spots(100, 4))]);
        assert_eq!(fills(&doc), [Paint::Solid(Color::Rgb { r: 1.0, g: 0.5, b: 0.25, a: 1.0 })]);
        let doc = read(vec![filled(10, spots(100, 5))]);
        assert!(fills(&doc).is_empty(), "{:?}", fills(&doc));
        assert!(warned(&doc, "spot colours nested deeper than 4 levels"), "{:?}", doc.warnings);
    }
}
