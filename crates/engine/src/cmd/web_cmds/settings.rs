use super::optimizer::to8;
use super::*;

// ---------- settings ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebFormat {
    Gif,
    Png8,
    Png24,
    Jpeg,
    Wbmp,
}

impl WebFormat {
    pub fn from_id(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "gif" => WebFormat::Gif,
            "png8" | "png-8" => WebFormat::Png8,
            "png24" | "png-24" | "png" | "png32" => WebFormat::Png24,
            "jpeg" | "jpg" => WebFormat::Jpeg,
            "wbmp" => WebFormat::Wbmp,
            _ => return None,
        })
    }
    pub fn ext(self) -> &'static str {
        match self {
            WebFormat::Gif => "gif",
            WebFormat::Png8 | WebFormat::Png24 => "png",
            WebFormat::Jpeg => "jpg",
            WebFormat::Wbmp => "wbmp",
        }
    }
    pub fn indexed(self) -> bool {
        matches!(self, WebFormat::Gif | WebFormat::Png8)
    }
}

/// Which metadata Save for Web embeds (JPEG only, like Photoshop's legacy exporter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebMetadata {
    None,
    Copyright,
    CopyrightAndContact,
    /// All the XMP, none of the photo's EXIF (camera, date taken, GPS).
    AllExceptCameraInfo,
    /// All the XMP and the photo's EXIF.
    All,
}

/// One optimisation preset (the right-hand side of the Save for Web dialog).
#[derive(Clone, Debug, PartialEq)]
pub struct WebSettings {
    pub format: WebFormat,
    pub palette: PaletteKind,
    pub colors: usize,
    /// Fixed opaque source palette in index order.
    pub color_table: Option<Vec<[u8; 3]>>,
    /// Replacements applied after quantization, preserving pixel indices.
    pub color_table_edits: std::collections::BTreeMap<usize, [u8; 3]>,
    pub dither: Dither,
    /// 0..=1.
    pub dither_amount: f32,
    pub transparency: bool,
    /// Edge / background colour (0..=1 RGB); None = no matte (hard alpha threshold).
    pub matte: Option<[f32; 3]>,
    pub interlaced: bool,
    /// Web Snap: palette entries within this distance (0..=1 of the channel range) snap to the
    /// web-safe cube.
    pub web_snap: f32,
    /// JPEG quality 0..=100.
    pub quality: u8,
    pub progressive: bool,
    pub optimized: bool,
    pub embed_icc: bool,
    pub metadata: WebMetadata,
    pub convert_to_srgb: bool,
}

impl Default for WebSettings {
    fn default() -> Self {
        WebSettings {
            format: WebFormat::Png24,
            palette: PaletteKind::Selective,
            colors: 256,
            color_table: None,
            color_table_edits: Default::default(),
            dither: Dither::Diffusion,
            dither_amount: 0.88,
            transparency: true,
            matte: Some([1.0, 1.0, 1.0]),
            interlaced: false,
            web_snap: 0.0,
            quality: 60,
            progressive: false,
            optimized: true,
            embed_icc: false,
            metadata: WebMetadata::Copyright,
            convert_to_srgb: true,
        }
    }
}

fn hex3(s: &str) -> Option<[f32; 3]> {
    let s = s.trim_start_matches('#');
    let b = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|v| f32::from(v) / 255.0);
    (s.len() == 6).then_some(())?;
    Some([b(0)?, b(2)?, b(4)?])
}

impl WebSettings {
    /// Settings from command parameters (missing keys keep the defaults; Photoshop's preset
    /// names such as "GIF 128 Dithered" or "JPEG High" are accepted as `"preset"`).
    ///
    /// Without a preset there is no matte, as in Photoshop's scripted Save for Web
    /// (`ExportOptionsSaveForWeb` left at its defaults): a GIF or PNG-8 keeps the colour of its
    /// partly transparent pixels instead of lightening them towards white (#4115). Formats
    /// without transparency still go over white.
    pub fn from_params(p: &Value, cmd: &str) -> Result<Self> {
        let mut st = WebSettings { matte: None, ..WebSettings::default() };
        if let Some(preset) = p.get("preset").and_then(Value::as_str) {
            st = preset_settings(preset).ok_or_else(|| bad(cmd, format!("unknown preset `{preset}`")))?;
        }
        let s = |k: &str| p.get(k).and_then(Value::as_str);
        if let Some(f) = s("format") {
            st.format = WebFormat::from_id(f).ok_or_else(|| bad(cmd, format!("unknown format `{f}` (gif|png8|png24|jpeg|wbmp)")))?;
        }
        if let Some(k) = s("palette") {
            st.palette = match k {
                "restrictive" | "web" => PaletteKind::Web,
                k => PaletteKind::from_id(k).ok_or_else(|| {
                    bad(cmd, format!("unknown palette `{k}` (perceptual|selective|adaptive|restrictive|exact|systemMac|systemWindows|uniform)"))
                })?,
            };
        }
        if let Some(n) = crate::commands::int(p, "colors").filter(|v| *v > 0).map(|v| v as u64) {
            st.colors = (n as usize).clamp(2, 256);
        }
        if let Some(d) = s("dither") {
            st.dither = match d {
                "none" | "noDither" => Dither::None,
                "diffusion" => Dither::Diffusion,
                "pattern" => Dither::Pattern,
                "noise" => Dither::Noise,
                d => return Err(bad(cmd, format!("unknown dither `{d}` (none|diffusion|pattern|noise)"))),
            };
        }
        if let Some(a) = p.get("ditherAmount").and_then(Value::as_f64) {
            st.dither_amount = (a as f32 / 100.0).clamp(0.0, 1.0);
        }
        if let Some(b) = p.get("transparency").and_then(Value::as_bool) {
            st.transparency = b;
        }
        match s("matte") {
            Some("none") => st.matte = None,
            Some(c) => st.matte = Some(hex3(c).ok_or_else(|| bad(cmd, "matte is \"none\" or \"#rrggbb\""))?),
            None => {}
        }
        for (k, slot) in [
            ("interlaced", &mut st.interlaced),
            ("progressive", &mut st.progressive),
            ("optimized", &mut st.optimized),
            ("embedIcc", &mut st.embed_icc),
            ("convertToSrgb", &mut st.convert_to_srgb),
        ] {
            if let Some(b) = p.get(k).and_then(Value::as_bool) {
                *slot = b;
            }
        }
        if let Some(w) = p.get("webSnap").and_then(Value::as_f64) {
            st.web_snap = (w as f32 / 100.0).clamp(0.0, 1.0);
        }
        if let Some(q) = p.get("quality").and_then(Value::as_f64) {
            st.quality = q.clamp(0.0, 100.0).round() as u8;
        }
        if let Some(m) = s("metadata") {
            st.metadata = match m {
                "none" => WebMetadata::None,
                "copyright" => WebMetadata::Copyright,
                "copyrightAndContact" | "contact" => WebMetadata::CopyrightAndContact,
                "allExceptCameraInfo" => WebMetadata::AllExceptCameraInfo,
                "all" => WebMetadata::All,
                m => return Err(bad(cmd, format!("unknown metadata `{m}` (none|copyright|copyrightAndContact|allExceptCameraInfo|all)"))),
            };
        }
        if let Some(table) = p.get("colorTable") {
            let table = table.as_array().filter(|a| (1..=256).contains(&a.len())).ok_or_else(|| bad(cmd, "colorTable must contain 1–256 #rrggbb colours"))?;
            st.color_table = Some(
                table
                    .iter()
                    .map(|v| v.as_str().and_then(hex3).map(|c| c.map(to8)).ok_or_else(|| bad(cmd, "colorTable entries must be #rrggbb colours")))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        if let Some(edits) = p.get("colorTableEdits") {
            let edits = edits
                .as_object()
                .filter(|m| m.len() <= 256)
                .ok_or_else(|| bad(cmd, "colorTableEdits must be an object of palette indices to #rrggbb colours"))?;
            let table = st.color_table.as_ref().ok_or_else(|| bad(cmd, "colorTableEdits requires colorTable"))?;
            for (index, value) in edits {
                let index = index.parse::<usize>().ok().filter(|i| *i < table.len()).ok_or_else(|| bad(cmd, "colorTableEdits index is outside colorTable"))?;
                let color = value.as_str().and_then(hex3).map(|c| c.map(to8)).ok_or_else(|| bad(cmd, "colorTableEdits values must be #rrggbb colours"))?;
                st.color_table_edits.insert(index, color);
            }
        }
        Ok(st)
    }
}

impl WebSettings {
    /// The settings as command parameters (the inverse of [`WebSettings::from_params`]).
    pub fn to_params(&self) -> Value {
        let format = match self.format {
            WebFormat::Gif => "gif",
            WebFormat::Png8 => "png8",
            WebFormat::Png24 => "png24",
            WebFormat::Jpeg => "jpeg",
            WebFormat::Wbmp => "wbmp",
        };
        let palette = match self.palette {
            PaletteKind::Exact => "exact",
            PaletteKind::SystemMac => "systemMac",
            PaletteKind::SystemWindows => "systemWindows",
            PaletteKind::Web => "restrictive",
            PaletteKind::Uniform => "uniform",
            PaletteKind::Perceptual => "perceptual",
            PaletteKind::Selective => "selective",
            PaletteKind::Adaptive => "adaptive",
        };
        let dither = match self.dither {
            Dither::None => "none",
            Dither::Diffusion => "diffusion",
            Dither::Pattern => "pattern",
            Dither::Noise => "noise",
        };
        let metadata = match self.metadata {
            WebMetadata::None => "none",
            WebMetadata::Copyright => "copyright",
            WebMetadata::CopyrightAndContact => "copyrightAndContact",
            WebMetadata::AllExceptCameraInfo => "allExceptCameraInfo",
            WebMetadata::All => "all",
        };
        let matte = self.matte.map_or("none".to_string(), |c| format!("#{:02x}{:02x}{:02x}", to8(c[0]), to8(c[1]), to8(c[2])));
        let mut p = json!({"format": format, "palette": palette, "colors": self.colors, "dither": dither, "ditherAmount": (self.dither_amount * 100.0).round(), "transparency": self.transparency, "matte": matte, "interlaced": self.interlaced, "webSnap": (self.web_snap * 100.0).round(), "quality": self.quality, "progressive": self.progressive, "optimized": self.optimized, "embedIcc": self.embed_icc, "metadata": metadata, "convertToSrgb": self.convert_to_srgb});
        if let Some(table) = &self.color_table {
            p["colorTable"] = json!(table.iter().map(|c| rgb_hex(*c)).collect::<Vec<_>>());
        }
        if !self.color_table_edits.is_empty() {
            p["colorTableEdits"] =
                json!(self.color_table_edits.iter().map(|(i, c)| (i.to_string(), rgb_hex(*c))).collect::<std::collections::BTreeMap<_, _>>());
        }
        p
    }
}

/// A palette colour in command/control-channel notation.
pub fn rgb_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Save for Web's named presets.
pub const PRESETS: [&str; 12] = [
    "GIF 128 Dithered",
    "GIF 128 No Dither",
    "GIF 32 Dithered",
    "GIF 32 No Dither",
    "GIF 64 Dithered",
    "GIF 64 No Dither",
    "GIF Restrictive",
    "JPEG High",
    "JPEG Low",
    "JPEG Medium",
    "PNG-24",
    "PNG-8 128 Dithered",
];

pub fn preset_settings(name: &str) -> Option<WebSettings> {
    let d = WebSettings::default();
    let gif = |colors: usize, dither: bool| WebSettings {
        format: WebFormat::Gif,
        colors,
        dither: if dither { Dither::Diffusion } else { Dither::None },
        ..WebSettings::default()
    };
    Some(match name {
        "GIF 128 Dithered" => gif(128, true),
        "GIF 128 No Dither" => gif(128, false),
        "GIF 32 Dithered" => gif(32, true),
        "GIF 32 No Dither" => gif(32, false),
        "GIF 64 Dithered" => gif(64, true),
        "GIF 64 No Dither" => gif(64, false),
        "GIF Restrictive" => WebSettings { palette: PaletteKind::Web, ..gif(256, true) },
        "JPEG High" => WebSettings { format: WebFormat::Jpeg, quality: 60, ..d },
        "JPEG Low" => WebSettings { format: WebFormat::Jpeg, quality: 10, ..d },
        "JPEG Medium" => WebSettings { format: WebFormat::Jpeg, quality: 30, ..d },
        "JPEG Maximum" => WebSettings { format: WebFormat::Jpeg, quality: 100, ..d },
        "JPEG Very High" => WebSettings { format: WebFormat::Jpeg, quality: 80, ..d },
        "PNG-24" => WebSettings { format: WebFormat::Png24, ..d },
        "PNG-8 128 Dithered" => WebSettings { format: WebFormat::Png8, colors: 128, ..d },
        _ => return None,
    })
}
