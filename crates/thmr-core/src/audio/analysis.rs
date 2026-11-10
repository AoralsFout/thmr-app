//! PCM analysis shared by the host and terminal display. No network or playback clock.
use super::{
    display::{self as spectrum_view, AdaptiveSpectrum},
    AudioFeature, ChannelLevels, MusicalFeatures, BANDS,
};
const FFT: usize = 2048;
/// Detect positive spectral changes before UI smoothing. Counters preserve
/// transients between terminal frames; timing uses PCM cadence, not HTTP chunks.
struct MusicalAnalyzer {
    previous: [f32; BANDS],
    baseline: f32,
    flux_floor: f32,
    high_floor: f32,
    time: f32,
    last_beat: f32,
    last_accent: f32,
    last_surge: f32,
    interval: f32,
    features: MusicalFeatures,
}
impl Default for MusicalAnalyzer {
    fn default() -> Self {
        Self {
            previous: [0.0; BANDS],
            baseline: 0.0,
            flux_floor: 0.0,
            high_floor: 0.0,
            time: 0.0,
            last_beat: 0.0,
            last_accent: 0.0,
            last_surge: 0.0,
            interval: 0.0,
            features: MusicalFeatures::default(),
        }
    }
}
impl MusicalAnalyzer {
    fn update(&mut self, bands: &[f32; BANDS], dt: f32) -> MusicalFeatures {
        self.time += dt;
        let level = bands.iter().sum::<f32>() / BANDS as f32;
        let flux = bands
            .iter()
            .zip(self.previous)
            .map(|(v, p)| (v - p).max(0.0))
            .sum::<f32>()
            / BANDS as f32;
        let high_flux = bands[32..]
            .iter()
            .zip(&self.previous[32..])
            .map(|(v, p)| (v - p).max(0.0))
            .sum::<f32>()
            / 16.0;
        let low_rise = bands[..16]
            .iter()
            .zip(&self.previous[..16])
            .map(|(v, p)| (v - p).max(0.0))
            .sum::<f32>()
            / 16.0;
        // Warm up before emitting events; attack/release and a refractory period
        // avoid multiple volleys for the same sustained note or noisy drum hit.
        if self.time > 0.2 && level > 0.035 {
            if flux > (self.flux_floor * 1.8 + 0.006)
                && low_rise > 0.012
                && self.time - self.last_beat > 0.19
            {
                let interval = self.time - self.last_beat;
                if self.last_beat > 0.0 && (0.25..=1.2).contains(&interval) {
                    let consistent = self.interval == 0.0
                        || (interval - self.interval).abs() < self.interval * 0.22;
                    self.features.beat_confidence = if consistent {
                        (self.features.beat_confidence + 0.18).min(1.0)
                    } else {
                        self.features.beat_confidence * 0.5
                    };
                    self.interval = if consistent && self.interval > 0.0 {
                        self.interval * 0.8 + interval * 0.2
                    } else {
                        interval
                    };
                }
                self.last_beat = self.time;
                self.features.beats += 1;
            }
            if high_flux > self.high_floor * 2.0 + 0.012 && self.time - self.last_accent > 0.13 {
                self.last_accent = self.time;
                self.features.accents += 1;
            }
            if level > self.baseline + 0.13
                && level > self.baseline * 1.3
                && self.time - self.last_surge > 5.0
            {
                self.last_surge = self.time;
                self.features.surges += 1;
            }
        }
        self.baseline += (level - self.baseline) * (1.0 - (-dt / 2.5).exp());
        self.flux_floor += (flux - self.flux_floor) * (1.0 - (-dt / 0.8).exp());
        self.high_floor += (high_flux - self.high_floor) * (1.0 - (-dt / 0.8).exp());
        self.features.beat_confidence *= (-dt / 4.0).exp();
        self.features.level = level;
        self.features.low = bands[..16].iter().sum::<f32>() / 16.0;
        self.features.mid = bands[16..32].iter().sum::<f32>() / 16.0;
        self.features.high = bands[32..].iter().sum::<f32>() / 16.0;
        self.features.beat_period = self.interval;
        self.features.intensity = if level > 0.035 {
            (level * 0.45 + level / self.baseline.max(0.12) * 0.35).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.previous = *bands;
        self.features
    }
}

pub struct Analyzer {
    ring: [[f32; FFT]; 2],
    position: usize,
    filled: usize,
    hop: usize,
    rate: u32,
    music: MusicalAnalyzer,
    levels: ChannelLevels,
    channel_squares: [f32; 2],
    channel_max: [f32; 2],
    channel_samples: usize,
    channels: usize,
    display: AdaptiveSpectrum,
}
impl Default for Analyzer {
    fn default() -> Self {
        Self {
            ring: [[0.0; FFT]; 2],
            position: 0,
            filled: 0,
            hop: 0,
            rate: 0,
            music: MusicalAnalyzer::default(),
            levels: ChannelLevels::default(),
            channel_squares: [0.0; 2],
            channel_max: [0.0; 2],
            channel_samples: 0,
            channels: 0,
            display: AdaptiveSpectrum::default(),
        }
    }
}
impl Analyzer {
    pub fn add(&mut self, pcm: &[f32], rate: u32, channels: usize) -> Option<[f32; BANDS]> {
        if channels == 0 || rate < 8000 {
            return None;
        }
        if self.rate != rate || self.channels != channels {
            *self = Self {
                rate,
                channels,
                ..Default::default()
            };
        }
        let mut latest = None;
        for sample in pcm.chunks_exact(channels) {
            // Preserve both waveforms through the FFT. Merge powers afterwards
            // so stereo phase differences cannot cancel real spectral energy.
            for channel in 0..2 {
                let value = sample[channel.min(channels - 1)];
                let value = if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                self.channel_squares[channel] += value * value;
                self.channel_max[channel] = self.channel_max[channel].max(value.abs());
                self.ring[channel][self.position] = value;
            }
            self.channel_samples += 1;
            self.position = (self.position + 1) % FFT;
            self.filled = (self.filled + 1).min(FFT);
            self.hop += 1;
            if self.filled == FFT && self.hop >= FFT / 2 {
                self.levels = ChannelLevels {
                    rms: self
                        .channel_squares
                        .map(|sum| (sum / self.channel_samples as f32).sqrt()),
                    peak: self.channel_max,
                };
                self.channel_squares.fill(0.0);
                self.channel_max.fill(0.0);
                self.channel_samples = 0;
                self.hop = 0;
                let left = std::array::from_fn(|i| self.ring[0][(self.position + i) % FFT]);
                let mut power = fft_power(&left);
                if channels > 1 {
                    let right = std::array::from_fn(|i| self.ring[1][(self.position + i) % FFT]);
                    let right = fft_power(&right);
                    for (left, right) in power.iter_mut().zip(right) {
                        *left = (*left + right) * 0.5;
                    }
                }
                let bands = spectrum_view::project(&power, rate, spectrum_view::full_range(rate));
                let dt = (FFT / 2) as f32 / rate as f32;
                self.music.update(&bands, dt);
                self.display.update(&power, &bands, rate, dt);
                latest = Some(bands);
            }
        }
        latest
    }
}

fn fft_power(samples: &[f32; FFT]) -> [f32; FFT / 2] {
    let mut re = [0.0f32; FFT];
    let mut im = [0.0f32; FFT];
    for (i, sample) in samples.iter().enumerate() {
        re[i] = sample * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (FFT - 1) as f32).cos());
    }
    let mut j = 0;
    for i in 1..FFT {
        let mut bit = FFT >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= FFT {
        let angle = -std::f32::consts::TAU / length as f32;
        let (sin, cos) = angle.sin_cos();
        for start in (0..FFT).step_by(length) {
            let (mut wr, mut wi) = (1.0, 0.0);
            for k in 0..length / 2 {
                let (a, b) = (start + k, start + k + length / 2);
                let (tr, ti) = (wr * re[b] - wi * im[b], wr * im[b] + wi * re[b]);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                (wr, wi) = (wr * cos - wi * sin, wr * sin + wi * cos);
            }
        }
        length *= 2;
    }
    let scale = (4.0 / FFT as f32).powi(2);
    std::array::from_fn(|i| (re[i] * re[i] + im[i] * im[i]) * scale)
}

#[cfg(test)]
fn analyze(samples: &[f32; FFT], rate: u32) -> [f32; BANDS] {
    spectrum_view::project(&fft_power(samples), rate, spectrum_view::full_range(rate))
}

impl Analyzer {
    /// Position of the most recent FFT window, rather than the end of the input block.
    pub fn snapshot(&self, pcm_end: u64) -> AudioFeature {
        AudioFeature {
            end_sample: pcm_end.saturating_sub(self.hop as u64),
            display: self.display.frame,
            levels: self.levels,
            music: self.music.features,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channel_levels_preserve_stereo_separation_and_opposite_phase() {
        let mut analyzer = Analyzer::default();
        let pcm: Vec<_> = (0..FFT).flat_map(|_| [0.5, 0.125]).collect();
        assert!(analyzer.add(&pcm, 48000, 2).is_some());
        assert_eq!(analyzer.levels.rms, [0.5, 0.125]);
        assert_eq!(analyzer.levels.peak, [0.5, 0.125]);

        let pcm: Vec<_> = (0..FFT).flat_map(|_| [0.5, -0.5]).collect();
        let bands = analyzer.add(&pcm, 48000, 2).unwrap();
        assert!(bands.iter().all(|v| v.is_finite()));
        assert_eq!(analyzer.levels.rms, [0.5; 2]);
        assert_eq!(analyzer.levels.peak, [0.5; 2]);
    }

    #[test]
    fn mono_levels_match_and_source_changes_clear_old_measurements() {
        let mut analyzer = Analyzer::default();
        analyzer.add(&[0.5; FFT], 48000, 1).unwrap();
        assert_eq!(analyzer.levels.rms, [0.5; 2]);
        assert_eq!(analyzer.levels.peak, [0.5; 2]);
        analyzer.add(&[0.0; FFT * 2], 48000, 2).unwrap();
        assert_eq!(analyzer.levels.rms, [0.0; 2]);
        assert_eq!(analyzer.levels.peak, [0.0; 2]);
        let invalid: Vec<_> = (0..FFT).flat_map(|_| [f32::NAN, f32::INFINITY]).collect();
        analyzer.add(&invalid, 48000, 2).unwrap();
        assert_eq!(analyzer.levels.rms, [0.0; 2]);
    }

    #[test]
    fn opposite_phase_high_frequency_stereo_matches_mono_energy() {
        let mono: Vec<_> = (0..FFT * 2)
            .map(|i| (std::f32::consts::TAU * 8000.0 * i as f32 / 48000.0).sin() * 0.25)
            .collect();
        let stereo: Vec<_> = mono.iter().flat_map(|v| [*v, -*v]).collect();
        let reference = Analyzer::default().add(&mono, 48000, 1).unwrap();
        let measured = Analyzer::default().add(&stereo, 48000, 2).unwrap();
        assert!(
            measured[40..].iter().any(|v| *v > 0.5),
            "high-frequency stereo energy was cancelled"
        );
        for (a, b) in reference.iter().zip(measured) {
            assert!((a - b).abs() < 0.0001);
        }
    }

    #[test]
    fn adaptive_display_does_not_reclassify_low_bandwidth_audio_as_high_frequency() {
        let samples = std::array::from_fn::<_, FFT, _>(|i| {
            (1..=24)
                .map(|h| {
                    (std::f32::consts::TAU * 140.625 * h as f32 * i as f32 / 48000.0
                        + h as f32 * 0.71)
                        .sin()
                        * 0.012
                })
                .sum()
        });
        let mut analyzer = Analyzer::default();
        let mut fixed = [0.0; BANDS];
        for _ in 0..500 {
            fixed = analyzer.add(&samples, 48000, 1).unwrap();
        }
        assert!(analyzer.display.frame.max_hz <= 4500.0);
        assert!(fixed[40..].iter().all(|v| *v < 0.01));
        assert!(analyzer.display.frame.bands[40..].iter().any(|v| *v > 0.15));
        assert!(
            (analyzer.music.features.high - fixed[32..].iter().sum::<f32>() / 16.0).abs() < 0.00001
        );
    }

    #[test]
    fn fft_silence_and_known_tone() {
        assert_eq!(analyze(&[0.0; FFT], 48000), [0.0; BANDS]);
        for hz in [440.0, 1000.0, 5000.0] {
            let samples = std::array::from_fn(|i| {
                0.5 * (std::f32::consts::TAU * hz * i as f32 / 48000.0).sin()
            });
            let result = analyze(&samples, 48000);
            let peak = result
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            let expected = (hz / 40.0).ln() / (16000.0f32 / 40.0).ln() * BANDS as f32;
            assert!(
                (peak as f32 - expected).abs() < 2.0,
                "{hz}: band {peak}, expected {expected}"
            );
        }
    }
    #[test]
    fn silence_and_sustained_notes_do_not_keep_triggering_volleys() {
        let mut music = MusicalAnalyzer::default();
        for _ in 0..200 {
            music.update(&[0.0; BANDS], 0.02);
        }
        assert_eq!(music.features.beats, 0);
        assert_eq!(music.features.accents, 0);
        assert_eq!(music.features.intensity, 0.0);
        music.update(&[0.6; BANDS], 0.02);
        let beats = music.features.beats;
        let accents = music.features.accents;
        for _ in 0..200 {
            music.update(&[0.6; BANDS], 0.02);
        }
        assert_eq!(music.features.beats, beats);
        assert_eq!(music.features.accents, accents);
        assert!(music.features.intensity.is_finite());
    }

    #[test]
    fn adaptive_transients_detect_soft_and_loud_rhythms_without_retriggering() {
        for amplitude in [0.2, 0.8] {
            let mut music = MusicalAnalyzer::default();
            for frame in 0..500 {
                let pulse = if frame % 25 < 3 { amplitude } else { 0.0 };
                let bands = std::array::from_fn(|i| {
                    0.06 + if !(16..32).contains(&i) { pulse } else { 0.0 }
                });
                music.update(&bands, 0.02);
            }
            assert!(
                (18..=20).contains(&music.features.beats),
                "{amplitude}: {:?}",
                music.features
            );
            assert!((18..=20).contains(&music.features.accents));
            assert!(music.features.beat_confidence > 0.5);
        }
    }
}
