//! Design system: themes, colour tokens, radii, typography.
//!
//! - **Studio**: near-black surfaces, rounded cards, Inter + JetBrains Mono, soft violet
//!   accent. Modelled on the look of modern pro editors (such as Photoshop 2025).
//! - **Studio Light**: the same system on light surfaces.
//! - **Classic**: a deliberately Windows-2000-era look (grey bevels, square corners, navy selection)
//!   for people who prefer it.
//! - **Adwaita** (Light and Dark): the Studio layout in GNOME's libadwaita palette, so the app sits
//!   with the rest of a GNOME desktop.
//! - **Solarized Dark**: Ethan Schoonover's Solarized palette (base03/base02 surfaces, base0 text,
//!   Solarized blue accent) with the Studio layout.
//! - **Breeze** (Light and Dark): the Studio layout adapted to the look of KDE Plasma's Breeze
//!   (blue-grey surfaces, Breeze blue accent, 5 px corners), so the app sits with the rest of a
//!   Plasma desktop.
//!
//! - **Pro (Light Gray)** and **Pro (Light)**: Photoshop's third and fourth interface brightness
//!   levels (#B8B8B8 and #F0F0F0 panels) with the Pro layout (#2661).
//!
//! Widgets read [`Tokens::get`] instead of hard-coding colours, so every theme applies everywhere.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals};
use photocraft_engine::prefs::UiFontSize;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeKind {
    /// Photoshop-style Spectrum dark: flat charcoal panels, tab strips, blue accents (darkest brightness).
    Pro,
    /// Photoshop's default (second) interface brightness: #535353 panels, #282828 canvas (default).
    #[default]
    ProMedium,
    /// Photoshop's third interface brightness: #B8B8B8 panels (#2661).
    ProLightGray,
    /// Photoshop's fourth (lightest) interface brightness: #F0F0F0 panels (#2661).
    ProLight,
    Studio,
    StudioLight,
    Classic,
    /// GNOME's libadwaita light style.
    Adwaita,
    /// GNOME's libadwaita dark style.
    AdwaitaDark,
    /// Solarized Dark (Ethan Schoonover's palette): the base03 background, base02 panels and
    /// base0 body text, with the Solarized blue accent.
    SolarizedDark,
    /// An adaptation of KDE Plasma's Breeze Light look.
    BreezeLight,
    /// An adaptation of KDE Plasma's Breeze Dark look.
    BreezeDark,
    /// A low-contrast dark theme: muted violet-grey surfaces, softened text and a periwinkle accent.
    SoftDark,
    /// A low-contrast light theme: pale violet-grey surfaces, softened dark text and a periwinkle accent.
    SoftLight,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 14] = [
        ThemeKind::Pro,
        ThemeKind::ProMedium,
        ThemeKind::ProLightGray,
        ThemeKind::ProLight,
        ThemeKind::Studio,
        ThemeKind::StudioLight,
        ThemeKind::Classic,
        ThemeKind::SolarizedDark,
        ThemeKind::Adwaita,
        ThemeKind::AdwaitaDark,
        ThemeKind::BreezeLight,
        ThemeKind::BreezeDark,
        ThemeKind::SoftDark,
        ThemeKind::SoftLight,
    ];
    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Pro => "Pro (Dark)",
            ThemeKind::ProMedium => "Pro (Medium Gray)",
            ThemeKind::ProLightGray => "Pro (Light Gray)",
            ThemeKind::ProLight => "Pro (Light)",
            ThemeKind::Studio => "Studio (Dark)",
            ThemeKind::StudioLight => "Studio (Light)",
            ThemeKind::Classic => "Classic",
            ThemeKind::Adwaita => "Adwaita (Light)",
            ThemeKind::AdwaitaDark => "Adwaita (Dark)",
            ThemeKind::SolarizedDark => "Solarized Dark",
            ThemeKind::BreezeLight => "Breeze Light",
            ThemeKind::BreezeDark => "Breeze Dark",
            ThemeKind::SoftDark => "Soft Dark",
            ThemeKind::SoftLight => "Soft Light",
        }
    }
    /// The canonical name: `ui.set {theme}` accepts it and the Window › Theme commands are
    /// `window.theme.<id>`.
    pub fn id(self) -> &'static str {
        match self {
            ThemeKind::Pro => "pro",
            ThemeKind::ProMedium => "proMedium",
            ThemeKind::ProLightGray => "proLightGray",
            ThemeKind::ProLight => "proLight",
            ThemeKind::Studio => "studio",
            ThemeKind::StudioLight => "studioLight",
            ThemeKind::Classic => "classic",
            ThemeKind::Adwaita => "adwaita",
            ThemeKind::AdwaitaDark => "adwaitaDark",
            ThemeKind::SolarizedDark => "solarizedDark",
            ThemeKind::BreezeLight => "breezeLight",
            ThemeKind::BreezeDark => "breezeDark",
            ThemeKind::SoftDark => "softDark",
            ThemeKind::SoftLight => "softLight",
        }
    }
    /// Photoshop-grammar layout (tab strips, flat panels): the four Pro brightness levels.
    pub fn is_pro(self) -> bool {
        matches!(self, ThemeKind::Pro | ThemeKind::ProMedium | ThemeKind::ProLightGray | ThemeKind::ProLight)
    }
    /// Whether this theme belongs to the dark appearance family.
    pub fn is_dark(self) -> bool {
        matches!(
            self,
            ThemeKind::Studio
                | ThemeKind::Pro
                | ThemeKind::ProMedium
                | ThemeKind::SolarizedDark
                | ThemeKind::AdwaitaDark
                | ThemeKind::BreezeDark
                | ThemeKind::SoftDark
        )
    }
    /// Whether this theme belongs to the light appearance family.
    pub fn is_light(self) -> bool {
        !self.is_dark()
    }
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|k| *k == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().replace([' ', '_', '-', '(', ')'], "").as_str() {
            "pro" | "prodark" | "photoshop" | "dark" => Some(ThemeKind::Pro),
            "promedium" | "promediumgray" | "medium" | "mediumgray" => Some(ThemeKind::ProMedium),
            "prolightgray" | "lightgray" | "mediumlight" => Some(ThemeKind::ProLightGray),
            "prolight" | "prolightest" | "lightest" | "light" => Some(ThemeKind::ProLight),
            "studio" | "studiodark" => Some(ThemeKind::Studio),
            "studiolight" => Some(ThemeKind::StudioLight),
            "classic" | "win2000" | "retro" => Some(ThemeKind::Classic),
            "adwaita" | "adwaitalight" | "gnome" | "gnomelight" => Some(ThemeKind::Adwaita),
            "adwaitadark" | "gnomedark" => Some(ThemeKind::AdwaitaDark),
            "solarizeddark" | "solarized" => Some(ThemeKind::SolarizedDark),
            "breezelight" | "breeze" | "kde" | "kdelight" | "plasma" | "plasmalight" => Some(ThemeKind::BreezeLight),
            "breezedark" | "kdedark" | "plasmadark" => Some(ThemeKind::BreezeDark),
            "softdark" => Some(ThemeKind::SoftDark),
            "softlight" => Some(ThemeKind::SoftLight),
            _ => None,
        }
    }
}

/// Colour and shape tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub kind: ThemeKind,
    /// Window chrome (title bar, toolbars).
    pub chrome: Color32,
    /// Canvas surround.
    pub canvas: Color32,
    /// Dot colour of the canvas grid pattern.
    pub canvas_dot: Color32,
    /// Dock background (behind cards).
    pub dock: Color32,
    /// Cards / panels.
    pub card: Color32,
    pub card_border: Color32,
    /// Inputs, fields, dropdowns.
    pub field: Color32,
    pub field_border: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub icon: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_border: Color32,
    pub accent_text: Color32,
    pub separator: Color32,
    pub shadow: Color32,
    pub primary_bg: Color32,
    pub primary_text: Color32,
    pub danger: Color32,
    pub warning: Color32,
    pub radius_sm: f32,
    pub radius: f32,
    pub radius_lg: f32,
    /// Classic theme draws 3D bevels instead of flat fills.
    pub bevel: bool,
    /// Pro (Photoshop-grammar) layout: tab strips, flat panels, checkboxes, pill buttons.
    pub pro: bool,
    /// Toolbar colour chips are large and round, stacked, with the colour buttons below (Studio).
    pub round_chips: bool,
    /// Panel tab-strip background (Pro).
    pub tab_strip: Color32,
    /// Selected list row (layers, history).
    pub row_selected: Color32,
    /// Scope plot background (histogram, vectorscope, clipping diagnostics).
    pub histogram_bg: Color32,
    /// Channel intensity of scope outlines and hue markers.
    pub histogram_level: u8,
    /// The custom title bar's Close button while hovered (Windows' red), and its glyph.
    pub caption_close: Color32,
    pub caption_close_text: Color32,
    /// Dims the main window behind a modal that takes all input (Camera Raw).
    pub scrim: Color32,
}

impl Tokens {
    /// Tool outlines need contrast on arbitrary document pixels, independently of UI theme.
    pub(crate) fn cursor_outline() -> [Color32; 2] {
        [Color32::from_black_alpha(160), Color32::from_white_alpha(235)]
    }

    pub fn for_kind(kind: ThemeKind) -> Self {
        match kind {
            // Sampled from Photoshop 2026's default brightness (raw display values).
            ThemeKind::ProMedium => Tokens {
                kind,
                chrome: Color32::from_rgb(83, 83, 83),
                dock: Color32::from_rgb(66, 66, 66),
                card: Color32::from_rgb(83, 83, 83),
                card_border: Color32::from_rgb(66, 66, 66),
                field: Color32::from_rgb(69, 69, 69),
                field_border: Color32::from_rgb(104, 104, 104),
                hover: Color32::from_rgb(98, 98, 98),
                pressed: Color32::from_rgb(112, 112, 112),
                text: Color32::from_rgb(238, 238, 238),
                text_dim: Color32::from_rgb(212, 212, 212),
                text_faint: Color32::from_rgb(160, 160, 160),
                icon: Color32::from_rgb(226, 226, 226),
                accent_soft: Color32::from_rgb(110, 110, 110),
                separator: Color32::from_rgb(62, 62, 62),
                tab_strip: Color32::from_rgb(74, 74, 74),
                row_selected: Color32::from_rgb(107, 107, 107),
                ..Tokens::for_kind(ThemeKind::Pro)
            },
            // Photoshop's medium-light brightness: #B8B8B8 panels, dark text, a mid-grey canvas.
            ThemeKind::ProLightGray => Tokens {
                kind,
                chrome: Color32::from_rgb(184, 184, 184),
                canvas: Color32::from_rgb(160, 160, 160),
                canvas_dot: Color32::from_rgb(160, 160, 160),
                dock: Color32::from_rgb(168, 168, 168),
                card: Color32::from_rgb(184, 184, 184),
                card_border: Color32::from_rgb(160, 160, 160),
                field: Color32::from_rgb(214, 214, 214),
                field_border: Color32::from_rgb(140, 140, 140),
                hover: Color32::from_rgb(198, 198, 198),
                pressed: Color32::from_rgb(170, 170, 170),
                text: Color32::from_rgb(20, 20, 20),
                text_dim: Color32::from_rgb(52, 52, 52),
                text_faint: Color32::from_rgb(96, 96, 96),
                icon: Color32::from_rgb(38, 38, 38),
                accent_soft: Color32::from_rgb(208, 208, 208),
                accent_text: Color32::from_rgb(0, 66, 150),
                separator: Color32::from_rgb(158, 158, 158),
                shadow: Color32::from_black_alpha(70),
                tab_strip: Color32::from_rgb(170, 170, 170),
                row_selected: Color32::from_rgb(212, 212, 212),
                ..Tokens::for_kind(ThemeKind::Pro)
            },
            // Photoshop's lightest brightness: #F0F0F0 panels, dark text, a light-grey canvas.
            ThemeKind::ProLight => Tokens {
                kind,
                chrome: Color32::from_rgb(240, 240, 240),
                canvas: Color32::from_rgb(214, 214, 214),
                canvas_dot: Color32::from_rgb(214, 214, 214),
                dock: Color32::from_rgb(226, 226, 226),
                card: Color32::from_rgb(240, 240, 240),
                card_border: Color32::from_rgb(214, 214, 214),
                field: Color32::WHITE,
                field_border: Color32::from_rgb(186, 186, 186),
                hover: Color32::from_rgb(226, 226, 226),
                pressed: Color32::from_rgb(206, 206, 206),
                text: Color32::from_rgb(20, 20, 20),
                text_dim: Color32::from_rgb(64, 64, 64),
                text_faint: Color32::from_rgb(120, 120, 120),
                icon: Color32::from_rgb(46, 46, 46),
                accent_soft: Color32::from_rgb(214, 214, 214),
                accent_text: Color32::from_rgb(16, 98, 196),
                separator: Color32::from_rgb(214, 214, 214),
                shadow: Color32::from_black_alpha(50),
                tab_strip: Color32::from_rgb(226, 226, 226),
                row_selected: Color32::from_rgb(204, 204, 204),
                ..Tokens::for_kind(ThemeKind::Pro)
            },
            ThemeKind::Pro => Tokens {
                kind,
                chrome: Color32::from_rgb(50, 50, 50),
                canvas: Color32::from_rgb(40, 40, 40),
                canvas_dot: Color32::from_rgb(40, 40, 40),
                dock: Color32::from_rgb(30, 30, 30),
                card: Color32::from_rgb(50, 50, 50),
                card_border: Color32::from_rgb(30, 30, 30),
                field: Color32::from_rgb(36, 36, 36),
                field_border: Color32::from_rgb(74, 74, 74),
                hover: Color32::from_rgb(66, 66, 66),
                pressed: Color32::from_rgb(78, 78, 78),
                text: Color32::from_rgb(222, 222, 222),
                text_dim: Color32::from_rgb(178, 178, 178),
                text_faint: Color32::from_rgb(128, 128, 128),
                icon: Color32::from_rgb(200, 200, 200),
                accent: Color32::from_rgb(55, 142, 240),
                accent_soft: Color32::from_rgb(78, 78, 78),
                accent_border: Color32::from_rgb(55, 142, 240),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(30, 30, 30),
                shadow: Color32::from_black_alpha(150),
                primary_bg: Color32::from_rgb(55, 142, 240),
                primary_text: Color32::WHITE,
                danger: Color32::from_rgb(236, 91, 98),
                warning: Color32::from_rgb(232, 176, 70),
                radius_sm: 3.0,
                radius: 4.0,
                radius_lg: 6.0,
                bevel: false,
                pro: true,
                round_chips: false,
                tab_strip: Color32::from_rgb(38, 38, 38),
                row_selected: Color32::from_rgb(82, 82, 82),
                histogram_bg: Color32::from_rgb(40, 40, 40),
                histogram_level: 225,
                caption_close: Color32::from_rgb(196, 43, 28),
                caption_close_text: Color32::WHITE,
                scrim: Color32::from_black_alpha(110),
            },
            ThemeKind::Studio => Tokens {
                kind,
                chrome: Color32::from_rgb(20, 20, 21),
                canvas: Color32::from_rgb(14, 14, 15),
                canvas_dot: Color32::from_rgb(46, 46, 50),
                dock: Color32::from_rgb(17, 17, 18),
                card: Color32::from_rgb(26, 26, 28),
                card_border: Color32::from_rgb(40, 40, 44),
                field: Color32::from_rgb(35, 35, 38),
                field_border: Color32::from_rgb(52, 52, 57),
                hover: Color32::from_rgb(44, 44, 48),
                pressed: Color32::from_rgb(56, 56, 62),
                text: Color32::from_rgb(236, 236, 240),
                text_dim: Color32::from_rgb(150, 150, 158),
                text_faint: Color32::from_rgb(96, 96, 104),
                icon: Color32::from_rgb(196, 196, 204),
                accent: Color32::from_rgb(139, 124, 246),
                accent_soft: Color32::from_rgba_unmultiplied(139, 124, 246, 46),
                accent_border: Color32::from_rgba_unmultiplied(160, 148, 255, 110),
                accent_text: Color32::from_rgb(214, 208, 255),
                separator: Color32::from_rgb(38, 38, 42),
                shadow: Color32::from_black_alpha(140),
                primary_bg: Color32::from_rgb(246, 246, 248),
                primary_text: Color32::from_rgb(12, 12, 14),
                danger: Color32::from_rgb(240, 96, 96),
                warning: Color32::from_rgb(240, 190, 90),
                radius_sm: 6.0,
                radius: 8.0,
                radius_lg: 12.0,
                bevel: false,
                pro: false,
                round_chips: true,
                tab_strip: Color32::TRANSPARENT,
                row_selected: Color32::TRANSPARENT,
                histogram_bg: Color32::from_rgb(14, 14, 15),
                histogram_level: 225,
                caption_close: Color32::from_rgb(196, 43, 28),
                caption_close_text: Color32::WHITE,
                scrim: Color32::from_black_alpha(110),
            },
            ThemeKind::StudioLight => Tokens {
                kind,
                chrome: Color32::from_rgb(246, 246, 248),
                canvas: Color32::from_rgb(226, 226, 230),
                canvas_dot: Color32::from_rgb(200, 200, 206),
                dock: Color32::from_rgb(240, 240, 243),
                card: Color32::from_rgb(252, 252, 253),
                card_border: Color32::from_rgb(222, 222, 228),
                field: Color32::from_rgb(242, 242, 245),
                field_border: Color32::from_rgb(214, 214, 220),
                hover: Color32::from_rgb(232, 232, 237),
                pressed: Color32::from_rgb(220, 220, 226),
                text: Color32::from_rgb(24, 24, 28),
                text_dim: Color32::from_rgb(96, 96, 106),
                text_faint: Color32::from_rgb(150, 150, 160),
                icon: Color32::from_rgb(60, 60, 68),
                accent: Color32::from_rgb(108, 92, 231),
                accent_soft: Color32::from_rgba_unmultiplied(108, 92, 231, 36),
                accent_border: Color32::from_rgba_unmultiplied(108, 92, 231, 120),
                accent_text: Color32::from_rgb(80, 64, 200),
                separator: Color32::from_rgb(226, 226, 232),
                shadow: Color32::from_black_alpha(50),
                primary_bg: Color32::from_rgb(20, 20, 24),
                primary_text: Color32::from_rgb(250, 250, 252),
                danger: Color32::from_rgb(210, 60, 60),
                warning: Color32::from_rgb(190, 130, 20),
                radius_sm: 6.0,
                radius: 8.0,
                radius_lg: 12.0,
                bevel: false,
                pro: false,
                round_chips: true,
                tab_strip: Color32::TRANSPARENT,
                row_selected: Color32::TRANSPARENT,
                histogram_bg: Color32::from_gray(40),
                histogram_level: 240,
                caption_close: Color32::from_rgb(196, 43, 28),
                caption_close_text: Color32::WHITE,
                scrim: Color32::from_black_alpha(110),
            },
            ThemeKind::Classic => Tokens {
                kind,
                chrome: Color32::from_rgb(212, 208, 200),
                canvas: Color32::from_rgb(128, 128, 128),
                canvas_dot: Color32::from_rgb(128, 128, 128),
                dock: Color32::from_rgb(212, 208, 200),
                card: Color32::from_rgb(212, 208, 200),
                card_border: Color32::from_rgb(128, 128, 128),
                field: Color32::WHITE,
                field_border: Color32::from_rgb(128, 128, 128),
                hover: Color32::from_rgb(226, 222, 214),
                pressed: Color32::from_rgb(190, 186, 178),
                text: Color32::BLACK,
                text_dim: Color32::from_rgb(64, 64, 64),
                text_faint: Color32::from_rgb(128, 128, 128),
                icon: Color32::BLACK,
                accent: Color32::from_rgb(10, 36, 106),
                accent_soft: Color32::from_rgb(10, 36, 106),
                accent_border: Color32::from_rgb(10, 36, 106),
                accent_text: Color32::WHITE,
                separator: Color32::from_rgb(128, 128, 128),
                shadow: Color32::from_black_alpha(0),
                primary_bg: Color32::from_rgb(212, 208, 200),
                primary_text: Color32::BLACK,
                danger: Color32::from_rgb(160, 0, 0),
                warning: Color32::from_rgb(128, 96, 0),
                radius_sm: 0.0,
                radius: 0.0,
                radius_lg: 0.0,
                bevel: true,
                pro: false,
                round_chips: false,
                tab_strip: Color32::from_rgb(212, 208, 200),
                row_selected: Color32::from_rgb(10, 36, 106),
                histogram_bg: Color32::from_gray(40),
                histogram_level: 240,
                caption_close: Color32::from_rgb(196, 43, 28),
                caption_close_text: Color32::WHITE,
                scrim: Color32::from_black_alpha(110),
            },
            // libadwaita's named colours (window, sidebar, card, view, accent, destructive) and
            // radii (6 px buttons, 12 px cards), mapped onto the Studio layout. GNOME's close
            // button is a neutral circle, not a red one, so the caption colours stay grey.
            ThemeKind::Adwaita => Tokens {
                kind,
                chrome: Color32::WHITE,
                canvas: Color32::from_rgb(222, 222, 223),
                canvas_dot: Color32::from_rgb(222, 222, 223),
                dock: Color32::from_rgb(235, 235, 237),
                card: Color32::WHITE,
                card_border: Color32::from_rgb(218, 218, 221),
                field: Color32::from_rgb(235, 235, 237),
                field_border: Color32::from_rgb(218, 218, 221),
                hover: Color32::from_rgb(226, 226, 229),
                pressed: Color32::from_rgb(212, 212, 216),
                text: Color32::from_rgb(50, 50, 54),
                text_dim: Color32::from_rgb(100, 100, 104),
                text_faint: Color32::from_rgb(155, 155, 160),
                icon: Color32::from_rgb(56, 56, 60),
                accent: Color32::from_rgb(53, 132, 228),
                accent_soft: Color32::from_rgba_unmultiplied(53, 132, 228, 40),
                accent_border: Color32::from_rgba_unmultiplied(53, 132, 228, 130),
                accent_text: Color32::from_rgb(28, 113, 216),
                separator: Color32::from_rgb(222, 222, 225),
                shadow: Color32::from_black_alpha(45),
                primary_bg: Color32::from_rgb(53, 132, 228),
                primary_text: Color32::WHITE,
                danger: Color32::from_rgb(192, 28, 40),
                warning: Color32::from_rgb(156, 110, 3),
                radius_sm: 6.0,
                radius: 9.0,
                radius_lg: 12.0,
                bevel: false,
                pro: false,
                round_chips: true,
                tab_strip: Color32::TRANSPARENT,
                row_selected: Color32::TRANSPARENT,
                histogram_bg: Color32::from_gray(40),
                histogram_level: 240,
                caption_close: Color32::from_rgb(212, 212, 216),
                caption_close_text: Color32::from_rgb(50, 50, 54),
                scrim: Color32::from_black_alpha(110),
            },
            ThemeKind::AdwaitaDark => Tokens {
                kind,
                chrome: Color32::from_rgb(46, 46, 50),
                canvas: Color32::from_rgb(29, 29, 32),
                canvas_dot: Color32::from_rgb(29, 29, 32),
                dock: Color32::from_rgb(34, 34, 38),
                card: Color32::from_rgb(54, 54, 58),
                card_border: Color32::from_rgb(64, 64, 68),
                field: Color32::from_rgb(64, 64, 68),
                field_border: Color32::from_rgb(76, 76, 80),
                hover: Color32::from_rgb(72, 72, 76),
                pressed: Color32::from_rgb(86, 86, 90),
                text: Color32::WHITE,
                text_dim: Color32::from_rgb(192, 192, 194),
                text_faint: Color32::from_rgb(145, 145, 148),
                icon: Color32::from_rgb(235, 235, 237),
                accent: Color32::from_rgb(53, 132, 228),
                accent_soft: Color32::from_rgba_unmultiplied(53, 132, 228, 60),
                accent_border: Color32::from_rgba_unmultiplied(120, 174, 237, 140),
                accent_text: Color32::from_rgb(120, 174, 237),
                separator: Color32::from_rgb(24, 24, 27),
                shadow: Color32::from_black_alpha(140),
                primary_bg: Color32::from_rgb(53, 132, 228),
                primary_text: Color32::WHITE,
                danger: Color32::from_rgb(255, 123, 99),
                warning: Color32::from_rgb(248, 228, 92),
                radius_sm: 6.0,
                radius: 9.0,
                radius_lg: 12.0,
                bevel: false,
                pro: false,
                round_chips: true,
                tab_strip: Color32::TRANSPARENT,
                row_selected: Color32::TRANSPARENT,
                histogram_bg: Color32::from_rgb(29, 29, 32),
                histogram_level: 225,
                caption_close: Color32::from_rgb(72, 72, 76),
                caption_close_text: Color32::WHITE,
                scrim: Color32::from_black_alpha(110),
            },
            // The light counterpart, adapted the same way: our own values, chosen a few levels
            // from Breeze Light's surfaces so the app blends into a Plasma desktop without being a
            // copy. Breeze's own inactive text and link blue are only about 2.8:1 on such
            // surfaces, so the secondary and faint text, the accent and danger text and the status
            // hues are darker to hold 4.5:1 (checked by the tests), and the primary button is a
            // deeper blue that takes white text.
            ThemeKind::BreezeLight => Tokens {
                kind,
                chrome: Color32::from_rgb(219, 222, 226),
                canvas: Color32::from_rgb(219, 222, 226),
                canvas_dot: Color32::from_rgb(219, 222, 226),
                dock: Color32::from_rgb(224, 227, 231),
                card: Color32::from_rgb(236, 238, 241),
                card_border: Color32::from_rgb(203, 208, 213),
                field: Color32::from_rgb(249, 250, 252),
                field_border: Color32::from_rgb(184, 191, 198),
                hover: Color32::from_rgb(226, 230, 235),
                pressed: Color32::from_rgb(208, 214, 221),
                text: Color32::from_rgb(33, 37, 42),
                text_dim: Color32::from_rgb(54, 63, 72),
                text_faint: Color32::from_rgb(81, 90, 100),
                icon: Color32::from_rgb(52, 58, 64),
                accent: Color32::from_rgb(58, 172, 232),
                accent_soft: Color32::from_rgba_unmultiplied(58, 172, 232, 48),
                accent_border: Color32::from_rgba_unmultiplied(58, 172, 232, 170),
                accent_text: Color32::from_rgb(30, 94, 136),
                separator: Color32::from_rgb(214, 218, 222),
                shadow: Color32::from_black_alpha(45),
                primary_bg: Color32::from_rgb(35, 122, 180),
                primary_text: Color32::WHITE,
                danger: Color32::from_rgb(170, 50, 64),
                warning: Color32::from_rgb(153, 72, 0),
                radius_sm: 5.0,
                radius: 5.0,
                radius_lg: 6.0,
                bevel: false,
                pro: false,
                round_chips: true,
                tab_strip: Color32::TRANSPARENT,
                row_selected: Color32::TRANSPARENT,
                histogram_bg: Color32::from_rgb(33, 37, 42),
                histogram_level: 240,
                caption_close: Color32::from_rgb(213, 51, 60),
                caption_close_text: Color32::WHITE,
                scrim: Color32::from_black_alpha(110),
            },
            // An adaptation of KDE Plasma's Breeze Dark look on the Studio layout, not a copy: our
            // own values, chosen a few levels from Breeze Dark's surfaces (the darkest canvas, then
            // the dock, the cards, and the chrome and controls) so the app blends into a Plasma
            // desktop. Near-white text, the blue accent and the orange warning hue sit close to
            // Breeze's, with its 5 px frame radius.
            // Readability wins over fidelity: every text colour holds 4.5:1 on every surface and on
            // the hover, pressed and selected states (checked by the tests), so the secondary and
            // faint text, the accent and danger text and the status hues are lighter than Breeze's,
            // hover and pressed are small steps, and the primary button is a deeper blue that takes
            // white text. Outlines and those steps are our own tints.
            ThemeKind::BreezeDark => Tokens {
                kind,
                chrome: Color32::from_rgb(38, 42, 48),
                canvas: Color32::from_rgb(17, 20, 24),
                canvas_dot: Color32::from_rgb(17, 20, 24),
                dock: Color32::from_rgb(26, 29, 34),
                card: Color32::from_rgb(29, 33, 38),
                card_border: Color32::from_rgb(52, 56, 61),
                field: Color32::from_rgb(38, 42, 48),
                field_border: Color32::from_rgb(68, 73, 79),
                hover: Color32::from_rgb(50, 54, 59),
                pressed: Color32::from_rgb(58, 63, 69),
                text: Color32::from_rgb(250, 251, 253),
                text_dim: Color32::from_rgb(200, 206, 213),
                text_faint: Color32::from_rgb(164, 170, 177),
                icon: Color32::from_rgb(236, 238, 240),
                accent: Color32::from_rgb(58, 172, 232),
                accent_soft: Color32::from_rgba_unmultiplied(58, 172, 232, 40),
                accent_border: Color32::from_rgba_unmultiplied(58, 172, 232, 160),
                accent_text: Color32::from_rgb(80, 184, 238),
                separator: Color32::from_rgb(46, 50, 54),
                shadow: Color32::from_black_alpha(150),
                primary_bg: Color32::from_rgb(30, 118, 176),
                primary_text: Color32::WHITE,
                danger: Color32::from_rgb(227, 111, 123),
                warning: Color32::from_rgb(244, 120, 6),
                radius_sm: 5.0,
                radius: 5.0,
                radius_lg: 6.0,
                bevel: false,
                pro: false,
                round_chips: true,
                tab_strip: Color32::TRANSPARENT,
                row_selected: Color32::TRANSPARENT,
                histogram_bg: Color32::from_rgb(17, 20, 24),
                histogram_level: 225,
                caption_close: Color32::from_rgb(213, 51, 60),
                caption_close_text: Color32::WHITE,
                scrim: Color32::from_black_alpha(110),
            },
            // Soft Dark: our own low-contrast palette on the Studio layout. Muted violet-grey
            // surfaces (never near-black), softened off-white text and a periwinkle accent; the
            // text colours still hold 4.5:1 on every surface (checked by the tests).
            ThemeKind::SoftDark => Tokens {
                kind,
                chrome: Color32::from_rgb(58, 58, 68),
                canvas: Color32::from_rgb(40, 40, 48),
                canvas_dot: Color32::from_rgb(40, 40, 48),
                dock: Color32::from_rgb(47, 47, 56),
                card: Color32::from_rgb(54, 54, 64),
                card_border: Color32::from_rgb(70, 70, 82),
                field: Color32::from_rgb(64, 64, 75),
                field_border: Color32::from_rgb(84, 84, 98),
                hover: Color32::from_rgb(68, 68, 80),
                pressed: Color32::from_rgb(74, 74, 88),
                text: Color32::from_rgb(238, 238, 246),
                text_dim: Color32::from_rgb(212, 212, 224),
                text_faint: Color32::from_rgb(190, 190, 206),
                icon: Color32::from_rgb(214, 214, 226),
                accent: Color32::from_rgb(138, 154, 232),
                accent_soft: Color32::from_rgba_unmultiplied(138, 154, 232, 48),
                accent_border: Color32::from_rgba_unmultiplied(138, 154, 232, 150),
                accent_text: Color32::from_rgb(190, 200, 252),
                separator: Color32::from_rgb(36, 36, 44),
                shadow: Color32::from_black_alpha(110),
                primary_bg: Color32::from_rgb(86, 104, 196),
                primary_text: Color32::WHITE,
                danger: Color32::from_rgb(250, 160, 172),
                warning: Color32::from_rgb(240, 200, 130),
                radius_sm: 7.0,
                radius: 10.0,
                radius_lg: 14.0,
                bevel: false,
                pro: false,
                round_chips: true,
                tab_strip: Color32::TRANSPARENT,
                row_selected: Color32::TRANSPARENT,
                histogram_bg: Color32::from_rgb(40, 40, 48),
                histogram_level: 220,
                caption_close: Color32::from_rgb(200, 72, 86),
                caption_close_text: Color32::WHITE,
                scrim: Color32::from_black_alpha(100),
            },
            // Soft Light: pale violet-grey surfaces, softened dark-violet text (not black) and a
            // periwinkle accent, with the same generous radii as Soft Dark.
            ThemeKind::SoftLight => Tokens {
                kind,
                chrome: Color32::from_rgb(232, 230, 236),
                canvas: Color32::from_rgb(218, 216, 224),
                canvas_dot: Color32::from_rgb(218, 216, 224),
                dock: Color32::from_rgb(226, 224, 231),
                card: Color32::from_rgb(240, 238, 243),
                card_border: Color32::from_rgb(208, 205, 215),
                field: Color32::from_rgb(250, 249, 252),
                field_border: Color32::from_rgb(190, 187, 201),
                hover: Color32::from_rgb(228, 226, 234),
                pressed: Color32::from_rgb(212, 209, 221),
                text: Color32::from_rgb(44, 42, 56),
                text_dim: Color32::from_rgb(62, 60, 76),
                text_faint: Color32::from_rgb(80, 78, 96),
                icon: Color32::from_rgb(64, 62, 80),
                accent: Color32::from_rgb(110, 128, 216),
                accent_soft: Color32::from_rgba_unmultiplied(110, 128, 216, 44),
                accent_border: Color32::from_rgba_unmultiplied(110, 128, 216, 170),
                accent_text: Color32::from_rgb(46, 58, 146),
                separator: Color32::from_rgb(210, 207, 218),
                shadow: Color32::from_black_alpha(40),
                primary_bg: Color32::from_rgb(80, 98, 190),
                primary_text: Color32::WHITE,
                danger: Color32::from_rgb(166, 44, 62),
                warning: Color32::from_rgb(140, 72, 0),
                radius_sm: 7.0,
                radius: 10.0,
                radius_lg: 14.0,
                bevel: false,
                pro: false,
                round_chips: true,
                tab_strip: Color32::TRANSPARENT,
                row_selected: Color32::TRANSPARENT,
                histogram_bg: Color32::from_rgb(44, 42, 56),
                histogram_level: 240,
                caption_close: Color32::from_rgb(200, 60, 72),
                caption_close_text: Color32::WHITE,
                scrim: Color32::from_black_alpha(90),
            },
            // Ethan Schoonover's Solarized Dark palette: base03 canvas, base02 surfaces, base0
            // body text and the Solarized blue accent. https://ethanschoonover.com/solarized/
            // (MIT; see LICENSE-solarized.txt)
            ThemeKind::SolarizedDark => Tokens {
                kind,
                chrome: Color32::from_rgb(7, 54, 66),
                canvas: Color32::from_rgb(0, 43, 54),
                canvas_dot: Color32::from_rgb(10, 61, 74),
                dock: Color32::from_rgb(0, 37, 47),
                card: Color32::from_rgb(7, 54, 66),
                card_border: Color32::from_rgb(0, 37, 47),
                field: Color32::from_rgb(0, 37, 47),
                field_border: Color32::from_rgb(21, 80, 95),
                hover: Color32::from_rgb(14, 74, 90),
                pressed: Color32::from_rgb(18, 89, 107),
                text: Color32::from_rgb(131, 148, 150),
                text_dim: Color32::from_rgb(101, 123, 131),
                text_faint: Color32::from_rgb(88, 110, 117),
                icon: Color32::from_rgb(147, 161, 161),
                accent: Color32::from_rgb(38, 139, 210),
                accent_soft: Color32::from_rgba_unmultiplied(38, 139, 210, 46),
                accent_border: Color32::from_rgba_unmultiplied(38, 139, 210, 120),
                accent_text: Color32::from_rgb(253, 246, 227),
                separator: Color32::from_rgb(0, 37, 47),
                shadow: Color32::from_black_alpha(150),
                primary_bg: Color32::from_rgb(38, 139, 210),
                primary_text: Color32::from_rgb(253, 246, 227),
                danger: Color32::from_rgb(220, 50, 47),
                warning: Color32::from_rgb(181, 137, 0),
                radius_sm: 6.0,
                radius: 8.0,
                radius_lg: 12.0,
                bevel: false,
                pro: false,
                round_chips: true,
                tab_strip: Color32::TRANSPARENT,
                row_selected: Color32::TRANSPARENT,
                histogram_bg: Color32::from_rgb(0, 43, 54),
                histogram_level: 225,
                caption_close: Color32::from_rgb(220, 50, 47),
                caption_close_text: Color32::from_rgb(253, 246, 227),
                scrim: Color32::from_black_alpha(110),
            },
        }
    }

    /// Tokens for the active theme (stored in egui's context data by [`apply`]).
    pub fn get(ctx: &egui::Context) -> Tokens {
        ctx.data(|d| d.get_temp::<Tokens>(egui::Id::unique("photocraft-theme"))).unwrap_or_else(|| Tokens::for_kind(ThemeKind::Studio))
    }

    /// Thin RGB outlines; the filled bands use the same hues with subdued coverage.
    pub fn histogram_color(&self, mask: u8) -> Color32 {
        let v = self.histogram_level;
        Color32::from_rgb(if mask & 1 != 0 { v } else { 0 }, if mask & 2 != 0 { v } else { 0 }, if mask & 4 != 0 { v } else { 0 })
    }

    pub fn histogram_fill(&self, mask: u8) -> Color32 {
        self.histogram_color(mask).gamma_multiply(0.3)
    }

    /// Semantic label hues stay recognisable across themes; None keeps the normal row surface.
    pub fn layer_label_colors(&self, label: photocraft_doc::LabelColor) -> Option<(Color32, Color32)> {
        use photocraft_doc::LabelColor;
        let (swatch, eye) = match label {
            LabelColor::None => return None,
            LabelColor::Red => ([0xFC, 0x5D, 0x5B], [0x9F, 0x2F, 0x30]),
            LabelColor::Orange => ([0xF7, 0x97, 0x44], [0x93, 0x4F, 0x0C]),
            LabelColor::Yellow => ([0xDC, 0xD6, 0x4B], [0x99, 0x78, 0x0C]),
            LabelColor::Green => ([0x85, 0xDC, 0x6A], [0x4E, 0x71, 0x2E]),
            LabelColor::Seafoam => ([0x1C, 0x84, 0x88], [0x0B, 0x54, 0x4F]),
            LabelColor::Blue => ([0x78, 0xAD, 0xF6], [0x41, 0x5B, 0x87]),
            LabelColor::Indigo => ([0x54, 0x4B, 0xE7], [0x36, 0x34, 0x8E]),
            LabelColor::Magenta => ([0xCC, 0x1A, 0x7E], [0x98, 0x1B, 0x51]),
            LabelColor::Fuchsia => ([0xB0, 0x14, 0xC0], [0x71, 0x0F, 0x74]),
            LabelColor::Violet => ([0x91, 0x76, 0xD5], [0x5D, 0x3F, 0x8E]),
            LabelColor::Gray => ([0x9C, 0x9C, 0x9C], [0x57, 0x57, 0x57]),
        };
        let rgb = |[r, g, b]: [u8; 3]| Color32::from_rgb(r, g, b);
        Some((rgb(swatch), rgb(eye)))
    }

    pub fn layer_label_icon(&self, label: photocraft_doc::LabelColor) -> Color32 {
        if label == photocraft_doc::LabelColor::None { self.icon } else { Color32::from_gray(226) }
    }

    /// Analysis colours are semantic hues, independent of the application accent palette.
    pub fn scope_hue(&self, h: f32, s: f32) -> Color32 {
        let v = f32::from(self.histogram_level) / 255.0;
        let f = |offset: f32| {
            let k = (offset + h * 6.0).rem_euclid(6.0);
            (v * (1.0 - s * k.min(4.0 - k).clamp(0.0, 1.0)) * 255.0) as u8
        };
        Color32::from_rgb(f(5.0), f(3.0), f(1.0))
    }

    pub fn histogram_background(&self) -> Color32 {
        self.histogram_bg
    }

    /// The background behind selected text in a text field (#3789): a subtle shade rather than
    /// egui's default blue. The monochrome Pro themes use a grey so a selection doesn't read as a
    /// second accent; coloured themes keep their own accent. Scoped to text fields by
    /// [`crate::widgets::text_field`], because `Visuals::selection` also fills selected list rows
    /// and slider trails, which must keep the theme's.
    pub fn field_selection(&self) -> Color32 {
        if self.pro { Color32::from_gray(if self.dark() { 96 } else { 201 }) } else { self.accent_soft }
    }

    /// The 1 px ring round a slider knob, if the theme draws one: the Pro layouts' dark ring, and
    /// in the light themes the dim text colour, so their white knob stands out from a pale panel
    /// (#7927). The dark Studio-layout themes need none; Classic draws a bevelled knob instead.
    pub fn knob_outline(&self) -> Option<Color32> {
        if self.pro {
            Some(Color32::from_gray(40))
        } else if self.dark() {
            None
        } else {
            Some(self.text_dim)
        }
    }

    pub fn dark(&self) -> bool {
        self.kind.is_dark()
    }
}

/// Register Inter (UI) and JetBrains Mono (numbers) plus named weights.
pub fn install_fonts(ctx: &egui::Context) {
    install_fonts_with(ctx, crate::cjk_fonts::Sources::system());
}

/// [`install_fonts`] with the CJK and other-script fallback fonts taken from `cjk` (tests swap the sources).
pub fn install_fonts_with(ctx: &egui::Context, cjk: crate::cjk_fonts::Sources) {
    // egui's default fonts stay last in the stack, for symbols and emoji Inter lacks.
    let mut fonts = crate::egui_fonts::definitions();
    // The bundled fonts are inflated on first use (photocraft_text::fonts). Should one ever fail
    // to inflate (empty bytes), it is left out and egui's default fonts draw that text.
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| -> Option<String> {
        if bytes.is_empty() {
            log::error!("bundled font {name} is unavailable; using the default UI font");
            return None;
        }
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
        Some(name.to_owned())
    };
    let inter = add(&mut fonts, "Inter", photocraft_text::fonts::INTER_REGULAR.as_slice());
    let medium = add(&mut fonts, "Inter-Medium", photocraft_text::fonts::INTER_MEDIUM.as_slice());
    let semibold = add(&mut fonts, "Inter-SemiBold", photocraft_text::fonts::INTER_SEMIBOLD.as_slice());
    let mono = add(&mut fonts, "JetBrainsMono", photocraft_text::fonts::JETBRAINS_MONO_REGULAR.as_slice());
    if let Some(inter) = inter {
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, inter);
    }
    if let Some(mono) = mono {
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, mono);
    }
    // The optional craft-fonts input carries Japanese, Simplified Chinese, Korean, Arabic, Thai and
    // other faces. They are not registered into this static stack: `cjk_fonts` installs a
    // per-character font provider for them, so a broad Simplified Chinese face is not put in the
    // family stack ahead of the Japanese or Korean face, and a locale without a Traditional Chinese
    // craft face leaves Han to the system font. (A single linear per-family chain still decides a
    // glyph once a broad face is discovered; see `cjk_fonts`.)
    // Named weights fall back to the default stack for missing glyphs.
    let fallback: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for (fam, primary) in [("medium", medium), ("semibold", semibold)] {
        let mut stack: Vec<String> = primary.into_iter().collect();
        stack.extend(fallback.iter().cloned());
        fonts.families.insert(FontFamily::Name(fam.into()), stack);
    }
    let size = ui_font_size(ctx);
    for (name, data) in &mut fonts.font_data {
        size_ui_font(ctx, name, Arc::make_mut(data));
    }
    ctx.set_fonts(fonts);
    // egui keeps the first plugin of a type, so a re-install (language change) resets the
    // existing one. The new definitions land next pass: the plugin must not rescale before then.
    if ctx
        .with_plugin::<UiFontSizePlugin, _>(|plugin| {
            plugin.applied = size;
            plugin.defer_after_install = true;
        })
        .is_none()
    {
        ctx.add_plugin(UiFontSizePlugin { applied: size, defer_after_install: true });
    }
    // CJK, Thai, Arabic, Indic and other script fallback fonts (craft-fonts' Japanese ones if built in, then
    // the system's) are registered on demand (cjk_fonts.rs).
    crate::cjk_fonts::install_with(ctx, cjk);
}

/// Has [`install_fonts`]'s stack reached the active fonts? Before that (the first pass, when
/// the desktop app installs fonts and loads preferences in the same frame) the active fonts
/// are egui's defaults, and rescaling a copy of those would overwrite the queued stack.
fn ui_fonts_active(ctx: &egui::Context) -> bool {
    ctx.fonts(|f| f.definitions().families.contains_key(&FontFamily::Name("medium".into())))
}

fn ui_fonts_id() -> egui::Id {
    egui::Id::unique("photocraft-ui-fonts")
}

fn ui_font_size(ctx: &egui::Context) -> UiFontSize {
    ctx.data(|d| d.get_temp::<UiFontSize>(ui_fonts_id())).unwrap_or_default()
}

/// The Interface > UI Font Size multiplier for the current context (including the default Small).
pub(crate) fn ui_font_scale(ctx: &egui::Context) -> f32 {
    font_scale(ui_font_size(ctx))
}

fn font_scale(size: UiFontSize) -> f32 {
    match size {
        UiFontSize::Tiny => 10.0 / 12.0,
        UiFontSize::Small => 1.0,
        UiFontSize::Medium => 14.0 / 12.0,
        UiFontSize::Large => 16.0 / 12.0,
    }
}

/// Register the original multiplier before sizing a face, including lazy CJK fallbacks.
pub(crate) fn size_ui_font(ctx: &egui::Context, name: &str, data: &mut FontData) {
    ctx.data_mut(|d| d.insert_temp(ui_fonts_id().with(name), data.tweak.scale));
    data.tweak.scale *= font_scale(ui_font_size(ctx));
}

/// Apply Interface › UI Font Size independently of display/canvas zoom. Scaling the registered
/// faces covers explicit RichText and painter font sizes as well as egui's text styles. egui
/// 0.36 uses these tweaks in shaping and row metrics; the layout tests below guard that contract.
pub(crate) fn set_ui_font_size(ctx: &egui::Context, size: UiFontSize) {
    if ui_font_size(ctx) == size {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(ui_fonts_id(), size));
    ctx.request_repaint();
}

/// Font access is valid after the first pass, including when preferences load before it.
/// Keep only the original multipliers, not a second copy of large system font files.
struct UiFontSizePlugin {
    applied: UiFontSize,
    /// `set_fonts` takes effect next pass: skip one hook so the rescale starts from that stack
    /// and not from the one it replaces.
    defer_after_install: bool,
}

impl egui::Plugin for UiFontSizePlugin {
    fn debug_name(&self) -> &'static str {
        "photocraft-ui-font-size"
    }

    fn output_hook(&mut self, ctx: &egui::Context, _output: &mut egui::FullOutput) {
        let size = ui_font_size(ctx);
        let deferred = std::mem::take(&mut self.defer_after_install);
        if size == self.applied {
            return;
        }
        if deferred || !ui_fonts_active(ctx) {
            // Cloning the active definitions now would discard the queued stack (the desktop
            // app installs fonts and loads the saved size in its first frame).
            ctx.request_repaint();
            return;
        }
        // Start from the current stack so lazily registered fallback faces are retained.
        let mut fonts = ctx.fonts(|f| f.definitions().clone());
        for (name, data) in &mut fonts.font_data {
            let id = ui_fonts_id().with(name);
            let original = ctx.data(|d| d.get_temp::<f32>(id)).unwrap_or(data.tweak.scale / font_scale(self.applied));
            ctx.data_mut(|d| d.insert_temp(id, original));
            Arc::make_mut(data).tweak.scale = original * font_scale(size);
        }
        ctx.set_fonts(fonts);
        // The provider must use the scale of the definitions that were active while this pass
        // laid out text. A face it discovers during the pass is inserted at that old scale; after
        // `set_fonts` the rescaled definitions take effect and the provider follows them.
        crate::cjk_fonts::set_craft_font_scale(ctx, font_scale(size));
        self.applied = size;
        ctx.request_repaint();
    }
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// Apply a theme to egui's global style and publish its tokens.
pub fn apply(ctx: &egui::Context, kind: ThemeKind) {
    // Keep native windows at SystemDefault so macOS continues reporting OS appearance changes,
    // including when an integration or restored egui state pinned its own theme preference.
    ctx.set_theme(egui::ThemePreference::System);
    let t = Tokens::for_kind(kind);
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("photocraft-theme"), t));
    let mut v = if t.dark() { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.chrome;
    v.window_fill = t.card;
    v.window_stroke = Stroke::new(1.0, t.card_border);
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.card;
    v.code_bg_color = t.field;
    v.override_text_color = Some(t.text);
    v.hyperlink_color = t.accent;
    v.warn_fg_color = t.warning;
    v.error_fg_color = t.danger;
    v.window_corner_radius = CornerRadius::same(t.radius_lg as u8);
    v.menu_corner_radius = CornerRadius::same(t.radius as u8);
    v.window_shadow = egui::Shadow { offset: [0, 10], blur: 32, spread: 0, color: t.shadow };
    v.popup_shadow = egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: t.shadow };
    v.selection.bg_fill = if t.bevel || t.pro { t.accent } else { t.accent_soft };
    v.selection.stroke = Stroke::new(1.0, t.accent_text);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;
    let r = CornerRadius::same(t.radius_sm as u8);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.card;
    w.noninteractive.weak_bg_fill = t.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.separator);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text_dim);
    w.noninteractive.corner_radius = r;
    for (wv, bg, stroke) in [
        (&mut w.inactive, t.field, t.field_border),
        (&mut w.hovered, t.hover, t.field_border),
        (&mut w.active, t.pressed, t.accent_border),
        (&mut w.open, t.hover, t.field_border),
    ] {
        wv.bg_fill = bg;
        wv.weak_bg_fill = bg;
        wv.bg_stroke = if t.bevel { Stroke::new(1.0, Color32::from_gray(64)) } else { Stroke::new(1.0, stroke) };
        wv.fg_stroke = Stroke::new(1.0, t.text);
        wv.corner_radius = r;
        wv.expansion = 0.0;
    }
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(10.5)),
            (TextStyle::Body, FontId::proportional(if t.pro { 12.0 } else { 12.5 })),
            (TextStyle::Button, FontId::proportional(if t.pro { 12.0 } else { 12.5 })),
            (TextStyle::Heading, semibold(15.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size = egui::vec2(24.0, 24.0);
        s.spacing.slider_width = 150.0;
        s.spacing.combo_width = 120.0;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.window_margin = egui::Margin::same(16);
        s.spacing.icon_width = 14.0;
        s.visuals.indent_has_left_vline = false;
        s.interaction.tooltip_delay = TOOLTIP_DELAY;
        // Thin overlay scrollbars that appear on hover (Photoshop/macOS style).
        // Thin bars stay visible, including while the pointer is on them. The stock thin
        // style fades the handle to the same opacity as the track, so the thumb disappears (#4611).
        let mut scroll = if t.bevel { egui::style::ScrollStyle::solid() } else { egui::style::ScrollStyle::thin() };
        if scroll.floating {
            scroll.interact_handle_opacity = 1.0;
            scroll.interact_background_opacity = 0.4;
            scroll.floating_width = 4.0;
        }
        s.spacing.scroll = scroll;
        s.spacing.tooltip_width = 280.0;
    });
    // egui selects its style branch using OS appearance, independently of PhotoCraft's fixed
    // Dark/Light mode or platform appearance service. Both branches need the selected palette.
    let style = ctx.global_style();
    ctx.set_style_of(egui::Theme::Light, Arc::clone(&style));
    ctx.set_style_of(egui::Theme::Dark, style);
}

/// Seconds the pointer rests on a control before its tooltip shows.
pub const TOOLTIP_DELAY: f32 = 0.35;

/// Vertical gap between stacked control rows in panels (Properties fields, the Layers panel's
/// Opacity and Fill rows); docks zero egui's item spacing, so rows add this themselves.
pub const ROW_GAP: f32 = 4.0;

pub fn canvas_bg(t: &Tokens) -> Color32 {
    t.canvas
}

#[cfg(test)]
mod tests;

/// Development feature: live design-token overrides.
///
/// Set `PHOTOCRAFT_THEME_FILE=/path/tokens.json` in a debug build; the file is polled and applied
/// on change, so colours, radii and sizes can be tuned without recompiling. Keys are `Tokens` field
/// names; values are `"#rrggbb"`, `"#rrggbbaa"` or numbers. Compiled out of release builds.
#[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
pub mod live {
    use super::Tokens;
    use egui::Color32;
    use std::time::SystemTime;

    #[derive(Default)]
    pub struct LiveTokens {
        path: Option<std::path::PathBuf>,
        stamp: Option<SystemTime>,
        last_check: f64,
    }

    impl LiveTokens {
        pub fn from_env() -> Self {
            Self { path: std::env::var_os("PHOTOCRAFT_THEME_FILE").map(Into::into), ..Default::default() }
        }

        /// Re-apply overrides if the file changed. Returns true when tokens were updated.
        pub fn poll(&mut self, ctx: &egui::Context, kind: super::ThemeKind) -> bool {
            let Some(path) = &self.path else { return false };
            let now = ctx.input(|i| i.time);
            if now - self.last_check < 0.4 {
                ctx.request_repaint_after(std::time::Duration::from_millis(400));
                return false;
            }
            self.last_check = now;
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
            let Ok(meta) = std::fs::metadata(path) else { return false };
            let stamp = meta.modified().ok();
            if stamp == self.stamp {
                return false;
            }
            self.stamp = stamp;
            let Ok(text) = std::fs::read_to_string(path) else { return false };
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) => {
                    super::apply(ctx, kind);
                    let mut t = Tokens::get(ctx);
                    let unknown = apply_overrides(&mut t, &v);
                    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("photocraft-theme"), t));
                    if !unknown.is_empty() {
                        log::warn!("unknown token keys: {unknown:?}");
                    }
                    true
                }
                Err(e) => {
                    log::warn!("theme file: {e}");
                    false
                }
            }
        }
    }

    fn color(v: &serde_json::Value) -> Option<Color32> {
        let s = v.as_str()?.trim_start_matches('#');
        let b = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
        match s.len() {
            6 => Some(Color32::from_rgb(b(0)?, b(2)?, b(4)?)),
            8 => Some(Color32::from_rgba_unmultiplied(b(0)?, b(2)?, b(4)?, b(6)?)),
            _ => None,
        }
    }

    /// Apply JSON overrides onto tokens; returns unknown keys.
    pub fn apply_overrides(t: &mut Tokens, v: &serde_json::Value) -> Vec<String> {
        let mut unknown = Vec::new();
        let Some(obj) = v.as_object() else { return unknown };
        for (k, val) in obj {
            if override_chrome_color(t, k, val) || override_text_color(t, k, val) || override_accent_color(t, k, val) {
                continue;
            }
            match k.as_str() {
                "radius_sm" => {
                    if let Some(x) = val.as_f64() {
                        t.radius_sm = x as f32;
                    }
                }
                "radius" => {
                    if let Some(x) = val.as_f64() {
                        t.radius = x as f32;
                    }
                }
                "radius_lg" => {
                    if let Some(x) = val.as_f64() {
                        t.radius_lg = x as f32;
                    }
                }
                _ => unknown.push(k.clone()),
            }
        }
        unknown
    }

    fn override_chrome_color(t: &mut Tokens, key: &str, value: &serde_json::Value) -> bool {
        match key {
            "caption_close" => {
                if let Some(c) = color(value) {
                    t.caption_close = c;
                }
            }
            "caption_close_text" => {
                if let Some(c) = color(value) {
                    t.caption_close_text = c;
                }
            }
            "chrome" => {
                if let Some(c) = color(value) {
                    t.chrome = c;
                }
            }
            "canvas" => {
                if let Some(c) = color(value) {
                    t.canvas = c;
                }
            }
            "canvas_dot" => {
                if let Some(c) = color(value) {
                    t.canvas_dot = c;
                }
            }
            "dock" => {
                if let Some(c) = color(value) {
                    t.dock = c;
                }
            }
            "card" => {
                if let Some(c) = color(value) {
                    t.card = c;
                }
            }
            "card_border" => {
                if let Some(c) = color(value) {
                    t.card_border = c;
                }
            }
            "field" => {
                if let Some(c) = color(value) {
                    t.field = c;
                }
            }
            "field_border" => {
                if let Some(c) = color(value) {
                    t.field_border = c;
                }
            }
            _ => return false,
        }
        true
    }

    fn override_text_color(t: &mut Tokens, key: &str, value: &serde_json::Value) -> bool {
        match key {
            "hover" => {
                if let Some(c) = color(value) {
                    t.hover = c;
                }
            }
            "pressed" => {
                if let Some(c) = color(value) {
                    t.pressed = c;
                }
            }
            "text" => {
                if let Some(c) = color(value) {
                    t.text = c;
                }
            }
            "text_dim" => {
                if let Some(c) = color(value) {
                    t.text_dim = c;
                }
            }
            "text_faint" => {
                if let Some(c) = color(value) {
                    t.text_faint = c;
                }
            }
            "icon" => {
                if let Some(c) = color(value) {
                    t.icon = c;
                }
            }
            "separator" => {
                if let Some(c) = color(value) {
                    t.separator = c;
                }
            }
            "shadow" => {
                if let Some(c) = color(value) {
                    t.shadow = c;
                }
            }
            "primary_bg" => {
                if let Some(c) = color(value) {
                    t.primary_bg = c;
                }
            }
            "primary_text" => {
                if let Some(c) = color(value) {
                    t.primary_text = c;
                }
            }
            _ => return false,
        }
        true
    }

    fn override_accent_color(t: &mut Tokens, key: &str, value: &serde_json::Value) -> bool {
        match key {
            "accent" => {
                if let Some(c) = color(value) {
                    t.accent = c;
                }
            }
            "accent_soft" => {
                if let Some(c) = color(value) {
                    t.accent_soft = c;
                }
            }
            "accent_border" => {
                if let Some(c) = color(value) {
                    t.accent_border = c;
                }
            }
            "accent_text" => {
                if let Some(c) = color(value) {
                    t.accent_text = c;
                }
            }
            "danger" => {
                if let Some(c) = color(value) {
                    t.danger = c;
                }
            }
            "warning" => {
                if let Some(c) = color(value) {
                    t.warning = c;
                }
            }
            "tab_strip" => {
                if let Some(c) = color(value) {
                    t.tab_strip = c;
                }
            }
            "row_selected" => {
                if let Some(c) = color(value) {
                    t.row_selected = c;
                }
            }
            "scrim" => {
                if let Some(c) = color(value) {
                    t.scrim = c;
                }
            }
            _ => return false,
        }
        true
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn overrides_apply_and_report_unknown() {
            let mut t = Tokens::for_kind(super::super::ThemeKind::Pro);
            let unknown = apply_overrides(&mut t, &serde_json::json!({"card": "#102030", "radius": 9, "nope": 1}));
            assert_eq!(t.card, Color32::from_rgb(16, 32, 48));
            assert_eq!(t.radius, 9.0);
            assert_eq!(unknown, vec!["nope".to_string()]);
        }
    }
}
