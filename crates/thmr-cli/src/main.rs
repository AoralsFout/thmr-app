mod tui;
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::{io::IsTerminal, path::PathBuf, process::Command as Process};
use thmr_core::{
    client::{spawn_detached, Client},
    protocol::{Command, Snapshot},
    runtime,
    settings::Settings,
    BUILD_NUMBER, PROTOCOL_VERSION, VERSION,
};

#[derive(Parser)]
#[command(name="thmr", version=VERSION, about="THMR 东方音乐电台 — CLI / TUI / GUI")]
struct Args {
    #[command(subcommand)]
    command: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    /// Open the terminal interface; q detaches and leaves playback running.
    Tui,
    /// Open the installed Flutter graphical client.
    Gui,
    /// Start background playback at the live edge.
    Play,
    Pause,
    /// Stop audio without shutting down the host.
    Stop,
    Toggle,
    /// Set THMR volume (0–100), independent of system volume.
    Volume {
        value: f64,
    },
    /// Select auto, source1, or source2.
    Source {
        source: String,
    },
    Quality {
        profile: String,
    },
    /// Show state without starting a host.
    Status {
        #[arg(long)]
        json: bool,
    },
    Refresh,
    /// Check the installation without starting playback.
    Doctor,
    /// Show non-secret installation paths.
    Paths {
        #[arg(long)]
        json: bool,
    },
    Daemon {
        #[command(subcommand)]
        command: DaemonAction,
    },
}
#[derive(Subcommand)]
enum DaemonAction {
    Start,
    Stop,
    PrepareUpdate,
}
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("thmr: {error:#}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let action = match Args::parse().command {
        Some(action) => action,
        None => {
            if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                bail!("choose a subcommand in non-interactive mode; see thmr --help");
            }
            let settings = Settings::load(&runtime::config_dir()?.join("settings.toml"))?;
            if settings.default_ui == "gui" {
                Action::Gui
            } else {
                Action::Tui
            }
        }
    };
    match action {
        Action::Paths { json } => {
            let paths = serde_json::json!({"runtime": runtime::runtime_dir()?, "config": runtime::config_dir()?});
            if json {
                println!("{paths}");
            } else {
                println!("{}", serde_json::to_string_pretty(&paths)?);
            }
            Ok(())
        }
        Action::Doctor => doctor().await,
        Action::Gui => open_gui().await,
        Action::Tui => {
            if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                bail!("TUI requires an interactive terminal");
            }
            tui::run(Client::ensure().await?).await
        }
        Action::Status { json } => {
            print_state(&Client::discover().await?.snapshot().await?, json);
            Ok(())
        }
        Action::Daemon {
            command: DaemonAction::Start,
        } => {
            print_state(&Client::ensure().await?.snapshot().await?, false);
            Ok(())
        }
        action => {
            let command = match action {
                Action::Play => Command::Play,
                Action::Pause => Command::Pause,
                Action::Stop => Command::Stop,
                Action::Toggle => Command::Toggle,
                Action::Volume { value } => Command::SetVolume { value },
                Action::Source { source } => Command::SetSource { source },
                Action::Quality { profile } => Command::SetProfile {
                    profile_id: profile,
                },
                Action::Refresh => Command::Refresh,
                Action::Daemon {
                    command: DaemonAction::Stop,
                } => Command::Shutdown,
                Action::Daemon {
                    command: DaemonAction::PrepareUpdate,
                } => Command::PrepareUpdate,
                _ => unreachable!(),
            };
            let client = if matches!(command, Command::Play | Command::Toggle) {
                Client::ensure().await?
            } else {
                Client::discover().await?
            };
            print_state(&client.execute(command).await?, false);
            Ok(())
        }
    }
}
fn print_state(state: &Snapshot, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(state).expect("snapshot serialization")
        );
        return;
    }
    let title = state
        .current()
        .and_then(|c| c.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("等待曲目信息");
    println!(
        "THMR {} · {:?} · {} · {} · 音量 {:.0}%\n{}",
        state.version, state.phase, state.source, state.profile_id, state.volume, title
    );
    if let Some(error) = &state.error {
        println!("播放：{error}");
    }
    if let Some(error) = &state.metadata_error {
        println!("元数据：{error}");
    }
}
async fn open_gui() -> Result<()> {
    let settings = Settings::load(&runtime::config_dir()?.join("settings.toml"))?;
    let exe = std::env::current_exe()?;
    let dir = exe.parent().context("cannot locate installation")?;
    let mut paths = Vec::<PathBuf>::new();
    if let Some(path) = std::env::var_os("THMR_GUI_PATH") {
        paths.push(path.into());
    }
    if let Some(path) = settings.gui_path {
        paths.push(path.into());
    }
    let registration = std::fs::read_to_string(runtime::config_dir()?.join("gui.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let registered = registration
        .as_ref()
        .filter(|j| {
            j.get("protocolVersion").and_then(|v| v.as_u64()) == Some(PROTOCOL_VERSION as u64)
        })
        .and_then(|j| j.get("executable"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from);
    if let Some(path) = &registered {
        paths.push(path.clone());
    }
    if cfg!(windows) {
        paths.push(dir.join("thmr_app.exe"));
    } else if cfg!(target_os = "macos") {
        paths.push(dir.join("../MacOS/THMR"));
        paths.push("/Applications/THMR.app/Contents/MacOS/THMR".into());
    } else {
        paths.push(dir.join("../thmr"));
        paths.push(dir.join("thmr_app"));
    }
    let gui = paths.into_iter().find(|p| p.is_file()).context(
        "compatible Flutter GUI not found; install the full package or set THMR_GUI_PATH",
    )?;
    if registered.as_ref() != Some(&gui) {
        let directory = gui.parent().context("invalid GUI path")?;
        let compatible = [
            directory.join("native-version.json"),
            directory.join("../Resources/native-version.json"),
        ]
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .any(|j| {
            j.get("protocolVersion").and_then(|v| v.as_u64()) == Some(PROTOCOL_VERSION as u64)
        });
        if !compatible {
            bail!("the installed GUI does not support the shared host; install a compatible full package");
        }
    }
    spawn_detached(&gui, &["--thmr-daemon"])?;
    println!("已打开 THMR 图形客户端");
    Ok(())
}
async fn doctor() -> Result<()> {
    println!("THMR {VERSION}+{BUILD_NUMBER} · protocol {PROTOCOL_VERSION}");
    println!(
        "OS/arch: {}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("Runtime: {}", runtime::runtime_dir()?.display());
    println!(
        "Settings: {}",
        runtime::config_dir()?.join("settings.toml").display()
    );
    let settings = Settings::load(&runtime::config_dir()?.join("settings.toml"))?;
    println!("Radio: {} · {}", settings.source_url(), settings.station_id);
    match Client::discover().await {
        Ok(client) => {
            let state = client.snapshot().await?;
            println!(
                "Host: {} · protocol {} · native media {}",
                state.version, state.protocol_version, state.capabilities.system_media
            );
        }
        Err(e) => println!("Host: {e}"),
    }
    let daemon = thmr_core::client::daemon_path()?;
    if !Process::new(daemon)
        .arg("--check")
        .status()
        .context("could not run thmrd installation check")?
        .success()
    {
        bail!("audio dependency check failed");
    }
    Ok(())
}
