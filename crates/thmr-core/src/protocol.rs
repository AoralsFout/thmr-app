use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{settings::Settings, BUILD_NUMBER, PROTOCOL_VERSION, VERSION};

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(
    tag = "command",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Command {
    Play,
    Pause,
    Stop,
    Toggle,
    SetVolume { value: f64 },
    SetSource { source: String },
    SetProfile { profile_id: String },
    Refresh,
    TestSources,
    Shutdown,
}

impl<'de> Deserialize<'de> for Command {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| D::Error::custom("command must be an object"))?;
        let name = object
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| D::Error::custom("missing command"))?;
        let extra = match name {
            "set_volume" => Some("value"),
            "set_source" => Some("source"),
            "set_profile" => Some("profileId"),
            _ => None,
        };
        if object
            .keys()
            .any(|key| key != "command" && Some(key.as_str()) != extra)
        {
            return Err(D::Error::custom("unknown command field"));
        }
        let text = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| D::Error::custom(format!("missing string {key}")))
        };
        Ok(match name {
            "play" => Self::Play,
            "pause" => Self::Pause,
            "stop" => Self::Stop,
            "toggle" => Self::Toggle,
            "refresh" => Self::Refresh,
            "test_sources" => Self::TestSources,
            "shutdown" => Self::Shutdown,
            "set_volume" => Self::SetVolume {
                value: object
                    .get("value")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| D::Error::custom("missing numeric value"))?,
            },
            "set_source" => Self::SetSource {
                source: text("source")?,
            },
            "set_profile" => Self::SetProfile {
                profile_id: text("profileId")?,
            },
            _ => return Err(D::Error::custom("unsupported THMR command")),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub protocol_version: u32,
    pub command: Command,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Connecting,
    Online,
    Buffering,
    Reconnecting,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub seek: bool,
    pub next: bool,
    pub previous: bool,
    pub system_media: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub protocol_version: u32,
    pub version: String,
    pub build_number: String,
    pub instance_id: String,
    pub revision: u64,
    pub phase: Phase,
    pub user_paused: bool,
    pub volume: f64,
    pub last_audible_volume: f64,
    pub listened_sec: u64,
    pub source: String,
    pub radio_base_url: String,
    pub station_id: String,
    pub profile_id: String,
    pub station_status: Option<Value>,
    pub error: Option<String>,
    pub metadata_error: Option<String>,
    pub source_testing: bool,
    pub source_measurements: Vec<Value>,
    pub capabilities: Capabilities,
}

impl Snapshot {
    pub fn new(instance_id: String, settings: &Settings) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            version: VERSION.into(),
            build_number: BUILD_NUMBER.into(),
            instance_id,
            revision: 0,
            phase: Phase::Idle,
            user_paused: false,
            volume: settings.volume,
            last_audible_volume: settings.last_audible_volume,
            listened_sec: 0,
            source: settings.source.clone(),
            radio_base_url: settings.source_url().into(),
            station_id: settings.station_id.clone(),
            profile_id: settings.profile_id.clone(),
            station_status: None,
            error: None,
            metadata_error: None,
            source_testing: false,
            source_measurements: Vec::new(),
            capabilities: Capabilities {
                seek: false,
                next: false,
                previous: false,
                system_media: false,
            },
        }
    }

    pub fn current(&self) -> Option<&Value> {
        self.station_status
            .as_ref()?
            .get("current")
            .filter(|v| !v.is_null())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_uses_shared_camel_case_fields_and_rejects_unsupported_commands() {
        let request = Request {
            protocol_version: 1,
            command: Command::SetProfile {
                profile_id: "128k".into(),
            },
        };
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"protocolVersion":1,"command":{"command":"set_profile","profileId":"128k"}})
        );
        assert!(serde_json::from_value::<Command>(
            serde_json::json!({"command":"seek","value":12})
        )
        .is_err());
        assert!(serde_json::from_value::<Command>(
            serde_json::json!({"command":"play","url":"http://arbitrary"})
        )
        .is_err());
    }
}
