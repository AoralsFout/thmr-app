//! One upstream and decoder per playback epoch. Playback consumes PCM; the host
//! clock retires precomputed features. Neither consumer owns a second radio stream.
pub mod decoder;
mod trace;

use anyhow::{bail, Result};
use futures_util::StreamExt;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant, SystemTime},
};
use thmr_core::audio::{Analyzer, AudioFeature, AudioFrame, AudioQueueStats};
use tokio::{sync::Notify, task::JoinHandle};

const AHEAD_SECONDS: u64 = 3;
const MAX_PENDING: usize = 32768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: usize,
}

#[derive(Clone, Debug)]
pub struct Failure {
    pub message: String,
    pub retry_at: Option<Instant>,
}

#[derive(Clone)]
pub struct Network {
    client: reqwest::Client,
    cooldown: Arc<Mutex<HashMap<String, Instant>>>,
}
impl Network {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .read_timeout(Duration::from_secs(8))
            .user_agent("THMR-Client/1.0 (+https://thmr.yunmoan.cn)")
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5
                    || attempt
                        .previous()
                        .first()
                        .is_some_and(|first| first.origin() != attempt.url().origin())
                {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()?;
        Ok(Self {
            client,
            cooldown: Arc::default(),
        })
    }

    async fn open(&self, url: &str) -> std::result::Result<reqwest::Response, Failure> {
        let origin = reqwest::Url::parse(url)
            .map_err(|_| Failure {
                message: "invalid radio URL".into(),
                retry_at: None,
            })?
            .origin()
            .ascii_serialization();
        let retry_at = self
            .cooldown
            .lock()
            .unwrap()
            .get(&origin)
            .copied()
            .filter(|t| *t > Instant::now());
        if let Some(retry_at) = retry_at {
            return Err(Failure {
                message: "radio service is cooling down".into(),
                retry_at: Some(retry_at),
            });
        }
        let mut request = self.client.get(url).header("Icy-MetaData", "0");
        if let Ok(token) = std::env::var("THMR_STREAM_TOKEN") {
            if !token.is_empty() {
                request = request.header("X-Stream-Token", token);
            }
        }
        let response = request.send().await.map_err(|_| Failure {
            message: "radio connection failed".into(),
            retry_at: None,
        })?;
        let code = response.status().as_u16();
        if response.status().is_success() {
            return Ok(response);
        }
        let requested = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.parse::<u64>().ok().map(Duration::from_secs).or_else(|| {
                    httpdate::parse_http_date(v)
                        .ok()
                        .map(|t| t.duration_since(SystemTime::now()).unwrap_or_default())
                })
            })
            .unwrap_or_default()
            .min(Duration::from_secs(86400));
        let delay = if matches!(code, 403 | 429) {
            requested.max(Duration::from_secs(60))
        } else {
            requested.max(Duration::from_secs(10))
        };
        let until = Instant::now() + delay;
        self.cooldown.lock().unwrap().insert(origin, until);
        Err(Failure {
            message: format!("radio returned HTTP {code}"),
            retry_at: Some(until),
        })
    }
}

#[derive(Default)]
struct Buffer {
    pcm: VecDeque<u8>,
    features: VecDeque<AudioFeature>,
    selected: Option<AudioFeature>,
    format: Option<Format>,
    played: u64,
    stats: AudioQueueStats,
    ended: bool,
    cancelled: bool,
    failure: Option<Failure>,
}
struct Shared {
    buffer: Mutex<Buffer>,
    readable: Condvar,
    progress: Notify,
}

#[derive(Clone)]
pub struct PcmStream(Arc<Shared>);
impl PcmStream {
    fn new() -> Self {
        Self(Arc::new(Shared {
            buffer: Mutex::new(Buffer::default()),
            readable: Condvar::new(),
            progress: Notify::new(),
        }))
    }

    /// Called only by libmpv's demux thread. Zero means actual EOF, never a temporary underrun.
    pub fn read(&self, out: &mut [u8]) -> i64 {
        let mut buffer = self.0.buffer.lock().unwrap();
        while buffer.pcm.is_empty() && !buffer.ended && !buffer.cancelled {
            buffer = self.0.readable.wait(buffer).unwrap();
        }
        if buffer.cancelled {
            return -1;
        }
        let count = out.len().min(buffer.pcm.len());
        let (first, second) = buffer.pcm.as_slices();
        let left = count.min(first.len());
        out[..left].copy_from_slice(&first[..left]);
        out[left..count].copy_from_slice(&second[..count - left]);
        buffer.pcm.drain(..count);
        buffer.stats.pcm_bytes = buffer.pcm.len();
        count as i64
    }

    pub fn cancel(&self) {
        self.0.buffer.lock().unwrap().cancelled = true;
        self.0.readable.notify_all();
        self.0.progress.notify_waiters();
    }

    fn ahead_full(&self) -> bool {
        let buffer = self.0.buffer.lock().unwrap();
        buffer.format.is_some_and(|format| {
            buffer.stats.decoded_samples.saturating_sub(buffer.played)
                >= format.rate as u64 * AHEAD_SECONDS
        })
    }

    fn push(&self, pcm: &[f32], format: Format, feature: Option<AudioFeature>) -> Result<()> {
        let mut buffer = self.0.buffer.lock().unwrap();
        if buffer.cancelled {
            bail!("audio epoch cancelled");
        }
        if buffer.format.is_some_and(|previous| previous != format) {
            bail!("radio audio format changed");
        }
        buffer.format = Some(format);
        buffer.stats.decoded_samples += (pcm.len() / format.channels) as u64;
        if let Some(feature) = feature {
            buffer.features.push_back(feature);
        }
        for sample in pcm {
            buffer.pcm.extend(sample.to_le_bytes());
        }
        buffer.stats.pcm_bytes = buffer.pcm.len();
        buffer.stats.pending_features = buffer.features.len();
        buffer.stats.max_pcm_bytes = buffer.stats.max_pcm_bytes.max(buffer.pcm.len());
        buffer.stats.max_pending_features =
            buffer.stats.max_pending_features.max(buffer.features.len());
        drop(buffer);
        self.0.readable.notify_all();
        Ok(())
    }

    fn present(
        &self,
        instance: &str,
        epoch: u64,
        sequence: u64,
        position: Option<f64>,
        active: bool,
    ) -> AudioFrame {
        let mut buffer = self.0.buffer.lock().unwrap();
        let mut frame = AudioFrame::empty(instance.into(), epoch);
        frame.sequence = sequence;
        if let Some(format) = buffer.format {
            frame.sample_rate = format.rate;
            if let Some(position) = position.filter(|p| p.is_finite() && *p >= 0.0) {
                buffer.played = (position * format.rate as f64) as u64;
                if active {
                    // A clock discontinuity must never release a future feature.
                    if buffer
                        .selected
                        .as_ref()
                        .is_some_and(|f| f.end_sample > buffer.played)
                    {
                        buffer.selected = None;
                    }
                    while buffer
                        .features
                        .front()
                        .is_some_and(|f| f.end_sample <= buffer.played)
                    {
                        buffer.selected = buffer.features.pop_front();
                        buffer.stats.retired_features += 1;
                    }
                    frame.feature = buffer.selected.clone();
                }
                frame.playback_sample = buffer.played;
            }
        }
        frame.active = active && frame.feature.is_some();
        buffer.stats.pending_features = buffer.features.len();
        frame.queue = buffer.stats.clone();
        drop(buffer);
        self.0.progress.notify_one();
        frame
    }
}

/// Owns cancellation and epoch lifetime, not the libmpv handle.
pub struct Pipeline {
    pub epoch: u64,
    stream: PcmStream,
    task: JoinHandle<()>,
    present_trace: Option<trace::Presentation>,
}
impl Pipeline {
    pub fn start(network: Network, url: String, epoch: u64) -> Self {
        let stream = PcmStream::new();
        let worker = stream.clone();
        // Open evidence files before spawning: Windows sandbox directory creation
        // can race with the presentation trace's initialization for this epoch.
        let source_trace = trace::Source::new(epoch);
        let task = tokio::spawn(async move {
            let result = receive(&worker, &network, &url, source_trace).await;
            let mut buffer = worker.0.buffer.lock().unwrap();
            buffer.ended = true;
            if let Err(failure) = result {
                if !buffer.cancelled {
                    buffer.failure = Some(failure);
                }
            }
            drop(buffer);
            worker.0.readable.notify_all();
        });
        Self {
            epoch,
            stream,
            task,
            present_trace: trace::Presentation::new(epoch),
        }
    }
    pub fn format(&self) -> Option<Format> {
        self.stream.0.buffer.lock().unwrap().format
    }
    pub fn failure(&self) -> Option<Failure> {
        self.stream.0.buffer.lock().unwrap().failure.clone()
    }
    pub fn stream(&self) -> PcmStream {
        self.stream.clone()
    }
    pub fn present(
        &mut self,
        instance: &str,
        sequence: u64,
        position: Option<f64>,
        active: bool,
    ) -> AudioFrame {
        let frame = self
            .stream
            .present(instance, self.epoch, sequence, position, active);
        if let Some(trace) = &mut self.present_trace {
            trace.record(&frame);
        }
        frame
    }
}
impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stream.cancel();
        self.task.abort();
    }
}

async fn receive(
    stream: &PcmStream,
    network: &Network,
    url: &str,
    mut trace: Option<trace::Source>,
) -> std::result::Result<(), Failure> {
    if let Some(trace) = &mut trace {
        trace.request();
    }
    let response = network.open(url).await?;
    let mut chunks = response.bytes_stream();
    let mut decoder = decoder::Decoder::new().map_err(|_| Failure {
        message: "MP3 decoder initialization failed".into(),
        retry_at: None,
    })?;
    let mut analyzer = Analyzer::default();
    let mut pending = Vec::with_capacity(MAX_PENDING);
    let mut decoded = 0u64;
    let mut synced = false;
    let mut last_pcm = Instant::now();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| Failure {
            message: "radio stream interrupted".into(),
            retry_at: None,
        })?;
        for part in chunk.chunks(8192) {
            pending.extend_from_slice(part);
            let mut consumed = 0;
            while pending.len() - consumed >= if synced { 4096 } else { 8192 } {
                while stream.ahead_full() {
                    tokio::select! {_=stream.0.progress.notified()=>{},_=tokio::time::sleep(Duration::from_millis(20))=>{}}
                }
                let (advance, pcm, rate, channels) = decoder.decode(&pending[consumed..]);
                if advance == 0 {
                    break;
                }
                let advance = if pcm.is_empty() {
                    advance.min(pending.len() - consumed - 4096)
                } else {
                    advance
                };
                if advance == 0 {
                    break;
                }
                consumed += advance;
                if pcm.is_empty() {
                    continue;
                }
                if !(8000..=192000).contains(&rate) || !(1..=2).contains(&channels) {
                    return Err(Failure {
                        message: "unsupported radio audio format".into(),
                        retry_at: None,
                    });
                }
                synced = true;
                last_pcm = Instant::now();
                decoded += (pcm.len() / channels) as u64;
                let feature = analyzer
                    .add(pcm, rate, channels)
                    .map(|_| analyzer.snapshot(decoded));
                if let Some(trace) = &mut trace {
                    trace.pcm(pcm, rate, channels, decoded);
                }
                stream
                    .push(pcm, Format { rate, channels }, feature)
                    .map_err(|_| Failure {
                        message: "radio audio format changed or epoch ended".into(),
                        retry_at: None,
                    })?;
            }
            pending.drain(..consumed);
            if pending.len() >= MAX_PENDING {
                pending.drain(..pending.len() - 8192);
            }
        }
        if last_pcm.elapsed() > Duration::from_secs(10) {
            return Err(Failure {
                message: "no MP3 audio decoded".into(),
                retry_at: None,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vendored_decoder_preserves_mono_and_stereo_fixture_formats() {
        for (data, expected_rate) in [
            (
                include_bytes!("../../../../test/fixtures/sine_440hz_48000_mono.mp3").as_slice(),
                48000,
            ),
            (
                include_bytes!("../../../../test/fixtures/sine_1000hz_44100_stereo.mp3").as_slice(),
                44100,
            ),
        ] {
            let mut decoder = decoder::Decoder::new().unwrap();
            let mut analyzer = Analyzer::default();
            let mut position = 0;
            let mut frames = 0;
            while position < data.len() {
                let (advance, pcm, rate, channels) = decoder.decode(&data[position..]);
                if advance == 0 {
                    break;
                }
                position += advance;
                if !pcm.is_empty() {
                    assert_eq!(rate, expected_rate);
                    if analyzer.add(pcm, rate, channels).is_some() {
                        frames += 1;
                    }
                }
            }
            assert!(frames > 2);
        }
    }

    #[tokio::test]
    async fn playback_cooldown_survives_new_epochs_and_cloned_networks() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/radio", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                let mut request = [0; 2048];
                let _ = socket.read(&mut request).await;
                let _=socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 120\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            }
        });
        let network = Network::new().unwrap();
        let failure = network.open(&url).await.err().unwrap();
        assert!(failure.retry_at.unwrap() > Instant::now() + Duration::from_secs(115));
        assert!(network.clone().open(&url).await.is_err());
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }
    fn feature(sample: u64) -> AudioFeature {
        AudioFeature {
            end_sample: sample,
            display: Default::default(),
            levels: Default::default(),
            music: Default::default(),
        }
    }

    #[test]
    fn buffered_features_wait_for_playback_and_retire_without_their_own_clock() {
        let stream = PcmStream::new();
        let format = Format {
            rate: 48000,
            channels: 1,
        };
        stream
            .push(&vec![0.1; 48000], format, Some(feature(24000)))
            .unwrap();
        stream
            .push(&vec![0.2; 48000], format, Some(feature(72000)))
            .unwrap();
        assert!(stream
            .present("host", 1, 1, Some(0.25), true)
            .feature
            .is_none());
        let first = stream.present("host", 1, 2, Some(0.6), true);
        assert_eq!(first.feature.unwrap().end_sample, 24000);
        assert_eq!(first.queue.retired_features, 1);
        // Repeated updates / buffering must not advance content or consume future data.
        for sequence in 3..20 {
            let frame = stream.present("host", 1, sequence, Some(0.6), false);
            assert!(!frame.active);
            assert_eq!(frame.queue.retired_features, 1);
        }
        let resumed = stream.present("host", 1, 20, Some(1.6), true);
        assert_eq!(resumed.feature.unwrap().end_sample, 72000);
        assert_eq!(resumed.queue.pending_features, 0);
        assert_eq!(resumed.queue.retired_features, 2);
    }

    #[test]
    fn cancellation_wakes_blocking_pcm_reader_and_new_epoch_has_no_old_features() {
        let stream = PcmStream::new();
        let reader = stream.clone();
        let thread = std::thread::spawn(move || reader.read(&mut [0; 32]));
        stream.cancel();
        assert_eq!(thread.join().unwrap(), -1);
        let fresh = PcmStream::new().present("host", 2, 1, Some(9.0), true);
        assert!(!fresh.active);
        assert!(fresh.feature.is_none());
        assert_eq!(fresh.epoch, 2);
    }

    #[test]
    fn ahead_limit_uses_playback_not_demux_reads_and_clock_regression_rejects_future_data() {
        let stream = PcmStream::new();
        let format = Format {
            rate: 8000,
            channels: 1,
        };
        stream
            .push(&vec![0.0; 24000], format, Some(feature(16000)))
            .unwrap();
        assert!(stream.ahead_full());
        assert_eq!(stream.read(&mut vec![0; 96000]), 96000);
        assert!(stream.ahead_full(), "prefetching PCM is not playback");
        let played = stream.present("host", 1, 1, Some(2.1), true);
        assert!(played.feature.is_some());
        assert!(!stream.ahead_full());
        assert!(stream
            .present("host", 1, 2, Some(1.0), true)
            .feature
            .is_none());
    }

    #[tokio::test]
    async fn single_upstream_shared_pcm_decodes_a_fragmented_stream_and_cancels_on_pause() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/radio", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            count.fetch_add(1, Ordering::SeqCst);
            let mut request = [0; 2048];
            let _ = socket.read(&mut request).await;
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
            let audio = include_bytes!("../../../../test/fixtures/sine_1000hz_44100_stereo.mp3");
            for part in audio.chunks(137) {
                if socket
                    .write_all(format!("{:x}\r\n", part.len()).as_bytes())
                    .await
                    .is_err()
                {
                    return;
                }
                if socket.write_all(part).await.is_err() {
                    return;
                }
                if socket.write_all(b"\r\n").await.is_err() {
                    return;
                }
            }
            let _ = socket.read(&mut request).await;
        });
        let pipeline = Pipeline::start(Network::new().unwrap(), url, 1);
        tokio::time::timeout(Duration::from_secs(5), async {
            while pipeline.format().is_none() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let pcm = pipeline.stream();
        let mut bytes = vec![0; 4096];
        assert!(pcm.read(&mut bytes) > 0);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        drop(pipeline);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pcm.read(&mut bytes), -1);
    }
}
