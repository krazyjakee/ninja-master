//! Presentation measurements, independent of the fixed simulation clock.
use imgui::{Condition, Ui, WindowFlags};
use std::time::{Duration, Instant};

const HISTORY: usize = 240;
pub(super) struct Performance {
    pub visible: bool,
    periods: [f32; HISTORY],
    cursor: usize,
    count: usize,
    pub raster_ms: f32,
    pub upload_ms: f32,
    pub submit_ms: f32,
    pub present_ms: f32,
    pub upload_bytes: usize,
    pub ticks: u32,
}
impl Default for Performance {
    fn default() -> Self {
        Self {
            visible: false,
            periods: [0.0; HISTORY],
            cursor: 0,
            count: 0,
            raster_ms: 0.0,
            upload_ms: 0.0,
            submit_ms: 0.0,
            present_ms: 0.0,
            upload_bytes: 0,
            ticks: 0,
        }
    }
}
impl Performance {
    pub fn observe(&mut self, period: Duration) {
        self.periods[self.cursor] = period.as_secs_f32() * 1000.0;
        self.cursor = (self.cursor + 1) % HISTORY;
        self.count = (self.count + 1).min(HISTORY);
    }
    fn timings(&self) -> Option<(f32, f32, f32)> {
        if self.count == 0 {
            return None;
        }
        let current = self.periods[(self.cursor + HISTORY - 1) % HISTORY];
        let average = self.periods[..self.count].iter().sum::<f32>() / self.count as f32;
        let maximum = self.periods[..self.count]
            .iter()
            .copied()
            .fold(0.0, f32::max);
        Some((current, average, maximum))
    }
    pub fn show(
        &self,
        ui: &Ui,
        source: (usize, usize),
        viewport: (u32, u32),
        sprites: usize,
        renderer: &str,
    ) {
        if !self.visible {
            return;
        }
        let [width, height] = ui.io().display_size;
        let scale = (width / 420.0).min(height / 260.0).clamp(0.45, 1.0);
        ui.window("Performance | F2")
            .position([width - 8.0, 8.0], Condition::Always)
            .position_pivot([1.0, 0.0])
            .size([(width - 16.0).min(410.0), 0.0], Condition::Always)
            .flags(
                WindowFlags::NO_DECORATION
                    | WindowFlags::NO_INPUTS
                    | WindowFlags::NO_SAVED_SETTINGS
                    | WindowFlags::ALWAYS_AUTO_RESIZE
                    | WindowFlags::NO_FOCUS_ON_APPEARING,
            )
            .bg_alpha(0.9)
            .build(|| {
                ui.set_window_font_scale(scale);
                ui.text("PERFORMANCE | F2");
                if let Some((current, average, maximum)) = self.timings() {
                    ui.text(format!(
                        "FPS {:.1} | avg {:.1} | {:.2} ms",
                        1000.0 / current.max(0.001),
                        1000.0 / average.max(0.001),
                        current
                    ));
                    ui.plot_lines("##frame periods", &self.periods[..self.count])
                        .values_offset(if self.count == HISTORY {
                            self.cursor
                        } else {
                            0
                        })
                        .scale_min(0.0)
                        .scale_max(maximum.max(33.34))
                        .graph_size([0.0, 65.0 * scale])
                        .build();
                    ui.text(format!(
                        "Frame ms: 0..{:.1} | peak {:.1} ({} frames)",
                        maximum.max(33.34),
                        maximum,
                        self.count
                    ));
                }
                ui.text(format!(
                    "CPU raster {:.2} | upload {:.2} ms",
                    self.raster_ms, self.upload_ms
                ));
                ui.text(format!(
                    "CPU submit {:.2} | present/wait {:.2} ms",
                    self.submit_ms, self.present_ms
                ));
                ui.text(format!(
                    "Source {}x{} | view {}x{} px",
                    source.0, source.1, viewport.0, viewport.1
                ));
                ui.text(format!(
                    "Sprites {} | upload {:.1} KiB | ticks {}",
                    sprites,
                    self.upload_bytes as f32 / 1024.0,
                    self.ticks
                ));
                ui.text(format!("Renderer: {renderer}"));
            });
    }
}

pub(super) fn feedback(ui: &Ui, toast: &Option<(String, Instant)>) {
    if let Some((message, until)) = toast
        && Instant::now() < *until
    {
        let [width, height] = ui.io().display_size;
        ui.window("Capture feedback")
            .position([8.0, height - 8.0], Condition::Always)
            .position_pivot([0.0, 1.0])
            .size([(width - 16.0).min(650.0), 0.0], Condition::Always)
            .flags(
                WindowFlags::NO_DECORATION
                    | WindowFlags::NO_INPUTS
                    | WindowFlags::NO_SAVED_SETTINGS
                    | WindowFlags::ALWAYS_AUTO_RESIZE
                    | WindowFlags::NO_FOCUS_ON_APPEARING,
            )
            .build(|| ui.text_wrapped(message));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_keeps_stutters_and_wraps_without_changing_the_clock() {
        let mut performance = Performance::default();
        assert!(performance.timings().is_none());
        for _ in 0..HISTORY {
            performance.observe(Duration::from_millis(16));
        }
        performance.observe(Duration::from_millis(100));
        let (current, average, peak) = performance.timings().unwrap();
        assert_eq!(current, 100.0);
        assert_eq!(peak, 100.0);
        assert!((average - 16.35).abs() < 0.001);
        assert_eq!(performance.cursor, 1);
        for _ in 0..HISTORY {
            performance.observe(Duration::from_millis(20));
        }
        assert_eq!(performance.timings(), Some((20.0, 20.0, 20.0)));
    }
}
