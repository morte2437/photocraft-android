use super::descriptors::{Target, class, code, get_ref, target};
use super::{D, Descriptor, ReferenceItem};

/// Photoshop's `BlnM` enumeration ↔ PhotoCraft's blend mode labels.
const BLEND_MODES: &[(&str, &str)] = &[
    ("Nrml", "Normal"),
    ("normal", "Normal"),
    ("Dslv", "Dissolve"),
    ("dissolve", "Dissolve"),
    ("Drkn", "Darken"),
    ("darken", "Darken"),
    ("Mltp", "Multiply"),
    ("multiply", "Multiply"),
    ("CBrn", "Color Burn"),
    ("colorBurn", "Color Burn"),
    ("linearBurn", "Linear Burn"),
    ("darkerColor", "Darker Color"),
    ("Lghn", "Lighten"),
    ("lighten", "Lighten"),
    ("Scrn", "Screen"),
    ("screen", "Screen"),
    ("CDdg", "Color Dodge"),
    ("colorDodge", "Color Dodge"),
    ("linearDodge", "Linear Dodge (Add)"),
    ("lighterColor", "Lighter Color"),
    ("Ovrl", "Overlay"),
    ("overlay", "Overlay"),
    ("SftL", "Soft Light"),
    ("softLight", "Soft Light"),
    ("HrdL", "Hard Light"),
    ("hardLight", "Hard Light"),
    ("vividLight", "Vivid Light"),
    ("linearLight", "Linear Light"),
    ("pinLight", "Pin Light"),
    ("hardMix", "Hard Mix"),
    ("Dfrn", "Difference"),
    ("difference", "Difference"),
    ("Xclu", "Exclusion"),
    ("exclusion", "Exclusion"),
    ("blendSubtraction", "Subtract"),
    ("blendDivide", "Divide"),
    ("H", "Hue"),
    ("hue", "Hue"),
    ("Strt", "Saturation"),
    ("saturation", "Saturation"),
    ("Clr", "Color"),
    ("color", "Color"),
    ("Lmns", "Luminosity"),
    ("luminosity", "Luminosity"),
    ("PsTh", "Pass Through"),
    ("passThrough", "Pass Through"),
];

pub(super) fn blend_label(ps: &str) -> Option<&'static str> {
    BLEND_MODES.iter().find(|(k, _)| *k == ps).map(|(_, v)| *v)
}

pub(super) fn blend_ps(label: &str) -> Option<&'static str> {
    crate::commands::blend_from_str(label).and_then(|m| BLEND_MODES.iter().find(|(_, v)| crate::commands::blend_from_str(v) == Some(m))).map(|(k, _)| *k)
}

/// Photoshop ↔ PhotoCraft value tables; Photoshop codes compare without their space padding.
pub(super) fn lookup<'a>(table: &'a [(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    table.iter().find(|(k, _)| k.trim_end() == key.trim_end()).map(|(_, v)| *v)
}

pub(super) fn lookup_back<'a>(table: &'a [(&'a str, &'a str)], value: &str) -> Option<&'a str> {
    table.iter().find(|(_, v)| *v == value).map(|(k, _)| *k)
}

/// `Chnl` enumeration ↔ the per-channel keys of the adjustment commands.
pub(super) const CHANNELS: &[(&str, &str)] = &[
    ("Rd", "red"),
    ("Grn", "green"),
    ("Bl", "blue"),
    ("Gry", "gray"),
    ("Cyn", "cyan"),
    ("Mgnt", "magenta"),
    ("Yllw", "yellow"),
    ("Blck", "black"),
    // Lab Lightness: Photoshop rejects `Lmnc` ("luminance") on a Lab document (#9384).
    ("Lght", "lightness"),
    ("A", "a"),
    ("B", "b"),
];

/// The channel key of a `Chnl` enumeration. `Lmnc` is what PhotoCraft wrote for Lightness before
/// #9384; reading it keeps those actions loading.
pub(super) fn channel_of(code: &str) -> Option<&'static str> {
    lookup(CHANNELS, code).or_else(|| (code.trim_end() == "Lmnc").then_some("lightness"))
}

/// The channel an adjustment entry's `Chnl` reference names: `None` = the composite.
pub(super) fn channel_key(d: &Descriptor) -> Option<Option<&'static str>> {
    let r = get_ref(d, "Chnl")?;
    Some(match target(r) {
        Target::Ordinal(o) if o == "Trgt" || o == "Cmps" => None,
        Target::Ordinal(o) => Some(channel_of(&o)?),
        _ => return None,
    })
}

/// A Levels or Curves entry's channel: `Chnl.Cmps` for the composite, as Photoshop records it.
/// Photoshop silently skips a Set whose composite is `Ordn.Trgt` (#8543).
pub(super) fn channel_ref(key: Option<&str>) -> D {
    let value = key.and_then(|k| lookup_back(CHANNELS, k));
    D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code(value.unwrap_or("Cmps")) }])
}

pub(super) const INTERPOLATION: &[(&str, &str)] = &[
    ("Bcbc", "bicubic"),
    ("Blnr", "bilinear"),
    ("Nrst", "nearest"),
    ("bicubicSharper", "bicubicSharper"),
    ("bicubicSmoother", "bicubicSmoother"),
    ("bicubicAutomatic", "bicubicAutomatic"),
    ("automaticInterpolation", "automatic"),
    ("preserveDetailsUpscale", "preserveDetails"),
    ("deepUpscale", "preserveDetails"),
];

pub(super) const NOISE: &[(&str, &str)] = &[("Unfr", "uniform"), ("Gsn ", "gaussian")];

pub(super) const OFFSET_FILL: &[(&str, &str)] = &[("Wrp ", "wrap"), ("Rpt ", "repeat"), ("Bckg", "transparent")];

pub(super) const TRIM: &[(&str, &str)] = &[("Trns", "transparent"), ("topLeftPixelColor", "topLeft"), ("bottomRightPixelColor", "bottomRight")];

pub(super) const EXTENSION: &[(&str, &str)] = &[("Bckg", "background"), ("Frgd", "foreground"), ("Wht ", "white"), ("Blck", "black"), ("Trns", "transparent")];

pub(super) const FILL_CONTENTS: &[(&str, &str)] = &[
    ("FrgC", "foreground"),
    ("BckC", "background"),
    ("Wht ", "white"),
    ("Blck", "black"),
    ("Gry ", "gray"),
    ("Clr ", "color"),
    ("HstS", "history"),
    ("contentAware", "contentAware"),
];

pub(super) const STROKE_LOCATION: &[(&str, &str)] = &[("Insd", "inside"), ("Cntr", "center"), ("OtsF", "outside")];

pub(super) const INTENT: &[(&str, &str)] = &[("Img ", "perceptual"), ("Clrm", "relative"), ("Grp ", "saturation"), ("AbsC", "absolute")];

pub(super) const PROFILES: &[(&str, &str)] =
    &[("sRGB IEC61966-2.1", "srgb"), ("Display P3", "display-p3"), ("Adobe RGB (1998)", "adobe-rgb-compat"), ("ProPhoto RGB", "prophoto-compat")];

/// Smart Blur quality. Photoshop's codes come first, so they are what an export writes (measured in
/// Photoshop: `Hgh `/`Mdm `/`Low ` play back as its fallback, low quality). The old codes after
/// them still read, for actions PhotoCraft wrote before.
pub(super) const SMART_BLUR_QUALITY: &[(&str, &str)] =
    &[("SBQL", "low"), ("SBQM", "medium"), ("SBQH", "high"), ("Hgh ", "high"), ("Mdm ", "medium"), ("Low ", "low")];

pub(super) const SMART_BLUR_MODE: &[(&str, &str)] = &[("SBMN", "normal"), ("SBME", "edgeOnly"), ("SBMO", "overlayEdge")];

pub(super) const RADIAL_METHOD: &[(&str, &str)] = &[("Spn ", "spin"), ("Zm  ", "zoom")];

pub(super) const RADIAL_QUALITY: &[(&str, &str)] = &[("Drft", "draft"), ("Gd  ", "good"), ("Bst ", "best")];

pub(super) const LABEL_COLORS: &[(&str, &str)] =
    &[("None", "none"), ("Rd", "red"), ("Orng", "orange"), ("Ylw", "yellow"), ("Grn", "green"), ("Bl", "blue"), ("Vlt", "violet"), ("Gry", "gray")];

pub(super) const ALIGN: &[(&str, &str)] =
    &[("AdLf", "leftEdges"), ("AdCH", "horizontalCenters"), ("AdRg", "rightEdges"), ("AdTp", "topEdges"), ("AdCV", "verticalCenters"), ("AdBt", "bottomEdges")];

pub(super) const PURGE: &[(&str, &str)] = &[("Al", "all"), ("Clpb", "clipboard"), ("Hsty", "histories"), ("Undo", "undo")];

pub(super) const MODES: &[(&str, &str)] = &[("RGBM", "rgb"), ("Grys", "grayscale"), ("CMYM", "cmyk"), ("LbCM", "lab"), ("Mltc", "multichannel")];

/// `MnIt` menu-item codes ↔ the shell's view commands.
pub(super) const MENU_ITEMS: &[(&str, &str)] =
    &[("FtOn", "view.fitOnScreen"), ("ActP", "view.actualPixels"), ("ZmIn", "view.zoomIn"), ("ZmOt", "view.zoomOut")];

/// Save As format classes (`As`) and the extensions PhotoCraft saves them under. The first
/// extension of a class is the one exported, and the first class of an extension is the one
/// written back. `WebPFormat` is Photoshop's built-in WebP (checked against Photoshop 2026); the
/// Google classes come from the older WebPShop plug-in and are only read.
pub(super) const SAVE_FORMATS: &[(&str, &[&str])] = &[
    ("Pht3", &["psd"]),
    ("PNGF", &["png"]),
    ("JPEG", &["jpg", "jpeg"]),
    ("TIFF", &["tif", "tiff"]),
    ("BMPF", &["bmp"]),
    ("WebPFormat", &["webp"]),
    ("Google WebPShop", &["webp"]),
    ("Google WebP", &["webp"]),
];
