mod battlefield;
mod choreography;
mod danmaku;
mod model;
mod render;
mod spectrum;
mod spectrum_view;
mod timing;

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
};
use futures_util::StreamExt;
use model::{Action, Overlay, Preferences, Ui};
use ratatui::layout::Position;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use thmr_core::{
    client::Client,
    protocol::{Command, Snapshot},
    PROTOCOL_VERSION,
};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue};

struct View {
    state: Snapshot,
    connection: Option<String>,
}
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), DisableMouseCapture, DisableFocusChange);
        ratatui::restore();
    }
}
struct Request {
    id: u64,
    command: Command,
}
struct Reply {
    id: u64,
    result: std::result::Result<(), String>,
}

pub async fn run(client: Client) -> Result<()> {
    let view = Arc::new(Mutex::new(View {
        state: client.snapshot().await?,
        connection: None,
    }));
    let reader = view.clone();
    let subscribe_task = tokio::spawn(async move {
        let mut client = client;
        loop {
            if subscribe(&client, &reader).await.is_err() {
                reader.lock().unwrap().connection = Some("主机连接中断，正在重连".into());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            if let Ok(reconnected) = Client::discover().await {
                client = reconnected;
            }
        }
    });
    let (command_tx, mut command_rx) = mpsc::channel::<Request>(32);
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Reply>();
    let command_view = view.clone();
    let commands = tokio::spawn(async move {
        while let Some(request) = command_rx.recv().await {
            // Serial execution preserves toggle/source order. Mutations are never retried:
            // an interrupted response can still mean the host executed the command.
            let result = async { Client::discover().await?.execute(request.command).await }.await;
            let result = match result {
                Ok(state) => {
                    merge_snapshot(&mut command_view.lock().unwrap(), state);
                    Ok(())
                }
                Err(_) => Err("操作未能确认，请检查主机状态后重试".into()),
            };
            if reply_tx
                .send(Reply {
                    id: request.id,
                    result,
                })
                .is_err()
            {
                break;
            }
        }
    });
    let (spectrum_tx, spectrum_rx) = watch::channel(None);
    let audio = Arc::new(Mutex::new(spectrum::Spectrum::default()));
    let spectrum_task = tokio::spawn(spectrum::run(spectrum_rx, audio.clone()));
    let result = tokio::task::spawn_blocking(move || -> Result<()> {
        let mut terminal = ratatui::try_init()?;
        let _restore = Restore;
        execute!(std::io::stdout(), EnableMouseCapture, EnableFocusChange)?;
        let mut ui = Ui::new(Preferences::load());
        let mut sequence = 0u64;
        let mut last_frame = Instant::now() - Duration::from_secs(1);
        let mut last_state = (String::new(), u64::MAX, None);
        let mut dirty = true;
        let mut timing = timing::Trace::new();
        let mut audio_position: (u64, u64);
        loop {
            let (state, connection) = {
                let view = view.lock().unwrap();
                (view.state.clone(), view.connection.clone())
            };
            if state.shutdown_reason.as_deref() == Some("update") {
                break;
            }
            let identity = (
                state.instance_id.clone(),
                state.revision,
                connection.clone(),
            );
            if identity != last_state {
                ui.sync(&state);
                last_state = identity;
                dirty = true;
            }
            while let Ok(reply) = reply_rx.try_recv() {
                ui.pending = ui.pending.saturating_sub(1);
                if ui.pending_volume.is_some_and(|(id, _)| id == reply.id) {
                    ui.pending_volume = None;
                }
                ui.notice = reply.result.err();
                dirty = true;
            }
            let source = ui.spectrum_source(&state, connection.is_none());
            spectrum_tx.send_if_modified(|current| {
                if *current != source {
                    *current = source;
                    true
                } else {
                    false
                }
            });
            {
                let audio = audio.lock().unwrap();
                if audio.instance_id.is_empty() || audio.instance_id == state.instance_id {
                    ui.tick(&state, connection.is_none(), &audio);
                    audio_position = (audio.epoch, audio.end_sample);
                } else {
                    ui.tick(&state, connection.is_none(), &spectrum::Spectrum::default());
                    audio_position = (0, 0);
                }
            }
            let animated = ui.animate(&state, connection.is_none());
            let frame_time = if animated {
                Duration::from_millis(33)
            } else {
                Duration::from_millis(250)
            };
            if dirty || last_frame.elapsed() >= frame_time {
                terminal
                    .draw(|frame| render::draw(frame, &mut ui, &state, connection.as_deref()))?;
                if let Some(timing) = &mut timing {
                    timing.draw(audio_position.0, audio_position.1);
                }
                last_frame = Instant::now();
                dirty = false;
            }
            // Wake at the next frame deadline even when keyboard events arrive.
            let wait = frame_time
                .saturating_sub(last_frame.elapsed())
                .min(Duration::from_millis(33));
            if event::poll(wait)? {
                let event = event::read()?;
                if quit_event(&event) {
                    break;
                }
                let old_hover = ui.hover;
                let action = action(&event, &mut ui);
                if matches!(event, Event::Resize(..)) {
                    dirty = true;
                }
                if let Some(action) = action {
                    dirty = true;
                    match action {
                        Action::Quit => break,
                        Action::Queue => {
                            ui.focus = Action::Queue;
                        }
                        Action::Select(index) => {
                            ui.selected = index.min(ui.tracks.len().saturating_sub(1));
                            ui.focus = Action::Queue;
                            ui.ensure_visible();
                        }
                        Action::Details => {
                            if !ui.tracks.is_empty() {
                                ui.modal(Overlay::Details);
                            }
                        }
                        Action::CurrentDetails => ui.modal(Overlay::CurrentDetails),
                        Action::Help => ui.modal(Overlay::Help),
                        Action::Close => {
                            ui.overlay = None;
                            ui.overlay_scroll = 0;
                            ui.focus = Action::Queue;
                        }
                        Action::Mode(mode) => ui.change_mode(mode),
                        _ => {
                            if let Some(command) = ui.command(action, &state) {
                                sequence += 1;
                                let volume = if let Command::SetVolume { value } = &command {
                                    Some(*value)
                                } else {
                                    None
                                };
                                if command_tx
                                    .try_send(Request {
                                        id: sequence,
                                        command,
                                    })
                                    .is_ok()
                                {
                                    ui.pending += 1;
                                    ui.notice = None;
                                    if let Some(value) = volume {
                                        ui.pending_volume = Some((sequence, value));
                                        ui.pulse_until =
                                            Some(Instant::now() + Duration::from_millis(150));
                                    }
                                } else {
                                    ui.notice = Some("操作较密集，请稍后再试".into());
                                }
                            }
                        }
                    }
                } else if matches!(
                    event,
                    Event::Key(_)
                        | Event::Mouse(event::MouseEvent {
                            kind: MouseEventKind::ScrollUp | MouseEventKind::ScrollDown,
                            ..
                        })
                ) || ui.hover != old_hover
                {
                    dirty = true;
                }
            }
        }
        Ok(())
    })
    .await;
    subscribe_task.abort();
    commands.abort();
    spectrum_task.abort();
    result?
}

fn quit_event(event: &Event) -> bool {
    matches!(event,Event::Key(key) if key.kind!=KeyEventKind::Release && ((key.code==KeyCode::Char('c')&&key.modifiers.contains(KeyModifiers::CONTROL))||key.code==KeyCode::Char('q')))
}
fn action(event: &Event, ui: &mut Ui) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            if ui.overlay.is_some() {
                return match key.code {
                    KeyCode::Esc | KeyCode::Char('?') => Some(Action::Close),
                    KeyCode::Up | KeyCode::Char('k') => {
                        ui.scroll_overlay(-1);
                        None
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        ui.scroll_overlay(1);
                        None
                    }
                    KeyCode::PageUp => {
                        ui.scroll_overlay(-8);
                        None
                    }
                    KeyCode::PageDown => {
                        ui.scroll_overlay(8);
                        None
                    }
                    KeyCode::Enter if ui.focus == Action::Close => Some(Action::Close),
                    _ => None,
                };
            }
            match key.code {
                KeyCode::Tab => {
                    ui.tab(key.modifiers.contains(KeyModifiers::SHIFT));
                    None
                }
                KeyCode::BackTab => {
                    ui.tab(true);
                    None
                }
                KeyCode::Char(' ') => Some(Action::Play),
                KeyCode::Char('s') => Some(Action::Stop),
                KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Right => Some(Action::Up),
                KeyCode::Char('-') | KeyCode::Left => Some(Action::Down),
                KeyCode::Char('a') => Some(Action::SourceNamed("auto")),
                KeyCode::Char('1') => Some(Action::SourceNamed("source1")),
                KeyCode::Char('2') => Some(Action::SourceNamed("source2")),
                KeyCode::Char('m') => Some(Action::Mode(ui.prefs.motion.next())),
                KeyCode::Char('x') => {
                    ui.prefs.ascii = !ui.prefs.ascii;
                    if ui.prefs.save().is_err() {
                        ui.notice = Some("无法保存界面偏好".into());
                    }
                    None
                }
                KeyCode::Char('?') | KeyCode::F(1) => Some(Action::Help),
                KeyCode::Char('r') => Some(Action::Refresh),
                KeyCode::Char('i') => Some(Action::CurrentDetails),
                KeyCode::Char('b') => Some(Action::Quality),
                KeyCode::Enter => Some(if ui.focus == Action::Queue {
                    Action::Details
                } else {
                    ui.focus
                }),
                KeyCode::Up | KeyCode::Char('k') => {
                    ui.select(-1);
                    None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    ui.select(1);
                    None
                }
                KeyCode::PageUp => {
                    ui.select(-(ui.queue_rect.height.max(1) as isize));
                    None
                }
                KeyCode::PageDown => {
                    ui.select(ui.queue_rect.height.max(1) as isize);
                    None
                }
                KeyCode::Home => {
                    ui.select(-(ui.selected as isize));
                    None
                }
                KeyCode::End => {
                    ui.select(ui.tracks.len() as isize);
                    None
                }
                _ => None,
            }
        }
        Event::Mouse(mouse) => {
            let position = Position::new(mouse.column, mouse.row);
            match mouse.kind {
                MouseEventKind::Moved => {
                    ui.pointer = Some(position);
                    ui.hover = ui.hit(position);
                    None
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    ui.pointer = Some(position);
                    let action = ui.hit(position);
                    if ui.overlay.is_some() && action != Some(Action::Close) {
                        return None;
                    }
                    if let Some(action) = action {
                        ui.focus = if matches!(action, Action::Select(_)) {
                            Action::Queue
                        } else {
                            action
                        };
                    }
                    action
                }
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    let delta = if mouse.kind == MouseEventKind::ScrollUp {
                        -3
                    } else {
                        3
                    };
                    if ui.overlay.is_some() {
                        ui.scroll_overlay(delta);
                    } else if ui.queue_rect.contains(position) {
                        ui.select(delta as isize);
                    }
                    None
                }
                // Deliberately ignore every drag and button-release event.
                _ => None,
            }
        }
        _ => None,
    }
}

fn merge_snapshot(view: &mut View, state: Snapshot) {
    if state.instance_id != view.state.instance_id || state.revision >= view.state.revision {
        view.state = state;
    }
}
async fn subscribe(client: &Client, view: &Arc<Mutex<View>>) -> Result<()> {
    let mut request = client
        .endpoint
        .url("/v1/events")
        .replacen("http:", "ws:", 1)
        .into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", client.endpoint.token))?,
    );
    request.headers_mut().insert(
        "X-THMR-Protocol",
        HeaderValue::from_str(&PROTOCOL_VERSION.to_string())?,
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
    while let Some(message) = socket.next().await {
        if let tokio_tungstenite::tungstenite::Message::Text(text) = message? {
            let state: Snapshot = serde_json::from_str(&text)?;
            if state.protocol_version != PROTOCOL_VERSION {
                anyhow::bail!("incompatible host protocol");
            }
            let mut view = view.lock().unwrap();
            merge_snapshot(&mut view, state);
            view.connection = None;
        }
    }
    anyhow::bail!("host disconnected")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mouse_drag_cannot_trigger_a_volume_command() {
        let mut ui = Ui::new(Preferences::default());
        ui.hits
            .push((ratatui::layout::Rect::new(1, 1, 10, 1), Action::Up));
        for kind in [
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            let event = Event::Mouse(event::MouseEvent {
                kind,
                column: 3,
                row: 1,
                modifiers: KeyModifiers::NONE,
            });
            assert_eq!(action(&event, &mut ui), None);
        }
        let event = Event::Mouse(event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: 1,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(action(&event, &mut ui), Some(Action::Up));
    }
    #[test]
    fn older_command_response_cannot_replace_newer_snapshot() {
        let mut state = Snapshot::new("test".into(), &Default::default());
        state.revision = 8;
        let mut view = View {
            state: state.clone(),
            connection: None,
        };
        state.revision = 7;
        state.volume = 2.0;
        merge_snapshot(&mut view, state);
        assert_eq!(view.state.revision, 8);
        assert_ne!(view.state.volume, 2.0);
    }
    #[test]
    fn modal_blocks_playback_actions_and_background_mouse_hits() {
        let mut ui = Ui::new(Preferences::default());
        ui.modal(Overlay::Help);
        let event = Event::Key(event::KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(action(&event, &mut ui), None);
    }
}
