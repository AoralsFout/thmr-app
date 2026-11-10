//! Audio content positions are local to (host instance, epoch), never wall clocks.
mod analysis;
pub mod display;

pub use analysis::Analyzer;
use serde::{Deserialize, Serialize};
pub const BANDS: usize = 48;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicalFeatures {
    pub level: f32,
    pub intensity: f32,
    pub low: f32,
    pub mid: f32,
    pub high: f32,
    pub beat_period: f32,
    pub beat_confidence: f32,
    pub beats: u64,
    pub accents: u64,
    pub surges: u64,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct ChannelLevels {
    pub rms: [f32; 2],
    pub peak: [f32; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioFeature {
    pub end_sample: u64,
    pub display: display::DisplaySpectrum,
    pub levels: ChannelLevels,
    pub music: MusicalFeatures,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioQueueStats {
    pub decoded_samples: u64,
    pub pcm_bytes: usize,
    pub pending_features: usize,
    pub retired_features: u64,
    pub max_pcm_bytes: usize,
    pub max_pending_features: usize,
}

/// A self-contained, already-due feature selected by the host's audio clock.
/// This has its own version and delivery cadence; normal state snapshots stay small.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioFrame {
    pub audio_version: u32,
    pub instance_id: String,
    pub epoch: u64,
    pub sequence: u64,
    pub active: bool,
    pub sample_rate: u32,
    pub playback_sample: u64,
    pub feature: Option<AudioFeature>,
    pub queue: AudioQueueStats,
}

impl AudioFrame {
    pub fn empty(instance_id: String, epoch: u64) -> Self {
        Self {
            audio_version: 1,
            instance_id,
            epoch,
            sequence: 0,
            active: false,
            sample_rate: 0,
            playback_sample: 0,
            feature: None,
            queue: Default::default(),
        }
    }

    pub fn valid(&self) -> bool {
        if self.audio_version != 1 {
            return false;
        }
        let Some(feature) = &self.feature else {
            return !self.active;
        };
        let unit = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        (8000..=192000).contains(&self.sample_rate)
            && feature.end_sample <= self.playback_sample
            && feature.display.bands.iter().copied().all(unit)
            && feature
                .levels
                .rms
                .iter()
                .chain(&feature.levels.peak)
                .copied()
                .all(unit)
            && feature.display.max_hz.is_finite()
            && (40.0..=96000.0).contains(&feature.display.max_hz)
            && [
                feature.music.level,
                feature.music.intensity,
                feature.music.low,
                feature.music.mid,
                feature.music.high,
                feature.music.beat_confidence,
            ]
            .into_iter()
            .all(unit)
            && feature.music.beat_period.is_finite()
            && (0.0..=2.0).contains(&feature.music.beat_period)
    }
}

mod band_array {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &[f32; BANDS],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_slice().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[f32; BANDS], D::Error> {
        Vec::<f32>::deserialize(deserializer)?
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected exactly 48 audio bands"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transport_preserves_full_spectrum_and_rejects_future_or_nonfinite_features() {
        let mut analyzer = Analyzer::default();
        analyzer.add(&vec![0.1; 4096], 48000, 2).unwrap();
        let mut frame = AudioFrame::empty("host".into(), 3);
        frame.sample_rate = 48000;
        frame.playback_sample = 2048;
        frame.active = true;
        frame.feature = Some(analyzer.snapshot(2048));
        assert!(frame.valid());
        let json = serde_json::to_string(&frame).unwrap();
        assert!(serde_json::from_str::<AudioFrame>(&json).unwrap().valid());
        frame.playback_sample = 2047;
        assert!(!frame.valid());
        frame.playback_sample = 2048;
        frame.feature.as_mut().unwrap().music.level = f32::NAN;
        assert!(!frame.valid());
    }
}
