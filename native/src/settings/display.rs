//! Presentation preferences passed between the desktop and native Options UI.
use super::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplaySettings {
    pub scale: u32,
    pub fullscreen: bool,
    pub widescreen: bool,
    pub aspect: usize,
    pub integer_scale: bool,
    pub smooth_wave_effects: bool,
    pub scanlines: u8,
    pub crt: bool,
    pub crt_curvature: u8,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self::from(&Settings::default())
    }
}

impl From<&Settings> for DisplaySettings {
    fn from(settings: &Settings) -> Self {
        Self {
            scale: settings.scale,
            fullscreen: settings.fullscreen,
            widescreen: settings.widescreen,
            aspect: settings.aspect,
            integer_scale: settings.integer_scale,
            smooth_wave_effects: settings.smooth_wave_effects,
            scanlines: settings.scanlines,
            crt: settings.crt,
            crt_curvature: settings.crt_curvature,
        }
    }
}

impl DisplaySettings {
    pub fn apply(self, settings: &mut Settings) {
        settings.scale = self.scale;
        settings.fullscreen = self.fullscreen;
        settings.widescreen = self.widescreen;
        settings.aspect = self.aspect;
        settings.integer_scale = self.integer_scale;
        settings.smooth_wave_effects = self.smooth_wave_effects;
        settings.scanlines = self.scanlines;
        settings.crt = self.crt;
        settings.crt_curvature = self.crt_curvature;
    }
}
