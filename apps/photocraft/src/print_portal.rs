//! Printing through the XDG Print portal (`org.freedesktop.portal.Print`): the system print
//! dialog, which a sandboxed app can reach without a CUPS socket. The engine prints over IPP
//! when it can reach a print server (`photocraft-ipp`); this is its fallback when it cannot, and
//! the only route with `PHOTOCRAFT_PRINT_PORTAL=1` (`0` disables it). `PreparePrint` shows the
//! dialog seeded with the job's copies and paper; `Print` hands it the PDF. Both answer through a
//! `Request` object's `Response` signal, which this module waits for on its own thread so the
//! window keeps painting while the dialog is open; the outcome goes to the log.
//! https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Print.html

#[cfg(target_os = "linux")]
use photocraft_engine::print_cmds::{PrintJob, Spooler};

/// The spooler to install on the session, unless `PHOTOCRAFT_PRINT_PORTAL=0`. The portal itself
/// is looked up per job, so a missing one costs nothing at start-up.
#[cfg(target_os = "linux")]
pub fn spooler() -> Option<Spooler> {
    let setting = std::env::var("PHOTOCRAFT_PRINT_PORTAL").ok();
    if setting.as_deref() == Some("0") {
        return None;
    }
    Some(Spooler { name: "portal", preferred: setting.as_deref() == Some("1"), run: std::sync::Arc::new(print) })
}

/// Opens the system print dialog for the job and returns as soon as it is up.
#[cfg(target_os = "linux")]
fn print(job: &PrintJob) -> Result<String, String> {
    let conn = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.method_timeout(std::time::Duration::from_secs(5)).build())
        .map_err(|e| format!("no session bus for the print portal ({e}); print to PDF instead"))?;
    let proxy = portal(&conn)?;
    let version: u32 = proxy.get_property("version").map_err(|e| format!("no print portal on this desktop ({e}); print to PDF instead"))?;
    // The document goes to a private temporary file (0600, created atomically by `tempfile`),
    // not a guessable name in a shared /tmp. The guard owns the file: it lives on the portal
    // thread until the dialog is done, and a failure to start that thread drops it here.
    let mut file = tempfile::Builder::new().prefix("photocraft-print-").suffix(".pdf").tempfile().map_err(|e| format!("cannot create the print file: {e}"))?;
    std::io::Write::write_all(&mut file, &job.pdf).map_err(|e| format!("cannot write the print file: {e}"))?;
    let (title, settings, page_setup) = (job.title.clone(), settings(job), page_setup(job));
    std::thread::Builder::new()
        .name("print-portal".into())
        .spawn(move || {
            let outcome = dialog(&conn, &title, &settings, &page_setup, file.path());
            drop(file);
            match outcome {
                Ok(what) => log::info!("print portal (v{version}): {what}"),
                Err(e) => log::warn!("print portal (v{version}): {e}"),
            }
        })
        .map_err(|e| format!("cannot start the print thread: {e}"))?;
    Ok("the system print dialog".into())
}

#[cfg(target_os = "linux")]
fn portal(conn: &zbus::blocking::Connection) -> Result<zbus::blocking::Proxy<'static>, String> {
    zbus::blocking::Proxy::new(conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Print")
        .map_err(|e| e.to_string())
}

/// `PreparePrint` then `Print`, each waited for on its request's `Response` signal.
#[cfg(target_os = "linux")]
fn dialog(
    conn: &zbus::blocking::Connection,
    title: &str,
    settings: &[(&'static str, Seed)],
    page_setup: &[(&'static str, Seed)],
    pdf: &std::path::Path,
) -> Result<String, String> {
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
    type Dict<'a> = HashMap<&'a str, Value<'a>>;
    fn as_dict(pairs: &[(&'static str, Seed)]) -> Dict<'static> {
        pairs
            .iter()
            .map(|(k, v)| {
                let value = match v {
                    Seed::Text(t) => Value::from(t.clone()),
                    Seed::Mm(mm) => Value::from(*mm),
                };
                (*k, value)
            })
            .collect()
    }
    let proxy = portal(conn)?;
    let sender = conn.unique_name().map(|n| n.as_str().to_string()).ok_or("the bus connection has no name")?;
    let wait = |token: &str| -> Result<zbus::blocking::proxy::SignalIterator<'static>, String> {
        let path = request_path(&sender, token);
        let request = zbus::blocking::Proxy::new(conn, "org.freedesktop.portal.Desktop", path, "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
        request.receive_signal("Response").map_err(|e| e.to_string())
    };
    let response = |mut signals: zbus::blocking::proxy::SignalIterator<'static>| -> Result<HashMap<String, OwnedValue>, String> {
        let message = signals.next().ok_or("the print portal went away")?;
        let (code, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize().map_err(|e| format!("odd portal reply: {e}"))?;
        match code {
            0 => Ok(results),
            1 => Err("cancelled in the print dialog".into()),
            _ => Err("the print dialog failed".into()),
        }
    };

    let token = format!("photocraft_prepare_{}", nonce());
    let signals = wait(&token)?;
    let mut options: Dict<'_> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    let _: OwnedObjectPath =
        proxy.call("PreparePrint", &("", title, as_dict(settings), as_dict(page_setup), options)).map_err(|e| format!("PreparePrint failed: {e}"))?;
    let results = response(signals)?;
    let prepared = results.get("token").and_then(|v| u32::try_from(v.clone()).ok()).ok_or("the print dialog gave no token")?;

    let file = std::fs::File::open(pdf).map_err(|e| format!("cannot reopen the print file: {e}"))?;
    let token = format!("photocraft_print_{}", nonce());
    let signals = wait(&token)?;
    let mut options: Dict<'_> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("token", Value::from(prepared));
    let fd = zbus::zvariant::Fd::from(std::os::fd::AsFd::as_fd(&file));
    let _: OwnedObjectPath = proxy.call("Print", &("", title, fd, options)).map_err(|e| format!("Print failed: {e}"))?;
    response(signals)?;
    Ok("printed through the system print dialog".into())
}

#[cfg(target_os = "linux")]
fn nonce() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    t ^ N.fetch_add(1, Ordering::Relaxed)
}

/// The request object the portal answers on: the sender's unique name without its colon and
/// with dots as underscores, then the handle token.
fn request_path(unique_name: &str, token: &str) -> String {
    format!("/org/freedesktop/portal/desktop/request/{}/{token}", unique_name.trim_start_matches(':').replace('.', "_"))
}

/// A value seeded into the dialog: GtkPrintSettings are strings; a GtkPageSetup's sizes are
/// doubles in millimetres (`gtk_paper_size_new_from_gvariant` ignores anything else).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum Seed {
    Text(String),
    Mm(f64),
}

fn text(s: &str) -> Seed {
    Seed::Text(s.to_string())
}

/// The standard paper sizes, as (PWG name, PPD name, width mm, height mm), portrait. GTK's
/// print settings take the PWG name; its page setup finds the size by the PPD name.
const PAPERS: [(&str, &str, f64, f64); 8] = [
    ("na_letter", "Letter", 215.9, 279.4),
    ("na_legal", "Legal", 215.9, 355.6),
    ("na_ledger", "Ledger", 279.4, 431.8),
    ("iso_a3", "A3", 297.0, 420.0),
    ("iso_a4", "A4", 210.0, 297.0),
    ("iso_a5", "A5", 148.0, 210.0),
    ("na_index-4x6", "4x6", 101.6, 152.4),
    ("na_5x7", "5x7", 127.0, 177.8),
];

const MM_PER_PT: f64 = 25.4 / 72.0;

type Paper = (&'static str, &'static str, f64, f64);

/// The job's paper: its row in `PAPERS` when it is a standard size, and the portrait size in
/// millimetres.
fn paper(paper_pt: (f64, f64)) -> (Option<&'static Paper>, f64, f64) {
    let (w, h) = (paper_pt.0.min(paper_pt.1) * MM_PER_PT, paper_pt.0.max(paper_pt.1) * MM_PER_PT);
    let row = PAPERS.iter().find(|(_, _, pw, ph)| (pw - w).abs() < 1.0 && (ph - h).abs() < 1.0);
    (row, w, h)
}

fn orientation(landscape: bool) -> Seed {
    text(if landscape { "landscape" } else { "portrait" })
}

/// `PreparePrint`'s settings (GtkPrintSettings keys, string values).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn settings_for(copies: u32, paper_pt: (f64, f64), landscape: bool) -> Vec<(&'static str, Seed)> {
    let (row, w, h) = paper(paper_pt);
    let mut v = vec![("n-copies", text(&copies.to_string())), ("orientation", orientation(landscape))];
    match row {
        Some((pwg, _, _, _)) => v.push(("paper-format", text(pwg))),
        None => v.extend([("paper-width", text(&format!("{w:.2}"))), ("paper-height", text(&format!("{h:.2}")))]),
    }
    v
}

/// `PreparePrint`'s page setup (GtkPageSetup keys): the size in millimetres, found by its PPD
/// name when it has one, else a custom size named by its dimensions.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn page_setup_for(paper_pt: (f64, f64), landscape: bool) -> Vec<(&'static str, Seed)> {
    let (row, w, h) = paper(paper_pt);
    let mut v = vec![("Orientation", orientation(landscape)), ("Width", Seed::Mm(w)), ("Height", Seed::Mm(h))];
    match row {
        Some((_, ppd, _, _)) => v.push(("PPDName", text(ppd))),
        None => v.push(("Name", text(&format!("custom_{w:.0}x{h:.0}mm")))),
    }
    v
}

#[cfg(target_os = "linux")]
fn settings(job: &PrintJob) -> Vec<(&'static str, Seed)> {
    settings_for(job.copies, job.paper, job.landscape)
}

#[cfg(target_os = "linux")]
fn page_setup(job: &PrintJob) -> Vec<(&'static str, Seed)> {
    page_setup_for(job.paper, job.landscape)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_path_follows_the_portal_rule() {
        assert_eq!(request_path(":1.42", "photocraft_prepare_7"), "/org/freedesktop/portal/desktop/request/1_42/photocraft_prepare_7");
    }

    #[test]
    fn standard_papers_get_their_pwg_and_ppd_names_and_custom_ones_their_size() {
        // A4 landscape as the engine lays it out: width > height, still A4.
        let s = settings_for(2, (841.89, 595.28), true);
        assert_eq!(s, vec![("n-copies", text("2")), ("orientation", text("landscape")), ("paper-format", text("iso_a4"))]);
        let p = page_setup_for((841.89, 595.28), true);
        assert_eq!(p[0], ("Orientation", text("landscape")));
        let Seed::Mm(w) = p[1].1 else { panic!("width is millimetres") };
        let Seed::Mm(h) = p[2].1 else { panic!("height is millimetres") };
        assert!((w - 210.0).abs() < 0.01 && (h - 297.0).abs() < 0.01, "{w} x {h}");
        assert_eq!(p[3], ("PPDName", text("A4")));
        assert_eq!(settings_for(1, (612.0, 792.0), false)[2], ("paper-format", text("na_letter")));
        assert_eq!(page_setup_for((612.0, 792.0), false)[3], ("PPDName", text("Letter")));
        assert_eq!(settings_for(1, (288.0, 432.0), false)[2], ("paper-format", text("na_index-4x6")));
        // 100 x 200 mm is nobody's standard.
        let custom = settings_for(1, (283.46, 566.93), false);
        assert_eq!(&custom[2..], &[("paper-width", text("100.00")), ("paper-height", text("200.00"))]);
        assert_eq!(page_setup_for((283.46, 566.93), false)[3], ("Name", text("custom_100x200mm")));
    }
}
