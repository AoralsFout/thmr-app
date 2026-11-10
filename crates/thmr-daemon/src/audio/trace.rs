//! Opt-in developer evidence, never enabled by normal playback. No URLs or credentials.
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use thmr_core::audio::AudioFrame;
fn wall() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn directory(epoch: u64) -> Option<PathBuf> {
    let directory =
        PathBuf::from(std::env::var_os("THMR_AUDIO_TRACE_DIR")?).join(format!("epoch-{epoch}"));
    fs::create_dir_all(&directory).ok()?;
    Some(directory)
}
pub struct Source {
    pcm: BufWriter<File>,
    decode: BufWriter<File>,
    requests: BufWriter<File>,
}
impl Source {
    pub fn new(epoch: u64) -> Option<Self> {
        let path = directory(epoch)?;
        Some(Self {
            pcm: BufWriter::new(File::create(path.join("source.f32")).ok()?),
            decode: BufWriter::new(File::create(path.join("decode.csv")).ok()?),
            requests: BufWriter::new(File::create(path.join("network.csv")).ok()?),
        })
    }
    pub fn request(&mut self) {
        let _ = writeln!(self.requests, "{},request,1", wall());
        let _ = self.requests.flush();
    }
    pub fn pcm(&mut self, pcm: &[f32], rate: u32, channels: usize, end: u64) {
        for frame in pcm.chunks_exact(channels) {
            let _ = self.pcm.write_all(&frame[0].to_le_bytes());
        }
        let _ = writeln!(self.decode, "{},{},{}", wall(), end, rate);
    }
}
pub struct Presentation(BufWriter<File>);
impl Presentation {
    pub fn new(epoch: u64) -> Option<Self> {
        Some(Self(BufWriter::new(
            File::create(directory(epoch)?.join("present.csv")).ok()?,
        )))
    }
    pub fn record(&mut self, frame: &AudioFrame) {
        let end = frame.feature.as_ref().map(|f| f.end_sample).unwrap_or(0);
        let _ = writeln!(
            self.0,
            "{},{},{},{},{},{},{},{}",
            wall(),
            frame.playback_sample,
            end,
            frame.sample_rate,
            frame.active as u8,
            frame.queue.decoded_samples,
            frame.queue.pending_features,
            frame.queue.retired_features
        );
    }
}
