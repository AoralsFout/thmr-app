use crate::PROTOCOL_VERSION;
use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use fs2::FileExt;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

pub fn random_id() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn runtime_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("THMR_RUNTIME_DIR") {
        return Ok(path.into());
    }
    let dirs = ProjectDirs::from("cn", "yunmoan", "thmr")
        .context("cannot locate user runtime directory")?;
    Ok(dirs.data_local_dir().join("runtime"))
}

pub fn config_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("THMR_CONFIG_DIR") {
        return Ok(path.into());
    }
    Ok(ProjectDirs::from("cn", "yunmoan", "thmr")
        .context("cannot locate user settings directory")?
        .config_dir()
        .to_owned())
}

pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let metadata = fs::symlink_metadata(path)?;
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        if !metadata.is_dir() || metadata.uid() != unsafe { geteuid() } {
            bail!("runtime directory is not owned by current user");
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub protocol_version: u32,
    pub port: u16,
    pub token: String,
    pub instance_id: String,
    pub pid: u32,
}

impl Endpoint {
    pub fn read(dir: &Path) -> Result<Self> {
        let endpoint: Self = serde_json::from_slice(
            &fs::read(dir.join("instance.json")).context("THMR is not running")?,
        )?;
        if endpoint.protocol_version != PROTOCOL_VERSION {
            bail!(
                "incompatible THMR protocol {}; expected {}",
                endpoint.protocol_version,
                PROTOCOL_VERSION
            );
        }
        if endpoint.port == 0
            || endpoint.token.len() != 64
            || !endpoint.token.bytes().all(|b| b.is_ascii_hexdigit())
        {
            bail!("invalid THMR instance file");
        }
        Ok(endpoint)
    }
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

pub struct InstanceGuard {
    _file: File,
    dir: PathBuf,
}
impl InstanceGuard {
    pub fn acquire(dir: &Path) -> Result<Self> {
        private_dir(dir)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(dir.join("instance.lock"))?;
        file.try_lock_exclusive()
            .context("THMR is already running in this user session")?;
        Ok(Self {
            _file: file,
            dir: dir.to_owned(),
        })
    }
    pub fn publish(&self, endpoint: &Endpoint) -> Result<()> {
        let tmp = self.dir.join("instance.tmp");
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(&tmp)?;
        file.write_all(&serde_json::to_vec(endpoint)?)?;
        file.sync_all()?;
        let path = self.dir.join("instance.json");
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(tmp, path)?;
        Ok(())
    }
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.dir.join("instance.json"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_instances_cannot_publish_or_remove_the_owner_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let guard = InstanceGuard::acquire(dir.path()).unwrap();
        let endpoint = Endpoint {
            protocol_version: 1,
            port: 1234,
            token: random_id(),
            instance_id: random_id(),
            pid: 1,
        };
        guard.publish(&endpoint).unwrap();
        assert!(InstanceGuard::acquire(dir.path()).is_err());
        assert_eq!(Endpoint::read(dir.path()).unwrap().port, 1234);
        drop(guard);
        assert!(Endpoint::read(dir.path()).is_err());
        assert!(InstanceGuard::acquire(dir.path()).is_ok());
    }
}
