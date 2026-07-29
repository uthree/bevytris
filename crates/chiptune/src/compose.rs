//! The composer: turns a seed and a gameplay context into a stream of
//! timestamped notes.
//!
//! The shape follows MetaCompose (Scirea et al. 2017): harmony first, then
//! melody against it, then accompaniment by pattern rules, with "mood" as
//! an orthogonal post-process rather than something baked into the
//! material. Two rules do most of the work of making it sound like music
//! rather than like a random note generator:
//!
//! * **Material is generated once per session and then re-ordered, never
//!   re-rolled.** Measured on real pop corpora, ~79% of songs are only
//!   15-35% new material; generated music sounds bad from too *much*
//!   novelty far more often than from too little. Melodies live in a pool
//!   of four one-bar motifs over at most eight rhythm cells.
//! * **Cadences are scheduled, not discovered.** The last bar of every
//!   four-bar phrase is overridden with V (antecedent) or I (consequent).
//!   A weighted chord walk produces plausible local motion and completely
//!   unconvincing phrase endings, so it never gets a vote on those.
//!
//! Everything is stored as scale degrees, so the danger level can darken
//! the mode from Ionian all the way to Phrygian dominant without a single
//! note being regenerated.

use crate::rng::Rng;
use crate::theory::{Chord, Mode, NOTE_NAMES, euclid_rot};
use crate::{FRAME_RATE, Inst, NoteEvent, SAMPLE_RATE};

pub const BARS_PER_SECTION: u32 = 8;

/// Time signature, rolled once per piece. Three and six both fit twelve
/// sixteenths in a bar; what separates them is where the accents fall,
/// which is why they are distinct variants rather than one "12 steps".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Meter {
    /// 4/4 — four beats of four sixteenths.
    Four,
    /// 3/4 — three beats of four sixteenths.
    Three,
    /// 6/8 — two dotted-quarter beats of six sixteenths.
    Six,
}

impl Meter {
    pub fn steps(self) -> u32 {
        match self {
            Meter::Four => 16,
            _ => 12,
        }
    }
    /// Felt beats per bar. Six-eight is felt in two, not six.
    pub fn beats(self) -> u32 {
        self.steps() / self.steps_per_beat()
    }
    /// Sixteenths per felt beat.
    pub fn steps_per_beat(self) -> u32 {
        match self {
            Meter::Four | Meter::Three => 4,
            Meter::Six => 6,
        }
    }
    /// Quarter notes per bar — what the piano roll's bar lines count.
    pub fn quarters_per_bar(self) -> u32 {
        self.steps() / 4
    }
    /// Where the backbeat lands. A general "every beat but the first"
    /// rule would put a snare on beat 3 of a 4/4 bar, which is wrong.
    fn snare_steps(self) -> &'static [u8] {
        match self {
            Meter::Four => &[4, 12],
            Meter::Three => &[4, 8],
            Meter::Six => &[6],
        }
    }
    /// Compound time already has its own lilt; swinging it on top just
    /// muddies the grouping.
    fn swings(self) -> bool {
        self != Meter::Six
    }
    /// Six-eight counted in sixteenths runs hot, so it gets pulled back
    /// a little; the waltz slightly less.
    fn tempo_scale(self) -> f32 {
        match self {
            Meter::Four => 1.0,
            Meter::Three => 0.95,
            Meter::Six => 0.86,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Meter::Four => "4/4",
            Meter::Three => "3/4",
            Meter::Six => "6/8",
        }
    }
    /// 0 = downbeat, 1 = the offbeat subdivision, 2 = everything else.
    fn prio(self, step: u8) -> u8 {
        if step as u32 % self.steps_per_beat() == 0 {
            0
        } else if step % 2 == 0 {
            1
        } else {
            2
        }
    }
}

/// Which of the four musical personalities is playing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Profile {
    /// Menus and the title screen: slow, sparse, no drums.
    Ambient = 0,
    /// Solo play: bright modes, moderate tempo, room to breathe.
    SoloCalm = 1,
    /// Versus: minor modes, fast, busy.
    VsIntense = 2,
    /// The result screen.
    Victory = 3,
}

/// Everything the composer is allowed to know about the game.
#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub profile: Profile,
    /// 0..1, already smoothed by the caller.
    pub intensity: f32,
    /// Semitones of level-up transposition.
    pub transpose: i32,
    /// The player's zone is running: thin the arrangement right out.
    pub zone: bool,
    /// Seconds since this profile started, for the layer schedule.
    pub elapsed: f32,
}

impl Default for Context {
    fn default() -> Self {
        Context {
            profile: Profile::Ambient,
            intensity: 0.0,
            transpose: 0,
            zone: false,
            elapsed: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Material
// ---------------------------------------------------------------------------

/// The rhythm cell bank. Every motif draws from this, which is what caps
/// the session's rhythmic vocabulary at eight patterns — real music uses
/// far fewer distinct onset patterns than the space allows, and that
/// scarcity is what makes a phrase recognizable when it comes back.
/// Every cell is deliberately short of filling its bar: the player is
/// generating their own rhythmic sound effects and needs the acoustic
/// space, and a melody with no rests reads as nagging within a minute.
const RHYTHMS_4_4: [&[(u8, u8)]; 8] = [
    &[(0, 3), (4, 3), (8, 3), (12, 3)],
    &[(0, 2), (2, 2), (4, 3), (8, 2), (10, 2), (12, 3)],
    &[(0, 3), (3, 3), (6, 2), (8, 3), (12, 3)],
    &[(0, 4), (6, 2), (8, 4), (12, 2), (14, 2)],
    &[(0, 2), (2, 2), (4, 2), (6, 2), (8, 3), (12, 3)],
    &[(0, 6), (6, 2), (8, 5), (14, 2)],
    &[(0, 7), (8, 4), (12, 3)],
    &[(0, 2), (3, 1), (4, 2), (7, 1), (8, 2), (11, 1), (12, 3)],
];

/// 3/4 — three groups of four.
const RHYTHMS_3_4: [&[(u8, u8)]; 6] = [
    &[(0, 3), (4, 3), (8, 3)],
    &[(0, 2), (2, 2), (4, 3), (8, 3)],
    &[(0, 3), (3, 1), (4, 3), (8, 2), (10, 1)],
    &[(0, 4), (6, 2), (8, 3)],
    &[(0, 2), (2, 2), (4, 2), (6, 2), (8, 3)],
    &[(0, 6), (8, 3)],
];

/// 6/8 — two groups of three eighths. Onsets stay on even steps so the
/// compound lilt survives.
const RHYTHMS_6_8: [&[(u8, u8)]; 6] = [
    &[(0, 2), (2, 2), (4, 2), (6, 2), (8, 2), (10, 1)],
    &[(0, 4), (4, 2), (6, 4), (10, 1)],
    &[(0, 2), (4, 2), (6, 2), (10, 1)],
    &[(0, 5), (6, 5)],
    &[(0, 2), (2, 2), (4, 1), (6, 2), (8, 2), (10, 1)],
    &[(0, 6), (6, 2), (8, 2), (10, 1)],
];

fn rhythm_bank(meter: Meter) -> &'static [&'static [(u8, u8)]] {
    match meter {
        Meter::Four => &RHYTHMS_4_4,
        Meter::Three => &RHYTHMS_3_4,
        Meter::Six => &RHYTHMS_6_8,
    }
}

/// I-V-vi-IV and friends. Degrees are 0-indexed, so 4 is V and 5 is vi
/// (or bVI in a minor mode — the same numbers work in every mode, which
/// is the whole point of storing degrees). Only the victory fanfare uses
/// these; everything else stays in a minor mode, which is both cooler and
/// closer to the genre.
const CALM_LOOPS: [[i32; 4]; 4] = [
    [0, 4, 5, 3],
    [5, 3, 0, 4],
    [0, 3, 4, 0],
    [0, 5, 3, 4],
];
/// The minor set. `[0, 6, 5, 4]` is the Andalusian cadence — the harmonic
/// cousin of the Korobeiniki idiom, so it reads as genre-correct without
/// quoting anything.
const DARK_LOOPS: [[i32; 4]; 6] = [
    [0, 3, 4, 0],
    [0, 5, 2, 6],
    [0, 6, 5, 4],
    [0, 1, 4, 0],
    [0, 2, 5, 6],
    [0, 6, 3, 4],
];

#[derive(Clone, Copy, Debug)]
struct MelNote {
    step: u8,
    len: u8,
    deg: i32,
    /// 0 = downbeat, 1 = offbeat eighth, 2 = sixteenth. Thinning the
    /// melody for the calm profile drops high priorities first, which is
    /// deterministic and keeps the phrase recognizable.
    prio: u8,
}

#[derive(Clone)]
struct Motif {
    rhythm: usize,
    contour: Vec<i32>,
}

struct Section {
    chords: [Chord; 8],
    melody: Vec<Vec<MelNote>>,
    hat_rot: usize,
    kick_rot: usize,
}

struct Material {
    sections: Vec<Section>,
    form: Vec<usize>,
    /// Which rhythm cells this piece drew. Nothing reads it at runtime;
    /// it exists so the "at most eight cells" invariant is testable.
    #[allow(dead_code)]
    rhythms_used: Vec<usize>,
}

/// Weighted meter roll. 4/4 stays the house style — the odd meters are a
/// change of scenery, and versus in particular needs a floor to stand on
/// more than it needs novelty.
fn roll_meter(profile: Profile, rng: &mut Rng) -> Meter {
    let w: [f32; 3] = match profile {
        Profile::Victory => [1.0, 0.0, 0.0],
        Profile::Ambient => [0.55, 0.25, 0.20],
        Profile::SoloCalm => [0.66, 0.17, 0.17],
        Profile::VsIntense => [0.78, 0.07, 0.15],
    };
    [Meter::Four, Meter::Three, Meter::Six][rng.weighted(&w)]
}

/// Antecedent/consequent plan: motif 0 opens three of the four phrases,
/// which is the cheapest possible guarantee of audible repetition.
const MELODY_PLAN: [(usize, Transform); 8] = [
    (0, Transform::None),
    (1, Transform::None),
    (0, Transform::None),
    (2, Transform::None),
    (0, Transform::None),
    (1, Transform::Up),
    (3, Transform::None),
    (2, Transform::Cadence),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Transform {
    None,
    /// Sequence the motif up a third.
    Up,
    /// Land on the tonic — used only on the last bar of a section.
    Cadence,
}

fn build_material(seed: u64, profile: Profile, meter: Meter) -> Material {
    let mut rng = Rng::new(seed ^ ((profile as u64 + 1) * 0x9E37_79B9));
    // Everything but the fanfare lives in minor. Major progressions read
    // as cheerful, and cheerful is the wrong register for a stack that is
    // about to top out.
    let dark = !matches!(profile, Profile::Victory);
    let bank = rhythm_bank(meter);

    // Four motifs, each on its own rhythm cell.
    let mut rhythms_used: Vec<usize> = Vec::new();
    let mut motifs: Vec<Motif> = Vec::new();
    while motifs.len() < 4 {
        let r = rng.below(bank.len());
        if rhythms_used.contains(&r) {
            continue;
        }
        rhythms_used.push(r);
        let n = bank[r].len();
        motifs.push(Motif {
            rhythm: r,
            contour: contour(&mut rng, n),
        });
    }

    let loops: &[[i32; 4]] = if dark { &DARK_LOOPS } else { &CALM_LOOPS };
    let mut sections = Vec::new();
    for _ in 0..3 {
        let base = *rng.pick(loops);
        let mut chords = [Chord::new(0); 8];
        for (i, c) in chords.iter_mut().enumerate() {
            *c = Chord::new(base[i % 4]);
        }
        // Scheduled cadences: half at the end of the antecedent phrase,
        // authentic at the end of the consequent.
        chords[3] = Chord::new(4);
        chords[7] = Chord::new(0);
        // A seventh on the dominant sharpens the pull home.
        chords[3].seventh = true;

        let melody = (0..8)
            .map(|bar| {
                let (mi, tf) = MELODY_PLAN[bar];
                realize(&motifs[mi], tf, chords[bar], meter)
            })
            .collect();

        sections.push(Section {
            chords,
            melody,
            hat_rot: rng.below(4),
            kick_rot: 0,
        });
    }

    Material {
        sections,
        // Three sections of material carry eighty bars, with section A
        // returning every other block — 30% new material, which is where
        // real songs sit.
        form: vec![0, 0, 1, 0, 2, 0, 1, 0, 2, 0],
        rhythms_used,
    }
}

/// A one-bar pitch contour: a short weighted walk with a bias back toward
/// the middle of the register, so motifs neither drift away nor sit still.
fn contour(rng: &mut Rng, n: usize) -> Vec<i32> {
    let mut out = Vec::with_capacity(n);
    let mut d = *rng.pick(&[0, 2, 4]);
    for _ in 0..n {
        out.push(d);
        // Steps are far more common than leaps; leaps larger than a fifth
        // never happen.
        let step = match rng.weighted(&[0.30, 0.30, 0.16, 0.16, 0.04, 0.04]) {
            0 => -1,
            1 => 1,
            2 => -2,
            3 => 2,
            4 => -4,
            _ => 4,
        };
        d += step;
        // Pull back into a comfortable octave and a bit.
        if d > 8 {
            d -= 7;
        }
        if d < -2 {
            d += 7;
        }
    }
    out
}

/// Turn a motif into the actual notes of one bar over `chord`.
fn realize(motif: &Motif, tf: Transform, chord: Chord, meter: Meter) -> Vec<MelNote> {
    let rhythm = rhythm_bank(meter)[motif.rhythm];
    let shift = match tf {
        Transform::None | Transform::Cadence => 0,
        Transform::Up => 2,
    };
    let mut out = Vec::with_capacity(rhythm.len());
    for (i, &(step, len)) in rhythm.iter().enumerate() {
        let mut deg = motif.contour[i.min(motif.contour.len() - 1)] + shift;
        let prio = meter.prio(step);
        // Strong beats must be chord tones: this single rule is the
        // difference between "a melody over the changes" and "notes".
        if prio == 0 {
            deg = chord.snap(deg);
        }
        out.push(MelNote {
            step,
            len,
            deg,
            prio,
        });
    }
    if tf == Transform::Cadence {
        // Land the phrase on the tonic, in whatever octave it was already
        // heading for.
        if let Some(last) = out.last_mut() {
            last.deg = (last.deg as f32 / 7.0).round() as i32 * 7;
            // Hold it a little longer, but never past the bar line.
            last.len = (last.len + 2).min(meter.steps() as u8 - last.step);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Per-bar arrangement knobs
// ---------------------------------------------------------------------------

fn band(intensity: f32) -> usize {
    if intensity < 0.30 {
        0
    } else if intensity < 0.62 {
        1
    } else if intensity < 0.82 {
        2
    } else {
        3
    }
}

/// Tempo is rolled **once per piece and never moves again.**
///
/// It used to climb with the danger level, NES-Tetris style. That is a
/// real convention and it does raise the stakes, but a tempo that shifts
/// under you is genuinely hard to play along with, and a puzzle game is
/// something you play *along with* for half an hour at a stretch. The
/// intensity signal still has plenty of voice: mode, note density, drum
/// density, bass figure, duty cycle and which layers are audible all
/// keep moving. Only the pulse holds still.
fn tempo_range(profile: Profile) -> (f32, f32) {
    match profile {
        Profile::Ambient => (92.0, 104.0),
        Profile::Victory => (128.0, 138.0),
        Profile::SoloCalm => (122.0, 144.0),
        Profile::VsIntense => (178.0, 206.0),
    }
}

fn roll_tempo(profile: Profile, meter: Meter, rng: &mut Rng) -> f32 {
    let (lo, hi) = tempo_range(profile);
    (lo + rng.unit() * (hi - lo)) * meter.tempo_scale()
}

/// Mode ladder, bright to dark. The tonic never moves, so the bass never
/// has to know this happened.
///
/// Every playing profile is minor: Dorian is the cool, unhurried one
/// (its major sixth keeps it from sounding sad), Aeolian is the plain
/// one, harmonic minor sharpens the pull home, and Phrygian's flat second
/// is the darkest diatonic mode there is — saved for real danger. Only
/// the victory fanfare goes major.
///
/// Phrygian dominant is deliberately unused here despite fitting the
/// mood: its tonic triad is *major*, and one bright chord is enough to
/// undo the whole effect.
fn mode_target(profile: Profile, intensity: f32) -> Mode {
    match profile {
        Profile::Ambient => Mode::Dorian,
        Profile::Victory => Mode::Lydian,
        Profile::SoloCalm => [
            Mode::Dorian,
            Mode::Aeolian,
            Mode::Aeolian,
            Mode::HarmonicMinor,
        ][band(intensity)],
        Profile::VsIntense => [
            Mode::Aeolian,
            Mode::Aeolian,
            Mode::HarmonicMinor,
            Mode::Phrygian,
        ][band(intensity)],
    }
}

/// What the third pulse channel does when it is switched on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Counter {
    /// A quiet delayed copy of the melody, panned opposite it.
    Echo,
    /// A fast chord arpeggio in sixteenths.
    Arp,
}

/// What the sawtooth channel does when it is switched on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SawRole {
    /// Doubles the bass an octave up, for weight.
    BassDouble,
    /// Doubles the melody's strong beats an octave down, for body.
    LeadDouble,
}

struct Arrangement {
    lead: bool,
    harmony: bool,
    perc: bool,
    /// Wavetable pad or bell under everything.
    pad: Option<Inst>,
    counter: Option<Counter>,
    saw: Option<SawRole>,
    /// Sixteenth-note shakers on top of the hats.
    shaker: bool,
    lead_inst: Inst,
    harm_inst: Inst,
    bass_pat: usize,
    hat_k: usize,
    kick_k: usize,
    /// Four-on-the-floor: a kick on every quarter, unconditionally, with
    /// [`Arrangement::kick_extra`] syncopations layered on top rather
    /// than replacing the pulse. A Euclidean kick is more interesting but
    /// it wanders off the beat, and versus wants a floor to stand on.
    four_floor: bool,
    kick_extra: &'static [u8],
    snare: bool,
    swing: f32,
    max_prio: u8,
}

fn arrange(profile: Profile, ctx: &Context) -> Arrangement {
    let i = ctx.intensity.clamp(0.0, 1.0);
    let b = band(i);
    // High intensity pulls the layer schedule forward: a desperate board
    // should not have to wait a minute for the drums.
    let t = ctx.elapsed * (1.0 + 2.0 * i);
    let (h_at, p_at, l_at) = match profile {
        Profile::Ambient => (6.0, f32::INFINITY, 22.0),
        Profile::SoloCalm => (16.0, 40.0, 62.0),
        Profile::VsIntense => (4.0, 10.0, 20.0),
        Profile::Victory => (0.0, 0.0, 0.0),
    };

    // The extra channels come in last, so the arrangement still builds
    // rather than arriving all at once.
    let mut a = match profile {
        Profile::Ambient => Arrangement {
            lead: t >= l_at,
            harmony: t >= h_at,
            perc: false,
            pad: Some(Inst::Bell),
            counter: (t >= l_at + 18.0).then_some(Counter::Echo),
            saw: None,
            shaker: false,
            lead_inst: Inst::Soft,
            harm_inst: Inst::Pad,
            bass_pat: 0,
            hat_k: 0,
            kick_k: 0,
            four_floor: false,
            kick_extra: &[],
            snare: false,
            swing: 0.58,
            max_prio: 0,
        },
        Profile::SoloCalm => Arrangement {
            lead: t >= l_at,
            harmony: t >= h_at,
            perc: t >= p_at,
            pad: (t >= h_at).then_some(if i < 0.55 {
                Inst::WaveOrgan
            } else {
                Inst::Glass
            }),
            counter: (t >= l_at + 14.0).then_some(Counter::Echo),
            saw: (i >= 0.66).then_some(SawRole::LeadDouble),
            shaker: i >= 0.82,
            lead_inst: if i < 0.45 { Inst::Soft } else { Inst::Sustain },
            harm_inst: if i < 0.62 { Inst::Pad } else { Inst::Organ },
            bass_pat: [0, 1, 2, 3][b],
            hat_k: [0, 2, 4, 8][b],
            kick_k: [2, 2, 3, 4][b],
            four_floor: false,
            kick_extra: &[],
            snare: i >= 0.50,
            swing: 0.56,
            max_prio: if i < 0.35 { 0 } else { 1 },
        },
        Profile::VsIntense => Arrangement {
            lead: t >= l_at,
            harmony: t >= h_at,
            perc: t >= p_at,
            pad: (t >= h_at + 6.0).then_some(Inst::WaveBass),
            counter: (t >= l_at + 10.0).then_some(if i < 0.5 {
                Counter::Echo
            } else {
                Counter::Arp
            }),
            saw: (i >= 0.30).then_some(SawRole::BassDouble),
            shaker: i >= 0.62,
            lead_inst: Inst::Pluck,
            harm_inst: if i < 0.55 { Inst::Organ } else { Inst::Stab },
            bass_pat: [1, 2, 2, 4][b],
            hat_k: [4, 8, 11, 13][b],
            kick_k: 4,
            four_floor: true,
            // Pickups layered onto the pulse as it heats up.
            kick_extra: [&[][..], &[14], &[7, 14], &[7, 11, 14]][b],
            snare: true,
            swing: 0.52,
            max_prio: if i < 0.30 { 1 } else { 2 },
        },
        Profile::Victory => Arrangement {
            lead: true,
            harmony: true,
            perc: true,
            pad: Some(Inst::Bell),
            counter: Some(Counter::Arp),
            saw: Some(SawRole::BassDouble),
            shaker: false,
            lead_inst: Inst::Sustain,
            harm_inst: Inst::Organ,
            bass_pat: 1,
            hat_k: 4,
            kick_k: 4,
            four_floor: true,
            kick_extra: &[],
            snare: true,
            swing: 0.54,
            max_prio: 1,
        },
    };

    // The zone treatment, straight out of Tetris Effect: pull the lead and
    // the drums, leave the harmony and bass, and let the filter close.
    // Losing the melody is what makes the moment feel like held breath.
    if ctx.zone {
        a.lead = false;
        a.perc = false;
        a.counter = None;
        a.saw = None;
        a.shaker = false;
        a.harm_inst = Inst::Pad;
        a.pad = Some(Inst::Glass);
    }
    a
}

// ---------------------------------------------------------------------------
// Composer
// ---------------------------------------------------------------------------

/// What the "now playing" toast prints, and what the piano roll needs to
/// draw its scale banding.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub seed: u64,
    pub tonic: i32,
    pub mode: Mode,
    pub meter: Meter,
    pub bpm: f32,
    pub bar: u32,
}

impl Info {
    /// e.g. `A minor · 6/8 · 148 BPM · #3f7a2c`
    pub fn label(&self) -> String {
        format!(
            "{} {} · {} · {:.0} BPM · #{:06x}",
            NOTE_NAMES[(self.tonic.rem_euclid(12)) as usize],
            self.mode.name(),
            self.meter.name(),
            self.bpm,
            self.seed & 0xff_ffff
        )
    }
}

pub struct Composer {
    seed: u64,
    profile: Profile,
    material: Material,
    mode: Mode,
    /// MIDI note of the tonic, in the harmony register.
    root: i32,
    /// Fixed for the whole piece — see [`tempo_range`].
    bpm: f32,
    meter: Meter,
    /// Counts profile activations, so each match rolls its own meter,
    /// tempo and material instead of replaying the last one.
    piece: u64,
    bar: u32,
    next_bar_at: u64,
    transpose: i32,
}

impl Composer {
    pub fn new(seed: u64) -> Self {
        let profile = Profile::Ambient;
        let root = 45 + (seed % 12) as i32;
        Composer {
            seed,
            profile,
            material: build_material(seed, profile, Meter::Four),
            mode: mode_target(profile, 0.0),
            root,
            bpm: roll_tempo(profile, Meter::Four, &mut Rng::new(seed)),
            meter: Meter::Four,
            piece: 0,
            bar: 0,
            next_bar_at: 0,
            transpose: 0,
        }
    }

    pub fn info(&self) -> Info {
        Info {
            seed: self.seed,
            tonic: self.root + self.transpose,
            mode: self.mode,
            meter: self.meter,
            bpm: self.bpm,
            bar: self.bar,
        }
    }

    pub fn profile(&self) -> Profile {
        self.profile
    }

    pub fn next_bar_at(&self) -> u64 {
        self.next_bar_at
    }

    /// Start a new piece with a different personality. Meter, tempo and
    /// material are all rolled here and then held for the whole piece —
    /// each match gets its own tune, but nothing shifts underneath the
    /// player mid-song.
    pub fn set_profile(&mut self, profile: Profile, intensity: f32) {
        if profile == self.profile {
            return;
        }
        self.piece += 1;
        let mut rng = Rng::new(self.seed ^ (self.piece.wrapping_mul(0x1000_0193)));
        self.profile = profile;
        self.meter = roll_meter(profile, &mut rng);
        self.bpm = roll_tempo(profile, self.meter, &mut rng);
        self.material = build_material(
            self.seed ^ self.piece.wrapping_mul(0xA24B_AED4),
            profile,
            self.meter,
        );
        self.mode = mode_target(profile, intensity);
        self.bar = 0;
    }

    /// Override the meter that [`Composer::set_profile`] rolled, and
    /// rebuild the material to match. Only the offline renderer uses
    /// this, to audition a time signature on demand.
    pub fn force_meter(&mut self, meter: Meter) {
        if meter == self.meter {
            return;
        }
        self.meter = meter;
        self.bpm = roll_tempo(
            self.profile,
            meter,
            &mut Rng::new(self.seed ^ self.piece.wrapping_mul(0x1000_0193)),
        );
        self.material = build_material(
            self.seed ^ self.piece.wrapping_mul(0xA24B_AED4),
            self.profile,
            meter,
        );
        self.bar = 0;
    }

    /// Drop the playhead at `at` — used once, when the audio stream starts.
    pub fn seek(&mut self, at: u64) {
        self.next_bar_at = at;
    }

    /// Plan bars until the score reaches `lookahead` samples past
    /// `playhead`. Emitted events are appended in time order.
    pub fn advance(
        &mut self,
        playhead: u64,
        lookahead: u64,
        ctx: &Context,
        out: &mut Vec<NoteEvent>,
    ) {
        if self.next_bar_at < playhead {
            // The stream ran ahead of us (a long frame hitch, or the very
            // first bar). Re-anchor rather than dumping a backlog of notes
            // that are already in the past.
            self.next_bar_at = playhead;
        }
        let mut guard = 0;
        while self.next_bar_at < playhead + lookahead && guard < 16 {
            self.plan_bar(ctx, out);
            guard += 1;
        }
    }

    fn plan_bar(&mut self, ctx: &Context, out: &mut Vec<NoteEvent>) {
        self.transpose = ctx.transpose;
        let phrase_start = self.bar % BARS_PER_SECTION == 0;
        if phrase_start {
            // Mode changes only at phrase boundaries; mid-bar they sound
            // like a bug rather than a modulation.
            self.mode = mode_target(self.profile, ctx.intensity);
        }
        // Tempo and meter are fixed for the whole piece — see tempo_range.
        let meter = self.meter;
        let steps = meter.steps();
        let arr = arrange(self.profile, ctx);
        let spb = samples_per_step(self.bpm);
        let fps = frames_per_step(self.bpm);
        let bar_at = self.next_bar_at;
        let start = out.len();

        let block = (self.bar / BARS_PER_SECTION) as usize % self.material.form.len();
        let sec_idx = self.material.form[block];
        let bar_in = (self.bar % BARS_PER_SECTION) as usize;
        let last_bar = bar_in == BARS_PER_SECTION as usize - 1;
        let (chord, hat_rot, kick_rot, melody) = {
            let sec = &self.material.sections[sec_idx];
            (
                sec.chords[bar_in],
                sec.hat_rot,
                sec.kick_rot,
                sec.melody[bar_in].clone(),
            )
        };

        let tonic = self.root + ctx.transpose;
        let mode = self.mode;
        let at = |step: f32| -> u64 {
            let swung = if meter.swings() && (step as i32) % 2 == 1 {
                step + (arr.swing - 0.5) * 2.0
            } else {
                step
            };
            bar_at + (swung as f64 * spb) as u64
        };
        let frames = |len: u8| -> u16 { ((len as f32 * fps) as u16).max(2) };

        // --- lead -----------------------------------------------------
        if arr.lead {
            for n in melody.iter().filter(|n| n.prio <= arr.max_prio) {
                out.push(NoteEvent {
                    at: at(n.step as f32),
                    inst: arr.lead_inst,
                    midi: clamp_midi(tonic + 24 + mode.pitch(n.deg)),
                    vel: if n.prio == 0 { 112 } else { 88 },
                    frames: frames(n.len),
                    arp: None,
                });
            }
        }

        // --- harmony --------------------------------------------------
        if arr.harmony {
            let arp = chord.arp(mode);
            let base = clamp_midi(tonic + 12 + mode.pitch(chord.degree));
            let spbeat = meter.steps_per_beat() as u8;
            if arr.harm_inst == Inst::Stab {
                // Offbeat stabs drive; a held pad would just sit there.
                // One per beat, halfway through it, whatever the meter.
                for beat in 0..meter.beats() as u8 {
                    out.push(NoteEvent {
                        at: at((beat * spbeat + spbeat / 2) as f32),
                        inst: Inst::Stab,
                        midi: base,
                        vel: 96,
                        frames: frames(2),
                        arp: Some(arp),
                    });
                }
            } else {
                let half = (steps / 2) as u8;
                for step in [0, half] {
                    out.push(NoteEvent {
                        at: at(step as f32),
                        inst: arr.harm_inst,
                        midi: base,
                        vel: 84,
                        frames: frames(half),
                        arp: Some(arp),
                    });
                }
            }
        }

        // --- counter-melody -------------------------------------------
        match arr.counter {
            Some(Counter::Echo) if arr.lead => {
                // Three sixteenths behind the lead and panned to the far
                // side of it, which is what turns a doubling into space.
                for n in melody.iter().filter(|n| n.prio <= arr.max_prio) {
                    let step = n.step as f32 + 3.0;
                    if step >= steps as f32 {
                        continue;
                    }
                    out.push(NoteEvent {
                        at: at(step),
                        inst: Inst::Echo,
                        midi: clamp_midi(tonic + 24 + mode.pitch(n.deg)),
                        vel: 62,
                        frames: frames(n.len.min(3)),
                        arp: None,
                    });
                }
            }
            Some(Counter::Arp) => {
                let tones = chord.tones();
                for i in (0..steps).step_by(1) {
                    let deg = tones[i as usize % 3] + 7;
                    out.push(NoteEvent {
                        at: at(i as f32),
                        inst: Inst::Arp,
                        midi: clamp_midi(tonic + 12 + mode.pitch(deg)),
                        vel: 72,
                        frames: frames(1),
                        arp: None,
                    });
                }
            }
            _ => {}
        }

        // --- wavetable pad --------------------------------------------
        if let Some(pad) = arr.pad {
            out.push(NoteEvent {
                at: at(0.0),
                inst: pad,
                midi: clamp_midi(tonic + mode.pitch(chord.degree)),
                vel: 76,
                frames: frames(steps as u8),
                arp: Some(chord.arp(mode)),
            });
        }

        // --- bass -----------------------------------------------------
        let bass = bass_pattern(arr.bass_pat, chord.degree, meter);
        for &(step, len, deg) in &bass {
            out.push(NoteEvent {
                at: at(step as f32),
                inst: Inst::Bass,
                midi: clamp_midi(tonic - 12 + mode.pitch(deg)),
                vel: 110,
                frames: frames(len),
                arp: None,
            });
        }

        // --- sawtooth -------------------------------------------------
        match arr.saw {
            Some(SawRole::BassDouble) => {
                for &(step, len, deg) in &bass {
                    out.push(NoteEvent {
                        at: at(step as f32),
                        inst: Inst::SawBass,
                        midi: clamp_midi(tonic + mode.pitch(deg)),
                        vel: 88,
                        frames: frames(len),
                        arp: None,
                    });
                }
            }
            Some(SawRole::LeadDouble) if arr.lead => {
                for n in melody.iter().filter(|n| n.prio == 0) {
                    out.push(NoteEvent {
                        at: at(n.step as f32),
                        inst: Inst::SawLead,
                        midi: clamp_midi(tonic + 12 + mode.pitch(n.deg)),
                        vel: 80,
                        frames: frames(n.len),
                        arp: None,
                    });
                }
            }
            _ => {}
        }

        // --- drums ----------------------------------------------------
        if arr.perc {
            // The unconditional fill in the last bar of every section is
            // what makes a loop feel like it has form.
            // The onset counts were tuned against sixteen steps, so an
            // odd meter scales them rather than thinning out.
            let scale = |k: usize| (k * steps as usize).div_ceil(16);
            let hat_k = if last_bar { arr.hat_k + 4 } else { arr.hat_k };
            let spbeat = meter.steps_per_beat() as usize;
            for (i, on) in euclid_rot(scale(hat_k), steps as usize, hat_rot)
                .iter()
                .enumerate()
            {
                if *on {
                    out.push(NoteEvent {
                        at: at(i as f32),
                        inst: Inst::Hat,
                        midi: 0,
                        vel: if i % spbeat == 0 { 96 } else { 70 },
                        frames: 4,
                        arp: None,
                    });
                }
            }
            let mut kick_mask = vec![false; steps as usize];
            if arr.four_floor {
                // One per felt beat: four in 4/4, three in a waltz, two
                // in 6/8.
                for step in (0..steps as usize).step_by(spbeat) {
                    kick_mask[step] = true;
                }
            } else {
                for (i, on) in euclid_rot(scale(arr.kick_k), steps as usize, kick_rot)
                    .iter()
                    .enumerate()
                {
                    kick_mask[i] = *on;
                }
            }
            for &step in arr.kick_extra {
                kick_mask[step as usize % steps as usize] = true;
            }
            for (i, on) in kick_mask.iter().enumerate() {
                if *on {
                    out.push(NoteEvent {
                        at: at(i as f32),
                        inst: Inst::Kick,
                        // Pickups sit under the four-on-the-floor pulse
                        // rather than competing with it.
                        vel: if i % spbeat == 0 { 120 } else { 92 },
                        midi: 0,
                        frames: 6,
                        arp: None,
                    });
                }
            }
            if arr.shaker {
                // Sixteenths on the second noise channel, filling in
                // between the hats without stealing them.
                for i in (1..steps as usize).step_by(2) {
                    out.push(NoteEvent {
                        at: at(i as f32),
                        inst: Inst::Shaker,
                        midi: 0,
                        vel: 58,
                        frames: 3,
                        arp: None,
                    });
                }
            }
            if arr.snare {
                for &step in meter.snare_steps() {
                    out.push(NoteEvent {
                        at: at(step as f32),
                        inst: Inst::Snare,
                        midi: 0,
                        vel: 104,
                        frames: 8,
                        arp: None,
                    });
                }
            }
            if last_bar {
                for k in 0..3u8 {
                    out.push(NoteEvent {
                        at: at((steps as u8 - 3 + k) as f32),
                        inst: Inst::Snare,
                        midi: 0,
                        vel: 90 + 12 * k,
                        frames: 3,
                        arp: None,
                    });
                }
            }
        }

        out[start..].sort_by_key(|e| e.at);
        self.next_bar_at = bar_at + (spb * steps as f64) as u64;
        self.bar += 1;
    }
}

fn clamp_midi(m: i32) -> u8 {
    m.clamp(12, 108) as u8
}

fn samples_per_step(bpm: f32) -> f64 {
    60.0 / bpm as f64 / 4.0 * SAMPLE_RATE as f64
}

fn frames_per_step(bpm: f32) -> f32 {
    60.0 / bpm / 4.0 * FRAME_RATE as f32
}

/// `(step, length, scale degree)` triples. Degrees are relative to the
/// tonic, so +4 is the chord's fifth and +7 is an octave.
///
/// Everything is expressed per beat or per subdivision rather than at
/// fixed step numbers, so the same five figures work in 4/4, 3/4 and 6/8.
fn bass_pattern(pat: usize, root: i32, meter: Meter) -> Vec<(u8, u8, i32)> {
    let steps = meter.steps() as u8;
    let spb = meter.steps_per_beat() as u8;
    match pat {
        // Whole bar.
        0 => vec![(0, steps, root)],
        // Root, then the fifth halfway through.
        1 => {
            let half = steps / 2;
            vec![(0, half, root), (half, half, root + 4)]
        }
        // Octave-jumping eighths.
        2 => (0..steps / 2)
            .map(|i| {
                let deg = if i % 2 == 0 { root } else { root + 7 };
                (i * 2, 2u8, deg)
            })
            .collect(),
        // Root-fifth-octave-fifth, one per beat.
        3 => (0..steps / spb)
            .map(|i| (i * spb, spb, root + [0, 4, 7, 4][i as usize % 4]))
            .collect(),
        // Arpeggiated sixteenths.
        _ => (0..steps)
            .map(|i| (i, 1u8, root + [0, 2, 4, 7][i as usize % 4]))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn run(profile: Profile, intensity: f32, bars: u32) -> Vec<NoteEvent> {
        let mut c = Composer::new(0xBEEF);
        c.set_profile(profile, intensity);
        let ctx = Context {
            profile,
            intensity,
            elapsed: 600.0,
            ..Default::default()
        };
        let mut out = Vec::new();
        for _ in 0..bars {
            c.plan_bar(&ctx, &mut out);
        }
        out
    }

    #[test]
    fn events_come_out_in_time_order() {
        let out = run(Profile::VsIntense, 0.7, 64);
        assert!(!out.is_empty());
        for w in out.windows(2) {
            assert!(w[0].at <= w[1].at, "events out of order");
        }
    }

    const METERS: [Meter; 3] = [Meter::Four, Meter::Three, Meter::Six];

    #[test]
    fn a_piece_uses_at_most_eight_rhythm_cells() {
        for p in [Profile::Ambient, Profile::SoloCalm, Profile::VsIntense] {
            for m in METERS {
                let mat = build_material(12345, p, m);
                assert!(
                    mat.rhythms_used.len() <= 8,
                    "{p:?} {} used {} rhythm cells",
                    m.name(),
                    mat.rhythms_used.len()
                );
                assert_eq!(mat.rhythms_used.iter().collect::<HashSet<_>>().len(), 4);
            }
        }
    }

    #[test]
    fn every_phrase_ends_on_a_cadence() {
        for meter in METERS {
            let m = build_material(999, Profile::SoloCalm, meter);
            for s in &m.sections {
                assert_eq!(s.chords[3].degree, 4, "antecedent must close on V");
                assert!(s.chords[3].seventh);
                assert_eq!(s.chords[7].degree, 0, "consequent must close on I");
            }
        }
    }

    #[test]
    fn strong_beats_are_chord_tones() {
        let mut checked = 0;
        for meter in METERS {
            let m = build_material(555, Profile::SoloCalm, meter);
            for s in &m.sections {
                for (bar, notes) in s.melody.iter().enumerate() {
                    for n in notes.iter().filter(|n| n.prio == 0) {
                        assert!(
                            s.chords[bar].contains(n.deg),
                            "downbeat degree {} is not in chord {:?}",
                            n.deg,
                            s.chords[bar]
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 100, "test did not actually look at much");
    }

    #[test]
    fn material_is_reused_not_re_rolled() {
        // The form must revisit sections: eighty bars of music from
        // twenty-four bars of material keeps new material inside the
        // ~35% budget real songs sit in.
        let m = build_material(7, Profile::VsIntense, Meter::Four);
        let unique = m.form.iter().collect::<HashSet<_>>().len();
        assert_eq!(unique, 3);
        let heard = m.form.len() * BARS_PER_SECTION as usize;
        let written = unique * BARS_PER_SECTION as usize;
        assert!(
            written as f32 / heard as f32 <= 0.35 + 1e-3,
            "too much unique material"
        );
    }

    #[test]
    fn the_melody_always_leaves_room_to_breathe() {
        // Every bar must have rests: the player is generating their own
        // rhythmic SFX and needs the acoustic space.
        for meter in METERS {
            let m = build_material(31337, Profile::VsIntense, meter);
            for s in &m.sections {
                for notes in &s.melody {
                    let covered: u32 = notes.iter().map(|n| n.len as u32).sum();
                    assert!(
                        covered < meter.steps(),
                        "a {} bar is completely full",
                        meter.name()
                    );
                    // Nothing may hang over the bar line either.
                    for n in notes {
                        assert!(n.step as u32 + n.len as u32 <= meter.steps());
                    }
                }
            }
        }
    }

    #[test]
    fn meters_are_internally_consistent() {
        for m in METERS {
            assert_eq!(m.beats() * m.steps_per_beat(), m.steps());
            assert_eq!(m.quarters_per_bar() * 4, m.steps());
            for &s in m.snare_steps() {
                assert!((s as u32) < m.steps());
                assert_eq!(s as u32 % m.steps_per_beat(), 0, "snare off the beat");
            }
        }
        assert_eq!(Meter::Four.beats(), 4);
        assert_eq!(Meter::Three.beats(), 3);
        // Six-eight is felt in two, not six.
        assert_eq!(Meter::Six.beats(), 2);
    }

    #[test]
    fn odd_meters_show_up_but_stay_the_exception() {
        let mut seen = [0u32; 3];
        for seed in 0..400u64 {
            let mut rng = Rng::new(seed);
            match roll_meter(Profile::SoloCalm, &mut rng) {
                Meter::Four => seen[0] += 1,
                Meter::Three => seen[1] += 1,
                Meter::Six => seen[2] += 1,
            }
        }
        assert!(seen.iter().all(|&n| n > 20), "a meter never came up: {seen:?}");
        assert!(seen[0] > seen[1] + seen[2], "4/4 should stay the house style");
        // Versus keeps a floor to stand on far more often.
        let mut vs_four = 0;
        for seed in 0..400u64 {
            if roll_meter(Profile::VsIntense, &mut Rng::new(seed)) == Meter::Four {
                vs_four += 1;
            }
        }
        assert!(vs_four > 280, "versus wandered off 4/4 too often: {vs_four}");
        // The fanfare is never in an odd meter.
        for seed in 0..64u64 {
            assert_eq!(
                roll_meter(Profile::Victory, &mut Rng::new(seed)),
                Meter::Four
            );
        }
    }

    #[test]
    fn tempo_never_moves_inside_a_piece() {
        // The whole point of the fixed tempo: a groove you can play along
        // with for half an hour.
        let mut c = Composer::new(0xC0FFEE);
        c.set_profile(Profile::VsIntense, 0.0);
        let started = c.bpm;
        let mut out = Vec::new();
        for i in 0..64 {
            let ctx = Context {
                profile: Profile::VsIntense,
                // Swing the danger level violently across the whole range.
                intensity: if i % 2 == 0 { 0.05 } else { 0.98 },
                elapsed: 600.0,
                ..Default::default()
            };
            c.plan_bar(&ctx, &mut out);
            assert_eq!(c.bpm, started, "tempo moved at bar {i}");
        }
        // Bars must therefore all be exactly the same length.
        let bar = (samples_per_step(started) * c.meter.steps() as f64) as u64;
        assert!(bar > 0);
    }

    #[test]
    fn each_new_piece_rolls_fresh_material() {
        let mut c = Composer::new(0x1234);
        let mut seen = Vec::new();
        for _ in 0..6 {
            c.set_profile(Profile::SoloCalm, 0.3);
            seen.push((c.bpm, c.meter, c.material.rhythms_used.clone()));
            c.set_profile(Profile::Ambient, 0.0);
        }
        assert!(
            seen.iter().any(|s| *s != seen[0]),
            "every match got the identical tune"
        );
    }

    #[test]
    fn calm_is_slower_and_brighter_than_versus() {
        assert!(tempo_range(Profile::SoloCalm).1 < tempo_range(Profile::VsIntense).0);
        // Everything that plays during a match is minor; only the
        // fanfare is allowed to be cheerful.
        for p in [Profile::Ambient, Profile::SoloCalm, Profile::VsIntense] {
            for i in [0.0f32, 0.5, 1.0] {
                assert!(mode_target(p, i).is_minor(), "{p:?} at {i} went major");
            }
        }
        assert!(!mode_target(Profile::Victory, 0.0).is_minor());
        // Solo is still the brighter of the two minors: Dorian's major
        // sixth is what keeps it from sounding grim.
        assert!(
            mode_target(Profile::SoloCalm, 0.0).pitch(5)
                > mode_target(Profile::VsIntense, 0.0).pitch(5)
        );
    }

    #[test]
    fn versus_kicks_on_every_beat() {
        let ctx = Context {
            profile: Profile::VsIntense,
            intensity: 0.9,
            elapsed: 600.0,
            ..Default::default()
        };
        assert!(arrange(Profile::VsIntense, &ctx).four_floor);

        // Check it in the emitted score, not just the flag — and in every
        // meter, since "four on the floor" means three in a waltz.
        for seed in [1u64, 2, 3, 5, 8, 13] {
            let mut c = Composer::new(seed);
            c.set_profile(Profile::VsIntense, 0.9);
            let mut out = Vec::new();
            for _ in 0..8 {
                c.plan_bar(&ctx, &mut out);
            }
            let spb = samples_per_step(c.bpm);
            let bar_len = (spb * c.meter.steps() as f64) as u64;
            let kicks: Vec<u64> = out
                .iter()
                .filter(|e| e.inst == Inst::Kick)
                .map(|e| e.at)
                .collect();
            let beats = c.meter.beats();
            assert!(
                kicks.len() >= 8 * beats as usize,
                "{} kicks in 8 bars of {}",
                kicks.len(),
                c.meter.name()
            );
            for beat in 0..beats as u64 {
                let want = (beat as f64 * c.meter.steps_per_beat() as f64 * spb) as u64;
                assert!(
                    kicks
                        .iter()
                        .any(|&k| (k % bar_len).abs_diff(want) < spb as u64 / 2),
                    "no kick on beat {} of {}",
                    beat + 1,
                    c.meter.name()
                );
            }
        }
    }

    #[test]
    fn only_the_lead_bends_its_long_notes() {
        use crate::inst_def;
        for i in [Inst::Pluck, Inst::Sustain, Inst::Soft] {
            let d = inst_def(i);
            assert!(d.fall > 0.0, "{i:?} should fall");
            assert!(d.vib_depth > 0.0 && d.vib_delay > 0, "{i:?} should wobble");
        }
        // A chord bed or a bass line that slides just sounds broken.
        for i in [
            Inst::Organ,
            Inst::Pad,
            Inst::Stab,
            Inst::Bass,
            Inst::Arp,
            Inst::SawBass,
            Inst::WaveOrgan,
            Inst::WaveBass,
            Inst::Bell,
        ] {
            assert_eq!(inst_def(i).fall, 0.0, "{i:?} must not fall");
            assert_eq!(inst_def(i).vib_depth, 0.0, "{i:?} must not wobble");
        }
    }

    #[test]
    fn the_extra_channels_arrive_late_and_leave_during_the_zone() {
        let ctx = |elapsed: f32, intensity: f32, zone: bool| Context {
            profile: Profile::VsIntense,
            intensity,
            elapsed,
            zone,
            ..Default::default()
        };
        // Nothing but the foundation at the start of a match.
        let early = arrange(Profile::VsIntense, &ctx(0.0, 0.1, false));
        assert!(early.counter.is_none() && early.saw.is_none() && !early.perc);
        // Everything by the time it has been running a while and the
        // stack is high.
        let late = arrange(Profile::VsIntense, &ctx(120.0, 0.9, false));
        assert!(late.counter.is_some() && late.saw.is_some() && late.pad.is_some());
        assert!(late.shaker && late.perc && late.lead);
        // The zone strips it back to a held chord over the bass.
        let zone = arrange(Profile::VsIntense, &ctx(120.0, 0.9, true));
        assert!(!zone.lead && !zone.perc && !zone.shaker);
        assert!(zone.counter.is_none() && zone.saw.is_none());
        assert!(zone.harmony && zone.pad.is_some());
    }

    #[test]
    fn a_full_arrangement_reaches_every_voice() {
        use std::collections::HashSet;
        let out = run(Profile::Victory, 0.9, 8);
        let voices: HashSet<u8> = out.iter().map(|e| e.voice() as u8).collect();
        assert_eq!(
            voices.len(),
            crate::VOICE_COUNT,
            "only {} of {} voices used: {voices:?}",
            voices.len(),
            crate::VOICE_COUNT
        );
    }

    #[test]
    fn versus_is_busier_than_solo() {
        let solo = run(Profile::SoloCalm, 0.5, 32).len();
        let vs = run(Profile::VsIntense, 0.5, 32).len();
        assert!(vs > solo, "VS ({vs}) should out-note solo ({solo})");
    }

    #[test]
    fn zone_thins_the_arrangement_out() {
        let ctx = Context {
            profile: Profile::VsIntense,
            intensity: 0.8,
            elapsed: 600.0,
            zone: true,
            ..Default::default()
        };
        let a = arrange(Profile::VsIntense, &ctx);
        assert!(!a.lead && !a.perc && a.harmony);
    }

    #[test]
    fn composition_is_deterministic() {
        let one = run(Profile::VsIntense, 0.55, 40);
        let two = run(Profile::VsIntense, 0.55, 40);
        assert_eq!(one.len(), two.len());
        for (a, b) in one.iter().zip(&two) {
            assert_eq!((a.at, a.midi, a.vel, a.frames), (b.at, b.midi, b.vel, b.frames));
        }
    }

    #[test]
    fn every_note_is_playable() {
        for p in [
            Profile::Ambient,
            Profile::SoloCalm,
            Profile::VsIntense,
            Profile::Victory,
        ] {
            for i in [0.0f32, 0.4, 0.8, 1.0] {
                for e in run(p, i, 24) {
                    // Drums carry no pitch, so only the tonal voices are
                    // checked against the playable range.
                    if !e.inst.is_drum() {
                        assert!((21..=108).contains(&e.midi), "{p:?} emitted midi {}", e.midi);
                    }
                    assert!(e.frames >= 2);
                    assert!(e.vel > 0);
                }
            }
        }
    }

    #[test]
    fn advance_fills_the_lookahead_and_stops() {
        let mut c = Composer::new(1);
        c.set_profile(Profile::SoloCalm, 0.3);
        let ctx = Context {
            profile: Profile::SoloCalm,
            elapsed: 600.0,
            ..Default::default()
        };
        let mut out = Vec::new();
        c.advance(0, 2 * SAMPLE_RATE as u64, &ctx, &mut out);
        assert!(!out.is_empty());
        assert!(c.next_bar_at() >= 2 * SAMPLE_RATE as u64);
        let planned = out.len();
        // A second call with the playhead unmoved must add nothing.
        c.advance(0, 2 * SAMPLE_RATE as u64, &ctx, &mut out);
        assert_eq!(out.len(), planned);
    }

    #[test]
    fn material_generation_is_reproducible() {
        for meter in METERS {
            let a = build_material(0xABCD, Profile::SoloCalm, meter);
            let b = build_material(0xABCD, Profile::SoloCalm, meter);
            assert_eq!(a.rhythms_used, b.rhythms_used);
            assert_eq!(a.form, b.form);
        }
    }
}
