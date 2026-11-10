//! Private libmpv ABI. The library outlives its handle; all access stays on the actor thread.
use crate::audio::{Format, PcmStream};
use anyhow::{bail, Context, Result};
use libloading::Library;
use std::{
    ffi::{c_char, c_double, c_int, c_ulong, c_void, CStr, CString},
    path::PathBuf,
    ptr,
    sync::Mutex,
};

#[repr(C)]
struct StreamInfo {
    cookie: *mut c_void,
    read: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, u64) -> i64>,
    seek: Option<unsafe extern "C" fn(*mut c_void, i64) -> i64>,
    size: Option<unsafe extern "C" fn(*mut c_void) -> i64>,
    close: Option<unsafe extern "C" fn(*mut c_void)>,
    cancel: Option<unsafe extern "C" fn(*mut c_void)>,
}
type Streams = Mutex<Option<(u64, PcmStream)>>;
unsafe extern "C" fn stream_read(cookie: *mut c_void, out: *mut c_char, size: u64) -> i64 {
    if size == 0 {
        return 0;
    }
    if out.is_null() {
        return -1;
    }
    let stream = &*(cookie as *const PcmStream);
    // libmpv buffers are small; cap each callback and avoid constructing an oversized slice.
    stream.read(std::slice::from_raw_parts_mut(
        out.cast(),
        size.min(65536) as usize,
    ))
}
unsafe extern "C" fn stream_close(cookie: *mut c_void) {
    drop(Box::from_raw(cookie as *mut PcmStream));
}
unsafe extern "C" fn stream_cancel(cookie: *mut c_void) {
    (&*(cookie as *const PcmStream)).cancel();
}
unsafe extern "C" fn stream_open(
    user: *mut c_void,
    uri: *mut c_char,
    info: *mut StreamInfo,
) -> c_int {
    let registry = &*(user as *const Streams);
    let registry = registry.lock().unwrap();
    let Some((epoch, stream)) = &*registry else {
        return -13;
    };
    if CStr::from_ptr(uri).to_bytes() != format!("thmrpcm://{epoch}").as_bytes() {
        return -13;
    }
    *info = StreamInfo {
        cookie: Box::into_raw(Box::new(stream.clone())).cast(),
        read: Some(stream_read),
        seek: None,
        size: None,
        close: Some(stream_close),
        cancel: Some(stream_cancel),
    };
    0
}

type Handle = c_void;
#[repr(C)]
struct Event {
    id: c_int,
    error: c_int,
    userdata: u64,
    data: *mut c_void,
}
#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
    entry: i64,
}
#[repr(C)]
struct StartFile {
    entry: i64,
}

pub enum PlaybackEvent {
    Loaded,
    Playing,
    Buffering,
    Ended,
    Error(String),
}

pub struct Mpv {
    handle: *mut Handle,
    command_fn: unsafe extern "C" fn(*mut Handle, *const *const c_char) -> c_int,
    get_string: unsafe extern "C" fn(*mut Handle, *const c_char) -> *mut c_char,
    free: unsafe extern "C" fn(*mut c_void),
    wait: unsafe extern "C" fn(*mut Handle, c_double) -> *mut Event,
    destroy: unsafe extern "C" fn(*mut Handle),
    option: unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int,
    get_native: unsafe extern "C" fn(*mut Handle, *const c_char, c_int, *mut c_void) -> c_int,
    streams: Box<Streams>,
    _library: Library,
    active_entry: Option<i64>,
    wanted: bool,
    loaded: bool,
}

impl Mpv {
    pub fn candidates() -> Vec<PathBuf> {
        if let Some(path) = std::env::var_os("THMR_MPV_LIBRARY") {
            return vec![path.into()];
        }
        let exe = std::env::current_exe().unwrap_or_default();
        let dir = exe.parent().unwrap_or_else(|| std::path::Path::new("."));
        if cfg!(windows) {
            vec![dir.join("libmpv-2.dll"), dir.join("mpv-2.dll")]
        } else if cfg!(target_os = "macos") {
            vec![
                dir.join("../Frameworks/Mpv.framework/Mpv"),
                dir.join("../Frameworks/libmpv.framework/libmpv"),
                dir.join("../Frameworks/libmpv.2.dylib"),
                dir.join("libmpv.2.dylib"),
                "/opt/homebrew/lib/libmpv.2.dylib".into(),
                "/usr/local/lib/libmpv.2.dylib".into(),
            ]
        } else {
            vec![
                dir.join("../lib/libmpv.so.2"),
                dir.join("lib/libmpv.so.2"),
                "libmpv.so.2".into(),
                "libmpv.so.1".into(),
            ]
        }
    }

    pub fn new(volume: f64) -> Result<Self> {
        let mut found = None;
        for path in Self::candidates() {
            // Only installation-owned paths (plus documented system library locations).
            if let Ok(library) = unsafe { Library::new(&path) } {
                found = Some(library);
                break;
            }
        }
        let library = found.context(
            "libmpv not found; install the complete THMR package or set THMR_MPV_LIBRARY",
        )?;
        unsafe {
            let version =
                library.get::<unsafe extern "C" fn() -> c_ulong>(b"mpv_client_api_version\0")?();
            if version >> 16 != 2 {
                bail!("unsupported libmpv client ABI major {}", version >> 16);
            }
            let create = *library.get::<unsafe extern "C" fn() -> *mut Handle>(b"mpv_create\0")?;
            let initialize =
                *library.get::<unsafe extern "C" fn(*mut Handle) -> c_int>(b"mpv_initialize\0")?;
            let option = *library.get::<unsafe extern "C" fn(
                *mut Handle,
                *const c_char,
                *const c_char,
            ) -> c_int>(b"mpv_set_option_string\0")?;
            // Resolve symbols before creating a handle, so failed resolution cannot leak it.
            let command_fn = *library.get(b"mpv_command\0")?;
            let get_string = *library.get(b"mpv_get_property_string\0")?;
            let free = *library.get(b"mpv_free\0")?;
            let wait = *library.get(b"mpv_wait_event\0")?;
            let destroy = *library.get(b"mpv_terminate_destroy\0")?;
            let get_native = *library.get(b"mpv_get_property\0")?;
            let register = *library.get::<unsafe extern "C" fn(
                *mut Handle,
                *const c_char,
                *mut c_void,
                unsafe extern "C" fn(*mut c_void, *mut c_char, *mut StreamInfo) -> c_int,
            ) -> c_int>(b"mpv_stream_cb_add_ro\0")?;
            let handle = create();
            if handle.is_null() {
                bail!("could not create libmpv handle");
            }
            let player = Self {
                handle,
                command_fn,
                get_string,
                free,
                wait,
                destroy,
                option,
                get_native,
                streams: Box::new(Mutex::new(None)),
                _library: library,
                active_entry: None,
                wanted: false,
                loaded: false,
            };
            for (name, value) in [
                ("config", "no"),
                ("terminal", "no"),
                ("vo", "null"),
                ("vid", "no"),
                ("idle", "yes"),
                ("input-default-bindings", "no"),
                ("input-vo-keyboard", "no"),
                ("cache", "yes"),
                ("demuxer-max-bytes", "2097152"),
                ("cache-secs", "1"),
                ("cache-pause-wait", "0.5"),
                ("demuxer", "rawaudio"),
                ("demuxer-rawaudio-format", "floatle"),
                ("network-timeout", "10"),
                ("user-agent", "THMR-Client/1.0 (+https://thmr.yunmoan.cn)"),
            ] {
                let code = option(
                    handle,
                    CString::new(name)?.as_ptr(),
                    CString::new(value)?.as_ptr(),
                );
                if code < 0 {
                    bail!("libmpv option {name} failed ({code})");
                }
            }
            if let Ok(output) = std::env::var("THMR_AUDIO_OUTPUT") {
                if option(handle, c"ao".as_ptr(), CString::new(output)?.as_ptr()) < 0 {
                    bail!("invalid THMR_AUDIO_OUTPUT");
                }
            }
            let code = initialize(handle);
            if code < 0 {
                bail!("libmpv initialization failed ({code})");
            }
            let registry = (&*player.streams as *const Streams).cast_mut().cast();
            if register(handle, c"thmrpcm".as_ptr(), registry, stream_open) < 0 {
                bail!("libmpv cannot register the shared PCM input");
            }
            player.command(&["set", "volume", &volume.to_string()])?;
            Ok(player)
        }
    }

    fn command(&self, args: &[&str]) -> Result<()> {
        let strings: Vec<CString> = args
            .iter()
            .map(|s| CString::new(*s))
            .collect::<std::result::Result<_, _>>()?;
        let pointers: Vec<*const c_char> = strings
            .iter()
            .map(|s| s.as_ptr())
            .chain(Some(ptr::null()))
            .collect();
        let code = unsafe { (self.command_fn)(self.handle, pointers.as_ptr()) };
        if code < 0 {
            bail!("libmpv command {} failed ({code})", args[0]);
        }
        Ok(())
    }
    pub fn play(&mut self, epoch: u64, stream: PcmStream, format: Format) -> Result<()> {
        self.stop()?;
        self.wanted = false;
        self.active_entry = None;
        self.loaded = false;
        for (name, value) in [
            ("demuxer-rawaudio-rate", format.rate.to_string()),
            (
                "demuxer-rawaudio-channels",
                if format.channels == 1 {
                    "mono"
                } else {
                    "stereo"
                }
                .into(),
            ),
        ] {
            let code = unsafe {
                (self.option)(
                    self.handle,
                    CString::new(name)?.as_ptr(),
                    CString::new(value)?.as_ptr(),
                )
            };
            if code < 0 {
                bail!("libmpv PCM option {name} failed ({code})");
            }
        }
        *self.streams.lock().unwrap() = Some((epoch, stream));
        self.command(&["loadfile", &format!("thmrpcm://{epoch}"), "replace"])?;
        self.wanted = true;
        Ok(())
    }
    pub fn stop(&mut self) -> Result<()> {
        if let Some((_, stream)) = self.streams.lock().unwrap().take() {
            stream.cancel();
        }
        self.wanted = false;
        self.loaded = false;
        self.active_entry = None;
        self.command(&["stop"])?;
        self.drain();
        Ok(())
    }
    pub fn volume(&self, value: f64) -> Result<()> {
        self.command(&["set", "volume", &value.to_string()])
    }
    pub fn audio_clock(&self) -> (Option<f64>, bool) {
        let mut position = 0.0f64;
        let code = unsafe {
            (self.get_native)(
                self.handle,
                c"audio-pts".as_ptr(),
                5,
                (&mut position as *mut f64).cast(),
            )
        };
        let position = (code >= 0 && position.is_finite()).then_some(position);
        let active = self.wanted
            && self.loaded
            && self.property(c"paused-for-cache").as_deref() != Some("yes")
            && self.property(c"core-idle").as_deref() == Some("no");
        (position, active)
    }
    fn property(&self, name: &CStr) -> Option<String> {
        let p = unsafe { (self.get_string)(self.handle, name.as_ptr()) };
        if p.is_null() {
            return None;
        }
        let value = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        unsafe { (self.free)(p.cast()) };
        Some(value)
    }
    fn drain(&mut self) {
        for _ in 0..1024 {
            if unsafe { (*(self.wait)(self.handle, 0.0)).id } == 0 {
                break;
            }
        }
    }
    pub fn events(&mut self) -> Vec<PlaybackEvent> {
        let mut events = Vec::new();
        for _ in 0..1024 {
            let event = unsafe { &*(self.wait)(self.handle, 0.0) };
            if event.id == 0 {
                break;
            }
            if !self.wanted {
                continue;
            }
            match event.id {
                6 if !event.data.is_null() => {
                    self.active_entry = Some(unsafe { (*(event.data.cast::<StartFile>())).entry });
                }
                8 => {
                    self.loaded = true;
                    events.push(PlaybackEvent::Loaded);
                }
                21 if self.loaded => events.push(PlaybackEvent::Playing),
                7 if !event.data.is_null() => {
                    let end = unsafe { &*event.data.cast::<EndFile>() };
                    if Some(end.entry) != self.active_entry {
                        continue;
                    }
                    if end.reason == 4 {
                        self.loaded = false;
                        self.wanted = false;
                        events.push(PlaybackEvent::Error(format!(
                            "audio stream failed ({})",
                            end.error
                        )));
                    } else if end.reason == 0 {
                        self.loaded = false;
                        self.wanted = false;
                        events.push(PlaybackEvent::Ended);
                    }
                }
                _ => {}
            }
        }
        if self.wanted && self.loaded {
            if self.property(c"paused-for-cache").as_deref() == Some("yes") {
                events.push(PlaybackEvent::Buffering);
            } else if self.property(c"core-idle").as_deref() == Some("no") {
                events.push(PlaybackEvent::Playing);
            }
        }
        events
    }
}
impl Drop for Mpv {
    fn drop(&mut self) {
        if let Some((_, stream)) = self.streams.lock().unwrap().take() {
            stream.cancel();
        }
        unsafe { (self.destroy)(self.handle) };
    }
}
