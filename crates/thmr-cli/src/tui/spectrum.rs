//! Receives already-due host features over authenticated loopback. Never opens a radio URL.
use anyhow::{bail, Result};
use futures_util::StreamExt;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub use thmr_core::audio::{ChannelLevels, MusicalFeatures, BANDS};
use thmr_core::{audio::AudioFrame, client::Client, PROTOCOL_VERSION};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};

pub struct Spectrum {
    pub display: thmr_core::audio::display::DisplaySpectrum,
    pub levels: ChannelLevels,
    pub music: MusicalFeatures,
    pub updated: Option<Instant>,
    pub status: &'static str,
    pub instance_id: String,
    pub epoch: u64,
    pub end_sample: u64,
}
impl Default for Spectrum {
    fn default() -> Self {
        Self {
            display: Default::default(),
            levels: Default::default(),
            music: Default::default(),
            updated: None,
            status: "等待播放",
            instance_id: String::new(),
            epoch: 0,
            end_sample: 0,
        }
    }
}
pub type SharedSpectrum = Arc<Mutex<Spectrum>>;

pub fn source(state: &thmr_core::protocol::Snapshot) -> Option<String> {
    Some(state.instance_id.clone())
}

pub async fn run(mut enabled: watch::Receiver<Option<String>>, output: SharedSpectrum) {
    loop {
        let instance = enabled.borrow_and_update().clone();
        let Some(instance) = instance else {
            *output.lock().unwrap() = Spectrum::default();
            if enabled.changed().await.is_err() {
                return;
            }
            continue;
        };
        tokio::select! {
            _=subscribe(&instance,&output)=> {
                *output.lock().unwrap()=Spectrum{status:"宿主音频指标已断开，正在重连",..Default::default()};
                tokio::select! {
                    _=tokio::time::sleep(Duration::from_secs(1))=>{},
                    changed=enabled.changed()=>{if changed.is_err(){return;}}
                }
            }
            changed=enabled.changed()=>{if changed.is_err(){return;}}
        }
    }
}

async fn subscribe(instance: &str, output: &SharedSpectrum) -> Result<()> {
    let client = Client::discover().await?;
    if client.endpoint.instance_id != instance {
        bail!("host instance changed");
    }
    let mut request = client
        .endpoint
        .url("/v1/audio")
        .replacen("http:", "ws:", 1)
        .into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", client.endpoint.token))?,
    );
    request.headers_mut().insert(
        "X-THMR-Protocol",
        HeaderValue::from_str(&PROTOCOL_VERSION.to_string())?,
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
    let mut last = (0u64, 0u64);
    while let Some(message) = socket.next().await {
        if let Message::Text(text) = message? {
            if text.len() > 16384 {
                bail!("oversized audio frame");
            }
            let frame: AudioFrame = serde_json::from_str(&text)?;
            if frame.instance_id != instance || !frame.valid() {
                bail!("invalid host audio frame");
            }
            if (frame.epoch, frame.sequence) < last {
                continue;
            }
            last = (frame.epoch, frame.sequence);
            accept(frame, output);
        }
    }
    bail!("host audio connection ended")
}

fn accept(frame: AudioFrame, output: &SharedSpectrum) {
    let mut output = output.lock().unwrap();
    let previous_music = (output.instance_id == frame.instance_id && output.epoch == frame.epoch)
        .then_some(output.music);
    let mut value = Spectrum {
        instance_id: frame.instance_id,
        epoch: frame.epoch,
        status: "等待播放位置对应的指标",
        ..Default::default()
    };
    if frame.active {
        if let Some(feature) = frame.feature {
            value.end_sample = feature.end_sample;
            value.display = feature.display;
            value.levels = feature.levels;
            value.music = feature.music;
            value.updated = Some(Instant::now());
            value.status = "播放时钟同步频谱";
        }
    } else if let Some(music) = previous_music {
        value.music = music;
    }
    *output = value;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_epoch_and_inactive_frames_clear_old_content_instead_of_showing_stale_metrics() {
        let output = Arc::new(Mutex::new(Spectrum {
            updated: Some(Instant::now()),
            end_sample: 999,
            music: MusicalFeatures {
                beats: 500,
                ..Default::default()
            },
            ..Default::default()
        }));
        accept(AudioFrame::empty("host".into(), 2), &output);
        let value = output.lock().unwrap();
        assert_eq!(value.epoch, 2);
        assert_eq!(value.end_sample, 0);
        assert!(value.updated.is_none());
        assert_eq!(value.music.beats, 0);
    }
    #[test]
    fn subscription_identity_is_the_host_not_the_radio_url() {
        let state = thmr_core::protocol::Snapshot::new("host".into(), &Default::default());
        assert_eq!(source(&state).as_deref(), Some("host"));
    }
    #[test]
    fn buffering_preserves_event_baseline_without_marking_old_data_as_fresh() {
        let output = Arc::new(Mutex::new(Spectrum {
            instance_id: "host".into(),
            epoch: 4,
            updated: Some(Instant::now()),
            music: MusicalFeatures {
                beats: 200,
                ..Default::default()
            },
            ..Default::default()
        }));
        accept(AudioFrame::empty("host".into(), 4), &output);
        let value = output.lock().unwrap();
        assert!(value.updated.is_none());
        assert_eq!(value.end_sample, 0);
        assert_eq!(value.music.beats, 200);
    }
}
