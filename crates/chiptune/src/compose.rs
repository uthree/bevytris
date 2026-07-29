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
use crate::{FRAME_RATE, Inst, NoteEvent, SAMPLE_RATE, STEPS_PER_BAR};

pub const BARS_PER_SECTION: u32 = 8;

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
const RHYTHMS: [&[(u8, u8)]; 8] = [
    &[(0, 3), (4, 3), (8, 3), (12, 3)],
    &[(0, 2), (2, 2), (4, 3), (8, 2), (10, 2), (12, 3)],
    &[(0, 3), (3, 3), (6, 2), (8, 3), (12, 3)],
    &[(0, 4), (6, 2), (8, 4), (12, 2), (14, 2)],
    &[(0, 2), (2, 2), (4, 2), (6, 2), (8, 3), (12, 3)],
    &[(0, 6), (6, 2), (8, 5), (14, 2)],
    &[(0, 7), (8, 4), (12, 3)],
    &[(0, 2), (3, 1), (4, 2), (7, 1), (8, 2), (11, 1), (12, 3)],
];

/// I-V-vi-IV and friends. Degrees are 0-indexed, so 4 is V and 5 is vi
/// (or bVI in a minor mode — the same numbers work in every mode, which
/// is the whole point of storing degrees).
const CALM_LOOPS: [[i32; 4]; 4] = [
    [0, 4, 5, 3],
    [5, 3, 0, 4],
    [0, 3, 4, 0],
    [0, 5, 3, 4],
];
/// The minor-mode set. `[0, 6, 5, 4]` is the Andalusian cadence — the
/// harmonic cousin of the Korobeiniki idiom, so it reads as genre-correct
/// without quoting anything.
const DARK_LOOPS: [[i32; 4]; 4] = [
    [0, 3, 4, 0],
    [0, 5, 2, 6],
    [0, 6, 5, 4],
    [0, 1, 4, 0],
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
    /// Which rhythm cells this session drew. Nothing reads it at runtime;
    /// it exists so the "at most eight cells" invariant is testable.
    #[allow(dead_code)]
    rhythms_used: Vec<usize>,
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

fn build_material(seed: u64, profile: Profile) -> Material {
    let mut rng = Rng::new(seed ^ ((profile as u64 + 1) * 0x9E37_79B9));
    let dark = matches!(profile, Profile::VsIntense);

    // Four motifs, each on its own rhythm cell.
    let mut rhythms_used: Vec<usize> = Vec::new();
    let mut motifs: Vec<Motif> = Vec::new();
    while motifs.len() < 4 {
        let r = rng.below(RHYTHMS.len());
        if rhythms_used.contains(&r) {
            continue;
        }
        rhythms_used.push(r);
        let n = RHYTHMS[r].len();
        motifs.push(Motif {
            rhythm: r,
            contour: contour(&mut rng, n),
        });
    }

    let loops = if dark { &DARK_LOOPS } else { &CALM_LOOPS };
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
                realize(&motifs[mi], tf, chords[bar])
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
fn realize(motif: &Motif, tf: Transform, chord: Chord) -> Vec<MelNote> {
    let rhythm = RHYTHMS[motif.rhythm];
    let shift = match tf {
        Transform::None | Transform::Cadence => 0,
        Transform::Up => 2,
    };
    let mut out = Vec::with_capacity(rhythm.len());
    for (i, &(step, len)) in rhythm.iter().enumerate() {
        let mut deg = motif.contour[i.min(motif.contour.len() - 1)] + shift;
        let prio = if step % 4 == 0 {
            0
        } else if step % 2 == 0 {
            1
        } else {
            2
        };
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
            last.len = (last.len + 2).min(STEPS_PER_BAR as u8 - last.step);
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

/// Discrete tempo ladder. Discrete rather than continuous on purpose: the
/// NES original is a two-state switch, a step is far more legible to the
/// player than a glide, and a step is much easier to keep musical.
fn tempo_target(profile: Profile, intensity: f32) -> f32 {
    match profile {
        Profile::Ambient => 96.0,
        Profile::Victory => 132.0,
        Profile::SoloCalm => [118.0, 132.0, 150.0, 174.0][band(intensity)],
        Profile::VsIntense => [148.0, 162.0, 178.0, 214.0][band(intensity)],
    }
}

/// Mode ladder, bright to dark. The tonic never moves, so the bass never
/// has to know this happened.
fn mode_target(profile: Profile, intensity: f32) -> Mode {
    match profile {
        Profile::Ambient => Mode::Ionian,
        Profile::Victory => Mode::Lydian,
        Profile::SoloCalm => [
            Mode::Ionian,
            Mode::Mixolydian,
            Mode::Dorian,
            Mode::Aeolian,
        ][band(intensity)],
        Profile::VsIntense => [
            Mode::Dorian,
            Mode::Aeolian,
            Mode::Aeolian,
            Mode::PhrygianDominant,
        ][band(intensity)],
    }
}

struct Arrangement {
    lead: bool,
    harmony: bool,
    perc: bool,
    lead_inst: Inst,
    harm_inst: Inst,
    bass_pat: usize,
    hat_k: usize,
    kick_k: usize,
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

    let mut a = match profile {
        Profile::Ambient => Arrangement {
            lead: t >= l_at,
            harmony: t >= h_at,
            perc: false,
            lead_inst: Inst::Soft,
            harm_inst: Inst::Pad,
            bass_pat: 0,
            hat_k: 0,
            kick_k: 0,
            snare: false,
            swing: 0.58,
            max_prio: 0,
        },
        Profile::SoloCalm => Arrangement {
            lead: t >= l_at,
            harmony: t >= h_at,
            perc: t >= p_at,
            lead_inst: if i < 0.45 { Inst::Soft } else { Inst::Sustain },
            harm_inst: if i < 0.62 { Inst::Pad } else { Inst::Organ },
            bass_pat: [0, 1, 2, 3][b],
            hat_k: [0, 2, 4, 8][b],
            kick_k: [2, 2, 3, 4][b],
            snare: i >= 0.50,
            swing: 0.56,
            max_prio: if i < 0.35 { 0 } else { 1 },
        },
        Profile::VsIntense => Arrangement {
            lead: t >= l_at,
            harmony: t >= h_at,
            perc: t >= p_at,
            lead_inst: Inst::Pluck,
            harm_inst: if i < 0.55 { Inst::Organ } else { Inst::Stab },
            bass_pat: [1, 2, 2, 4][b],
            hat_k: [4, 8, 11, 13][b],
            kick_k: [4, 4, 5, 6][b],
            snare: true,
            swing: 0.52,
            max_prio: if i < 0.30 { 1 } else { 2 },
        },
        Profile::Victory => Arrangement {
            lead: true,
            harmony: true,
            perc: true,
            lead_inst: Inst::Sustain,
            harm_inst: Inst::Organ,
            bass_pat: 1,
            hat_k: 4,
            kick_k: 3,
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
        a.harm_inst = Inst::Pad;
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
    pub bpm: f32,
    pub bar: u32,
}

impl Info {
    /// e.g. `A minor · 148 BPM · #3f7a2c`
    pub fn label(&self) -> String {
        format!(
            "{} {} · {:.0} BPM · #{:06x}",
            NOTE_NAMES[(self.tonic.rem_euclid(12)) as usize],
            self.mode.name(),
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
    bpm: f32,
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
            material: build_material(seed, profile),
            mode: mode_target(profile, 0.0),
            root,
            bpm: tempo_target(profile, 0.0),
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

    /// Restart the form with a different personality. Material is derived
    /// from `seed ^ profile`, so coming back to a profile later brings
    /// back the same tunes rather than rolling new ones.
    pub fn set_profile(&mut self, profile: Profile, intensity: f32) {
        if profile == self.profile {
            return;
        }
        self.profile = profile;
        self.material = build_material(self.seed, profile);
        self.mode = mode_target(profile, intensity);
        self.bpm = tempo_target(profile, intensity);
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
        // Tempo is latched per bar and ramped, so a threshold that flaps
        // cannot make the music stutter.
        let target = tempo_target(self.profile, ctx.intensity);
        self.bpm += (target - self.bpm).clamp(-9.0, 9.0);

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
            let swung = if (step as i32) % 2 == 1 {
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
            if arr.harm_inst == Inst::Stab {
                // Offbeat stabs drive; a held pad would just sit there.
                for step in [2u8, 6, 10, 14] {
                    out.push(NoteEvent {
                        at: at(step as f32),
                        inst: Inst::Stab,
                        midi: base,
                        vel: 96,
                        frames: frames(2),
                        arp: Some(arp),
                    });
                }
            } else {
                for step in [0u8, 8] {
                    out.push(NoteEvent {
                        at: at(step as f32),
                        inst: arr.harm_inst,
                        midi: base,
                        vel: 84,
                        frames: frames(8),
                        arp: Some(arp),
                    });
                }
            }
        }

        // --- bass -----------------------------------------------------
        for (step, len, deg) in bass_pattern(arr.bass_pat, chord.degree) {
            out.push(NoteEvent {
                at: at(step as f32),
                inst: Inst::Bass,
                midi: clamp_midi(tonic - 12 + mode.pitch(deg)),
                vel: 110,
                frames: frames(len),
                arp: None,
            });
        }

        // --- drums ----------------------------------------------------
        if arr.perc {
            // The unconditional fill in the last bar of every section is
            // what makes a loop feel like it has form.
            let (hat_k, kick_k) = if last_bar {
                ((arr.hat_k + 4).min(16), arr.kick_k)
            } else {
                (arr.hat_k, arr.kick_k)
            };
            for (i, on) in euclid_rot(hat_k, 16, hat_rot).iter().enumerate() {
                if *on {
                    out.push(NoteEvent {
                        at: at(i as f32),
                        inst: Inst::Hat,
                        midi: 0,
                        vel: if i % 4 == 0 { 96 } else { 70 },
                        frames: 4,
                        arp: None,
                    });
                }
            }
            for (i, on) in euclid_rot(kick_k, 16, kick_rot).iter().enumerate() {
                if *on {
                    out.push(NoteEvent {
                        at: at(i as f32),
                        inst: Inst::Kick,
                        midi: 0,
                        vel: 118,
                        frames: 6,
                        arp: None,
                    });
                }
            }
            if arr.snare {
                for step in [4u8, 12] {
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
                for (k, step) in [13u8, 14, 15].iter().enumerate() {
                    out.push(NoteEvent {
                        at: at(*step as f32),
                        inst: Inst::Snare,
                        midi: 0,
                        vel: 90 + 12 * k as u8,
                        frames: 3,
                        arp: None,
                    });
                }
            }
        }

        out[start..].sort_by_key(|e| e.at);
        self.next_bar_at = bar_at + (spb * STEPS_PER_BAR as f64) as u64;
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
fn bass_pattern(pat: usize, root: i32) -> Vec<(u8, u8, i32)> {
    match pat {
        0 => vec![(0, 16, root)],
        1 => vec![(0, 8, root), (8, 8, root + 4)],
        2 => (0..8)
            .map(|i| {
                let step = i * 2;
                let deg = if i % 2 == 0 { root } else { root + 7 };
                (step as u8, 2u8, deg)
            })
            .collect(),
        3 => vec![
            (0, 4, root),
            (4, 4, root + 4),
            (8, 4, root + 7),
            (12, 4, root + 4),
        ],
        _ => (0..16)
            .map(|i| {
                let deg = root + [0, 2, 4, 7][i % 4];
                (i as u8, 1u8, deg)
            })
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

    #[test]
    fn a_session_uses_at_most_eight_rhythm_cells() {
        for p in [Profile::Ambient, Profile::SoloCalm, Profile::VsIntense] {
            let m = build_material(12345, p);
            assert!(
                m.rhythms_used.len() <= 8,
                "{p:?} used {} rhythm cells",
                m.rhythms_used.len()
            );
            assert_eq!(m.rhythms_used.iter().collect::<HashSet<_>>().len(), 4);
        }
    }

    #[test]
    fn every_phrase_ends_on_a_cadence() {
        let m = build_material(999, Profile::SoloCalm);
        for s in &m.sections {
            assert_eq!(s.chords[3].degree, 4, "antecedent must close on V");
            assert!(s.chords[3].seventh);
            assert_eq!(s.chords[7].degree, 0, "consequent must close on I");
        }
    }

    #[test]
    fn strong_beats_are_chord_tones() {
        let m = build_material(555, Profile::SoloCalm);
        let mut checked = 0;
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
        assert!(checked > 40, "test did not actually look at much");
    }

    #[test]
    fn material_is_reused_not_re_rolled() {
        // The form must revisit sections: 64 bars of music from 24 bars of
        // material keeps new material inside the ~35% budget real songs
        // sit in.
        let m = build_material(7, Profile::VsIntense);
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
        let m = build_material(31337, Profile::VsIntense);
        for s in &m.sections {
            for notes in &s.melody {
                let covered: u32 = notes.iter().map(|n| n.len as u32).sum();
                assert!(covered < 16, "a bar is completely full");
            }
        }
    }

    #[test]
    fn calm_is_slower_and_brighter_than_versus() {
        assert!(tempo_target(Profile::SoloCalm, 0.5) < tempo_target(Profile::VsIntense, 0.5));
        assert!(!mode_target(Profile::SoloCalm, 0.0).is_minor());
        assert!(mode_target(Profile::SoloCalm, 0.9).is_minor());
        assert!(mode_target(Profile::VsIntense, 0.0).is_minor());
        // Danger really does push the tempo up.
        assert!(tempo_target(Profile::VsIntense, 0.95) > tempo_target(Profile::VsIntense, 0.1));
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
                    if !matches!(e.inst, Inst::Kick | Inst::Snare | Inst::Hat) {
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
    fn re_entering_a_profile_brings_back_the_same_tunes() {
        let a = build_material(0xABCD, Profile::SoloCalm);
        let b = build_material(0xABCD, Profile::SoloCalm);
        assert_eq!(a.rhythms_used, b.rhythms_used);
        assert_eq!(a.form, b.form);
    }
}
