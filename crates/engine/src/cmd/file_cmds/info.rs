use super::{EngineError, Result, Session, Value, json};

// ---------- File Info (XMP) ----------

/// The File Info fields we edit, as (JSON key, XMP property, container kind).
const INFO_FIELDS: [(&str, &str, &str); 6] = [
    ("title", "dc:title", "Alt"),
    ("author", "dc:creator", "Seq"),
    ("description", "dc:description", "Alt"),
    ("keywords", "dc:subject", "Bag"),
    ("copyright", "dc:rights", "Alt"),
    ("authorTitle", "photoshop:AuthorsPosition", ""),
];

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.as_bytes().iter().take(13).position(|&b| b == b';').and_then(|e| {
            let c = match &rest[1..e] {
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "amp" => '&',
                n => {
                    let n = n.strip_prefix('#')?;
                    let code = match n.strip_prefix(['x', 'X']) {
                        Some(h) => u32::from_str_radix(h, 16).ok()?,
                        None => n.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, e + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The element `<tag …>…</tag>` (first occurrence) as (start, end, inner).
pub(super) fn find_element<'a>(xmp: &'a str, tag: &str) -> Option<(usize, usize, &'a str)> {
    let open = format!("<{tag}");
    let mut from = 0;
    while let Some(i) = xmp[from..].find(&open).map(|i| i + from) {
        let after = xmp[i + open.len()..].chars().next();
        if matches!(after, Some('>' | ' ' | '/' | '\n' | '\r' | '\t')) {
            let gt = xmp[i..].find('>')? + i;
            if xmp[..gt].ends_with('/') {
                return Some((i, gt + 1, ""));
            }
            let close = format!("</{tag}>");
            let c = xmp[gt..].find(&close)? + gt;
            return Some((i, c + close.len(), &xmp[gt + 1..c]));
        }
        from = i + open.len();
    }
    None
}

/// `rdf:li` values of a container (or the element text for simple properties).
fn li_values(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = inner;
    while let Some((a, b, v)) = find_element(rest, "rdf:li") {
        out.push(xml_unescape(v.trim()));
        rest = &rest[b..];
        let _ = a;
    }
    if out.is_empty() && !inner.contains('<') && !inner.trim().is_empty() {
        out.push(xml_unescape(inner.trim()));
    }
    out
}

/// Attribute form `prop="value"` on an `rdf:Description`.
fn find_attr<'a>(xmp: &'a str, prop: &str) -> Option<(usize, usize, &'a str)> {
    let key = format!(" {prop}=\"");
    let i = xmp.find(&key)?;
    let v0 = i + key.len();
    let v1 = xmp[v0..].find('"')? + v0;
    Some((i, v1 + 1, &xmp[v0..v1]))
}

/// File Info fields read from an XMP packet.
pub fn read_file_info(xmp: Option<&str>) -> Value {
    let mut m = serde_json::Map::new();
    let x = xmp.unwrap_or("");
    for (key, prop, kind) in INFO_FIELDS {
        let vals = match find_element(x, prop) {
            Some((_, _, inner)) => li_values(inner),
            None => find_attr(x, prop).map(|(_, _, v)| vec![xml_unescape(v)]).unwrap_or_default(),
        };
        let v = if kind == "Bag" {
            json!(vals)
        } else if kind == "Seq" {
            json!(vals.join("; "))
        } else {
            json!(vals.into_iter().next().unwrap_or_default())
        };
        m.insert(key.into(), v);
    }
    let marked =
        find_element(x, "xmpRights:Marked").map(|(_, _, v)| v.trim().to_string()).or_else(|| find_attr(x, "xmpRights:Marked").map(|(_, _, v)| v.to_string()));
    m.insert(
        "copyrightStatus".into(),
        json!(match marked.as_deref() {
            Some("True") => "copyrighted",
            Some("False") => "publicDomain",
            _ => "unknown",
        }),
    );
    let url = find_element(x, "xmpRights:WebStatement")
        .map(|(_, _, v)| xml_unescape(v.trim()))
        .or_else(|| find_attr(x, "xmpRights:WebStatement").map(|(_, _, v)| xml_unescape(v)));
    m.insert("copyrightUrl".into(), json!(url.unwrap_or_default()));
    Value::Object(m)
}

/// File Info's read-only Camera Data, as (result key, English label) in display order.
pub const CAMERA_FIELDS: [(&str, &str); 9] = [
    ("make", "Make"),
    ("model", "Model"),
    ("dateTaken", "Date Taken"),
    ("exposure", "Exposure"),
    ("fNumber", "F-Stop"),
    ("iso", "ISO Speed"),
    ("focalLength", "Focal Length"),
    ("focalLength35mm", "Focal Length (35 mm)"),
    ("lens", "Lens"),
];

/// `v` with at most `decimals` decimals and no trailing zeros (33.10 → "33.1", 8.0 → "8").
fn short_num(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// `v` to three significant digits, with no trailing zeros (0.031714 → "0.0317", 0.4 → "0.4").
/// Two decimals threw away most of a slow shutter speed: a stored 0.031714 s printed as "0.03 s",
/// which is 5 % off, and a stored 0.0215 s printed as "0.02 s", 7 % off (#6958).
fn sig3(v: f64) -> String {
    let magnitude = v.abs().log10().floor();
    // Clamped at both ends: a value at the large end must not ask for a negative field width, and
    // at the small end the field is widened rather than truncated, so a positive exposure never
    // reads as "0 s". 320 is beyond f64's decimal range, so it always terminates.
    let decimals = if magnitude.is_finite() { (2.0 - magnitude).clamp(0.0, 320.0) } else { 0.0 };
    short_num(v, decimals as usize)
}

/// Exposure time as cameras print it: "1/250 s" for fractions of a second, "0.4 s" or "2 s"
/// when the reciprocal isn't a whole number or the exposure is a second or longer.
pub(super) fn exposure_text(t: f64) -> Option<String> {
    if !(t.is_finite() && t > 0.0) {
        return None;
    }
    let r = 1.0 / t;
    let n = r.round();
    // Below 1/10 s the reciprocal is above 10, so its nearest whole number puts the shown time
    // within 5 % of the stored one, and is the form a photographer expects ("1/32 s"). The test
    // used to be relative to the reciprocal, so it shrank as the exposure slowed: under 1/50 s it
    // fell below half a rounding step and nearly every fraction fell through to the decimal form
    // (#6958). Above 1/10 s the fraction is only right when the reciprocal is very nearly whole
    // (1/2 s, 1/4 s) — 0.4 s stays "0.4 s" rather than becoming "1/3 s".
    let fraction = n.is_finite() && (1.0..=1e9).contains(&n) && (t < 0.1 || (t < 1.0 && n > 1.0 && (r - n).abs() <= r * 0.01));
    if fraction {
        return Some(format!("1/{} s", n as u64));
    }
    Some(format!("{} s", sig3(t)))
}

/// EXIF (`2024:02:17 12:40:19`) or XMP (`2024-02-17T12:40:19.00+01:00`) dates as
/// `2024-02-17 12:40:19`; anything else is shown as stored.
fn date_text(s: &str) -> String {
    let b = s.trim().as_bytes();
    let digits = |r: &[usize]| r.iter().all(|&i| b.get(i).is_some_and(u8::is_ascii_digit));
    let sep = |i: usize, set: &[u8]| b.get(i).is_some_and(|c| set.contains(c));
    let ch = |i: usize| b.get(i).map_or('0', |&c| c as char);
    let part = |r: std::ops::Range<usize>| r.map(ch).collect::<String>();
    if !(digits(&[0, 1, 2, 3, 5, 6, 8, 9]) && sep(4, b":-") && sep(7, b":-")) {
        return s.trim().to_string();
    }
    let date = format!("{}-{}-{}", part(0..4), part(5..7), part(8..10));
    if digits(&[11, 12, 14, 15, 17, 18]) && sep(10, b" T") && sep(13, b":") && sep(16, b":") { format!("{date} {}", part(11..19)) } else { date }
}

/// A single XMP value: the first `rdf:li` of a container, element text or attribute.
fn xmp_value(x: &str, prop: &str) -> Option<String> {
    let v = match find_element(x, prop) {
        Some((_, _, inner)) => li_values(inner).into_iter().next(),
        None => find_attr(x, prop).map(|(_, _, v)| xml_unescape(v)),
    }?;
    let v = v.trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// An XMP number: a rational "331/10" or a decimal.
fn xmp_number(x: &str, prop: &str) -> Option<f64> {
    let v = xmp_value(x, prop)?;
    let n = match v.split_once('/') {
        Some((a, b)) => {
            let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
            (b != 0.0).then(|| a / b)?
        }
        None => v.parse::<f64>().ok()?,
    };
    n.is_finite().then_some(n)
}

/// File Info's read-only Camera Data (keys of [`CAMERA_FIELDS`], present ones only), formatted
/// for display. Read from the EXIF block (JPEG APP1, PSD resource 1058, …); properties it lacks
/// fall back to the XMP `tiff:`/`exif:`/`aux:` copies (files without an EXIF block).
pub fn camera_data(exif: Option<&[u8]>, xmp: Option<&str>) -> Value {
    let e = exif.map(photocraft_algo::exif::read).unwrap_or_default();
    let x = xmp.unwrap_or("");
    let text = |v: Option<String>, props: &[&str]| v.or_else(|| props.iter().find_map(|p| xmp_value(x, p)));
    let num =
        |v: Option<f64>, props: &[&str]| v.filter(|v| v.is_finite() && *v > 0.0).or_else(|| props.iter().find_map(|p| xmp_number(x, p))).filter(|v| *v > 0.0);
    let values = [
        text(e.make, &["tiff:Make"]),
        text(e.model, &["tiff:Model"]),
        text(e.date_taken, &["exif:DateTimeOriginal"]).map(|d| date_text(&d)),
        num(e.exposure_time, &["exif:ExposureTime"]).and_then(exposure_text),
        num(e.f_number, &["exif:FNumber"]).map(|n| format!("f/{}", short_num(n, 1))),
        num(e.iso, &["exif:ISOSpeedRatings", "exifEX:PhotographicSensitivity"]).map(|n| format!("ISO {}", n.round())),
        num(e.focal_length, &["exif:FocalLength"]).map(|f| format!("{} mm", short_num(f, 1))),
        num(e.focal_length_35mm, &["exif:FocalLengthIn35mmFilm"]).map(|f| format!("{} mm", short_num(f, 1))),
        text(e.lens, &["exifEX:LensModel", "aux:Lens"]),
    ];
    let mut m = serde_json::Map::new();
    for ((key, _), v) in CAMERA_FIELDS.into_iter().zip(values) {
        // Hostile files can carry megabytes of "lens name"; the dialog shows a line.
        if let Some(v) = v.map(|v| v.chars().take(200).collect::<String>()).filter(|v| !v.is_empty()) {
            m.insert(key.into(), json!(v));
        }
    }
    Value::Object(m)
}

/// Rewrites only changed File Info fields of an XMP packet (creating one when there is none),
/// keeping the other properties verbatim. Edited properties move into their own
/// `rdf:Description` (several descriptions per packet are valid XMP).
pub fn write_file_info(xmp: Option<&str>, info: &Value) -> String {
    let cur = read_file_info(xmp);
    // The dialog submits all fields, including unchanged language alternatives.
    let changed = |key: &str| info.get(key).is_some_and(|v| cur.get(key) != Some(v));
    let mut x = xmp.filter(|s| s.contains("</rdf:RDF>")).map(str::to_string).unwrap_or_else(|| {
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>".to_string()
    });
    for (key, prop, _) in INFO_FIELDS.into_iter().chain([("copyrightStatus", "xmpRights:Marked", ""), ("copyrightUrl", "xmpRights:WebStatement", "")]) {
        if !changed(key) {
            continue;
        }
        while let Some((a, b, _)) = find_element(&x, prop) {
            x.replace_range(a..b, "");
        }
        while let Some((a, b, _)) = find_attr(&x, prop) {
            x.replace_range(a..b, "");
        }
    }
    let get = |k: &str| info.get(k).filter(|_| changed(k)).cloned().unwrap_or(Value::Null);
    let mut body = String::new();
    for (key, prop, kind) in INFO_FIELDS {
        let vals: Vec<String> = match get(key) {
            Value::Array(a) => a.iter().filter_map(|v| v.as_str()).map(str::to_string).filter(|v| !v.is_empty()).collect(),
            Value::String(s) if kind == "Bag" || kind == "Seq" => s.split([';', ',']).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect(),
            Value::String(s) if !s.is_empty() => vec![s],
            _ => Vec::new(),
        };
        if vals.is_empty() {
            continue;
        }
        let items: String = vals
            .iter()
            .map(|v| {
                if kind == "Alt" { format!("<rdf:li xml:lang=\"x-default\">{}</rdf:li>", xml_escape(v)) } else { format!("<rdf:li>{}</rdf:li>", xml_escape(v)) }
            })
            .collect();
        if kind.is_empty() {
            body.push_str(&format!("   <{prop}>{}</{prop}>\n", xml_escape(&vals[0])));
        } else {
            body.push_str(&format!("   <{prop}><rdf:{kind}>{items}</rdf:{kind}></{prop}>\n"));
        }
    }
    match get("copyrightStatus").as_str() {
        Some("copyrighted") => body.push_str("   <xmpRights:Marked>True</xmpRights:Marked>\n"),
        Some("publicDomain") => body.push_str("   <xmpRights:Marked>False</xmpRights:Marked>\n"),
        _ => {}
    }
    if let Some(u) = get("copyrightUrl").as_str().filter(|u| !u.is_empty()) {
        body.push_str(&format!("   <xmpRights:WebStatement>{}</xmpRights:WebStatement>\n", xml_escape(u)));
    }
    if !body.is_empty() {
        let desc = format!(
            "  <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\" xmlns:xmpRights=\"http://ns.adobe.com/xap/1.0/rights/\">\n{body}  </rdf:Description>\n "
        );
        let at = x.rfind("</rdf:RDF>").unwrap_or(x.len());
        x.insert_str(at, &desc);
    }
    x
}

pub(super) fn file_info(s: &mut Session, p: &Value) -> Result<Value> {
    let keys = ["title", "author", "authorTitle", "description", "keywords", "copyright", "copyrightStatus", "copyrightUrl"];
    let edits = p.as_object().is_some_and(|m| m.keys().any(|k| keys.contains(&k.as_str())));
    if edits {
        let xmp = s.active().ok_or(EngineError::NoDocument)?.doc.metadata.xmp.clone();
        let new = write_file_info(xmp.as_deref(), p);
        s.edit("File Info", |doc, _| {
            doc.metadata.xmp = Some(new.into());
            Ok(())
        })?;
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let mut info = read_file_info(d.doc.metadata.xmp.as_deref());
    if let Value::Object(m) = &mut info {
        m.insert("camera".into(), camera_data(d.doc.metadata.exif.as_deref().map(Vec::as_slice), d.doc.metadata.xmp.as_deref()));
    }
    Ok(info)
}
