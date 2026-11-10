//! A bounded, deterministic bullet stage. World units are square; the viewport
//! supplies its character-cell aspect ratio. Rendering never advances simulation.
use super::choreography::{Director, Formation, Pattern, Phrase, Rhythm, Scene};
use super::spectrum::MusicalFeatures;
use std::f32::consts::{PI, TAU};

const STEP: f32 = 1.0 / 120.0;
pub const MAX_BULLETS: usize = 2400;
const MAX_LASERS: usize = 6;
const LASER_WARNING: f32 = 0.75;
const LASER_LIFETIME: f32 = 1.95;
const RESUME_TIME: f32 = 0.35;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EmitterId {
    wave: u64,
    slot: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BulletKind {
    Orb,
    Needle,
    Petal,
    Rice,
    Star,
    Ring,
    Kunai,
    Butterfly,
}
impl BulletKind {
    pub fn ascii(self) -> char {
        match self {
            Self::Orb => '.',
            Self::Needle => '|',
            Self::Petal => '*',
            Self::Rice => '/',
            Self::Star => '+',
            Self::Ring => 'o',
            Self::Kunai => 'v',
            Self::Butterfly => 'w',
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Bullet {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub age: f32,
    pub color: usize,
    pub kind: BulletKind,
    pub shimmer_phase: f32,
    turn: f32,
    gravity: f32,
}
impl Bullet {
    /// Small, asynchronous brightness changes. Age is simulation time, so pause
    /// and modals freeze shimmer too; the static poster has constant brightness.
    pub fn glow(&self, still: bool) -> f32 {
        if still {
            return 1.0;
        }
        (0.92
            + 0.06 * (self.age * TAU * 0.85 + self.shimmer_phase).sin()
            + 0.025 * (self.age * TAU * 1.35 + self.shimmer_phase * 0.73).sin())
        .clamp(0.83, 1.0)
    }
}

#[derive(Clone, Copy)]
pub struct Enemy {
    pub x: f32,
    pub y: f32,
    pub color: usize,
    pub boss: bool,
    firing: bool,
    slot: usize,
}

#[derive(Clone, Copy)]
pub struct Laser {
    pub x: f32,
    pub y: f32,
    pub angle: f32,
    pub age: f32,
    pub color: usize,
    pub sweep: f32,
    owner: EmitterId,
}
impl Laser {
    pub fn warning(&self) -> bool {
        self.age < LASER_WARNING
    }
    pub fn direction(&self) -> f32 {
        self.angle + (self.age - LASER_WARNING).max(0.0) * self.sweep
    }
    pub fn opacity(&self) -> f32 {
        if self.warning() {
            0.42
        } else {
            ((LASER_LIFETIME - self.age) / 0.65).clamp(0.0, 1.0)
        }
    }
}

pub struct Stage {
    pub bullets: Vec<Bullet>,
    pub enemies: Vec<Enemy>,
    pub lasers: Vec<Laser>,
    pub aspect: f32,
    pub time: f32,
    pub phase: usize,
    pub started: bool,
    director: Director,
    scene: Scene,
    phrase: Phrase,
    phase_age: f32,
    phrase_age: f32,
    wave: u64,
    path_time: [f32; 6],
    path_motion: [f32; 6],
    resume_speed: [f32; 6],
    accumulator: f32,
    emitter_in: [f32; 6],
    emitter_shots: [u32; 6],
    laser_in: f32,
    accent_in: f32,
    strength: f32,
    last_music: Option<MusicalFeatures>,
    beats: u64,
    events: Option<(u64, u64, u64)>,
    beat_pending: bool,
    accent_pending: bool,
    surge_pending: bool,
    shot_index: u32,
}
impl Default for Stage {
    fn default() -> Self {
        Self::seeded(0x74686d725f737461)
    }
}
impl Stage {
    pub fn seeded(seed: u64) -> Self {
        let mut director = Director::new(seed);
        let music = MusicalFeatures {
            intensity: 0.5,
            ..Default::default()
        };
        let scene = director.scene(&music);
        let phrase = director.phrase(&music);
        Self {
            bullets: Vec::with_capacity(MAX_BULLETS),
            enemies: Vec::with_capacity(6),
            lasers: Vec::with_capacity(MAX_LASERS),
            aspect: 1.8,
            time: 0.0,
            phase: scene.formation as usize,
            started: false,
            director,
            scene,
            phrase,
            phase_age: 0.0,
            phrase_age: 0.0,
            wave: 0,
            path_time: [0.0; 6],
            path_motion: [0.0; 6],
            resume_speed: [1.0; 6],
            accumulator: 0.0,
            emitter_in: std::array::from_fn(|i| i as f32 * 0.13),
            emitter_shots: [0; 6],
            laser_in: 1.0,
            accent_in: 0.0,
            strength: 0.0,
            last_music: None,
            beats: 0,
            events: None,
            beat_pending: false,
            accent_pending: false,
            surge_pending: false,
            shot_index: 0,
        }
    }

    pub fn for_track(session: u64, track: &str) -> Self {
        let hash = track.bytes().fold(0xcbf29ce484222325u64, |h, byte| {
            (h ^ byte as u64).wrapping_mul(0x100000001b3)
        });
        Self::seeded(session ^ hash)
    }

    pub fn label(&self) -> &'static str {
        self.phrase.primary.label()
    }

    /// Last input actually consumed by a simulation step, plus the smoothed drive strength.
    /// Suspension preserves this snapshot instead of exposing newer, unused audio.
    pub fn audio_metrics(&self) -> Option<(MusicalFeatures, f32)> {
        self.last_music.map(|music| (music, self.strength))
    }

    pub fn viewport(&mut self, width: u16, height: u16) {
        let aspect = (width as f32 / (height.max(1) as f32 * 2.0)).clamp(0.35, 8.0);
        let scale = aspect / self.aspect;
        // Reflow actor anchors together on resize. Projectile world distances
        // stay unchanged so round bullets retain their physical proportions.
        for enemy in &mut self.enemies {
            enemy.x *= scale;
        }
        for laser in &mut self.lasers {
            laser.x *= scale;
        }
        self.aspect = aspect;
    }

    pub fn suspend(&mut self, music: &MusicalFeatures) {
        self.events = Some((music.beats, music.accents, music.surges));
        self.beat_pending = false;
        self.accent_pending = false;
        self.surge_pending = false;
        self.accumulator = 0.0;
    }

    pub fn advance(&mut self, dt: f32, music: &MusicalFeatures) {
        let events = (music.beats, music.accents, music.surges);
        if let Some(previous) = self.events {
            self.beat_pending |= events.0 > previous.0;
            self.accent_pending |= events.1 > previous.1;
            self.surge_pending |= events.2 > previous.2;
        }
        self.events = Some(events);
        self.accumulator += dt.clamp(0.0, 0.1);
        while self.accumulator + 1e-6 >= STEP {
            self.accumulator = (self.accumulator - STEP).max(0.0);
            self.step(music);
        }
    }

    fn step(&mut self, music: &MusicalFeatures) {
        self.last_music = Some(*music);
        self.started = true;
        self.time += STEP;
        self.strength += (music.intensity - self.strength) * (1.0 - (-STEP / 0.6).exp());
        let sounding = music.level > 0.035;
        let beat = std::mem::take(&mut self.beat_pending) && sounding;
        let accent = std::mem::take(&mut self.accent_pending) && sounding;
        let surge = std::mem::take(&mut self.surge_pending) && sounding;
        if sounding {
            self.phase_age += STEP;
            self.phrase_age += STEP;
            for slot in 0..self.scene.count {
                if self.parked(slot).is_some() {
                    self.resume_speed[slot] = 0.0;
                } else {
                    self.resume_speed[slot] =
                        (self.resume_speed[slot] + STEP / RESUME_TIME).min(1.0);
                    let dt = STEP * self.resume_speed[slot];
                    self.path_time[slot] += dt;
                    self.path_motion[slot] += dt * (0.6 + self.strength * 0.55 + music.mid * 0.2);
                }
            }
            let completed = self.path_time[..self.scene.count]
                .iter()
                .copied()
                .fold(f32::INFINITY, f32::min);
            let boundary = beat || music.beat_confidence < 0.45;
            if self.lasers.is_empty()
                && ((completed >= self.scene.duration && boundary)
                    || completed > self.scene.duration + 1.1)
            {
                self.scene = self.director.scene(music);
                self.phase = self.scene.formation as usize;
                self.phase_age = 0.0;
                self.wave = self.wave.wrapping_add(1);
                self.path_time.fill(0.0);
                self.path_motion.fill(0.0);
                self.resume_speed.fill(1.0);
                self.emitter_in = std::array::from_fn(|i| 0.3 + i as f32 * 0.17);
            }
            if (self.phrase_age >= self.phrase.duration && boundary)
                || self.phrase_age > self.phrase.duration + 0.85
                || (surge && self.phrase_age > 2.0)
            {
                self.phrase = self.director.phrase(music);
                self.phrase_age = 0.0;
                self.emitter_shots.fill(0);
                self.emitter_in =
                    std::array::from_fn(|i| 0.12 + i as f32 * self.phrase.interval * 0.35);
            }
        }
        self.formation();
        self.laser_in = (self.laser_in - STEP).max(0.0);
        self.accent_in = (self.accent_in - STEP).max(0.0);
        for i in 0..self.enemies.len() {
            let e = self.enemies[i];
            let slot = e.slot;
            self.emitter_in[slot] -= STEP;
            if !sounding || !e.firing || self.emitter_in[slot] > 0.0 {
                continue;
            }
            let shot = self.emitter_shots[slot];
            let p = self.phrase;
            let answer = ((self.phrase_age / (p.duration * 0.5)) as usize + slot).is_multiple_of(2);
            let allowed = p.rhythm != Rhythm::Answer || answer;
            // Let a dense passage drain before adding another full volley.
            if allowed && self.bullets.len() < MAX_BULLETS * 4 / 5 {
                self.emit(p.primary, e, p.kinds[0], false);
            }
            self.emitter_shots[slot] = shot.wrapping_add(1);
            let base =
                p.interval * (1.0 + self.scene.count as f32 * 0.08) / (0.75 + self.strength * 0.6);
            self.emitter_in[slot] = match p.rhythm {
                Rhythm::Burst if shot % 3 != 2 => 0.10,
                Rhythm::Burst => base * 3.0,
                Rhythm::Syncopated => {
                    let pulse = if music.beat_confidence > 0.45 {
                        music.beat_period.clamp(0.25, 1.2)
                    } else {
                        base
                    };
                    pulse * [0.65, 0.35, 1.0][shot as usize % 3]
                }
                Rhythm::Stagger => base * (0.8 + slot as f32 * 0.13),
                _ => base,
            };
        }
        if beat {
            self.beats = self.beats.wrapping_add(1);
            if self.beats.is_multiple_of(self.phrase.beat_divider) {
                let lead = (self.beats as usize + self.phrase.offset as usize) % self.scene.count;
                for i in 0..self.enemies.len() {
                    let mut e = self.enemies[i];
                    if e.firing
                        && (e.slot == lead
                            || (self.strength > 0.85 && e.slot == (lead + 2) % self.scene.count))
                    {
                        e.color = (e.color + 1) % 4;
                        self.emit(self.phrase.accent, e, self.phrase.kinds[1], true);
                    }
                }
            }
        }
        if accent && self.accent_in == 0.0 && !self.enemies.is_empty() {
            self.accent_in = 0.2 + (1.0 - music.high) * 0.15;
            let lead = (self.shot_index as usize + self.beats as usize) % self.enemies.len();
            let e = self.enemies[lead];
            if e.firing {
                let aim = self.aim(e);
                self.fan(
                    e,
                    aim,
                    self.phrase.arms,
                    self.phrase.spread * 0.65,
                    self.phrase.speed * 1.5,
                    self.phrase.kinds[1],
                );
            }
        }
        if sounding
            && self.phrase.lasers
            && self.laser_in == 0.0
            && self.phase_age + LASER_LIFETIME + RESUME_TIME < self.scene.duration
            && (beat || surge || music.beat_confidence < 0.45)
        {
            if let Some(e) = self.enemies.iter().find(|e| e.firing).copied() {
                let count = if self.strength > 0.75 { 2 } else { 1 };
                self.charge_laser(e, count);
            }
        }
        for b in &mut self.bullets {
            b.age += STEP;
            if b.kind == BulletKind::Petal && b.age < 1.1 {
                b.vx *= 1.0 - STEP * 0.6;
                b.vy *= 1.0 - STEP * 0.6;
            }
            let turn = match b.kind {
                BulletKind::Rice => b.turn + (b.age * 5.0 + b.shimmer_phase).sin() * 0.32,
                BulletKind::Butterfly => b.turn + (b.age * 3.8 + b.shimmer_phase).sin() * 1.1,
                _ if b.age > 0.65 => b.turn,
                _ => 0.0,
            } * STEP;
            if b.kind == BulletKind::Kunai && b.age > 0.4 {
                b.vx *= 1.0 + STEP * 0.18;
                b.vy *= 1.0 + STEP * 0.18;
            }
            b.vy -= STEP * b.gravity;
            let (sin, cos) = turn.sin_cos();
            (b.vx, b.vy) = (b.vx * cos - b.vy * sin, b.vx * sin + b.vy * cos);
            b.x += b.vx * STEP;
            b.y += b.vy * STEP;
        }
        self.bullets
            .retain(|b| b.age < 11.0 && b.x.abs() < self.aspect + 0.18 && b.y.abs() < 1.22);
        for l in &mut self.lasers {
            l.age += STEP;
        }
        let wave = self.wave;
        let count = self.scene.count;
        self.lasers
            .retain(|l| l.age < LASER_LIFETIME && l.owner.wave == wave && l.owner.slot < count);
    }

    fn parked(&self, slot: usize) -> Option<(f32, f32)> {
        let owner = EmitterId {
            wave: self.wave,
            slot,
        };
        self.lasers
            .iter()
            .find(|l| l.owner == owner && l.age < LASER_LIFETIME)
            .map(|l| (l.x, l.y))
    }

    fn charge_laser(&mut self, source: Enemy, count: usize) {
        if self.parked(source.slot).is_some() || self.lasers.len() >= MAX_LASERS {
            return;
        }
        let owner = EmitterId {
            wave: self.wave,
            slot: source.slot,
        };
        for i in 0..count {
            if self.lasers.len() >= MAX_LASERS {
                break;
            }
            self.lasers.push(Laser {
                x: source.x,
                y: source.y,
                angle: self.aim(source)
                    + (i as f32 - (count - 1) as f32 / 2.0) * self.phrase.spread * 4.0,
                age: 0.0,
                color: source.color,
                sweep: self.phrase.spin * 0.28,
                owner,
            });
        }
        self.resume_speed[source.slot] = 0.0;
        self.laser_in = 2.4 + self.phrase.interval * 2.0;
        if let Some(enemy) = self.enemies.iter_mut().find(|e| e.slot == source.slot) {
            enemy.firing = false;
        }
    }

    fn formation(&mut self) {
        self.enemies.clear();
        let s = self.scene;
        let a = self.aspect;
        let smooth = |v: f32| {
            let u = v.clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u)
        };
        let path = |time: f32| {
            let segment = (time / s.duration * 3.0).clamp(0.0, 2.999);
            let index = segment as usize;
            let blend = smooth((segment.fract() - 0.4) / 0.6);
            let (x, y) = s.points[index];
            let (tx, ty) = s.points[index + 1];
            ((x + (tx - x) * blend) * a, y + (ty - y) * blend)
        };
        for slot in 0..s.count {
            if let Some((x, y)) = self.parked(slot) {
                self.enemies.push(Enemy {
                    x,
                    y,
                    color: (s.color + slot) % 4,
                    boss: s.count == 1 || (s.formation == Formation::Satellites && slot == 0),
                    firing: false,
                    slot,
                });
                continue;
            }
            let t = self.path_time[slot];
            let motion = self.path_motion[slot];
            let i = slot as f32;
            let side = if slot.is_multiple_of(2) {
                -s.mirror
            } else {
                s.mirror
            };
            let angle = motion * s.rate * s.mirror + s.phase + i * TAU / s.count as f32;
            let mut firing = t > 0.9 + i * 0.16 && t < s.duration - 1.2;
            let (mut x, mut y) = match s.formation {
                Formation::Crossing => {
                    let u = ((t - i * 0.45) / (s.duration - i * 0.45)).clamp(0.0, 1.0);
                    firing &= (0.08..0.87).contains(&u);
                    (
                        side * a * (1.18 - u * 2.36),
                        s.points[slot % 4].1 - u * 0.6 + (u * PI).sin() * 0.18,
                    )
                }
                Formation::Roaming => path(t),
                Formation::Orbit => (
                    (s.points[0].0 * 0.35 + angle.cos() * s.radius * 0.55) * a,
                    0.3 + angle.sin() * s.radius * 0.3,
                ),
                Formation::FigureEight => (
                    (angle.sin() * s.radius * 0.65) * a,
                    0.32 + (angle * 2.0).sin() * 0.29,
                ),
                Formation::Pendulum => (
                    ((i / (s.count - 1) as f32 * 2.0 - 1.0) * 0.55
                        + (motion * s.rate).sin() * 0.19)
                        * a,
                    0.48 + angle.cos() * 0.15,
                ),
                Formation::Rain => {
                    let u = ((t - i * 0.5) / (s.duration - i * 0.5)).clamp(0.0, 1.0);
                    firing &= (0.09..0.86).contains(&u);
                    (
                        (s.points[slot % 4].0 + (u * PI).sin() * side * 0.2) * a,
                        1.2 - u * 2.4,
                    )
                }
                Formation::Satellites => {
                    let (cx, cy) = path(t);
                    if slot == 0 {
                        (cx, cy)
                    } else {
                        (
                            cx + angle.cos() * a * 0.25 * s.radius,
                            cy + angle.sin() * 0.24 * s.radius,
                        )
                    }
                }
                Formation::Flanks => (
                    side * a * (0.75 + angle.sin() * 0.08),
                    0.35 + (angle * 0.7).sin() * 0.35,
                ),
            };
            if !matches!(s.formation, Formation::Crossing | Formation::Rain) {
                let entry = smooth((t - i * 0.16) / 1.1);
                let exit = smooth((t - s.duration + 1.2) / 1.2);
                if s.formation == Formation::Flanks {
                    x = side * a * 1.2 + (x - side * a * 1.2) * entry;
                } else {
                    y = 1.25 + (y - 1.25) * entry;
                }
                y += exit * (1.3 - y);
            }
            self.enemies.push(Enemy {
                x,
                y,
                color: (s.color + slot) % 4,
                boss: s.count == 1 || (s.formation == Formation::Satellites && slot == 0),
                firing,
                slot,
            });
        }
    }

    fn aim(&self, e: Enemy) -> f32 {
        let target = (self.time * 0.23 + self.scene.phase).sin() * self.aspect * 0.5;
        (-0.65 - e.y).atan2(target - e.x)
    }

    fn emit(&mut self, pattern: Pattern, e: Enemy, kind: BulletKind, accent: bool) {
        let p = self.phrase;
        let rotation = p.offset + self.phrase_age * p.spin + e.slot as f32 * 0.67;
        let aim = self.aim(e);
        let count = ((p.count as f32
            * (0.55 + self.strength * 0.5)
            * if self.scene.count > 3 { 0.75 } else { 1.0 }) as usize)
            .max(8);
        let speed = p.speed * (0.8 + self.strength * 0.45) * if accent { 1.12 } else { 1.0 };
        match pattern {
            Pattern::Fan => self.fan(
                e,
                aim + rotation.sin() * 0.22,
                p.arms * 2 + 1,
                p.spread,
                speed,
                kind,
            ),
            Pattern::Spiral => {
                for i in 0..p.arms {
                    self.spawn(
                        e,
                        rotation + i as f32 * TAU / p.arms as f32,
                        speed,
                        kind,
                        p.turn,
                    );
                }
            }
            Pattern::Flower | Pattern::Polygon => {
                for i in 0..count {
                    let angle = rotation + i as f32 * TAU / count as f32;
                    let radius = if pattern == Pattern::Flower {
                        0.55 + 0.45 * (angle * p.lobes - rotation * (p.lobes - 1.0)).cos().abs()
                    } else {
                        let sector = TAU / p.lobes;
                        (PI / p.lobes).cos()
                            / ((angle - rotation + sector / 2.0).rem_euclid(sector) - sector / 2.0)
                                .cos()
                    };
                    self.spawn(e, angle, speed * radius, kind, p.turn);
                }
            }
            Pattern::Weave => {
                for i in 0..p.arms * 2 + 3 {
                    let offset = i as f32 - (p.arms + 1) as f32;
                    self.spawn(
                        e,
                        aim + offset * p.spread,
                        speed * (0.85 + 0.25 * (offset * 0.7 + rotation).sin()),
                        kind,
                        p.turn + (offset * 0.6 + rotation).cos() * 0.35,
                    );
                }
            }
            Pattern::Curtain => {
                for i in 0..p.arms * 2 {
                    let mut source = e;
                    source.x += (i as f32 - p.arms as f32 + 0.5) * 0.09;
                    self.spawn(
                        source,
                        -PI / 2.0 + rotation.sin() * 0.3,
                        speed * (0.85 + 0.2 * (i as f32 * 0.8 + rotation).cos()),
                        kind,
                        p.turn * 0.5,
                    );
                }
            }
            Pattern::BrokenRing => {
                for i in 0..count {
                    if (i / 3 + e.slot).is_multiple_of(3) {
                        continue;
                    }
                    self.spawn(
                        e,
                        rotation + i as f32 * TAU / count as f32,
                        speed,
                        kind,
                        p.turn,
                    );
                }
            }
            Pattern::Tangent => {
                for i in 0..p.arms * 2 {
                    let a = rotation + i as f32 * TAU / (p.arms * 2) as f32;
                    let mut source = e;
                    source.x += a.cos() * 0.16;
                    source.y += a.sin() * 0.16;
                    self.spawn(source, a + p.spin.signum() * PI * 0.65, speed, kind, p.turn);
                }
            }
            Pattern::Chevron => {
                for i in 0..p.arms {
                    for side in [-1.0, 1.0] {
                        self.spawn(
                            e,
                            aim + side * (0.16 + i as f32 * p.spread),
                            speed * (0.65 + i as f32 * 0.08),
                            kind,
                            side * p.turn,
                        );
                    }
                }
            }
            Pattern::Echo => {
                for layer in [0.67, 1.12] {
                    for i in 0..count / 2 {
                        self.spawn(
                            e,
                            rotation + i as f32 * TAU / (count / 2) as f32,
                            speed * layer,
                            kind,
                            p.turn,
                        );
                    }
                }
            }
            Pattern::Cross => {
                for arm in 0..4 {
                    for i in 0..3 {
                        self.spawn(
                            e,
                            rotation + arm as f32 * PI / 2.0 + (i as f32 - 1.0) * p.spread * 0.6,
                            speed,
                            kind,
                            p.turn,
                        );
                    }
                }
            }
            Pattern::Fountain => {
                for i in 0..p.arms * 2 + 1 {
                    if self.bullets.len() >= MAX_BULLETS {
                        break;
                    }
                    let offset = i as f32 - p.arms as f32;
                    self.spawn(
                        e,
                        PI / 2.0 + offset * p.spread * 1.5,
                        speed * (0.9 + 0.1 * offset.abs()),
                        kind,
                        p.turn,
                    );
                    if let Some(bullet) = self.bullets.last_mut() {
                        bullet.gravity = 1.1;
                    }
                }
            }
        }
    }
    fn fan(&mut self, e: Enemy, aim: f32, count: usize, spread: f32, speed: f32, kind: BulletKind) {
        for i in 0..count {
            let angle = aim + (i as f32 - (count - 1) as f32 / 2.0) * spread;
            self.spawn(e, angle, speed * (1.0 + self.strength * 0.35), kind, 0.0);
        }
    }

    fn spawn(&mut self, e: Enemy, angle: f32, speed: f32, kind: BulletKind, turn: f32) {
        if self.bullets.len() < MAX_BULLETS {
            self.shot_index = self.shot_index.wrapping_add(1);
            let seed = self.shot_index.wrapping_mul(2654435761);
            self.bullets.push(Bullet {
                x: e.x,
                y: e.y,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed,
                age: 0.0,
                color: e.color,
                kind,
                shimmer_phase: (seed >> 8) as f32 / 16777216.0 * TAU,
                turn,
                gravity: if kind == BulletKind::Kunai { 0.06 } else { 0.0 },
            });
        }
    }

    /// A composed still, independent of audio, wall time, and animation history.
    pub fn poster() -> Self {
        let mut stage = Self {
            phase: 1,
            phase_age: 4.0,
            started: true,
            ..Self::default()
        };
        stage.formation();
        let e = Enemy {
            x: 0.0,
            y: 0.2,
            color: 1,
            boss: true,
            firing: true,
            slot: 0,
        };
        stage.enemies = vec![e];
        for ring in 0..5 {
            for i in 0..72 {
                let angle = TAU * i as f32 / 72.0 + ring as f32 * 0.16;
                let r = 0.22 + ring as f32 * 0.18 + 0.12 * (angle * 6.0).cos();
                stage.bullets.push(Bullet {
                    x: angle.cos() * r,
                    y: e.y + angle.sin() * r,
                    vx: angle.cos() * 0.25,
                    vy: angle.sin() * 0.25,
                    age: 2.0,
                    color: ring % 3,
                    kind: BulletKind::Petal,
                    shimmer_phase: i as f32 * 2.4 + ring as f32,
                    turn: 0.0,
                    gravity: 0.0,
                });
            }
        }
        stage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_metrics_follow_consumed_steps_and_stay_frozen_during_suspension() {
        let mut stage = Stage::seeded(7);
        let first = MusicalFeatures {
            level: 0.6,
            intensity: 0.8,
            beats: 12,
            ..Default::default()
        };
        assert!(stage.audio_metrics().is_none());
        stage.advance(STEP / 4.0, &first);
        assert!(stage.audio_metrics().is_none());
        stage.advance(STEP, &first);
        let (music, drive) = stage.audio_metrics().unwrap();
        assert_eq!(
            serde_json::to_value(music).unwrap(),
            serde_json::to_value(first).unwrap()
        );
        assert!(drive > 0.0 && drive < first.intensity);
        let next = MusicalFeatures {
            level: 0.2,
            intensity: 0.3,
            beats: 24,
            ..first
        };
        stage.suspend(&next);
        let (music, strength) = stage.audio_metrics().unwrap();
        assert_eq!(music.beats, first.beats);
        assert_eq!(strength, drive);
        stage.advance(STEP / 4.0, &next);
        assert_eq!(stage.audio_metrics().unwrap().0.beats, first.beats);
        stage.advance(STEP, &next);
        assert_eq!(
            serde_json::to_value(stage.audio_metrics().unwrap().0).unwrap(),
            serde_json::to_value(next).unwrap()
        );
        assert!(Stage::for_track(7, "next-track").audio_metrics().is_none());
    }

    fn charging_stage(formation: Formation, duration: f32) -> (Stage, MusicalFeatures, usize) {
        let music = MusicalFeatures {
            level: 0.7,
            intensity: 0.8,
            ..Default::default()
        };
        let mut stage = Stage::seeded(7);
        stage.scene.formation = formation;
        stage.scene.duration = duration;
        stage.scene.count = match formation {
            Formation::Roaming => 1,
            Formation::Satellites => 3,
            _ => 2,
        };
        stage.phrase.lasers = false;
        for _ in 0..240 {
            stage.step(&music);
        }
        stage.phrase.lasers = true;
        stage.laser_in = 0.0;
        stage.step(&music);
        let slot = stage.lasers[0].owner.slot;
        stage.laser_in = 100.0;
        (stage, music, slot)
    }

    #[test]
    fn laser_source_stays_parked_through_warning_and_firing() {
        for formation in [
            Formation::Crossing,
            Formation::Roaming,
            Formation::Orbit,
            Formation::FigureEight,
            Formation::Pendulum,
            Formation::Rain,
            Formation::Satellites,
            Formation::Flanks,
        ] {
            let (mut stage, music, slot) = charging_stage(formation, 16.0);
            let origin = (stage.lasers[0].x, stage.lasers[0].y);
            let path_time = stage.path_time[slot];
            let other = (slot + 1) % stage.scene.count;
            let other_time = stage.path_time[other];
            for _ in 0..210 {
                stage.step(&music);
                let source = stage.enemies.iter().find(|e| e.slot == slot).unwrap();
                assert!(
                    (source.x - origin.0).abs() < 0.00001 && (source.y - origin.1).abs() < 0.00001,
                    "{formation:?}: source moved away from its charge position"
                );
                assert!(!source.firing);
                assert_eq!(stage.path_time[slot], path_time);
            }
            if other != slot {
                assert!(stage.path_time[other] > other_time + 1.5);
            }
            for _ in 0..60 {
                if stage.lasers.is_empty() {
                    break;
                }
                stage.step(&music);
            }
            assert!(stage.lasers.is_empty());
            let mut previous = origin;
            for _ in 0..120 {
                stage.step(&music);
                let source = stage.enemies.iter().find(|e| e.slot == slot).unwrap();
                assert!(
                    (source.x - previous.0).hypot(source.y - previous.1) < 0.05,
                    "{formation:?}: teleported after releasing the laser"
                );
                previous = (source.x, source.y);
            }
            assert!(
                stage.path_time[slot] > path_time + 0.5,
                "{formation:?}: movement failed to resume"
            );
        }
    }

    #[test]
    fn wave_waits_for_parked_actor_to_resume_and_finish_its_path() {
        let (mut stage, music, slot) = charging_stage(Formation::Orbit, 5.0);
        let wave = stage.wave;
        for _ in 0..400 {
            if stage.phase_age > 5.25 {
                break;
            }
            stage.step(&music);
        }
        assert!(stage.phase_age > stage.scene.duration);
        assert!(stage.lasers.is_empty());
        assert_eq!(stage.wave, wave);
        assert!(stage.path_time[slot] < stage.scene.duration);
        for _ in 0..600 {
            if stage.wave != wave {
                break;
            }
            stage.step(&music);
        }
        assert_eq!(stage.wave, wave + 1);
        assert!(stage.lasers.is_empty());
    }

    #[test]
    fn resize_preserves_charge_anchor_and_reused_slots_cannot_capture_old_lasers() {
        let (mut stage, music, slot) = charging_stage(Formation::Orbit, 16.0);
        let age = stage.lasers[0].age;
        let path = stage.path_time[slot];
        stage.suspend(&music);
        assert_eq!(stage.lasers[0].age, age);
        assert_eq!(stage.path_time[slot], path);
        stage.viewport(70, 25);
        let source = stage.enemies.iter().find(|e| e.slot == slot).unwrap();
        assert_eq!((source.x, source.y), (stage.lasers[0].x, stage.lasers[0].y));
        stage.step(&music);
        let source = stage.enemies.iter().find(|e| e.slot == slot).unwrap();
        assert_eq!((source.x, source.y), (stage.lasers[0].x, stage.lasers[0].y));
        stage.wave += 1;
        assert!(stage.parked(slot).is_none());
        stage.step(&music);
        assert!(stage.lasers.is_empty());
    }

    #[test]
    fn long_music_run_is_bounded_and_visits_every_formation() {
        let mut stage = Stage::default();
        let mut phases = [false; 8];
        let mut music = MusicalFeatures {
            level: 0.7,
            intensity: 1.0,
            ..Default::default()
        };
        let mut saw_laser = false;
        let mut kinds = [false; 8];
        let mut patterns = [false; 12];
        for frame in 0..12000 {
            if frame % 12 == 0 {
                music.beats += 1;
            }
            if frame % 5 == 0 {
                music.accents += 1;
            }
            stage.advance(1.0 / 30.0, &music);
            phases[stage.phase] = true;
            patterns[stage.phrase.primary as usize] = true;
            saw_laser |= !stage.lasers.is_empty();
            for bullet in &stage.bullets {
                kinds[bullet.kind as usize] = true;
            }
            assert!(stage.bullets.len() <= MAX_BULLETS);
            assert!(stage.lasers.len() <= MAX_LASERS);
            assert!(stage
                .bullets
                .iter()
                .all(|b| b.x.is_finite() && b.y.is_finite()));
        }
        assert!(phases.into_iter().all(|v| v));
        assert!(patterns.into_iter().all(|v| v));
        assert!(saw_laser);
        assert!(
            kinds.into_iter().all(|v| v),
            "some bullet shapes never reached the stage"
        );
    }

    #[test]
    fn formation_and_attack_timelines_vary_independently() {
        let mut stage = Stage::seeded(17);
        let music = MusicalFeatures {
            level: 0.6,
            intensity: 0.65,
            ..Default::default()
        };
        let mut scene = stage.phase;
        let mut pattern = stage.phrase.primary;
        let mut pattern_changes_inside_scene = 0;
        let mut scene_durations = Vec::new();
        let mut last_scene = 0.0;
        for _ in 0..6000 {
            stage.advance(1.0 / 30.0, &music);
            if stage.phase != scene {
                scene_durations.push(stage.time - last_scene);
                last_scene = stage.time;
                scene = stage.phase;
            } else if stage.phrase.primary != pattern {
                pattern_changes_inside_scene += 1;
            }
            pattern = stage.phrase.primary;
        }
        assert!(pattern_changes_inside_scene > 15);
        assert!(scene_durations.len() > 10);
        let minimum = scene_durations
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let maximum = scene_durations.iter().copied().fold(0.0, f32::max);
        assert!(maximum - minimum > 4.0);
        let a = Stage::for_track(33, "track-a");
        let b = Stage::for_track(33, "track-b");
        assert_ne!(a.scene.phase, b.scene.phase);
    }

    #[test]
    fn shimmer_is_subtle_asynchronous_and_frozen_when_suspended() {
        let mut stage = Stage::poster();
        let mut a = stage.bullets[0];
        let mut b = a;
        a.shimmer_phase = 0.0;
        a.age = 0.0;
        b.shimmer_phase = 2.0;
        let mut low = 1.0f32;
        let mut high = 0.0f32;
        let mut difference = 0.0f32;
        let mut previous = a.glow(false);
        for i in 0..1000 {
            a.age = i as f32 * 0.01;
            b.age = a.age;
            let glow = a.glow(false);
            low = low.min(glow);
            high = high.max(glow);
            difference = difference.max((glow - b.glow(false)).abs());
            assert!((0.83..=1.0).contains(&glow));
            assert!((glow - previous).abs() < 0.007);
            assert_eq!(a.glow(true), 1.0);
            previous = glow;
        }
        assert!(high - low > 0.1 && difference > 0.1);
        let before: Vec<_> = stage.bullets.iter().map(|b| b.glow(false)).collect();
        stage.suspend(&MusicalFeatures::default());
        let after: Vec<_> = stage.bullets.iter().map(|b| b.glow(false)).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn simulation_is_independent_of_render_rate_and_drops_suspended_events() {
        let music = MusicalFeatures {
            level: 0.6,
            intensity: 0.8,
            ..Default::default()
        };
        let mut slow = Stage::default();
        let mut fast = Stage::default();
        for _ in 0..240 {
            slow.advance(1.0 / 30.0, &music);
        }
        for _ in 0..480 {
            fast.advance(1.0 / 60.0, &music);
        }
        assert_eq!(slow.bullets.len(), fast.bullets.len());
        assert!((slow.time - fast.time).abs() < 0.001);
        for (a, b) in slow.bullets.iter().zip(&fast.bullets) {
            assert!((a.x - b.x).abs() < 0.001 && (a.y - b.y).abs() < 0.001);
        }
        let time = slow.time;
        let music = MusicalFeatures {
            beats: 40,
            accents: 25,
            ..music
        };
        slow.suspend(&music);
        assert_eq!(slow.time, time);
        slow.advance(STEP, &music);
        fast.advance(STEP, &music);
        assert_eq!(fast.beats, slow.beats + 1);
    }
}
