//! Audio content positions are local to (host instance, epoch), never wall clocks.
mod analysis;
pub mod display;

pub use analysis::Analyzer;
use serde::{Deserialize, Serialize};
pub const BANDS: usize = 48;
pub const WAVEFORM_POINTS: usize = 128;

/// Signed min/max PCM envelope, combining stereo extrema without phase cancellation.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Waveform {
    #[serde(with = "float_array")]
    pub min: [f32; WAVEFORM_POINTS],
    #[serde(with = "float_array")]
    pub max: [f32; WAVEFORM_POINTS],
}
impl Default for Waveform {
    fn default() -> Self {
        Self {
            min: [0.0; WAVEFORM_POINTS],
            max: [0.0; WAVEFORM_POINTS],
        }
    }
}
impl Waveform {
    pub fn valid(&self) -> bool {
        self.min.iter().zip(&self.max).all(|(min, max)| {
            min.is_finite() && max.is_finite() && *min >= -1.0 && *max <= 1.0 && min <= max
        })
    }
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveform: Option<Waveform>,
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
            && feature.waveform.as_ref().is_none_or(Waveform::valid)
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

mod float_array {
    use super::*;
    pub fn serialize<S: serde::Serializer, const N: usize>(
        value: &[f32; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_slice().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[f32; N], D::Error> {
        Vec::<f32>::deserialize(deserializer)?
            .try_into()
            .map_err(|_| serde::de::Error::custom(format!("expected exactly {N} audio values")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_transport_is_bounded_validated_and_compatible_with_older_hosts() {
        let mut analyzer = Analyzer::default();
        analyzer.add(&vec![0.3; 4096], 48000, 2).unwrap();
        let mut frame = AudioFrame::empty("wave-host".into(), 1);
        frame.active = true;
        frame.sample_rate = 48000;
        frame.playback_sample = 2048;
        frame.feature = Some(analyzer.snapshot(2048));
        assert!(frame.valid());
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.len() < 16384);
        let decoded: AudioFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(
            decoded.feature.unwrap().waveform.unwrap().max,
            [0.3; WAVEFORM_POINTS]
        );
        let mut legacy = serde_json::to_value(&frame).unwrap();
        legacy["feature"]
            .as_object_mut()
            .unwrap()
            .remove("waveform");
        let decoded: AudioFrame = serde_json::from_value(legacy).unwrap();
        assert!(decoded.valid());
        assert!(decoded.feature.unwrap().waveform.is_none());
        let mut malformed = serde_json::to_value(&frame).unwrap();
        malformed["feature"]["waveform"]["min"] = serde_json::json!([0.0]);
        assert!(serde_json::from_value::<AudioFrame>(malformed).is_err());
        for (min, max) in [
            (f32::NAN, 0.3),
            (-0.3, f32::INFINITY),
            (-1.1, 0.3),
            (-0.3, 1.1),
            (0.4, 0.3),
        ] {
            let wave = frame.feature.as_mut().unwrap().waveform.as_mut().unwrap();
            wave.min[0] = min;
            wave.max[0] = max;
            assert!(!frame.valid());
        }
    }

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
