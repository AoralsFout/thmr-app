use anyhow::Result;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Receiver,
    Arc,
};
use thmr_core::protocol::{Command, Snapshot};
use tokio::sync::mpsc::UnboundedSender;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
type Platform = windows::Media;
#[cfg(target_os = "linux")]
type Platform = linux::Media;
#[cfg(target_os = "macos")]
type Platform = macos::Media;

pub struct Host {
    platform: Option<Platform>,
}
impl Host {
    pub fn new(commands: UnboundedSender<Command>) -> Result<Self> {
        if std::env::var_os("THMR_NO_SYSTEM_MEDIA").is_some() {
            return Ok(Self::disabled());
        }
        Ok(Self {
            platform: Some(Platform::new(commands)?),
        })
    }
    pub fn disabled() -> Self {
        Self { platform: None }
    }
    pub fn enabled(&self) -> bool {
        self.platform.is_some()
    }
    pub fn run(&mut self, updates: Receiver<Snapshot>, done: Arc<AtomicBool>) -> Result<()> {
        #[cfg(target_os = "macos")]
        if let Some(media) = self.platform.take() {
            return media.run(updates, done);
        }
        while !done.load(Ordering::Acquire) {
            if let Some(media) = &mut self.platform {
                #[cfg(not(target_os = "macos"))]
                media.pump();
            }
            if let Ok(mut snapshot) = updates.recv_timeout(std::time::Duration::from_millis(50)) {
                while let Ok(newer) = updates.try_recv() {
                    snapshot = newer;
                }
                if let Some(media) = &mut self.platform {
                    if let Err(error) = media.update(&snapshot) {
                        tracing::debug!(%error, "media metadata update failed");
                    }
                }
            }
        }
        Ok(())
    }
}
