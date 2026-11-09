use anyhow::{Context, Result};
use block2::RcBlock;
use objc2::{rc::Retained, runtime::AnyObject, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSTimer};
use objc2_media_player::{
    MPMediaItemPropertyArtist, MPMediaItemPropertyTitle, MPNowPlayingInfoCenter,
    MPNowPlayingInfoPropertyIsLiveStream, MPNowPlayingPlaybackState, MPRemoteCommand,
    MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};
use std::{
    cell::RefCell,
    ptr::NonNull,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
        Arc,
    },
};
use thmr_core::protocol::{Command, Phase, Snapshot};
use tokio::sync::mpsc::UnboundedSender;

pub struct Media {
    app: Retained<NSApplication>,
    bindings: Vec<(Retained<MPRemoteCommand>, Retained<AnyObject>)>,
}
impl Media {
    pub fn new(commands: UnboundedSender<Command>) -> Result<Self> {
        let marker = MainThreadMarker::new().context("macOS media host must run on main thread")?;
        let app = NSApplication::sharedApplication(marker);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        let center = unsafe { MPRemoteCommandCenter::sharedCommandCenter() };
        let mut bindings = Vec::new();
        for (native, command) in unsafe {
            [
                (center.playCommand(), Command::Play),
                (center.pauseCommand(), Command::Pause),
                (center.stopCommand(), Command::Stop),
                (center.togglePlayPauseCommand(), Command::Toggle),
            ]
        } {
            let commands = commands.clone();
            let callback = RcBlock::new(move |_event: NonNull<MPRemoteCommandEvent>| {
                let _ = commands.send(command.clone());
                MPRemoteCommandHandlerStatus::Success
            });
            let token = unsafe { native.addTargetWithHandler(&callback) };
            unsafe {
                native.setEnabled(true);
            }
            bindings.push((native, token));
        }
        unsafe {
            for command in [
                center.nextTrackCommand(),
                center.previousTrackCommand(),
                center.changePlaybackPositionCommand(),
                center.skipForwardCommand(),
                center.skipBackwardCommand(),
                center.seekForwardCommand(),
                center.seekBackwardCommand(),
            ] {
                command.setEnabled(false);
            }
            MPNowPlayingInfoCenter::defaultCenter()
                .setPlaybackState(MPNowPlayingPlaybackState::Stopped);
        }
        Ok(Self { app, bindings })
    }
    pub fn update(&mut self, state: &Snapshot) -> Result<()> {
        let current = state.current();
        let title = NSString::from_str(
            current
                .and_then(|c| c.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("THMR 东方音乐电台"),
        );
        let artist = NSString::from_str(
            current
                .and_then(|c| c.get("artist"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        );
        let live = NSNumber::new_bool(true);
        let values: [&AnyObject; 3] = [&title, &artist, &live];
        unsafe {
            let keys = [
                MPMediaItemPropertyTitle,
                MPMediaItemPropertyArtist,
                MPNowPlayingInfoPropertyIsLiveStream,
            ];
            let info = NSDictionary::<NSString, AnyObject>::from_slices(&keys, &values);
            let center = MPNowPlayingInfoCenter::defaultCenter();
            center.setNowPlayingInfo(Some(&info));
            center.setPlaybackState(match state.phase {
                Phase::Online | Phase::Buffering => MPNowPlayingPlaybackState::Playing,
                _ if state.user_paused => MPNowPlayingPlaybackState::Paused,
                _ => MPNowPlayingPlaybackState::Stopped,
            });
        }
        Ok(())
    }
    pub fn run(self, updates: Receiver<Snapshot>, done: Arc<AtomicBool>) -> Result<()> {
        let app = self.app.clone();
        let media = Rc::new(RefCell::new(self));
        let callback = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            // This timer is scheduled and retained on the main run loop only.
            if done.load(Ordering::Acquire) {
                media.borrow().app.stop(None);
                return;
            }
            if let Ok(mut snapshot) = updates.try_recv() {
                while let Ok(newer) = updates.try_recv() {
                    snapshot = newer;
                }
                if let Err(error) = media.borrow_mut().update(&snapshot) {
                    tracing::debug!(%error, "Now Playing update failed");
                }
            }
        });
        let timer =
            unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.05, true, &callback) };
        app.run();
        timer.invalidate();
        Ok(())
    }
}
impl Drop for Media {
    fn drop(&mut self) {
        unsafe {
            for (command, token) in &self.bindings {
                command.removeTarget(Some(token));
            }
            let center = MPNowPlayingInfoCenter::defaultCenter();
            center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
            center.setNowPlayingInfo(None);
        }
    }
}
