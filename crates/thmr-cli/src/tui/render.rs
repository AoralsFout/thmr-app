use super::model::{text, Action, Motion, Overlay, Track, Ui};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use thmr_core::protocol::{Phase, Snapshot};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const BG: Color = Color::Rgb(16, 21, 34);
const FG: Color = Color::Rgb(227, 231, 239);
const SUB: Color = Color::Rgb(168, 179, 198);
const EDGE: Color = Color::Rgb(52, 64, 86);
const CYAN: Color = Color::Rgb(131, 227, 219);
const VIOLET: Color = Color::Rgb(193, 162, 238);
const BRIGHT: Color = Color::Rgb(245, 251, 255);
const ERROR: Color = Color::Rgb(255, 164, 168);
const MIN_CONTROL_WIDTH: u16 = 38;
const MIN_SPECTRUM_WIDTH: u16 = 40;

struct BodyAreas {
    meter: Option<Rect>,
    queue: Rect,
    stage: Rect,
    split: Option<u16>,
}

fn body_layout(area: Rect) -> BodyAreas {
    let (meter, content) = if area.width >= 110 && area.height >= 14 {
        let columns = Layout::horizontal([Constraint::Length(12), Constraint::Min(80)]).split(area);
        (Some(columns[0]), columns[1])
    } else {
        (None, area)
    };
    if content.width >= 80 {
        let columns = Layout::horizontal([Constraint::Percentage(31), Constraint::Percentage(69)])
            .split(content);
        BodyAreas {
            meter,
            queue: columns[0],
            stage: columns[1],
            split: Some(columns[1].x),
        }
    } else {
        let rows = Layout::vertical([
            Constraint::Min(4),
            Constraint::Length(if content.height >= 20 { 6 } else { 4 }),
        ])
        .split(content);
        BodyAreas {
            meter,
            queue: rows[1],
            stage: rows[0],
            split: None,
        }
    }
}

fn footer_can_align(area: Rect, split: Option<u16>) -> bool {
    split.is_some_and(|x| {
        x.saturating_sub(area.x) >= MIN_CONTROL_WIDTH
            && area.right().saturating_sub(x) >= MIN_SPECTRUM_WIDTH
    })
}

fn style(color: Color) -> Style {
    Style::new().fg(color).bg(BG)
}
fn inside(area: Rect) -> Rect {
    area.inner(ratatui::layout::Margin::new(1, 1))
}
fn row(area: Rect, offset: u16) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(offset),
        area.width,
        if offset < area.height { 1 } else { 0 },
    )
}
fn fit(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for grapheme in value.graphemes(true) {
        let w = UnicodeWidthStr::width(grapheme);
        if used + w + 1 > width {
            break;
        }
        result.push_str(grapheme);
        used += w;
    }
    result.push('~');
    result
}
fn wrap_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![];
    }
    let mut result = vec![];
    for line in lines {
        let mut current = String::new();
        let mut used = 0;
        for glyph in line.to_string().graphemes(true) {
            let cells = UnicodeWidthStr::width(glyph);
            if used + cells > width && !current.is_empty() {
                result.push(Line::styled(std::mem::take(&mut current), line.style));
                used = 0;
            }
            current.push_str(glyph);
            used += cells;
        }
        result.push(Line::styled(current, line.style));
    }
    result
}
fn block(title: &str, focused: bool) -> Block<'_> {
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(style(if focused { CYAN } else { EDGE }))
        .style(style(FG))
}

pub fn draw(frame: &mut Frame, ui: &mut Ui, state: &Snapshot, connection: Option<&str>) {
    let area = frame.area();
    frame.render_widget(Block::default().style(style(FG)), area);
    ui.hits.clear();
    ui.focusables.clear();
    ui.queue_rect = Rect::default();
    let minimum_height = if ui.prefs.motion == Motion::Static {
        14
    } else {
        16
    };
    if area.width < 38 || area.height < minimum_height {
        compact(frame, ui, state, area, connection);
        return;
    }
    if ui.prefs.motion == Motion::Static {
        static_view(frame, ui, state, area, connection);
    } else {
        dynamic_view(frame, ui, state, area, connection);
    }
    if ui.overlay.is_some() {
        overlay(frame, ui, state, area);
    }
    if !ui.focusables.contains(&ui.focus) {
        ui.focus = ui.focusables.first().copied().unwrap_or(Action::Queue);
    }
    ui.hover = ui.pointer.and_then(|p| ui.hit(p));
}

fn header_lines(state: &Snapshot, connection: Option<&str>) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled(" THMR", style(CYAN).bold()),
            Span::styled(" // GENSOKYO SIGNAL", style(SUB)),
        ]),
        Line::from(format!(
            " 东方音乐电台  /  {}{}  /  已收听 {}:{:02}",
            phase(state, connection.is_none()),
            if state.volume <= 0.0 {
                " · 静音"
            } else {
                ""
            },
            state.listened_sec / 60,
            state.listened_sec % 60
        )),
    ]
}

fn static_view(
    frame: &mut Frame,
    ui: &mut Ui,
    state: &Snapshot,
    area: Rect,
    connection: Option<&str>,
) {
    let sections = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(3),
        Constraint::Length(7),
    ])
    .split(area);
    let title = text(state.current().and_then(|track| track.get("title")));
    let artist = text(state.current().and_then(|track| track.get("artist")));
    let current = if title.is_empty() {
        " 等待曲目信息".to_owned()
    } else if artist.is_empty() {
        format!(" {title}")
    } else {
        format!(" {title} / {artist}")
    };
    let mut lines = header_lines(state, connection);
    let title_style =
        if ui.focus == Action::CurrentDetails || ui.hover == Some(Action::CurrentDetails) {
            style(CYAN).bold().underlined()
        } else {
            style(CYAN).bold()
        };
    lines.push(Line::styled(
        fit(&current, area.width as usize),
        title_style,
    ));
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(style(EDGE)),
        ),
        sections[0],
    );
    queue(frame, ui, state, sections[1], connection.is_none());
    ui.hits.push((row(sections[0], 2), Action::CurrentDetails));
    ui.focusables.push(Action::CurrentDetails);
    let footer = sections[2];
    controls(
        frame,
        ui,
        state,
        Rect::new(footer.x, footer.y, footer.width, 5),
    );
    footer_hints(
        frame,
        ui,
        state,
        Rect::new(footer.x, footer.y + 5, footer.width, 2),
        connection,
    );
}

fn dynamic_view(
    frame: &mut Frame,
    ui: &mut Ui,
    state: &Snapshot,
    area: Rect,
    connection: Option<&str>,
) {
    // First evaluate the upper columns with a compact footer; its real divider
    // includes the optional channel meter and becomes the footer divider too.
    let candidate_body = body_layout(Rect::new(
        area.x,
        area.y + 3,
        area.width,
        area.height.saturating_sub(11),
    ));
    let footer_height = if footer_can_align(area, candidate_body.split) {
        6
    } else {
        5 + area.height.saturating_sub(14).clamp(2, 5)
    };
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(footer_height),
        Constraint::Length(2),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(header_lines(state, connection)).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(style(EDGE)),
        ),
        sections[0],
    );
    let body = body_layout(sections[1]);
    if let Some(meter) = body.meter {
        channel_meter(frame, ui, meter);
    }
    // Preserve the existing focus order for horizontal and vertical arrangements.
    if body.split.is_some() {
        queue(frame, ui, state, body.queue, connection.is_none());
        now_playing(frame, ui, state, body.stage, connection.is_none());
    } else {
        now_playing(frame, ui, state, body.stage, connection.is_none());
        queue(frame, ui, state, body.queue, connection.is_none());
    }
    let (control_area, spectrum_area) = footer_layout(sections[2], body.split);
    controls(frame, ui, state, control_area);
    footer_spectrum(frame, ui, spectrum_area);
    footer_hints(frame, ui, state, sections[3], connection);
}

fn footer_hints(
    frame: &mut Frame,
    ui: &Ui,
    state: &Snapshot,
    area: Rect,
    connection: Option<&str>,
) {
    let notice = ui
        .notice
        .as_deref()
        .or(connection)
        .or(state.error.as_deref())
        .or(state.metadata_error.as_deref());
    let hint = ui.hover.unwrap_or(ui.focus).hint();
    let lines = vec![
        Line::styled(
            fit(notice.unwrap_or(hint), area.width as usize),
            style(if notice.is_some() { ERROR } else { SUB }),
        ),
        Line::styled(
            fit(
                "Tab 切换焦点 · ↑↓ 浏览 · Enter 详情 · ? 帮助 · q 退出，后台继续",
                area.width as usize,
            ),
            style(SUB),
        ),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}
fn phase(state: &Snapshot, connected: bool) -> &'static str {
    if !connected {
        return "主机重连中";
    }
    match state.phase {
        Phase::Online => "正在放送",
        Phase::Idle if state.user_paused => "已暂停",
        Phase::Idle => "已停止",
        Phase::Connecting => "连接中",
        Phase::Buffering => "缓冲中",
        Phase::Reconnecting => "电台重连中",
    }
}

fn channel_meter(frame: &mut Frame, ui: &Ui, area: Rect) {
    frame.render_widget(block("dBFS", false), area);
    let inner = inside(area);
    let plot = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(2),
    );
    for (label, offset) in [("L", 4), ("R", 7)] {
        frame.render_widget(
            Paragraph::new(label).style(style(SUB)),
            Rect::new(inner.x + offset, inner.y, 2, 1),
        );
    }
    for db in [0, -6, -12, -24, -36, -48, -60] {
        let y =
            plot.y + ((-db) as f32 / 60.0 * plot.height.saturating_sub(1) as f32).round() as u16;
        frame.render_widget(
            Paragraph::new(format!("{db:>3}")).style(style(SUB)),
            Rect::new(plot.x, y, 3, 1),
        );
    }
    let dbfs = |amplitude: f32| 20.0 * amplitude.max(0.000001).log10();
    let partial = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    for channel in 0..2 {
        let height =
            ((dbfs(ui.channel_rms[channel]) + 60.0) / 60.0).clamp(0.0, 1.0) * plot.height as f32;
        let x = plot.x + 4 + channel as u16 * 3;
        for y in 0..plot.height {
            let amount = (height - y as f32).clamp(0.0, 1.0);
            let db = -60.0 + (y + 1) as f32 / plot.height as f32 * 60.0;
            let color = if db > -3.0 {
                ERROR
            } else if db > -12.0 {
                Color::Rgb(255, 207, 119)
            } else {
                CYAN
            };
            let symbol = if ui.prefs.ascii {
                if amount > 0.0 {
                    "|"
                } else {
                    ":"
                }
            } else if amount > 0.0 {
                partial[(amount * 8.0).round() as usize]
            } else {
                "░"
            };
            for dx in 0..2 {
                frame.buffer_mut()[(x + dx, plot.bottom() - 1 - y)]
                    .set_symbol(symbol)
                    .set_style(style(if amount > 0.0 { color } else { EDGE }));
            }
        }
        if ui.channel_peaks[channel] > 0.001 {
            let peak = ((dbfs(ui.channel_peaks[channel]) + 60.0) / 60.0).clamp(0.0, 1.0);
            let y =
                plot.bottom() - 1 - (peak * plot.height.saturating_sub(1) as f32).round() as u16;
            for dx in 0..2 {
                frame.buffer_mut()[(x + dx, y)]
                    .set_symbol(if ui.prefs.ascii { "-" } else { "─" })
                    .set_style(style(if ui.channel_peaks[channel] >= 0.999 {
                        ERROR
                    } else {
                        BRIGHT
                    }));
            }
        }
    }
    let reading = |amplitude: f32| {
        if amplitude < 0.001 {
            " --".to_owned()
        } else {
            format!("{:>3.0}", dbfs(amplitude))
        }
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{} /{}",
            reading(ui.channel_rms[0]),
            reading(ui.channel_rms[1])
        ))
        .style(style(SUB)),
        row(inner, inner.height - 1),
    );
}

fn queue(frame: &mut Frame, ui: &mut Ui, state: &Snapshot, area: Rect, connected: bool) {
    if area.width == 0 || area.height < 2 {
        return;
    }
    let border = block("电台排播", ui.focus == Action::Queue);
    let inner = border.inner(area);
    frame.render_widget(border, area);
    ui.queue_rect = inner;
    ui.ensure_visible();
    ui.focusables.push(Action::Queue);
    ui.hits.push((area, Action::Queue));
    if ui.tracks.is_empty() {
        frame.render_widget(Paragraph::new("等待排播信息").style(style(SUB)), inner);
        return;
    }
    let blink = ui.prefs.motion == Motion::Dynamic
        && ui.active(state, connected)
        && state.volume > 0.0
        && ui.overlay.is_none();
    let brightness = if blink {
        0.825
            + 0.175 * (ui.title_changed.elapsed().as_secs_f64() * std::f64::consts::TAU / 1.2).cos()
    } else {
        1.0
    };
    let note_color = match CYAN {
        Color::Rgb(r, g, b) => Color::Rgb(
            (f64::from(r) * brightness).round() as u8,
            (f64::from(g) * brightness).round() as u8,
            (f64::from(b) * brightness).round() as u8,
        ),
        color => color,
    };
    let current_count = usize::from(ui.tracks[0].is_current);
    for (index, track) in ui
        .tracks
        .iter()
        .enumerate()
        .skip(ui.offset)
        .take(inner.height as usize)
    {
        let rect = row(inner, (index - ui.offset) as u16);
        let selected = index == ui.selected;
        let marker = if selected {
            if ui.prefs.ascii {
                ">"
            } else {
                "▸"
            }
        } else {
            " "
        };
        let label = if track.is_current {
            if ui.prefs.ascii { "* " } else { "♪ " }.to_owned()
        } else {
            format!("{:02}", index + 1 - current_count)
        };
        let line = format!("{marker} {label} {}", track.title);
        let hovered = ui.hover == Some(Action::Select(index));
        frame.render_widget(
            Paragraph::new(fit(&line, rect.width as usize)).style(if selected || hovered {
                style(CYAN).bg(EDGE)
            } else {
                style(FG)
            }),
            rect,
        );
        // Pulse only the note's foreground, preserving the glyph and selection background.
        if track.is_current && rect.width > 2 {
            frame.buffer_mut()[(rect.x + 2, rect.y)].set_fg(note_color);
        }
        ui.hits.push((rect, Action::Select(index)));
    }
    if ui.tracks.len() > inner.height as usize && inner.height > 0 {
        let label = format!(" {}/{} ", ui.selected + 1, ui.tracks.len());
        let width = UnicodeWidthStr::width(label.as_str()) as u16;
        if width + 2 < area.width {
            frame.render_widget(
                Paragraph::new(label).style(style(SUB)),
                Rect::new(area.right() - width - 1, area.bottom() - 1, width, 1),
            );
        }
    }
}
fn stage_audio_lines(
    ui: &Ui,
    state: &Snapshot,
    connected: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let metrics = ui.stage.audio_metrics();
    let status = if ui.prefs.motion == Motion::Static {
        "STATIC"
    } else if metrics.is_none() {
        "WAIT"
    } else if !connected {
        "HOLD:LINK"
    } else if !ui.active(state, connected) {
        "HOLD:STOP"
    } else if state.volume <= 0.0 {
        "HOLD:MUTE"
    } else if ui.overlay.is_some() {
        "HOLD:MODAL"
    } else if !ui.available {
        "HOLD:STALE"
    } else {
        "LIVE"
    };
    let mut lines = vec![Line::styled(
        fit(&format!("AUDIO {status}"), width),
        style(CYAN),
    )];
    if ui.prefs.motion == Motion::Static {
        return lines;
    }
    let Some((music, strength)) = metrics else {
        return lines;
    };
    let fields = [
        format!("LVL {:.3}", music.level),
        format!("INT {:.3}", music.intensity),
        format!("DRV {:.3}", strength),
        format!("LO {:.3}", music.low),
        format!("MID {:.3}", music.mid),
        format!("HI {:.3}", music.high),
        format!("BP {:.3}s", music.beat_period),
        format!("BC {:.3}", music.beat_confidence),
        format!("BT {}", music.beats),
        format!("ACC {}", music.accents),
        format!("SRG {}", music.surges),
    ];
    let mut line = String::new();
    for field in fields {
        let field = fit(&field, width);
        if !line.is_empty()
            && UnicodeWidthStr::width(line.as_str()) + 2 + UnicodeWidthStr::width(field.as_str())
                > width
        {
            lines.push(Line::styled(std::mem::take(&mut line), style(SUB)));
        }
        if !line.is_empty() {
            line.push_str("  ");
        }
        line.push_str(&field);
    }
    if !line.is_empty() {
        lines.push(Line::styled(line, style(SUB)));
    }
    lines
}

fn now_playing(frame: &mut Frame, ui: &mut Ui, state: &Snapshot, area: Rect, connected: bool) {
    if area.height < 2 {
        return;
    }
    let inner = inside(area);
    frame.render_widget(block("正在放送", false), area);
    let content_height = inner.height;
    let audio_lines = stage_audio_lines(ui, state, connected, inner.width as usize);
    // Keep room for the title, artist and on-air label in short windows.
    let metrics_height = if content_height >= audio_lines.len() as u16 + 4 {
        audio_lines.len() as u16
    } else {
        0
    };
    let waveform_height = if ui.prefs.motion == Motion::Static {
        0
    } else {
        (content_height - metrics_height).saturating_sub(6).min(3)
    };
    let stage_height = content_height - metrics_height - waveform_height;
    let stage_area = Rect::new(inner.x, inner.y, inner.width, stage_height);
    ui.stage.viewport(stage_area.width, stage_area.height);
    ui.poster.viewport(stage_area.width, stage_area.height);
    let still = ui.prefs.motion == Motion::Static || !ui.stage.started;
    let stage = if still { &ui.poster } else { &ui.stage };
    super::battlefield::draw(frame, stage_area, stage, ui.prefs.ascii, still);
    let title = text(state.current().and_then(|v| v.get("title")));
    let title = if title.is_empty() {
        "等待曲目信息"
    } else {
        title.as_str()
    };
    let artist = text(state.current().and_then(|v| v.get("artist")));
    let width = inner.width as usize;
    let elapsed = ui.title_changed.elapsed().as_secs_f64();
    let age = ui.clock % 10.0;
    let flow = ui.prefs.motion != Motion::Static
        && ui.overlay.is_none()
        && (2.0..2.8).contains(&age)
        && state.phase == Phase::Online
        && state.volume > 0.0
        && ui.available;
    let glyphs: Vec<&str> = title.graphemes(true).collect();
    let title_width = UnicodeWidthStr::width(title);
    let overflow = title_width.saturating_sub(width);
    // A long title rests first, scrolls on display-cell boundaries, then rests at its end.
    let skip = if ui.prefs.motion == Motion::Static || overflow == 0 {
        0
    } else {
        let period = overflow as f64 / 5.0 + 6.0;
        ((ui.clock % period - 3.0).max(0.0) * 5.0).min(overflow as f64) as usize
    };
    let mut cell = 0;
    let mut visible = 0;
    let mut spans = vec![];
    for (i, glyph) in glyphs.iter().enumerate() {
        let w = UnicodeWidthStr::width(*glyph);
        let start = cell;
        cell += w;
        if start < skip {
            continue;
        }
        if visible + w > width {
            break;
        }
        visible += w;
        let center = (age - 2.0) / 0.8 * (glyphs.len() + 4) as f64 - 2.0;
        let focused =
            ui.focus == Action::CurrentDetails || ui.hover == Some(Action::CurrentDetails);
        spans.push(Span::styled(
            (*glyph).to_owned(),
            style(if flow && (i as f64 - center).abs() < 2.0 {
                BRIGHT
            } else {
                CYAN
            })
            .bold()
            .patch(if focused {
                Style::new().underlined()
            } else {
                Style::default()
            }),
        ));
    }
    super::battlefield::label(
        frame,
        row(inner, 0),
        Line::from(spans),
        visible.saturating_add(2).min(width) as u16,
    );
    ui.hits.push((
        Rect::new(inner.x, inner.y, inner.width, inner.height.min(2)),
        Action::CurrentDetails,
    ));
    ui.focusables.push(Action::CurrentDetails);
    if inner.height > 1 {
        let artist = fit(&artist, width);
        let artist_width = UnicodeWidthStr::width(artist.as_str()) as u16 + 2;
        super::battlefield::label(
            frame,
            row(inner, 1),
            Line::styled(artist, style(SUB)),
            artist_width,
        );
    }
    if inner.height > 2 {
        let reveal = ui.prefs.motion != Motion::Static && elapsed < 0.28 && ui.overlay.is_none();
        let count = if reveal {
            (elapsed / 0.28 * 16.0) as usize
        } else {
            16
        };
        let line = if ui.prefs.ascii { "-" } else { "─" };
        let on_air = match state.phase {
            Phase::Online => "ON AIR",
            Phase::Idle if state.user_paused => "PAUSED",
            Phase::Idle => "OFF AIR",
            _ => "CONNECTING",
        };
        let label = format!(
            "{} {on_air} {}",
            line.repeat(count.min(3)),
            line.repeat(count.min(3))
        );
        let label_width = UnicodeWidthStr::width(label.as_str()) as u16;
        let label_area = Rect::new(
            inner.x + inner.width.saturating_sub(label_width) / 2,
            inner.y + 2,
            label_width.min(inner.width),
            1,
        );
        super::battlefield::label(
            frame,
            label_area,
            Line::styled(label, style(VIOLET)),
            label_width,
        );
    }

    if waveform_height > 0 {
        waveform(
            frame,
            ui,
            Rect::new(
                inner.x,
                inner.y + stage_height,
                inner.width,
                waveform_height,
            ),
        );
    }
    if metrics_height > 0 {
        frame.render_widget(
            Paragraph::new(audio_lines).style(style(SUB)),
            Rect::new(
                inner.x,
                inner.y + stage_height + waveform_height,
                inner.width,
                metrics_height,
            ),
        );
    }
}

fn waveform_range(wave: &super::spectrum::Waveform, point: usize, points: usize) -> (f32, f32) {
    let length = wave.min.len();
    let start = (point * length / points).min(length - 1);
    let end = ((point + 1) * length)
        .div_ceil(points)
        .max(start + 1)
        .min(length);
    (
        wave.min[start..end].iter().copied().fold(1.0, f32::min),
        wave.max[start..end].iter().copied().fold(-1.0, f32::max),
    )
}

fn waveform(frame: &mut Frame, ui: &Ui, area: Rect) {
    use ratatui::{
        symbols::Marker,
        widgets::canvas::{Canvas, Line as Stroke},
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(style(SUB)), area);
    let middle = row(area, area.height / 2);
    let Some(wave) = ui.waveform.as_ref() else {
        frame.render_widget(
            Paragraph::new(fit("WAVE WAIT", area.width as usize)).style(style(SUB)),
            middle,
        );
        return;
    };
    frame.render_widget(Paragraph::new("WAVE").style(style(SUB)), middle);
    if area.width <= 6 {
        return;
    }
    let plot = Rect::new(area.x + 6, area.y, area.width - 6, area.height);
    if ui.prefs.ascii {
        for x in 0..plot.width {
            frame.buffer_mut()[(plot.x + x, middle.y)]
                .set_symbol("-")
                .set_style(style(EDGE));
            let (min, max) = waveform_range(wave, x as usize, plot.width as usize);
            let y = |value: f32| {
                ((1.0 - value) * 0.5 * plot.height.saturating_sub(1) as f32).round() as u16
            };
            for offset in y(max)..=y(min) {
                frame.buffer_mut()[(plot.x + x, plot.y + offset)]
                    .set_symbol(if min == 0.0 && max == 0.0 { "-" } else { "|" })
                    .set_style(style(CYAN));
            }
        }
    } else {
        let points = plot.width as usize * 2;
        frame.render_widget(
            Canvas::default()
                .background_color(BG)
                .marker(Marker::Braille)
                .x_bounds([0.0, (points - 1) as f64])
                .y_bounds([-1.0, 1.0])
                .paint(|context| {
                    context.draw(&Stroke {
                        x1: 0.0,
                        y1: 0.0,
                        x2: (points - 1) as f64,
                        y2: 0.0,
                        color: EDGE,
                    });
                    for point in 0..points {
                        let (min, max) = waveform_range(wave, point, points);
                        context.draw(&Stroke {
                            x1: point as f64,
                            y1: min as f64,
                            x2: point as f64,
                            y2: max as f64,
                            color: CYAN,
                        });
                    }
                }),
            plot,
        );
    }
}

fn spectrum_caption(ui: &Ui) -> String {
    if ui.prefs.motion == Motion::Static {
        "频谱 · 静态".into()
    } else if !ui.available {
        format!("频谱 · {}", ui.spectrum_status)
    } else {
        format!(
            "频谱 40 Hz{}{:.1} kHz · {}",
            if ui.prefs.ascii { "-" } else { "–" },
            ui.spectrum_max_hz / 1000.0,
            ui.stage.label()
        )
    }
}

fn footer_layout(area: Rect, split: Option<u16>) -> (Rect, Rect) {
    if footer_can_align(area, split) {
        let x = split.unwrap();
        (
            Rect::new(area.x, area.y, x - area.x, area.height),
            Rect::new(x, area.y, area.right() - x, area.height),
        )
    } else {
        let rows = Layout::vertical([Constraint::Length(5), Constraint::Min(2)]).split(area);
        (rows[0], rows[1])
    }
}

fn footer_spectrum(frame: &mut Frame, ui: &Ui, area: Rect) {
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(style(EDGE))
            .title(Line::styled(
                fit(&format!(" {} ", spectrum_caption(ui)), area.width as usize),
                style(SUB),
            )),
        area,
    );
    spectrum(
        frame,
        ui,
        Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        ),
    );
}

fn spectrum(frame: &mut Frame, ui: &Ui, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let plot = area;
    frame.render_widget(Clear, plot);
    frame.render_widget(Block::default().style(style(SUB)), plot);
    if ui.prefs.motion == Motion::Static {
        frame.render_widget(
            Paragraph::new("频谱已关闭")
                .alignment(Alignment::Center)
                .style(style(SUB)),
            row(plot, plot.height / 2),
        );
    } else {
        // Whole-cell bars and one-cell gaps avoid alternating widths after rounding.
        // Reduce the displayed band count as needed and center the unused columns.
        let width = plot.width as usize;
        let stride = (width + 1).div_ceil(ui.bands.len()).max(2);
        let count = ((width + 1) / stride).max(1);
        let bar_width = (stride - 1).min(width);
        let padding = (width - (count * stride - 1)) / 2;
        let partial = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
        for b in 0..count {
            let left = padding + b * stride;
            let bar_end = left + bar_width;
            let start = b * ui.bands.len() / count;
            let end = ((b + 1) * ui.bands.len() / count).max(start + 1);
            let value = ui.bands[start..end].iter().copied().fold(0.0, f32::max);
            let peak = ui.peaks[start..end].iter().copied().fold(0.0, f32::max);
            let height = value * plot.height as f32;
            let color = if b * 3 < count * 2 { CYAN } else { VIOLET };
            for y in 0..plot.height {
                let level = (height - y as f32).clamp(0.0, 1.0);
                let symbol = if ui.prefs.ascii {
                    if level > 0.0 {
                        "|"
                    } else {
                        " "
                    }
                } else {
                    partial[(level * 8.0).round() as usize]
                };
                for x in left..bar_end {
                    frame.buffer_mut()[(plot.x + x as u16, plot.bottom() - 1 - y)]
                        .set_symbol(symbol)
                        .set_style(style(color));
                }
            }
            if plot.height >= 3 && peak > 0.015 {
                let y = plot
                    .bottom()
                    .saturating_sub((peak * plot.height as f32).ceil().max(1.0) as u16)
                    .max(plot.y);
                for x in left..bar_end {
                    frame.buffer_mut()[(plot.x + x as u16, y)]
                        .set_symbol(if ui.prefs.ascii { "-" } else { "─" })
                        .set_style(style(BRIGHT));
                }
            }
        }
    }
    if !ui.available && ui.prefs.motion != Motion::Static {
        frame.render_widget(
            Paragraph::new(fit(ui.spectrum_status, area.width as usize))
                .alignment(Alignment::Center)
                .style(style(SUB)),
            row(area, 0),
        );
    }
}
fn button(
    frame: &mut Frame,
    ui: &mut Ui,
    rect: Rect,
    label: &str,
    action: Action,
    selected: bool,
    disabled: bool,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let focused = ui.focus == action;
    let hovered = ui.hover == Some(action);
    let s = if disabled {
        style(SUB)
    } else if focused || hovered {
        style(BRIGHT).bg(EDGE)
    } else if selected {
        style(CYAN).bold()
    } else {
        style(SUB)
    };
    frame.render_widget(
        Paragraph::new(fit(label, rect.width as usize)).style(s),
        rect,
    );
    if !disabled {
        ui.hits.push((rect, action));
        ui.focusables.push(action);
    }
}
fn buttons(frame: &mut Frame, ui: &mut Ui, area: Rect, items: &[(&str, Action, bool, bool)]) {
    let mut x = area.x;
    for (label, action, selected, disabled) in items {
        let width = (UnicodeWidthStr::width(*label) as u16).min(area.right().saturating_sub(x));
        if width < 3 {
            break;
        }
        button(
            frame,
            ui,
            Rect::new(x, area.y, width, 1),
            label,
            *action,
            *selected,
            *disabled,
        );
        x = x.saturating_add(width + 1);
    }
}
fn controls(frame: &mut Frame, ui: &mut Ui, state: &Snapshot, area: Rect) {
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(style(EDGE)),
        area,
    );
    let play = if state.phase == Phase::Idle {
        "[播放]"
    } else {
        "[暂停]"
    };
    let volume = ui.volume(state);
    let volume_label = format!(
        " {:3.0}%{}",
        volume,
        if ui.pending_volume.is_some() {
            "*"
        } else {
            " "
        }
    );
    let prefix = UnicodeWidthStr::width(play)
        + 1
        + UnicodeWidthStr::width("[停止]")
        + 1
        + UnicodeWidthStr::width("[↓ 降低]")
        + 1;
    if prefix + volume_label.len() + 12 < area.width as usize {
        buttons(
            frame,
            ui,
            row(area, 1),
            &[
                (play, Action::Play, true, false),
                ("[停止]", Action::Stop, false, false),
                ("[↓ 降低]", Action::Down, false, volume <= 0.0),
            ],
        );
        let rect = Rect::new(area.x + prefix as u16, area.y + 1, 7, 1);
        frame.render_widget(
            Paragraph::new(volume_label).style(style(
                if ui
                    .pulse_until
                    .is_some_and(|t| t > std::time::Instant::now())
                {
                    BRIGHT
                } else {
                    FG
                },
            )),
            rect,
        );
        button(
            frame,
            ui,
            Rect::new(rect.right() + 1, rect.y, 10, 1),
            "[↑ 提高]",
            Action::Up,
            false,
            volume >= 100.0,
        );
    } else {
        // This shorter row still exposes both volume actions on a narrow terminal.
        buttons(
            frame,
            ui,
            row(area, 1),
            &[
                (play, Action::Play, true, false),
                ("[停止]", Action::Stop, false, false),
                ("[-]", Action::Down, false, volume <= 0.0),
                ("[+]", Action::Up, false, volume >= 100.0),
            ],
        );
        let label = format!(" {:0.0}%", volume);
        let width = label.len() as u16;
        frame.render_widget(
            Paragraph::new(label).style(style(FG)),
            Rect::new(
                area.right().saturating_sub(width),
                area.y + 1,
                width.min(area.width),
                1,
            ),
        );
    }
    let source = format!(
        "[音源:{}]",
        match state.source.as_str() {
            "source1" => "1",
            "source2" => "2",
            _ => "自动",
        }
    );
    let quality = format!(
        "[音质:{}]",
        text(Some(&serde_json::Value::String(state.profile_id.clone())))
    );
    buttons(
        frame,
        ui,
        row(area, 2),
        &[
            (&source, Action::Source, false, false),
            (&quality, Action::Quality, false, false),
            ("[刷新]", Action::Refresh, false, false),
        ],
    );
    buttons(
        frame,
        ui,
        row(area, 3),
        &[
            (
                "[静态]",
                Action::Mode(Motion::Static),
                ui.prefs.motion == Motion::Static,
                false,
            ),
            (
                "[动态]",
                Action::Mode(Motion::Dynamic),
                ui.prefs.motion == Motion::Dynamic,
                false,
            ),
            ("[详情]", Action::Details, false, ui.tracks.is_empty()),
            ("[帮助]", Action::Help, false, false),
            ("[退出]", Action::Quit, false, false),
        ],
    );
    let summary = ui
        .tracks
        .get(ui.selected)
        .map(|t| format!("选中：{} / {}", t.title, t.artist))
        .unwrap_or_else(|| "排播 · m 切换动效 · x 基础字符".into());
    frame.render_widget(
        Paragraph::new(fit(&summary, area.width as usize)).style(style(SUB)),
        row(area, 4),
    );
}
fn clear_popup_overhang(frame: &mut Frame, rect: Rect) {
    // A glyph starting outside Clear's rectangle can still occupy the left border cell.
    // Remove that whole visible prefix so terminal diffing cannot skip the border as
    // a continuation cell of a double-width background character.
    for y in rect.y..rect.bottom() {
        let mut x = frame.area().x;
        while x < rect.x {
            let cells = UnicodeWidthStr::width(frame.buffer_mut()[(x, y)].symbol()).max(1) as u16;
            if x.saturating_add(cells) > rect.x {
                for column in x..rect.x {
                    frame.buffer_mut()[(column, y)].set_symbol(" ");
                }
            }
            x = x.saturating_add(cells);
        }
    }
}

fn overlay(frame: &mut Frame, ui: &mut Ui, state: &Snapshot, area: Rect) {
    let width = area.width.saturating_sub(4).min(76);
    let height = area.height.saturating_sub(2).min(22);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    clear_popup_overhang(frame, rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(Block::default().style(style(FG)), rect);
    let title = if ui.overlay == Some(Overlay::Help) {
        "使用帮助"
    } else if ui.overlay == Some(Overlay::CurrentDetails) {
        "正在放送 · 曲目信息"
    } else {
        "排播详情"
    };
    let inner = inside(rect);
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let lines: Vec<Line> = if ui.overlay == Some(Overlay::Help) {
        [
            "Space 播放 / 暂停 · s 停止 · q 退出界面，后台继续",
            "Tab / Shift+Tab 切换焦点 · Enter 激活按钮 / 查看详情",
            "↑↓ 或 j/k 浏览排播 · PageUp/PageDown 翻页 · Home/End 首尾",
            "鼠标点击按钮和曲目 · 排播区域滚轮浏览",
            "+ / - 调整音量，每次 2% · 不使用拖动操作",
            "a 自动音源 · 1/2 指定音源 · b 切换音质 · r 刷新",
            "i 查看正在放送的曲目信息，也可点击当前曲名",
            "m 切换动效：动态 / 静态 · x 切换基础字符",
            "静态：播放信息 / 排播列表 / 控制，隐藏全部音频可视化",
            "动态：8 种队形、12 种构图、5 种节奏独立组合，避免近期重复",
            "重拍由部分敌机领奏，高频短促插入，渐强可提前变奏",
            "暂停、静音或打开详情时弹幕冻结，音频按播放状态继续",
            "波形、频谱与舞台共用宿主播放时钟和同一份 PCM",
            "界面只订阅本地指标，不额外下载电台音频",
            "频谱失败会自动退避重试，播放不受影响",
            "偏好自动保存在 tui.json · Esc 关闭",
            "环境变量 THMR_TUI_ASCII=1 使用基础绘图字符",
            "THMR_TUI_REDUCED_MOTION=1 初始采用静态档位",
        ]
        .into_iter()
        .map(|s| Line::from(s.to_owned()))
        .collect()
    } else {
        let track = if ui.overlay == Some(Overlay::CurrentDetails) {
            state.current().map(Track::from)
        } else {
            ui.tracks.get(ui.selected).cloned()
        };
        if let Some(track) = track {
            vec![
                Line::styled(track.title, style(CYAN).bold()),
                Line::from(""),
                Line::from(format!(
                    "艺术家：{}",
                    if track.artist.is_empty() {
                        "暂无信息"
                    } else {
                        &track.artist
                    }
                )),
                Line::from(format!(
                    "专辑：{}",
                    if track.album.is_empty() {
                        "暂无信息"
                    } else {
                        &track.album
                    }
                )),
                Line::from(format!(
                    "原曲：{}",
                    if track.original.is_empty() {
                        "暂无信息"
                    } else {
                        &track.original
                    }
                )),
                Line::from(""),
                Line::styled("电台按固定排播放送，曲目仅供浏览。", style(SUB)),
            ]
        } else {
            vec![Line::from("暂无曲目信息")]
        }
    };
    let wrapped = wrap_lines(lines, body.width as usize);
    let line_count = wrapped.len();
    let paragraph = Paragraph::new(wrapped).style(style(FG));
    ui.overlay_max_scroll = line_count
        .saturating_sub(body.height as usize)
        .min(u16::MAX as usize) as u16;
    ui.overlay_scroll = ui.overlay_scroll.min(ui.overlay_max_scroll);
    frame.render_widget(paragraph.scroll((ui.overlay_scroll, 0)), body);
    ui.hits.clear();
    ui.focusables.clear();
    button(
        frame,
        ui,
        row(inner, inner.height.saturating_sub(1)),
        "[关闭 / Esc]",
        Action::Close,
        true,
        false,
    );
    // Draw only the border and caption last; keep the content and close-button styles intact.
    frame.render_widget(
        Block::default()
            .title(Line::styled(format!(" {title} "), style(CYAN)))
            .borders(Borders::ALL)
            .border_style(style(CYAN)),
        rect,
    );
}
fn compact(frame: &mut Frame, ui: &mut Ui, state: &Snapshot, area: Rect, connection: Option<&str>) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let title = text(state.current().and_then(|v| v.get("title")));
    frame.render_widget(
        Paragraph::new(fit(
            &format!("THMR · {}", phase(state, connection.is_none())),
            area.width as usize,
        ))
        .style(style(CYAN)),
        row(area, 0),
    );
    if area.height > 1 {
        frame.render_widget(
            Paragraph::new(fit(&title, area.width as usize)).style(style(FG)),
            row(area, 1),
        );
    }
    if area.height > 2 {
        buttons(
            frame,
            ui,
            row(area, 2),
            &[
                ("[播放/暂停]", Action::Play, true, false),
                ("[-]", Action::Down, false, false),
                ("[+]", Action::Up, false, false),
            ],
        );
    }
    if area.height > 3 {
        frame.render_widget(
            Paragraph::new(fit(
                &format!(
                    "音量 {:0.0}% · m {} · ? 帮助 · q 退出",
                    ui.volume(state),
                    ui.prefs.motion.label()
                ),
                area.width as usize,
            ))
            .style(style(SUB)),
            row(area, 3),
        );
    }
    if area.height > 4 {
        frame.render_widget(
            Paragraph::new(fit(
                ui.notice
                    .as_deref()
                    .or(connection)
                    .unwrap_or("放大终端以显示排播和弹幕舞台"),
                area.width as usize,
            ))
            .style(style(SUB)),
            row(area, 4),
        );
    }
    if ui.overlay.is_some() && area.width >= 16 && area.height >= 6 {
        overlay(frame, ui, state, area);
    }
    ui.hover = ui.pointer.and_then(|p| ui.hit(p));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};
    use std::time::Instant;

    fn visible_text(buffer: &ratatui::buffer::Buffer) -> String {
        let mut result = String::new();
        for y in buffer.area.y..buffer.area.bottom() {
            let mut x = buffer.area.x;
            while x < buffer.area.right() {
                let symbol = buffer[(x, y)].symbol();
                result.push_str(symbol);
                // TestBackend does not erase cells covered by a double-width glyph.
                x += UnicodeWidthStr::width(symbol).max(1) as u16;
            }
            result.push('\n');
        }
        result
    }

    #[test]
    fn popup_border_remains_visible_above_wide_background_text() {
        let mut state = Snapshot::new("popup-border".into(), &Default::default());
        state.phase = Phase::Online;
        let wide_title = "东方幻想交响曲".repeat(30);
        state.station_status = Some(serde_json::json!({
            "current":{"title":wide_title, "artist":"柏木るざりん"},
            "queue":(1..=30).map(|id| serde_json::json!({"trackId":id, "title":wide_title, "artist":"柏木るざりん", "album":"幻想乡音乐专辑"})).collect::<Vec<_>>()
        }));
        for (width, height) in [
            (110, 34),
            (112, 34),
            (114, 34),
            (140, 40),
            (142, 40),
            (38, 16),
        ] {
            for motion in [Motion::Static, Motion::Dynamic] {
                for kind in [Overlay::Details, Overlay::CurrentDetails, Overlay::Help] {
                    let mut ui = Ui::new(super::super::model::Preferences {
                        motion,
                        ascii: false,
                    });
                    ui.sync(&state);
                    ui.selected = 1;
                    ui.modal(kind);
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    for scroll in [0, u16::MAX] {
                        ui.overlay_scroll = scroll;
                        terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
                        let buffer = terminal.backend().buffer();
                        let popup = Rect::new(
                            (width - (width - 4).min(76)) / 2,
                            (height - (height - 2).min(22)) / 2,
                            (width - 4).min(76),
                            (height - 2).min(22),
                        );
                        for y in popup.y..popup.bottom() {
                            let mut x = 0;
                            while x < popup.x {
                                let symbol = buffer[(x, y)].symbol();
                                let cells = UnicodeWidthStr::width(symbol).max(1) as u16;
                                assert!(x + cells <= popup.x, "border at ({}, {y}) covered by {symbol:?} at x={x}, viewport {width}x{height}, mode={motion:?}", popup.x);
                                x += cells;
                            }
                            for edge in [popup.x, popup.right() - 1] {
                                let expected = if y == popup.y {
                                    if edge == popup.x {
                                        "┌"
                                    } else {
                                        "┐"
                                    }
                                } else if y == popup.bottom() - 1 {
                                    if edge == popup.x {
                                        "└"
                                    } else {
                                        "┘"
                                    }
                                } else {
                                    "│"
                                };
                                assert_eq!(
                                    buffer[(edge, y)].symbol(),
                                    expected,
                                    "edge ({edge}, {y}) at {width}x{height}"
                                );
                                assert_eq!(buffer[(edge, y)].fg, CYAN);
                            }
                        }
                        assert_eq!(ui.hits.len(), 1);
                        assert_eq!(ui.hits[0].1, Action::Close);
                        if kind == Overlay::Details && scroll == 0 {
                            assert_eq!(buffer[(popup.x + 1, popup.y + 1)].fg, CYAN);
                            assert_eq!(buffer[(ui.hits[0].0.x, ui.hits[0].0.y)].bg, EDGE);
                            capture(buffer, &format!("popup-border-{width}x{height}-{motion:?}"));
                        }
                    }
                }
            }
        }
    }

    fn waveform_fixture() -> super::super::spectrum::Waveform {
        let pcm: Vec<_> = (0..2048)
            .map(|i| {
                let time = i as f32;
                (std::f32::consts::TAU * time / 512.0).sin()
                    * (0.6 + 0.3 * (std::f32::consts::TAU * time / 2048.0).cos())
            })
            .collect();
        let mut analyzer = thmr_core::audio::Analyzer::default();
        analyzer.add(&pcm, 48000, 1).unwrap();
        analyzer.snapshot(2048).waveform.unwrap()
    }

    #[test]
    fn static_mode_uses_three_full_width_bands_without_audio_visualizations() {
        let mut state = Snapshot::new("static-preview".into(), &Default::default());
        state.phase = Phase::Online;
        state.capabilities.synced_audio = true;
        state.station_status = Some(serde_json::json!({
            "current":{"trackId":"live", "title":"Quiet song", "artist":"Artist"},
            "queue":(1..=24).map(|i| serde_json::json!({"trackId":i, "title":format!("Queue track {i:02}"), "artist":"Artist"})).collect::<Vec<_>>()
        }));
        for (width, height) in [(38, 14), (38, 16), (79, 24), (140, 40), (320, 70)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut ui = Ui::new(super::super::model::Preferences::default());
            ui.sync(&state);
            ui.available = true;
            ui.bands.fill(0.9);
            ui.peaks.fill(1.0);
            ui.channel_rms.fill(0.6);
            ui.channel_peaks.fill(0.9);
            ui.waveform = Some(waveform_fixture());
            ui.stage.advance(
                0.1,
                &super::super::spectrum::MusicalFeatures {
                    level: 0.6,
                    intensity: 0.7,
                    ..Default::default()
                },
            );
            terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
            ui.prefs.motion = Motion::Static;
            ui.selected = 15;
            terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
            assert_eq!(ui.queue_rect, Rect::new(1, 5, width - 2, height - 13));
            assert!(
                ui.selected >= ui.offset && ui.selected < ui.offset + ui.queue_rect.height as usize
            );
            assert!(ui.spectrum_source(&state, true).is_none());
            assert!(!ui.animate(&state, true));
            let current = ui
                .hits
                .iter()
                .find(|(_, action)| *action == Action::CurrentDetails)
                .unwrap()
                .0;
            assert_eq!(current.y, 2);
            assert_eq!(
                ui.hit(ratatui::layout::Position::new(1, 2)),
                Some(Action::CurrentDetails)
            );
            for action in [
                Action::Play,
                Action::Stop,
                Action::Down,
                Action::Up,
                Action::Source,
                Action::Quality,
                Action::Mode(Motion::Dynamic),
                Action::Details,
                Action::Help,
                Action::Quit,
            ] {
                assert!(
                    ui.hits.iter().any(|(_, candidate)| *candidate == action),
                    "missing {action:?} at {width}x{height}"
                );
            }
            let buffer = terminal.backend().buffer();
            let content = visible_text(buffer).replace(' ', "");
            assert!(content.contains("Quietsong/Artist"));
            for removed in ["dBFS", "频谱", "WAVE", "AUDIO", "ON AIR"] {
                assert!(
                    !content.contains(&removed.replace(' ', "")),
                    "{removed} remains in static mode"
                );
            }
            capture(buffer, &format!("static-three-bands-{width}x{height}"));
            // TestBackend retains covered continuation cells; a wide glyph owns both columns.
            let mut graphics = Vec::new();
            for y in 0..height {
                let mut x = 0;
                while x < width {
                    let symbol = buffer[(x, y)].symbol();
                    if symbol.chars().any(|c| {
                        ('\u{2800}'..='\u{28ff}').contains(&c)
                            || ('\u{2580}'..='\u{259f}').contains(&c)
                    }) {
                        graphics.push((x, y, symbol));
                    }
                    x += UnicodeWidthStr::width(symbol).max(1) as u16;
                }
            }
            assert!(
                graphics.is_empty(),
                "graphics at {width}x{height}: {graphics:?}"
            );
            ui.prefs.motion = Motion::Dynamic;
            terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
            if width >= 140 {
                let content = visible_text(terminal.backend().buffer()).replace(' ', "");
                assert!(content.contains("dBFS"));
                assert!(content.contains("频谱"));
            }
        }
    }

    #[test]
    fn waveform_keeps_transients_when_resizing_and_draws_inside_its_strip() {
        let mut transient = super::super::spectrum::Waveform::default();
        transient.min[0] = -0.9;
        transient.max[127] = 0.8;
        assert_eq!(waveform_range(&transient, 0, 1), (-0.9, 0.8));
        assert_eq!(waveform_range(&transient, 0, 2), (-0.9, 0.0));
        assert_eq!(waveform_range(&transient, 1, 2), (0.0, 0.8));
        for ascii in [false, true] {
            let mut ui = Ui::new(super::super::model::Preferences {
                ascii,
                ..Default::default()
            });
            ui.waveform = Some(waveform_fixture());
            for width in [0, 1, 6, 7, 38, 84, 200] {
                for height in [0, 1, 2, 3] {
                    let mut terminal =
                        Terminal::new(TestBackend::new(width + 6, height + 4)).unwrap();
                    let area = Rect::new(3, 2, width, height);
                    terminal.draw(|f| waveform(f, &ui, area)).unwrap();
                    let buffer = terminal.backend().buffer();
                    for y in 0..height + 4 {
                        for x in 0..width + 6 {
                            let cell = &buffer[(x, y)];
                            if !area.contains(ratatui::layout::Position::new(x, y)) {
                                assert_eq!(cell.symbol(), " ");
                                assert_eq!(cell.bg, Color::Reset);
                            } else if ascii {
                                assert!(cell.symbol().is_ascii());
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn footer_spectrum_docks_right_or_below_without_covering_controls() {
        let mut state = Snapshot::new("footer-dock".into(), &Default::default());
        state.phase = Phase::Online;
        state.station_status =
            Some(serde_json::json!({"current":{"title":"Footer spectrum", "artist":"Fixture"}}));
        for (width, height, aligned) in [
            (38, 16, false),
            (38, 24, false),
            (79, 30, false),
            (99, 24, false),
            (100, 16, false),
            (100, 30, false),
            (110, 24, false),
            (110, 25, true),
            (120, 24, false),
            (122, 24, true),
            (140, 16, true),
            (140, 40, true),
            (320, 70, true),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut ui = Ui::new(super::super::model::Preferences::default());
            ui.sync(&state);
            ui.available = true;
            ui.focus = Action::Play;
            ui.bands = std::array::from_fn(|i| 0.3 + 0.6 * (i as f32 * 0.14).sin().abs());
            ui.peaks = ui.bands.map(|value| (value + 0.1).min(1.0));
            ui.stage.advance(
                0.1,
                &super::super::spectrum::MusicalFeatures {
                    level: 0.6,
                    intensity: 0.7,
                    ..Default::default()
                },
            );
            terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
            let control_y = ui
                .hits
                .iter()
                .find(|(_, action)| *action == Action::Play)
                .unwrap()
                .0
                .y
                - 1;
            let footer = Rect::new(0, control_y, width, height - 2 - control_y);
            let stage = ui
                .hits
                .iter()
                .find(|(_, action)| *action == Action::CurrentDetails)
                .unwrap()
                .0;
            let split = (stage.x > ui.queue_rect.x).then_some(stage.x - 1);
            let (control_area, spectrum_area) = footer_layout(footer, split);
            assert_eq!(
                spectrum_area.y == control_area.y,
                aligned,
                "layout at {width}x{height}"
            );
            if aligned {
                assert_eq!(spectrum_area.x, stage.x - 1);
                assert_eq!(spectrum_area.x, ui.queue_rect.right() + 1);
                assert_eq!(control_area.right(), spectrum_area.x);
                assert_eq!(spectrum_area.y, control_area.y);
                assert!(control_area.width >= MIN_CONTROL_WIDTH);
                assert!(spectrum_area.width >= MIN_SPECTRUM_WIDTH);
            } else {
                assert_eq!(spectrum_area.y, control_area.bottom());
                assert_eq!(spectrum_area.width, width);
            }
            assert!(spectrum_area.height >= 2);
            assert_eq!(spectrum_area.bottom(), height - 2);
            for (rect, _) in &ui.hits {
                let intersection = rect.intersection(spectrum_area);
                assert!(intersection.width == 0 || intersection.height == 0);
            }
            for action in [
                Action::Play,
                Action::Stop,
                Action::Down,
                Action::Up,
                Action::Source,
                Action::Quality,
                Action::Help,
                Action::Quit,
            ] {
                assert!(
                    ui.hits.iter().any(|(_, candidate)| *candidate == action),
                    "missing {action:?} at {width}x{height}"
                );
            }
            let buffer = terminal.backend().buffer();
            let caption = (spectrum_area.x..spectrum_area.right())
                .map(|x| buffer[(x, spectrum_area.y)].symbol())
                .collect::<String>()
                .replace(' ', "");
            assert!(caption.contains("频谱"));
            assert!((spectrum_area.x..spectrum_area.right()).any(|x| buffer
                [(x, spectrum_area.bottom() - 1)]
                .symbol()
                .chars()
                .any(|c| ('\u{2581}'..='\u{2588}').contains(&c))));
            for y in control_area.y..control_area.bottom() {
                for x in control_area.x..control_area.right() {
                    assert!(matches!(buffer[(x, y)].bg, BG | EDGE | Color::Reset));
                }
            }
            capture(buffer, &format!("footer-dock-{width}x{height}"));
        }
    }

    fn capture(buffer: &ratatui::buffer::Buffer, name: &str) {
        if let Some(path) = std::env::var_os("THMR_TUI_CAPTURE_DIR") {
            let directory = std::path::PathBuf::from(path);
            std::fs::create_dir_all(&directory).unwrap();
            let rgb = |color| match color {
                Color::Rgb(r, g, b) => [r, g, b],
                _ => [16, 21, 34],
            };
            let cells: Vec<_> = buffer.content.iter().map(|cell| {
                serde_json::json!({"text":cell.symbol(),"fg":rgb(cell.fg),"bg":rgb(cell.bg)})
            }).collect();
            std::fs::write(
                directory.join(format!("{name}.json")),
                serde_json::to_vec(&serde_json::json!({"width":buffer.area.width,"height":buffer.area.height,"cells":cells})).unwrap(),
            ).unwrap();
        }
    }

    #[test]
    fn stage_audio_panel_shows_consumed_values_and_labels_frozen_or_unused_input() {
        let mut state = Snapshot::new("metrics-preview".into(), &Default::default());
        state.phase = Phase::Online;
        state.station_status =
            Some(serde_json::json!({"current":{"title":"Audio metrics", "artist":"Fixture"}}));
        let music = super::super::spectrum::MusicalFeatures {
            level: 0.321,
            intensity: 0.765,
            low: 0.123,
            mid: 0.456,
            high: 0.789,
            beat_period: 0.5,
            beat_confidence: 0.875,
            beats: 123,
            accents: 45,
            surges: 6,
        };
        let mut ui = Ui::new(super::super::model::Preferences::default());
        ui.sync(&state);
        ui.available = true;
        ui.waveform = Some(waveform_fixture());
        ui.stage.advance(0.1, &music);
        for (width, height) in [(140, 40), (100, 30), (79, 40), (38, 45)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
            let buffer = terminal.backend().buffer();
            let locate = |label: &str| {
                (0..height)
                    .find(|y| {
                        (0..width)
                            .map(|x| buffer[(x, *y)].symbol())
                            .collect::<String>()
                            .replace(' ', "")
                            .contains(label)
                    })
                    .unwrap()
            };
            assert!(locate("WAVE") < locate("AUDIO"));
            let content = buffer
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .replace(' ', "");
            for value in [
                "AUDIO",
                "LIVE",
                "LVL 0.321",
                "INT 0.765",
                "LO 0.123",
                "MID 0.456",
                "HI 0.789",
                "BP 0.500s",
                "BC 0.875",
                "BT 123",
                "ACC 45",
                "SRG 6",
            ] {
                assert!(
                    content.contains(&value.replace(' ', "")),
                    "missing {value} at {width}x{height}"
                );
            }
            assert!(content.contains(&format!("DRV{:.3}", ui.stage.audio_metrics().unwrap().1)));
            assert!(stage_audio_lines(&ui, &state, true, width as usize)
                .iter()
                .all(|line| line.to_string().is_ascii()));
            capture(buffer, &format!("stage-metrics-{width}x{height}"));
        }
        ui.stage.suspend(&super::super::spectrum::MusicalFeatures {
            beats: 999,
            ..music
        });
        state.user_paused = true;
        let frozen = stage_audio_lines(&ui, &state, true, 84)
            .iter()
            .map(ToString::to_string)
            .collect::<String>();
        assert!(frozen.contains("HOLD:STOP"));
        assert!(frozen.contains("BT 123"));
        assert!(!frozen.contains("999"));
        ui.prefs.motion = Motion::Static;
        let still = stage_audio_lines(&ui, &state, true, 84)
            .iter()
            .map(ToString::to_string)
            .collect::<String>();
        assert!(still.contains("STATIC"));
        assert!(!still.contains("0.321"));
        ui.prefs.motion = Motion::Dynamic;
        ui.stage = super::super::danmaku::Stage::seeded(7);
        assert!(stage_audio_lines(&ui, &state, true, 84)[0]
            .to_string()
            .contains("WAIT"));
    }

    #[test]
    fn queue_note_pulses_brightness_without_hiding_or_changing_row_styles() {
        for (ascii, motion, phase, paused, connected, volume, overlay, blinking) in [
            (
                false,
                Motion::Dynamic,
                Phase::Online,
                false,
                true,
                65.0,
                None,
                true,
            ),
            (
                true,
                Motion::Dynamic,
                Phase::Online,
                false,
                true,
                65.0,
                None,
                true,
            ),
            (
                false,
                Motion::Static,
                Phase::Online,
                false,
                true,
                65.0,
                None,
                false,
            ),
            (
                false,
                Motion::Dynamic,
                Phase::Online,
                true,
                true,
                65.0,
                None,
                false,
            ),
            (
                false,
                Motion::Dynamic,
                Phase::Idle,
                false,
                true,
                65.0,
                None,
                false,
            ),
            (
                false,
                Motion::Dynamic,
                Phase::Online,
                false,
                false,
                65.0,
                None,
                false,
            ),
            (
                false,
                Motion::Dynamic,
                Phase::Online,
                false,
                true,
                0.0,
                None,
                false,
            ),
            (
                false,
                Motion::Dynamic,
                Phase::Online,
                false,
                true,
                65.0,
                Some(Overlay::Help),
                false,
            ),
        ] {
            let mut state = Snapshot::new("queue-preview".into(), &Default::default());
            state.phase = phase;
            state.user_paused = paused;
            state.volume = volume;
            state.station_status = Some(serde_json::json!({
                "current":{"trackId":"current", "title":"Shared title"},
                "queue":[{"trackId":"next", "title":"Shared title"}]
            }));
            let mut ui = Ui::new(super::super::model::Preferences { ascii, motion });
            ui.sync(&state);
            ui.overlay = overlay;
            let mut terminal = Terminal::new(TestBackend::new(55, 6)).unwrap();
            for selected in [0, 1] {
                ui.selected = selected;
                let mut colors = Vec::new();
                for age in [100, 700] {
                    ui.title_changed = Instant::now() - std::time::Duration::from_millis(age);
                    terminal
                        .draw(|f| queue(f, &mut ui, &state, f.area(), connected))
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let note = &buffer[(3, 1)];
                    assert_eq!(note.symbol(), if ascii { "*" } else { "♪" });
                    assert_eq!(note.bg, if selected == 0 { EDGE } else { BG });
                    assert_ne!(note.fg, note.bg);
                    colors.push(note.fg);
                    assert_eq!(buffer[(3, 2)].symbol(), "0");
                    assert_eq!(buffer[(4, 2)].symbol(), "1");
                    for y in [1, 2] {
                        let is_selected = selected == y as usize - 1;
                        assert_eq!(
                            buffer[(1, y)].symbol(),
                            if !is_selected {
                                " "
                            } else if ascii {
                                ">"
                            } else {
                                "▸"
                            }
                        );
                        let title = (6..18).map(|x| buffer[(x, y)].symbol()).collect::<String>();
                        assert_eq!(title, "Shared title");
                        assert_eq!(buffer[(6, y)].fg, if is_selected { CYAN } else { FG });
                        assert_eq!(buffer[(6, y)].bg, if is_selected { EDGE } else { BG });
                    }
                    if blinking && !ascii {
                        capture(buffer, &format!("queue-note-{selected}-{age}"));
                    }
                }
                if blinking {
                    assert_ne!(colors[0], colors[1]);
                } else {
                    assert_eq!(colors[0], colors[1]);
                }
            }
        }
    }

    #[test]
    fn composed_stage_keeps_foreground_readable_across_changing_scores() {
        use super::super::spectrum::MusicalFeatures;
        let mut state = Snapshot::new("preview".into(), &Default::default());
        state.phase = Phase::Online;
        state.station_status = Some(serde_json::json!({
            "current":{"title":"少女綺想曲 ～ Dream Battle", "artist":"上海アリス幻樂団"},
            "queue":[{"title":"少女綺想曲 ～ Dream Battle", "artist":"上海アリス幻樂団"},
                {"title":"幽雅に咲かせ、墨染の桜", "artist":"上海アリス幻樂団"},
                {"title":"U.N.オーエンは彼女なのか？", "artist":"上海アリス幻樂団"}]
        }));
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        let mut ui = Ui::new(super::super::model::Preferences::default());
        ui.sync(&state);
        ui.stage = super::super::danmaku::Stage::seeded(7);
        ui.available = true;
        ui.title_changed = Instant::now() - std::time::Duration::from_secs(2);
        terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
        let mut seen = std::collections::BTreeSet::new();
        let mut decorated = 0;
        for frame in 0..1350 {
            let music = MusicalFeatures {
                level: 0.65,
                intensity: 0.65 + 0.3 * (frame as f32 * 0.005).sin(),
                beat_confidence: 0.8,
                beat_period: 0.5,
                low: 0.55 + 0.2 * (frame as f32 * 0.02).sin(),
                mid: 0.45 + 0.2 * (frame as f32 * 0.01).cos(),
                high: 0.3 + 0.25 * (frame as f32 * 0.04).sin(),
                beats: frame / 15,
                accents: frame / 7,
                ..Default::default()
            };
            ui.stage.advance(1.0 / 30.0, &music);
            ui.clock = frame as f64 / 30.0;
            let window = [120..180, 495..555, 870..930, 1230..1290]
                .iter()
                .any(|w| w.contains(&frame));
            if !window || frame % 3 != 0 {
                continue;
            }
            ui.bands = std::array::from_fn(|i| {
                0.2 + 0.55 * (i as f32 * 0.21 + frame as f32 * 0.03).sin().abs()
            });
            ui.peaks = ui.bands.map(|v| (v + 0.07).min(1.0));
            ui.channel_rms = [
                0.18 + 0.12 * (frame as f32 * 0.05).sin().abs(),
                0.08 + 0.14 * (frame as f32 * 0.04).cos().abs(),
            ];
            ui.channel_peaks = [0.65, 0.48];
            terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
            let buffer = terminal.backend().buffer();
            let title_area = ui
                .hits
                .iter()
                .find(|(_, a)| *a == Action::CurrentDetails)
                .unwrap()
                .0;
            let row = (title_area.x..title_area.right())
                .map(|x| buffer[(x, title_area.y)].symbol())
                .collect::<String>();
            assert!(
                row.replace(' ', "").contains("少女綺想曲"),
                "foreground was overwritten: {row}"
            );
            let dots = buffer
                .content
                .iter()
                .filter(|cell| {
                    cell.symbol()
                        .chars()
                        .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))
                })
                .count();
            if dots > 30 {
                decorated += 1;
            }
            seen.insert((ui.stage.phase, ui.stage.label()));
            capture(buffer, &format!("motion-{frame:04}"));
        }
        assert!(seen.len() >= 4, "too few distinct scores: {seen:?}");
        assert!(decorated >= 40, "too many empty captures: {decorated}/80");
    }
    #[test]
    fn spectrum_bars_have_equal_widths_and_gaps() {
        let mut ui = Ui::new(super::super::model::Preferences::default());
        ui.available = true;
        ui.bands.fill(0.5);
        ui.peaks.fill(0.8);
        for ascii in [false, true] {
            ui.prefs.ascii = ascii;
            for width in 1..=320 {
                let mut terminal = Terminal::new(TestBackend::new(width + 6, 12)).unwrap();
                let area = Rect::new(3, 2, width, 8);
                terminal.draw(|f| spectrum(f, &ui, area)).unwrap();
                let buffer = terminal.backend().buffer();
                if [97, 112, 208].contains(&width) && !ascii {
                    capture(buffer, &format!("equal-bars-{width}"));
                }
                let mut bars = Vec::new();
                let mut x = area.x;
                while x < area.right() {
                    if buffer[(x, area.bottom() - 1)].symbol() == " " {
                        x += 1;
                        continue;
                    }
                    let start = x;
                    while x < area.right() && buffer[(x, area.bottom() - 1)].symbol() != " " {
                        x += 1;
                    }
                    bars.push(start..x);
                }
                assert!(!bars.is_empty());
                let bar_width = bars[0].len();
                assert!(
                    bars.iter().all(|bar| bar.len() == bar_width),
                    "unequal bar widths at {width} columns, ascii={ascii}: {bars:?}"
                );
                assert!(bars.windows(2).all(|pair| pair[1].start - pair[0].end == 1));
                let left_margin = bars[0].start - area.x;
                let right_margin = area.right() - bars.last().unwrap().end;
                assert!(left_margin.abs_diff(right_margin) <= 1);
                let peak_y = area.bottom() - 7;
                for x in area.x..area.right() {
                    assert_eq!(
                        buffer[(x, peak_y)].symbol() != " ",
                        buffer[(x, area.bottom() - 1)].symbol() != " ",
                        "peak width differs at {width} columns, x={x}"
                    );
                }
            }
        }
    }

    #[test]
    fn spectrum_spreads_bands_across_wide_and_narrow_viewports() {
        let mut ui = Ui::new(super::super::model::Preferences::default());
        ui.available = true;
        ui.bands = std::array::from_fn(|i| 0.2 + 0.6 * i as f32 / 47.0);
        ui.peaks = ui.bands.map(|v| v + 0.05);
        for ascii in [false, true] {
            ui.prefs.ascii = ascii;
            for width in [208, 160, 97, 96, 95, 48, 3, 2, 1] {
                let mut terminal = Terminal::new(TestBackend::new(width + 6, 12)).unwrap();
                let area = Rect::new(3, 2, width, 8);
                terminal.draw(|f| spectrum(f, &ui, area)).unwrap();
                let buffer = terminal.backend().buffer();
                let last = (area.x..area.right())
                    .rev()
                    .find(|x| buffer[(*x, area.bottom() - 1)].symbol() != " ")
                    .unwrap();
                let first = (area.x..area.right())
                    .find(|x| buffer[(*x, area.bottom() - 1)].symbol() != " ")
                    .unwrap();
                let bar_width = (first..area.right())
                    .take_while(|x| buffer[(*x, area.bottom() - 1)].symbol() != " ")
                    .count() as u16;
                assert!(
                    last >= area.right().saturating_sub(bar_width + 1),
                    "{width} columns, ascii={ascii}: last painted column is {}, expected near {}",
                    last - area.x,
                    width - 1
                );
                for (index, cell) in buffer.content.iter().enumerate() {
                    let position = ratatui::layout::Position::new(
                        index as u16 % (width + 6),
                        index as u16 / (width + 6),
                    );
                    if !area.contains(position) {
                        assert_eq!(cell.symbol(), " ");
                    }
                }
            }
        }
    }

    #[test]
    fn wide_spectrum_keeps_missing_frequencies_empty_and_scales_peak_lines() {
        let mut terminal = Terminal::new(TestBackend::new(320, 70)).unwrap();
        let area = Rect::new(108, 50, 208, 12);
        let mut ui = Ui::new(super::super::model::Preferences::default());
        ui.available = true;
        ui.bands.fill(0.5);
        ui.peaks.fill(0.8);
        for ascii in [false, true] {
            ui.prefs.ascii = ascii;
            terminal.draw(|f| spectrum(f, &ui, area)).unwrap();
            let buffer = terminal.backend().buffer();
            let peak_y = area.bottom() - (0.8 * area.height as f32).ceil() as u16;
            assert!(
                (area.right() - 5..area.right())
                    .filter(|x| buffer[(*x, peak_y)].symbol() == if ascii { "-" } else { "─" })
                    .count()
                    >= 3
            );
        }
        ui.bands[40..].fill(0.0);
        ui.peaks[40..].fill(0.0);
        terminal.draw(|f| spectrum(f, &ui, area)).unwrap();
        let buffer = terminal.backend().buffer();
        for y in area.y..area.bottom() {
            // Grouped analysis bands can straddle the cutoff by one displayed bar.
            for x in area.x + 40 * area.width / 48 + area.width.div_ceil(48)..area.right() {
                assert_eq!(buffer[(x, y)].symbol(), " ");
            }
        }
    }

    #[test]
    fn adaptive_spectrum_range_is_visible_and_reclaims_empty_columns() {
        use super::super::spectrum_view::{project, AdaptiveSpectrum};
        let mut state = Snapshot::new("range-preview".into(), &Default::default());
        state.phase = Phase::Online;
        state.station_status = Some(serde_json::json!({"current": {
            "title": "自适应频谱 · 4 kHz 带宽测试", "artist": "合成低通音频 / Synthetic audio"
        }}));
        let power: Vec<_> = (0..1024)
            .map(|i| {
                let hz = i as f32 * 48000.0 / 2048.0;
                if (80.0..=4000.0).contains(&hz) {
                    0.001 * (0.45 + 0.55 * (hz * 0.003).sin().abs())
                } else {
                    0.0
                }
            })
            .collect();
        let fixed = project(&power, 48000, 16000.0);
        let mut view = AdaptiveSpectrum::default();
        let mut ui = Ui::new(super::super::model::Preferences::default());
        ui.sync(&state);
        ui.available = true;
        ui.title_changed = Instant::now() - std::time::Duration::from_secs(2);
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        ui.bands = fixed;
        ui.peaks = fixed;
        terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
        capture(terminal.backend().buffer(), "range-before");
        for _ in 0..1000 {
            view.update(&power, &fixed, 48000, 0.02);
        }
        ui.bands = view.frame.bands;
        ui.peaks = view.frame.bands;
        ui.spectrum_max_hz = view.frame.max_hz;
        terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("4.5 kHz"));
        capture(terminal.backend().buffer(), "range-after");
    }

    #[test]
    fn stereo_meters_hide_when_either_dimension_is_too_small() {
        let state = Snapshot::new("layout".into(), &Default::default());
        for (width, height, visible) in [
            (140, 40, true),
            (110, 25, true),
            (109, 40, false),
            (140, 24, false),
            (79, 40, false),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut ui = Ui::new(super::super::model::Preferences::default());
            terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert_eq!(text.contains("dBFS"), visible, "{width}x{height}");
            assert_eq!(ui.queue_rect.x, if visible { 13 } else { 1 });
        }
    }

    #[test]
    fn all_sizes_modes_and_overlays_render_with_hitboxes_inside_terminal() {
        let mut state = Snapshot::new("test".into(), &Default::default());
        state.phase = Phase::Online;
        state.station_status = Some(
            serde_json::json!({"current":{"title":"少女綺想曲 ～ Dream Battle👩‍🎤","artist":"上海アリス幻樂団"},"queue":[{"title":"幽雅に咲かせ、墨染の桜","artist":"上海アリス幻樂団"}]}),
        );
        for (w, h) in [
            (320, 70),
            (140, 40),
            (100, 30),
            (79, 24),
            (40, 15),
            (30, 8),
            (10, 3),
            (1, 1),
            (0, 0),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            for mode in [Motion::Static, Motion::Dynamic] {
                for overlay in [None, Some(Overlay::Help), Some(Overlay::Details)] {
                    let mut ui = Ui::new(super::super::model::Preferences {
                        motion: mode,
                        ascii: false,
                    });
                    ui.sync(&state);
                    ui.overlay = overlay;
                    if mode != Motion::Static {
                        ui.bands = std::array::from_fn(|i| {
                            0.12 + 0.68 * ((i as f32 * 0.22).sin() * (i as f32 * 0.08).cos()).abs()
                        });
                        ui.peaks = ui.bands.map(|v| (v + 0.08).min(1.0));
                        ui.available = true;
                        ui.clock = 4.0;
                    }
                    terminal.draw(|f| draw(f, &mut ui, &state, None)).unwrap();
                    for (rect, _) in &ui.hits {
                        assert!(rect.right() <= w && rect.bottom() <= h, "{w}x{h}: {rect:?}");
                    }
                    if overlay.is_none() && mode == Motion::Dynamic && [140, 79, 40].contains(&w) {
                        capture(terminal.backend().buffer(), &format!("{w}x{h}"));
                    }
                    ui.prefs.ascii = true;
                    terminal
                        .draw(|f| draw(f, &mut ui, &state, Some("重连中")))
                        .unwrap();
                }
            }
        }
    }
    #[test]
    fn truncation_keeps_wide_graphemes_whole() {
        for width in 0..20 {
            let s = fit("東方👩‍🎤音楽の夜", width);
            assert!(UnicodeWidthStr::width(s.as_str()) <= width);
        }
    }
}
