//! Photoshop's Filter Gallery steps (#10592): the `GEfc` event with the effect's id in `GEfk`
//! and its settings, for all 47 effects. Each maps to its `filter.gallery.<effect>` command with
//! the keys, limits and defaults Photoshop measures (`docs/atn.md`, Group A); a stack of several
//! effects stays a Photoshop step.

use super::descriptors::{color_desc, color_hex, desc, enm, get_bool, get_enum, get_num, get_obj, jstr, trimmed};
use super::events::pixel_target;
use super::fields::{Mapped, step};
use super::tables::{lookup, lookup_back};
use super::{AtnEvent, AtnItem, D, Descriptor, Map, Value, json};
use photocraft_algo::{GalleryEffect, GalleryFilter};

/// One key of an effect, in recorded order.
enum G {
    /// An integer clamped to `min..=max` on import, as Photoshop clamps it, and its default.
    Int(&'static str, &'static str, i32, i32, i32),
    /// An enumeration (Photoshop ↔ PhotoCraft values) and Photoshop's default value. An unknown
    /// value keeps the step.
    Enum(&'static str, &'static str, &'static [(&'static str, &'static str)], &'static str),
    Bool(&'static str, &'static str, bool),
    /// A colour object ↔ `#rrggbb` and Photoshop's default, which a step recorded without the
    /// key loads with. A PhotoCraft step without it saves the effect's own default colour, the
    /// one it plays with.
    Color(&'static str, &'static str, &'static str),
}

impl G {
    fn key(&self) -> &'static str {
        match self {
            G::Int(k, ..) | G::Enum(k, ..) | G::Bool(k, ..) | G::Color(k, ..) => k,
        }
    }

    fn param(&self) -> &'static str {
        match self {
            G::Int(_, p, ..) | G::Enum(_, p, ..) | G::Bool(_, p, _) | G::Color(_, p, _) => p,
        }
    }
}

/// The type Photoshop records an enumeration key with. Its terminology gives stroke direction
/// its own type (`strokeDirectionType`); every other gallery enumeration's type is its key (Glass
/// records `TxtT` / `TxtT`).
fn enum_type(key: &'static str) -> &'static str {
    match key {
        "SDir" => "StrD",
        k => k,
    }
}

/// Photoshop's effect id (`GEfk`), the command, and the effect's keys in recorded order.
/// Generated from the event reference's Group A tables; Glass maps only the textures whose
/// codes the reference gives (Frosted, Canvas).
const GALLERY: &[(&str, &str, &[G])] = &[
    // Accented Edges
    (
        "AccE",
        "filter.gallery.accentedEdges",
        &[G::Int("EdgW", "edgeWidth", 1, 14, 2), G::Int("EdgB", "edgeBrightness", 0, 50, 38), G::Int("Smth", "smoothness", 1, 15, 5)],
    ),
    // Angled Strokes
    (
        "AngS",
        "filter.gallery.angledStrokes",
        &[G::Int("DrcB", "directionBalance", 0, 100, 50), G::Int("StrL", "strokeLength", 3, 50, 15), G::Int("Shrp", "sharpness", 0, 10, 3)],
    ),
    // Bas Relief
    (
        "BsRl",
        "filter.gallery.basRelief",
        &[
            G::Int("Dtl ", "detail", 1, 15, 13),
            G::Int("Smth", "smoothness", 1, 15, 3),
            G::Enum(
                "LghD",
                "light",
                &[
                    ("LDBL", "bottomLeft"),
                    ("LDBR", "bottomRight"),
                    ("LDBt", "bottom"),
                    ("LDLf", "left"),
                    ("LDRg", "right"),
                    ("LDTL", "topLeft"),
                    ("LDTR", "topRight"),
                    ("LDTp", "top"),
                ],
                "LDBt",
            ),
        ],
    ),
    // Chalk Charcoal
    (
        "ChlC",
        "filter.gallery.chalkCharcoal",
        &[G::Int("ChrA", "charcoalArea", 0, 20, 6), G::Int("ChlA", "chalkArea", 0, 20, 6), G::Int("StrP", "strokePressure", 0, 5, 1)],
    ),
    // Charcoal
    (
        "Chrc",
        "filter.gallery.charcoal",
        &[G::Int("ChAm", "charcoalThickness", 1, 7, 1), G::Int("Dtl ", "detail", 0, 5, 5), G::Int("LgDr", "lightDarkBalance", 0, 100, 50)],
    ),
    // Chrome
    ("Chrm", "filter.gallery.chrome", &[G::Int("Dtl ", "detail", 0, 10, 4), G::Int("Smth", "smoothness", 0, 10, 7)]),
    // Colored Pencil
    (
        "ClrP",
        "filter.gallery.coloredPencil",
        &[G::Int("Pncl", "pencilWidth", 1, 24, 4), G::Int("StrP", "strokePressure", 0, 15, 8), G::Int("PprB", "paperBrightness", 0, 50, 25)],
    ),
    // Conte Crayon
    (
        "CntC",
        "filter.gallery.conteCrayon",
        &[
            G::Int("FrgL", "foregroundLevel", 1, 15, 11),
            G::Int("BckL", "backgroundLevel", 1, 15, 7),
            G::Enum("TxtT", "texture", &[("TxBr", "brick"), ("TxBu", "burlap"), ("TxCa", "canvas"), ("TxSt", "sandstone")], "TxCa"),
            G::Int("Scln", "scaling", 50, 200, 100),
            G::Int("Rlf ", "relief", 0, 50, 4),
            G::Enum(
                "LghD",
                "light",
                &[
                    ("LDBL", "bottomLeft"),
                    ("LDBR", "bottomRight"),
                    ("LDBt", "bottom"),
                    ("LDLf", "left"),
                    ("LDRg", "right"),
                    ("LDTL", "topLeft"),
                    ("LDTR", "topRight"),
                    ("LDTp", "top"),
                ],
                "LDTp",
            ),
            G::Bool("InvT", "invert", false),
        ],
    ),
    // Craquelure
    (
        "Crql",
        "filter.gallery.craquelure",
        &[G::Int("CrcS", "crackSpacing", 2, 100, 15), G::Int("CrcD", "crackDepth", 0, 10, 6), G::Int("CrcB", "crackBrightness", 0, 10, 9)],
    ),
    // Crosshatch
    (
        "Crsh",
        "filter.gallery.crosshatch",
        &[G::Int("StrL", "strokeLength", 3, 50, 9), G::Int("Shrp", "sharpness", 0, 20, 6), G::Int("Strg", "strength", 1, 3, 1)],
    ),
    // Cutout
    (
        "Ct",
        "filter.gallery.cutout",
        &[G::Int("NmbL", "numberOfLevels", 2, 8, 4), G::Int("EdgS", "edgeSimplicity", 0, 10, 4), G::Int("EdgF", "edgeFidelity", 1, 3, 2)],
    ),
    // Dark Strokes
    (
        "DrkS",
        "filter.gallery.darkStrokes",
        &[G::Int("Blnc", "balance", 0, 10, 5), G::Int("BlcI", "blackIntensity", 0, 10, 6), G::Int("WhtI", "whiteIntensity", 0, 10, 2)],
    ),
    // Diffuse Glow
    (
        "DfsG",
        "filter.gallery.diffuseGlow",
        &[G::Int("Grns", "graininess", 0, 10, 6), G::Int("GlwA", "glowAmount", 0, 20, 10), G::Int("ClrA", "clearAmount", 0, 20, 15)],
    ),
    // Dry Brush
    ("DryB", "filter.gallery.dryBrush", &[G::Int("BrsS", "brushSize", 0, 10, 2), G::Int("BrsD", "brushDetail", 0, 10, 8), G::Int("Txtr", "texture", 1, 3, 1)]),
    // Film Grain
    (
        "FlmG",
        "filter.gallery.filmGrain",
        &[G::Int("Grn ", "grain", 0, 20, 4), G::Int("HghA", "highlightArea", 0, 20, 0), G::Int("Intn", "intensity", 0, 10, 10)],
    ),
    // Fresco
    ("Frsc", "filter.gallery.fresco", &[G::Int("BrsS", "brushSize", 0, 10, 2), G::Int("BrsD", "brushDetail", 0, 10, 8), G::Int("Txtr", "texture", 1, 3, 1)]),
    // Glass
    (
        "Gls",
        "filter.gallery.glass",
        &[
            G::Int("Dstr", "distortion", 0, 20, 5),
            G::Int("Smth", "smoothness", 1, 15, 3),
            G::Enum("TxtT", "texture", &[("TxFr", "frosted"), ("TxCa", "canvas")], "TxFr"),
            G::Int("Scln", "scaling", 50, 200, 100),
            G::Bool("InvT", "invert", false),
        ],
    ),
    // Glowing Edges
    (
        "GlwE",
        "filter.gallery.glowingEdges",
        &[G::Int("EdgW", "edgeWidth", 1, 14, 2), G::Int("EdgB", "edgeBrightness", 0, 20, 6), G::Int("Smth", "smoothness", 1, 15, 5)],
    ),
    // Graphic Pen
    (
        "GraP",
        "filter.gallery.graphicPen",
        &[
            G::Int("StrL", "strokeLength", 1, 15, 15),
            G::Int("LgDr", "lightDarkBalance", 0, 100, 50),
            G::Enum("SDir", "strokeDirection", &[("SDHz", "horizontal"), ("SDLD", "leftDiagonal"), ("SDRD", "rightDiagonal"), ("SDVt", "vertical")], "SDRD"),
        ],
    ),
    // Halftone Screen
    (
        "HlfS",
        "filter.gallery.halftonePattern",
        &[
            G::Int("HlSz", "size", 1, 12, 1),
            G::Int("Cntr", "contrast", 0, 50, 5),
            G::Enum("ScrT", "patternType", &[("ScrC", "circle"), ("ScrD", "dot"), ("ScrL", "line")], "ScrD"),
        ],
    ),
    // Ink Outlines
    (
        "InkO",
        "filter.gallery.inkOutlines",
        &[G::Int("StrL", "strokeLength", 1, 50, 4), G::Int("DrkI", "darkIntensity", 0, 50, 20), G::Int("LghI", "lightIntensity", 0, 50, 10)],
    ),
    // Mosaic  Tiles
    (
        "MscT",
        "filter.gallery.mosaicTiles",
        &[G::Int("TlSz", "tileSize", 2, 100, 12), G::Int("GrtW", "groutWidth", 1, 15, 3), G::Int("LghG", "lightenGrout", 0, 10, 9)],
    ),
    // Neon Glow
    (
        "NGlw",
        "filter.gallery.neonGlow",
        &[G::Int("Sz  ", "glowSize", -24, 24, 5), G::Int("Brgh", "glowBrightness", 0, 50, 15), G::Color("Clr ", "glowColor", "#0000ff")],
    ),
    // Note Paper
    (
        "NtPr",
        "filter.gallery.notePaper",
        &[G::Int("ImgB", "imageBalance", 0, 50, 25), G::Int("Grns", "graininess", 0, 20, 10), G::Int("Rlf ", "relief", 0, 25, 11)],
    ),
    // Ocean Ripple
    ("OcnR", "filter.gallery.oceanRipple", &[G::Int("RplS", "rippleSize", 1, 15, 9), G::Int("RplM", "rippleMagnitude", 0, 20, 9)]),
    // Paint Daubs
    (
        "PntD",
        "filter.gallery.paintDaubs",
        &[
            G::Int("Sz  ", "brushSize", 1, 50, 8),
            G::Int("Shrp", "sharpness", 0, 40, 7),
            G::Enum(
                "BrsT",
                "brushType",
                &[("BrDR", "darkRough"), ("BrSm", "simple"), ("BrSp", "sparkle"), ("BrbW", "wideBlurry"), ("BrsL", "lightRough"), ("BrsW", "wideSharp")],
                "BrSm",
            ),
        ],
    ),
    // Palette Knife
    (
        "PltK",
        "filter.gallery.paletteKnife",
        &[G::Int("StrS", "strokeSize", 1, 50, 25), G::Int("StDt", "strokeDetail", 1, 3, 3), G::Int("Sftn", "softness", 0, 10, 0)],
    ),
    // Patchwork
    ("Ptch", "filter.gallery.patchwork", &[G::Int("SqrS", "squareSize", 0, 10, 4), G::Int("Rlf ", "relief", 0, 25, 8)]),
    // Photocopy
    ("Phtc", "filter.gallery.photocopy", &[G::Int("Dtl ", "detail", 1, 24, 7), G::Int("Drkn", "darkness", 1, 50, 8)]),
    // Plaster
    (
        "Plst",
        "filter.gallery.plaster",
        &[
            G::Int("ImgB", "imageBalance", 0, 50, 20),
            G::Int("Smth", "smoothness", 1, 15, 2),
            G::Enum(
                "LghP",
                "light",
                &[
                    ("LPBL", "bottomLeft"),
                    ("LPBr", "bottomRight"),
                    ("LPBt", "bottom"),
                    ("LPLf", "left"),
                    ("LPRg", "right"),
                    ("LPTL", "topLeft"),
                    ("LPTR", "topRight"),
                    ("LPTp", "top"),
                ],
                "LPTp",
            ),
        ],
    ),
    // Plastic Wrap
    (
        "PlsW",
        "filter.gallery.plasticWrap",
        &[G::Int("HghS", "highlightStrength", 0, 20, 15), G::Int("Dtl ", "detail", 1, 15, 9), G::Int("Smth", "smoothness", 1, 15, 7)],
    ),
    // Poster Edges
    (
        "PstE",
        "filter.gallery.posterEdges",
        &[G::Int("EdgT", "edgeThickness", 0, 10, 2), G::Int("EdgI", "edgeIntensity", 0, 10, 1), G::Int("Pstr", "posterization", 0, 6, 2)],
    ),
    // Reticulation
    (
        "Rtcl",
        "filter.gallery.reticulation",
        &[G::Int("Dnst", "density", 0, 50, 12), G::Int("BlcL", "foregroundLevel", 0, 50, 40), G::Int("WhtL", "backgroundLevel", 0, 50, 5)],
    ),
    // Rough Pastels
    (
        "RghP",
        "filter.gallery.roughPastels",
        &[
            G::Int("StrL", "strokeLength", 0, 40, 6),
            G::Int("StDt", "strokeDetail", 1, 20, 4),
            G::Enum("TxtT", "texture", &[("TxBr", "brick"), ("TxBu", "burlap"), ("TxCa", "canvas"), ("TxSt", "sandstone")], "TxCa"),
            G::Int("Scln", "scaling", 50, 200, 100),
            G::Int("Rlf ", "relief", 0, 50, 20),
            G::Enum(
                "LghD",
                "light",
                &[
                    ("LDBL", "bottomLeft"),
                    ("LDBR", "bottomRight"),
                    ("LDBt", "bottom"),
                    ("LDLf", "left"),
                    ("LDRg", "right"),
                    ("LDTL", "topLeft"),
                    ("LDTR", "topRight"),
                    ("LDTp", "top"),
                ],
                "LDBt",
            ),
            G::Bool("InvT", "invert", false),
        ],
    ),
    // Smudge Stick
    (
        "SmdS",
        "filter.gallery.smudgeStick",
        &[G::Int("StrL", "strokeLength", 0, 10, 2), G::Int("HghA", "highlightArea", 0, 20, 0), G::Int("Intn", "intensity", 0, 10, 10)],
    ),
    // Spatter
    ("Spt", "filter.gallery.spatter", &[G::Int("SprR", "sprayRadius", 0, 25, 10), G::Int("Smth", "smoothness", 1, 15, 5)]),
    // Sponge
    (
        "Spng",
        "filter.gallery.sponge",
        &[G::Int("BrsS", "brushSize", 0, 10, 2), G::Int("Dfnt", "definition", 0, 25, 12), G::Int("Smth", "smoothness", 1, 15, 5)],
    ),
    // Sprayed Strokes
    (
        "SprS",
        "filter.gallery.sprayedStrokes",
        &[
            G::Int("StrL", "strokeLength", 0, 20, 12),
            G::Int("SprR", "sprayRadius", 0, 25, 7),
            G::Enum("SDir", "strokeDirection", &[("SDHz", "horizontal"), ("SDLD", "leftDiagonal"), ("SDRD", "rightDiagonal"), ("SDVt", "vertical")], "SDRD"),
        ],
    ),
    // Stained Glass
    (
        "StnG",
        "filter.gallery.stainedGlass",
        &[G::Int("ClSz", "cellSize", 2, 50, 10), G::Int("BrdT", "borderThickness", 1, 20, 4), G::Int("LghI", "lightIntensity", 0, 10, 3)],
    ),
    // Stamp
    ("Stmp", "filter.gallery.stamp", &[G::Int("LgDr", "lightDarkBalance", 0, 50, 25), G::Int("Smth", "smoothness", 1, 50, 5)]),
    // Sumie
    (
        "Smie",
        "filter.gallery.sumiE",
        &[G::Int("StrW", "strokeWidth", 3, 15, 10), G::Int("StrP", "strokePressure", 0, 15, 2), G::Int("Cntr", "contrast", 0, 40, 16)],
    ),
    // Texturizer
    (
        "Txtz",
        "filter.gallery.texturizer",
        &[
            G::Enum("TxtT", "texture", &[("TxBr", "brick"), ("TxBu", "burlap"), ("TxCa", "canvas"), ("TxSt", "sandstone")], "TxCa"),
            G::Int("Scln", "scaling", 50, 200, 100),
            G::Int("Rlf ", "relief", 0, 50, 4),
            G::Enum(
                "LghD",
                "light",
                &[
                    ("LDBL", "bottomLeft"),
                    ("LDBR", "bottomRight"),
                    ("LDBt", "bottom"),
                    ("LDLf", "left"),
                    ("LDRg", "right"),
                    ("LDTL", "topLeft"),
                    ("LDTR", "topRight"),
                    ("LDTp", "top"),
                ],
                "LDTp",
            ),
            G::Bool("InvT", "invert", false),
        ],
    ),
    // Torn Edges
    (
        "TrnE",
        "filter.gallery.tornEdges",
        &[G::Int("ImgB", "imageBalance", 0, 50, 25), G::Int("Smth", "smoothness", 1, 15, 11), G::Int("Cntr", "contrast", 1, 25, 17)],
    ),
    // Underpainting
    (
        "Undr",
        "filter.gallery.underpainting",
        &[
            G::Int("BrsS", "brushSize", 0, 40, 6),
            G::Int("TxtC", "textureCoverage", 0, 40, 16),
            G::Enum("TxtT", "texture", &[("TxBr", "brick"), ("TxBu", "burlap"), ("TxCa", "canvas"), ("TxSt", "sandstone")], "TxCa"),
            G::Int("Scln", "scaling", 50, 200, 100),
            G::Int("Rlf ", "relief", 0, 50, 4),
            G::Enum(
                "LghD",
                "light",
                &[
                    ("LDBL", "bottomLeft"),
                    ("LDBR", "bottomRight"),
                    ("LDBt", "bottom"),
                    ("LDLf", "left"),
                    ("LDRg", "right"),
                    ("LDTL", "topLeft"),
                    ("LDTR", "topRight"),
                    ("LDTp", "top"),
                ],
                "LDTp",
            ),
            G::Bool("InvT", "invert", false),
        ],
    ),
    // Water Paper
    (
        "WtrP",
        "filter.gallery.waterPaper",
        &[G::Int("FbrL", "fiberLength", 3, 50, 15), G::Int("Brgh", "brightness", 0, 100, 60), G::Int("Cntr", "contrast", 0, 100, 80)],
    ),
    // Watercolor
    (
        "Wtrc",
        "filter.gallery.watercolor",
        &[G::Int("BrsD", "brushDetail", 1, 14, 9), G::Int("ShdI", "shadowIntensity", 0, 10, 1), G::Int("Txtr", "texture", 1, 3, 1)],
    ),
    // Grain
    (
        "Grn",
        "filter.gallery.grain",
        &[
            G::Int("Intn", "intensity", 0, 100, 40),
            G::Int("Cntr", "contrast", 0, 100, 50),
            G::Enum(
                "Grnt",
                "grainType",
                &[
                    ("GrCn", "contrasty"),
                    ("GrSf", "soft"),
                    ("GrSp", "speckle"),
                    ("GrSr", "sprinkles"),
                    ("GrSt", "stippled"),
                    ("GrnC", "clumped"),
                    ("GrnE", "enlarged"),
                    ("GrnH", "horizontal"),
                    ("GrnR", "regular"),
                    ("GrnV", "vertical"),
                ],
                "GrnR",
            ),
        ],
    ),
];

pub(super) const GALLERY_IDS: &[&str] = &[
    "filter.gallery.accentedEdges",
    "filter.gallery.angledStrokes",
    "filter.gallery.basRelief",
    "filter.gallery.chalkCharcoal",
    "filter.gallery.charcoal",
    "filter.gallery.chrome",
    "filter.gallery.coloredPencil",
    "filter.gallery.conteCrayon",
    "filter.gallery.craquelure",
    "filter.gallery.crosshatch",
    "filter.gallery.cutout",
    "filter.gallery.darkStrokes",
    "filter.gallery.diffuseGlow",
    "filter.gallery.dryBrush",
    "filter.gallery.filmGrain",
    "filter.gallery.fresco",
    "filter.gallery.glass",
    "filter.gallery.glowingEdges",
    "filter.gallery.graphicPen",
    "filter.gallery.halftonePattern",
    "filter.gallery.inkOutlines",
    "filter.gallery.mosaicTiles",
    "filter.gallery.neonGlow",
    "filter.gallery.notePaper",
    "filter.gallery.oceanRipple",
    "filter.gallery.paintDaubs",
    "filter.gallery.paletteKnife",
    "filter.gallery.patchwork",
    "filter.gallery.photocopy",
    "filter.gallery.plaster",
    "filter.gallery.plasticWrap",
    "filter.gallery.posterEdges",
    "filter.gallery.reticulation",
    "filter.gallery.roughPastels",
    "filter.gallery.smudgeStick",
    "filter.gallery.spatter",
    "filter.gallery.sponge",
    "filter.gallery.sprayedStrokes",
    "filter.gallery.stainedGlass",
    "filter.gallery.stamp",
    "filter.gallery.sumiE",
    "filter.gallery.texturizer",
    "filter.gallery.tornEdges",
    "filter.gallery.underpainting",
    "filter.gallery.waterPaper",
    "filter.gallery.watercolor",
    "filter.gallery.grain",
];

/// Each effect's own event by string ID, the form newer Photoshop versions record it in
/// (`docs/atn.md`, Group A). Mosaic Tiles (`MscT`) has none.
const EVENT_NAMES: &[(&str, &str)] = &[
    ("AccE", "accentedEdges"),
    ("AngS", "angledStrokes"),
    ("BsRl", "basRelief"),
    ("ChlC", "chalkCharcoal"),
    ("Chrc", "charcoal"),
    ("Chrm", "chrome"),
    ("ClrP", "coloredPencil"),
    ("CntC", "conteCrayon"),
    ("Crql", "craquelure"),
    ("Crsh", "crosshatch"),
    ("Ct", "cutout"),
    ("DrkS", "darkStrokes"),
    ("DfsG", "diffuseGlow"),
    ("DryB", "dryBrush"),
    ("FlmG", "filmGrain"),
    ("Frsc", "fresco"),
    ("Gls", "glass"),
    ("GlwE", "glowingEdges"),
    ("GraP", "graphicPen"),
    ("HlfS", "halftoneScreen"),
    ("InkO", "inkOutlines"),
    ("NGlw", "neonGlow"),
    ("NtPr", "notePaper"),
    ("OcnR", "oceanRipple"),
    ("PntD", "paintDaubs"),
    ("PltK", "paletteKnife"),
    ("Ptch", "patchwork"),
    ("Phtc", "photocopy"),
    ("Plst", "plaster"),
    ("PlsW", "plasticWrap"),
    ("PstE", "posterEdges"),
    ("Rtcl", "reticulation"),
    ("RghP", "roughPastels"),
    ("SmdS", "smudgeStick"),
    ("Spt", "spatter"),
    ("Spng", "sponge"),
    ("SprS", "sprayedStrokes"),
    ("StnG", "stainedGlass"),
    ("Stmp", "stamp"),
    ("Smie", "sumie"),
    ("Txtz", "texturizer"),
    ("TrnE", "tornEdges"),
    ("Undr", "underpainting"),
    ("WtrP", "waterPaper"),
    ("Wtrc", "watercolor"),
    ("Grn", "grain"),
];

/// Whether `event` is a Filter Gallery effect's own event: its 4-char code, or its string ID.
/// The reference has each effect recorded as its own event too (Filter › Artistic › Cutout…),
/// carrying the same `GEfk` and settings.
pub(super) fn is_effect(event: &AtnEvent) -> bool {
    effect_command(event).is_some()
}

/// The command of the effect whose own event `event` is (`AccE` or `accentedEdges` →
/// `filter.gallery.accentedEdges`).
fn effect_command(event: &AtnEvent) -> Option<&'static str> {
    let id = event.id();
    let code = match event {
        AtnEvent::Code(_) => GALLERY.iter().map(|(c, _, _)| *c).find(|c| *c == id.trim_end()),
        AtnEvent::Name(_) => EVENT_NAMES.iter().find(|(_, n)| *n == id).map(|(c, _)| *c),
    }?;
    GALLERY.iter().find(|(c, _, _)| *c == code).map(|(_, cmd, _)| *cmd)
}

/// The hidden param a step loaded from an effect's own event carries, so it saves back as that
/// event (and panel name) rather than as a `GEfc` Filter Gallery step. Playback strips it
/// (`actions_cmds::playback_params`), so the command, the journal and smart filters never see it.
pub(crate) const RECORDED_EVENT: &str = "_atnEvent";

/// Remember the event of a step loaded from an effect's own event (`AccE`, `accentedEdges`).
/// A `GEfc` step carries nothing and saves as `GEfc`, as does a step made in PhotoCraft.
pub(super) fn remember_event(item: &AtnItem, steps: &mut [(String, Value)]) {
    if !is_effect(&item.event) {
        return;
    }
    let [(id, Value::Object(p))] = steps else { return };
    if effect_command(&item.event) != Some(id.as_str()) {
        return;
    }
    let kind = match item.event {
        AtnEvent::Code(_) => "code",
        AtnEvent::Name(_) => "name",
    };
    // `dictionary` keeps the panel name's bytes as recorded (a GBK or Shift_JIS name would not
    // survive a round trip through text); `name` is for reading.
    let dictionary = photocraft_paint::tile::b64_encode(&item.dictionary);
    p.insert(RECORDED_EVENT.into(), json!({"event": item.event.id(), "eventKind": kind, "name": item.dictionary(), "dictionary": dictionary}));
}

/// The event and panel name bytes a step of command `id` remembered by [`remember_event`] saves
/// as; `None` (save as `GEfc`) when it has none or it is not the own event of `id`'s effect.
pub(super) fn recorded_event(id: &str, params: &Value) -> Option<(AtnEvent, Vec<u8>)> {
    let r = params.get(RECORDED_EVENT)?;
    let event = jstr(r, "event")?;
    let event = match jstr(r, "eventKind")? {
        "code" => AtnEvent::Code(event.as_bytes().try_into().ok()?),
        "name" => AtnEvent::name(event),
        _ => return None,
    };
    let name = match jstr(r, "dictionary").and_then(photocraft_paint::tile::b64_decode) {
        Some(bytes) => bytes,
        None => jstr(r, "name").unwrap_or("").as_bytes().to_vec(),
    };
    (effect_command(&event) == Some(id)).then_some((event, name))
}

/// A Filter Gallery step: the effect `GEfk` names, every key clamped or looked up, Photoshop's
/// default for what it leaves out. An unknown effect, value or key keeps the Photoshop step.
pub(super) fn gallery_from(d: &Descriptor) -> Mapped {
    effect_step(d).unwrap_or(Mapped::Unsupported { harmless: false })
}

fn effect_step(d: &Descriptor) -> Option<Mapped> {
    let effect = get_enum(d, "GEfk")?;
    let (_, id, keys) = GALLERY.iter().find(|(code, _, _)| *code == effect)?;
    let known: Vec<&str> = keys.iter().map(|k| k.key().trim_end()).chain(["GEfk"]).collect();
    if !d.items.iter().all(|(k, _)| known.contains(&trimmed(k).as_str())) {
        return None;
    }
    // A key recorded twice would play and save one of its values: keep the step instead.
    let names: Vec<String> = d.items.iter().map(|(k, _)| trimmed(k)).collect();
    if names.iter().enumerate().any(|(i, n)| names[..i].contains(n)) {
        return None;
    }
    let mut p = Map::new();
    for k in keys.iter() {
        // Only a missing key takes Photoshop's default: a key that is there with the wrong type
        // or a non-finite number keeps the step (`docs/atn.md`) rather than playing a default.
        let present = d.get(k.key()).is_some();
        let v = match k {
            G::Int(key, _, min, max, default) => {
                Value::from(if present { get_num(d, key)?.round().clamp(f64::from(*min), f64::from(*max)) as i32 } else { *default })
            }
            G::Enum(key, _, table, default) => Value::from(lookup(table, &if present { get_enum(d, key)? } else { (*default).to_string() })?),
            G::Bool(key, _, default) => Value::from(if present { get_bool(d, key)? } else { *default }),
            G::Color(key, _, default) => {
                if present {
                    Value::from(color_hex(get_obj(d, key)?)?)
                } else {
                    Value::from(*default)
                }
            }
        };
        p.insert(k.param().into(), v);
    }
    Some(step(id, Value::Object(p)))
}

/// The colour `id`'s effect plays with when the step names none (Neon Glow's glow colour), as
/// Photoshop's 0–255 components, unrounded, so the step saves the colour it plays with.
fn default_color(id: &str) -> Option<Descriptor> {
    let filter = id.strip_prefix("filter.gallery.").and_then(GalleryFilter::from_key)?;
    let [r, g, b, _] = GalleryEffect::new(filter).color;
    let c = |v: f32| D::Double(f64::from(v) * 255.0);
    Some(desc("RGBC").with("Rd  ", c(r)).with("Grn ", c(g)).with("Bl  ", c(b)))
}

/// Every key in recorded order, Photoshop's default (which is the command's) for what the step
/// leaves out. A setting the event has no key for (the Sketch effects' pinned colours, a mask
/// target), a fraction in an integer key, an integer outside its setting's range or a value
/// without a Photoshop code refuses the step.
pub(super) fn gallery_to(id: &str, p: &Value) -> Option<Descriptor> {
    let (code, _, keys) = GALLERY.iter().find(|(_, cmd, _)| *cmd == id)?;
    let allowed: Vec<&str> = keys.iter().map(G::param).chain(["target", RECORDED_EVENT]).collect();
    if !pixel_target(p) || !p.as_object().is_none_or(|m| m.iter().all(|(k, v)| v.is_null() || allowed.contains(&k.as_str()))) {
        return None;
    }
    let mut d = desc("null").with("GEfk", enm("GEft", code));
    for k in keys.iter() {
        let v = p.get(k.param()).filter(|v| !v.is_null());
        let out = match k {
            G::Int(_, _, min, max, default) => {
                let n = v.map_or(Some(f64::from(*default)), Value::as_f64)?;
                // Outside the setting's range, Photoshop would play it clamped and it would load
                // back as another value (`edgeWidth` 99 as 14).
                if n.fract() != 0.0 || !(f64::from(*min)..=f64::from(*max)).contains(&n) {
                    return None;
                }
                D::Integer(n as i32)
            }
            G::Enum(key, _, table, default) => enm(enum_type(key), v.map_or(Some(*default), |v| v.as_str().and_then(|s| lookup_back(table, s)))?),
            G::Bool(_, _, default) => D::Boolean(v.map_or(Some(*default), Value::as_bool)?),
            G::Color(..) => D::Descriptor(match v {
                Some(v) => color_desc(v.as_str()?)?,
                None => default_color(id)?,
            }),
        };
        d = d.with(k.key(), out);
    }
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::GalleryParamKind;

    fn params(m: Mapped) -> (String, Value) {
        match m {
            Mapped::Steps(mut s) if s.len() == 1 => s.remove(0),
            _ => panic!("not one step"),
        }
    }

    /// Every effect recorded with nothing but `GEfk` loads with Photoshop's defaults, saves
    /// back, and loads again as the same step (#10592).
    #[test]
    fn every_gallery_effect_with_its_defaults_round_trips() {
        assert_eq!(GALLERY.len(), GALLERY_IDS.len());
        for (code, id, _) in GALLERY {
            assert!(GALLERY_IDS.contains(id), "{id} is not in GALLERY_IDS");
            let (got, first) = params(gallery_from(&desc("null").with("GEfk", enm("GEft", code))));
            assert_eq!(got, *id, "{code}");
            let back = gallery_to(id, &first).unwrap_or_else(|| panic!("{id} {first} does not export"));
            assert_eq!(params(gallery_from(&back)), (id.to_string(), first.clone()), "{id}");
        }
    }

    /// Each effect's integer limits and defaults, enumeration default and Boolean default are
    /// the command's own (`GalleryFilter`'s schema), and every setting of the schema has a key.
    #[test]
    fn every_gallery_effect_matches_the_commands_schema() {
        for (code, id, keys) in GALLERY {
            let schema = GalleryFilter::from_key(id).unwrap_or_else(|| panic!("{id} has no schema")).params();
            for p in &schema {
                assert!(keys.iter().any(|k| k.param() == p.key), "{id}: {} has no key", p.key);
            }
            for k in keys.iter() {
                let kind = schema.iter().find(|p| p.key == k.param()).map(|p| &p.kind);
                match (k, kind) {
                    (G::Int(_, param, min, max, default), Some(GalleryParamKind::Range { min: lo, max: hi, default: d })) => {
                        assert_eq!((*min as f32, *max as f32, *default as f32), (*lo, *hi, *d), "{id} {param}");
                    }
                    (G::Enum(_, param, table, default), Some(GalleryParamKind::Choice(choices))) => {
                        assert_eq!(lookup(table, default), choices.first().copied(), "{id} {param}");
                        assert!(table.iter().all(|(_, ours)| choices.contains(ours)), "{id} {param}");
                    }
                    (G::Bool(_, param, default), Some(GalleryParamKind::Bool(d))) => assert_eq!(default, d, "{id} {param}"),
                    (G::Color(..), None) => {}
                    (k, kind) => panic!("{code} {id} {}: {kind:?}", k.param()),
                }
            }
        }
    }
}
