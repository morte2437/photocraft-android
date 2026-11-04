//! Batch output names are planned before opening or writing files. Names substituted from an
//! input are literal: braces in a source filename must never be interpreted as template tokens.

use super::{BatchInput, EngineError, Result, Value, join, stem};

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "file.automate.batch".into(), msg: message.into() }
}

enum Part<'a> {
    Literal(&'a str),
    Name(NameCase),
    Serial(usize),
    Letter(bool),
    Date(DateFormat),
    Ext(bool),
}

#[derive(Clone, Copy)]
enum NameCase {
    AsIs,
    Upper,
    Title,
}

#[derive(Clone, Copy)]
enum DateFormat {
    Iso,
    Mmddyy,
    Mmdd,
    Yyyymmdd,
    Yymmdd,
    Yyddmm,
    Ddmmyy,
}

#[derive(Clone, Copy)]
struct Date {
    year: u32,
    month: u32,
    day: u32,
}

impl Date {
    fn from_iso(value: &str) -> Option<Self> {
        Some(Self { year: value.get(..4)?.parse().ok()?, month: value.get(5..7)?.parse().ok()?, day: value.get(8..10)?.parse().ok()? })
    }

    fn format(self, kind: DateFormat) -> String {
        let (year, month, day, short) = (self.year, self.month, self.day, self.year % 100);
        match kind {
            DateFormat::Iso => format!("{year:04}-{month:02}-{day:02}"),
            DateFormat::Mmddyy => format!("{month:02}{day:02}{short:02}"),
            DateFormat::Mmdd => format!("{month:02}{day:02}"),
            DateFormat::Yyyymmdd => format!("{year:04}{month:02}{day:02}"),
            DateFormat::Yymmdd => format!("{short:02}{month:02}{day:02}"),
            DateFormat::Yyddmm => format!("{short:02}{day:02}{month:02}"),
            DateFormat::Ddmmyy => format!("{day:02}{month:02}{short:02}"),
        }
    }
}

/// Serial letters use the one-based count; recorded components pass the input index (#4579 by @FabioLissi).
fn letters(mut serial: u64, upper: bool) -> String {
    let mut bytes = Vec::new();
    while serial > 0 {
        serial -= 1;
        let letter = b'a' + (serial % 26) as u8;
        bytes.push(if upper { letter.to_ascii_uppercase() } else { letter });
        serial /= 26;
    }
    bytes.reverse();
    String::from_utf8(bytes).unwrap_or_default()
}

/// Photoshop's component identifiers and the structured native naming choices.
pub(crate) const COMPONENTS: &[(&str, &str)] = &[
    ("Nm  ", "name"),
    ("lowerCase", "nameLower"),
    ("upperCase", "nameUpper"),
    ("oneDigit", "serial1"),
    ("twoDigit", "serial2"),
    ("threeDigit", "serial3"),
    ("fourDigit", "serial4"),
    ("lowerCaseSerial", "letter"),
    ("upperCaseSerial", "LETTER"),
    ("mmddyy", "mmddyy"),
    ("mmdd", "mmdd"),
    ("yyyymmdd", "yyyymmdd"),
    ("yymmdd", "yymmdd"),
    ("yyddmm", "yyddmm"),
    ("ddmmyy", "ddmmyy"),
    ("ddmm", "ddmm"),
    ("lowerCaseExtension", "extension"),
    ("upperCaseExtension", "EXTENSION"),
];

/// Bound structured fields before file processing; literals never pass through the legacy template parser.
fn components(params: &Value) -> Result<Option<&[Value]>> {
    let Some(value) = params.get("namingComponents").filter(|v| !v.is_null()) else { return Ok(None) };
    let list = value.as_array().filter(|a| (1..=6).contains(&a.len())).ok_or_else(|| bad("namingComponents needs one to six components"))?;
    for item in list {
        let valid = item.as_object().is_some_and(|o| {
            o.len() == 1
                && (o.get("text").and_then(Value::as_str).is_some_and(|t| t.len() <= 255)
                    || o.get("component").and_then(Value::as_str).is_some_and(|kind| COMPONENTS.iter().any(|(_, native)| kind == *native)))
        });
        if !valid {
            return Err(bad("each naming component needs one known component or literal text of at most 255 bytes"));
        }
    }
    Ok(Some(list))
}

/// Read explicit portability flags; path-containment and host-filesystem checks remain mandatory.
fn compatibility(params: &Value) -> Result<(bool, bool, bool)> {
    let Some(value) = params.get("compatibility").filter(|v| !v.is_null()) else { return Ok((false, false, false)) };
    let flags = value.as_object().ok_or_else(|| bad("compatibility must be an object"))?;
    if flags.iter().any(|(key, value)| !["macintosh", "windows", "unix"].contains(&key.as_str()) || !value.is_boolean()) {
        return Err(bad("compatibility accepts only macintosh, windows and unix booleans"));
    }
    Ok((
        flags.get("macintosh").and_then(Value::as_bool).unwrap_or(false),
        flags.get("windows").and_then(Value::as_bool).unwrap_or(false),
        flags.get("unix").and_then(Value::as_bool).unwrap_or(false),
    ))
}

/// Construct the entire filename in recorded order, adding a dot only for the final extension component.
fn expand_components(parts: &[Value], name: &str, serial: u32, index: usize, ext: &str, date: chrono::NaiveDate) -> Result<String> {
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            out.push_str(text);
        } else {
            let kind = part.get("component").and_then(Value::as_str).ok_or_else(|| bad("missing naming component"))?;
            let value = match kind {
                "name" => name.to_string(),
                "nameLower" => name.to_lowercase(),
                "nameUpper" => name.to_uppercase(),
                "serial1" => format!("{serial}"),
                "serial2" => format!("{serial:02}"),
                "serial3" => format!("{serial:03}"),
                "serial4" => format!("{serial:04}"),
                "letter" | "LETTER" => {
                    letters(u64::try_from(index).ok().and_then(|n| n.checked_add(1)).ok_or_else(|| bad("too many serial letters"))?, kind == "LETTER")
                }
                "extension" | "EXTENSION" => {
                    format!("{}{}", if i + 1 == parts.len() { "." } else { "" }, if kind == "EXTENSION" { ext.to_uppercase() } else { ext.to_lowercase() })
                }
                "mmddyy" => date.format("%m%d%y").to_string(),
                "mmdd" => date.format("%m%d").to_string(),
                "yyyymmdd" => date.format("%Y%m%d").to_string(),
                "yymmdd" => date.format("%y%m%d").to_string(),
                "yyddmm" => date.format("%y%d%m").to_string(),
                "ddmmyy" => date.format("%d%m%y").to_string(),
                "ddmm" => date.format("%d%m").to_string(),
                _ => return Err(bad("unknown naming component")),
            };
            out.push_str(&value);
        }
        if out.len() > 255 {
            return Err(bad("naming must produce at most 255 bytes"));
        }
    }
    Ok(out)
}

fn parse(template: &str) -> Result<Vec<Part<'_>>> {
    let mut parts = Vec::new();
    let mut rest = template;
    while let Some(at) = rest.find(['{', '}']) {
        parts.push(Part::Literal(&rest[..at]));
        rest = &rest[at..];
        if rest.starts_with('}') {
            return Err(bad("naming: unmatched closing brace"));
        }
        let end = rest.find('}').ok_or_else(|| bad("naming: unclosed token"))?;
        parts.push(match &rest[1..end] {
            "name" => Part::Name(NameCase::AsIs),
            "NAME" => Part::Name(NameCase::Upper),
            "Name" => Part::Name(NameCase::Title),
            "serial" => Part::Serial(3),
            "letter" => Part::Letter(false),
            "LETTER" => Part::Letter(true),
            "date" => Part::Date(DateFormat::Iso),
            "date:mmddyy" => Part::Date(DateFormat::Mmddyy),
            "date:mmdd" => Part::Date(DateFormat::Mmdd),
            "date:yyyymmdd" => Part::Date(DateFormat::Yyyymmdd),
            "date:yymmdd" => Part::Date(DateFormat::Yymmdd),
            "date:yyddmm" => Part::Date(DateFormat::Yyddmm),
            "date:ddmmyy" => Part::Date(DateFormat::Ddmmyy),
            "ext" => Part::Ext(false),
            "EXT" => Part::Ext(true),
            token => match token.strip_prefix("serial:").and_then(|value| value.parse::<usize>().ok()) {
                Some(width) if (1..=4).contains(&width) => Part::Serial(width),
                _ => return Err(bad(format!("naming: unknown token {{{token}}}"))),
            },
        });
        rest = &rest[end + 1..];
    }
    parts.push(Part::Literal(rest));
    Ok(parts)
}

fn expand(parts: &[Part<'_>], name: &str, ext: &str, serial: u32, date: Date) -> String {
    let mut out = String::new();
    for (index, part) in parts.iter().enumerate() {
        match part {
            Part::Literal(s) => out.push_str(s),
            Part::Name(NameCase::AsIs) => out.push_str(name),
            Part::Name(NameCase::Upper) => out.push_str(&name.to_uppercase()),
            Part::Name(NameCase::Title) => {
                let mut chars = name.chars();
                if let Some(first) = chars.next() {
                    out.extend(first.to_uppercase());
                    out.push_str(&chars.as_str().to_lowercase());
                }
            }
            Part::Serial(width) => out.push_str(&format!("{serial:0width$}")),
            Part::Letter(upper) => out.push_str(&letters(u64::from(serial), *upper)),
            Part::Date(kind) => out.push_str(&date.format(*kind)),
            Part::Ext(upper) => {
                // The reference app adds the separator only for the final extension component.
                // A literal dot in the template already supplies that separator.
                if !parts[index + 1..].iter().any(|part| !matches!(part, Part::Literal(""))) && !out.ends_with('.') {
                    out.push('.');
                }
                if *upper {
                    out.push_str(&ext.to_uppercase());
                } else {
                    out.push_str(ext);
                }
            }
        }
    }
    out
}

/// An absent or empty template preserves the old filename path exactly. An explicit template
/// determines the whole filename. Recorded components retain their separate format choice.
pub fn plan(inputs: &[BatchInput], output: &str, format: &str, params: &Value) -> Result<Option<Vec<String>>> {
    let structured = components(params)?;
    let compat = compatibility(params)?;
    let flat = match params.get("flatOutput") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(v)) => *v,
        _ => return Err(bad("flatOutput must be a boolean")),
    };
    if flat && structured.is_none() {
        return Err(bad("flatOutput needs namingComponents"));
    }
    let start = match params.get("startingSerial") {
        None => 1,
        Some(n) => {
            n.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|&n| n > 0).ok_or_else(|| bad("startingSerial must be an integer from 1 to 4294967295"))?
        }
    };
    let template = match params.get("naming") {
        None => "",
        Some(Value::String(s)) => s,
        Some(_) => return Err(bad("naming must be a string")),
    };
    if structured.is_some() && !template.is_empty() {
        return Err(bad("use naming or namingComponents, not both"));
    }
    if params.get("compatibility").is_some_and(|v| !v.is_null()) && structured.is_none() {
        return Err(bad("compatibility needs namingComponents"));
    }
    if template.is_empty() && structured.is_none() {
        return Ok(None);
    }
    if template.len() > 255 {
        return Err(bad("naming must be at most 255 bytes"));
    }
    let parts = parse(template)?;
    let has_serial = parts.iter().any(|p| matches!(p, Part::Serial(_) | Part::Letter(_)))
        || structured.is_some_and(|list| {
            list.iter().any(|p| p.get("component").and_then(Value::as_str).is_some_and(|kind| matches!(kind, "serial1" | "serial2" | "serial3" | "serial4")))
        });
    let timestamp = crate::analysis_cmds::now_iso();
    let date = Date::from_iso(&timestamp).ok_or_else(|| bad("naming: the current UTC date is unavailable"))?;
    let calendar = if structured.is_some() {
        Some(chrono::NaiveDate::parse_from_str(timestamp.get(..10).unwrap_or(""), "%Y-%m-%d").map_err(|_| bad("naming: current UTC date unavailable"))?)
    } else {
        None
    };
    let mut outputs = Vec::with_capacity(inputs.len());
    for (index, BatchInput { path, rel_dir }) in inputs.iter().enumerate() {
        let serial = if has_serial {
            u32::try_from(index).ok().and_then(|index| start.checked_add(index)).ok_or_else(|| bad("startingSerial plus the input count exceeds 4294967295"))?
        } else {
            start
        };
        let ext = if format == "same" { super::same_format(path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
        let name = if let (Some(components), Some(calendar)) = (structured, calendar) {
            expand_components(components, &stem(path), serial, index, &ext, calendar)?
        } else {
            expand(&parts, &stem(path), &ext, serial, date)
        };
        if name.is_empty() || name.ends_with(['.', ' ']) {
            return Err(bad("naming must produce a nonempty filename stem without a trailing dot or space"));
        }
        if name.len() > 255 || name.chars().any(|c| c.is_control() || "/\\".contains(c)) {
            return Err(bad("naming must produce a filename of at most 255 bytes without path separators or reserved characters"));
        }
        let windows = structured.is_none() || cfg!(windows) || compat.1;
        if windows && (name.contains(['<', '>', ':', '"', '|', '?', '*']) || structured.is_some() && super::is_windows_device_name(&name))
            || compat.0 && name.contains(':')
        {
            return Err(bad("naming is incompatible with the requested platform"));
        }
        // Preserve the recorded structured routing; ordinary native templates use main's flat output.
        let subfolder = if structured.is_some() && !flat { rel_dir.as_str() } else { "" };
        outputs.push(join(&join(output, subfolder), &name));
    }
    // Renaming within the source folder could replace a later input before it is read. Resolve
    // existing paths so aliases such as `source/./001.png` cannot bypass the check. The default
    // (no template) path keeps its existing in-place behavior.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let sources: std::collections::HashSet<_> = inputs.iter().filter_map(|i| std::fs::canonicalize(&i.path).ok()).collect();
        for output in &outputs {
            let identity = super::paths::destination_identity(std::path::Path::new(output))
                .ok_or_else(|| bad(format!("naming output path cannot be resolved safely: {output}")))?;
            if sources.contains(&identity) {
                return Err(bad(format!("naming would replace an input file: {output}; choose a different destination or template")));
            }
        }
    }
    Ok(Some(outputs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Fix the date and input index so naming checks are independent of the clock and serial start.
    #[test]
    fn batch_components_cover_names_serials_letters_and_every_measured_date() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let expected = [
            ("name", "Photo{serial}"),
            ("nameLower", "photo{serial}"),
            ("nameUpper", "PHOTO{SERIAL}"),
            ("serial1", "7"),
            ("serial2", "07"),
            ("serial3", "007"),
            ("serial4", "0007"),
            ("letter", "a"),
            ("LETTER", "A"),
            ("mmddyy", "110126"),
            ("mmdd", "1101"),
            ("yyyymmdd", "20261101"),
            ("yymmdd", "261101"),
            ("yyddmm", "260111"),
            ("ddmmyy", "011126"),
            ("ddmm", "0111"),
            ("extension", ".png"),
            ("EXTENSION", ".PNG"),
        ];
        for (component, want) in expected {
            assert_eq!(expand_components(&[json!({"component":component})], "Photo{serial}", 7, 0, "png", date).unwrap(), want, "{component}");
        }
        assert_eq!(expected.len(), COMPONENTS.len());
        assert_eq!(letters(26, false), "z");
        assert_eq!(letters(27, true), "AA");
        assert_eq!(expand_components(&[json!({"component":"letter"})], "x", 9999, 27, "png", date).unwrap(), "ab");
    }

    /// Literal braces stay literal and an extension adds its dot only in the final position.
    #[test]
    fn batch_extension_position_and_literal_text_determine_the_entire_name() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        for (parts, want) in [
            (json!([{"component":"name"},{"component":"extension"}]), "a.png"),
            (json!([{"component":"name"},{"component":"EXTENSION"}]), "a.PNG"),
            (json!([{"component":"name"},{"component":"extension"},{"component":"serial3"}]), "apng001"),
            (json!([{"component":"name"},{"component":"serial3"}]), "a001"),
            (json!([{"component":"name"},{"component":"extension"},{"text":"x"}]), "apngx"),
            (json!([{"component":"name"},{"component":"extension"},{"component":"extension"}]), "apng.png"),
            (json!([{"component":"name"},{"text":".jpg"}]), "a.jpg"),
        ] {
            assert_eq!(expand_components(parts.as_array().unwrap(), "a", 1, 0, "png", date).unwrap(), want);
        }
    }

    /// Check filename admission before processing, including counter overflow and portability limits.
    #[test]
    fn batch_component_preflight_retains_bounds_platform_checks_and_serial_overflow() {
        let inputs =
            vec![BatchInput { path: "input/Photo.png".into(), rel_dir: "sub".into() }, BatchInput { path: "input/Other.png".into(), rel_dir: String::new() }];
        let mut p = json!({"namingComponents":[{"component":"letter"}],"startingSerial":u32::MAX,"flatOutput":true,
            "compatibility":{"macintosh":true,"windows":true,"unix":true}});
        let outputs = plan(&inputs, "output", "same", &p).unwrap().unwrap();
        assert_eq!(crate::file_cmds::file_name(&outputs[0]), "a");
        assert_eq!(crate::file_cmds::file_name(&outputs[1]), "b");
        assert!(!outputs[0].contains("sub"));
        p["namingComponents"] = json!([{"component":"serial1"}]);
        assert!(plan(&inputs, "output", "same", &p).is_err());
        for components in [
            json!([{"component":"unknown"}]),
            json!([{"component":"name","text":"x"}]),
            json!(vec![json!({"text":"x"}); 7]),
            json!([{"text":"../escape"}]),
            json!([{"text":"CON"}]),
        ] {
            p["startingSerial"] = json!(1);
            p["namingComponents"] = components;
            assert!(plan(&inputs, "output", "same", &p).is_err(), "{p}");
        }
        p["namingComponents"] = json!([{"text":"safe"}]);
        p["compatibility"]["windows"] = json!("yes");
        assert!(plan(&inputs, "output", "same", &p).is_err());
    }

    #[test]
    fn substituted_names_are_literal_and_serials_do_not_wrap_or_truncate() {
        let parts = parse("{date}_{name}_{serial}").unwrap();
        let date = Date { year: 2026, month: 10, day: 13 };
        assert_eq!(expand(&parts, "photo{serial}", "png", 7, date), "2026-10-13_photo{serial}_007");
        assert_eq!(expand(&parts, "photo", "png", 1000, date), "2026-10-13_photo_1000");
        let parts = parse("{NAME}_{Name}_{serial:1}_{letter}{LETTER}_{date:mmddyy}.{EXT}").unwrap();
        assert_eq!(expand(&parts, "my Photo", "png", 27, date), "MY PHOTO_My photo_27_aaAA_101326.PNG");
    }

    #[test]
    fn extension_component_only_adds_a_dot_at_the_end() {
        let date = Date { year: 2026, month: 11, day: 1 };
        for (template, expected) in [("{name}{ext}", "a.png"), ("{name}{ext}{serial}", "apng001"), ("{name}{serial}", "a001"), ("{name}{ext}{ext}", "apng.png")]
        {
            assert_eq!(expand(&parse(template).unwrap(), "a", "png", 1, date), expected, "{template}");
        }
    }
}
