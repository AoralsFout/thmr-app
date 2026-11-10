//! Display-only frequency mapping. Musical features always use the full range.
use super::BANDS;
use serde::{Deserialize, Serialize};

pub const MIN_HZ: f32 = 40.0;
pub fn full_range(rate: u32) -> f32 {
    (rate as f32 / 2.0).min(16000.0)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DisplaySpectrum {
    #[serde(with = "super::band_array")]
    pub bands: [f32; BANDS],
    pub max_hz: f32,
}
impl Default for DisplaySpectrum {
    fn default() -> Self {
        Self {
            bands: [0.0; BANDS],
            max_hz: 16000.0,
        }
    }
}

pub fn project(power: &[f32], rate: u32, max_hz: f32) -> [f32; BANDS] {
    if power.len() < 2 || rate == 0 {
        return [0.0; BANDS];
    }
    let fft = power.len() * 2;
    let maximum = max_hz.min(full_range(rate)).max(MIN_HZ + 1.0);
    std::array::from_fn(|band| {
        let low = MIN_HZ * (maximum / MIN_HZ).powf(band as f32 / BANDS as f32);
        let high = MIN_HZ * (maximum / MIN_HZ).powf((band + 1) as f32 / BANDS as f32);
        let start = ((low * fft as f32 / rate as f32).floor() as usize).clamp(1, power.len() - 1);
        let end = ((high * fft as f32 / rate as f32).ceil() as usize).clamp(start + 1, power.len());
        let peak = power[start..end].iter().copied().fold(0.0f32, f32::max);
        ((10.0 * peak.max(1e-18).log10() + 65.0) / 65.0).clamp(0.0, 1.0)
    })
}

pub struct AdaptiveSpectrum {
    pub frame: DisplaySpectrum,
    rate: u32,
    target: f32,
    shrink_time: f32,
    shrink_ceiling: f32,
    expand_time: f32,
    richness: f32,
    silence_time: f32,
}
impl Default for AdaptiveSpectrum {
    fn default() -> Self {
        Self {
            frame: DisplaySpectrum::default(),
            rate: 0,
            target: 16000.0,
            shrink_time: 0.0,
            shrink_ceiling: 0.0,
            expand_time: 0.0,
            richness: 0.0,
            silence_time: 0.0,
        }
    }
}
impl AdaptiveSpectrum {
    pub fn update(&mut self, power: &[f32], fixed: &[f32; BANDS], rate: u32, dt: f32) {
        let full = full_range(rate);
        if self.rate != rate {
            *self = Self {
                rate,
                target: full,
                frame: DisplaySpectrum {
                    max_hz: full,
                    ..Default::default()
                },
                ..Default::default()
            };
        }
        let fft = power.len() * 2;
        let end = ((full * fft as f32 / rate as f32).ceil() as usize).min(power.len());
        let peak = power[..end].iter().copied().fold(0.0f32, f32::max);
        // Require both an absolute signal floor (-60 dBFS) and a relative floor
        // (-40 dB below the strongest bin). Noise cannot inflate the range.
        let threshold = (peak * 0.0001).max(1e-6);
        let first = power[..end]
            .iter()
            .enumerate()
            .find(|(i, p)| **p > threshold && *i as f32 * rate as f32 / fft as f32 >= MIN_HZ)
            .map(|(i, _)| i);
        let last = power[..end].iter().rposition(|p| *p > threshold);
        let valid = peak > 1e-5 && first.is_some();
        let normalized_floor = ((10.0 * threshold.log10() + 65.0) / 65.0).max(0.0);
        let occupied = fixed.iter().filter(|v| **v > normalized_floor).count();
        let rich = match (first, last) {
            (Some(low), Some(high)) => occupied >= 8 && high as f32 / low.max(1) as f32 >= 3.0,
            _ => false,
        };
        self.richness +=
            ((if rich { 1.0 } else { 0.0 }) - self.richness) * (1.0 - (-dt / 0.6).exp());
        if valid {
            self.silence_time = 0.0;
            let highest = last.unwrap_or(0) as f32 * rate as f32 / fft as f32;
            // A small margin, rounded to 500 Hz, keeps real content in view and
            // avoids the repeated boundary changes of a per-frame auto-zoom.
            let desired =
                ((highest * 1.06 / 500.0).ceil() * 500.0).clamp(4000.0f32.min(full), full);
            if desired > self.target + 250.0 {
                self.expand_time += dt;
                self.shrink_time = 0.0;
                if self.expand_time >= 0.12 {
                    self.target = desired;
                    self.expand_time = 0.0;
                }
            } else {
                self.expand_time = 0.0;
                if desired < self.target * 0.9 && self.richness > 0.7 {
                    if self.shrink_time == 0.0 {
                        self.shrink_ceiling = desired;
                    }
                    self.shrink_ceiling = self.shrink_ceiling.max(desired);
                    self.shrink_time += dt;
                    if self.shrink_time >= 10.0 {
                        self.target = self.shrink_ceiling;
                        self.shrink_time = 0.0;
                    }
                } else {
                    self.shrink_time = 0.0;
                }
            }
        } else {
            self.expand_time = 0.0;
            self.silence_time += dt;
            if self.silence_time > 0.35 {
                self.shrink_time = 0.0;
            }
        }
        if valid && (self.target >= self.frame.max_hz || self.richness > 0.7) {
            let tau = if self.target > self.frame.max_hz {
                0.25
            } else {
                1.0
            };
            self.frame.max_hz += (self.target - self.frame.max_hz) * (1.0 - (-dt / tau).exp());
            if (self.target - self.frame.max_hz).abs() < 10.0 {
                self.frame.max_hz = self.target;
            }
        }
        self.frame.bands = project(power, rate, self.frame.max_hz);
        for (i, value) in self.frame.bands.iter_mut().enumerate() {
            // At most 3 dB of visual high-frequency lift, with a soft noise gate.
            // Exact zeros stay zero. This never feeds back into music analysis.
            let gate = ((*value - 0.02) / 0.08).clamp(0.0, 1.0);
            *value = (*value + gate * 3.0 / 65.0 * i as f32 / (BANDS - 1) as f32).min(1.0);
        }
    }
}

/// Move displayed history along with the axis, rather than leaving peak marks
/// at positions that now represent a different physical frequency.
pub fn remap(values: &[f32; BANDS], old_max: f32, new_max: f32) -> [f32; BANDS] {
    std::array::from_fn(|i| {
        let frequency = MIN_HZ * (new_max / MIN_HZ).powf((i as f32 + 0.5) / BANDS as f32);
        let old =
            ((frequency / MIN_HZ).ln() / (old_max / MIN_HZ).ln() * BANDS as f32 - 0.5).max(0.0);
        if old > (BANDS - 1) as f32 {
            return 0.0;
        }
        let low = old.floor() as usize;
        let high = (low + 1).min(BANDS - 1);
        values[low] + (values[high] - values[low]) * old.fract()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spectrum(cutoff: f32) -> Vec<f32> {
        (0..1024)
            .map(|i| {
                if (80.0..=cutoff).contains(&(i as f32 * 48000.0 / 2048.0)) {
                    0.001
                } else {
                    0.0
                }
            })
            .collect()
    }
    fn feed(view: &mut AdaptiveSpectrum, power: &[f32], seconds: f32) {
        let fixed = project(power, 48000, 16000.0);
        for _ in 0..(seconds / 0.02).round() as usize {
            view.update(power, &fixed, 48000, 0.02);
        }
    }
    #[test]
    fn sustained_limited_bandwidth_reclaims_columns_but_short_passages_do_not_zoom() {
        let power = spectrum(4000.0);
        let fixed = project(&power, 48000, 16000.0);
        let mut view = AdaptiveSpectrum::default();
        feed(&mut view, &power, 8.0);
        assert_eq!(view.frame.max_hz, 16000.0);
        feed(&mut view, &power, 12.0);
        assert!((4000.0..=4600.0).contains(&view.frame.max_hz));
        let before = fixed.iter().rev().take_while(|v| **v == 0.0).count();
        let after = view
            .frame
            .bands
            .iter()
            .rev()
            .take_while(|v| **v == 0.0)
            .count();
        assert!(
            before >= 9 && after <= 2,
            "empty columns: {before} -> {after}"
        );
    }
    #[test]
    fn silence_noise_and_single_tones_cannot_collapse_the_axis_or_invent_energy() {
        let mut view = AdaptiveSpectrum::default();
        for level in [0.0, 1e-9] {
            feed(&mut view, &vec![level; 1024], 30.0);
            assert_eq!(view.frame.max_hz, 16000.0);
            assert!(view.frame.bands.iter().all(|v| *v == 0.0));
        }
        let mut tone = vec![0.0; 1024];
        tone[19] = 0.05;
        feed(&mut view, &tone, 30.0);
        assert_eq!(view.frame.max_hz, 16000.0);
    }
    #[test]
    fn history_tracks_physical_frequency_and_new_high_bands_start_empty() {
        let mut values = [0.0; BANDS];
        values[20] = 1.0;
        let mapped = remap(&values, 16000.0, 4500.0);
        let peak = mapped
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .unwrap()
            .0;
        let original_hz = MIN_HZ * (16000.0 / MIN_HZ).powf(20.5 / BANDS as f32);
        let mapped_hz = MIN_HZ * (4500.0 / MIN_HZ).powf((peak as f32 + 0.5) / BANDS as f32);
        assert!((mapped_hz / original_hz - 1.0).abs() < 0.07);
        let expanded = remap(&[0.8; BANDS], 4000.0, 16000.0);
        assert!(expanded[38..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn returning_highs_expand_quickly_and_rate_changes_reset_the_range() {
        let mut view = AdaptiveSpectrum::default();
        feed(&mut view, &spectrum(4000.0), 20.0);
        let narrow = view.frame.max_hz;
        feed(&mut view, &vec![0.0; 1024], 5.0);
        assert_eq!(view.frame.max_hz, narrow);
        feed(&mut view, &spectrum(16000.0), 1.5);
        assert!(view.frame.max_hz > 15500.0);
        view.update(&vec![0.0; 1024], &[0.0; BANDS], 8000, 0.02);
        assert_eq!(view.frame.max_hz, 4000.0);
    }
}
