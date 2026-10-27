use super::*;

/// Selected tab per dock card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockTabs {
    pub properties: usize,
    pub color: usize,
    pub layers: usize,
    /// Navigator | Histogram | Info.
    pub navigator: usize,
    /// History | Actions.
    pub history: usize,
    /// Character | Paragraph.
    pub character: usize,
    /// Brushes presets.
    pub brushes: usize,
}

/// Color panel state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorPanelState {
    /// The panel edits the background colour (its chip was clicked), not the foreground.
    pub background: bool,
    /// Color Picker › Only Web Colors, kept across opens like Photoshop (#4995). `web_only` is
    /// its name before the camelCase rename (#6000).
    #[serde(default, alias = "web_only")]
    pub web_only: bool,
    /// What the panel shows, picked from its panel menu as in Photoshop (#6000).
    pub mode: ColorPanelMode,
    /// The colour ramp under the sliders.
    pub ramp: ColorRamp,
    /// Make Ramp Web Safe: the ramp shows (and picks) web colours only.
    pub web_safe_ramp: bool,
}

/// The Color panel's display, Photoshop's panel menu choices in its order. Hue Cube is
/// Photoshop's default; the theme never changes it (#6000).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorPanelMode {
    #[default]
    HueCube,
    BrightnessCube,
    ColorWheel,
    GrayscaleSlider,
    RgbSliders,
    HsbSliders,
    CmykSliders,
    LabSliders,
    WebColorSliders,
}

impl ColorPanelMode {
    /// Every mode, in Photoshop's panel menu order.
    pub const ALL: [ColorPanelMode; 9] = [
        ColorPanelMode::HueCube,
        ColorPanelMode::BrightnessCube,
        ColorPanelMode::ColorWheel,
        ColorPanelMode::GrayscaleSlider,
        ColorPanelMode::RgbSliders,
        ColorPanelMode::HsbSliders,
        ColorPanelMode::CmykSliders,
        ColorPanelMode::LabSliders,
        ColorPanelMode::WebColorSliders,
    ];

    /// The panel menu item.
    pub fn label(self) -> &'static str {
        match self {
            ColorPanelMode::HueCube => "Hue Cube",
            ColorPanelMode::BrightnessCube => "Brightness Cube",
            ColorPanelMode::ColorWheel => "Color Wheel",
            ColorPanelMode::GrayscaleSlider => "Grayscale Slider",
            ColorPanelMode::RgbSliders => "RGB Sliders",
            ColorPanelMode::HsbSliders => "HSB Sliders",
            ColorPanelMode::CmykSliders => "CMYK Sliders",
            ColorPanelMode::LabSliders => "Lab Sliders",
            ColorPanelMode::WebColorSliders => "Web Color Sliders",
        }
    }

    /// The slider modes, which show the colour ramp under their sliders.
    pub fn is_sliders(self) -> bool {
        !matches!(self, ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube | ColorPanelMode::ColorWheel)
    }
}

/// The Color panel's colour ramp, from its panel menu (or Shift-click on the ramp to cycle).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorRamp {
    #[default]
    RgbSpectrum,
    CmykSpectrum,
    GrayscaleRamp,
    CurrentColors,
}

impl ColorRamp {
    /// Every ramp, in Photoshop's panel menu order (also the Shift-click cycle).
    pub const ALL: [ColorRamp; 4] = [ColorRamp::RgbSpectrum, ColorRamp::CmykSpectrum, ColorRamp::GrayscaleRamp, ColorRamp::CurrentColors];

    /// The panel menu item.
    pub fn label(self) -> &'static str {
        match self {
            ColorRamp::RgbSpectrum => "RGB Spectrum",
            ColorRamp::CmykSpectrum => "CMYK Spectrum",
            ColorRamp::GrayscaleRamp => "Grayscale Ramp",
            ColorRamp::CurrentColors => "Current Colors",
        }
    }

    /// The next ramp in the Shift-click cycle.
    pub fn next(self) -> ColorRamp {
        let i = ColorRamp::ALL.iter().position(|r| *r == self).unwrap_or(0);
        ColorRamp::ALL.get(i + 1).copied().unwrap_or(ColorRamp::RgbSpectrum)
    }
}
