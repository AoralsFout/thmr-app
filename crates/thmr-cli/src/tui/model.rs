use super::{
    danmaku::Stage,
    spectrum::{self, Spectrum, BANDS},
    spectrum_view,
};
use ratatui::layout::{Position, Rect};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thmr_core::protocol::{Command, Phase, Snapshot};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    Static,
    #[default]
    #[serde(alias = "standard", alias = "fancy")]
    Dynamic,
}
impl Motion {
    pub fn label(self) -> &'static str {
        match self {
            Self::Static => "静态",
            Self::Dynamic => "动态",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Static => Self::Dynamic,
            Self::Dynamic => Self::Static,
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub motion: Motion,
    pub ascii: bool,
}
impl Preferences {
    fn path() -> anyhow::Result<PathBuf> {
        Ok(thmr_core::runtime::config_dir()?.join("tui.json"))
    }
    pub fn load() -> Self {
        let mut value: Self = Self::path()
            .ok()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        if std::env::var_os("THMR_TUI_ASCII").is_some_and(|v| v == "1") {
            value.ascii = true;
        }
        if std::env::var_os("THMR_TUI_REDUCED_MOTION").is_some_and(|v| v == "1") {
            value.motion = Motion::Static;
        }
        value
    }
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Queue,
    Play,
    Stop,
    Down,
    Up,
    Source,
    SourceNamed(&'static str),
    Quality,
    Refresh,
    Mode(Motion),
    Details,
    CurrentDetails,
    Help,
    Close,
    Quit,
    Select(usize),
}
impl Action {
    pub fn hint(self) -> &'static str {
        match self {
            Self::Queue | Self::Select(_) => "↑↓ / j k 浏览 · Enter 查看详情 · 排播只读",
            Self::Play => "Space 播放 / 暂停",
            Self::Stop => "s 停止播放",
            Self::Down => "- 音量降低 2%",
            Self::Up => "+ 音量提高 2%",
            Self::Source | Self::SourceNamed(_) => "a 自动 · 1 / 2 手动音源",
            Self::Quality => "b 切换可用音质",
            Self::Refresh => "r 刷新排播",
            Self::Mode(_) => "m 循环动效档位 · 自动保存",
            Self::Details => "Enter 查看选中曲目详情",
            Self::CurrentDetails => "i 查看正在放送的曲目详情",
            Self::Help => "? 查看帮助",
            Self::Close => "Esc 关闭",
            Self::Quit => "q 退出界面，后台播放继续",
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Help,
    Details,
    CurrentDetails,
}
#[derive(Clone)]
pub struct Track {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub original: String,
}
pub fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')
        })
        .collect()
}
impl Track {
    pub fn from(value: &Value) -> Self {
        let title = text(value.get("title"));
        let artist = text(value.get("artist"));
        Self {
            key: format!(
                "{}|{title}|{artist}",
                value
                    .get("trackId")
                    .or_else(|| value.get("id"))
                    .unwrap_or(&Value::Null)
            ),
            title,
            artist,
            album: text(value.get("album")),
            original: text(value.get("originalTitle").or(value.get("original"))),
        }
    }
}

pub struct Ui {
    pub prefs: Preferences,
    pub tracks: Vec<Track>,
    pub selected: usize,
    pub offset: usize,
    pub queue_rect: Rect,
    pub hits: Vec<(Rect, Action)>,
    pub focusables: Vec<Action>,
    pub focus: Action,
    pub hover: Option<Action>,
    pub pointer: Option<Position>,
    pub overlay: Option<Overlay>,
    pub overlay_scroll: u16,
    pub overlay_max_scroll: u16,
    pub notice: Option<String>,
    pub pending_volume: Option<(u64, f64)>,
    pub pending: usize,
    pub bands: [f32; BANDS],
    pub peaks: [f32; BANDS],
    peak_hold: [f32; BANDS],
    pub spectrum_max_hz: f32,
    pub channel_rms: [f32; 2],
    pub channel_peaks: [f32; 2],
    channel_hold: [f32; 2],
    pub spectrum_status: &'static str,
    pub available: bool,
    pub clock: f64,
    pub stage: Stage,
    pub poster: Stage,
    session_seed: u64,
    pub title_changed: Instant,
    current_key: String,
    pub pulse_until: Option<Instant>,
    last_tick: Instant,
    instance: String,
    revision: u64,
    audio_epoch: u64,
}
impl Ui {
    pub fn new(prefs: Preferences) -> Self {
        let session_seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
            ^ (std::process::id() as u64).rotate_left(32);
        Self {
            prefs,
            tracks: vec![],
            selected: 0,
            offset: 0,
            queue_rect: Rect::default(),
            hits: vec![],
            focusables: vec![],
            focus: Action::Queue,
            hover: None,
            pointer: None,
            overlay: None,
            overlay_scroll: 0,
            overlay_max_scroll: 0,
            notice: None,
            pending_volume: None,
            pending: 0,
            bands: [0.0; BANDS],
            peaks: [0.0; BANDS],
            peak_hold: [0.0; BANDS],
            spectrum_max_hz: 16000.0,
            channel_rms: [0.0; 2],
            channel_peaks: [0.0; 2],
            channel_hold: [0.0; 2],
            spectrum_status: "等待播放",
            available: false,
            clock: 0.0,
            stage: Stage::for_track(session_seed, ""),
            poster: Stage::poster(),
            session_seed,
            title_changed: Instant::now(),
            current_key: String::new(),
            pulse_until: None,
            last_tick: Instant::now(),
            instance: String::new(),
            revision: u64::MAX,
            audio_epoch: 0,
        }
    }
    pub fn sync(&mut self, state: &Snapshot) {
        if self.revision == state.revision && self.instance == state.instance_id {
            return;
        }
        if self.instance != state.instance_id {
            self.pending_volume = None;
        }
        self.instance = state.instance_id.clone();
        self.revision = state.revision;
        let old_key = self.tracks.get(self.selected).map(|t| t.key.clone());
        self.tracks = state
            .station_status
            .as_ref()
            .and_then(|s| s.get("queue"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(Track::from)
            .collect();
        self.selected = old_key
            .and_then(|key| self.tracks.iter().position(|t| t.key == key))
            .unwrap_or(self.selected)
            .min(self.tracks.len().saturating_sub(1));
        let key = state
            .current()
            .map(Track::from)
            .map(|t| t.key)
            .unwrap_or_default();
        if key != self.current_key {
            self.title_changed = Instant::now();
            self.current_key = key;
            self.clock = 0.0;
            let aspect = self.stage.aspect;
            self.stage = Stage::for_track(self.session_seed, &self.current_key);
            self.stage.aspect = aspect;
        }
        self.ensure_visible();
    }
    pub fn ensure_visible(&mut self) {
        let visible = self.queue_rect.height.max(1) as usize;
        if self.selected < self.offset {
            self.offset = self.selected;
        }
        if self.selected >= self.offset + visible {
            self.offset = self.selected + 1 - visible;
        }
        self.offset = self.offset.min(self.tracks.len().saturating_sub(visible));
    }
    pub fn select(&mut self, delta: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.tracks.len().saturating_sub(1));
        self.focus = Action::Queue;
        self.ensure_visible();
    }
    pub fn tab(&mut self, backwards: bool) {
        if self.focusables.is_empty() {
            return;
        }
        let current = self
            .focusables
            .iter()
            .position(|a| *a == self.focus)
            .unwrap_or(0);
        let length = self.focusables.len();
        self.focus = self.focusables[(current + if backwards { length - 1 } else { 1 }) % length];
    }
    pub fn hit(&self, position: Position) -> Option<Action> {
        self.hits
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(position))
            .map(|(_, action)| *action)
    }
    pub fn modal(&mut self, overlay: Overlay) {
        self.overlay = Some(overlay);
        self.focus = Action::Close;
        self.overlay_scroll = 0;
        self.overlay_max_scroll = 0;
    }
    pub fn scroll_overlay(&mut self, delta: i16) {
        self.overlay_scroll = self
            .overlay_scroll
            .saturating_add_signed(delta)
            .min(self.overlay_max_scroll);
    }
    pub fn change_mode(&mut self, mode: Motion) {
        self.prefs.motion = mode;
        if mode == Motion::Static {
            self.bands.fill(0.0);
            self.peaks.fill(0.0);
            self.channel_rms.fill(0.0);
            self.channel_peaks.fill(0.0);
            self.channel_hold.fill(0.0);
        }
        if let Err(error) = self.prefs.save() {
            self.notice = Some(format!("无法保存界面偏好：{error}"));
        }
    }
    pub fn volume(&self, state: &Snapshot) -> f64 {
        self.pending_volume.map(|(_, v)| v).unwrap_or(state.volume)
    }
    pub fn active(&self, state: &Snapshot, connected: bool) -> bool {
        connected && state.phase == Phase::Online && !state.user_paused
    }
    pub fn spectrum_source(&self, state: &Snapshot, connected: bool) -> Option<String> {
        if self.prefs.motion != Motion::Static
            && self.active(state, connected)
            && state.capabilities.synced_audio
        {
            spectrum::source(state)
        } else {
            None
        }
    }
    pub fn tick(&mut self, state: &Snapshot, connected: bool, spectrum: &Spectrum) {
        let was_available = self.available;
        if self.audio_epoch != spectrum.epoch {
            self.audio_epoch = spectrum.epoch;
            self.stage.suspend(&spectrum.music);
            self.bands.fill(0.0);
            self.peaks.fill(0.0);
            self.channel_rms.fill(0.0);
            self.channel_peaks.fill(0.0);
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.1);
        self.last_tick = now;
        self.available = spectrum
            .updated
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1));
        self.spectrum_status = if self.prefs.motion == Motion::Static {
            "静态模式 · 频谱已关闭"
        } else if !state.capabilities.synced_audio {
            "请重启或升级播放宿主以启用同步频谱"
        } else if !connected {
            "主机连接中断 · 频谱已收束"
        } else if !self.active(state, connected) {
            "等待播放"
        } else if spectrum.updated.is_some() && !self.available {
            "频谱数据已过期，等待新音频"
        } else {
            spectrum.status
        };
        let active =
            self.prefs.motion != Motion::Static && self.active(state, connected) && self.available;
        if active && !was_available {
            self.stage.suspend(&spectrum.music);
        }
        let muted = state.volume <= 0.0;
        if active && (self.spectrum_max_hz - spectrum.display.max_hz).abs() > 0.01 {
            self.bands =
                spectrum_view::remap(&self.bands, self.spectrum_max_hz, spectrum.display.max_hz);
            self.peaks =
                spectrum_view::remap(&self.peaks, self.spectrum_max_hz, spectrum.display.max_hz);
            self.peak_hold.fill(0.0);
            self.spectrum_max_hz = spectrum.display.max_hz;
        }
        for channel in 0..2 {
            let target = if active && !muted {
                spectrum.levels.rms[channel]
            } else {
                0.0
            };
            let peak = if active && !muted {
                spectrum.levels.peak[channel]
            } else {
                0.0
            };
            let tau = if target > self.channel_rms[channel] {
                0.035
            } else {
                0.24
            };
            self.channel_rms[channel] +=
                (target - self.channel_rms[channel]) * (1.0 - (-dt / tau).exp());
            if self.channel_rms[channel] < 0.00001 {
                self.channel_rms[channel] = 0.0;
            }
            if peak > 0.0 && peak >= self.channel_peaks[channel] {
                self.channel_peaks[channel] = peak;
                self.channel_hold[channel] = 0.65;
            } else {
                if !active || muted {
                    self.channel_hold[channel] = 0.0;
                }
                self.channel_hold[channel] = (self.channel_hold[channel] - dt).max(0.0);
                if self.channel_hold[channel] == 0.0 {
                    self.channel_peaks[channel] =
                        (self.channel_peaks[channel] * 10.0f32.powf(-24.0 * dt / 20.0)).max(peak);
                }
            }
            if self.channel_peaks[channel] < 0.00001 {
                self.channel_peaks[channel] = 0.0;
            }
        }
        for i in 0..BANDS {
            let target = if active {
                spectrum.display.bands[i] * if muted { 0.18 } else { 1.0 }
            } else {
                0.0
            };
            let tau = if target > self.bands[i] { 0.045 } else { 0.22 };
            if active {
                self.bands[i] += (target - self.bands[i]) * (1.0 - (-dt / tau).exp());
            } else {
                self.bands[i] = (self.bands[i] - dt * 2.5).max(0.0);
                self.peaks[i] = (self.peaks[i] - dt * 2.5).max(self.bands[i]);
                self.peak_hold[i] = 0.0;
            }
            if self.bands[i] < 0.001 {
                self.bands[i] = 0.0;
            }
            if self.bands[i] >= self.peaks[i] {
                self.peaks[i] = self.bands[i];
                self.peak_hold[i] = 0.18;
            } else {
                self.peak_hold[i] = (self.peak_hold[i] - dt).max(0.0);
                if self.peak_hold[i] <= 0.0 {
                    self.peaks[i] = (self.peaks[i] - dt * 0.65).max(self.bands[i]);
                }
            }
        }
        if active && !muted && self.overlay.is_none() {
            self.clock += dt as f64;
            self.stage.advance(dt, &spectrum.music);
        } else {
            self.stage.suspend(&spectrum.music);
        }
    }
    pub fn animate(&self, state: &Snapshot, connected: bool) -> bool {
        self.prefs.motion != Motion::Static
            && ((self.active(state, connected)
                && self.available
                && self.overlay.is_none()
                && state.volume > 0.0)
                || self.peaks.iter().any(|v| *v > 0.001)
                || self.channel_peaks.iter().any(|v| *v > 0.00001))
    }
    pub fn command(&self, action: Action, state: &Snapshot) -> Option<Command> {
        match action {
            Action::SourceNamed(source) => Some(Command::SetSource {
                source: source.into(),
            }),
            Action::Play => Some(Command::Toggle),
            Action::Stop => Some(Command::Stop),
            Action::Down => Some(Command::SetVolume {
                value: (self.volume(state) - 2.0).max(0.0),
            }),
            Action::Up => Some(Command::SetVolume {
                value: (self.volume(state) + 2.0).min(100.0),
            }),
            Action::Refresh => Some(Command::Refresh),
            Action::Source => Some(Command::SetSource {
                source: match state.source.as_str() {
                    "auto" => "source1",
                    "source1" => "source2",
                    _ => "auto",
                }
                .into(),
            }),
            Action::Quality => {
                next_profile(state).map(|profile_id| Command::SetProfile { profile_id })
            }
            _ => None,
        }
    }
}
pub fn next_profile(state: &Snapshot) -> Option<String> {
    let profiles: Vec<&str> = state
        .station_status
        .as_ref()?
        .pointer("/station/bitrates")?
        .as_array()?
        .iter()
        .filter(|p| p.get("enabled").and_then(Value::as_bool) != Some(false))
        .filter_map(|p| p.get("id")?.as_str())
        .collect();
    let index = profiles
        .iter()
        .position(|p| *p == state.profile_id)
        .unwrap_or(0);
    if profiles.is_empty() {
        None
    } else {
        Some(profiles[(index + 1) % profiles.len()].into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stereo_meters_follow_channels_and_fall_on_pause_mute_and_stale_audio() {
        for reason in 0..3 {
            let mut state = Snapshot::new("test".into(), &Default::default());
            state.phase = Phase::Online;
            state.volume = 70.0;
            let mut ui = Ui::new(Preferences::default());
            let mut audio = Spectrum {
                levels: spectrum::ChannelLevels {
                    rms: [0.5, 0.125],
                    peak: [0.8, 0.3],
                },
                updated: Some(Instant::now()),
                ..Default::default()
            };
            for _ in 0..10 {
                ui.last_tick = Instant::now() - Duration::from_millis(100);
                ui.tick(&state, true, &audio);
            }
            assert!(ui.channel_rms[0] > ui.channel_rms[1] * 3.9);
            assert_eq!(ui.channel_peaks, [0.8, 0.3]);
            match reason {
                0 => state.user_paused = true,
                1 => state.volume = 0.0,
                _ => audio.updated = Some(Instant::now() - Duration::from_secs(2)),
            }
            for _ in 0..50 {
                ui.last_tick = Instant::now() - Duration::from_millis(100);
                ui.tick(&state, true, &audio);
            }
            assert_eq!(ui.channel_rms, [0.0; 2]);
            assert_eq!(ui.channel_peaks, [0.0; 2]);
        }
    }

    #[test]
    fn old_motion_preferences_migrate_without_losing_ascii_setting() {
        for name in ["standard", "fancy", "dynamic"] {
            let prefs: Preferences =
                serde_json::from_value(serde_json::json!({"motion": name, "ascii": true})).unwrap();
            assert_eq!(prefs.motion, Motion::Dynamic);
            assert!(prefs.ascii);
            assert_eq!(serde_json::to_value(&prefs).unwrap()["motion"], "dynamic");
        }
        assert_eq!(Motion::default(), Motion::Dynamic);
        assert_eq!(Motion::Dynamic.next(), Motion::Static);
        assert_eq!(Motion::Static.next(), Motion::Dynamic);
    }

    #[test]
    fn pause_mute_overlay_and_disconnection_freeze_the_stage() {
        let mut state = Snapshot::new("test".into(), &Default::default());
        state.phase = Phase::Online;
        state.volume = 50.0;
        let mut ui = Ui::new(Preferences::default());
        let mut audio = Spectrum {
            display: Default::default(),
            levels: Default::default(),
            music: spectrum::MusicalFeatures {
                level: 0.6,
                intensity: 0.8,
                ..Default::default()
            },
            updated: Some(Instant::now()),
            status: "ready",
            ..Default::default()
        };
        ui.last_tick = Instant::now() - Duration::from_millis(50);
        ui.tick(&state, true, &audio);
        assert!(ui.stage.time > 0.0);
        let frozen = ui.stage.time;
        for reason in 0..5 {
            state.user_paused = reason == 0;
            state.volume = if reason == 1 { 0.0 } else { 50.0 };
            ui.overlay = if reason == 2 {
                Some(Overlay::Help)
            } else {
                None
            };
            audio.updated = Some(
                Instant::now()
                    - if reason == 4 {
                        Duration::from_secs(2)
                    } else {
                        Duration::ZERO
                    },
            );
            audio.music.beats += 1;
            ui.last_tick = Instant::now() - Duration::from_millis(50);
            ui.tick(&state, reason != 3, &audio);
            assert_eq!(ui.stage.time, frozen, "freeze reason {reason}");
        }
    }

    #[test]
    fn selection_survives_queue_update_and_scroll_stays_bounded() {
        let mut ui = Ui::new(Preferences::default());
        let mut state = Snapshot::new("test".into(), &Default::default());
        state.station_status =
            Some(serde_json::json!({"queue":[{"title":"一"},{"title":"二"},{"title":"三"}]}));
        ui.queue_rect = Rect::new(0, 0, 20, 2);
        ui.sync(&state);
        ui.select(2);
        assert_eq!(ui.offset, 1);
        state.revision += 1;
        state.station_status = Some(serde_json::json!({"queue":[{"title":"二"},{"title":"三"}]}));
        ui.sync(&state);
        assert_eq!(ui.selected, 1);
        assert_eq!(ui.offset, 0);
        ui.select(100);
        assert_eq!(ui.selected, 1);
    }
    #[test]
    fn pending_volume_accumulates_without_waiting_for_host() {
        let state = Snapshot::new("test".into(), &Default::default());
        let mut ui = Ui::new(Preferences::default());
        ui.pending_volume = Some((3, 98.0));
        assert_eq!(
            ui.command(Action::Up, &state),
            Some(Command::SetVolume { value: 100.0 })
        );
        ui.pending_volume = Some((4, 0.0));
        assert_eq!(
            ui.command(Action::Down, &state),
            Some(Command::SetVolume { value: 0.0 })
        );
    }
    #[test]
    fn stale_audio_decays_and_static_mode_never_requests_a_stream() {
        let mut state = Snapshot::new("test".into(), &Default::default());
        state.phase = Phase::Online;
        let mut ui = Ui::new(Preferences::default());
        ui.bands.fill(1.0);
        let spectrum = Spectrum {
            display: Default::default(),
            levels: Default::default(),
            music: Default::default(),
            updated: Some(Instant::now() - Duration::from_secs(2)),
            status: "ready",
            ..Default::default()
        };
        ui.last_tick = Instant::now() - Duration::from_millis(100);
        ui.tick(&state, true, &spectrum);
        assert!(ui.bands[0] < 1.0);
        assert!(!ui.available);
        ui.prefs.motion = Motion::Static;
        assert!(ui.spectrum_source(&state, true).is_none());
    }
}
