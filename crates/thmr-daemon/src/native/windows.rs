use anyhow::Result;
use thmr_core::protocol::{Command, Phase, Snapshot};
use tokio::sync::mpsc::UnboundedSender;
use windows::{
    core::{w, HSTRING},
    Foundation::{TypedEventHandler, Uri},
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
        SystemMediaTransportControlsButton as Button,
        SystemMediaTransportControlsButtonPressedEventArgs,
    },
    Storage::Streams::RandomAccessStreamReference,
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW,
            WinRT::{
                ISystemMediaTransportControlsInterop, RoInitialize, RoUninitialize,
                RO_INIT_SINGLETHREADED,
            },
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW,
            RegisterClassW, TranslateMessage, MSG, PM_REMOVE, WINDOW_EX_STYLE, WINDOW_STYLE,
            WNDCLASSW,
        },
    },
};

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wp, lp) }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}
pub struct Media {
    controls: SystemMediaTransportControls,
    token: i64,
    _window: Window,
    _apartment: Apartment,
    metadata: String,
}
impl Media {
    pub fn new(commands: UnboundedSender<Command>) -> Result<Self> {
        unsafe {
            RoInitialize(RO_INIT_SINGLETHREADED)?;
        }
        let apartment = Apartment;
        let instance = unsafe { HINSTANCE(GetModuleHandleW(None)?.0) };
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("THMR.PlaybackHost"),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let window = Window(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("THMR.PlaybackHost"),
                w!("THMR"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )?
        });
        let interop = windows::core::factory::<
            SystemMediaTransportControls,
            ISystemMediaTransportControlsInterop,
        >()?;
        let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(window.0)? };
        controls.SetIsEnabled(true)?;
        controls.SetIsPlayEnabled(true)?;
        controls.SetIsPauseEnabled(true)?;
        controls.SetIsStopEnabled(true)?;
        controls.SetIsNextEnabled(false)?;
        controls.SetIsPreviousEnabled(false)?;
        controls.SetIsFastForwardEnabled(false)?;
        controls.SetIsRewindEnabled(false)?;
        let token = controls.ButtonPressed(&TypedEventHandler::new(
            move |_,
                  args: windows::core::Ref<
                '_,
                SystemMediaTransportControlsButtonPressedEventArgs,
            >| {
                let command = match args.ok()?.Button()? {
                    Button::Play => Command::Play,
                    Button::Pause => Command::Pause,
                    Button::Stop => Command::Stop,
                    _ => return Ok(()),
                };
                let _ = commands.send(command);
                Ok(())
            },
        ))?;
        controls
            .DisplayUpdater()?
            .SetType(MediaPlaybackType::Music)?;
        controls.SetPlaybackStatus(MediaPlaybackStatus::Stopped)?;
        Ok(Self {
            controls,
            token,
            _window: window,
            _apartment: apartment,
            metadata: String::new(),
        })
    }
    pub fn pump(&self) {
        let mut message = MSG::default();
        unsafe {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    pub fn update(&mut self, state: &Snapshot) -> Result<()> {
        self.controls.SetPlaybackStatus(match state.phase {
            Phase::Online | Phase::Buffering => MediaPlaybackStatus::Playing,
            _ if state.user_paused => MediaPlaybackStatus::Paused,
            _ => MediaPlaybackStatus::Stopped,
        })?;
        let current = state.current();
        let signature = current.map(|c| c.to_string()).unwrap_or_default();
        // Position changes do not require fetching/replacing cover artwork every second.
        let title = current
            .and_then(|c| c.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("THMR 东方音乐电台");
        let artist = current
            .and_then(|c| c.get("artist"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let cover = current
            .and_then(|c| c.get("coverUrl"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let key = format!("{title}|{artist}|{cover}");
        if key != self.metadata {
            let updater = self.controls.DisplayUpdater()?;
            updater.MusicProperties()?.SetTitle(&HSTRING::from(title))?;
            updater
                .MusicProperties()?
                .SetArtist(&HSTRING::from(artist))?;
            updater
                .MusicProperties()?
                .SetAlbumTitle(&HSTRING::from("东方音乐电台 THMR"))?;
            if !cover.is_empty() {
                let url = if cover.starts_with("http://") || cover.starts_with("https://") {
                    cover.to_owned()
                } else {
                    format!("{}{cover}", state.radio_base_url)
                };
                if let Ok(uri) = Uri::CreateUri(&HSTRING::from(url)) {
                    if let Ok(reference) = RandomAccessStreamReference::CreateFromUri(&uri) {
                        let _ = updater.SetThumbnail(&reference);
                    }
                }
            }
            updater.Update()?;
            self.metadata = key;
        }
        let _ = signature;
        Ok(())
    }
}
impl Drop for Media {
    fn drop(&mut self) {
        let _ = self.controls.SetIsEnabled(false);
        let _ = self.controls.RemoveButtonPressed(self.token);
    }
}
