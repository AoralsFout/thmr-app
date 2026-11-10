//! EXPERIMENT ONLY: one live MP3 request -> one decode -> PCM playback + queued features.
//! Built by tool/verify_single_stream_pcm.py. Does not modify the normal host.
#![allow(dead_code)]
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use std::{
    collections::VecDeque,
    ffi::{c_char, c_int, c_void, CStr, CString},
    fs::File,
    io::{BufWriter, Write},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thmr_core::protocol::{Phase, Snapshot};

// Paths are injected into a generated Cargo example, pointing to copies of the
// current production modules plus the small decoder/analyzer access wrapper.
// MODULES

fn wall() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

#[derive(Clone)]
struct Feature {
    end: u64,
    computed: f64,
    display: spectrum_view::DisplaySpectrum,
    levels: spectrum::ChannelLevels,
    music: spectrum::MusicalFeatures,
}
struct Data {
    pcm: VecDeque<u8>,
    features: VecDeque<Feature>,
    rate: u32,
    channels: usize,
    decoded: u64,
    played: u64,
    read_bytes: u64,
    retired: u64,
    max_pcm_bytes: usize,
    max_features: usize,
    max_ahead_s: f64,
    done: bool,
    cancelled: bool,
    error: Option<String>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            pcm: VecDeque::new(),
            features: VecDeque::new(),
            rate: 0,
            channels: 0,
            decoded: 0,
            played: 0,
            read_bytes: 0,
            retired: 0,
            max_pcm_bytes: 0,
            max_features: 0,
            max_ahead_s: 0.0,
            done: false,
            cancelled: false,
            error: None,
        }
    }
}
type Shared = Arc<(Mutex<Data>, Condvar)>;

#[repr(C)]
struct StreamInfo {
    cookie: *mut c_void,
    read: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, u64) -> i64>,
    seek: Option<unsafe extern "C" fn(*mut c_void, i64) -> i64>,
    size: Option<unsafe extern "C" fn(*mut c_void) -> i64>,
    close: Option<unsafe extern "C" fn(*mut c_void)>,
    cancel: Option<unsafe extern "C" fn(*mut c_void)>,
}
unsafe extern "C" fn stream_read(cookie: *mut c_void, buf: *mut c_char, size: u64) -> i64 {
    let shared = &*(cookie as *const Shared);
    let mut data = shared.0.lock().unwrap();
    while data.pcm.is_empty() && !data.done && !data.cancelled {
        data = shared.1.wait(data).unwrap();
    }
    if data.cancelled {
        return -1;
    }
    let count = (size as usize).min(data.pcm.len()).min(65536);
    for i in 0..count {
        *buf.add(i) = data.pcm.pop_front().unwrap() as c_char;
    }
    data.read_bytes += count as u64;
    count as i64
}
unsafe extern "C" fn stream_close(_cookie: *mut c_void) {}
unsafe extern "C" fn stream_cancel(cookie: *mut c_void) {
    let shared = &*(cookie as *const Shared);
    shared.0.lock().unwrap().cancelled = true;
    shared.1.notify_all();
}
unsafe extern "C" fn stream_open(
    user: *mut c_void,
    _uri: *mut c_char,
    info: *mut StreamInfo,
) -> c_int {
    *info = StreamInfo {
        cookie: user,
        read: Some(stream_read),
        seek: None,
        size: None,
        close: Some(stream_close),
        cancel: Some(stream_cancel),
    };
    0
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(path: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> c_int;
}
struct Player {
    library: *mut c_void,
    handle: *mut c_void,
    command: unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int,
    get: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int,
    destroy: unsafe extern "C" fn(*mut c_void),
    _cookie: Box<Shared>,
}
impl Player {
    unsafe fn new(shared: Shared, rate: u32, channels: usize) -> Result<Self> {
        use std::os::windows::ffi::OsStrExt;
        let path = std::path::PathBuf::from(
            std::env::var_os("THMR_MPV_LIBRARY").context("set THMR_MPV_LIBRARY")?,
        );
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let library = LoadLibraryW(path.as_ptr());
        if library.is_null() {
            bail!("LoadLibrary libmpv failed");
        }
        unsafe fn symbol<T: Copy>(library: *mut c_void, name: &[u8]) -> Result<T> {
            let pointer = GetProcAddress(library, name.as_ptr().cast());
            if pointer.is_null() {
                bail!("missing libmpv symbol");
            }
            Ok(std::mem::transmute_copy(&pointer))
        }
        let create: unsafe extern "C" fn() -> *mut c_void = symbol(library, b"mpv_create\0")?;
        let option: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int =
            symbol(library, b"mpv_set_option_string\0")?;
        let initialize: unsafe extern "C" fn(*mut c_void) -> c_int =
            symbol(library, b"mpv_initialize\0")?;
        let register: unsafe extern "C" fn(
            *mut c_void,
            *const c_char,
            *mut c_void,
            unsafe extern "C" fn(*mut c_void, *mut c_char, *mut StreamInfo) -> c_int,
        ) -> c_int = symbol(library, b"mpv_stream_cb_add_ro\0")?;
        let handle = create();
        if handle.is_null() {
            bail!("mpv_create failed");
        }
        let cookie = Box::new(shared);
        let player = Self {
            library,
            handle,
            command: symbol(library, b"mpv_command\0")?,
            get: symbol(library, b"mpv_get_property\0")?,
            destroy: symbol(library, b"mpv_terminate_destroy\0")?,
            _cookie: cookie,
        };
        let channel_layout = if channels == 1 { "mono" } else { "stereo" };
        for (name, value) in [
            ("config", "no"),
            ("terminal", "no"),
            ("vo", "null"),
            ("vid", "no"),
            ("idle", "yes"),
            ("input-default-bindings", "no"),
            ("demuxer", "rawaudio"),
            ("demuxer-rawaudio-format", "floatle"),
            ("demuxer-rawaudio-channels", channel_layout),
            ("cache", "yes"),
            ("cache-secs", "1"),
            ("cache-pause-wait", "0.5"),
            ("demuxer-max-bytes", "1048576"),
        ] {
            let result = option(
                handle,
                CString::new(name)?.as_ptr(),
                CString::new(value)?.as_ptr(),
            );
            if result < 0 {
                bail!("option {name} failed: {result}");
            }
        }
        if option(
            handle,
            c"demuxer-rawaudio-rate".as_ptr(),
            CString::new(rate.to_string())?.as_ptr(),
        ) < 0
        {
            bail!("rawaudio rate failed");
        }
        if initialize(handle) < 0 {
            bail!("mpv initialize failed");
        }
        let cookie_ptr = (&*player._cookie as *const Shared).cast_mut().cast();
        if register(handle, c"thmrpcm".as_ptr(), cookie_ptr, stream_open) < 0 {
            bail!("stream callback registration failed");
        }
        player.cmd(&["set", "volume", "45"])?;
        player.cmd(&["loadfile", "thmrpcm://live", "replace"])?;
        Ok(player)
    }
    fn cmd(&self, args: &[&str]) -> Result<()> {
        let strings: Vec<CString> = args.iter().map(|s| CString::new(*s).unwrap()).collect();
        let pointers: Vec<*const c_char> = strings
            .iter()
            .map(|s| s.as_ptr())
            .chain(Some(std::ptr::null()))
            .collect();
        let result = unsafe { (self.command)(self.handle, pointers.as_ptr()) };
        if result < 0 {
            bail!("mpv command {} failed ({result})", args[0]);
        }
        Ok(())
    }
    fn number(&self, name: &CStr) -> Option<f64> {
        let mut value = 0.0f64;
        let result = unsafe {
            (self.get)(
                self.handle,
                name.as_ptr(),
                5,
                (&mut value as *mut f64).cast(),
            )
        };
        (result >= 0 && value.is_finite()).then_some(value)
    }
    fn flag(&self, name: &CStr) -> bool {
        let mut value = 0i32;
        unsafe {
            (self.get)(
                self.handle,
                name.as_ptr(),
                3,
                (&mut value as *mut i32).cast(),
            ) >= 0
                && value != 0
        }
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        unsafe {
            stream_cancel((&*self._cookie as *const Shared).cast_mut().cast());
            (self.destroy)(self.handle);
            FreeLibrary(self.library);
        }
    }
}

async fn receive(
    shared: Shared,
    dir: std::path::PathBuf,
    source: String,
    scenario: bool,
    start: Instant,
) -> Result<()> {
    let mut source_log = BufWriter::new(File::create(dir.join("source.f32"))?);
    let mut decode_log = BufWriter::new(File::create(dir.join("decode.csv"))?);
    let mut network_log = BufWriter::new(File::create(dir.join("network.csv"))?);
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .build()?;
    // Exactly one send(), no retries or redirects. PCM never leaves this process.
    let mut request = client
        .get(source)
        .header("Icy-MetaData", "0")
        .header("User-Agent", "THMR-Client/1.0 (+https://thmr.yunmoan.cn)");
    if let Ok(token) = std::env::var("THMR_STREAM_TOKEN") {
        if !token.is_empty() {
            request = request.header("X-Stream-Token", token);
        }
    }
    writeln!(network_log, "{},request,1", wall())?;
    network_log.flush()?;
    let response = request.send().await?.error_for_status()?;
    let mut stream = response.bytes_stream();
    let mut processor = spectrum::AheadProcessor::new()?;
    let mut pending = Vec::new();
    let mut total = 0u64;
    let mut stalled = false;
    loop {
        if shared.0.lock().unwrap().cancelled {
            break;
        }
        if scenario && !stalled && start.elapsed().as_secs_f64() > 37.0 {
            stalled = true;
            writeln!(network_log, "{},injected_ingest_stall_start,6", wall())?;
            network_log.flush()?;
            tokio::time::sleep(Duration::from_secs(6)).await;
            writeln!(network_log, "{},injected_ingest_stall_end,6", wall())?;
            network_log.flush()?;
        }
        let Some(chunk) = stream.next().await else {
            break;
        };
        let chunk = chunk?;
        writeln!(network_log, "{},bytes,{}", wall(), chunk.len())?;
        pending.extend_from_slice(&chunk);
        let mut consumed = 0;
        while pending.len() - consumed >= 8192 {
            loop {
                let full = {
                    let d = shared.0.lock().unwrap();
                    if d.cancelled {
                        return Ok(());
                    }
                    d.rate > 0 && d.decoded.saturating_sub(d.played) >= d.rate as u64 * 3
                };
                if !full {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(4)).await;
            }
            let (advance, pcm, rate, channels, prepared) =
                processor.decode(&pending[consumed..])?;
            if advance == 0 {
                break;
            }
            consumed += advance;
            if pcm.is_empty() {
                continue;
            }
            let computed = wall();
            total += (pcm.len() / channels) as u64;
            for frame in pcm.chunks_exact(channels) {
                source_log.write_all(&frame[0].to_le_bytes())?;
            }
            writeln!(decode_log, "{},{},{}", computed, total, rate)?;
            let mut data = shared.0.lock().unwrap();
            if data.rate != 0 && (data.rate != rate || data.channels != channels) {
                bail!("format changed: restart epoch required");
            }
            data.rate = rate;
            data.channels = channels;
            data.decoded = total;
            if let Some((end, display, levels, music)) = prepared {
                data.features.push_back(Feature {
                    end,
                    computed,
                    display,
                    levels,
                    music,
                });
            }
            for sample in pcm {
                data.pcm.extend(sample.to_le_bytes());
            }
            data.max_pcm_bytes = data.max_pcm_bytes.max(data.pcm.len());
            data.max_features = data.max_features.max(data.features.len());
            data.max_ahead_s = data
                .max_ahead_s
                .max(data.decoded.saturating_sub(data.played) as f64 / rate as f64);
            drop(data);
            shared.1.notify_all();
        }
        pending.drain(..consumed);
    }
    source_log.flush()?;
    decode_log.flush()?;
    network_log.flush()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let dir =
        std::path::PathBuf::from(std::env::var_os("THMR_TRACE_DIR").context("trace directory")?);
    let source = std::env::var("THMR_PROBE_SOURCE")?;
    let seconds = std::env::var("THMR_PROBE_SECONDS")
        .unwrap_or("75".into())
        .parse::<f64>()?;
    let stress = std::env::var_os("THMR_PROBE_STRESS").is_some();
    let shared = Arc::new((Mutex::new(Data::default()), Condvar::new()));
    let receive_shared = shared.clone();
    let receive_dir = dir.clone();
    let start = Instant::now();
    let receiver = tokio::spawn(async move {
        let result = receive(receive_shared.clone(), receive_dir, source, stress, start).await;
        let mut d = receive_shared.0.lock().unwrap();
        d.done = true;
        if let Err(error) = result {
            d.error = Some(error.to_string());
        }
        drop(d);
        receive_shared.1.notify_all();
    });
    let (rate, channels) = loop {
        {
            let data = shared.0.lock().unwrap();
            if let Some(error) = &data.error {
                bail!("upstream: {error}");
            }
            if data.rate > 0 {
                break (data.rate, data.channels);
            }
        }
        if start.elapsed() > Duration::from_secs(15) {
            bail!("no decoded PCM");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let player = unsafe { Player::new(shared.clone(), rate, channels)? };
    let mut terminal = ratatui::try_init()?;
    let mut ui = model::Ui::new(model::Preferences::default());
    let mut state = Snapshot::new("single-stream-experiment".into(), &Default::default());
    state.phase = Phase::Online;
    state.volume = 45.0;
    state.revision = 1;
    ui.sync(&state);
    let mut draw = BufWriter::new(File::create(dir.join("draw.csv"))?);
    let mut metrics = BufWriter::new(File::create(dir.join("metrics.csv"))?);
    let mut current: Option<Feature> = None;
    let mut paused = false;
    let mut paused_frames = 0u64;
    let mut buffering_frames = 0u64;
    let mut analysed_before_playback = 0u64;
    let mut selected_count = 0u64;
    let mut max_clock_skew = 0.0f64;
    let result = (|| -> Result<()> {
        while start.elapsed().as_secs_f64() < seconds {
            let age = start.elapsed().as_secs_f64();
            let pause_now = stress && (20.0..22.0).contains(&age);
            if pause_now != paused {
                player.cmd(&["set", "pause", if pause_now { "yes" } else { "no" }])?;
                paused = pause_now;
            }
            let clock = player.number(c"audio-pts");
            let buffering = player.flag(c"paused-for-cache");
            if paused {
                paused_frames += 1;
            }
            if buffering {
                buffering_frames += 1;
            }
            if let Some(pts) = clock {
                let sample = (pts.max(0.0) * rate as f64) as u64;
                let mut d = shared.0.lock().unwrap();
                d.played = sample;
                if !paused && !buffering {
                    while d.features.front().is_some_and(|f| f.end <= sample) {
                        let feature = d.features.pop_front().unwrap();
                        selected_count += 1;
                        if feature.computed < wall() {
                            analysed_before_playback += 1;
                        }
                        current = Some(feature);
                        d.retired += 1;
                    }
                }
                let feature_end = current.as_ref().map(|f| f.end).unwrap_or(0);
                let computed = current.as_ref().map(|f| f.computed).unwrap_or(0.0);
                max_clock_skew = max_clock_skew.max((pts - feature_end as f64 / rate as f64).abs());
                writeln!(
                    metrics,
                    "{},{},{},{},{},{},{},{},{},{}",
                    wall(),
                    pts,
                    feature_end as f64 / rate as f64,
                    d.decoded as f64 / rate as f64,
                    d.features.len(),
                    d.pcm.len(),
                    paused as u8,
                    buffering as u8,
                    computed,
                    d.retired
                )?;
            }
            state.user_paused = paused;
            state.phase = if paused {
                Phase::Idle
            } else if buffering {
                Phase::Buffering
            } else {
                Phase::Online
            };
            let spectrum = if let Some(f) = &current {
                spectrum::Spectrum {
                    display: f.display,
                    levels: f.levels,
                    music: f.music,
                    updated: Some(Instant::now()),
                    status: "单连接 PCM 时钟验证",
                }
            } else {
                spectrum::Spectrum::default()
            };
            ui.tick(&state, true, &spectrum);
            terminal.draw(|frame| render::draw(frame, &mut ui, &state, None))?;
            if !paused && !buffering {
                if let Some(f) = &current {
                    writeln!(draw, "{},{},{}", wall(), f.end, ui.channel_rms[0])?;
                }
            }
            std::thread::sleep(Duration::from_millis(33));
        }
        Ok(())
    })();
    ratatui::restore();
    shared.0.lock().unwrap().cancelled = true;
    shared.1.notify_all();
    // Let buffered recording files close cleanly before cancellation if the
    // task is blocked reading the remote stream.
    let _ = tokio::time::timeout(Duration::from_secs(3), receiver).await;
    draw.flush()?;
    metrics.flush()?;
    let d = shared.0.lock().unwrap();
    let summary = serde_json::json!({"upstream_requests":1,"rate":rate,"channels":channels,
        "decoded_samples":d.decoded,"played_samples":d.played,"retired_features":d.retired,
        "queued_features_at_exit":d.features.len(),"max_features":d.max_features,
        "max_pcm_queue_bytes":d.max_pcm_bytes,"max_analysis_ahead_s":d.max_ahead_s,
        "selected_features":selected_count,"computed_before_selection":analysed_before_playback,
        "paused_frames":paused_frames,"buffering_frames":buffering_frames,
        "upstream_error":d.error,"stress":stress});
    std::fs::write(
        dir.join("prototype.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    drop(d);
    drop(player);
    result
}
