use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use thmr_core::{
    client::Client,
    protocol::{Command, Snapshot},
    PROTOCOL_VERSION,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue};

#[derive(Default)]
struct View {
    state: Option<Snapshot>,
    notice: Option<String>,
}
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

pub async fn run(client: Client) -> Result<()> {
    let view = Arc::new(Mutex::new(View {
        state: Some(client.snapshot().await?),
        notice: None,
    }));
    let reader_view = view.clone();
    let task = tokio::spawn(async move {
        let mut client = client;
        loop {
            if let Err(error) = subscribe(&client, &reader_view).await {
                reader_view.lock().unwrap().notice =
                    Some(format!("连接中断：{error}；正在重新连接"));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            if let Ok(reconnected) = Client::discover().await {
                client = reconnected;
            }
        }
    });
    let input_view = view.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<()> {
        let mut terminal = ratatui::try_init()?;
        let _restore = Restore;
        loop {
            terminal.draw(|frame| draw(frame, &input_view.lock().unwrap()))?;
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    if key.code == KeyCode::Char('q')
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL))
                    {
                        break;
                    }
                    let state = input_view.lock().unwrap().state.clone();
                    let command = match key.code {
                        KeyCode::Char(' ') => Some(Command::Toggle),
                        KeyCode::Char('s') => Some(Command::Stop),
                        KeyCode::Char('r') => Some(Command::Refresh),
                        KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Right => {
                            state.as_ref().map(|s| Command::SetVolume {
                                value: (s.volume + 2.0).min(100.0),
                            })
                        }
                        KeyCode::Char('-') | KeyCode::Left => {
                            state.as_ref().map(|s| Command::SetVolume {
                                value: (s.volume - 2.0).max(0.0),
                            })
                        }
                        KeyCode::Char('1') => Some(Command::SetSource {
                            source: "source1".into(),
                        }),
                        KeyCode::Char('2') => Some(Command::SetSource {
                            source: "source2".into(),
                        }),
                        KeyCode::Char('a') => Some(Command::SetSource {
                            source: "auto".into(),
                        }),
                        KeyCode::Char('b') => state
                            .as_ref()
                            .and_then(next_profile)
                            .map(|profile_id| Command::SetProfile { profile_id }),
                        _ => None,
                    };
                    if let Some(command) = command {
                        let result = tokio::runtime::Handle::current()
                            .block_on(async { Client::discover().await?.execute(command).await });
                        let mut view = input_view.lock().unwrap();
                        match result {
                            Ok(state) => {
                                view.state = Some(state);
                                view.notice = None;
                            }
                            Err(error) => view.notice = Some(error.to_string()),
                        }
                    }
                }
            }
        }
        Ok(())
    })
    .await?;
    task.abort();
    result
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
            let snapshot: Snapshot = serde_json::from_str(&text)?;
            if snapshot.protocol_version != PROTOCOL_VERSION {
                anyhow::bail!("incompatible host protocol");
            }
            let mut view = view.lock().unwrap();
            view.state = Some(snapshot);
            view.notice = None;
        }
    }
    anyhow::bail!("host disconnected")
}
fn next_profile(state: &Snapshot) -> Option<String> {
    let profiles: Vec<&str> = state
        .station_status
        .as_ref()?
        .pointer("/station/bitrates")?
        .as_array()?
        .iter()
        .filter(|p| p.get("enabled").and_then(|v| v.as_bool()) != Some(false))
        .filter_map(|p| p.get("id")?.as_str())
        .collect();
    if profiles.is_empty() {
        return None;
    }
    let current = profiles
        .iter()
        .position(|id| *id == state.profile_id)
        .unwrap_or(0);
    Some(profiles[(current + 1) % profiles.len()].into())
}
fn draw(frame: &mut Frame, view: &View) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(7),
        Constraint::Min(3),
        Constraint::Length(4),
    ])
    .split(frame.area());
    let accent = Color::Rgb(223, 145, 161);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("THMR", Style::new().fg(accent).bold()),
            Span::raw("  东方音乐电台 / Touhou Music Radio"),
        ]))
        .block(Block::default().borders(Borders::ALL)),
        areas[0],
    );
    let Some(state) = &view.state else {
        return;
    };
    let current = state.current();
    let title = current
        .and_then(|c| c.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("等待曲目信息");
    let artist = current
        .and_then(|c| c.get("artist"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let phase = match state.phase {
        thmr_core::protocol::Phase::Idle => {
            if state.user_paused {
                "已暂停"
            } else {
                "已停止"
            }
        }
        thmr_core::protocol::Phase::Online => "正在播放",
        thmr_core::protocol::Phase::Connecting => "连接中",
        thmr_core::protocol::Phase::Buffering => "缓冲中",
        thmr_core::protocol::Phase::Reconnecting => "重连中",
    };
    let now = vec![
        Line::styled(title, Style::new().fg(accent).bold()),
        Line::from(artist),
        Line::from(""),
        Line::from(format!(
            "{phase}  ·  {} / {}  ·  音量 {:.0}%  ·  已收听 {}:{:02}",
            state.source,
            state.profile_id,
            state.volume,
            state.listened_sec / 60,
            state.listened_sec % 60
        )),
    ];
    frame.render_widget(
        Paragraph::new(now)
            .wrap(Wrap { trim: false })
            .block(Block::default().title(" 正在放送 ").borders(Borders::ALL)),
        areas[1],
    );
    let queue: Vec<ListItem> = state
        .station_status
        .as_ref()
        .and_then(|s| s.get("queue"))
        .and_then(|q| q.as_array())
        .into_iter()
        .flatten()
        .take(30)
        .map(|track| {
            ListItem::new(format!(
                "{}  —  {}",
                track.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                track.get("artist").and_then(|v| v.as_str()).unwrap_or("")
            ))
        })
        .collect();
    frame.render_widget(
        List::new(queue).block(
            Block::default()
                .title(" 电台排播（只读） ")
                .borders(Borders::ALL),
        ),
        areas[2],
    );
    let notice = view
        .notice
        .as_ref()
        .or(state.error.as_ref())
        .or(state.metadata_error.as_ref())
        .map(String::as_str)
        .unwrap_or("q 退出界面，后台播放继续");
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("空格 播放/暂停  s 停止  +/- 音量  1/2/a 来源  b 码率  r 刷新"),
            Line::styled(notice, Style::new().fg(accent)),
        ])
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL)),
        areas[3],
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_renders_chinese_and_handles_small_windows() {
        use ratatui::{backend::TestBackend, Terminal};
        let view = View {
            state: Some(Snapshot::new("test".into(), &Default::default())),
            notice: None,
        };
        for (w, h) in [(100, 30), (30, 8), (10, 3)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| draw(f, &view)).unwrap();
        }
    }
}
