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

fn letters(mut serial: u32, upper: bool) -> String {
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
            Part::Letter(upper) => out.push_str(&letters(serial, *upper)),
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
/// generates one component per input; the export format supplies the extension separately.
pub fn plan(inputs: &[BatchInput], output: &str, format: &str, params: &Value) -> Result<Option<Vec<String>>> {
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
    if template.is_empty() {
        return Ok(None);
    }
    if template.len() > 255 {
        return Err(bad("naming must be at most 255 bytes"));
    }
    let parts = parse(template)?;
    let has_serial = parts.iter().any(|p| matches!(p, Part::Serial(_) | Part::Letter(_)));
    let timestamp = crate::analysis_cmds::now_iso();
    let date = Date::from_iso(&timestamp).ok_or_else(|| bad("naming: the current UTC date is unavailable"))?;
    let mut outputs = Vec::with_capacity(inputs.len());
    for (index, BatchInput { path, .. }) in inputs.iter().enumerate() {
        let serial = if has_serial {
            u32::try_from(index).ok().and_then(|index| start.checked_add(index)).ok_or_else(|| bad("startingSerial plus the input count exceeds 4294967295"))?
        } else {
            start
        };
        let ext = if format == "same" { super::same_format(path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
        let name = expand(&parts, &stem(path), &ext, serial, date);
        if name.is_empty() || name.ends_with(['.', ' ']) || name.len() > 255 || name.chars().any(|c| c.is_control() || "/\\<>:\"|?*".contains(c)) {
            return Err(bad("naming must produce a nonempty filename of at most 255 bytes without trailing spaces, path separators or reserved characters"));
        }
        // Photoshop Batch writes all subfolder results into the destination itself.
        outputs.push(join(output, &name));
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
