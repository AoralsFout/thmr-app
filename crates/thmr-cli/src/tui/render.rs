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
    if area.width < 38 || area.height < 16 {
        compact(frame, ui, state, area, connection);
        return;
    }
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(6),
        Constraint::Length(2),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" ☯ THMR", style(CYAN).bold()),
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
        ])
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(style(EDGE)),
        ),
        sections[0],
    );
    let body = sections[1];
    let body = if body.width >= 110 && body.height >= 14 {
        let columns = Layout::horizontal([Constraint::Length(12), Constraint::Min(80)]).split(body);
        channel_meter(frame, ui, columns[0]);
        columns[1]
    } else {
        body
    };
    if body.width >= 80 {
        let columns = Layout::horizontal([Constraint::Percentage(31), Constraint::Percentage(69)])
            .split(body);
        queue(frame, ui, columns[0]);
        now_playing(frame, ui, state, columns[1]);
    } else {
        let heights = Layout::vertical([
            Constraint::Min(4),
            Constraint::Length(if body.height >= 20 { 6 } else { 4 }),
        ])
        .split(body);
        now_playing(frame, ui, state, heights[0]);
        queue(frame, ui, heights[1]);
    }
    controls(frame, ui, state, sections[2]);
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
    frame.render_widget(Paragraph::new(lines), sections[3]);
    if ui.overlay.is_some() {
        overlay(frame, ui, state, area);
    }
    if !ui.focusables.contains(&ui.focus) {
        ui.focus = ui.focusables.first().copied().unwrap_or(Action::Queue);
    }
    ui.hover = ui.pointer.and_then(|p| ui.hit(p));
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

fn spectrum_height(height: u16, motion: Motion) -> u16 {
    if motion == Motion::Static || height < 9 {
        1
    } else {
        (height / 3).clamp(2, 12)
    }
}
fn queue(frame: &mut Frame, ui: &mut Ui, area: Rect) {
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
        let line = format!("{marker} {:02} {}", index + 1, track.title);
        let hovered = ui.hover == Some(Action::Select(index));
        frame.render_widget(
            Paragraph::new(fit(&line, rect.width as usize)).style(if selected || hovered {
                style(CYAN).bg(EDGE)
            } else {
                style(FG)
            }),
            rect,
        );
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
fn now_playing(frame: &mut Frame, ui: &mut Ui, state: &Snapshot, area: Rect) {
    if area.height < 2 {
        return;
    }
    let inner = inside(area);
    frame.render_widget(block("正在放送", false), area);
    let stage_height = if inner.height >= 6 {
        inner.height - spectrum_height(inner.height, ui.prefs.motion) - 1
    } else {
        inner.height
    };
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

    if inner.height >= 6 {
        let height = spectrum_height(inner.height, ui.prefs.motion);
        let plot = Rect::new(inner.x, inner.bottom() - height, inner.width, height);
        spectrum(frame, ui, plot);
        let caption = if ui.prefs.motion == Motion::Static {
            "静态 · 花瓣结界"
        } else if !ui.available {
            ui.spectrum_status
        } else {
            ui.stage.label()
        };
        let caption = if ui.prefs.motion == Motion::Dynamic && ui.available {
            format!(
                "频谱 40 Hz{}{:.1} kHz · {caption}",
                if ui.prefs.ascii { "-" } else { "–" },
                ui.spectrum_max_hz / 1000.0
            )
        } else {
            format!("频谱 · {caption}")
        };
        let caption_width = UnicodeWidthStr::width(caption.as_str()) as u16 + 2;
        let separator = row(inner, inner.height - height - 1);
        frame.render_widget(
            Paragraph::new(if ui.prefs.ascii { "-" } else { "─" }.repeat(inner.width as usize))
                .style(style(EDGE)),
            separator,
        );
        super::battlefield::label(
            frame,
            separator,
            Line::styled(format!(" {caption} "), style(SUB)),
            caption_width,
        );
    }
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
        let count = (plot.width as usize).div_ceil(2).min(ui.bands.len()).max(1);
        let partial = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
        for b in 0..count {
            // The 48 analysis bands are a frequency resolution, not a width
            // limit. Allocate every bar a proportional slice of the viewport.
            let left = b * plot.width as usize / count;
            let right = (b + 1) * plot.width as usize / count;
            let bar_end = right - usize::from(right - left > 1);
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
fn overlay(frame: &mut Frame, ui: &mut Ui, state: &Snapshot, area: Rect) {
    let width = area.width.saturating_sub(4).min(76);
    let height = area.height.saturating_sub(2).min(22);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(
        block(
            if ui.overlay == Some(Overlay::Help) {
                "使用帮助"
            } else if ui.overlay == Some(Overlay::CurrentDetails) {
                "正在放送 · 曲目信息"
            } else {
                "排播详情"
            },
            true,
        ),
        rect,
    );
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
            "静态：固定弹幕构图，关闭音频分析和持续动画",
            "动态：8 种队形、12 种构图、5 种节奏独立组合，避免近期重复",
            "重拍由部分敌机领奏，高频短促插入，渐强可提前变奏",
            "暂停、静音或打开详情时弹幕冻结，音频按播放状态继续",
            "频谱分析当前电台 MP3 流，会额外建立一条流连接",
            "频谱与播放有独立缓冲，可能存在少量时间偏差",
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
                assert!(
                    last >= area.right().saturating_sub(2),
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
            for x in area.x + 40 * area.width / 48..area.right() {
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
        assert_eq!(spectrum_height(27, Motion::Dynamic), 9);
        assert_eq!(spectrum_height(27, Motion::Static), 1);
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
