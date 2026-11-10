//! Composable scores chosen at musical boundaries, never randomized per frame.
//! Formation and attack phrases have independent lifetimes and recent-history
//! exclusions. A seed makes long performances reproducible for diagnosis.
use super::{danmaku::BulletKind, spectrum::MusicalFeatures};
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Formation {
    Crossing,
    Roaming,
    Orbit,
    FigureEight,
    Pendulum,
    Rain,
    Satellites,
    Flanks,
}
impl Formation {
    const ALL: [Self; 8] = [
        Self::Crossing,
        Self::Roaming,
        Self::Orbit,
        Self::FigureEight,
        Self::Pendulum,
        Self::Rain,
        Self::Satellites,
        Self::Flanks,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Fan,
    Spiral,
    Flower,
    Polygon,
    Weave,
    Curtain,
    BrokenRing,
    Tangent,
    Chevron,
    Echo,
    Cross,
    Fountain,
}
impl Pattern {
    const ALL: [Self; 12] = [
        Self::Fan,
        Self::Spiral,
        Self::Flower,
        Self::Polygon,
        Self::Weave,
        Self::Curtain,
        Self::BrokenRing,
        Self::Tangent,
        Self::Chevron,
        Self::Echo,
        Self::Cross,
        Self::Fountain,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Fan => "追踪扇面",
            Self::Spiral => "多臂螺旋",
            Self::Flower => "花轮绽放",
            Self::Polygon => "棱镜星阵",
            Self::Weave => "交织波纹",
            Self::Curtain => "流星帘幕",
            Self::BrokenRing => "缺口轮舞",
            Self::Tangent => "切线回旋",
            Self::Chevron => "雁形分流",
            Self::Echo => "重环回声",
            Self::Cross => "十字风车",
            Self::Fountain => "弧光喷泉",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rhythm {
    Flow,
    Stagger,
    Burst,
    Answer,
    Syncopated,
}

#[derive(Clone, Copy)]
pub struct Scene {
    pub formation: Formation,
    pub duration: f32,
    pub count: usize,
    pub mirror: f32,
    pub rate: f32,
    pub radius: f32,
    pub phase: f32,
    pub color: usize,
    pub points: [(f32, f32); 4],
}

#[derive(Clone, Copy)]
pub struct Phrase {
    pub primary: Pattern,
    pub accent: Pattern,
    pub kinds: [BulletKind; 2],
    pub rhythm: Rhythm,
    pub duration: f32,
    pub interval: f32,
    pub arms: usize,
    pub count: usize,
    pub lobes: f32,
    pub speed: f32,
    pub spread: f32,
    pub spin: f32,
    pub turn: f32,
    pub offset: f32,
    pub beat_divider: u64,
    pub lasers: bool,
}
impl Phrase {
    fn signature(&self) -> u64 {
        self.primary as u64
            | (self.accent as u64) << 4
            | (self.kinds[0] as u64) << 8
            | (self.kinds[1] as u64) << 12
            | (self.rhythm as u64) << 16
    }
}

struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = self.0;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / 16777216.0
    }
    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + self.unit() * (high - low)
    }
    fn index(&mut self, count: usize) -> usize {
        (self.unit() * count as f32) as usize
    }
}

pub struct Director {
    random: Random,
    formations: [usize; 2],
    patterns: [usize; 3],
    formation_age: [u16; 8],
    pattern_age: [u16; 12],
    combinations: [u64; 16],
}
impl Director {
    pub fn new(seed: u64) -> Self {
        Self {
            random: Random(seed),
            formations: [usize::MAX; 2],
            patterns: [usize::MAX; 3],
            formation_age: [0; 8],
            pattern_age: [0; 12],
            combinations: [u64::MAX; 16],
        }
    }

    fn choose<const N: usize>(
        random: &mut Random,
        ages: &[u16; N],
        excluded: &[usize],
        biases: [f32; N],
    ) -> usize {
        let weights: [f32; N] = std::array::from_fn(|i| {
            if excluded.contains(&i) {
                0.0
            } else {
                (1.0 + ages[i] as f32 * 0.14) * biases[i]
            }
        });
        let mut pick = random.unit() * weights.iter().sum::<f32>();
        for (i, weight) in weights.into_iter().enumerate() {
            if weight > 0.0 && pick < weight {
                return i;
            }
            pick -= weight;
        }
        (0..N).find(|i| !excluded.contains(i)).unwrap_or(0)
    }

    pub fn scene(&mut self, music: &MusicalFeatures) -> Scene {
        let energetic = music.intensity > 0.65;
        let biases = if energetic {
            [1.3, 0.7, 1.1, 1.2, 0.8, 1.3, 1.0, 1.1]
        } else {
            [0.8, 1.5, 1.3, 1.0, 1.4, 0.6, 1.1, 0.8]
        };
        let index = Self::choose(
            &mut self.random,
            &self.formation_age,
            &self.formations,
            biases,
        );
        self.formations.rotate_right(1);
        self.formations[0] = index;
        self.formation_age
            .iter_mut()
            .for_each(|age| *age = (*age + 1).min(100));
        self.formation_age[index] = 0;
        let formation = Formation::ALL[index];
        let count = match formation {
            Formation::Roaming => 1,
            Formation::Satellites => 3,
            Formation::Flanks => 2,
            Formation::Rain | Formation::Crossing => 3 + self.random.index(3),
            _ => 2 + self.random.index(3),
        };
        Scene {
            formation,
            duration: self.random.range(9.0, 18.0),
            count,
            mirror: if self.random.index(2) == 0 { -1.0 } else { 1.0 },
            rate: self.random.range(0.35, 0.85),
            radius: self.random.range(0.55, 1.0),
            phase: self.random.range(0.0, TAU),
            color: self.random.index(4),
            points: std::array::from_fn(|_| {
                (
                    self.random.range(-0.65, 0.65),
                    self.random.range(0.12, 0.65),
                )
            }),
        }
    }

    pub fn phrase(&mut self, music: &MusicalFeatures) -> Phrase {
        const KINDS: [BulletKind; 8] = [
            BulletKind::Orb,
            BulletKind::Needle,
            BulletKind::Petal,
            BulletKind::Rice,
            BulletKind::Star,
            BulletKind::Ring,
            BulletKind::Kunai,
            BulletKind::Butterfly,
        ];
        let mut biases = [1.0; 12];
        if music.high > music.low + 0.06 {
            biases[Pattern::Weave as usize] = 2.0;
            biases[Pattern::Curtain as usize] = 1.7;
        }
        if music.low > music.high + 0.06 {
            biases[Pattern::Echo as usize] = 2.0;
            biases[Pattern::Flower as usize] = 1.7;
        }
        let primary = Self::choose(&mut self.random, &self.pattern_age, &self.patterns, biases);
        self.patterns.rotate_right(1);
        self.patterns[0] = primary;
        self.pattern_age
            .iter_mut()
            .for_each(|age| *age = (*age + 1).min(100));
        self.pattern_age[primary] = 0;
        // Coarse combinations also have a cooldown, beyond the primary pattern.
        // 17 distinct candidates against 16 recent combinations guarantees a
        // free choice, without an unbounded reroll loop on the rendering thread.
        let start = self.random.index(11 * 8 * 7 * 5);
        for attempt in 0..=self.combinations.len() {
            let mut code = (start + attempt) % (11 * 8 * 7 * 5);
            let accent = (primary + 1 + code % 11) % 12;
            code /= 11;
            let kind = code % 8;
            code /= 8;
            let other_kind = (kind + 1 + code % 7) % 8;
            code /= 7;
            let phrase = Phrase {
                primary: Pattern::ALL[primary],
                accent: Pattern::ALL[accent],
                kinds: [KINDS[kind], KINDS[other_kind]],
                rhythm: [
                    Rhythm::Flow,
                    Rhythm::Stagger,
                    Rhythm::Burst,
                    Rhythm::Answer,
                    Rhythm::Syncopated,
                ][code % 5],
                duration: self.random.range(3.2, 7.5),
                interval: self.random.range(0.25, 0.65),
                arms: 3 + self.random.index(6),
                count: 14 + self.random.index(23),
                lobes: (3 + self.random.index(6)) as f32,
                speed: self.random.range(0.25, 0.54),
                spread: self.random.range(0.055, 0.18),
                spin: self.random.range(0.35, 1.5)
                    * if self.random.index(2) == 0 { -1.0 } else { 1.0 },
                turn: self.random.range(-0.28, 0.28),
                offset: self.random.range(0.0, TAU),
                beat_divider: if music.intensity > 0.7 {
                    1 + self.random.index(2) as u64
                } else {
                    2 + self.random.index(3) as u64
                },
                lasers: music.intensity > 0.35 && self.random.unit() < 0.28,
            };
            let signature = phrase.signature();
            if !self.combinations.contains(&signature) {
                self.combinations.rotate_right(1);
                self.combinations[0] = signature;
                return phrase;
            }
        }
        unreachable!("17 distinct combinations cannot all occur in a 16-entry history")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeded_scores_are_reproducible_and_audio_changes_the_selection() {
        let low = MusicalFeatures {
            low: 0.8,
            mid: 0.5,
            high: 0.1,
            intensity: 0.75,
            ..Default::default()
        };
        let high = MusicalFeatures {
            low: 0.1,
            high: 0.8,
            ..low
        };
        let mut first = Director::new(42);
        let mut same = Director::new(42);
        let mut changed = Director::new(42);
        let mut differences = 0;
        let mut low_choices = [0; 2];
        let mut high_choices = [0; 2];
        for _ in 0..512 {
            let a = first.phrase(&low);
            let b = same.phrase(&low);
            let c = changed.phrase(&high);
            assert_eq!(a.signature(), b.signature());
            assert_eq!(a.duration, b.duration);
            differences += usize::from(a.signature() != c.signature());
            low_choices[0] += usize::from(matches!(a.primary, Pattern::Flower | Pattern::Echo));
            low_choices[1] += usize::from(matches!(c.primary, Pattern::Flower | Pattern::Echo));
            high_choices[0] += usize::from(matches!(a.primary, Pattern::Weave | Pattern::Curtain));
            high_choices[1] += usize::from(matches!(c.primary, Pattern::Weave | Pattern::Curtain));
        }
        assert!(differences > 0);
        assert!(
            low_choices[0] > low_choices[1],
            "low-frequency bias: {low_choices:?}"
        );
        assert!(
            high_choices[1] > high_choices[0],
            "high-frequency bias: {high_choices:?}"
        );
    }

    #[test]
    fn long_scores_exclude_recent_choices_and_reach_the_full_vocabulary() {
        let mut director = Director::new(7);
        let music = MusicalFeatures {
            intensity: 0.8,
            ..Default::default()
        };
        let mut scenes = Vec::new();
        let mut phrases = Vec::new();
        let mut combinations = Vec::new();
        for _ in 0..240 {
            let scene = director.scene(&music);
            assert!(!scenes.iter().rev().take(2).any(|s| *s == scene.formation));
            scenes.push(scene.formation);
            let phrase = director.phrase(&music);
            assert!(!phrases.iter().rev().take(3).any(|p| *p == phrase.primary));
            assert!(!combinations
                .iter()
                .rev()
                .take(16)
                .any(|p| *p == phrase.signature()));
            assert_ne!(phrase.primary, phrase.accent);
            assert_ne!(phrase.kinds[0], phrase.kinds[1]);
            phrases.push(phrase.primary);
            combinations.push(phrase.signature());
        }
        assert!(Formation::ALL.iter().all(|f| scenes.contains(f)));
        assert!(Pattern::ALL.iter().all(|p| phrases.contains(p)));
    }
}
