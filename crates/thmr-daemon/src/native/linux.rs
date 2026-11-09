use anyhow::Result;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use thmr_core::protocol::{Command, Phase, Snapshot};
use tokio::sync::mpsc::UnboundedSender;
use zbus::{
    blocking::Connection,
    zvariant::{ObjectPath, OwnedValue, Str, Value},
};

const PATH: &str = "/org/mpris/MediaPlayer2";
type Shared = Arc<RwLock<Snapshot>>;
pub struct Media {
    connection: Connection,
    state: Shared,
}
struct Root {
    commands: UnboundedSender<Command>,
}
struct Player {
    state: Shared,
    commands: UnboundedSender<Command>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported("use thmr gui".into()))
    }
    fn quit(&self) {
        let _ = self.commands.send(Command::Shutdown);
    }
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> &str {
        "THMR 东方音乐电台"
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        "thmr"
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec![]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec!["audio/mpeg".into()]
    }
}
fn status(state: &Snapshot) -> &str {
    match state.phase {
        Phase::Online | Phase::Buffering => "Playing",
        _ if state.user_paused => "Paused",
        _ => "Stopped",
    }
}
fn metadata(state: &Snapshot) -> HashMap<String, OwnedValue> {
    let mut result = HashMap::new();
    let Some(current) = state.current() else {
        return result;
    };
    let text = |key| current.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let track = format!(
        "/cn/yunmoan/thmr/track/t{}",
        text("trackId")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>()
    );
    if let Ok(path) = ObjectPath::try_from(track) {
        result.insert("mpris:trackid".into(), OwnedValue::from(path));
    }
    result.insert(
        "xesam:title".into(),
        OwnedValue::from(Str::from(text("title").to_owned())),
    );
    if let Ok(artist) = OwnedValue::try_from(Value::from(vec![text("artist").to_owned()])) {
        result.insert("xesam:artist".into(), artist);
    }
    let cover = text("coverUrl");
    if !cover.is_empty() {
        let url = if cover.starts_with("http://") || cover.starts_with("https://") {
            cover.to_owned()
        } else {
            format!("{}{cover}", state.radio_base_url)
        };
        result.insert("mpris:artUrl".into(), OwnedValue::from(Str::from(url)));
    }
    result
}
impl Player {
    fn send(&self, command: Command) -> zbus::fdo::Result<()> {
        self.commands
            .send(command)
            .map_err(|_| zbus::fdo::Error::Failed("host stopping".into()))
    }
}
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn play(&self) -> zbus::fdo::Result<()> {
        self.send(Command::Play)
    }
    fn pause(&self) -> zbus::fdo::Result<()> {
        self.send(Command::Pause)
    }
    fn play_pause(&self) -> zbus::fdo::Result<()> {
        self.send(Command::Toggle)
    }
    fn stop(&self) -> zbus::fdo::Result<()> {
        self.send(Command::Stop)
    }
    fn next(&self) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported("live radio".into()))
    }
    fn previous(&self) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported("live radio".into()))
    }
    fn seek(&self, _offset: i64) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported("live radio".into()))
    }
    fn set_position(&self, _track_id: ObjectPath<'_>, _position: i64) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported("live radio".into()))
    }
    fn open_uri(&self, _uri: &str) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported(
            "only configured THMR stations".into(),
        ))
    }
    #[zbus(property)]
    fn playback_status(&self) -> String {
        status(&self.state.read().unwrap()).into()
    }
    #[zbus(property)]
    fn loop_status(&self) -> &str {
        "None"
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn set_rate(&self, value: f64) -> zbus::fdo::Result<()> {
        if value == 1.0 {
            Ok(())
        } else {
            Err(zbus::fdo::Error::NotSupported("live radio".into()))
        }
    }
    #[zbus(property)]
    fn shuffle(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        metadata(&self.state.read().unwrap())
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.state.read().unwrap().volume / 100.0
    }
    #[zbus(property)]
    fn set_volume(&self, value: f64) -> zbus::fdo::Result<()> {
        if !value.is_finite() {
            return Err(zbus::fdo::Error::InvalidArgs("invalid volume".into()));
        }
        self.send(Command::SetVolume {
            value: (value * 100.0).clamp(0.0, 100.0),
        })
    }
    #[zbus(property)]
    fn position(&self) -> i64 {
        0
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}
impl Media {
    pub fn new(commands: UnboundedSender<Command>) -> Result<Self> {
        let state = Arc::new(RwLock::new(Snapshot::new(
            "native".into(),
            &Default::default(),
        )));
        let connection = zbus::blocking::connection::Builder::session()?
            .name("org.mpris.MediaPlayer2.thmr")?
            .serve_at(
                PATH,
                Root {
                    commands: commands.clone(),
                },
            )?
            .serve_at(
                PATH,
                Player {
                    state: state.clone(),
                    commands,
                },
            )?
            .build()?;
        Ok(Self { connection, state })
    }
    pub fn pump(&self) {}
    pub fn update(&mut self, state: &Snapshot) -> Result<()> {
        let mut changed = HashMap::<String, OwnedValue>::new();
        changed.insert(
            "PlaybackStatus".into(),
            OwnedValue::from(Str::from(status(state))),
        );
        changed.insert("Volume".into(), OwnedValue::from(state.volume / 100.0));
        if let Ok(value) = OwnedValue::try_from(Value::from(metadata(state))) {
            changed.insert("Metadata".into(), value);
        }
        *self.state.write().unwrap() = state.clone();
        self.connection.emit_signal(
            None::<&str>,
            PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &(
                "org.mpris.MediaPlayer2.Player",
                changed,
                Vec::<String>::new(),
            ),
        )?;
        Ok(())
    }
}
