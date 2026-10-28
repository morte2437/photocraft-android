use super::assets::{asset_output_path, asset_path_components};
use super::optimizer::{optimize, web_document, web_exif, web_warnings, web_xmp};
use super::settings::{WebSettings, rgb_hex};
use super::{EngineError, Rect, ResolvedSlice, Result, Session, SliceKind, Value, bad, join, json, other, portable_device_name, slices, stem, write_file};

/// A file-name-safe slice name.
fn slice_file(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' }).collect();
    let s = if s.is_empty() { "slice".into() } else { s };
    portable_generated_stem(s)
}

/// Generated filenames should remain usable on Windows even when a document or slice is named
/// after a reserved device. Caller-supplied path components are still rejected below.
fn portable_generated_stem(stem: String) -> String {
    portable_device_name(stem)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Imported slice URLs are document data. Only ordinary web/contact links and relative links
/// become anchors; an unsupported scheme leaves the image unlinked.
fn safe_slice_url(url: &str) -> bool {
    if url.is_empty() || url.trim() != url || url.chars().any(|c| c.is_control() || c == '\\') {
        return false;
    }
    let delimiter = url.find(['/', '?', '#']).unwrap_or(url.len());
    let Some(colon) = url.find(':').filter(|pos| *pos < delimiter) else {
        return true;
    };
    let scheme = url.get(..colon).unwrap_or_default().to_ascii_lowercase();
    let rest = url.get(colon + 1..).unwrap_or_default();
    match scheme.as_str() {
        "http" | "https" | "ftp" | "ftps" => rest.strip_prefix("//").is_some_and(|host| !host.is_empty() && !host.starts_with('/')),
        "mailto" | "tel" => !rest.is_empty(),
        _ => false,
    }
}

/// Keep the image folder relative to the selected export directory on every platform.
fn web_images_folder(images: &str, cmd: &str) -> Result<String> {
    if images.is_empty() {
        return Ok(String::new());
    }
    let trimmed = images.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return Err(bad(cmd, "imagesFolder must be a relative folder name"));
    }
    let parts = asset_path_components(trimmed).map_err(|e| bad(cmd, format!("imagesFolder: {e}")))?;
    Ok(parts.join("/"))
}

fn web_output_path(dir: &str, relative: &str) -> Result<String> {
    let root = if dir.is_empty() { "." } else { dir };
    asset_output_path(root, relative).map(|(actual, _)| actual)
}

fn web_derived_path(dir: &str, relative: &str, native_paths: bool) -> Result<String> {
    if native_paths {
        web_output_path(dir, relative)
    } else {
        // Browser downloads use logical names, without a host filesystem to inspect.
        asset_path_components(relative)?;
        Ok(join(dir, relative))
    }
}

fn scale_rect(r: Rect, sx: f64, sy: f64) -> Rect {
    Rect::new(
        (f64::from(r.x0) * sx).round() as i32,
        (f64::from(r.y0) * sy).round() as i32,
        (f64::from(r.x1) * sx).round() as i32,
        (f64::from(r.y1) * sy).round() as i32,
    )
}

/// The HTML table for `cells` (rect, td body) over a `w`×`h` image: one column per distinct x
/// edge, one row per y edge, colspan/rowspan per slice, and a spacer row fixing column widths.
fn html_table(title: &str, w: u32, h: u32, cells: &[(Rect, String, String)], spacer: Option<&str>) -> String {
    let mut xs: Vec<i32> = cells.iter().flat_map(|(r, _, _)| [r.x0, r.x1]).collect();
    let mut ys: Vec<i32> = cells.iter().flat_map(|(r, _, _)| [r.y0, r.y1]).collect();
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    let col = |x: i32| xs.iter().position(|v| *v == x).unwrap_or(0);
    let row = |y: i32| ys.iter().position(|v| *v == y).unwrap_or(0);
    let (nc, nr) = (xs.len().saturating_sub(1), ys.len().saturating_sub(1));
    let mut taken = vec![false; nc * nr];
    let mut by_row: Vec<Vec<(usize, String)>> = vec![Vec::new(); nr];
    for (r, attrs, body) in cells {
        let (c0, c1, r0, r1) = (col(r.x0), col(r.x1), row(r.y0), row(r.y1));
        if (r0..r1).any(|j| (c0..c1).any(|i| taken[j * nc + i])) {
            continue; // overlapped (the caller cuts slices into visible pieces, so this can't happen)
        }
        for j in r0..r1 {
            for i in c0..c1 {
                taken[j * nc + i] = true;
            }
        }
        let mut td = String::from("\t\t<td");
        if c1 - c0 > 1 {
            td.push_str(&format!(" colspan=\"{}\"", c1 - c0));
        }
        if r1 - r0 > 1 {
            td.push_str(&format!(" rowspan=\"{}\"", r1 - r0));
        }
        td.push_str(attrs);
        td.push('>');
        td.push_str(body);
        td.push_str("</td>\n");
        by_row[r0].push((c0, td));
    }
    let mut s = format!(
        "<html>\n<head>\n<title>{t}</title>\n<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\">\n</head>\n<body bgcolor=\"#FFFFFF\" leftmargin=\"0\" topmargin=\"0\" marginwidth=\"0\" marginheight=\"0\">\n<!-- Save for Web Slices ({t}) -->\n<table id=\"Table_01\" width=\"{w}\" height=\"{h}\" border=\"0\" cellpadding=\"0\" cellspacing=\"0\">\n",
        t = html_escape(title)
    );
    for (j, mut tds) in by_row.into_iter().enumerate() {
        tds.sort_by_key(|(c, _)| *c);
        s.push_str("\t<tr>\n");
        for (_, td) in tds {
            s.push_str(&td);
        }
        if let Some(sp) = spacer {
            s.push_str(&format!("\t\t<td>\n\t\t\t<img src=\"{}\" width=\"1\" height=\"{}\" alt=\"\"></td>\n", html_escape(sp), ys[j + 1] - ys[j]));
        }
        s.push_str("\t</tr>\n");
    }
    if let Some(sp) = spacer {
        s.push_str("\t<tr>\n");
        for i in 0..nc {
            s.push_str(&format!("\t\t<td>\n\t\t\t<img src=\"{}\" width=\"{}\" height=\"1\" alt=\"\"></td>\n", html_escape(sp), xs[i + 1] - xs[i]));
        }
        s.push_str("\t\t<td></td>\n\t</tr>\n");
    }
    s.push_str("</table>\n<!-- End Save for Web Slices -->\n</body>\n</html>\n");
    s
}

/// File › Export › Save for Web (Legacy).
pub(super) fn save_for_web(s: &mut Session, p: &Value) -> Result<Value> {
    save_for_web_impl(s, p, &mut write_file, true)
}

/// Save for Web through a platform writer (native files or browser downloads).
/// The writer receives every image, spacer and HTML file. A write error stops the export;
/// only a complete export remembers its settings and fires the export event. A custom writer
/// that targets the local filesystem must enforce its own path confinement.
pub fn save_for_web_with_writer(s: &mut Session, p: &Value, write: &mut dyn FnMut(&str, &[u8]) -> Result<()>) -> Result<Value> {
    save_for_web_impl(s, p, write, false)
}

/// Settings to seed the Save for Web dialog from on the next launch: `file_menu` is session
/// state, so a complete export also writes the preferences' remembered dialog settings (like
/// Fill, Liquify and the filter dialogs). The destination is per export, never remembered.
fn remember_web_settings(s: &mut Session, cmd: &str, settings: &Value) {
    let mut m = settings.clone();
    if let Some(o) = m.as_object_mut() {
        for k in ["path", "dir", "trustedSliceHtml"] {
            o.remove(k);
        }
    }
    s.prefs.edit(|p| {
        p.dialogs.insert(cmd.to_string(), m);
    });
}

fn save_for_web_impl(s: &mut Session, p: &Value, write: &mut dyn FnMut(&str, &[u8]) -> Result<()>, native_paths: bool) -> Result<Value> {
    let cmd = "file.export.saveForWebLegacy";
    if let Some(authorize) = s.authorize {
        authorize(cmd, p)?;
    }
    // Direct platform calls must export the moved selection as it appears on the canvas,
    // just as Session::execute does before dispatching the native command.
    crate::float_cmds::before_command(s, cmd)?;
    let st = WebSettings::from_params(p, cmd)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = d.doc.clone();
    let (wdoc, sx, sy) = web_document(&doc, p, &st)?;
    let buf = photocraft_compose::flatten(&wdoc);
    let bw = wdoc.size.width as usize;
    let icc = wdoc.icc_profile.as_deref().map(|v| v.as_slice());
    let xmp = web_xmp(&doc, st.metadata);
    let exif = web_exif(&doc, st.metadata);
    let dpi = wdoc.resolution_dpi;
    let warnings = web_warnings(&st, exif, dpi);
    let all = slices::resolve(&doc);
    let pick = p.get("slices").and_then(Value::as_str).unwrap_or("all");
    let numbers: Option<Vec<u64>> = p.get("numbers").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect());
    let chosen: Vec<&ResolvedSlice> = all
        .iter()
        .filter(|r| match (&numbers, pick) {
            (Some(n), _) => n.contains(&(r.number as u64)),
            (None, "user") => r.origin != photocraft_doc::SliceOrigin::Auto,
            _ => true,
        })
        .collect();
    let single = p.get("path").and_then(Value::as_str).filter(|v| !v.is_empty());
    let dir = p.get("dir").and_then(Value::as_str).filter(|v| !v.is_empty());
    let mut last_web = p.clone();
    // A trusted-markup decision applies to this export only, never to another document opened
    // with the dialog's remembered settings.
    if let Some(last) = last_web.as_object_mut() {
        last.remove("trustedSliceHtml");
    }
    // Estimate only (the dialog's annotations): the whole image, or each chosen slice.
    if single.is_none() && dir.is_none() {
        let o = optimize(&buf.px, bw, wdoc.bounds(), &st, icc, exif, xmp.as_deref(), dpi, false)?;
        return Ok(
            json!({"format": o.ext, "bytes": o.bytes.len(), "width": o.width, "height": o.height, "colors": o.colors, "colorTable": o.color_table.iter().map(|c| rgb_hex(*c)).collect::<Vec<_>>(), "transparentIndex": o.transparent_index}),
        );
    }
    if let Some(path) = single.filter(|_| dir.is_none() && (all.len() == 1 || p.get("slices").is_none())) {
        let o = optimize(&buf.px, bw, wdoc.bounds(), &st, icc, exif, xmp.as_deref(), dpi, false)?;
        write(path, &o.bytes)?;
        remember_web_settings(s, cmd, &last_web);
        s.file_menu.last_web = Some(last_web);
        crate::automate_cmds::fire_event(s, "export");
        return Ok(json!({"files": [path], "bytes": o.bytes.len(), "width": o.width, "height": o.height, "colors": o.colors, "warnings": warnings}));
    }
    let dir =
        dir.map(str::to_string).or_else(|| single.and_then(|f| std::path::Path::new(f).parent().map(|d| d.to_string_lossy().into_owned()))).unwrap_or_default();
    let html = p.get("html").and_then(Value::as_bool).unwrap_or(false);
    let images = web_images_folder(p.get("imagesFolder").and_then(Value::as_str).unwrap_or("images"), cmd)?;
    let trusted_slice_html = p.get("trustedSliceHtml").and_then(Value::as_bool).unwrap_or(false);
    let base = portable_generated_stem(single.map(stem).unwrap_or_else(|| slices::base_name(&doc)));
    let mut files = Vec::new();
    let mut cells: Vec<(Rect, String, String)> = Vec::new();
    let mut total = 0usize;
    // Output names are unique (case-insensitively, for macOS and Windows file systems): two
    // slices named alike, or names that sanitize alike, must not overwrite each other. The HTML
    // page's spacer image is reserved.
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    if html {
        used.insert("spacer.gif".into());
    }
    let mut unique = |stem: &str, ext: &str| {
        let mut name = format!("{stem}.{ext}");
        let mut k = 2u32;
        while !used.insert(name.to_lowercase()) {
            name = format!("{stem}_{k}.{ext}");
            k = k.saturating_add(1);
        }
        name
    };
    // Slices never overlap here: one covered by later slices comes as its visible pieces (see
    // `slices::resolve`), each its own image and cell, as Photoshop's subslices. A no-image cell's
    // text goes into its first piece only.
    let mut texted = std::collections::HashSet::new();
    for r in &chosen {
        let piece = scale_rect(r.rect, sx, sy).intersect(&wdoc.bounds());
        if piece.is_empty() {
            continue;
        }
        let stored = r.id.and_then(|id| doc.slices.get(id));
        if r.kind == SliceKind::NoImage {
            let text = stored
                .filter(|s| texted.insert(s.id))
                .map(|s| if trusted_slice_html && s.cell_text_is_html { s.cell_text.clone() } else { html_escape(&s.cell_text) })
                .unwrap_or_default();
            let bg = stored.and_then(|s| s.background).map(|c| format!(" bgcolor=\"#{:02X}{:02X}{:02X}\"", c[1], c[2], c[3])).unwrap_or_default();
            cells.push((piece, bg, text));
            continue;
        }
        let o = optimize(&buf.px, bw, piece, &st, icc, exif, xmp.as_deref(), dpi, false)?;
        let name = unique(&slice_file(&r.name), o.ext);
        let rel = if images.is_empty() { name.clone() } else { format!("{images}/{name}") };
        let out = join(&dir, &rel);
        write(&web_derived_path(&dir, &rel, native_paths)?, &o.bytes)?;
        total += o.bytes.len();
        files.push(out);
        let alt = stored.map_or(String::new(), |s| html_escape(&s.alt));
        let img = format!("\n\t\t\t<img src=\"{}\" width=\"{}\" height=\"{}\" alt=\"{alt}\">", html_escape(&rel), piece.width(), piece.height());
        let body = match stored.filter(|s| safe_slice_url(&s.url)) {
            Some(s) => {
                let target = if s.target.is_empty() { String::new() } else { format!(" target=\"{}\"", html_escape(&s.target)) };
                format!("\n\t\t\t<a href=\"{}\"{target}>{img}</a>", html_escape(&s.url))
            }
            None => img,
        };
        cells.push((piece, String::new(), body));
    }
    let mut html_path = None;
    if html {
        let grid = cells.len() > 1;
        let spacer = grid.then(|| if images.is_empty() { "spacer.gif".to_string() } else { format!("{images}/spacer.gif") });
        if let Some(sp) = &spacer {
            let gif = photocraft_codecs::web::encode_gif_indexed(1, 1, &[0], &[[0, 0, 0]], Some(0), false).map_err(other)?;
            write(&web_derived_path(&dir, sp, native_paths)?, &gif)?;
        }
        let page = html_table(&base, wdoc.size.width, wdoc.size.height, &cells, spacer.as_deref());
        let hp = join(&dir, &format!("{base}.html"));
        write(&web_derived_path(&dir, &format!("{base}.html"), native_paths)?, page.as_bytes())?;
        html_path = Some(hp);
    }
    remember_web_settings(s, cmd, &last_web);
    s.file_menu.last_web = Some(last_web);
    crate::automate_cmds::fire_event(s, "export");
    Ok(
        json!({"files": files, "html": html_path, "bytes": total, "slices": chosen.len(), "width": wdoc.size.width, "height": wdoc.size.height, "warnings": warnings}),
    )
}
