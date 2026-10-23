//! File › Print…, Print One Copy, File › Package… and File › Export › Paths to Illustrator….
//!
//! Printing renders the flattened document into a PDF page (paper size, orientation, position,
//! scale, colour handling with printer profile / intent / black-point compensation, corner and
//! centre crop marks, registration marks, description and label) with a small PDF writer
//! (ISO 32000: one page, a Flate-compressed image XObject, vector marks, Helvetica text), then
//! hands its bytes to `craft-print` (CUPS on macOS/Linux, Windows PDF printing on Windows).
//! `"dryRun": true` reports the backend command without spooling; a PDF is kept only at an
//! explicit output path. Shared spooler transport files are private and automatically removed.
//!
//! Where `lp` is not on the path (a Flatpak has none), the job goes to CUPS over IPP in-process
//! (`photocraft-ipp`, through the sandbox's `--socket=cups`), and when no print server or no
//! printer is reachable, to the app's spooler, the XDG Print portal (`Session::print_spooler`;
//! `PHOTOCRAFT_PRINT_PORTAL=1` prefers it, `=0` removes it). `"printServer"` names a server for
//! one job and always takes the IPP route; `file.printers` lists the queues (#2728).
//!
//! Paths to Illustrator writes the document's paths as an Adobe Illustrator 3 compatible
//! PostScript file (`m`/`L`/`C` path construction, `n`/`N` unpainted closed/open paths, `*u`/`*U`
//! compound paths), following the public PostScript Language Reference and the AI3 format notes.

use std::io::Write as _;

mod windows_marks;

use photocraft_color::ColorMode;
use photocraft_doc::{Document, LayerContent, SmartSource};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::file_cmds::{file_name, join, native_doc, read_file, stem, write_file};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
fn other(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

// ---------- Spooling ----------

/// A rendered page on its way to the printer: what the IPP route and the app's spooler need.
#[derive(Debug, Clone)]
pub struct PrintJob {
    pub pdf: Vec<u8>,
    /// The printer asked for; `None` means the default.
    pub printer: Option<String>,
    pub copies: u32,
    /// The queue entry's name: the document's.
    pub title: String,
    /// A print server named in the request (`printServer`), else the system's.
    pub server: Option<String>,
    /// The page size in points, as laid out (landscape already applied).
    pub paper: (f64, f64),
    pub landscape: bool,
}

/// A spooler's work: the job in, where it went out.
pub type SpoolFn = std::sync::Arc<dyn Fn(&PrintJob) -> std::result::Result<String, String> + Send + Sync>;

/// A further route to the printer, installed by the app on `Session::print_spooler`: the XDG
/// Print portal, which shows the system print dialog from inside a sandbox. It takes the jobs
/// no other route can (no print server reachable, no printer set up), or every job when
/// `preferred`.
#[derive(Clone)]
pub struct Spooler {
    /// Reported as the job's `backend`.
    pub name: &'static str,
    pub preferred: bool,
    /// Returns where the job went ("the system print dialog").
    pub run: SpoolFn,
}

/// Why the IPP route could not spool: no route at all (the app's spooler may have one), or a
/// failure on a route that exists (a wrong printer name, a rejected job).
enum SpoolError {
    NoRoute(String),
    Failed(String),
}

/// Sends the job and says where it went and through what. A named print server takes the IPP
/// route. Otherwise: the app's spooler when it is preferred; else `craft-print` (`lp`, or the
/// Windows spooler) when it is there; else CUPS over IPP; and the app's spooler when that had
/// no route either. A route that exists but fails (an unknown printer, a refused job) is an
/// error, never a fall-through.
fn spool_job(spooler: Option<&Spooler>, job: &PrintJob, lp: &craft_print::Job) -> Result<(String, String)> {
    let app = |sp: &Spooler| (sp.run)(job).map(|m| (m, sp.name.to_string())).map_err(other);
    if job.server.is_none() {
        if let Some(sp) = spooler.filter(|sp| sp.preferred) {
            return app(sp);
        }
        if let Some(name) = system_spooler() {
            require_windows_pdf()?;
            return craft_print::submit(&job.pdf, lp).map(|m| (m, name.to_string())).map_err(other);
        }
    }
    match cups_spool(job) {
        Ok(m) => Ok((m, "ipp".into())),
        Err(SpoolError::NoRoute(why)) => match spooler {
            Some(sp) => app(sp),
            None => Err(other(why)),
        },
        Err(SpoolError::Failed(why)) => Err(other(why)),
    }
}

/// The spooler `craft-print` can talk to, as the job's `backend` name: `lp` when CUPS' command
/// is on the path, the Windows spooler on Windows. `None` in a sandbox without `lp` and in the
/// browser.
fn system_spooler() -> Option<&'static str> {
    match craft_print::backend()? {
        craft_print::Backend::WindowsPdf => Some("windows"),
        craft_print::Backend::Cups => {
            let path = std::env::var_os("PATH")?;
            std::env::split_paths(&path).any(|dir| dir.join("lp").is_file()).then_some("lp")
        }
    }
}

/// The printer a job goes to: the one asked for when the server knows it, else the default,
/// else the only one; otherwise an error that names what there is.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "windows")))]
fn resolve_printer(requested: Option<&str>, default: Option<&str>, printers: &[photocraft_ipp::Printer]) -> std::result::Result<String, SpoolError> {
    let names: Vec<&str> = printers.iter().map(|p| p.name.as_str()).collect();
    let known = |n: &str| names.iter().find(|k| k.eq_ignore_ascii_case(n)).map(|k| (*k).to_string());
    if let Some(r) = requested {
        let choices = if names.is_empty() { "none".to_string() } else { names.join(", ") };
        return known(r).ok_or_else(|| SpoolError::Failed(format!("no printer called `{r}`; the print server knows: {choices}")));
    }
    if printers.is_empty() {
        return Err(SpoolError::NoRoute("no printer is set up on this computer; add one in the system settings, or print to PDF".into()));
    }
    if let Some(d) = default.and_then(known) {
        return Ok(d);
    }
    match names.as_slice() {
        [only] => Ok((*only).to_string()),
        _ => Err(SpoolError::Failed(format!("no default printer; choose one in the Printer field: {}", names.join(", ")))),
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "windows")))]
fn cups_server(named: Option<&str>) -> std::result::Result<photocraft_ipp::Endpoint, SpoolError> {
    match named {
        Some(server) => {
            let ep = photocraft_ipp::Endpoint::parse(server);
            if !ep.reachable() {
                return Err(SpoolError::Failed(format!("no print server at {ep}")));
            }
            Ok(ep)
        }
        None => photocraft_ipp::Endpoint::detect()
            .ok_or_else(|| SpoolError::NoRoute("no print server: CUPS is not running, or not reachable from this sandbox; print to PDF instead".into())),
    }
}

/// The IPP route: the job to the system's (or the named) CUPS server.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "windows")))]
fn cups_spool(job: &PrintJob) -> std::result::Result<String, SpoolError> {
    let failed = |e: photocraft_ipp::Error| SpoolError::Failed(e.to_string());
    let ep = cups_server(job.server.as_deref())?;
    let printers = photocraft_ipp::printers(&ep).map_err(failed)?;
    let default = printers.iter().find(|p| p.is_default).map(|p| p.name.clone());
    let printer = resolve_printer(job.printer.as_deref(), default.as_deref(), &printers)?;
    let is_class = printers.iter().any(|p| p.name == printer && p.is_class);
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "photocraft".into());
    let id = photocraft_ipp::print_job(
        &ep,
        &photocraft_ipp::Job { printer: &printer, is_class, title: &job.title, user: &user, copies: job.copies, document: &job.pdf, format: "application/pdf" },
    )
    .map_err(failed)?;
    Ok(format!("job {id} on {printer}"))
}

/// No IPP route here: a named server is a failure, the system's a missing route (the app's
/// spooler may have one).
#[cfg(any(target_arch = "wasm32", target_os = "windows"))]
fn cups_spool(job: &PrintJob) -> std::result::Result<String, SpoolError> {
    let why = if cfg!(target_arch = "wasm32") {
        "printing needs the desktop app"
    } else {
        "a print server over IPP is not available on Windows yet; print to PDF instead"
    };
    match job.server {
        Some(_) => Err(SpoolError::Failed(why.into())),
        None => Err(SpoolError::NoRoute(why.into())),
    }
}

/// `file.printers`: the printers the system's (or `printServer`'s) print server knows, with the default. From
/// `craft-print` (`lpstat`, the Windows spooler) where that is the route; over IPP for a named
/// server and where there is no `lp` (a sandbox).
fn system_printers(_s: &mut Session, p: &Value) -> Result<Value> {
    let named = p.get("printServer").and_then(Value::as_str).filter(|v| !v.is_empty());
    if named.is_none() && system_spooler().is_some() {
        let list = craft_print::printers();
        let default = list.iter().find(|p| p.default).map(|p| p.name.clone());
        let printers: Vec<Value> = list.into_iter().map(|p| json!({"name": p.name, "default": p.default})).collect();
        return Ok(json!({"server": Value::Null, "default": default, "printers": printers}));
    }
    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "windows")))]
    {
        let ep = match cups_server(named) {
            Ok(ep) => ep,
            Err(SpoolError::NoRoute(why) | SpoolError::Failed(why)) => {
                return Ok(json!({"server": Value::Null, "default": Value::Null, "printers": [], "note": why}));
            }
        };
        let list = photocraft_ipp::printers(&ep).map_err(other)?;
        let default = list.iter().find(|p| p.is_default).map(|p| p.name.clone());
        let printers: Vec<Value> = list
            .iter()
            .map(|p| {
                json!({"name": p.name, "info": p.info, "state": match p.state { 4 => "printing", 5 => "stopped", _ => "idle" }, "acceptingJobs": p.accepting_jobs, "default": p.is_default, "class": p.is_class})
            })
            .collect();
        Ok(json!({"server": ep.to_string(), "default": default, "printers": printers}))
    }
    #[cfg(not(all(not(target_arch = "wasm32"), not(target_os = "windows"))))]
    {
        let _ = named;
        Ok(json!({"server": Value::Null, "default": Value::Null, "printers": [], "note": "listing printers is not available on this platform yet"}))
    }
}

// ---------- PDF ----------

/// Paper sizes in points (portrait).
pub const PAPERS: [(&str, f64, f64); 8] = [
    ("letter", 612.0, 792.0),
    ("legal", 612.0, 1008.0),
    ("tabloid", 792.0, 1224.0),
    ("a3", 841.89, 1190.55),
    ("a4", 595.28, 841.89),
    ("a5", 419.53, 595.28),
    ("4x6", 288.0, 432.0),
    ("5x7", 360.0, 504.0),
];

/// The image placed on a print page.
pub struct PrintImage {
    pub width: u32,
    pub height: u32,
    /// 1 (gray), 3 (RGB) or 4 (CMYK, 1 = full ink), 8-bit interleaved.
    pub channels: usize,
    pub data: Vec<u8>,
    pub icc: Option<Vec<u8>>,
}

/// Printer marks around the image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Marks {
    pub corner_crop: bool,
    pub center_crop: bool,
    pub registration: bool,
    /// Photoshop's Functions › Bleed, in points: corner crop marks sit this far inside the image
    /// edge, so the trim cuts through the image and leaves no paper-white sliver. 0 = on the edge.
    pub bleed: f64,
}

/// Gap between the image edge and a crop mark, and the mark's length (pt).
const MARK_GAP: f64 = 6.0;
const MARK_LEN: f64 = 18.0;

/// Corner crop marks around `(x, y, w, h)` as eight `(from, to)` segments, a horizontal then a
/// vertical one per corner. Each starts `MARK_GAP` outside the image (so it never prints on it)
/// and lies on the trim line, `bleed` inside the edge. The inset is symmetric, so this holds for
/// both bottom-up (PDF) and top-down (Windows) coordinates.
fn corner_crop_segments(bleed: f64, (x, y, w, h): (f64, f64, f64, f64)) -> [((f64, f64), (f64, f64)); 8] {
    let mut out = [((0.0, 0.0), (0.0, 0.0)); 8];
    for (i, (cx, cy, dx, dy)) in [(x, y, -1.0, -1.0), (x + w, y, 1.0, -1.0), (x, y + h, -1.0, 1.0), (x + w, y + h, 1.0, 1.0)].into_iter().enumerate() {
        let (tx, ty) = (cx - dx * bleed, cy - dy * bleed);
        out[2 * i] = ((cx + dx * MARK_GAP, ty), (cx + dx * (MARK_GAP + MARK_LEN), ty));
        out[2 * i + 1] = ((tx, cy + dy * MARK_GAP), (tx, cy + dy * (MARK_GAP + MARK_LEN)));
    }
    out
}

/// One page: paper (pt), image box (pt, origin bottom-left), marks and texts.
pub struct PrintPage {
    pub paper: (f64, f64),
    pub image: PrintImage,
    pub rect: (f64, f64, f64, f64),
    pub marks: Marks,
    pub description: Option<String>,
    pub label: Option<String>,
    /// Photoshop's Functions › Emulsion Down: the whole page mirrored left to right, for transfer
    /// printing (sublimation, iron-on, DTF, film read from the back).
    pub emulsion_down: bool,
}

// ISO 32000-1, Annex D.2: the standard Helvetica font below uses WinAnsiEncoding,
// not UTF-8. Escape non-ASCII bytes as three octal digits so the content stays ASCII
// and a following digit cannot become part of the escape. Other scripts still need
// an embedded font; retain the existing '?' fallback for unsupported characters.
fn pdf_string(s: &str) -> String {
    let mut o = String::from("(");
    for c in s.chars() {
        let byte = match c {
            '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => c as u8,
            '€' => 0x80,
            '‚' => 0x82,
            'ƒ' => 0x83,
            '„' => 0x84,
            '…' => 0x85,
            '†' => 0x86,
            '‡' => 0x87,
            'ˆ' => 0x88,
            '‰' => 0x89,
            'Š' => 0x8a,
            '‹' => 0x8b,
            'Œ' => 0x8c,
            'Ž' => 0x8e,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '˜' => 0x98,
            '™' => 0x99,
            'š' => 0x9a,
            '›' => 0x9b,
            'œ' => 0x9c,
            'ž' => 0x9e,
            'Ÿ' => 0x9f,
            _ => b'?',
        };
        match byte {
            b'(' | b')' | b'\\' => {
                o.push('\\');
                o.push(char::from(byte));
            }
            0x80..=0xff => o.push_str(&format!("\\{byte:03o}")),
            _ => o.push(char::from(byte)),
        }
    }
    o.push(')');
    o
}

fn deflate(data: &[u8]) -> Vec<u8> {
    // Fast: print PDFs are transient and large; level 1 is ~4× quicker than the default.
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// Vector marks around `(x, y, w, h)`.
fn marks_ops(m: Marks, (x, y, w, h): (f64, f64, f64, f64)) -> String {
    let mut s = String::from("q 0 G 0.3 w\n");
    let (gap, len) = (MARK_GAP, MARK_LEN);
    let line = |s: &mut String, x0: f64, y0: f64, x1: f64, y1: f64| s.push_str(&format!("{x0:.2} {y0:.2} m {x1:.2} {y1:.2} l S\n"));
    if m.corner_crop {
        for ((x0, y0), (x1, y1)) in corner_crop_segments(m.bleed, (x, y, w, h)) {
            line(&mut s, x0, y0, x1, y1);
        }
    }
    if m.center_crop {
        let (mx, my) = (x + w / 2.0, y + h / 2.0);
        line(&mut s, mx, y - gap, mx, y - gap - len);
        line(&mut s, mx, y + h + gap, mx, y + h + gap + len);
        line(&mut s, x - gap, my, x - gap - len, my);
        line(&mut s, x + w + gap, my, x + w + gap + len, my);
    }
    if m.registration {
        let r = 5.0;
        let off = gap + len / 2.0;
        for (cx, cy) in [(x + w / 2.0, y - off), (x + w / 2.0, y + h + off), (x - off, y + h / 2.0), (x + w + off, y + h / 2.0)] {
            // Circle (four Béziers) plus crosshair.
            let k = 0.5523 * r;
            s.push_str(&format!(
                "{:.2} {cy:.2} m {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c S\n",
                cx + r, cx + r, cy + k, cx + k, cy + r, cy + r, cx - k, cy + r, cx - r, cy + k, cx - r, cx - r, cy - k, cx - k, cy - r, cy - r, cx + k, cy - r, cx + r, cy - k, cx + r
            ));
            line(&mut s, cx - r * 1.6, cy, cx + r * 1.6, cy);
            line(&mut s, cx, cy - r * 1.6, cx, cy + r * 1.6);
        }
    }
    s.push_str("Q\n");
    s
}

/// A one-page PDF.
pub fn print_pdf(page: &PrintPage) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    let img = &page.image;
    let (pw, ph) = page.paper;
    let (x, y, w, h) = page.rect;
    // 1 catalog, 2 pages, 3 page, 4 contents, 5 image, 6 font, 7 ICC (optional).
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    obj(
        &mut out,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 5 0 R >> /Font << /F1 6 0 R >> >> /Contents 4 0 R >>"
        )
        .as_bytes(),
    );
    // An explicit white page, so every viewer (and rasteriser) shows paper, not transparency.
    let mut content = format!("q 1 g 0 0 {pw:.2} {ph:.2} re f Q\n");
    if page.emulsion_down {
        content.push_str(&format!("q -1 0 0 1 {pw:.2} 0 cm\n"));
    }
    content.push_str(&format!("q {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Im0 Do Q\n"));
    content.push_str(&marks_ops(page.marks, page.rect));
    let any_marks = page.marks.corner_crop || page.marks.center_crop || page.marks.registration;
    let pad = if any_marks { 30.0 } else { 6.0 };
    if let Some(l) = page.label.as_deref().filter(|l| !l.is_empty()) {
        content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y + h + pad, pdf_string(l)));
    }
    if let Some(d) = page.description.as_deref().filter(|d| !d.is_empty()) {
        content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y - pad - 8.0, pdf_string(d)));
    }
    if page.emulsion_down {
        content.push_str("Q\n");
    }
    let c = deflate(content.as_bytes());
    let mut body = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", c.len()).into_bytes();
    body.extend_from_slice(&c);
    body.extend_from_slice(b"\nendstream");
    obj(&mut out, &body);
    let device = match img.channels {
        1 => "/DeviceGray",
        4 => "/DeviceCMYK",
        _ => "/DeviceRGB",
    };
    let cs = if img.icc.is_some() { "[/ICCBased 7 0 R]".to_string() } else { device.to_string() };
    let z = deflate(&img.data);
    let mut body = format!(
        "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {cs} /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
        img.width,
        img.height,
        z.len()
    )
    .into_bytes();
    body.extend_from_slice(&z);
    body.extend_from_slice(b"\nendstream");
    obj(&mut out, &body);
    obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    if let Some(icc) = &img.icc {
        let z = deflate(icc);
        let mut body = format!("<< /N {} /Alternate {device} /Filter /FlateDecode /Length {} >>\nstream\n", img.channels, z.len()).into_bytes();
        body.extend_from_slice(&z);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

// ---------- Print ----------

/// The flattened, colour-handled document as 8-bit samples in its print colour space.
fn print_image(doc: &Document, p: &Value, cmd: &str) -> Result<(PrintImage, Value)> {
    let mut t = Session::new();
    t.add_document(doc.clone(), None);
    let handling = p.get("colorHandling").and_then(Value::as_str).unwrap_or("printerManages");
    let mut info = json!({"colorHandling": handling});
    if !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale | ColorMode::Cmyk) {
        t.execute("image.mode.rgb", json!({}))?;
    }
    match handling {
        "photocraftManages" | "photoshopManages" => {
            let profile = p.get("printerProfile").and_then(Value::as_str).ok_or_else(|| bad(cmd, "\"photocraftManages\" needs a \"printerProfile\""))?;
            let intent = p.get("intent").and_then(Value::as_str).unwrap_or("relative");
            let bpc = p.get("bpc").and_then(Value::as_bool).unwrap_or(true);
            t.execute("edit.convertToProfile", json!({"profile": profile, "intent": intent, "bpc": bpc}))?;
            info["printerProfile"] = json!(profile);
            info["intent"] = json!(intent);
            info["bpc"] = json!(bpc);
        }
        "printerManages" | "noColorManagement" => {}
        "separations" => return Err(bad(cmd, "Separations printing is not supported; use photocraftManages with a CMYK printer profile")),
        h => return Err(bad(cmd, format!("unknown colorHandling `{h}` (printerManages|photocraftManages|noColorManagement)"))),
    }
    let d = &t.active().ok_or(EngineError::NoDocument)?.doc;
    let icc = if handling == "noColorManagement" { None } else { d.icc_profile.as_ref().map(|v| v.to_vec()) };
    // RGB and Grayscale: the composite over white is the print image (no layer flatten).
    if matches!(d.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        let gray = d.mode == ColorMode::Grayscale;
        let buf = photocraft_compose::flatten(d).over_background([1.0, 1.0, 1.0]);
        let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let data: Vec<u8> =
            if gray { buf.px.iter().map(|p| to8(p[0])).collect() } else { buf.px.iter().flat_map(|p| [to8(p[0]), to8(p[1]), to8(p[2])]).collect() };
        return Ok((PrintImage { width: d.size.width, height: d.size.height, channels: if gray { 1 } else { 3 }, data, icc }, info));
    }
    t.execute("layer.flattenImage", json!({}))?;
    let d = &t.active().ok_or(EngineError::NoDocument)?.doc;
    let fmt = d.pixel_format();
    let k = fmt.mode.color_channels();
    let n = fmt.channels();
    let surf = d.layers.first().and_then(|l| l.surface()).ok_or_else(|| other("nothing to print"))?;
    let vals = surf.read_region(d.bounds());
    let mut data = Vec::with_capacity(vals.len() / n * k);
    for px in vals.chunks_exact(n) {
        for v in &px[..k] {
            data.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    Ok((PrintImage { width: d.size.width, height: d.size.height, channels: k, data, icc }, info))
}

fn paper(p: &Value, cmd: &str) -> Result<(f64, f64)> {
    let (w, h) = match p.get("paper") {
        None => (612.0, 792.0),
        Some(Value::String(name)) => PAPERS
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, w, h)| (*w, *h))
            .ok_or_else(|| bad(cmd, format!("unknown paper `{name}` ({})", PAPERS.map(|p| p.0).join("|"))))?,
        Some(Value::Array(a)) if a.len() == 2 => (a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)),
        _ => return Err(bad(cmd, "paper is a name or [width, height] in points")),
    };
    if !(w > 36.0 && h > 36.0) {
        return Err(bad(cmd, "paper is too small"));
    }
    Ok(if p.get("orientation").and_then(Value::as_str) == Some("landscape") { (h.max(w), w.min(h)) } else { (w, h) })
}

/// The driver's printable area (`printable`: [x, y, width, height] pt from the paper's top-left,
/// as `file.print.page` reports it), clipped to the paper, with the layout's bottom-left origin.
fn printable(p: &Value, (pw, ph): (f64, f64), cmd: &str) -> Result<Option<(f64, f64, f64, f64)>> {
    let Some(v) = p.get("printable").filter(|v| !v.is_null()) else { return Ok(None) };
    let r: Option<Vec<f64>> = v.as_array().and_then(|a| a.iter().map(Value::as_f64).collect());
    let Some([x, y, w, h]) = r.as_deref().and_then(|r| <[f64; 4]>::try_from(r).ok()) else {
        return Err(bad(cmd, "printable is [x, y, width, height] in points from the paper's top-left"));
    };
    // Drivers report their paper a fraction of a point off the named size (842.29 pt for A3's
    // 841.89), so clip that much; an area that doesn't fit the paper (another paper or
    // orientation) is refused. 2 pt, as `print_windows` allows between driver and layout.
    const SLACK: f64 = 2.0;
    if !(x >= -SLACK && y >= -SLACK && x + w <= pw + SLACK && y + h <= ph + SLACK) {
        return Err(bad(cmd, "the printable area doesn't fit the paper and orientation"));
    }
    let (x0, y0, x1, y1) = (x.max(0.0), y.max(0.0), (x + w).min(pw), (y + h).min(ph));
    if !(x1 > x0 && y1 > y0) {
        return Err(bad(cmd, "the printable area is empty"));
    }
    Ok(Some((x0, ph - y1, x1 - x0, y1 - y0)))
}

/// Where the image lands on the paper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrintLayout {
    /// Paper size in points (after orientation).
    pub paper: (f64, f64),
    /// Image box in points, origin bottom-left.
    pub rect: (f64, f64, f64, f64),
    /// Print scale (1.0 = 100 %).
    pub scale: f64,
    pub marks: Marks,
    /// Emulsion Down: the page prints mirrored (`rect` is where the image is before mirroring).
    pub emulsion_down: bool,
}

/// Symmetric safety bound for manual Top/Left offsets, shared with the dialog (inches).
pub const MAX_POSITION_INCHES: f64 = 10000.0;

/// Resolves the shared preview/PDF layout without changing `doc`.
///
/// Paper and image bounds use points (72 per inch), with the image origin at bottom-left.
/// Manual `top`/`left` are inches from the paper's top-left; negative values place artwork
/// off-page. Center ignores these offsets. Scale to Fit preserves the aspect ratio and uses
/// the available paper area after the existing mark margins, overriding percentage scale.
/// Emulsion Down is recorded for rendering; the returned rectangle is not yet mirrored.
///
/// Returns an error for malformed manual offsets or offsets beyond [`MAX_POSITION_INCHES`],
/// nonfinite geometry, geometry magnitudes above 1e9, or nonpositive image dimensions.
pub fn layout(doc: &Document, p: &Value, cmd: &str) -> Result<PrintLayout> {
    let (pw, ph) = paper(p, cmd)?;
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    let (iw, ih) = (f64::from(doc.size.width) * 72.0 / dpi, f64::from(doc.size.height) * 72.0 / dpi);
    // Bleed in inches, like Photoshop's Position fields; the marks need it in points.
    let bleed = match p.get("bleed") {
        None | Some(Value::Null) => 0.0,
        Some(v) => v.as_f64().filter(|b| b.is_finite() && *b >= 0.0).ok_or_else(|| bad(cmd, "bleed is a width in inches, 0 or more"))? * 72.0,
    };
    let mut marks = Marks {
        corner_crop: p.get("cornerCropMarks").and_then(Value::as_bool).unwrap_or(false),
        center_crop: p.get("centerCropMarks").and_then(Value::as_bool).unwrap_or(false),
        registration: p.get("registrationMarks").and_then(Value::as_bool).unwrap_or(false),
        bleed,
    };
    let has_marks = marks.corner_crop || marks.center_crop || marks.registration;
    // Scale to Fit Media fits the driver's printable area when it is known (the whole sheet for
    // borderless paper); marks keep their inset inside it. Without one (CUPS, PDF), the paper
    // minus a fixed margin.
    let emulsion_down = p.get("emulsionDown").and_then(Value::as_bool).unwrap_or(false);
    let fit = p.get("scaleToFit").and_then(Value::as_bool).unwrap_or(false);
    // Only a fitted print reads the area, so a stale one (a replayed action, another paper) can't
    // fail a print at a manual scale.
    let area = if fit { printable(p, (pw, ph), cmd)? } else { None };
    let fit_box = match area {
        Some((x, y, w, h)) => {
            // Emulsion Down mirrors the page as it prints: fit the mirror of the printable area,
            // so the printed image lands inside the real one.
            let x = if emulsion_down { pw - x - w } else { x };
            let inset = if has_marks { 36.0 } else { 0.0 };
            if !(w > 2.0 * inset && h > 2.0 * inset) {
                return Err(bad(cmd, "the printable area is too small for crop and registration marks"));
            }
            (x + inset, y + inset, w - 2.0 * inset, h - 2.0 * inset)
        }
        None => {
            let margin = if has_marks { 36.0 } else { 18.0 };
            (margin, margin, pw - 2.0 * margin, ph - 2.0 * margin)
        }
    };
    let scale = if fit { (fit_box.2 / iw).min(fit_box.3 / ih) } else { p.get("scale").and_then(Value::as_f64).filter(|v| *v > 0.0).unwrap_or(100.0) / 100.0 };
    let (w, h) = (iw * scale, ih * scale);
    // The trim box must keep some area, or the marks of opposite corners would cross.
    if 2.0 * marks.bleed >= w.min(h) {
        return Err(bad(cmd, format!("bleed {:.3} in leaves no image inside the trim ({:.2} × {:.2} in printed)", marks.bleed / 72.0, w / 72.0, h / 72.0)));
    }
    // Bleed only moves crop marks; without them it has nothing to change.
    if !marks.corner_crop {
        marks.bleed = 0.0;
    }
    let center = p.get("center").and_then(Value::as_bool).unwrap_or(true);
    let (x, y) = if center && fit {
        (fit_box.0 + (fit_box.2 - w) / 2.0, fit_box.1 + (fit_box.3 - h) / 2.0)
    } else if center {
        ((pw - w) / 2.0, (ph - h) / 2.0)
    } else {
        // Top / left in inches from the paper's top-left corner (Photoshop's Position fields).
        let position = |key: &str| -> Result<f64> {
            let value = match p.get(key) {
                None => 0.0,
                Some(v) => v.as_f64().ok_or_else(|| bad(cmd, "print positions must be finite numbers in inches"))?,
            };
            if !value.is_finite() || value.abs() > MAX_POSITION_INCHES {
                return Err(bad(cmd, "print positions must be within -10000 to 10000 inches"));
            }
            Ok(value * 72.0)
        };
        let top = position("top")?;
        let left = position("left")?;
        (left, ph - top - h)
    };
    // Guard geometry before converting to screen points or writing PDF operators. Even a
    // finite JSON scale can overflow when multiplied by document dimensions.
    if ![pw, ph, x, y, w, h, scale].into_iter().all(|v| v.is_finite() && v.abs() <= 1.0e9) || w <= 0.0 || h <= 0.0 {
        return Err(bad(cmd, "print layout must have finite geometry within 1000000000 points and positive image dimensions"));
    }
    Ok(PrintLayout { paper: (pw, ph), rect: (x, y, w, h), scale, marks, emulsion_down })
}

/// Lays out, renders and (unless dry-run) spools a print. Shared by Print and Print One Copy.
fn do_print(s: &mut Session, p: &Value, cmd: &str) -> Result<Value> {
    let output = p.get("output").and_then(Value::as_str).filter(|v| !v.is_empty());
    let send = p.get("send").and_then(Value::as_bool).unwrap_or(output.is_none());
    let dry = p.get("dryRun").and_then(Value::as_bool).unwrap_or(false);
    if !dry && !send && output.is_none() {
        return Err(bad(cmd, "send: false needs an output path or dryRun: true"));
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = d.doc.clone();
    let doc_id = doc.id;
    let params = p.clone();
    let worker_params = params.clone();
    let command = cmd.to_string();
    let spooler = s.print_spooler.clone();
    if send && !dry {
        // Session::execute remains synchronous; the desktop's Session::start snapshots here
        // and runs PDF composition and the potentially slow driver on a worker.
        return crate::jobs::run(
            s,
            "Print",
            false,
            move |ctx| {
                ctx.progress(0.0, "Print");
                render_and_spool(&doc, &worker_params, &command, spooler.as_ref())
            },
            move |s, result| {
                remember_print(s, &params);
                // A user can switch tabs while printing. Hooks belong to the source document;
                // a closed source has no document left to modify, as for background saves.
                let viewed = s.active().map(|st| st.doc.id);
                if let Some(index) = s.documents().iter().position(|st| st.doc.id == doc_id) {
                    s.set_active(index);
                    crate::automate_cmds::fire_event(s, "print");
                    if let Some(index) = viewed.and_then(|id| s.documents().iter().position(|st| st.doc.id == id)) {
                        s.set_active(index);
                    }
                }
                Ok(result)
            },
        );
    }
    let result = render_and_spool(&doc, p, cmd, spooler.as_ref())?;
    remember_print(s, p);
    crate::automate_cmds::fire_event(s, "print");
    Ok(result)
}

/// True when a print may go to a Windows printer through the driver rather than a PDF spooler.
fn native_route(p: &Value, spooler: Option<&Spooler>) -> bool {
    // A named print server always takes the IPP route and a preferred app spooler takes every
    // job (the route order in `spool_job`), on Windows too: neither may enter the native path,
    // which would print on the local default printer instead.
    let named_server = p.get("printServer").and_then(Value::as_str).is_some_and(|v| !v.is_empty());
    let skip_native = named_server || spooler.is_some_and(|sp| sp.preferred);
    let output = p.get("output").and_then(Value::as_str).is_some_and(|v| !v.is_empty());
    let send = p.get("send").and_then(Value::as_bool).unwrap_or(!output);
    let dry = p.get("dryRun").and_then(Value::as_bool).unwrap_or(false);
    cfg!(windows) && !skip_native && send && !dry && !output
}

/// `p` with the driver's printable area, read from the printer itself, so Print One Copy, the
/// CLI, control and replayed actions fit the same area as the Print dialog does (#7782). A
/// native print never fits an area the caller passed: without the driver's, it has none.
fn with_driver_printable(p: &Value) -> Value {
    let mut out = p.clone();
    if let Some(o) = out.as_object_mut() {
        o.remove("printable");
        if let Some(area) = driver_printable(p) {
            o.insert("printable".into(), area);
        }
    }
    out
}

fn driver_printable(p: &Value) -> Option<Value> {
    let printer = printer_name(p).ok()?;
    let pg = craft_print::rgb::page(&printer, printer_settings(p).ok()?.as_ref()).ok()?;
    // Only for the paper the driver is set to: a print laid out on another paper keeps the
    // fixed margin (and `print_windows` refuses it), as before.
    let (pw, ph) = paper(p, "file.print").ok()?;
    if (pg.paper.0 - pw).abs() > 2.0 || (pg.paper.1 - ph).abs() > 2.0 {
        return None;
    }
    Some(json!([pg.printable.0, pg.printable.1, pg.printable.2, pg.printable.3]))
}

fn render_and_spool(doc: &Document, p: &Value, cmd: &str, spooler: Option<&Spooler>) -> Result<Value> {
    let fit = p.get("scaleToFit").and_then(Value::as_bool).unwrap_or(false);
    let driver = (fit && native_route(p, spooler)).then(|| with_driver_printable(p));
    let p = driver.as_ref().unwrap_or(p);
    let PrintLayout { paper: (pw, ph), rect: (x, y, w, h), scale, marks, emulsion_down } = layout(doc, p, cmd)?;
    let (img, color) = print_image(doc, p, cmd)?;
    let description = p
        .get("description")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .then(|| crate::file_cmds::read_file_info(doc.metadata.xmp.as_deref())["description"].as_str().unwrap_or_default().to_string());
    let label = p.get("labels").and_then(Value::as_bool).unwrap_or(false).then(|| doc.name.clone());
    let page = PrintPage { paper: (pw, ph), image: img, rect: (x, y, w, h), marks, description, label, emulsion_down };
    let output = p.get("output").and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string);
    let send = p.get("send").and_then(Value::as_bool).unwrap_or(output.is_none());
    let copies = crate::commands::int(p, "copies").unwrap_or(1).clamp(1, 999) as u32;
    let dry = p.get("dryRun").and_then(Value::as_bool).unwrap_or(false);
    // Shared RGB transport handles compatible Windows pages; PDF-only content retains its route.
    // Sending follows the shared command contract; fuzz tests explicitly request dryRun.
    // The worker returns its result before repeat settings and hooks are applied once.
    let handling = p.get("colorHandling").and_then(Value::as_str).unwrap_or("printerManages");
    let native = native_route(p, spooler);
    if native && native_rgb_compatible(&page, handling) {
        require_printing_enabled()?;
        let printed = print_windows(&page, p, &doc.name, copies)?;
        return Ok(json!({
            "pdf": Value::Null, "paper": [pw, ph], "imageRect": [x, y, w, h], "emulsionDown": emulsion_down, "bleed": marks.bleed / 72.0, "scale": scale * 100.0,
            "copies": copies, "command": Value::Null, "sent": true, "spooler": printed, "color": color,
        }));
    }
    if native && printer_settings(p)?.is_some() {
        return Err(other("the selected color profile or page text requires PDF printing. Save as PDF, or clear Print Settings to use PDF printing."));
    }
    let pdf = print_pdf(&page);
    let pdf_len = pdf.len();
    let job = craft_print::Job {
        printer: p.get("printer").and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string),
        copies,
        title: doc.name.clone(),
        // PhotoCraft already placed and scaled the image on its physical sheet. Scaling again
        // in the driver would change the preview, print resolution and crop-mark positions.
        scaling: craft_print::Scaling::Exact,
        ..Default::default()
    };
    let command = if send { json!(craft_print::command_preview(&job).map_err(other)?) } else { Value::Null };
    let pdf_path = match output {
        Some(path) => {
            write_file(&path, &pdf)?;
            Some(path)
        }
        None => None,
    };
    let mut sent = false;
    let mut spool = Value::Null;
    let mut backend = Value::Null;
    if send && !dry {
        require_printing_enabled()?;
        let print_job = PrintJob {
            printer: job.printer.clone(),
            copies,
            title: doc.name.clone(),
            server: p.get("printServer").and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string),
            paper: (pw, ph),
            landscape: p.get("orientation").and_then(Value::as_str) == Some("landscape"),
            pdf,
        };
        let (where_, via) = spool_job(spooler, &print_job, &job)?;
        spool = json!(where_);
        backend = json!(via);
        sent = true;
    }
    Ok(
        json!({"pdf": pdf_path, "bytes": pdf_len, "paper": [pw, ph], "imageRect": [x, y, w, h], "emulsionDown": emulsion_down, "bleed": marks.bleed / 72.0, "scale": scale * 100.0, "copies": copies, "command": command, "sent": sent, "spooler": spool, "backend": backend, "color": color}),
    )
}

/// Windows prints a PDF through Windows.Data.Pdf.dll, which the apps delay-load (see their
/// `build.rs`) so a Windows image without it still starts. Calling into it when it cannot load
/// would abort the process, so check that it loads first (#5884).
fn require_windows_pdf() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| std::ffi::OsString::from(r"C:\Windows"));
        if !windows_pdf_loads(&windows_pdf_dll(std::path::Path::new(&root))) {
            return Err(other(
                "printing this page needs the Windows PDF component (Windows.Data.Pdf.dll), which is missing or cannot load. Save as PDF and print it from another app, or print an RGB image without page text or labels to print directly.",
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_pdf_dll(system_root: &std::path::Path) -> std::path::PathBuf {
    system_root.join("System32").join("Windows.Data.Pdf.dll")
}

/// Loads the DLL rather than checking the file exists: one that is present but cannot load (a
/// dependency stripped too) would still abort at the delay-load stub.
#[cfg(target_os = "windows")]
fn windows_pdf_loads(dll: &std::path::Path) -> bool {
    dll.to_str().is_some_and(|p| winsafe::HINSTANCE::LoadLibrary(p).is_ok())
}

/// Keep CMYK output profiles and page text on the existing PDF composition path. GDI currently
/// draws RGB/gray and vector marks only; it must never silently omit requested page content.
fn native_rgb_compatible(page: &PrintPage, handling: &str) -> bool {
    matches!(page.image.channels, 1 | 3)
        && (page.image.icc.is_none() || matches!(handling, "photocraftManages" | "photoshopManages" | "noColorManagement"))
        && page.description.as_deref().is_none_or(str::is_empty)
        && page.label.as_deref().is_none_or(str::is_empty)
}

/// Where the settings of the last print are kept in Preferences' remembered dialog settings.
pub const REMEMBERED: &str = "file.print";

/// Print One Copy repeats these, and Print opens with them. They are kept in the preferences as
/// well, so they survive a relaunch, as Photoshop's do (#7410).
fn remember_print(s: &mut Session, p: &Value) {
    let mut remembered = p.clone();
    if let Some(o) = remembered.as_object_mut() {
        o.remove("output");
        o.remove("dryRun");
        // Output and dry-run choices are not remembered, so a send override must not survive them.
        o.remove("send");
        // The driver's printable area belongs to the printer and paper chosen now: the dialog
        // reads it again, and a later print to PDF or another printer must not inherit it.
        o.remove("printable");
    }
    let kept = remembered.clone();
    s.prefs.edit(|prefs| {
        prefs.dialogs.insert(REMEMBERED.into(), kept);
    });
    s.file_menu.last_print = Some(remembered);
}

/// Drops the `printer` of the settings `q` (and its driver settings) when the print server lists
/// printers but not that one, so the job goes to the default printer. An empty list can't tell.
fn drop_vanished_printer(q: &mut Value, listed: &[&str]) {
    let Some(name) = q.get("printer").and_then(Value::as_str).filter(|n| !n.is_empty()) else { return };
    if listed.is_empty() || listed.iter().any(|l| l.eq_ignore_ascii_case(name)) {
        return;
    }
    if let Some(o) = q.as_object_mut() {
        o.remove("printer");
        o.remove("printerSettings");
    }
}

/// The settings of the last print: this session's, else those remembered from an earlier one.
pub fn last_print(s: &Session) -> Option<Value> {
    s.file_menu.last_print.clone().or_else(|| s.prefs().dialogs.get(REMEMBERED).filter(|v| v.is_object()).cloned())
}

/// The `printer` parameter, or the system's default printer.
fn printer_name(p: &Value) -> Result<String> {
    if let Some(name) = p.get("printer").and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
        return Ok(name.to_string());
    }
    craft_print::rgb::default_printer().map_err(other)?.ok_or_else(|| other("no printer chosen and no default printer is set"))
}

/// The driver settings chosen with Print Settings… (`printerSettings`, hex), when any.
fn printer_settings(p: &Value) -> Result<Option<craft_print::rgb::DevMode>> {
    match p.get("printerSettings").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        Some(hex) => craft_print::rgb::DevMode::from_hex(hex).map(Some).map_err(|e| bad("file.print", e.to_string())),
        None => Ok(None),
    }
}

/// `dmFields` and `dmOrientation` in DEVMODEW (wingdi.h: after the 32-wchar device name and four
/// header words), and the `dmFields` bit that says `dmOrientation` is set.
const DM_FIELDS_AT: usize = 72;
const DM_ORIENTATION_AT: usize = 76;
const DM_ORIENTATION: u32 = 0x1;
/// `DMORIENT_PORTRAIT` and `DMORIENT_LANDSCAPE`.
const DMORIENT_PORTRAIT: i16 = 1;
const DMORIENT_LANDSCAPE: i16 = 2;

/// Driver settings turned to portrait or landscape, as Photoshop's Layout buttons turn them:
/// `dmOrientation` set and flagged in `dmFields`, every other byte (the driver's private settings
/// included) kept. The driver validates the result when it lays out the page, as it does for
/// settings from Print Settings….
pub fn turned_settings(settings: &craft_print::rgb::DevMode, landscape: bool) -> Result<craft_print::rgb::DevMode> {
    let mut b = settings.as_bytes().to_vec();
    let short = || other("printer settings are too short to hold an orientation");
    let fields: [u8; 4] = b.get(DM_FIELDS_AT..DM_FIELDS_AT + 4).and_then(|s| s.try_into().ok()).ok_or_else(short)?;
    let fields = u32::from_le_bytes(fields) | DM_ORIENTATION;
    b.get_mut(DM_FIELDS_AT..DM_FIELDS_AT + 4).ok_or_else(short)?.copy_from_slice(&fields.to_le_bytes());
    let orientation = if landscape { DMORIENT_LANDSCAPE } else { DMORIENT_PORTRAIT };
    b.get_mut(DM_ORIENTATION_AT..DM_ORIENTATION_AT + 2).ok_or_else(short)?.copy_from_slice(&orientation.to_le_bytes());
    craft_print::rgb::DevMode::from_bytes(b).map_err(other)
}

/// Whether a driver page of `driver` points is the layout's `paper` turned the other way round
/// (within the 2 pt the driver and layout may differ by): then turning the driver settings, not
/// choosing another paper, makes them agree. A square paper is never turned.
fn turned_paper(driver: (f64, f64), paper: (f64, f64)) -> bool {
    let near = |a: f64, b: f64| (a - b).abs() <= 2.0;
    let same = near(driver.0, paper.0) && near(driver.1, paper.1);
    !same && near(driver.0, paper.1) && near(driver.1, paper.0)
}

/// The driver settings and page for printing a layout of `paper` points on `printer`: the given
/// settings (or the printer's own) as they are, or turned to the layout's orientation when the
/// driver has that paper the other way round. A landscape layout so prints landscape with portrait
/// Print Settings, as a script or another app asking for landscape expects.
fn settings_for_layout(
    printer: &str,
    settings: Option<craft_print::rgb::DevMode>,
    paper: (f64, f64),
) -> Result<(Option<craft_print::rgb::DevMode>, craft_print::rgb::Page)> {
    let page = craft_print::rgb::page(printer, settings.as_ref()).map_err(other)?;
    if !turned_paper(page.paper, paper) {
        return Ok((settings, page));
    }
    let base = match settings {
        Some(s) => s,
        None => craft_print::rgb::default_settings(printer).map_err(other)?,
    };
    let turned = turned_settings(&base, paper.0 > paper.1)?;
    let page = craft_print::rgb::page(printer, Some(&turned)).map_err(other)?;
    Ok((Some(turned), page))
}

/// The page's image as 8-bit RGB rows (gray expanded), mirrored for Emulsion Down.
pub fn page_rgb(page: &PrintPage) -> Result<Vec<u8>> {
    let img = &page.image;
    let (w, h) = (img.width as usize, img.height as usize);
    let mut rgb = match img.channels {
        3 => img.data.clone(),
        1 => img.data.iter().flat_map(|g| [*g, *g, *g]).collect(),
        _ => return Err(other("this printer profile is CMYK; Windows printers take RGB, so choose the printer's own (RGB) profile")),
    };
    let row = w.checked_mul(3).ok_or_else(|| other("print image is too large"))?;
    if row == 0 || row.checked_mul(h) != Some(rgb.len()) {
        return Err(other("nothing to print"));
    }
    if page.emulsion_down {
        for row in rgb.chunks_exact_mut(row) {
            row.as_chunks_mut::<3>().0.reverse();
        }
    }
    Ok(rgb)
}

/// The image box with its origin at the sheet's top-left (GDI's), mirrored for Emulsion Down.
pub fn page_rect_top_left(page: &PrintPage) -> (f64, f64, f64, f64) {
    let (pw, ph) = page.paper;
    let (x, y, w, h) = page.rect;
    let x = if page.emulsion_down { pw - x - w } else { x };
    (x, ph - y - h, w, h)
}

/// Prints `page` on a Windows printer. Corner/centre crop and registration marks are drawn with
/// GDI vector calls, matching the PDF path's geometry; description and label text are still
/// PDF-only (no font/text layout in the GDI path yet).
fn print_windows(page: &PrintPage, p: &Value, title: &str, copies: u32) -> Result<Value> {
    let printer = printer_name(p)?;
    let (settings, device_page) = settings_for_layout(&printer, printer_settings(p)?, page.paper)?;
    if (device_page.paper.0 - page.paper.0).abs() > 2.0 || (device_page.paper.1 - page.paper.1).abs() > 2.0 {
        return Err(other("the driver paper or orientation differs from the print layout; choose the matching paper in Print Settings before printing"));
    }
    let rgb = page_rgb(page)?;
    let image = craft_print::rgb::RgbImage { width: page.image.width, height: page.image.height, data: &rgb };
    let rect = page_rect_top_left(page);
    let primitives = windows_marks::primitives(page.marks, rect);
    let color_policy = match p.get("colorHandling").and_then(Value::as_str) {
        Some("photocraftManages" | "photoshopManages") => craft_print::rgb::ColorPolicy::ApplicationManaged,
        Some("noColorManagement") => craft_print::rgb::ColorPolicy::Unmanaged,
        _ => craft_print::rgb::ColorPolicy::DriverManaged,
    };
    let job = craft_print::rgb::Job {
        printer: &printer,
        settings: settings.as_ref(),
        title,
        output_file: None,
        copies: copies.clamp(1, 999),
        color_policy,
        primitives: &primitives,
    };
    let done = craft_print::rgb::print(&job, &image, rect).map_err(other)?;
    Ok(
        json!({"printer": printer, "dpi": [done.dpi.0, done.dpi.1], "deviceRect": [done.device_rect.0, done.device_rect.1, done.device_rect.2, done.device_rect.3]}),
    )
}

fn printers(_: &mut Session, _: &Value) -> Result<Value> {
    match craft_print::rgb::printers() {
        Ok(list) => Ok(json!({"supported": true, "printers": list.iter().map(|p| json!({"name": p.name, "default": p.default})).collect::<Vec<_>>()})),
        Err(craft_print::rgb::PrintError::Unsupported) => Ok(json!({"supported": false, "printers": []})),
        Err(e) => Err(other(e)),
    }
}

fn printer_page(_: &mut Session, p: &Value) -> Result<Value> {
    // Checked before the printer, so a bad value is the error whether or not a printer answers.
    let landscape = match p.get("orientation").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => match v.as_str() {
            Some("portrait") => Some(false),
            Some("landscape") => Some(true),
            _ => return Err(bad("file.print.page", "orientation is \"portrait\" or \"landscape\"")),
        },
    };
    let printer = printer_name(p)?;
    let mut settings = printer_settings(p)?;
    if let Some(landscape) = landscape {
        let base = match settings {
            Some(s) => s,
            None => craft_print::rgb::default_settings(&printer).map_err(other)?,
        };
        settings = Some(turned_settings(&base, landscape)?);
    }
    let pg = craft_print::rgb::page(&printer, settings.as_ref()).map_err(other)?;
    let mut out = json!({"printer": printer, "paper": [pg.paper.0, pg.paper.1], "printable": [pg.printable.0, pg.printable.1, pg.printable.2, pg.printable.3], "dpi": [pg.dpi.0, pg.dpi.1]});
    // The turned settings, to pass on as `printerSettings` (the Print dialog's Orientation does).
    if let (Some(_), Some(s), Some(o)) = (landscape, settings, out.as_object_mut()) {
        o.insert("printerSettings".into(), json!(s.to_hex()));
    }
    Ok(out)
}

/// Set once by test harnesses (the panic hunt) so no command in the process can reach a printer.
static PRINTING_DISABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Refuse to spool anything from this process from now on (tests, fuzzing). The
/// `PHOTOCRAFT_NO_PRINT` environment variable does the same for a whole shell (CI).
pub fn disable_printing() {
    PRINTING_DISABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Whether spooling is blocked by [`disable_printing`] or `PHOTOCRAFT_NO_PRINT`.
pub fn printing_disabled() -> bool {
    PRINTING_DISABLED.load(std::sync::atomic::Ordering::Relaxed) || std::env::var_os("PHOTOCRAFT_NO_PRINT").is_some_and(|v| !v.is_empty() && v != "0")
}

fn require_printing_enabled() -> Result<()> {
    if printing_disabled() {
        Err(other("printing is disabled in this process (photocraft_engine::print_cmds::disable_printing or PHOTOCRAFT_NO_PRINT); use dryRun: true"))
    } else {
        Ok(())
    }
}

fn print(s: &mut Session, p: &Value) -> Result<Value> {
    do_print(s, p, "file.print")
}

fn print_one_copy(s: &mut Session, p: &Value) -> Result<Value> {
    let mut q = last_print(s).unwrap_or_else(|| json!({}));
    // A printer remembered from an earlier session may be gone: then the default takes the job,
    // as the Print dialog falls back (#7410). A list that can't be read decides nothing.
    if let Ok(listed) = system_printers(s, &q) {
        let names: Vec<&str> = listed["printers"].as_array().into_iter().flatten().filter_map(|p| p["name"].as_str()).collect();
        drop_vanished_printer(&mut q, &names);
    }
    if let (Some(o), Some(extra)) = (q.as_object_mut(), p.as_object()) {
        for (k, v) in extra {
            o.insert(k.clone(), v.clone());
        }
    }
    q["copies"] = json!(1);
    do_print(s, &q, "file.printOneCopy")
}

// ---------- Package ----------

fn package(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.package";
    let dir = p.get("dir").and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or_else(|| bad(cmd, "missing \"dir\" (where to create the package folder)"))?;
    // The copies are relinked by absolute path: a relative one would be resolved against the
    // package document's folder, not the working directory `dir` is relative to.
    // `folder`, `document` and `links` are built on it with `join`, so it is spelt the same way:
    // `/` throughout, not a native prefix with `/` after it.
    let dir = std::path::absolute(dir).map_or_else(|_| dir.to_string(), |d| d.to_string_lossy().into_owned());
    let dir = crate::file_cmds::forward_slashes(dir);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = (*d.doc).clone();
    let document = d.path.clone();
    let name = stem(&d.path.clone().unwrap_or_else(|| doc.name.clone()));
    // The native format keeps external links; our PSD writer has no `lnkE` (external linked
    // file) support yet, so a PSD package would embed or flatten them.
    let ext = match p.get("format").and_then(Value::as_str) {
        Some(f @ ("psd" | "psb" | "pcraft")) => f.to_string(),
        Some(f) => return Err(bad(cmd, format!("unknown package format `{f}` (pcraft|psd|psb)"))),
        None => "pcraft".into(),
    };
    let folder = join(&dir, &crate::file_cmds::sanitize(&name));
    let links_dir = join(&folder, "Links");
    // Copy every linked file once; relink the copy of the document to the copies.
    let mut copied: Vec<(String, String)> = Vec::new();
    let mut relinks: Vec<(photocraft_doc::LayerId, String)> = Vec::new();
    let mut missing = Vec::new();
    for (_, _, l) in doc.walk() {
        let LayerContent::Smart(so) = &l.content else { continue };
        let SmartSource::Linked { path } = &so.source else { continue };
        let target = match copied.iter().find(|(src, _)| src == path) {
            Some((_, t)) => t.clone(),
            None => {
                let mut fname = file_name(path);
                if copied.iter().any(|(_, t)| file_name(t) == fname) {
                    fname = format!("{}-{}", copied.len(), fname);
                }
                let t = join(&links_dir, &fname);
                // A path that is not followed is packaged as missing, like a file that is gone.
                let Ok(file) = crate::doc_path::resolve_in(path, document.as_deref().map(std::path::Path::new)) else {
                    missing.push(path.clone());
                    continue;
                };
                match read_file(&file.to_string_lossy()) {
                    Ok(bytes) => {
                        write_file(&t, &bytes)?;
                        copied.push((path.clone(), t.clone()));
                        t
                    }
                    Err(_) => {
                        missing.push(path.clone());
                        continue;
                    }
                }
            }
        };
        relinks.push((l.id, target));
    }
    let mut t = Session::new();
    t.add_document(doc, None);
    for (id, path) in &relinks {
        t.execute("layer.smartObjects.relinkToFile", json!({"layer": id.0, "path": path}))?;
    }
    let out = join(&folder, &format!("{}.{ext}", crate::file_cmds::sanitize(&name)));
    let pdoc = t.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let warnings = crate::file_cmds::save_doc(&pdoc, &out, None)?;
    Ok(json!({"folder": folder, "document": out, "links": copied.iter().map(|(_, t)| t).collect::<Vec<_>>(), "missing": missing, "warnings": warnings}))
}

// ---------- Paths to Illustrator ----------

/// The document's paths as an Illustrator 3 PostScript file. `which`: "all", "work" or a path
/// name. Coordinates are points (pixels at the document resolution), y up.
pub fn paths_to_ai(doc: &Document, which: &str) -> Result<(String, usize)> {
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    let k = 72.0 / dpi;
    let (w, h) = (f64::from(doc.size.width) * k, f64::from(doc.size.height) * k);
    let mut chosen: Vec<(String, &photocraft_doc::Path)> = Vec::new();
    match which {
        "all" => {
            chosen.extend(doc.paths.iter().map(|p| (p.name.clone(), &p.path)));
            if let Some(wp) = &doc.work_path {
                chosen.push(("Work Path".into(), wp));
            }
        }
        "work" => chosen.extend(doc.work_path.iter().map(|wp| ("Work Path".to_string(), wp))),
        name => chosen.extend(doc.paths.iter().filter(|p| p.name == name).map(|p| (p.name.clone(), &p.path))),
    }
    let mut s = String::new();
    s.push_str("%!PS-Adobe-2.0 EPSF-1.2\n%%Creator: PhotoCraft\n");
    s.push_str(&format!("%%Title: ({})\n", doc.name.replace([')', '('], "_")));
    s.push_str(&format!(
        "%%BoundingBox: 0 0 {} {}\n%%HiResBoundingBox: 0 0 {w:.4} {h:.4}\n%AI3_Cropmarks: 0 0 {w:.4} {h:.4}\n",
        w.ceil() as i64,
        h.ceil() as i64
    ));
    s.push_str("%%DocumentProcessColors: Black\n%%EndComments\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n");
    let pt = |x: f64, y: f64| (x * k, h - y * k);
    let mut n = 0;
    for (name, path) in &chosen {
        let subs: Vec<_> = path.subpaths.iter().filter(|sp| !sp.knots.is_empty()).collect();
        if subs.is_empty() {
            continue;
        }
        n += 1;
        s.push_str(&format!("%%Note: {name}\n"));
        let compound = subs.len() > 1;
        if compound {
            s.push_str("*u\n");
        }
        for sp in subs {
            let (x, y) = pt(sp.knots[0].anchor.x, sp.knots[0].anchor.y);
            s.push_str(&format!("{x:.4} {y:.4} m\n"));
            for seg in sp.segments() {
                let [a, c1, c2, b] = seg;
                let straight = c1 == a && c2 == b;
                let (bx, by) = pt(b.x, b.y);
                if straight {
                    s.push_str(&format!("{bx:.4} {by:.4} L\n"));
                } else {
                    let (x1, y1) = pt(c1.x, c1.y);
                    let (x2, y2) = pt(c2.x, c2.y);
                    s.push_str(&format!("{x1:.4} {y1:.4} {x2:.4} {y2:.4} {bx:.4} {by:.4} C\n"));
                }
            }
            // Unpainted: `n` closes the path, `N` leaves it open.
            s.push_str(if sp.closed { "n\n" } else { "N\n" });
        }
        if compound {
            s.push_str("*U\n");
        }
    }
    s.push_str("%%PageTrailer\n%%Trailer\n%%EOF\n");
    Ok((s, n))
}

fn paths_to_illustrator(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.pathsToIllustrator";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let which = p.get("paths").and_then(Value::as_str).unwrap_or("all");
    let (text, n) = paths_to_ai(&d.doc, which)?;
    if n == 0 {
        return Err(bad(cmd, format!("no paths match `{which}`")));
    }
    match p.get("path").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        Some(path) => {
            write_file(path, text.as_bytes())?;
            Ok(json!({"path": path, "paths": n}))
        }
        None => Ok(json!({"ai": text, "paths": n})),
    }
}

fn has_paths(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.paths.is_empty() && d.doc.work_path.is_none() { Err("the document has no paths".into()) } else { Ok(()) }
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    const PRINT_PARAMS: &str = r##"{"printerSettings":hex? (Windows driver settings),"printer":name? (default printer),"copies":1..999=1,"paper":"letter|legal|tabloid|a3|a4|a5|4x6|5x7"|[w,h] pt="letter","orientation":"portrait|landscape"="portrait","center":bool=true,"top":in?,"left":in?,"scale":%=100,"scaleToFit":bool=false,"printable":[x,y,w,h] pt? (the driver's printable area from the paper's top-left, as file.print.page reports it; Scale to Fit Media fits it, else the paper minus 18 pt; a native Windows print reads the driver's own instead),"colorHandling":"printerManages|photocraftManages|noColorManagement"="printerManages","printerProfile":profile? (photocraftManages),"intent":"perceptual|relative|saturation|absolute"="relative","bpc":bool=true,"cornerCropMarks":bool,"centerCropMarks":bool,"registrationMarks":bool,"bleed":in=0 (Functions › Bleed: corner crop marks this far inside the image edge, on the trim line),"description":bool,"labels":bool,"emulsionDown":bool=false (mirror the page left to right, for sublimation and other transfer printing),"output":pdf path? (print to PDF; then "send" defaults to false),"send":bool?,"printServer":"host[:port]|socket path"? (default: the system's CUPS; a named server is always printed to over IPP),"dryRun":bool=false (render the PDF, report the backend command, don't spool; write a PDF only at an explicit output path)} → {pdf:path|null (null without explicit output), imageRect, command:diagnostic array|null, sent, spooler:where the job went|null, backend:"lp|windows|ipp|portal"|null}"##;

    vec![
        spec!("file.print", "Print…", &["File"], Some("Cmd+P"), PRINT_PARAMS, native_doc, print),
        CommandSpec {
            id: "file.print.printers",
            label: "Printers",
            menu: &[],
            shortcut: None,
            params: r##"{} → {supported, printers: [{name, default}]} (supported: Windows; elsewhere Print spools with lp)"##,
            enabled: |_| Ok(()),
            journal: false,
            run: printers,
        },
        CommandSpec {
            id: "file.print.page",
            label: "Printer Page",
            menu: &[],
            shortcut: None,
            params: r##"{"printer":name? (default printer),"printerSettings":hex?,"orientation":"portrait|landscape"? (turn the settings, or the printer's own, that way)} → {printer, paper: [w,h] pt, printable: [x,y,w,h] pt from the top-left, dpi: [x,y], printerSettings: hex (only with orientation: the turned settings)} (Windows)"##,
            enabled: |_| Ok(()),
            journal: false,
            run: printer_page,
        },
        CommandSpec {
            id: "file.printers",
            label: "Printers",
            menu: &[],
            shortcut: None,
            params: r##"{} → {printers:[{name, default}], default:name|null} (the system spooler's printers; a name is Print's "printer")"##,
            enabled: crate::commands::always,
            journal: false,
            run: system_printers,
        },
        spec!(
            "file.printOneCopy",
            "Print One Copy",
            &["File"],
            Some("Cmd+Alt+Shift+P"),
            "{} (the last Print settings, one copy; any Print key overrides)",
            native_doc,
            print_one_copy
        ),
        spec!(
            "file.package",
            "Package…",
            &["File"],
            None,
            r##"{"dir":folder,"format":"pcraft|psd|psb"="pcraft"} → {folder, document, links, missing} (copies the document and its linked files into <dir>/<name>/, relinked to Links/)"##,
            native_doc,
            package
        ),
        spec!(
            "file.export.pathsToIllustrator",
            "Paths to Illustrator…",
            &["File", "Export"],
            None,
            r##"{"path":str? (.ai; omit to return the text),"paths":"all|work|<path name>"="all"} → {path, paths}"##,
            has_paths,
            paths_to_illustrator
        ),
    ]
}

#[cfg(test)]
#[path = "print_cmds/tests.rs"]
mod tests;
