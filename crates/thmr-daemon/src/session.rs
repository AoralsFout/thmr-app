use crate::{
    mpv::{Mpv, PlaybackEvent},
    server::Envelope,
};
use anyhow::{bail, Result};
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thmr_core::{
    protocol::{Command, Phase, Snapshot},
    settings::Settings,
};
use tokio::sync::{mpsc, watch};

pub struct Session {
    pub state: Snapshot,
    settings: Settings,
    path: PathBuf,
    player: Mpv,
    wanted: bool,
    retry: Option<Instant>,
    failures: u32,
    connecting_since: Option<Instant>,
    online_since: Option<Instant>,
    elapsed: Instant,
    last_open: Option<Instant>,
}

impl Session {
    pub fn new(settings: Settings, path: PathBuf, instance: String) -> Result<Self> {
        let player = Mpv::new(settings.volume)?;
        Ok(Self {
            state: Snapshot::new(instance, &settings),
            settings,
            path,
            player,
            wanted: false,
            retry: None,
            failures: 0,
            connecting_since: None,
            online_since: None,
            elapsed: Instant::now(),
            last_open: None,
        })
    }
    fn open(&mut self) -> Result<()> {
        if let Some(at) = self
            .last_open
            .filter(|at| at.elapsed() < Duration::from_secs(1))
        {
            self.retry = Some(at + Duration::from_secs(1));
            return Ok(());
        }
        self.last_open = Some(Instant::now());
        self.retry = None;
        self.state.phase = Phase::Connecting;
        self.connecting_since = Some(Instant::now());
        self.online_since = None;
        self.player.play(&self.settings.stream_url())
    }
    fn reconnect(&mut self, message: String) {
        if !self.wanted || self.state.user_paused || self.retry.is_some() {
            return;
        }
        self.failures += 1;
        self.state.error = Some(message);
        self.online_since = None;
        self.connecting_since = None;
        if self.failures >= 5 {
            self.wanted = false;
            self.state.phase = Phase::Idle;
            let _ = self.player.stop();
        } else {
            self.state.phase = Phase::Reconnecting;
            // Bounded exponential backoff with per-instance jitter; no rapid reconnect loop.
            let jitter = self
                .state
                .instance_id
                .as_bytes()
                .first()
                .copied()
                .unwrap_or(0) as u64
                * 11;
            self.retry = Some(
                Instant::now()
                    + Duration::from_millis((1u64 << (self.failures - 1)) * 1000 + jitter),
            );
        }
    }
    pub fn execute(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Play => {
                if self.wanted
                    && matches!(
                        self.state.phase,
                        Phase::Online | Phase::Connecting | Phase::Buffering
                    )
                {
                    return Ok(());
                }
                self.wanted = true;
                self.state.user_paused = false;
                self.failures = 0;
                self.state.error = None;
                if let Err(error) = self.open() {
                    self.reconnect(error.to_string());
                }
            }
            Command::Pause | Command::Stop | Command::Shutdown | Command::PrepareUpdate => {
                self.wanted = false;
                self.retry = None;
                self.connecting_since = None;
                self.online_since = None;
                self.state.user_paused = command == Command::Pause;
                self.state.phase = Phase::Idle;
                self.state.error = None;
                self.state.shutdown_reason = if command == Command::PrepareUpdate {
                    Some("update".into())
                } else {
                    None
                };
                self.player.stop()?;
                if command == Command::Stop {
                    self.state.listened_sec = 0;
                }
            }
            Command::Toggle => {
                return self.execute(if self.wanted {
                    Command::Pause
                } else {
                    Command::Play
                });
            }
            Command::SetVolume { value } => {
                if !value.is_finite() || !(0.0..=100.0).contains(&value) {
                    bail!("volume must be between 0 and 100");
                }
                let mut settings = self.settings.clone();
                settings.volume = value;
                if value > 0.0 {
                    settings.last_audible_volume = value;
                }
                settings.save(&self.path)?;
                self.player.volume(value)?;
                self.settings = settings;
                self.state.volume = value;
                self.state.last_audible_volume = self.settings.last_audible_volume;
            }
            Command::SetSource { source } => {
                if !["auto", "source1", "source2"].contains(&source.as_str()) {
                    bail!("source must be auto, source1 or source2");
                }
                if source == self.settings.source {
                    return Ok(());
                }
                let mut settings = self.settings.clone();
                settings.source = source;
                settings.save(&self.path)?;
                self.settings = settings;
                self.state.source = self.settings.source.clone();
                self.state.radio_base_url = self.settings.source_url().into();
                self.state.station_status = None;
                if self.wanted {
                    self.failures = 0;
                    if let Err(e) = self.open() {
                        self.reconnect(e.to_string());
                    }
                }
            }
            Command::SetProfile { profile_id } => {
                if profile_id == self.settings.profile_id {
                    return Ok(());
                }
                if !enabled_profiles(&self.state)
                    .iter()
                    .any(|p| *p == profile_id)
                {
                    bail!("profile is not currently enabled; refresh radio status first");
                }
                let mut settings = self.settings.clone();
                settings.profile_id = profile_id;
                settings.save(&self.path)?;
                self.settings = settings;
                self.state.profile_id = self.settings.profile_id.clone();
                if self.wanted {
                    self.failures = 0;
                    if let Err(e) = self.open() {
                        self.reconnect(e.to_string());
                    }
                }
            }
            Command::Refresh | Command::TestSources => {}
        }
        self.elapsed = Instant::now();
        Ok(())
    }
    fn tick(&mut self) {
        for event in self.player.events() {
            if !self.wanted {
                continue;
            }
            match event {
                PlaybackEvent::Loaded => {}
                PlaybackEvent::Playing => {
                    self.state.phase = Phase::Online;
                    self.state.error = None;
                    self.connecting_since = None;
                    let since = *self.online_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= Duration::from_secs(10) {
                        self.failures = 0;
                    }
                }
                PlaybackEvent::Buffering => {
                    self.state.phase = Phase::Buffering;
                    self.connecting_since.get_or_insert_with(Instant::now);
                    self.online_since = None;
                }
                PlaybackEvent::Ended => self.reconnect("live stream ended".into()),
                PlaybackEvent::Error(e) => self.reconnect(e),
            }
        }
        if self.wanted
            && self
                .connecting_since
                .is_some_and(|time| time.elapsed() > Duration::from_secs(15))
        {
            let _ = self.player.stop();
            self.reconnect("audio connection stalled".into());
        }
        if self.wanted && self.retry.is_some_and(|at| Instant::now() >= at) {
            if let Err(e) = self.open() {
                self.reconnect(e.to_string());
            }
        }
        let elapsed = self.elapsed.elapsed().as_secs();
        if elapsed > 0 {
            if self.state.phase == Phase::Online {
                self.state.listened_sec += elapsed;
            }
            self.elapsed = Instant::now();
        }
    }
}

fn enabled_profiles(state: &Snapshot) -> Vec<&str> {
    state
        .station_status
        .as_ref()
        .and_then(|s| s.pointer("/station/bitrates"))
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|p| p.get("enabled").and_then(|v| v.as_bool()) != Some(false))
        .filter_map(|p| p.get("id").and_then(|v| v.as_str()))
        .collect()
}

struct Metadata {
    generation: u64,
    result: Result<serde_json::Value>,
}

pub async fn run(
    mut session: Session,
    mut commands: mpsc::Receiver<Envelope>,
    snapshots: watch::Sender<Snapshot>,
    mut media_commands: mpsc::UnboundedReceiver<Command>,
    stop: watch::Sender<bool>,
) {
    let http = crate::network::PublicHttp::new().expect("HTTP TLS setup");
    let (meta_tx, mut meta_rx) = mpsc::channel::<Metadata>(4);
    let mut timer = tokio::time::interval(Duration::from_millis(200));
    let mut next_meta = Instant::now();
    let mut metadata_not_before = Instant::now();
    let mut metadata_failures = 0u32;
    let mut meta_generation = 0u64;
    let mut meta_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut publish_at = Instant::now();
    let mut last_state = String::new();
    let (probe_tx, mut probe_rx) = mpsc::channel::<(u64, Vec<serde_json::Value>)>(2);
    let mut probe_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut probe_generation = 0;
    let mut probe_requested = session.settings.source == "auto";
    let mut next_probe = Instant::now();
    loop {
        let mut shutdown = false;
        tokio::select! {
            envelope = commands.recv() => {
                let Some(envelope) = envelope else { break; };
                let command = envelope.request.command;
                shutdown = matches!(command, Command::Shutdown | Command::PrepareUpdate);
                let source_change = matches!(&command, Command::SetSource { source } if source != &session.settings.source);
                let refresh = matches!(command, Command::Refresh) || source_change;
                let probe = matches!(&command, Command::TestSources) || matches!(&command, Command::SetSource { source } if source == "auto");
                let result = session.execute(command);
                if result.is_ok() && refresh {
                    if source_change {
                        meta_generation += 1;
                        if let Some(task) = meta_task.take() { task.abort(); }
                        metadata_not_before = Instant::now();
                        metadata_failures = 0;
                    }
                    // Refresh joins current work; it never aborts/restarts an HTTP request.
                    if source_change || meta_task.as_ref().is_none_or(|t| t.is_finished()) {
                        next_meta = Instant::now().max(metadata_not_before);
                    }
                }
                if result.is_ok() && source_change {
                    probe_requested = false;
                    probe_generation += 1;
                    if let Some(task) = probe_task.take() { task.abort(); }
                    session.state.source_testing = false;
                }
                if result.is_ok() && probe && Instant::now() >= next_probe &&
                    probe_task.as_ref().is_none_or(|t| t.is_finished()) {
                    probe_requested = true;
                }
                session.state.revision += 1;
                snapshots.send_replace(session.state.clone());
                let _ = envelope.reply.send(result.map(|_| session.state.clone()).map_err(|e| e.to_string()));
            }
            Some(command) = media_commands.recv() => {
                if let Err(error) = session.execute(command) { tracing::warn!(%error, "system media command failed"); }
            }
            Some((generation, results)) = probe_rx.recv() => {
                if generation == probe_generation {
                    session.state.source_testing = false;
                    if session.settings.source == "auto" {
                        if let Some(selected) = crate::sources::choose(&results, &session.settings.selected_source) {
                            if selected != session.settings.selected_source {
                                let mut settings = session.settings.clone();
                                settings.selected_source = selected;
                                if settings.save(&session.path).is_ok() {
                                    session.settings = settings;
                                    session.state.radio_base_url = session.settings.source_url().into();
                                    session.state.station_status = None;
                                    if session.wanted { if let Err(e) = session.open() { session.reconnect(e.to_string()); } }
                                    meta_generation += 1;
                                    if let Some(task) = meta_task.take() { task.abort(); }
                                    next_meta = Instant::now();
                                    metadata_not_before = Instant::now();
                                    metadata_failures = 0;
                                }
                            }
                        }
                    }
                    session.state.source_measurements = results;
                }
            }
            metadata = meta_rx.recv() => {
                if let Some(metadata) = metadata {
                    if metadata.generation == meta_generation {
                        match metadata.result {
                            Ok(status) => {
                                metadata_failures = 0;
                                let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                                if let Some(end) = status.pointer("/current/expectedEndMs").and_then(|v| v.as_u64()) {
                                    if end + 800 > now_ms { next_meta = next_meta.min(Instant::now() + Duration::from_millis((end + 800 - now_ms).min(10000))); }
                                }
                                session.state.metadata_error = None;
                                session.state.station_status = Some(status);
                            }
                            Err(_) => {
                                metadata_failures = (metadata_failures + 1).min(6);
                                metadata_not_before = Instant::now() + Duration::from_secs((10 * (1 << (metadata_failures - 1))).min(300));
                                next_meta = metadata_not_before;
                                session.state.metadata_error = Some("radio metadata request failed; audio playback is independent".into());
                            }
                        }
                    }
                }
            }
            _ = timer.tick() => session.tick(),
            _ = tokio::signal::ctrl_c() => { let _ = session.execute(Command::Shutdown); shutdown = true; }
        }
        if shutdown {
            if session.state.shutdown_reason.as_deref() == Some("update") {
                // Let subscribed terminal frontends restore their terminals and exit.
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            break;
        }
        if probe_requested && probe_task.as_ref().is_none_or(|t| t.is_finished()) {
            probe_requested = false;
            next_probe = Instant::now() + Duration::from_secs(60);
            session.state.source_testing = true;
            let http = http.clone();
            let settings = session.settings.clone();
            let send = probe_tx.clone();
            let generation = probe_generation;
            probe_task = Some(tokio::spawn(async move {
                let results = crate::sources::measure(&http, &settings).await;
                let _ = send.send((generation, results)).await;
            }));
        }
        if Instant::now() >= next_meta && meta_task.as_ref().is_none_or(|t| t.is_finished()) {
            let url = format!(
                "{}/api/stations/{}/status",
                session.state.radio_base_url.trim_end_matches('/'),
                session.state.station_id
            );
            let http = http.clone();
            let send = meta_tx.clone();
            let generation = meta_generation;
            meta_task = Some(tokio::spawn(async move {
                let result = async {
                    let response = http.get(&url).await?;
                    let status: serde_json::Value = response.json().await?;
                    if status.get("station").is_none() {
                        bail!("invalid station status");
                    }
                    Ok(status)
                }
                .await;
                let _ = send.send(Metadata { generation, result }).await;
            }));
            next_meta = Instant::now() + Duration::from_secs(10);
            metadata_not_before = Instant::now() + Duration::from_secs(1);
        }
        let mut comparable = session.state.clone();
        comparable.revision = 0;
        let serialized = serde_json::to_string(&comparable).unwrap_or_default();
        if serialized != last_state || publish_at.elapsed() >= Duration::from_secs(1) {
            session.state.revision += 1;
            snapshots.send_replace(session.state.clone());
            last_state = serialized;
            publish_at = Instant::now();
        }
    }
    if let Some(task) = meta_task {
        task.abort();
    }
    if let Some(task) = probe_task {
        task.abort();
    }
    let _ = session.player.stop();
    stop.send_replace(true);
}
