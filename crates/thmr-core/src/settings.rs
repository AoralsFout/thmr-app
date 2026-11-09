use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub station_id: String,
    pub profile_id: String,
    pub source: String,
    pub source1: String,
    pub source2: String,
    pub selected_source: String,
    pub volume: f64,
    pub last_audible_volume: f64,
    pub default_ui: String,
    pub gui_path: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            station_id: "touhou".into(),
            profile_id: "128k".into(),
            source: "auto".into(),
            source1: "https://radio.yunmoan.cn".into(),
            source2: "https://radio.zyghit.cn".into(),
            selected_source: "source1".into(),
            volume: 70.0,
            last_audible_volume: 70.0,
            default_ui: "tui".into(),
            gui_path: None,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        let settings: Self = match fs::read_to_string(path) {
            Ok(data) => toml::from_str(&data).context("invalid THMR settings")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.into()),
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<()> {
        for id in [&self.station_id, &self.profile_id] {
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
            {
                bail!("station/profile must be a nonempty identifier");
            }
        }
        if !["auto", "source1", "source2"].contains(&self.source.as_str())
            || !["source1", "source2"].contains(&self.selected_source.as_str())
        {
            bail!("source must be auto, source1 or source2");
        }
        for source in [&self.source1, &self.source2] {
            let url = reqwest::Url::parse(source)?;
            if !["http", "https"].contains(&url.scheme())
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
            {
                bail!("radio source must be an HTTP(S) origin without credentials or query");
            }
        }
        if !self.volume.is_finite()
            || !(0.0..=100.0).contains(&self.volume)
            || !self.last_audible_volume.is_finite()
            || !(0.0..=100.0).contains(&self.last_audible_volume)
        {
            bail!("volume must be between 0 and 100");
        }
        if !["tui", "gui"].contains(&self.default_ui.as_str()) {
            bail!("defaultUi must be tui or gui");
        }
        Ok(())
    }

    pub fn source_url(&self) -> &str {
        let id = if self.source == "auto" {
            &self.selected_source
        } else {
            &self.source
        };
        if id == "source2" {
            &self.source2
        } else {
            &self.source1
        }
    }

    pub fn stream_url(&self) -> String {
        format!(
            "{}/radio/{}/{}",
            self.source_url().trim_end_matches('/'),
            self.station_id,
            self.profile_id
        )
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, toml::to_string_pretty(self)?)?;
        // rename replaces atomically on Unix; Windows needs ReplaceFileW semantics.
        #[cfg(windows)]
        if path.exists() {
            use std::os::windows::ffi::OsStrExt;
            let old: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let new: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
            unsafe extern "system" {
                fn MoveFileExW(old: *const u16, new: *const u16, flags: u32) -> i32;
            }
            if unsafe { MoveFileExW(new.as_ptr(), old.as_ptr(), 1 | 8) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            return Ok(());
        }
        fs::rename(tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_roundtrip_and_reject_invalid_urls_volume() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut settings = Settings {
            volume: 0.0,
            last_audible_volume: 42.0,
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        settings.volume = 35.0;
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap().volume, 35.0);
        settings.source1 = "https://example.org/?token=secret".into();
        assert!(settings.validate().is_err());
        settings.source1 = Settings::default().source1;
        settings.volume = f64::NAN;
        assert!(settings.validate().is_err());
    }
}
