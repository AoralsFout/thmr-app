//! Terminal rasterization of the stage, with a plain ASCII fallback.
use super::danmaku::{Bullet, BulletKind, Stage};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    symbols::Marker,
    text::Line,
    widgets::{
        canvas::{Canvas, Line as Stroke, Points},
        Clear, Paragraph,
    },
    Frame,
};

const BG: Color = Color::Rgb(16, 21, 34);
const PALETTE: [[u8; 3]; 4] = [
    [97, 225, 229],
    [207, 139, 245],
    [255, 147, 177],
    [255, 207, 119],
];
fn color(index: usize, scale: f32) -> Color {
    let [r, g, b] = PALETTE[index % PALETTE.len()];
    Color::Rgb(
        (r as f32 * scale) as u8,
        (g as f32 * scale) as u8,
        (b as f32 * scale) as u8,
    )
}

const GLOW_LEVELS: usize = 8;
const GROUPS: usize = PALETTE.len() * GLOW_LEVELS;
fn glow_group(bullet: &Bullet, still: bool) -> usize {
    let glow = ((bullet.glow(still) - 0.83) / 0.17 * (GLOW_LEVELS - 1) as f32).round() as usize;
    bullet.color % PALETTE.len() * GLOW_LEVELS + glow.min(GLOW_LEVELS - 1)
}
fn group_color(group: usize, scale: f32) -> Color {
    let glow = 0.83 + (group % GLOW_LEVELS) as f32 * 0.17 / (GLOW_LEVELS - 1) as f32;
    color(group / GLOW_LEVELS, scale * glow)
}

fn head_points(b: &Bullet, pixel_x: f32, pixel_y: f32, points: &mut Vec<(f64, f64)>) {
    let angle = b.vy.atan2(b.vx);
    let (sin, cos) = angle.sin_cos();
    let mut point = |forward: f32, side: f32| {
        points.push((
            (b.x + (forward * cos - side * sin) * pixel_x) as f64,
            (b.y + (forward * sin + side * cos) * pixel_y) as f64,
        ));
    };
    match b.kind {
        BulletKind::Orb => {
            for (x, y) in [
                (-0.55, -0.55),
                (0.55, -0.55),
                (-0.55, 0.55),
                (0.55, 0.55),
                (0.0, 0.0),
            ] {
                point(x, y);
            }
        }
        BulletKind::Needle => {
            for i in 0..5 {
                point(-i as f32 * 0.85, 0.0);
            }
        }
        BulletKind::Petal => {
            for (x, y) in [
                (0.0, 0.0),
                (-1.4, 0.0),
                (1.4, 0.0),
                (0.0, -0.85),
                (0.0, 0.85),
            ] {
                point(x, y);
            }
        }
        BulletKind::Rice => {
            for (x, y) in [
                (-1.3, 0.0),
                (-0.5, -0.5),
                (-0.5, 0.5),
                (0.5, -0.5),
                (0.5, 0.5),
                (1.3, 0.0),
            ] {
                point(x, y);
            }
        }
        BulletKind::Star => {
            point(0.0, 0.0);
            for i in 0..10 {
                let a = i as f32 * std::f32::consts::TAU / 10.0 + b.age * 0.9;
                let r = if i % 2 == 0 { 2.1 } else { 0.9 };
                point(a.cos() * r, a.sin() * r);
            }
        }
        BulletKind::Ring => {
            for i in 0..16 {
                let a = i as f32 * std::f32::consts::TAU / 16.0;
                point(a.cos() * 1.6, a.sin() * 1.6);
            }
        }
        BulletKind::Kunai => {
            for (x, y) in [
                (1.9, 0.0),
                (0.8, -0.6),
                (0.8, 0.6),
                (-0.3, -1.0),
                (-0.3, 1.0),
                (-0.3, 0.0),
                (-1.5, 0.0),
            ] {
                point(x, y);
            }
        }
        BulletKind::Butterfly => {
            let wing = 0.85 + 0.15 * (b.age * 4.0 + b.shimmer_phase).sin();
            for (x, y) in [
                (0.0, 0.0),
                (1.1, 0.0),
                (-1.0, 0.0),
                (0.8, 1.7),
                (0.8, -1.7),
                (-0.5, 2.0),
                (-0.5, -2.0),
                (-1.2, 1.3),
                (-1.2, -1.3),
            ] {
                point(x, y * wing);
            }
        }
    }
}

pub fn draw(frame: &mut Frame, area: Rect, stage: &Stage, ascii: bool, still: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let brightness = if still { 0.65 } else { 0.92 };
    if ascii {
        let mut cells = vec![(' ', 0); area.width as usize * area.height as usize];
        let mut point = |x: f32, y: f32, symbol: char, palette: usize| {
            if x.abs() > stage.aspect || y.abs() > 1.0 {
                return;
            }
            let px = ((x / stage.aspect + 1.0) * 0.5 * area.width.saturating_sub(1) as f32).round()
                as usize;
            let py = ((1.0 - y) * 0.5 * area.height.saturating_sub(1) as f32).round() as usize;
            cells[py * area.width as usize + px] = (symbol, palette);
        };
        for laser in &stage.lasers {
            let angle = laser.direction();
            for i in 0..240 {
                if laser.warning() && i % 6 >= 2 {
                    continue;
                }
                let r = i as f32 * 0.035;
                point(
                    laser.x + angle.cos() * r,
                    laser.y + angle.sin() * r,
                    if laser.warning() { ':' } else { '+' },
                    laser.color * GLOW_LEVELS + GLOW_LEVELS - 1,
                );
            }
        }
        for b in &stage.bullets {
            point(b.x, b.y, b.kind.ascii(), glow_group(b, still));
        }
        for e in &stage.enemies {
            point(
                e.x,
                e.y,
                if e.boss { 'W' } else { 'x' },
                e.color * GLOW_LEVELS + GLOW_LEVELS - 1,
            );
        }
        for y in 0..area.height {
            for x in 0..area.width {
                let (symbol, palette) = cells[y as usize * area.width as usize + x as usize];
                if symbol != ' ' {
                    frame.buffer_mut()[(area.x + x, area.y + y)]
                        .set_char(symbol)
                        .set_style(Style::new().fg(group_color(palette, brightness)).bg(BG));
                }
            }
        }
        return;
    }

    let mut heads: [Vec<(f64, f64)>; GROUPS] = std::array::from_fn(|_| Vec::new());
    let mut trails: [Vec<(f64, f64)>; GROUPS] = std::array::from_fn(|_| Vec::new());
    let pixel_x = stage.aspect * 2.0 / (area.width as f32 * 2.0 - 1.0).max(1.0);
    let pixel_y = 2.0 / (area.height as f32 * 4.0 - 1.0).max(1.0);
    for b in &stage.bullets {
        let palette = glow_group(b, still);
        head_points(b, pixel_x, pixel_y, &mut heads[palette]);
        if b.kind != BulletKind::Ring {
            let tail = if matches!(b.kind, BulletKind::Needle | BulletKind::Kunai) {
                0.11
            } else {
                0.065
            };
            for i in 1..=3 {
                let age = (i as f32 * tail / 3.0).min(b.age);
                trails[palette].push(((b.x - b.vx * age) as f64, (b.y - b.vy * age) as f64));
            }
        }
    }
    frame.render_widget(
        Canvas::default()
            .background_color(BG)
            .marker(Marker::Braille)
            .x_bounds([-stage.aspect as f64, stage.aspect as f64])
            .y_bounds([-1.0, 1.0])
            .paint(|context| {
                for (i, coords) in trails.iter().enumerate() {
                    context.draw(&Points {
                        coords,
                        color: group_color(i, brightness * 0.45),
                    });
                }
                context.layer();
                for (i, coords) in heads.iter().enumerate() {
                    context.draw(&Points {
                        coords,
                        color: group_color(i, brightness),
                    });
                }
                for laser in &stage.lasers {
                    let angle = laser.direction();
                    let scale = laser.opacity();
                    for i in 0..240 {
                        if laser.warning() && i % 9 >= 3 {
                            continue;
                        }
                        let r = i as f32 * 0.035;
                        let x = laser.x + angle.cos() * r;
                        let y = laser.y + angle.sin() * r;
                        let coords = [(x as f64, y as f64)];
                        context.draw(&Points {
                            coords: &coords,
                            color: color(laser.color, scale),
                        });
                        if !laser.warning() {
                            let coords = [(
                                (x - angle.sin() * 0.012) as f64,
                                (y + angle.cos() * 0.012) as f64,
                            )];
                            context.draw(&Points {
                                coords: &coords,
                                color: color(laser.color, scale),
                            });
                        }
                    }
                }
                context.layer();
                for enemy in &stage.enemies {
                    let (x, y) = (enemy.x as f64, enemy.y as f64);
                    let r = if enemy.boss { 0.065 } else { 0.038 };
                    for (dx, dy, ex, ey) in [
                        (0.0, r, r * 1.5, 0.0),
                        (r * 1.5, 0.0, 0.0, -r),
                        (0.0, -r, -r * 1.5, 0.0),
                        (-r * 1.5, 0.0, 0.0, r),
                    ] {
                        context.draw(&Stroke {
                            x1: x + dx,
                            y1: y + dy,
                            x2: x + ex,
                            y2: y + ey,
                            color: color(enemy.color, 1.0),
                        });
                    }
                    context.draw(&Stroke {
                        x1: x - r * 2.0,
                        y1: y,
                        x2: x + r * 2.0,
                        y2: y,
                        color: Color::Rgb(245, 251, 255),
                    });
                }
            }),
        area,
    );
}

/// Clear only the actual foreground label, leaving the rest of its row alive.
pub fn label(frame: &mut Frame, area: Rect, text: Line<'_>, width: u16) {
    let area = Rect::new(area.x, area.y, width.min(area.width), area.height.min(1));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(text).style(Style::new().bg(BG)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn shimmer_changes_color_without_blinking_geometry_and_static_is_constant() {
        let mut stage = Stage::poster();
        stage.enemies.clear();
        stage.bullets.truncate(1);
        let b = &mut stage.bullets[0];
        b.x = 0.0;
        b.y = 0.0;
        b.vx = 0.3;
        b.vy = 0.0;
        b.shimmer_phase = 0.0;
        for ascii in [false, true] {
            for still in [false, true] {
                let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
                stage.bullets[0].age = 1.0;
                terminal
                    .draw(|f| draw(f, f.area(), &stage, ascii, still))
                    .unwrap();
                let first = terminal.backend().buffer().clone();
                stage.bullets[0].age = 1.4;
                terminal
                    .draw(|f| draw(f, f.area(), &stage, ascii, still))
                    .unwrap();
                let second = terminal.backend().buffer();
                assert!(first
                    .content
                    .iter()
                    .zip(&second.content)
                    .all(|(a, b)| a.symbol() == b.symbol()));
                if still {
                    assert_eq!(&first, second);
                } else {
                    assert_ne!(&first, second);
                }
            }
        }
    }
}
