mod mpv;
mod native;
mod server;
mod session;
mod sources;

use anyhow::{Context, Result};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc as std_mpsc, Arc,
};
use thmr_core::{
    runtime::{self, Endpoint, InstanceGuard},
    settings::Settings,
};
use tokio::sync::{mpsc, watch};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    if let Err(error) = run() {
        eprintln!("thmrd: {error:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version") {
        println!(
            "THMR {}+{} · protocol {}",
            thmr_core::VERSION,
            thmr_core::BUILD_NUMBER,
            thmr_core::PROTOCOL_VERSION
        );
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--help") {
        println!("thmrd [--check | --version]\nUser-session THMR host. Configure THMR_MPV_LIBRARY for development.");
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--check") {
        let _player = mpv::Mpv::new(0.0)?;
        println!("libmpv ABI 2: ready");
        return Ok(());
    }
    if !args.is_empty() {
        anyhow::bail!("unknown host argument; see thmrd --help");
    }
    let guard = InstanceGuard::acquire(&runtime::runtime_dir()?)?;
    runtime::private_dir(&runtime::config_dir()?)?;
    let path = runtime::config_dir()?.join("settings.toml");
    let settings = Settings::load(&path)?;
    let (media_tx, media_rx) = mpsc::unbounded_channel();
    let (native_tx, native_rx) = std_mpsc::channel();
    let done = Arc::new(AtomicBool::new(false));
    let worker_done = done.clone();
    // Native initialization happens on the process main thread, before owning audio.
    let mut native = native::Host::new(media_tx).unwrap_or_else(|error| { tracing::warn!(%error, "native media controls unavailable; normal playback remains available"); native::Host::disabled() });
    let system_media = native.enabled();
    let worker = std::thread::spawn(move || -> Result<()> {
        struct Complete(Arc<AtomicBool>);
        impl Drop for Complete {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let _done = Complete(worker_done);
        let _guard = guard;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async move {
            let instance = runtime::random_id();
            let mut session = session::Session::new(settings, path, instance.clone())?;
            session.state.capabilities.system_media = system_media;
            let (snapshots, snapshot_rx) = watch::channel(session.state.clone());
            let (commands, command_rx) = mpsc::channel(64);
            let (stop, mut stop_rx) = watch::channel(false);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let endpoint = Endpoint {
                protocol_version: thmr_core::PROTOCOL_VERSION,
                port: listener.local_addr()?.port(),
                token: runtime::random_id(),
                instance_id: instance,
                pid: std::process::id(),
            };
            let app = server::router(server::AppState {
                token: endpoint.token.clone(),
                commands,
                snapshots: snapshot_rx.clone(),
            });
            let server = tokio::spawn(async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(async move {
                        while !*stop_rx.borrow_and_update() {
                            if stop_rx.changed().await.is_err() {
                                break;
                            }
                        }
                    })
                    .await
            });
            _guard.publish(&endpoint)?;
            let update = tokio::spawn(async move {
                let mut snapshot_rx = snapshot_rx;
                loop {
                    let snapshot = snapshot_rx.borrow_and_update().clone();
                    if native_tx.send(snapshot).is_err() {
                        break;
                    }
                    if snapshot_rx.changed().await.is_err() {
                        break;
                    }
                }
            });
            session::run(session, command_rx, snapshots, media_rx, stop).await;
            update.abort();
            // Open WebSockets must not indefinitely hold the process during shutdown.
            let _ = tokio::time::timeout(std::time::Duration::from_millis(500), server).await;
            Ok(())
        })
    });
    native.run(native_rx, done)?;
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("playback worker panicked"))?
        .context("playback host stopped")
}
