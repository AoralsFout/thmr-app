//! Private libmpv ABI. The library outlives its handle; all access stays on the actor thread.
use anyhow::{bail, Context, Result};
use libloading::Library;
use std::{
    ffi::{c_char, c_double, c_int, c_ulong, c_void, CStr, CString},
    path::PathBuf,
    ptr,
};

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
                dir.join("../Frameworks/libmpv.framework/libmpv"),
                dir.join("../Frameworks/libmpv.2.dylib"),
                dir.join("libmpv.2.dylib"),
                "/opt/homebrew/lib/libmpv.2.dylib".into(),
                "/usr/local/lib/libmpv.2.dylib".into(),
            ]
        } else {
            vec![
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
            if let Ok(token) = std::env::var("THMR_STREAM_TOKEN") {
                if !token.is_empty() {
                    if token.contains(['\r', '\n', ',']) {
                        bail!("invalid stream authentication header");
                    }
                    if option(
                        handle,
                        c"http-header-fields".as_ptr(),
                        CString::new(format!("X-Stream-Token: {token}"))?.as_ptr(),
                    ) < 0
                    {
                        bail!("cannot set stream authentication");
                    }
                }
            }
            let code = initialize(handle);
            if code < 0 {
                bail!("libmpv initialization failed ({code})");
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
    pub fn play(&mut self, url: &str) -> Result<()> {
        self.wanted = false;
        self.command(&["stop"])?;
        self.drain();
        self.active_entry = None;
        self.loaded = false;
        self.command(&["loadfile", url, "replace"])?;
        self.wanted = true;
        Ok(())
    }
    pub fn stop(&mut self) -> Result<()> {
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
        unsafe { (self.destroy)(self.handle) };
    }
}
