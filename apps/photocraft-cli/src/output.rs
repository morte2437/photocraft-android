use photocraft_io::ExportOptions;
use serde_json::Value;
use std::io::Write;

use crate::{Result, args::ExportArgs};

/// The `--format` family's options. A bad value never gets here: clap refused it.
impl From<&ExportArgs> for ExportOptions {
    fn from(a: &ExportArgs) -> Self {
        // TIFF output is flat unless asked: scripted conversions keep predictable sizes.
        let mut o = ExportOptions { tiff_layers: a.tiff_layers, tga_bits: a.tga_bits, ..Default::default() };
        if let Some(q) = a.quality {
            o.encode.jpeg_quality = q;
            // Asking for a quality asks for a lossy WebP; the default WebP stays lossless.
            o.encode.webp_quality = q;
            o.encode.webp_lossless = false;
        }
        o.encode.dpx_bits = a.dpx_bits;
        if let Some(c) = a.tiff_compression {
            o.encode.tiff_compression = c;
        }
        if let Some(f) = a.jpeg_format {
            o.encode.jpeg_format = f;
        }
        if let Some(c) = a.dds_compression {
            o.encode.dds.compression = c;
        }
        o.encode.dds.mipmaps = a.dds_mipmaps;
        o
    }
}

pub(crate) fn warn_all(err: &mut dyn Write, ws: &[String]) {
    for w in ws {
        let _ = writeln!(err, "warning: {w}");
    }
}

/// How [`print_json`] lays the JSON out.
pub(crate) enum JsonStyle {
    Pretty,
    Compact,
}

pub(crate) fn print_json(out: &mut dyn Write, v: &Value, style: JsonStyle) -> Result {
    let s = match style {
        JsonStyle::Pretty => serde_json::to_string_pretty(v),
        JsonStyle::Compact => serde_json::to_string(v),
    }?;
    Ok(writeln!(out, "{s}")?)
}
