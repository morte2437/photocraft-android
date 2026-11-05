//! Tests for the headless GPU backend, capability isolation, cache mutation, and CPU/GPU parity.

#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
mod tests {
    use std::fs;

    use photocraft_codecs::decode;
    use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
    use photocraft_doc::{Document, Layer, LayerContent};
    use photocraft_geom::{Rect, Size};
    use photocraft_io::BandRenderer;
    use photocraft_raster::Surface;

    use photocraft_automation::workspace::AuthorizedWorkspace;
    use photocraft_automation::{GpuRenderer, Headless};

    fn make_test_doc(w: u32, h: u32, depth: SampleType) -> Document {
        let mut doc = Document::with_background("test_doc", Size::new(w, h), ColorMode::Rgb, depth, Color::WHITE);
        let mut s = Surface::new(doc.pixel_format());
        s.fill_rect(Rect::new(10, 10, w as i32 - 10, h as i32 - 10), &[0.2, 0.5, 0.8, 0.8]);
        let mut top = Layer::new("top", LayerContent::Raster(s));
        top.opacity = 0.75;
        top.blend = BlendMode::Multiply;
        doc.layers.push(top);
        doc
    }

    #[test]
    fn headless_with_gpu_renders_png_full_and_scaled() {
        let mut h = Headless::trusted_local().with_gpu();
        let doc = make_test_doc(120, 80, SampleType::U8);
        h.session.open_document(doc, None);

        // Full size render (max_side = 0)
        let full_png = h.render_png(None, 0).expect("full render_png");
        let full_img = decode(&full_png).expect("decode full");
        assert_eq!((full_img.width(), full_img.height()), (120, 80));
        let diagnostics = h.drain_diagnostics();

        // Scaled preview (max_side = 40)
        let thumb_png = h.render_png(None, 40).expect("thumbnail render_png");
        let thumb_img = decode(&thumb_png).expect("decode thumbnail");
        assert_eq!((thumb_img.width(), thumb_img.height()), (40, 27));

        // Skip when this host has no usable GPU: then the full-size render legitimately used
        // the CPU path instead, and there is no GPU result to check.
        if diagnostics.iter().any(|d| d.contains("no GPU adapter available") || d.contains("could not build GPU compositor")) {
            return;
        }
        assert!(diagnostics.iter().any(|d| d.contains("GPU adapter:")), "GPU was not initialized: {diagnostics:?}");
        assert!(!diagnostics.iter().any(|d| d.contains("custom renderer unavailable")), "full-size render fell back to CPU: {diagnostics:?}");
    }

    #[test]
    fn capability_scoped_writes_preserved_with_gpu() {
        let dir = std::env::temp_dir().join(format!("photocraft-cap-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let workspace = AuthorizedWorkspace::new(Some(&dir), Some(&dir)).expect("authorized workspace");

        let mut h = Headless::with_workspace(workspace).with_gpu();
        let doc = make_test_doc(64, 64, SampleType::U8);
        h.session.open_document(doc, None);

        let valid_target = dir.join("out.png");
        let res = h.save(None, Some(&valid_target), Some("png"), &Default::default());
        assert!(res.is_ok(), "authorized write must succeed: {:?}", res);
        assert!(valid_target.exists());

        // Attempt write outside authorized workspace
        let invalid_target = std::env::temp_dir().join("outside_unauthorized.png");
        let denied = h.save(None, Some(&invalid_target), Some("png"), &Default::default());
        assert!(denied.is_err(), "write outside authorized workspace must be denied");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_invalidation_across_mutations_and_distinct_docs() {
        let mut h = Headless::trusted_local().with_gpu();

        // 1. Render Doc 1
        let doc1 = make_test_doc(64, 64, SampleType::U8);
        h.session.open_document(doc1, None);
        let png1_initial = h.render_png(Some(0), 0).expect("initial render");
        let diagnostics = h.drain_diagnostics();
        if diagnostics.iter().any(|d| d.contains("no GPU adapter available") || d.contains("could not build GPU compositor")) {
            return;
        }
        assert!(diagnostics.iter().any(|d| d.contains("GPU adapter:")), "GPU was not initialized: {diagnostics:?}");
        assert!(!diagnostics.iter().any(|d| d.contains("custom renderer unavailable")), "initial render fell back to CPU: {diagnostics:?}");
        let img1_initial = decode(&png1_initial).expect("decode initial");

        // 2. Mutate Doc 1 in place (change top layer opacity to 0.1)
        h.session
            .edit("Change layer opacity", |doc, _| {
                doc.layers[1].opacity = 0.1;
                Ok(())
            })
            .expect("edit opacity");
        let png1_mutated = h.render_png(Some(0), 0).expect("mutated render");
        let mutated_diagnostics = h.drain_diagnostics();
        assert!(!mutated_diagnostics.iter().any(|d| d.contains("custom renderer unavailable")), "mutated render fell back to CPU: {mutated_diagnostics:?}");
        let img1_mutated = decode(&png1_mutated).expect("decode mutated");

        assert_ne!(img1_initial.data(), img1_mutated.data(), "mutating document must invalidate cache and produce fresh pixels");

        // 3. Open Doc 2 with distinct contents
        let mut doc2 = Document::with_background("doc2", Size::new(64, 64), ColorMode::Rgb, SampleType::U8, Color::BLACK);
        let mut s2 = Surface::new(doc2.pixel_format());
        s2.fill_rect(Rect::new(0, 0, 64, 64), &[1.0, 0.0, 0.0, 1.0]);
        doc2.layers.push(Layer::new("red", LayerContent::Raster(s2)));
        h.session.open_document(doc2, None);

        let png2 = h.render_png(Some(1), 0).expect("doc2 render");
        let doc2_diagnostics = h.drain_diagnostics();
        assert!(!doc2_diagnostics.iter().any(|d| d.contains("custom renderer unavailable")), "second-document render fell back to CPU: {doc2_diagnostics:?}");
        let img2 = decode(&png2).expect("decode doc2");

        assert_ne!(img2.data(), img1_mutated.data(), "distinct document must not return stale pixels from previous doc");
    }

    #[test]
    fn cpu_gpu_direct_parity_and_matrix_verification() {
        let mut gpu_renderer = match GpuRenderer::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SKIPPED: GpuRenderer unavailable ({e}); no GPU adapter present");
                return;
            }
        };

        let adapter = gpu_renderer.adapter_name().unwrap_or("unknown");
        let is_software = gpu_renderer.is_software();
        println!("GPU parity verified using adapter '{adapter}' (software: {is_software})");

        // Matrix: 8-bit output x Alpha modes (opaque background, translucent document).
        // U16 and F32 are deliberately excluded: band readback uploads half-precision textures,
        // so both must reject and let the export fall back to the CPU (checked below).
        for with_alpha in [false, true] {
            let doc = if with_alpha {
                let mut d = Document::new("translucent", Size::new(96, 96), ColorMode::Rgb, SampleType::U8);
                let mut s = Surface::new(d.pixel_format());
                s.fill_rect(Rect::new(10, 10, 86, 86), &[0.3, 0.6, 0.9, 0.5]);
                d.layers.push(Layer::new("trans", LayerContent::Raster(s)));
                d
            } else {
                make_test_doc(96, 96, SampleType::U8)
            };

            // Independent reference CPU compositor oracle
            let mut cpu_pixels = Vec::new();
            photocraft_compose::render_bands(&doc, doc.bounds(), 16, |buf| {
                cpu_pixels.extend_from_slice(&buf.px);
                Ok::<(), std::convert::Infallible>(())
            })
            .expect("CPU reference compose failed");

            // Direct GPU renderer invocation (MUST NOT fall back to CPU)
            let mut gpu_pixels = Vec::new();
            gpu_renderer
                .render_bands(&doc, doc.bounds(), 16, &photocraft_raster::Interrupt::NONE, &mut |band| {
                    gpu_pixels.extend_from_slice(&band.pixels);
                    Ok::<(), photocraft_io::IoError>(())
                })
                .expect("direct GPU render_bands failed");

            assert_eq!(cpu_pixels.len(), gpu_pixels.len());
            let tol = 1.0 / 255.0;
            for (i, (c, g)) in cpu_pixels.iter().zip(&gpu_pixels).enumerate() {
                for ch in 0..4 {
                    assert!((c[ch] - g[ch]).abs() <= tol, "alpha {} pixel {} ch {} differs: cpu={}, gpu={}", with_alpha, i, ch, c[ch], g[ch]);
                }
            }
        }

        // U16 and F32 need the CPU compositor: band readback would quantise them to half.
        for depth in [SampleType::U16, SampleType::F32] {
            let doc = make_test_doc(64, 64, depth);
            let res = gpu_renderer.render_bands(&doc, doc.bounds(), 16, &photocraft_raster::Interrupt::NONE, &mut |_| Ok::<(), photocraft_io::IoError>(()));
            assert!(
                matches!(res, Err(photocraft_io::RenderError::Unsupported(_))),
                "{depth:?} documents must be rejected so the export falls back to CPU: {res:?}"
            );
        }
    }

    #[test]
    fn high_depth_exports_fall_back_to_the_cpu_compositor() {
        let mut gpu_renderer = match GpuRenderer::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SKIPPED: GpuRenderer unavailable ({e}); no GPU adapter present");
                return;
            }
        };

        let opts = photocraft_io::ExportOptions::default();
        for depth in [SampleType::U16, SampleType::F32] {
            let doc = make_test_doc(64, 64, depth);
            let expected = photocraft_io::export_flat(&doc, photocraft_codecs::Format::Png, &opts).expect("CPU export");
            let res = photocraft_io::export_flat_with_renderer(
                &doc,
                photocraft_codecs::Format::Png,
                &opts,
                &photocraft_raster::Interrupt::NONE,
                Some(&mut gpu_renderer),
            )
            .expect("the export must retry on the CPU");
            assert_eq!(expected.bytes, res.bytes, "{depth:?} exports must keep the CPU compositor's precision");
            assert!(res.warnings.iter().any(|w| w.contains("falling back to CPU")), "{depth:?} export must report its CPU fallback: {:?}", res.warnings);
        }
    }

    #[test]
    fn gpu_renderer_diagnostics_and_bounded_storage() {
        let mut r = match GpuRenderer::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SKIPPED: GpuRenderer unavailable ({e}); no GPU adapter present");
                return;
            }
        };

        let diags = r.drain_diagnostics();
        assert!(!diags.is_empty(), "adapter status must be reported once in diagnostics");
        assert!(diags[0].contains("GPU adapter:"), "unexpected diagnostic: {:?}", diags);

        // After draining, second drain should be empty
        assert!(r.drain_diagnostics().is_empty());

        // Push diagnostics up to limit and verify bounded dedup storage
        for i in 0..30 {
            r.push_diagnostic(format!("test diagnostic {i}"));
        }
        let drained = r.drain_diagnostics();
        assert!(drained.len() <= 16, "diagnostics must be capped to bounded limit, got {}", drained.len());
    }

    #[test]
    fn gpu_renderer_preserves_sink_errors_and_cancellation() {
        let mut gpu_renderer = match GpuRenderer::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SKIPPED: GpuRenderer unavailable ({e}); no GPU adapter present");
                return;
            }
        };

        let doc = make_test_doc(64, 64, SampleType::U8);

        // 1. Cancellation preservation
        let cancel_res =
            gpu_renderer.render_bands(&doc, doc.bounds(), 16, &photocraft_raster::Interrupt::NONE, &mut |_| Err(photocraft_io::IoError::Cancelled));
        assert!(matches!(cancel_res, Err(photocraft_io::RenderError::Cancelled)), "cancellation must be preserved: {cancel_res:?}");

        // 2. Sink I/O error preservation
        let sink_res = gpu_renderer.render_bands(&doc, doc.bounds(), 16, &photocraft_raster::Interrupt::NONE, &mut |_| {
            Err(photocraft_io::IoError::Read("simulated sink error".into()))
        });
        assert!(matches!(sink_res, Err(photocraft_io::RenderError::Sink(_))), "sink error must be preserved as RenderError::Sink: {sink_res:?}");
    }

    #[test]
    fn gpu_renderer_bounded_readback_staging() {
        let mut gpu_renderer = match GpuRenderer::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SKIPPED: GpuRenderer unavailable ({e}); no GPU adapter present");
                return;
            }
        };

        let doc = make_test_doc(128, 128, SampleType::U8);
        gpu_renderer.set_max_staging_bytes(32 * 1024);

        let mut received_rows = 0;
        let res = gpu_renderer.render_bands(&doc, doc.bounds(), 0, &photocraft_raster::Interrupt::NONE, &mut |band| {
            received_rows += band.rect.height();
            Ok(())
        });
        assert!(res.is_ok(), "bounded staging readback must succeed: {res:?}");
        assert_eq!(received_rows, 128);
    }
}
