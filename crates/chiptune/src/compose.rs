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

/// How far the final chorus lifts the key. A whole tone is the classic
/// gear change — a semitone is subtler than the moment deserves, and
/// anything larger stops sounding like the same song.
const LIFT_STEP: i32 = 2;

/// Bars of outro after the final chorus. The song is a loop, so the key
/// has to come back down or an hour-long session ends up somewhere
/// absurd; the outro is what brings it home, and what makes the seam back
/// to bar one sound like a repeat rather than a restart.
///
/// The way back is a pivot, not a retreat. A whole tone above the home
/// key, the flat-seventh chord is built on the home tonic itself — so the
/// outro opens on that chord in the new key and simply lets the ear
/// reinterpret it as home, then cadences there. Four bars is enough to
/// land it and short enough not to drag.
const OUTRO_BARS: u32 = 4;

/// Chord per outro bar, and whether that bar is still in the lifted key.
/// Bar 0 is the pivot: flat-seventh of the lifted key, which sounds the
/// home tonic. Then iv - V7 - i at home.
const OUTRO: [(i32, bool, bool); OUTRO_BARS as usize] = [
    // (scale degree, still lifted, dominant seventh)
    (6, true, false),
    (3, false, false),
    (4, false, true),
    (0, false, false),
];

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

    pub const ALL: [Meter; 3] = [Meter::Four, Meter::Three, Meter::Six];
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
    /// Zen: the slowest and most open writing here. Nothing in that mode
    /// can be lost, so the music has nothing to warn about — it brightens
    /// when the field is clear and only clouds over as the stack climbs.
    Zen = 4,
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
const CALM_LOOPS: [[i32; 4]; 4] = [[0, 4, 5, 3], [5, 3, 0, 4], [0, 3, 4, 0], [0, 5, 3, 4]];
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

/// Which drum kit a piece uses. The chip kit is the console's noise
/// channel doing its best; the sampled kit swaps the kick and snare for
/// synthesized one-shots with real bodies and keeps the chip hats on top,
/// which is what tracker musicians did the moment they had the memory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kit {
    Chip,
    Sampled,
}

impl Kit {
    fn kick(self) -> Inst {
        match self {
            Kit::Chip => Inst::Kick,
            Kit::Sampled => Inst::PcmKick,
        }
    }
    fn snare(self, clap: bool) -> Inst {
        match (self, clap) {
            (Kit::Chip, _) => Inst::Snare,
            (Kit::Sampled, false) => Inst::PcmSnare,
            (Kit::Sampled, true) => Inst::Clap,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Kit::Chip => "chip",
            Kit::Sampled => "pcm",
        }
    }

    pub const ALL: [Kit; 2] = [Kit::Chip, Kit::Sampled];
}

impl Profile {
    pub const ALL: [Profile; 5] = [
        Profile::Ambient,
        Profile::SoloCalm,
        Profile::VsIntense,
        Profile::Victory,
        Profile::Zen,
    ];

    /// Short English name, for the listening screen.
    pub fn name(self) -> &'static str {
        match self {
            Profile::Ambient => "AMBIENT",
            Profile::SoloCalm => "SOLO",
            Profile::VsIntense => "VERSUS",
            Profile::Victory => "VICTORY",
            Profile::Zen => "ZEN",
        }
    }
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
        // Zen is the one place a lopsided bar is an asset: there is no
        // stack to react to on the beat, so 3/4 and 6/8 just float.
        Profile::Zen => [0.40, 0.30, 0.30],
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

/// How stepwise each profile's melodies are by default (see [`contour`]).
/// Everything here leans smooth: a puzzle game is background listening,
/// and a line that keeps jumping registers is the part of background
/// listening people notice for the wrong reason.
pub fn default_smoothness(profile: Profile) -> f32 {
    match profile {
        Profile::Zen => 0.90,
        Profile::Ambient => 0.75,
        Profile::SoloCalm => 0.70,
        // The two that are allowed some angularity: a fanfare wants
        // fanfare intervals, and versus wants energy.
        Profile::Victory => 0.50,
        Profile::VsIntense => 0.50,
    }
}

fn build_material(seed: u64, profile: Profile, meter: Meter, smoothness: f32) -> Material {
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
            contour: contour(&mut rng, n, smoothness),
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

/// Bounds of the melodic register, in scale degrees, and the degree the
/// walk is drawn back toward.
const CONTOUR_LOW: i32 = -2;
const CONTOUR_HIGH: i32 = 8;
const CONTOUR_CENTER: i32 = 3;

/// A one-bar pitch contour: a short weighted walk, shaped by `smoothness`
/// (0 = the widest intervals this writes, 1 = very nearly stepwise).
///
/// Two things here decide whether a motif sounds *shaped* or merely
/// restless. The interval weights are the obvious one. The other is what
/// happens at the edge of the register: this used to wrap (`d -= 7`),
/// which dropped a leap of a seventh between two consecutive notes and
/// was by far the biggest source of a melody that would not sit still.
/// It now turns around instead, and drifts back toward the middle on its
/// own — so a motif arches rather than wanders.
fn contour(rng: &mut Rng, n: usize, smoothness: f32) -> Vec<i32> {
    let s = smoothness.clamp(0.0, 1.0);
    // Smoothness moves weight out of the wide intervals and into seconds.
    // The three pairs always sum to 1: seconds 0.60 -> 0.88, thirds
    // 0.32 -> 0.12, fifths 0.08 -> 0.
    let second = 0.30 + 0.14 * s;
    let third = 0.16 - 0.10 * s;
    let fifth = 0.04 - 0.04 * s;
    let weights = [second, second, third, third, fifth, fifth];

    let mut out = Vec::with_capacity(n);
    let mut d = *rng.pick(&[0, 2, 4]);
    for _ in 0..n {
        out.push(d);
        let mut step: i32 = match rng.weighted(&weights) {
            0 => -1,
            1 => 1,
            2 => -2,
            3 => 2,
            4 => -4,
            _ => 4,
        };
        // The further the walk has drifted from the middle, the likelier
        // the next move is inward. A walk that only ever turns at the
        // edges reads as wandering; one that leans home reads as a line.
        let offset = d - CONTOUR_CENTER;
        if offset != 0 && offset.signum() == step.signum() {
            let pull = offset.abs() as f32 / (CONTOUR_HIGH - CONTOUR_CENTER) as f32;
            if rng.chance(pull * (0.35 + 0.35 * s)) {
                step = -step;
            }
        }
        // Turn around at the register edge rather than jumping an octave.
        if d + step > CONTOUR_HIGH || d + step < CONTOUR_LOW {
            step = -step;
        }
        d = (d + step).clamp(CONTOUR_LOW, CONTOUR_HIGH);
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
        let covered: u32 = out.iter().map(|n| n.len as u32).sum();
        if let Some(last) = out.last_mut() {
            last.deg = (last.deg as f32 / 7.0).round() as i32 * 7;
            // Hold it a little longer, but never past the bar line, and
            // never so long that the bar ends up with no rest at all.
            // The player is generating their own rhythm on top of this
            // and needs the acoustic space; a cadence that swallows the
            // last step takes it away exactly where the phrase breathes.
            let room_to_bar = (meter.steps() - last.step as u32).max(1);
            let others = covered - last.len as u32;
            let room_for_rest = (meter.steps() - 1).saturating_sub(others).max(1);
            last.len = (last.len as u32 + 2).min(room_to_bar).min(room_for_rest) as u8;
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
        // Slower than the menus, which is to say slower than anything
        // else the composer writes.
        Profile::Zen => (74.0, 88.0),
        Profile::Ambient => (92.0, 104.0),
        Profile::Victory => (128.0, 138.0),
        Profile::SoloCalm => (122.0, 144.0),
        Profile::VsIntense => (178.0, 206.0),
    }
}

/// Rolled per piece. Sampled drums are the exception rather than the
/// house style: they are a change of texture, and the chip kit is what
/// makes the rest of the arrangement sound like a chip.
fn roll_kit(profile: Profile, rng: &mut Rng) -> Kit {
    let sampled = match profile {
        Profile::Zen => 0.20,
        Profile::Ambient => 0.25,
        Profile::SoloCalm => 0.35,
        Profile::VsIntense => 0.45,
        Profile::Victory => 0.5,
    };
    if rng.chance(sampled) {
        Kit::Sampled
    } else {
        Kit::Chip
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
        // Zen inverts the usual reading of intensity. Nothing here is at
        // stake, so a clear field gets the brightest mode in the set and
        // a tall stack only shades it back toward minor — colour, not a
        // warning.
        Profile::Zen => [Mode::Lydian, Mode::Ionian, Mode::Dorian, Mode::Aeolian][band(intensity)],
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

impl Arrangement {
    /// Which layers are audible, as a bitmask. Purely for the debug
    /// readout — "the profile switched but you cannot hear it" and "the
    /// profile did not switch" look identical from outside, and this is
    /// what tells them apart.
    fn layers(&self) -> u8 {
        // The bass is unconditional, so it is always bit 0.
        let mut m = 1u8;
        for (bit, on) in [
            self.harmony,
            self.pad.is_some(),
            self.perc,
            self.lead,
            self.counter.is_some(),
            self.saw.is_some(),
            self.shaker,
        ]
        .iter()
        .enumerate()
        {
            if *on {
                m |= 1 << (bit + 1);
            }
        }
        m
    }
}

/// Names matching the bits of [`Arrangement::layers`].
pub const LAYER_NAMES: [&str; 8] = ["bass", "harm", "pad", "perc", "lead", "cntr", "saw", "shkr"];

/// Lead instruments each profile is allowed to sing with, one rolled per
/// piece. This is the main reason two pieces on the same profile do not
/// sound like the same song twice: the melody is the thing you follow,
/// so changing what plays it changes more than changing anything else.
///
/// The palettes are not interchangeable. Zen never picks anything with a
/// hard attack, versus never picks a mallet, and the fanfare only gets
/// instruments that can hold a note.
pub fn lead_palette(profile: Profile) -> &'static [Inst] {
    match profile {
        Profile::Ambient => &[Inst::Soft, Inst::Piano, Inst::Marimba],
        Profile::Zen => &[Inst::Soft, Inst::Piano, Inst::Guitar, Inst::Marimba],
        Profile::SoloCalm => &[
            Inst::Soft,
            Inst::Sustain,
            Inst::Pluck,
            Inst::Piano,
            Inst::Guitar,
            Inst::Marimba,
            Inst::Brass,
        ],
        Profile::VsIntense => &[
            Inst::Pluck,
            Inst::Sustain,
            Inst::Guitar,
            Inst::Brass,
            Inst::Piano,
        ],
        Profile::Victory => &[Inst::Sustain, Inst::Brass, Inst::Piano],
    }
}

/// Short name for the listening screen and the now-playing toast.
pub fn lead_name(inst: Inst) -> &'static str {
    match inst {
        Inst::Pluck => "pluck",
        Inst::Sustain => "sustain",
        Inst::Soft => "soft",
        Inst::Piano => "piano",
        Inst::Guitar => "guitar",
        Inst::Marimba => "marimba",
        Inst::Brass => "brass",
        _ => "lead",
    }
}

fn arrange(profile: Profile, ctx: &Context, lead: Inst) -> Arrangement {
    let i = ctx.intensity.clamp(0.0, 1.0);
    let b = band(i);
    // High intensity pulls the layer schedule forward: a desperate board
    // should not have to wait a minute for the drums.
    let t = ctx.elapsed * (1.0 + 2.0 * i);
    // Every piece starts from a complete-sounding base — bass, harmony
    // and pad from bar one — and builds percussion and melody on top.
    //
    // These used to be much longer, on the reasoning that a session-long
    // build is what Tetris Effect does. It is, but that music is
    // continuous; ours starts a *new song* at every screen change, and a
    // new song that opens on sixteen seconds of unaccompanied bass does
    // not sound like it began, it sounds like the old one stopped. The
    // build is still there, it just starts from a song.
    let (h_at, p_at, l_at) = match profile {
        Profile::Ambient => (0.0, f32::INFINITY, 12.0),
        Profile::SoloCalm => (0.0, 6.0, 16.0),
        Profile::VsIntense => (0.0, 2.0, 7.0),
        Profile::Victory => (0.0, 0.0, 0.0),
        // A soft pulse from the start (zen still wants a tempo to play
        // to) and a melody that takes its time arriving.
        Profile::Zen => (0.0, 4.0, 10.0),
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
            lead_inst: lead,
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
        Profile::Zen => Arrangement {
            lead: t >= l_at,
            harmony: t >= h_at,
            perc: t >= p_at,
            pad: Some(if i < 0.6 {
                Inst::Glass
            } else {
                Inst::WaveOrgan
            }),
            counter: (t >= l_at + 12.0).then_some(Counter::Echo),
            saw: None,
            shaker: false,
            lead_inst: lead,
            harm_inst: Inst::Pad,
            // The drums stay a texture rather than a beat: a sparse
            // Euclidean kick, no snare, no backbeat to nod along to.
            bass_pat: 0,
            hat_k: [0, 2, 3, 4][b],
            kick_k: [1, 2, 2, 3][b],
            four_floor: false,
            kick_extra: &[],
            snare: false,
            swing: 0.60,
            max_prio: 0,
        },
        Profile::SoloCalm => Arrangement {
            lead: t >= l_at,
            harmony: t >= h_at,
            perc: t >= p_at,
            pad: Some(if i < 0.55 {
                Inst::WaveOrgan
            } else {
                Inst::Glass
            }),
            counter: (t >= l_at + 14.0).then_some(Counter::Echo),
            saw: (i >= 0.66).then_some(SawRole::LeadDouble),
            shaker: i >= 0.82,
            lead_inst: lead,
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
            pad: Some(Inst::WaveBass),
            counter: (t >= l_at + 10.0).then_some(if i < 0.5 {
                Counter::Echo
            } else {
                Counter::Arp
            }),
            saw: (i >= 0.30).then_some(SawRole::BassDouble),
            shaker: i >= 0.62,
            lead_inst: lead,
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
            lead_inst: lead,
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
    /// Which piece this describes. A caller that has just asked for a new
    /// profile can use it to tell whether the engine has caught up yet —
    /// the snapshot it reads may still be about the outgoing piece.
    pub profile: Profile,
    pub tonic: i32,
    pub mode: Mode,
    pub meter: Meter,
    pub kit: Kit,
    /// What this piece is singing the melody with.
    pub lead: Inst,
    pub bpm: f32,
    pub bar: u32,
    /// Bitmask of the layers the last planned bar actually used; see
    /// [`LAYER_NAMES`].
    pub layers: u8,
}

impl Info {
    /// e.g. `bass harm pad perc` — what is currently audible.
    pub fn layer_list(&self) -> String {
        LAYER_NAMES
            .iter()
            .enumerate()
            .filter(|(i, _)| self.layers & (1 << i) != 0)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl Info {
    /// e.g. `A minor・6/8・148 BPM・#3f7a2c`
    ///
    /// The separator is the katakana middle dot rather than the Latin-1
    /// one, which is not a typographic quibble: the game draws this in an
    /// 8x8 JIS X 0208 pixel font, and U+00B7 is not in that set — it came
    /// out as a tofu box. See the test below.
    pub fn label(&self) -> String {
        format!(
            "{} {}・{}・{:.0} BPM・{}・#{:06x}",
            NOTE_NAMES[(self.tonic.rem_euclid(12)) as usize],
            self.mode.name(),
            self.meter.name(),
            self.bpm,
            lead_name(self.lead),
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
    kit: Kit,
    /// The instrument this piece sings its melody with, rolled per piece.
    lead: Inst,
    /// Set by [`Composer::force_lead`]; survives re-rolls.
    lead_override: Option<Inst>,
    /// How stepwise this piece's melodies are, 0..=1 (see [`contour`]).
    smoothness: f32,
    /// Set by [`Composer::force_smoothness`]; survives re-rolls, where
    /// `smoothness` goes back to the profile default.
    smooth_override: Option<f32>,
    /// Counts profile activations, so each match rolls its own meter,
    /// tempo and material instead of replaying the last one.
    piece: u64,
    bar: u32,
    next_bar_at: u64,
    transpose: i32,
    /// Semitones the final chorus has lifted the key by so far.
    lift: i32,
    /// Layer bitmask of the last bar planned, for the debug readout.
    layers: u8,
    /// Set on the bar where the lift lands, so the caller can re-announce
    /// the key.
    modulated: bool,
}

impl Composer {
    pub fn new(seed: u64) -> Self {
        let profile = Profile::Ambient;
        let root = 45 + (seed % 12) as i32;
        Composer {
            seed,
            profile,
            material: build_material(seed, profile, Meter::Four, default_smoothness(profile)),
            mode: mode_target(profile, 0.0),
            root,
            bpm: roll_tempo(profile, Meter::Four, &mut Rng::new(seed)),
            meter: Meter::Four,
            kit: Kit::Chip,
            lead: lead_palette(profile)[0],
            lead_override: None,
            smoothness: default_smoothness(profile),
            smooth_override: None,
            piece: 0,
            bar: 0,
            next_bar_at: 0,
            transpose: 0,
            lift: 0,
            layers: 1,
            modulated: false,
        }
    }

    /// True once, on the bar where the final chorus changes key.
    pub fn take_modulated(&mut self) -> bool {
        std::mem::take(&mut self.modulated)
    }

    pub fn info(&self) -> Info {
        Info {
            seed: self.seed,
            profile: self.profile,
            tonic: self.root + self.transpose + self.lift,
            mode: self.mode,
            meter: self.meter,
            kit: self.kit,
            lead: self.lead,
            bpm: self.bpm,
            bar: self.bar,
            layers: self.layers,
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
        self.kit = roll_kit(profile, &mut rng);
        self.smoothness = self
            .smooth_override
            .unwrap_or_else(|| default_smoothness(profile));
        let palette = lead_palette(profile);
        // An explicit pin wins over the palette — the palettes are taste,
        // and someone auditioning an instrument wants to hear it on
        // whatever preset they picked. It still has to belong to the lead
        // channel, or it would silently land on someone else's voice.
        self.lead = self
            .lead_override
            .filter(|i| i.voice() == crate::Voice::Lead)
            .unwrap_or_else(|| palette[rng.below(palette.len())]);
        self.material = build_material(
            self.seed ^ self.piece.wrapping_mul(0xA24B_AED4),
            profile,
            self.meter,
            self.smoothness,
        );
        self.mode = mode_target(profile, intensity);
        self.bar = 0;
        // A new song starts back in its own key.
        self.lift = 0;
    }

    /// Override the drum kit [`Composer::set_profile`] rolled. Used by
    /// the offline renderer to audition a kit on demand.
    pub fn force_kit(&mut self, kit: Kit) {
        self.kit = kit;
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
            self.smoothness,
        );
        self.bar = 0;
    }

    /// Override how stepwise the melodies are (0 = widest intervals,
    /// 1 = very nearly stepwise). `None` hands the choice back to the
    /// profile. Rebuilds the material, since the contours are baked in.
    pub fn force_smoothness(&mut self, smoothness: Option<f32>) {
        let want = smoothness
            .map(|s| s.clamp(0.0, 1.0))
            .unwrap_or_else(|| default_smoothness(self.profile));
        self.smooth_override = smoothness.map(|s| s.clamp(0.0, 1.0));
        if (want - self.smoothness).abs() < f32::EPSILON {
            return;
        }
        self.smoothness = want;
        self.material = build_material(
            self.seed ^ self.piece.wrapping_mul(0xA24B_AED4),
            self.profile,
            self.meter,
            want,
        );
        self.bar = 0;
    }

    pub fn smoothness(&self) -> f32 {
        self.smoothness
    }

    /// Override the instrument the melody is played on. `None` hands the
    /// choice back to the per-piece roll. Anything that is not a lead
    /// instrument is ignored rather than played on the wrong channel.
    pub fn force_lead(&mut self, lead: Option<Inst>) {
        let lead = lead.filter(|i| i.voice() == crate::Voice::Lead);
        if lead == self.lead_override {
            return;
        }
        self.lead_override = lead;
        let palette = lead_palette(self.profile);
        self.lead = lead.unwrap_or_else(|| {
            // Back to what this piece's own roll would have chosen.
            let mut rng = Rng::new(self.seed ^ (self.piece.wrapping_mul(0x1000_0193)));
            // Consume the same draws set_profile made before the lead.
            roll_meter(self.profile, &mut rng);
            roll_tempo(self.profile, self.meter, &mut rng);
            roll_kit(self.profile, &mut rng);
            palette[rng.below(palette.len())]
        });
    }

    pub fn lead(&self) -> Inst {
        self.lead
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

    /// Compose one bar, starting where the last one ended. Events are
    /// appended in time order with absolute-sample timestamps.
    pub fn plan_bar(&mut self, ctx: &Context, out: &mut Vec<NoteEvent>) {
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
        let spb = samples_per_step(self.bpm);
        let fps = frames_per_step(self.bpm);
        let bar_at = self.next_bar_at;
        let start = out.len();

        // The last block of the form is the final chorus: the key steps up
        // and the arrangement goes all in, then an outro brings it home so
        // the whole thing can loop. Menus and the short fanfare sit this
        // out — a title screen does not want a key change, and the fanfare
        // is over before a pass would finish.
        let big_finish = matches!(self.profile, Profile::SoloCalm | Profile::VsIntense);
        let blocks = self.material.form.len();
        let body_bars = blocks as u32 * BARS_PER_SECTION;
        let cycle_bars = body_bars + if big_finish { OUTRO_BARS } else { 0 };
        let cycle_bar = self.bar % cycle_bars;

        if cycle_bar >= body_bars {
            self.plan_outro(
                (cycle_bar - body_bars) as usize,
                ctx,
                out,
                meter,
                spb,
                fps,
                bar_at,
                start,
            );
            return;
        }

        let block = (cycle_bar / BARS_PER_SECTION) as usize % blocks;
        let sec_idx = self.material.form[block];
        let bar_in = (cycle_bar % BARS_PER_SECTION) as usize;
        let last_bar = bar_in == BARS_PER_SECTION as usize - 1;

        let final_block = big_finish && block == blocks - 1;
        // The bar immediately before it, where the run-up happens.
        let run_up = big_finish && block == blocks - 2 && last_bar;
        if final_block && bar_in == 0 && self.lift == 0 {
            self.lift = LIFT_STEP;
            self.modulated = true;
        }

        let mut arr = arrange(self.profile, ctx, self.lead);
        if final_block && !ctx.zone {
            // Everything at once, whatever the layer schedule had planned.
            arr.lead = true;
            arr.harmony = true;
            arr.perc = true;
            arr.counter = Some(Counter::Arp);
            arr.saw = Some(SawRole::BassDouble);
            arr.pad = arr.pad.or(Some(Inst::Bell));
            arr.shaker = true;
            arr.max_prio = arr.max_prio.max(1);
        }
        self.layers = arr.layers();
        let (chord, hat_rot, kick_rot, melody) = {
            let sec = &self.material.sections[sec_idx];
            (
                sec.chords[bar_in],
                sec.hat_rot,
                sec.kick_rot,
                sec.melody[bar_in].clone(),
            )
        };

        let tonic = self.root + ctx.transpose + self.lift;
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
            let mut prev: Option<i32> = None;
            for n in melody.iter().filter(|n| n.prio <= arr.max_prio) {
                // Scoop into big leaps only. A glide on every note is a
                // slide whistle; on a leap it is expression.
                let glide = match prev {
                    Some(p) if (n.deg - p).abs() >= 3 => 2,
                    _ => 0,
                };
                prev = Some(n.deg);
                out.push(NoteEvent {
                    at: at(n.step as f32),
                    inst: arr.lead_inst,
                    midi: clamp_midi(tonic + 24 + mode.pitch(n.deg)),
                    vel: if n.prio == 0 { 112 } else { 88 },
                    frames: frames(n.len),
                    arp: None,
                    glide,
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
                        glide: 0,
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
                        glide: 0,
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
                        glide: 0,
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
                        glide: 0,
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
                glide: 0,
            });
        }

        // --- bass -----------------------------------------------------
        let bass = bass_pattern(arr.bass_pat, chord.degree, meter);
        for (i, &(step, len, deg)) in bass.iter().enumerate() {
            out.push(NoteEvent {
                at: at(step as f32),
                inst: Inst::Bass,
                midi: clamp_midi(tonic - 12 + mode.pitch(deg)),
                vel: 110,
                frames: frames(len),
                arp: None,
                // Octave-jumping figures slur into each note. On the
                // triangle a two-frame portamento is the difference
                // between a bass line and a row of separate notes.
                glide: if arr.bass_pat == 2 && i > 0 { 2 } else { 0 },
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
                        glide: 0,
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
                        // The saw slides between the melody's strong
                        // beats, so it reads as a line under the lead
                        // rather than as a second lead.
                        glide: 3,
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
                        glide: 0,
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
                        inst: self.kit.kick(),
                        // Pickups sit under the four-on-the-floor pulse
                        // rather than competing with it.
                        vel: if i % spbeat == 0 { 120 } else { 92 },
                        midi: 0,
                        frames: 6,
                        arp: None,
                        glide: 0,
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
                        glide: 0,
                    });
                }
            }
            // The run-up bar's roll replaces the backbeat rather than
            // fighting through it.
            if arr.snare && !run_up {
                for (n, &step) in meter.snare_steps().iter().enumerate() {
                    out.push(NoteEvent {
                        at: at(step as f32),
                        // A clap on the second backbeat is the oldest
                        // trick there is for making a loop breathe.
                        inst: self.kit.snare(n % 2 == 1),
                        midi: 0,
                        vel: 104,
                        frames: 8,
                        arp: None,
                        glide: 0,
                    });
                }
            }
            if last_bar && !run_up {
                for k in 0..3u8 {
                    out.push(NoteEvent {
                        at: at((steps as u8 - 3 + k) as f32),
                        inst: self.kit.snare(false),
                        midi: 0,
                        vel: 90 + 12 * k,
                        frames: 3,
                        arp: None,
                        glide: 0,
                    });
                }
            }
        }

        // --- into the final chorus -------------------------------------
        if run_up && !ctx.zone {
            // The sweep. It is a second and a half long and ends in
            // silence, so it starts well before the bar it belongs to and
            // the gap at the top is what the downbeat lands in.
            let sweep_start = (steps as f32 - 1.5 * self.bpm / 60.0 * 4.0).max(0.0);
            out.push(NoteEvent {
                at: at(sweep_start),
                inst: Inst::Riser,
                midi: 0,
                vel: 96,
                frames: 120,
                arp: None,
                glide: 0,
            });
        }
        if run_up && !ctx.zone {
            // A full-bar snare roll and an ascending run, both crescendo,
            // both still in the old key. Landing the new key cold on the
            // downbeat is the whole trick.
            let n = (steps / 2) as i32;
            for k in 0..n {
                let step = steps as i32 - n + k;
                let grow = k as f32 / n as f32;
                out.push(NoteEvent {
                    at: at(step as f32),
                    inst: self.kit.snare(false),
                    midi: 0,
                    vel: (72.0 + 50.0 * grow) as u8,
                    frames: 3,
                    arp: None,
                    glide: 0,
                });
                out.push(NoteEvent {
                    at: at(step as f32),
                    inst: Inst::Arp,
                    // Climb the scale to just under the octave, so the
                    // modulation resolves it.
                    midi: clamp_midi(tonic + 12 + mode.pitch(7 - n + k)),
                    vel: (70.0 + 48.0 * grow) as u8,
                    frames: frames(1),
                    arp: None,
                    // Slurred, so the run reads as one gesture rather
                    // than as eight separate notes.
                    glide: 1,
                });
            }
        }
        if final_block && bar_in == 0 && !ctx.zone {
            // The arrival: a crash on the chip side and a sub-bass drop
            // under it, which is the half the console could never do.
            out.push(NoteEvent {
                at: at(0.0),
                inst: Inst::Crash,
                midi: 0,
                vel: 127,
                frames: 26,
                arp: None,
                glide: 0,
            });
            out.push(NoteEvent {
                at: at(0.0),
                inst: Inst::Impact,
                midi: 0,
                vel: 120,
                frames: 90,
                arp: None,
                glide: 0,
            });
        }

        out[start..].sort_by_key(|e| e.at);
        self.next_bar_at = bar_at + (spb * steps as f64) as u64;
        self.bar += 1;
    }

    /// The wind-down after the final chorus. The texture thins bar by bar
    /// and the key steps back home on the pivot chord, so that when the
    /// form comes round again it reads as the song repeating rather than
    /// as the music having been restarted.
    #[allow(clippy::too_many_arguments)]
    fn plan_outro(
        &mut self,
        outro_bar: usize,
        ctx: &Context,
        out: &mut Vec<NoteEvent>,
        meter: Meter,
        spb: f64,
        fps: f32,
        bar_at: u64,
        start: usize,
    ) {
        let steps = meter.steps();
        let (degree, lifted, seventh) = OUTRO[outro_bar.min(OUTRO.len() - 1)];
        self.lift = if lifted { LIFT_STEP } else { 0 };
        let chord = Chord { degree, seventh };
        let tonic = self.root + ctx.transpose + self.lift;
        // The pivot needs a flat seventh; harmonic minor's leading tone
        // would land a semitone off home. Softening the mode also suits a
        // wind-down.
        let mode = self.mode.with_flat_seventh();
        let last = outro_bar == OUTRO.len() - 1;

        let at = |step: f32| -> u64 { bar_at + (step as f64 * spb) as u64 };
        let frames = |len: u8| -> u16 { ((len as f32 * fps) as u16).max(2) };
        let arp = chord.arp(mode);

        if !ctx.zone {
            // Held chord and bass, all the way down.
            out.push(NoteEvent {
                at: at(0.0),
                inst: if last { Inst::Bell } else { Inst::WaveOrgan },
                midi: clamp_midi(tonic + mode.pitch(chord.degree)),
                vel: if last { 92 } else { 80 },
                frames: frames(steps as u8),
                arp: Some(arp),
                glide: 0,
            });
            out.push(NoteEvent {
                at: at(0.0),
                inst: Inst::Organ,
                midi: clamp_midi(tonic + 12 + mode.pitch(chord.degree)),
                vel: 78,
                frames: frames(steps as u8),
                arp: Some(arp),
                glide: 0,
            });
        }
        out.push(NoteEvent {
            at: at(0.0),
            inst: Inst::Bass,
            midi: clamp_midi(tonic - 12 + mode.pitch(chord.degree)),
            vel: 108,
            frames: frames(steps as u8),
            arp: None,
            glide: 0,
        });

        if ctx.zone {
            out[start..].sort_by_key(|e| e.at);
            self.next_bar_at = bar_at + (spb * steps as f64) as u64;
            self.bar += 1;
            return;
        }

        let spbeat = meter.steps_per_beat() as usize;
        match outro_bar {
            // Still moving, just without the top end.
            0 | 1 => {
                for step in (0..steps as usize).step_by(spbeat) {
                    out.push(NoteEvent {
                        at: at(step as f32),
                        inst: self.kit.kick(),
                        midi: 0,
                        vel: if step == 0 { 116 } else { 90 },
                        frames: 6,
                        arp: None,
                        glide: 0,
                    });
                }
            }
            // One last fill over the dominant.
            2 => {
                out.push(NoteEvent {
                    at: at(0.0),
                    inst: self.kit.kick(),
                    midi: 0,
                    vel: 112,
                    frames: 6,
                    arp: None,
                    glide: 0,
                });
                for k in 0..4u8 {
                    out.push(NoteEvent {
                        at: at((steps as u8 - 4 + k) as f32),
                        inst: self.kit.snare(false),
                        midi: 0,
                        vel: 84 + 12 * k,
                        frames: 3,
                        arp: None,
                        glide: 0,
                    });
                }
            }
            // The last bar only rings.
            _ => {
                out.push(NoteEvent {
                    at: at(0.0),
                    inst: Inst::Crash,
                    midi: 0,
                    vel: 104,
                    frames: 30,
                    arp: None,
                    glide: 0,
                });
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
                let mat = build_material(12345, p, m, default_smoothness(p));
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
            let m = build_material(
                999,
                Profile::SoloCalm,
                meter,
                default_smoothness(Profile::SoloCalm),
            );
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
            let m = build_material(
                555,
                Profile::SoloCalm,
                meter,
                default_smoothness(Profile::SoloCalm),
            );
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
        let m = build_material(
            7,
            Profile::VsIntense,
            Meter::Four,
            default_smoothness(Profile::VsIntense),
        );
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
        // rhythmic SFX and needs the acoustic space. Swept across seeds,
        // profiles and smoothness because this used to hold only by luck
        // of the draw — a cadence could extend its last note over the
        // final step and leave the bar with nowhere to breathe.
        for meter in METERS {
            for profile in Profile::ALL {
                for seed in [1u64, 7, 99, 808, 12345, 31337, 0xDEAD_BEEF] {
                    for smooth in [0.0, 0.5, 1.0] {
                        let m = build_material(seed, profile, meter, smooth);
                        for s in &m.sections {
                            for notes in &s.melody {
                                let covered: u32 = notes.iter().map(|n| n.len as u32).sum();
                                assert!(
                                    covered < meter.steps(),
                                    "a {} bar is completely full (seed {seed}, {profile:?})",
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
            }
        }
    }

    /// Average absolute interval between consecutive notes of a contour.
    fn mean_leap(smoothness: f32, seed: u64) -> f32 {
        let mut rng = Rng::new(seed);
        let mut total = 0.0;
        let mut count = 0.0;
        for _ in 0..400 {
            let c = contour(&mut rng, 8, smoothness);
            for w in c.windows(2) {
                total += (w[1] - w[0]).abs() as f32;
                count += 1.0;
            }
        }
        total / count
    }

    #[test]
    fn smoothness_actually_narrows_the_intervals() {
        let angular = mean_leap(0.0, 42);
        let smooth = mean_leap(1.0, 42);
        assert!(
            smooth < angular * 0.8,
            "smoothness should visibly narrow the line: {angular:.2} -> {smooth:.2}"
        );
        // Fully smooth is stepwise in all but name.
        assert!(smooth < 1.25, "fully smooth mean leap was {smooth:.2}");
    }

    #[test]
    fn a_contour_never_jumps_an_octave() {
        // The old register wrap subtracted a seventh in one move, which
        // put an octave between two consecutive notes. Nothing may leap
        // wider than the fifth the weights actually offer.
        let mut rng = Rng::new(0xC0FFEE);
        for smooth in [0.0, 0.25, 0.5, 0.75, 1.0] {
            for _ in 0..500 {
                let c = contour(&mut rng, 12, smooth);
                for w in c.windows(2) {
                    assert!(
                        (w[1] - w[0]).abs() <= 4,
                        "leap of {} at smoothness {smooth}",
                        (w[1] - w[0]).abs()
                    );
                }
                for d in &c {
                    assert!(
                        (CONTOUR_LOW..=CONTOUR_HIGH).contains(d),
                        "degree {d} left the register"
                    );
                }
            }
        }
    }

    #[test]
    fn every_lead_palette_holds_only_lead_instruments() {
        // A palette entry on the wrong channel would not be a wrong
        // sound, it would be a melody played over the pad or the bass.
        for profile in Profile::ALL {
            let palette = lead_palette(profile);
            assert!(!palette.is_empty(), "{profile:?} has no lead to play");
            for inst in palette {
                assert_eq!(
                    inst.voice(),
                    crate::Voice::Lead,
                    "{inst:?} in {profile:?}'s palette is not a lead instrument"
                );
                assert_ne!(lead_name(*inst), "lead", "{inst:?} has no display name");
            }
        }
    }

    #[test]
    fn pieces_do_not_all_pick_the_same_lead() {
        // The whole point of the palette: two pieces on one profile
        // should not keep arriving with the same voice on the melody.
        let mut seen: Vec<Inst> = Vec::new();
        for seed in 0..40u64 {
            let mut c = Composer::new(seed);
            c.set_profile(Profile::SoloCalm, 0.4);
            if !seen.contains(&c.lead()) {
                seen.push(c.lead());
            }
        }
        assert!(
            seen.len() >= 4,
            "40 seeds only ever produced {} lead(s): {seen:?}",
            seen.len()
        );
    }

    #[test]
    fn pinning_a_lead_holds_it_and_unpinning_gives_it_back() {
        let mut c = Composer::new(31);
        c.set_profile(Profile::SoloCalm, 0.4);
        let rolled = c.lead();

        c.force_lead(Some(Inst::Guitar));
        assert_eq!(c.lead(), Inst::Guitar);

        // Dropping the pin restores exactly what THIS piece rolled — the
        // roll is per piece, so this only holds without a re-roll in
        // between.
        c.force_lead(None);
        assert_eq!(c.lead(), rolled, "unpinning must restore the rolled lead");

        // A pin outlives the next piece, like the meter and kit pins.
        c.force_lead(Some(Inst::Guitar));
        c.set_profile(Profile::VsIntense, 0.8);
        assert_eq!(c.lead(), Inst::Guitar);
    }

    #[test]
    fn a_pin_on_the_wrong_channel_is_refused() {
        let mut c = Composer::new(5);
        c.set_profile(Profile::SoloCalm, 0.4);
        let rolled = c.lead();
        // Bass is a triangle instrument; honouring this would put the
        // melody on the bass channel.
        c.force_lead(Some(Inst::Bass));
        assert_eq!(c.lead(), rolled);
        assert_eq!(c.lead().voice(), crate::Voice::Lead);
    }

    #[test]
    fn unpinning_smoothness_restores_the_profile_default() {
        let mut c = Composer::new(7);
        c.set_profile(Profile::VsIntense, 0.5);
        let rolled = c.smoothness();
        assert_eq!(rolled, default_smoothness(Profile::VsIntense));
        c.force_smoothness(Some(1.0));
        assert_eq!(c.smoothness(), 1.0);
        // A pin survives the next piece; dropping it does not.
        c.set_profile(Profile::SoloCalm, 0.5);
        assert_eq!(c.smoothness(), 1.0, "a pin must survive a re-roll");
        c.force_smoothness(None);
        assert_eq!(c.smoothness(), default_smoothness(Profile::SoloCalm));
    }

    /// The game draws the label in Misaki, an 8x8 JIS X 0208 pixel font.
    /// A character outside that set does not fall back to anything — it
    /// renders as an empty box. A Latin-1 middle dot (U+00B7) used as the
    /// separator did exactly that; the katakana one (U+30FB) is in the
    /// set, as is the musical note the toast prefixes.
    #[test]
    fn the_label_only_uses_characters_the_game_can_draw() {
        const ALLOWED_NON_ASCII: [char; 2] = ['・', '♪'];
        let mut checked = 0;
        for p in [
            Profile::Ambient,
            Profile::SoloCalm,
            Profile::VsIntense,
            Profile::Victory,
        ] {
            for seed in [0u64, 1, 0xC0FFEE, u64::MAX] {
                let mut c = Composer::new(seed);
                c.set_profile(p, 0.5);
                for meter in METERS {
                    c.force_meter(meter);
                    for mode in [
                        Mode::Lydian,
                        Mode::Ionian,
                        Mode::Mixolydian,
                        Mode::Dorian,
                        Mode::Aeolian,
                        Mode::Phrygian,
                        Mode::HarmonicMinor,
                        Mode::PhrygianDominant,
                    ] {
                        c.mode = mode;
                        let label = c.info().label();
                        for ch in label.chars() {
                            assert!(
                                ch.is_ascii() || ALLOWED_NON_ASCII.contains(&ch),
                                "label {label:?} contains U+{:04X} {ch:?}, which the \
                                 8x8 pixel font will draw as an empty box",
                                ch as u32
                            );
                        }
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 100);
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
        assert!(
            seen.iter().all(|&n| n > 20),
            "a meter never came up: {seen:?}"
        );
        assert!(
            seen[0] > seen[1] + seen[2],
            "4/4 should stay the house style"
        );
        // Versus keeps a floor to stand on far more often.
        let mut vs_four = 0;
        for seed in 0..400u64 {
            if roll_meter(Profile::VsIntense, &mut Rng::new(seed)) == Meter::Four {
                vs_four += 1;
            }
        }
        assert!(
            vs_four > 280,
            "versus wandered off 4/4 too often: {vs_four}"
        );
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
        assert!(arrange(Profile::VsIntense, &ctx, Inst::Pluck).four_floor);

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
                .filter(|e| e.inst.is_kick())
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

    /// The bug this test exists for: the profile switched correctly, the
    /// key and tempo changed, and the player heard nothing recognizable
    /// because the layer schedule opened on sixteen seconds of solo bass.
    /// A screen change has to sound like a *different song*, immediately.
    #[test]
    fn a_piece_is_never_bare_at_bar_one() {
        use std::collections::HashSet;
        for profile in [Profile::Ambient, Profile::SoloCalm, Profile::VsIntense] {
            let mut c = Composer::new(0xD00D);
            c.set_profile(profile, 0.0);
            // Bar one of a brand new piece: no elapsed time, no danger.
            let ctx = Context {
                profile,
                intensity: 0.0,
                elapsed: 0.0,
                ..Default::default()
            };
            let mut out = Vec::new();
            c.plan_bar(&ctx, &mut out);

            let voices: HashSet<u8> = out.iter().map(|e| e.voice() as u8).collect();
            assert!(
                voices.len() >= 3,
                "{profile:?} opens on {} voice(s) — that reads as the old \
                 music having stopped, not as new music starting",
                voices.len()
            );
            assert!(
                out.len() >= 4,
                "{profile:?} opens with only {} notes",
                out.len()
            );
            // Bass, harmony and pad specifically: the base has to be a
            // song, not a bass line.
            let layers = c.info().layer_list();
            for want in ["bass", "harm", "pad"] {
                assert!(
                    layers.contains(want),
                    "{profile:?} opens without {want}: {layers}"
                );
            }
        }
    }

    #[test]
    fn the_arrangement_still_builds() {
        // Immediate does not mean everything at once — percussion and
        // melody still have to arrive later, or there is no build left.
        let at = |elapsed: f32| {
            let ctx = Context {
                profile: Profile::SoloCalm,
                intensity: 0.0,
                elapsed,
                ..Default::default()
            };
            arrange(Profile::SoloCalm, &ctx, Inst::Pluck)
        };
        let opening = at(0.0);
        assert!(!opening.perc && !opening.lead);
        assert!(at(8.0).perc, "the drums never came in");
        assert!(at(20.0).lead, "the melody never came in");
        assert!(at(0.0).layers() < at(60.0).layers());
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
        let early = arrange(Profile::VsIntense, &ctx(0.0, 0.1, false), Inst::Pluck);
        assert!(early.counter.is_none() && early.saw.is_none() && !early.perc);
        // Everything by the time it has been running a while and the
        // stack is high.
        let late = arrange(Profile::VsIntense, &ctx(120.0, 0.9, false), Inst::Pluck);
        assert!(late.counter.is_some() && late.saw.is_some() && late.pad.is_some());
        assert!(late.shaker && late.perc && late.lead);
        // The zone strips it back to a held chord over the bass.
        let zone = arrange(Profile::VsIntense, &ctx(120.0, 0.9, true), Inst::Pluck);
        assert!(!zone.lead && !zone.perc && !zone.shaker);
        assert!(zone.counter.is_none() && zone.saw.is_none());
        assert!(zone.harmony && zone.pad.is_some());
    }

    /// The form's last block is the final chorus: key up a whole tone,
    /// everything in, and a run-up into it.
    #[test]
    fn the_final_chorus_changes_key_and_goes_all_in() {
        let mut c = Composer::new(0x5A5A);
        c.set_profile(Profile::VsIntense, 0.4);
        let ctx = Context {
            profile: Profile::VsIntense,
            intensity: 0.4,
            elapsed: 600.0,
            ..Default::default()
        };
        let blocks = c.material.form.len() as u32;
        let bars_per_pass = blocks * BARS_PER_SECTION;

        // Everything up to the last block stays in the home key.
        let mut out = Vec::new();
        for _ in 0..bars_per_pass - BARS_PER_SECTION {
            c.plan_bar(&ctx, &mut out);
        }
        assert_eq!(c.lift, 0, "the key moved before the final chorus");
        assert!(!c.take_modulated());
        let home = c.info().tonic;

        let mark = out.len();
        c.plan_bar(&ctx, &mut out);
        assert_eq!(c.lift, LIFT_STEP, "the final chorus did not modulate");
        assert!(c.take_modulated(), "the modulation was not reported");
        assert!(!c.take_modulated(), "the flag must only fire once");
        assert_eq!(c.info().tonic, home + LIFT_STEP);

        // The downbeat of the final chorus carries the crash.
        let bar = &out[mark..];
        assert!(
            bar.iter().any(|e| e.inst == Inst::Crash),
            "no crash on the downbeat of the final chorus"
        );
        // ...and every voice is playing.
        let voices: std::collections::HashSet<u8> = bar.iter().map(|e| e.voice() as u8).collect();
        assert_eq!(voices.len(), crate::VOICE_COUNT, "not all in: {voices:?}");
    }

    #[test]
    fn the_run_up_crescendos_in_the_old_key() {
        let mut c = Composer::new(0x5A5A);
        c.set_profile(Profile::SoloCalm, 0.5);
        let ctx = Context {
            profile: Profile::SoloCalm,
            intensity: 0.5,
            elapsed: 600.0,
            ..Default::default()
        };
        let blocks = c.material.form.len() as u32;
        let mut out = Vec::new();
        // Stop one bar short of the final block.
        for _ in 0..(blocks - 1) * BARS_PER_SECTION - 1 {
            c.plan_bar(&ctx, &mut out);
        }
        let before = c.info().tonic;
        let mark = out.len();
        c.plan_bar(&ctx, &mut out);
        let bar = &out[mark..];

        assert_eq!(
            c.info().tonic,
            before,
            "the run-up must stay in the old key"
        );
        let mut snares: Vec<u8> = bar
            .iter()
            .filter(|e| e.inst.is_snare())
            .map(|e| e.vel)
            .collect();
        assert!(snares.len() >= 6, "the roll is only {} hits", snares.len());
        let first = snares[0];
        snares.sort_unstable();
        assert_eq!(snares[0], first, "the roll does not start at its quietest");
        assert!(
            *snares.last().unwrap() > first + 30,
            "the roll does not crescendo"
        );
        // An ascending run on the counter channel.
        let run: Vec<u8> = bar
            .iter()
            .filter(|e| e.inst == Inst::Arp)
            .map(|e| e.midi)
            .collect();
        assert!(run.len() >= 6);
        assert!(
            run.windows(2).all(|w| w[1] >= w[0]),
            "the run is not rising"
        );
    }

    /// The point of the outro: however long a session runs, the song is a
    /// loop and the key always comes back to where it started.
    #[test]
    fn the_key_always_comes_home() {
        let mut c = Composer::new(7);
        c.set_profile(Profile::VsIntense, 0.5);
        let ctx = Context {
            profile: Profile::VsIntense,
            intensity: 0.5,
            elapsed: 600.0,
            ..Default::default()
        };
        let body = c.material.form.len() as u32 * BARS_PER_SECTION;
        let cycle = body + OUTRO_BARS;
        let mut out = Vec::new();
        let mut lifts = Vec::new();
        // Ten cycles: far longer than any real match.
        for _ in 0..cycle * 10 {
            c.plan_bar(&ctx, &mut out);
            lifts.push(c.lift);
        }
        assert!(
            lifts.iter().all(|&l| l == 0 || l == LIFT_STEP),
            "the key wandered somewhere unplanned"
        );
        assert!(lifts.contains(&LIFT_STEP), "it never modulated at all");
        // Every cycle ends at home, which is what makes the seam a repeat.
        for c_i in 0..10u32 {
            let last_bar_of_cycle = (c_i * cycle + cycle - 1) as usize;
            assert_eq!(lifts[last_bar_of_cycle], 0, "cycle {c_i} did not come home");
        }
        // Notes must still be playable after the lift.
        for e in &out {
            if !e.inst.is_drum() {
                assert!((21..=108).contains(&e.midi));
            }
        }
    }

    /// The pivot is the whole reason the descent works: a whole tone up,
    /// the flat-seventh chord is built on the home tonic.
    #[test]
    fn the_outro_pivots_through_the_home_tonic() {
        for mode in [
            Mode::Aeolian,
            Mode::Dorian,
            Mode::Phrygian,
            Mode::HarmonicMinor,
        ] {
            let lifted_flat_seven = LIFT_STEP + mode.with_flat_seventh().pitch(6);
            assert_eq!(
                lifted_flat_seven.rem_euclid(12),
                0,
                "{mode:?}: the pivot chord is not the home tonic"
            );
        }
        // The outro table must actually open on that chord, in the lifted
        // key, and finish at home on the tonic.
        assert_eq!(OUTRO[0], (6, true, false));
        assert!(!OUTRO.last().unwrap().1);
        assert_eq!(OUTRO.last().unwrap().0, 0);
        // ...via a dominant seventh, or it is not a cadence.
        assert!(OUTRO.iter().any(|&(d, _, s)| d == 4 && s));
    }

    #[test]
    fn the_outro_winds_down_and_rings_out() {
        let mut c = Composer::new(11);
        c.set_profile(Profile::VsIntense, 0.7);
        let ctx = Context {
            profile: Profile::VsIntense,
            intensity: 0.7,
            elapsed: 600.0,
            ..Default::default()
        };
        let body = c.material.form.len() as u32 * BARS_PER_SECTION;
        let mut out = Vec::new();
        for _ in 0..body {
            c.plan_bar(&ctx, &mut out);
        }
        let mut bars = Vec::new();
        for _ in 0..OUTRO_BARS {
            let mark = out.len();
            c.plan_bar(&ctx, &mut out);
            bars.push(out[mark..].to_vec());
        }

        // No melody anywhere in the outro: the top end is what drops first.
        for (i, bar) in bars.iter().enumerate() {
            assert!(
                !bar.iter().any(|e| e.voice() == crate::Voice::Lead),
                "outro bar {i} still has a melody"
            );
            assert!(
                bar.iter().any(|e| e.inst == Inst::Bass),
                "outro bar {i} lost the bass"
            );
        }
        // It thins as it goes.
        assert!(
            bars[0].len() > bars[3].len(),
            "the outro did not thin out ({} -> {})",
            bars[0].len(),
            bars[3].len()
        );
        // The last bar just rings.
        assert!(bars[3].iter().any(|e| e.inst == Inst::Crash));
        assert!(bars[3].iter().any(|e| e.inst == Inst::Bell));
        assert!(!bars[3].iter().any(|e| e.inst.is_kick()));
        // And the loop point is back in the home key.
        assert_eq!(c.lift, 0);
    }

    #[test]
    fn menus_never_get_a_final_chorus() {
        for p in [Profile::Ambient, Profile::Victory] {
            let mut c = Composer::new(3);
            c.set_profile(p, 0.3);
            let ctx = Context {
                profile: p,
                elapsed: 600.0,
                ..Default::default()
            };
            let mut out = Vec::new();
            for _ in 0..c.material.form.len() as u32 * BARS_PER_SECTION * 3 {
                c.plan_bar(&ctx, &mut out);
            }
            assert_eq!(c.lift, 0, "{p:?} modulated");
            assert!(!out.iter().any(|e| e.inst == Inst::Crash));
        }
    }

    #[test]
    fn a_full_arrangement_reaches_every_voice() {
        use crate::Voice;
        use std::collections::HashSet;
        let voices = |kit: Kit| -> HashSet<u8> {
            let mut c = Composer::new(0xBEEF);
            c.set_profile(Profile::Victory, 0.9);
            c.force_kit(kit);
            let ctx = Context {
                profile: Profile::Victory,
                intensity: 0.9,
                elapsed: 600.0,
                ..Default::default()
            };
            let mut out = Vec::new();
            for _ in 0..8 {
                c.plan_bar(&ctx, &mut out);
            }
            out.iter().map(|e| e.voice() as u8).collect()
        };

        // The chip kit uses every console and expansion voice; the sample
        // channel only speaks for sweeps and impacts, which belong to the
        // final chorus rather than to an ordinary bar.
        let chip = voices(Kit::Chip);
        for v in 0..Voice::Sample as u8 {
            assert!(chip.contains(&v), "chip kit never used voice {v}: {chip:?}");
        }
        // The sampled kit moves the kick and snare off the noise channel
        // and onto the sample channel; the hats stay where they are.
        let pcm = voices(Kit::Sampled);
        assert!(pcm.contains(&(Voice::Sample as u8)));
        assert!(pcm.contains(&(Voice::Hat as u8)));
        assert!(
            !pcm.contains(&(Voice::Perc as u8)),
            "the sampled kit still used the noise drums"
        );
    }

    #[test]
    fn both_kits_play_the_same_pattern() {
        // Swapping the kit must change the timbre and nothing else: the
        // groove is the composer's, not the drum machine's.
        let hits = |kit: Kit| -> Vec<(u64, bool)> {
            let mut c = Composer::new(4242);
            c.set_profile(Profile::VsIntense, 0.8);
            c.force_kit(kit);
            let ctx = Context {
                profile: Profile::VsIntense,
                intensity: 0.8,
                elapsed: 600.0,
                ..Default::default()
            };
            let mut out = Vec::new();
            for _ in 0..16 {
                c.plan_bar(&ctx, &mut out);
            }
            out.iter()
                .filter(|e| e.inst.is_kick() || e.inst.is_snare())
                .map(|e| (e.at, e.inst.is_kick()))
                .collect()
        };
        assert_eq!(hits(Kit::Chip), hits(Kit::Sampled));
        assert!(!hits(Kit::Chip).is_empty());
    }

    #[test]
    fn the_riser_arrives_before_the_chorus_and_the_impact_with_it() {
        let mut c = Composer::new(0x1111);
        c.set_profile(Profile::VsIntense, 0.6);
        let ctx = Context {
            profile: Profile::VsIntense,
            intensity: 0.6,
            elapsed: 600.0,
            ..Default::default()
        };
        let blocks = c.material.form.len() as u32;
        let mut out = Vec::new();
        // Up to the run-up bar.
        for _ in 0..(blocks - 1) * BARS_PER_SECTION - 1 {
            c.plan_bar(&ctx, &mut out);
        }
        let mark = out.len();
        c.plan_bar(&ctx, &mut out); // run-up
        let downbeat = c.next_bar_at();
        c.plan_bar(&ctx, &mut out); // final chorus, bar one
        let tail = &out[mark..];

        let riser = tail
            .iter()
            .find(|e| e.inst == Inst::Riser)
            .expect("no riser into the final chorus");
        let impact = tail
            .iter()
            .find(|e| e.inst == Inst::Impact)
            .expect("no impact on the downbeat");
        assert!(riser.at < downbeat, "the riser starts too late to sweep");
        assert_eq!(impact.at, downbeat, "the impact missed the downbeat");
        // The sweep is a second and a half long, so it has to start about
        // that far ahead or it gets cut off mid-climb.
        let lead_in = (downbeat - riser.at) as f32 / SAMPLE_RATE as f32;
        assert!(
            (0.9..=1.6).contains(&lead_in),
            "the riser leads in by {lead_in:.2} s"
        );
    }

    #[test]
    fn glissando_is_used_sparingly_and_only_where_it_means_something() {
        let out = run(Profile::VsIntense, 0.9, 32);
        let glides = out.iter().filter(|e| e.glide > 0).count();
        assert!(glides > 0, "nothing ever slides");
        assert!(
            glides * 3 < out.len(),
            "{glides} of {} notes slide — that is a slide whistle",
            out.len()
        );
        // Percussion has no pitch to slide.
        assert!(!out.iter().any(|e| e.inst.is_drum() && e.glide > 0));
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
        let a = arrange(Profile::VsIntense, &ctx, Inst::Pluck);
        assert!(!a.lead && !a.perc && a.harmony);
    }

    #[test]
    fn composition_is_deterministic() {
        let one = run(Profile::VsIntense, 0.55, 40);
        let two = run(Profile::VsIntense, 0.55, 40);
        assert_eq!(one.len(), two.len());
        for (a, b) in one.iter().zip(&two) {
            assert_eq!(
                (a.at, a.midi, a.vel, a.frames),
                (b.at, b.midi, b.vel, b.frames)
            );
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
                        assert!(
                            (21..=108).contains(&e.midi),
                            "{p:?} emitted midi {}",
                            e.midi
                        );
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
            let a = build_material(
                0xABCD,
                Profile::SoloCalm,
                meter,
                default_smoothness(Profile::SoloCalm),
            );
            let b = build_material(
                0xABCD,
                Profile::SoloCalm,
                meter,
                default_smoothness(Profile::SoloCalm),
            );
            assert_eq!(a.rhythms_used, b.rhythms_used);
            assert_eq!(a.form, b.form);
        }
    }
}
